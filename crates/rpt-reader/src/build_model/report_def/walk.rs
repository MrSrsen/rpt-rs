//! The ordered walk over the report-definition record stream: the cursor into the layout tree being
//! built, the values one record carries forward to a later one, and one handler per record that
//! writes into it.

use super::conditions::{condition_formula_bodies, condition_slots, resolve_conditions};
use super::data_source::{field_data_source, field_object, group_display_number};
use super::formats::{
    apply_field_format_child, apply_object_format, build_border, build_font, build_object_name,
    build_object_pos, decode_area_format, decode_section_format, font_color_mut, is_section_format,
};
use super::objects::{current_object, open_object, push_text_run, set_object_text};
use super::sections::current_section;
use super::summary::{collect_summary_defs, SummaryDef};
use super::{sections, DrawingShapeKind, RdRecord};
use crate::build_model::record_values::colorref_or_white;
use crate::build_model::row_of;
use crate::codec::RecordNode;
use crate::field_table::table::{Cell, Row, Table, UNSET_FIELD_INDEX};
use crate::field_table::tables as ft;
use crate::model::{
    Alignment, Area, AreaSectionKind, FieldRefKind, Group, ReportObject, ReportObjectKind,
    TextObject, Twips,
};
use std::collections::BTreeMap;

/// The walk over the flat report-definition record stream.
///
/// The stream is a grammar in document order, not a bag of independent records: an *opener* starts a
/// report object and the attribute records that follow decorate that object until the next opener,
/// and a few records hand a value forward to a later one. The fields below are the walk's only
/// memory between records, so a handler that reads one is coupled to the handler that wrote it, and
/// feeding the records out of order changes the result. [`RdRecord`] names each record's role in
/// that grammar.
pub(super) struct RdWalk<'a> {
    /// The layout tree so far. The cursor into it is always the last of the last: the current area
    /// is the last area, its current section that area's last section, and the current object that
    /// section's last object.
    areas: Vec<Area>,
    /// The stream's logical bytes, which every record's content is read out of.
    logical: &'a [u8],
    /// The report's groups, which a field object's data-source reference resolves against.
    groups: &'a [Group],
    /// Conditional-format formula bodies, keyed by global formula index; an object's or section's
    /// condition-slot record names the exact body by that index.
    conditions: BTreeMap<usize, (String, String)>,
    /// The ordered summary definitions a Summary field object indexes into by its opener code.
    sum_defs: Vec<SummaryDef>,
    /// The bound field reference of a blob-field object: set by its `0xb1` wrapper, consumed by the
    /// `0xae` picture opener that immediately follows (the wrapper is the opener's parent record).
    pending_blob_ds: Option<String>,
    /// Border color condition formulas read from a `0xed` wrapper, awaiting the `0xec` border it
    /// parents (that record rebuilds the border fresh, so conditions can only be attached
    /// afterwards). One wrapper precedes each border, so each is consumed before the next is set.
    pending_border_conditions: Vec<(String, String)>,
    /// The band kind announced by the most recent band marker (`0x8d`..`0x99`), consumed by the
    /// `0x8c` section it parents to set the authoritative area/section kind.
    pending_band_kind: Option<AreaSectionKind>,
    /// Whether the current object has taken its font. A text object stores one font per run
    /// (`0x08`), interleaved with the run text (`0xc2`), and the engine reports the object's font as
    /// the FIRST run's, so later runs are ignored. Re-armed at every object opener.
    font_set: bool,
    /// Whether the current object has taken its font color. `0x0100` streams once per run, ahead of
    /// that run's `0x08`, and the first color after an opener likewise wins. Re-armed alongside
    /// [`Self::font_set`].
    color_set: bool,
    /// Whether the current field still awaits the first of the two numeric-format records it
    /// streams — the currency slot, which precedes the number slot. Re-armed at every object opener.
    numeric_currency_slot_pending: bool,
    /// True while the walk is inside a folded-away auxiliary detail-pair area (DetailHeader/Footer),
    /// whose format records must not leak onto the real Detail area. Set at every area marker.
    in_aux_area: bool,
}

impl<'a> RdWalk<'a> {
    /// Start a walk over `tree`. The two whole-tree collections it resolves records against — the
    /// conditional-format formula bodies and the summary definitions — are gathered up front, since
    /// a record may name one that streams later.
    pub(super) fn new(tree: &[RecordNode], logical: &'a [u8], groups: &'a [Group]) -> Self {
        Self {
            areas: Vec::new(),
            logical,
            groups,
            conditions: condition_formula_bodies(tree, logical),
            sum_defs: collect_summary_defs(tree, logical),
            pending_blob_ds: None,
            pending_border_conditions: Vec::new(),
            pending_band_kind: None,
            font_set: false,
            color_set: false,
            numeric_currency_slot_pending: false,
            in_aux_area: false,
        }
    }

    /// The layout tree the walk built.
    pub(super) fn into_areas(self) -> Vec<Area> {
        self.areas
    }

    /// Consume one record. Records must be fed in stream order — see the type's own documentation.
    pub(super) fn feed(&mut self, node: &RecordNode) {
        let rd = RdRecord::classify(node);
        if rd.opens_object() {
            // A new object begins a fresh run of attribute records: re-arm the first-wins captures.
            self.font_set = false;
            self.color_set = false;
            self.numeric_currency_slot_pending = true;
        }
        match rd {
            // The layout tree: the markers that delimit areas and their sections.
            RdRecord::Area => self.open_area(node),
            RdRecord::Band(kind) => self.pending_band_kind = Some(kind),
            RdRecord::Section => self.open_section(node),
            RdRecord::SectionCodeHeaderFooter => self.apply_area_group_level(node),

            // Object openers, each starting the run of attribute records that follows it.
            RdRecord::OpenText => self.open(ReportObjectKind::Text(TextObject::default())),
            RdRecord::OpenField => self.open_field(node),
            RdRecord::OpenShape => self.open_shape(node),
            RdRecord::BlobFieldRef => self.stash_blob_field_ref(node),
            RdRecord::OpenPicture => self.open_picture(),
            RdRecord::OpenSubreport => self.open_subreport(node),
            RdRecord::OpenCrossTab => self.open(ReportObjectKind::CrossTab(
                crate::model::CrossTabObject::default(),
            )),

            // Attributes of the object the current run opened.
            RdRecord::FieldHeadingLink => self.promote_to_field_heading(node),
            RdRecord::Name => self.apply_name(node),
            RdRecord::Position => self.apply_position(node),
            RdRecord::Format => self.apply_format(node),
            RdRecord::ObjectCondition => self.apply_object_conditions(node),
            RdRecord::OleObjectItem => self.apply_ole_ordinal(node),
            RdRecord::BorderCondition => self.stash_border_conditions(node),
            RdRecord::Border => self.apply_border(node),
            RdRecord::FontColor => self.apply_font_color(node),
            RdRecord::Font => self.apply_font(node),
            RdRecord::FontCondition => self.apply_font_conditions(node),
            RdRecord::FieldFormatBlock => self.apply_field_format(node),

            // The current text object's body, run by run.
            RdRecord::TextObjectFormat => self.open_paragraph(node),
            RdRecord::EmbeddedField => self.push_embedded_field(node),
            RdRecord::TextContent => self.push_text(node),

            // Attributes of the current area or section.
            RdRecord::AreaSectionFormat if self.in_aux_area => {
                // Belongs to a folded-away auxiliary detail-pair area; drop it so it cannot
                // overwrite the real Detail area's (or its section's) format.
            }
            RdRecord::AreaSectionFormat => self.apply_area_section_format(node),
            RdRecord::SectionCondition => self.apply_section_conditions(node),

            RdRecord::Other => {}
        }
    }

    /// Read a record's content through the field table that describes it.
    fn row(&self, table: &Table, node: &RecordNode) -> Row {
        row_of(node, self.logical, table)
    }

    /// The most-recently-opened object, which the current run of attribute records decorates.
    fn object(&mut self) -> Option<&mut ReportObject> {
        current_object(&mut self.areas)
    }

    /// The current object's text body, if it is a text object.
    fn text(&mut self) -> Option<&mut TextObject> {
        match self.object().map(|o| &mut o.kind) {
            Some(ReportObjectKind::Text(t)) => Some(t),
            _ => None,
        }
    }

    /// Open an object in the current section; the attribute records that follow fill the rest in.
    fn open(&mut self, kind: ReportObjectKind) {
        open_object(&mut self.areas, kind);
    }

    /// `0x8a` — open an area.
    ///
    /// The Detail band is stored as an area *triplet* (`DetailHeaderN` / `DetailAreaN` /
    /// `DetailFooterN`) that the engine's `Areas` collection folds into the single `Detail` area,
    /// exposing that area's own format. The auxiliary Header/Footer markers are dropped, but their
    /// trailing `0x00fe` format records would otherwise leak onto whichever area is currently last —
    /// the real `DetailArea` — and clobber its `EnableHideForDrillDown`. Tracking the auxiliary span
    /// suppresses them: the auxiliary half's area-format record disagrees with the Detail area's,
    /// and the Detail area's is the one kept.
    fn open_area(&mut self, node: &RecordNode) {
        self.in_aux_area = !sections::open_area(&mut self.areas, node, self.logical);
    }

    /// `0x8c` — open a section in the current area, under the band kind its marker announced.
    fn open_section(&mut self, node: &RecordNode) {
        let band_kind = self.pending_band_kind.take();
        sections::open_section(&mut self.areas, node, self.logical, band_kind);
    }

    /// `0x9c` — the current area's group nesting level.
    ///
    /// The wrapper parents one `0x9b` record, which types the area and — for a group area — gives
    /// its 0-based nesting level; a group area sees this right after its own marker. That level is
    /// the authoritative one for ordering the group bands and scoping group summaries, independent
    /// of the area name and of the areas' binary storage order.
    fn apply_area_group_level(&mut self, node: &RecordNode) {
        /// The area type a group area states, the one that carries a nesting level.
        const GROUP_AREA_TYPE: u32 = 0x03;

        let Some(child) = node.children.first() else {
            return;
        };
        let row = self.row(&ft::SECTION_CODE_AREA_TYPE, child);
        if row.u("area_type") != GROUP_AREA_TYPE {
            return;
        }
        if let Some(area) = self.areas.last_mut() {
            area.group_level = row.get("group_level").and_then(Cell::u).map(|v| v as usize);
        }
    }

    /// `0x9f` — open a field object. A summary's group scope is the group owning the section it sits
    /// in: the 1-based nesting level of the hosting area, whose `0x9c` streams ahead of its field
    /// objects. Report/page bands carry no level, yielding a grand total.
    fn open_field(&mut self, node: &RecordNode) {
        let row = self.row(&ft::FIELD_OBJECT, node);
        let group_no = self.areas.last().and_then(|a| a.group_level).map(|l| l + 1);
        let field = field_object(&row, self.groups, &self.sum_defs, group_no);
        self.open(ReportObjectKind::Field(Box::new(field)));
    }

    /// `0xa9` — open a drawing object.
    ///
    /// The opener carries the object's authoritative geometry as absolute twips — Right then Bottom,
    /// the bottom-right corner — plus the EnableExtendToBottomOfSection flag. For boxes this
    /// geometry overrides the (occasionally inflated) Width/Height in the `0x9e` name record; lines
    /// agree. Which of the two the object is only becomes known at its `0xec` border record, so it
    /// always opens as a line.
    fn open_shape(&mut self, node: &RecordNode) {
        let row = self.row(&ft::DRAWING_OBJECT, node);
        let shape = crate::model::DrawingShape {
            right: Twips(row.u("right") as i32),
            bottom: Twips(row.u("bottom") as i32),
            line_thickness: Twips(0),
            extend_to_bottom_of_section: row.i("extend_to_bottom_of_section") != 0,
            end_section_index: row.i("end_section_index") as u16,
        };
        self.open(ReportObjectKind::Line(crate::model::LineShape { shape }));
    }

    /// `0xb1` — the database field a blob-field object's picture comes from. Stash the reference for
    /// the picture opener it wraps, which the wrapper declares as its first child.
    fn stash_blob_field_ref(&mut self, node: &RecordNode) {
        let row = self.row(&ft::BLOB_FIELD_WRAPPER, node);
        self.pending_blob_ds = Some(row.text("data_source").to_owned());
    }

    /// `0xae` — open a picture object. One wrapped by a `0xb1` is a blob-field object bound to a
    /// database field, and opens as such carrying the field reference; an unwrapped picture is a
    /// static image or a chart placeholder.
    fn open_picture(&mut self) {
        match self.pending_blob_ds.take() {
            Some(raw) if !raw.is_empty() => {
                self.open(ReportObjectKind::BlobField(crate::model::BlobFieldObject {
                    data_source: format!("{{{raw}}}"),
                }))
            }
            _ => self.open(ReportObjectKind::Picture(
                crate::model::PictureObject::default(),
            )),
        }
    }

    /// `0xa3` — open a subreport placeholder. The subreport's friendly name (SubreportName) lives in
    /// the backing `Subdocument N`, not here; the opener gives that index `N`, which `io` uses to
    /// resolve the name after the subreports are decoded.
    fn open_subreport(&mut self, node: &RecordNode) {
        let row = self.row(&ft::SUBREPORT_OBJECT, node);
        self.open(ReportObjectKind::Subreport(crate::model::SubreportObject {
            subdoc_index: row.u("subdocument_index"),
            on_demand: row.i("on_demand") != 0,
            ..Default::default()
        }));
    }

    /// `0x0166` — promote the current text object to a field-heading object, carrying its text and
    /// font color over and recording the field object it heads.
    fn promote_to_field_heading(&mut self, node: &RecordNode) {
        let row = self.row(&ft::FIELD_HEADING_LINK, node);
        let Some(name) = row.get("field_object_name").and_then(Cell::text) else {
            return;
        };
        let name = name.to_owned();
        if let Some(obj) = self.object() {
            if let ReportObjectKind::Text(t) = &obj.kind {
                obj.kind = ReportObjectKind::FieldHeading(crate::model::FieldHeadingObject {
                    field_object_name: name,
                    // Carry the full multi-line content (`display`), not just the last literal run,
                    // so multi-line headings render correctly.
                    text: if t.display.is_empty() {
                        t.text.clone()
                    } else {
                        t.display.clone()
                    },
                    max_lines: t.max_lines,
                    font_color: t.font_color.clone(),
                    reading_order: t.reading_order,
                });
            }
        }
    }

    /// `0x9e` — the current object's name and size.
    fn apply_name(&mut self, node: &RecordNode) {
        let (name, width, height) = build_object_name(node, self.logical);
        if let Some(obj) = self.object() {
            obj.name = name;
            obj.bounds.width = Twips(width);
            obj.bounds.height = Twips(height);
        }
    }

    /// `0xbe` — the current object's position.
    fn apply_position(&mut self, node: &RecordNode) {
        let row = self.row(&ft::OBJECT_POSITION, node);
        if let Some(obj) = self.object() {
            let (left, top) = build_object_pos(&row);
            obj.bounds.left = Twips(left);
            obj.bounds.top = Twips(top);
        }
    }

    /// `0xfc` — the current object's format flags.
    fn apply_format(&mut self, node: &RecordNode) {
        let row = self.row(&ft::OBJECT_FORMAT, node);
        if let Some(obj) = self.object() {
            apply_object_format(&row, obj);
        }
    }

    /// `0xfd` — the current object's conditional-format formulas.
    fn apply_object_conditions(&mut self, node: &RecordNode) {
        let row = self.row(&ft::OBJECT_FORMAT_WRAPPER, node);
        let resolved = resolve_conditions(&condition_slots(&row), &self.conditions);
        if let Some(obj) = self.object() {
            obj.format.condition_formulas.extend(resolved);
        }
    }

    /// `0xbd` — the `Embedding N` storage ordinal of a static/OLE picture. The image bytes are not
    /// in this stream but in that storage's `CONTENTS`; stash the ordinal on the just-opened picture
    /// so `io` can resolve the bytes once the container is known.
    fn apply_ole_ordinal(&mut self, node: &RecordNode) {
        let row = self.row(&ft::OLE_OBJECT_ITEM, node);
        if let Some(ReportObjectKind::Picture(pic)) = self.object().map(|o| &mut o.kind) {
            pic.ole_ordinal = row.get("embedding_ordinal").and_then(Cell::u);
        }
    }

    /// `0xed` — the border's color condition formulas (`@Fore_Color` → BorderColor, `@Back_Color` →
    /// BackgroundColor). Stash them for the `0xec` child that follows, which rebuilds the border.
    fn stash_border_conditions(&mut self, node: &RecordNode) {
        let row = self.row(&ft::BORDER_WRAPPER, node);
        self.pending_border_conditions =
            resolve_conditions(&condition_slots(&row), &self.conditions);
    }

    /// `0xec` — the current object's border, and for a drawing object its line properties and true
    /// type.
    ///
    /// The record doubles as the drawing object's line-properties carrier: it stores the line
    /// thickness in twips (0 = hairline) and the enum that authoritatively types the `0xa9` shape —
    /// `1` = box, `2` = line. That enum is what distinguishes a box from a line when their geometry
    /// is identical (a zero-height box and a horizontal line both have height 0). A box also carries
    /// its rounded-corner ellipse (0 = square corners), which the render pipeline reads to draw
    /// rounded and elliptical boxes.
    fn apply_border(&mut self, node: &RecordNode) {
        let row = self.row(&ft::BORDER, node);
        let line_thickness = Twips(row.i("line_width"));
        let shape_type = DrawingShapeKind::from_byte(row.u("shape_kind") as u8);
        let corner_ellipse_width = Twips(row.u("corner_ellipse_width") as i32);
        let corner_ellipse_height = Twips(row.u("corner_ellipse_height") as i32);
        let mut border = build_border(&row);
        border.condition_formulas = std::mem::take(&mut self.pending_border_conditions);
        // The box's own section height, to detect a box that spans past it (below).
        let section_height = self
            .areas
            .last()
            .and_then(|a| a.sections.last())
            .map(|s| s.height.0)
            .unwrap_or(0);
        let Some(obj) = self.object() else {
            return;
        };
        // Reclassify a freshly-opened drawing object (always opened as a Line at `0xa9`) to a Box
        // when the shape enum says so, carrying over its drawing properties. A box's true geometry
        // is the opener's absolute bottom-right corner (the `0x9e` size can be inflated), so derive
        // Width/Height from it.
        if shape_type == DrawingShapeKind::Box {
            if let ReportObjectKind::Line(l) = &obj.kind {
                let shape = l.shape;
                obj.kind = ReportObjectKind::Box(crate::model::BoxShape {
                    shape,
                    corner_ellipse_width,
                    corner_ellipse_height,
                });
                if shape.right.0 > 0 {
                    obj.bounds.width = Twips(shape.right.0 - obj.bounds.left.0);
                    // A cross-section box (spanning into a later section) keeps the `0x9e` height
                    // (the true span); its end section is resolved in a post-pass. Detected when the
                    // opener bottom is above the top, or the box's bottom edge (top + span) extends
                    // past its own section. A box that fits one section instead takes the opener
                    // span as its height.
                    let top = obj.bounds.top.0;
                    let spans = shape.bottom.0 < top || top + obj.bounds.height.0 > section_height;
                    if !spans {
                        obj.bounds.height = Twips(shape.bottom.0 - top);
                    }
                }
            }
        }
        match &mut obj.kind {
            ReportObjectKind::Line(s) => s.shape.line_thickness = line_thickness,
            ReportObjectKind::Box(s) => s.shape.line_thickness = line_thickness,
            _ => {}
        }
        obj.border = border;
    }

    /// `0x0100` — the current object's font color. First run wins: a multi-run text object keeps the
    /// color of its first run.
    fn apply_font_color(&mut self, node: &RecordNode) {
        if self.color_set {
            return;
        }
        let row = self.row(&ft::FONT_COLOR, node);
        let color = colorref_or_white(row.u("color"));
        if let Some(fc) = self.object().and_then(font_color_mut) {
            fc.color = color;
            self.color_set = true;
        }
    }

    /// `0x08` — a font. It styles the text run it follows (`0xc2`/`0xc4`), and — for the first run
    /// only — is also the object's own font, which is what the model reports.
    fn apply_font(&mut self, node: &RecordNode) {
        let row = self.row(&ft::FONT, node);
        let Some(font) = build_font(&row) else {
            return;
        };
        if let Some(run) = self
            .text()
            .and_then(|t| t.paragraphs.last_mut())
            .and_then(|p| p.runs.last_mut())
        {
            run.font = Some(font.clone());
        }
        if !self.font_set {
            if let Some(fc) = self.object().and_then(font_color_mut) {
                fc.font = font;
                self.font_set = true;
            }
        }
    }

    /// `0x0101` — the current object's font conditional-format formulas.
    fn apply_font_conditions(&mut self, node: &RecordNode) {
        let row = self.row(&ft::FONT_CONDITION_FORMAT, node);
        let resolved = resolve_conditions(&condition_slots(&row), &self.conditions);
        if let Some(fc) = self.object().and_then(font_color_mut) {
            fc.condition_formulas.extend(resolved);
        }
    }

    /// One of the typed field-format wrappers — decode its value child into the current field
    /// object's `FieldFormat`. Every field opener is followed by the full block, so the format
    /// becomes `Some` for every field.
    fn apply_field_format(&mut self, node: &RecordNode) {
        let Some(child) = node.children.first() else {
            return;
        };
        // Reaches for `areas` rather than `self.object()` so the currency-slot flag, a disjoint
        // field, stays borrowable alongside the object.
        let Some(ReportObjectKind::Field(f)) = current_object(&mut self.areas).map(|o| &mut o.kind)
        else {
            return;
        };
        let ff = f.format.get_or_insert_with(Default::default);
        apply_field_format_child(
            child,
            self.logical,
            ff,
            &mut self.numeric_currency_slot_pending,
        );
    }

    /// `0xc0` — open one line (paragraph) of the current text object. `0xc0`, `0xc2` (text), `0x08`
    /// (font) repeat per line; the first opens line 1, and every subsequent `0xc0` within the same
    /// object is a line break.
    ///
    /// Text and field-heading objects also carry their authoritative horizontal alignment here (the
    /// `0xfc` value is a conditional override). This record streams after `0xfc`, so it correctly
    /// supersedes it; field objects have no `0xc0`.
    fn open_paragraph(&mut self, node: &RecordNode) {
        let row = self.row(&ft::TEXT_OBJECT_FORMAT, node);
        // The line-spacing value is a 16.16 multiplier when the type says multiple, twips when it
        // says exact.
        let indent = crate::model::IndentAndSpacingFormat {
            left_indent: Twips(row.i("left_indent")),
            right_indent: Twips(row.i("right_indent")),
            first_line_indent: Twips(row.i("first_line_indent")),
            line_spacing: crate::model::LineSpacing {
                spacing_type: match row.u("line_spacing_type") {
                    1 => crate::model::LineSpacingType::Exact,
                    _ => crate::model::LineSpacingType::Multiple,
                },
                raw: row.u("line_spacing"),
            },
        };
        if let Some(t) = self.text() {
            if !t.display.is_empty() {
                t.display.push('\n');
            }
            t.paragraphs.push(crate::model::Paragraph {
                indent,
                ..Default::default()
            });
        }
        if let (Some(obj), Some(a)) = (
            self.object(),
            row.get("horizontal_alignment").and_then(Cell::u),
        ) {
            obj.format.horizontal_alignment = Alignment::from_code(a as i32);
        }
    }

    /// `0xc4` — a field reference embedded in the current text object.
    ///
    /// The record opens with the same composite a field object does — the display text, the pool it
    /// names and the index within that pool — so the reference renders inline exactly as a field
    /// object's `DataSource` does, and the raw `alias.name` / `@formula` / `?param` reference is
    /// recorded alongside.
    fn push_embedded_field(&mut self, node: &RecordNode) {
        let row = self.row(&ft::TEXT_EMBEDDED_FIELD, node);
        let raw = row.text("data_source").to_owned();
        if raw.is_empty() {
            return;
        }
        let handle = row.get("data_source").and_then(Cell::handle);
        let kind = handle
            .map(|(pool, _)| FieldRefKind::from_code(pool as u8))
            .unwrap_or_default();
        // A special field's specific kind is the low half of the reference's index.
        let code = handle.map(|(_, index)| index.unwrap_or(UNSET_FIELD_INDEX) as u8);
        // A GroupName special field embedded in text renders as the engine's SDK form
        // `GroupName ({condition field})`, not the internal `Group #N Name` display reference.
        // Recover the 1-based group number from that reference (its sole ASCII digit run) so the
        // rendering reconstructs the SDK form, matching the field-object opener path.
        let group_display = matches!(kind, FieldRefKind::GroupName)
            .then(|| group_display_number(&raw))
            .flatten();
        let rendered = field_data_source(kind, &raw, self.groups, group_display, code, None, None);
        if let Some(t) = self.text() {
            t.display.push_str(&rendered);
            push_text_run(
                t,
                crate::model::TextRun {
                    text: rendered,
                    field_ref: Some(raw.clone()),
                    font: None,
                    // The record stores a spacing of its own, in the slot a literal run stores one;
                    // the model does not carry it yet.
                    character_spacing: Twips(0),
                },
            );
            t.embedded_fields.push(raw);
        }
    }

    /// `0xc2` — a literal run of the current text object's text.
    fn push_text(&mut self, node: &RecordNode) {
        let row = self.row(&ft::TEXT_OBJECT, node);
        let text = row.text("text").to_owned();
        let character_spacing = Twips(row.i("character_spacing"));
        if let Some(t) = self.text() {
            t.display.push_str(&text);
            push_text_run(
                t,
                crate::model::TextRun {
                    text: text.clone(),
                    field_ref: None,
                    font: None,
                    character_spacing,
                },
            );
        }
        if let Some(obj) = self.object() {
            set_object_text(&mut obj.kind, text);
        }
    }

    /// `0xfe` — the format flags of the current section or of its area; the record states which.
    fn apply_area_section_format(&mut self, node: &RecordNode) {
        let row = self.row(&ft::AREA_SECTION_FORMAT, node);
        if is_section_format(&row) {
            if let Some(sec) = current_section(&mut self.areas) {
                sec.format = decode_section_format(&row);
            }
        } else if let Some(area) = self.areas.last_mut() {
            let group = area.format.group; // GroupAreaFormat is not in this record; keep it.
            area.format = decode_area_format(&row);
            area.format.group = group;
        }
    }

    /// `0xff` — the current section's conditional-format formulas.
    fn apply_section_conditions(&mut self, node: &RecordNode) {
        let row = self.row(&ft::SECTION_FORMAT_WRAPPER, node);
        let resolved = resolve_conditions(&condition_slots(&row), &self.conditions);
        if let Some(sec) = current_section(&mut self.areas) {
            sec.condition_formulas.extend(resolved);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_model::enhanced_header_byte;
    use crate::codec::tslv::StringFormat;
    use crate::field_table::cursor::{Encoder, Piece};
    use crate::model::Color;
    use crate::model::Font;
    use crate::model::FontColor;
    use crate::records::rtype::*;

    /// A synthetic report-definition stream, appended to in the order the walk will see it.
    ///
    /// Every node points into the one `logical` buffer they share, so a test can feed the same
    /// records in a different order and state what that changes — which is the whole of what these
    /// rules say.
    #[derive(Default)]
    struct Stream {
        logical: Vec<u8>,
    }

    /// The header bytes each synthetic record reserves. Only the first is read back — a record
    /// declares the wire form its own strings are framed in there.
    const HEADER_LEN: usize = 4;

    impl Stream {
        /// A childless record of `rtype`, its field bytes written by `fill`.
        fn record(&mut self, rtype: u16, fill: impl FnOnce(&mut Encoder)) -> RecordNode {
            let offset = self.header(rtype);
            let content_start = self.logical.len();
            self.content(fill);
            RecordNode {
                rtype,
                schema: 0x0700,
                offset,
                content_start,
                content_end: self.logical.len(),
                mask: 0,
                children: Vec::new(),
            }
        }

        /// A record of `rtype` opening with one nested record of `child_rtype`, then its own field
        /// bytes — the shape of every record whose table declares a child.
        fn nested(
            &mut self,
            rtype: u16,
            child_rtype: u16,
            child_fill: impl FnOnce(&mut Encoder),
            fill: impl FnOnce(&mut Encoder),
        ) -> RecordNode {
            let offset = self.header(rtype);
            let content_start = self.logical.len();
            let child = self.record(child_rtype, child_fill);
            self.content(fill);
            RecordNode {
                rtype,
                schema: 0x0700,
                offset,
                content_start,
                content_end: self.logical.len(),
                mask: 0,
                children: vec![child],
            }
        }

        /// Reserve a record header declaring the enhanced string form; returns its offset.
        fn header(&mut self, rtype: u16) -> usize {
            let offset = self.logical.len();
            self.logical.extend_from_slice(&[0u8; HEADER_LEN]);
            self.logical[offset] = enhanced_header_byte(rtype, 0);
            offset
        }

        /// Append the field bytes `fill` writes, framed as this stream declares.
        fn content(&mut self, fill: impl FnOnce(&mut Encoder)) {
            let mut enc = Encoder::with_strings(StringFormat::Enhanced);
            fill(&mut enc);
            for piece in enc.finish() {
                if let Piece::Run(bytes) = piece {
                    self.logical.extend_from_slice(&bytes);
                }
            }
        }

        /// An area marker (`0x8a`) named `name` — the name the area's initial kind is guessed from.
        fn area(&mut self, name: &str) -> RecordNode {
            self.record(AREA, |e| {
                e.i32_be(0);
                e.u16_be(1);
                e.string(name.as_bytes());
            })
        }

        /// A section marker (`0x8c`) of `height` twips.
        fn section(&mut self, height: i32) -> RecordNode {
            self.record(SECTION, |e| e.i32_be(height))
        }

        /// A font record (`0x08`) naming `face` at ten points.
        fn font(&mut self, face: &str) -> RecordNode {
            self.record(FONT, |e| {
                e.string(face.as_bytes());
                e.narrowing(1, 0); // family
                e.narrowing(1, 0); // pitch
                e.narrowing(1, 0);
                e.u16_be(10); // size, in whole points
            })
        }

        /// A font-colour record (`0x0100`) carrying one `COLORREF`.
        fn font_color(&mut self, colorref: u32) -> RecordNode {
            self.record(FONT_COLOR, |e| e.u32_be(colorref))
        }

        /// A field-object opener (`0x9f`), which every table-declared opener leads with its
        /// `ObjectName` child.
        fn field_object(&mut self, data_source: &str) -> RecordNode {
            self.nested(
                FIELD_OBJECT,
                OBJECT_NAME,
                |_| {},
                |e| {
                    e.string(data_source.as_bytes());
                    e.narrowing(1, 0); // the pool the reference names
                    e.u16_be(0); // the index within it
                    e.u16_be(0); // old highlight count
                },
            )
        }

        /// One numeric field-format block (`0xf9` wrapping `0xf8`) spelling `decimal_places`.
        fn numeric_format(&mut self, decimal_places: u16) -> RecordNode {
            self.nested(
                NUMERIC_FIELD_FORMAT_WRAPPER,
                NUMERIC_FIELD_FORMAT,
                move |e| {
                    e.i16_be(0); // suppress if zero
                    e.narrowing(1, 0); // negative type
                    e.i16_be(0); // thousands separator
                    e.i16_be(0); // leading zero
                    e.u16_be(decimal_places);
                },
                |_| {},
            )
        }

        /// An area format record (`0xfe`) — `is_section` is `0`, so it formats the area itself.
        fn area_format(&mut self, visible: bool) -> RecordNode {
            self.record(AREA_SECTION_FORMAT, move |e| {
                e.narrowing(1, 0); // area kind
                e.i16_be(0); // is header
                e.i16_be(0); // is section
                e.i16_be(i16::from(visible));
            })
        }
    }

    /// Feed `nodes` to a fresh walk in the given order and return the tree it built.
    fn walk(logical: &[u8], nodes: &[&RecordNode]) -> Vec<Area> {
        let mut w = RdWalk::new(&[], logical, &[]);
        for node in nodes {
            w.feed(node);
        }
        w.into_areas()
    }

    /// The one object the walk placed.
    fn only_object(areas: &[Area]) -> &ReportObject {
        let sections = &areas.first().expect("one area").sections;
        &sections.first().expect("one section").objects[0]
    }

    /// The font/colour pair an object reports as its own.
    fn font_color_of(obj: &ReportObject) -> &FontColor {
        match &obj.kind {
            ReportObjectKind::Text(t) => &t.font_color,
            ReportObjectKind::Field(f) => &f.font_color,
            ReportObjectKind::FieldHeading(h) => &h.font_color,
            _ => panic!("this object kind carries no font"),
        }
    }

    fn font_of(obj: &ReportObject) -> &Font {
        &font_color_of(obj).font
    }

    fn color_of(obj: &ReportObject) -> Color {
        font_color_of(obj).color
    }

    /// The format block the field-format records built on a field object.
    fn format_of(obj: &ReportObject) -> &crate::model::FieldFormat {
        match &obj.kind {
            ReportObjectKind::Field(f) => f.format.as_ref().expect("a format block was decoded"),
            _ => panic!("not a field object"),
        }
    }

    /// A text object stores one font per run and the object's own font is the FIRST run's, so a
    /// later run's font does not replace it. Fed the other way round, the other face wins — the rule
    /// is about arrival order and nothing else.
    #[test]
    fn the_first_font_after_an_opener_is_the_object_font() {
        let mut s = Stream::default();
        let (area, section) = (s.area("ReportHeaderArea1"), s.section(320));
        let text = s.record(TEXT_OBJECT_CONTAINER, |_| {});
        let (first, second) = (s.font("Alpha"), s.font("Beta"));

        let areas = walk(&s.logical, &[&area, &section, &text, &first, &second]);
        assert_eq!(font_of(only_object(&areas)).name, "Alpha");

        let areas = walk(&s.logical, &[&area, &section, &text, &second, &first]);
        assert_eq!(font_of(only_object(&areas)).name, "Beta");
    }

    /// A new opener re-arms the capture: the next object takes its own first font, not the previous
    /// object's.
    #[test]
    fn an_opener_re_arms_the_font_capture() {
        let mut s = Stream::default();
        let (area, section) = (s.area("ReportHeaderArea1"), s.section(320));
        let (one, two) = (
            s.record(TEXT_OBJECT_CONTAINER, |_| {}),
            s.record(TEXT_OBJECT_CONTAINER, |_| {}),
        );
        let (first, second) = (s.font("Alpha"), s.font("Beta"));

        let areas = walk(&s.logical, &[&area, &section, &one, &first, &two, &second]);
        let objects = &areas[0].sections[0].objects;
        assert_eq!(font_of(&objects[0]).name, "Alpha");
        assert_eq!(font_of(&objects[1]).name, "Beta");
    }

    /// The colour record streams once per run too, and likewise the first after an opener wins.
    #[test]
    fn the_first_font_colour_after_an_opener_is_the_object_colour() {
        const RED: u32 = 0x0000_00ff;
        const BLUE: u32 = 0x00ff_0000;
        let mut s = Stream::default();
        let (area, section) = (s.area("ReportHeaderArea1"), s.section(320));
        let text = s.record(TEXT_OBJECT_CONTAINER, |_| {});
        let (red, blue) = (s.font_color(RED), s.font_color(BLUE));

        let areas = walk(&s.logical, &[&area, &section, &text, &red, &blue]);
        assert_eq!(
            color_of(only_object(&areas)),
            Color {
                a: 255,
                r: 255,
                g: 0,
                b: 0
            }
        );

        let areas = walk(&s.logical, &[&area, &section, &text, &blue, &red]);
        assert_eq!(
            color_of(only_object(&areas)),
            Color {
                a: 255,
                r: 0,
                g: 0,
                b: 255
            }
        );
    }

    /// A field streams the numeric format twice — the currency slot first, then the number slot —
    /// and the two are told apart by that order alone. The number slot is the field's reported
    /// format, so it overwrites the first; the currency slot keeps what came before it.
    #[test]
    fn the_numeric_currency_slot_is_the_one_before_the_number_slot() {
        let mut s = Stream::default();
        let (area, section) = (s.area("ReportHeaderArea1"), s.section(320));
        let field = s.field_object("Table.amount");
        let (currency, number) = (s.numeric_format(3), s.numeric_format(1));

        let areas = walk(&s.logical, &[&area, &section, &field, &currency, &number]);
        let format = format_of(only_object(&areas));
        assert_eq!(format.currency_numeric.decimal_places, 3);
        assert_eq!(format.numeric.decimal_places, 1);

        // Fed the other way round the two slots swap, which is what makes the order the rule.
        let areas = walk(&s.logical, &[&area, &section, &field, &number, &currency]);
        let format = format_of(only_object(&areas));
        assert_eq!(format.currency_numeric.decimal_places, 1);
        assert_eq!(format.numeric.decimal_places, 3);
    }

    /// A lone numeric record fills both slots: the currency slot is still pending when the number
    /// slot's value arrives.
    #[test]
    fn a_single_numeric_record_fills_both_slots() {
        let mut s = Stream::default();
        let (area, section) = (s.area("ReportHeaderArea1"), s.section(320));
        let field = s.field_object("Table.amount");
        let only = s.numeric_format(4);

        let areas = walk(&s.logical, &[&area, &section, &field, &only]);
        let format = format_of(only_object(&areas));
        assert_eq!(format.currency_numeric.decimal_places, 4);
        assert_eq!(format.numeric.decimal_places, 4);
    }

    /// The detail band is stored as an area triplet whose auxiliary halves are folded away. Their
    /// format records must fold away with them: left alone they would land on whichever area is
    /// currently last — the real Detail area — and state the opposite of what it stores.
    #[test]
    fn a_folded_away_detail_area_does_not_format_the_area_before_it() {
        let mut s = Stream::default();
        let detail = s.area("DetailArea1");
        let visible = s.area_format(true);
        let aux = s.area("DetailFooter1");
        let hidden = s.area_format(false);

        let areas = walk(&s.logical, &[&detail, &visible, &aux, &hidden]);
        assert_eq!(
            areas.len(),
            1,
            "the auxiliary half opens no area of its own"
        );
        assert!(!areas[0].format.base.suppress);

        // The same record outside an auxiliary span does reach the area, so what suppresses it is
        // the span and not the record.
        let areas = walk(&s.logical, &[&detail, &visible, &hidden]);
        assert!(areas[0].format.base.suppress);
    }

    /// A band marker announces the kind of the section that FOLLOWS it: the area/section name is
    /// user-renameable (a group band is commonly named after its group field), so the marker is the
    /// authority. A section opened before any marker keeps the name-derived guess.
    #[test]
    fn a_band_marker_names_the_section_that_follows_it() {
        let mut s = Stream::default();
        let area = s.area("nameHeader"); // reads as a report header by name alone
        let unmarked = s.section(320);
        let marker = s.record(GROUP_FOOTER_BAND, |_| {});
        let marked = s.section(480);

        let areas = walk(&s.logical, &[&area, &unmarked, &marker, &marked]);
        let sections = &areas[0].sections;
        assert_eq!(sections[0].kind, AreaSectionKind::ReportHeader);
        assert_eq!(sections[1].kind, AreaSectionKind::GroupFooter);
        assert_eq!(areas[0].kind, AreaSectionKind::GroupFooter);
    }

    /// One marker names one section: the kind is taken, not left standing for every section after
    /// it.
    #[test]
    fn a_band_marker_is_consumed_by_the_section_it_names() {
        let mut s = Stream::default();
        let first = s.area("nameHeader");
        let marker = s.record(GROUP_FOOTER_BAND, |_| {});
        let marked = s.section(320);
        let second = s.area("PageHeaderArea1");
        let unmarked = s.section(480);

        let areas = walk(&s.logical, &[&first, &marker, &marked, &second, &unmarked]);
        assert_eq!(areas[0].sections[0].kind, AreaSectionKind::GroupFooter);
        assert_eq!(areas[1].sections[0].kind, AreaSectionKind::PageHeader);
    }
}
