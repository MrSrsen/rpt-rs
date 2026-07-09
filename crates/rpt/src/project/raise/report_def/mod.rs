//! Report definition — the area/section/object layout tree. This module root holds the record
//! grammar ([`RdRecord`]) and the top-level walk ([`raise_report_definition`]) that orchestrates it;
//! the submodules own the parsing detail:
//!
//! - [`sections`] — area/section construction and the canonical band ordering.
//! - [`objects`] — object cursors and the post-walk object-tree transforms.
//! - [`summary`] — the non-running-total summary definitions a Summary object indexes.
//! - [`grid`] / [`chart`] / [`crosstab`] — chart / cross-tab bindings, chart definitions, cross-tab
//!   dimensions + grid formatting.
//! - [`formats`] (per-object/area/section format records), [`conditions`] (conditional-format
//!   formula slots), [`data_source`] (field/object reference text).

use super::*;

mod chart;
mod conditions;
mod crosstab;
mod data_source;
mod formats;
mod grid;
mod objects;
mod sections;
mod summary;

use chart::*;
use conditions::*;
use crosstab::*;
use data_source::*;
use formats::*;
use grid::*;
use objects::*;
use sections::*;
use summary::*;

// `condition_formula_bodies` is also consumed by the data-definition raise (a sibling module), so
// re-export it up to the parent `raise` module.
pub(in crate::project::raise) use conditions::condition_formula_bodies;
// The parent `raise` runs `resolve_heading_alignment` after the database is decoded, so re-export it.
pub(in crate::project::raise) use objects::resolve_heading_alignment;

/// A record's role in the `ReportDefinition` stream. The stream is a flat, ordered sequence:
/// area/section markers delimit the layout tree, an *opener* starts a report object, and the
/// *attribute* records that follow decorate the most-recently-opened object until the next opener.
pub(super) enum RdRecord {
    /// `0x8a` — opens an area (`ReportHeaderArea1`, `DetailArea1`, …).
    Area,
    /// `0x8c` — opens a section within the current area.
    Section,
    /// `0xa5` — opens a text object. Whether it is a field-heading object is only known once (and
    /// if) its [`RdRecord::FieldHeadingLink`] record is seen, so it always opens as a text object.
    OpenText,
    /// `0x0166` — names the FieldObject this text object is the heading for (promotes it to a
    /// FieldHeadingObject). A plain text object has no such record.
    FieldHeadingLink,
    /// `0x9f` — opens a field object.
    OpenField,
    /// `0xa9` — opens a drawing object (line or box, told apart by geometry).
    OpenShape,
    /// `0xae` — opens a picture/OLE object.
    OpenPicture,
    /// `0xb1` — wraps the picture opener of a blob-field object; its leaf bytes hold the bound
    /// database field reference (`table.field`). Streams immediately before its `0xae` child.
    BlobFieldRef,
    /// `0xbd` — decorates the just-opened static/OLE picture; leaf `[0..4]` (big-endian) is the
    /// 1-based `Embedding N` storage ordinal whose `CONTENTS` stream holds the image bytes.
    OleObjectItem,
    /// `0xa3` — opens a subreport placeholder object.
    OpenSubreport,
    /// `0xb8` — opens a cross-tab object (wrapped by `0xb9`; the `0x9e` name nests inside it).
    OpenCrossTab,
    /// `0x9e` — the current object's Name + Width/Height.
    Name,
    /// `0xbe` — the current object's Left/Top.
    Position,
    /// `0xfc` — the current object's format flags (horizontal alignment).
    Format,
    /// `0xfd` — the current object's conditional-format formula slot array.
    ObjectCondition,
    /// `0xfe` — the current area's or section's format flags (discriminated by byte 4).
    AreaSectionFormat,
    /// `0xff` — the current section's conditional-format formula slot array.
    SectionCondition,
    /// `0x0101` — the current object's font conditional-format formula slot array.
    FontCondition,
    /// `0xec` — the current object's border styles + border/background colours.
    Border,
    /// `0xed` — wrapper parenting `0xec`; carries the border's colour condition-formula slots
    /// (`@Fore_Color` → BorderColor, `@Back_Color` → BackgroundColor). Visited before its `0xec`
    /// child, so it stashes pending conditions that the `Border` arm attaches after rebuilding.
    BorderCondition,
    /// `0x0100` — the current object's font colour.
    FontColor,
    /// `0x08` — the current object's font.
    Font,
    /// `0xc0` — a text/heading object's paragraph format (the authoritative horizontal alignment).
    TextObjectFormat,
    /// `0xc4` — an embedded field/formula/parameter reference inside the current text object.
    EmbeddedField,
    /// `0xc2` — the current text object's literal text.
    TextContent,
    /// One of the typed field-format wrappers (`0xf1`/`0xf9`/`0xef`/`0xfb` and the runtime-resolved
    /// `0xf3`/`0xf7`/`0xf5`). Each parents its value child; the byte-derived children populate the
    /// current field object's `FieldFormat`.
    FieldFormatBlock,
    /// Any record not part of the object layout.
    Other,
}

impl RdRecord {
    /// Classify a record by its type.
    fn classify(node: &RecordNode) -> RdRecord {
        match node.rtype {
            AREA_MARKER => RdRecord::Area,
            SECTION_MARKER => RdRecord::Section,
            TEXT_OBJECT => RdRecord::OpenText,
            FIELD_HEADING_LINK => RdRecord::FieldHeadingLink,
            FIELD_OBJECT => RdRecord::OpenField,
            LINE_OBJECT => RdRecord::OpenShape,
            PICTURE_OBJECT => RdRecord::OpenPicture,
            BLOB_FIELD_REF => RdRecord::BlobFieldRef,
            OLE_OBJECT_ITEM => RdRecord::OleObjectItem,
            SUBREPORT_OBJECT => RdRecord::OpenSubreport,
            CROSSTAB_OBJECT => RdRecord::OpenCrossTab,
            OBJECT_NAME => RdRecord::Name,
            OBJECT_POS => RdRecord::Position,
            OBJECT_FORMAT => RdRecord::Format,
            OBJECT_COND => RdRecord::ObjectCondition,
            AREA_SECTION_FORMAT => RdRecord::AreaSectionFormat,
            SECTION_COND => RdRecord::SectionCondition,
            FONT_COND => RdRecord::FontCondition,
            OBJECT_BORDER => RdRecord::Border,
            OBJECT_BORDER_COND => RdRecord::BorderCondition,
            FONT_COLOR => RdRecord::FontColor,
            FONT => RdRecord::Font,
            TEXT_OBJECT_FORMAT => RdRecord::TextObjectFormat,
            TEXT_EMBEDDED_FIELD => RdRecord::EmbeddedField,
            TEXT_CONTENT => RdRecord::TextContent,
            FF_COMMON_WRAPPER | FF_NUMERIC_WRAPPER | FF_BOOLEAN_WRAPPER | FF_STRING_WRAPPER
            | FF_DATE_WRAPPER | FF_TIME_WRAPPER | FF_DATETIME_WRAPPER => RdRecord::FieldFormatBlock,
            _ => RdRecord::Other,
        }
    }
}

/// The kind of a `0xa9` drawing object, taken from byte 25 of its `0xec` border record. Crystal's
/// drawing primitives are exactly lines and boxes, so this byte is `1` (box) or `2` (line); `Other`
/// captures any other value (treated as the `0xa9` default, a line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrawingShapeKind {
    Box,
    Line,
    Other(u8),
}

impl DrawingShapeKind {
    fn from_byte(b: u8) -> Self {
        match b {
            1 => DrawingShapeKind::Box,
            2 => DrawingShapeKind::Line,
            other => DrawingShapeKind::Other(other),
        }
    }
}

/// SDK `ReportDefinition`: the area / section / object layout tree, projected from the flat
/// record stream. See [`RdRecord`] for the record grammar.
pub(super) fn raise_report_definition(
    tree: &[RecordNode],
    logical: &[u8],
    groups: &[Group],
) -> ReportDefinition {
    let mut areas: Vec<Area> = Vec::new();
    // The bound field reference of a blob-field object: set by its `0xb1` wrapper, consumed by the
    // `0xae` picture opener that immediately follows (the wrapper is the opener's parent record).
    let mut pending_blob_ds: Option<String> = None;
    // Border colour condition formulas read from a `0xed` wrapper, awaiting the `0xec` border it
    // parents (the border is rebuilt fresh by the `Border` arm, so conditions can only be attached
    // after that). One wrapper precedes each border, so this is consumed before the next is set.
    let mut pending_border_conditions: Vec<(String, String)> = Vec::new();
    // Conditional-format formula bodies, keyed by global formula index; an object/section's
    // condition-slot record names the exact body by that index below.
    let conditions = condition_formula_bodies(tree, logical);
    // A text object stores one font per text run (`0x08`), interleaved with the run text (`0xc2`).
    // The engine reports the object's font as the FIRST run's font, so once an object has received a
    // font we ignore later runs. Reset on every object opener.
    let mut font_set = false;
    // True while the walk is inside a folded-away auxiliary detail-pair area (DetailHeader/Footer),
    // whose format records must not leak onto the real Detail area. Set per Area marker.
    let mut in_aux_area = false;
    let sum_defs = collect_summary_defs(tree, logical);
    for node in flatten(tree) {
        let rd = RdRecord::classify(node);
        // Each object opener begins a fresh object: re-arm first-font capture.
        if matches!(
            rd,
            RdRecord::OpenText
                | RdRecord::OpenField
                | RdRecord::OpenShape
                | RdRecord::OpenPicture
                | RdRecord::OpenSubreport
                | RdRecord::OpenCrossTab
        ) {
            font_set = false;
        }
        match rd {
            RdRecord::Area => {
                // The Detail band is stored as an area *triplet* (`DetailHeaderN` / `DetailAreaN` /
                // `DetailFooterN`); the engine's `Areas` collection folds it into the single `Detail`
                // area, exposing that area's own format. `open_area` drops the auxiliary
                // Header/Footer markers, but their trailing `0x00fe` format records would otherwise
                // leak onto the real `DetailArea` (whichever area is currently `last`) and clobber
                // its `EnableHideForDrillDown`. Track the auxiliary span so those records are ignored:
                // the auxiliary half carries an area-format record disagreeing with the Detail area,
                // and the Detail area's is the one kept.
                in_aux_area = !open_area(&mut areas, node, logical);
            }
            RdRecord::Section => open_section(&mut areas, node, logical),
            RdRecord::OpenText => {
                open_object(&mut areas, ReportObjectKind::Text(TextObject::default()));
            }
            RdRecord::FieldHeadingLink => {
                // Promote the current text object to a field-heading object, carrying its text and
                // font colour over and recording the FieldObject it heads.
                if let Some(name) = first_string(node, logical) {
                    if let Some(obj) = current_object(&mut areas) {
                        if let ReportObjectKind::Text(t) = &obj.kind {
                            obj.kind =
                                ReportObjectKind::FieldHeading(crate::model::FieldHeadingObject {
                                    field_object_name: name,
                                    // Carry the full multi-line content (`display`), not just the
                                    // last literal run, so multi-line headings render correctly.
                                    text: if t.display.is_empty() {
                                        t.text.clone()
                                    } else {
                                        t.display.clone()
                                    },
                                    max_lines: t.max_lines,
                                    font_color: t.font_color.clone(),
                                });
                        }
                    }
                }
            }
            RdRecord::OpenField => {
                // The object opener is `[u32 length][NUL-terminated reference][kind byte]…`. The
                // raw reference is the engine's display form; the kind byte selects how the engine
                // renders the `DataSource` (a `{…}` reference, a summary, a special field, …).
                let mut raw = object_data_string(node, logical)
                    .or_else(|| first_string(node, logical))
                    .unwrap_or_default();
                let lb = node.leaf_bytes(logical);
                let p = lb.get(3).map(|len| 4 + *len as usize).unwrap_or(0);
                let kind = lb
                    .get(p)
                    .map(|c| FieldRefKind::from_code(*c))
                    .unwrap_or_default();
                // A special field's specific kind is the byte at `p+2` (its display string is
                // localized, so the code — not the string — is authoritative). For a GroupName it is
                // the 0-based group index; for a Summary it is the index into `sum_defs`.
                let code = lb.get(p + 2).copied();
                // The 1-based group number for a GroupName object. Its authoritative source is the
                // leaf's own display reference (`Group #N Name`), bytes `4..p`: every locale embeds
                // the group number as the sole ASCII digit run there. The ObjectName *child* (`raw`)
                // is user-renameable and the opener's `code` byte is not the group index, so neither
                // is reliable. Digits are ASCII even when the surrounding text is not, so scan the raw
                // name bytes (read_lp_string would reject the non-ASCII form outright).
                let group_display = matches!(kind, FieldRefKind::GroupName)
                    .then(|| {
                        lb.get(4..p)
                            .and_then(|n| std::str::from_utf8(n).ok().map(str::to_owned))
                            .or_else(|| {
                                lb.get(4..p)
                                    .map(|n| String::from_utf8_lossy(n).into_owned())
                            })
                    })
                    .flatten()
                    .as_deref()
                    .and_then(group_display_number);
                // A localized Summary reference can't be parsed from the (non-ASCII) display string;
                // recover its engine display form ("Op of field") from the indexed summary def.
                if matches!(kind, FieldRefKind::Summary) && !raw.contains(" of ") {
                    if let Some((op, operand, _)) = code.and_then(|c| sum_defs.get(c as usize)) {
                        raw = format!("{} of {operand}", summary_op_token(*op));
                    }
                }
                // A summary's group scope is the group owning the section it sits in — its canonical
                // level (not the raw area suffix). Report/page bands yield none (a grand total). The
                // level map is derived from the areas built so far, which — because a group's header
                // always precedes its own fields — holds the entry every group-band lookup needs.
                let group_no = match areas.last().map(|a| (a.kind, trailing_digits(&a.name))) {
                    Some((AreaSectionKind::GroupHeader | AreaSectionKind::GroupFooter, suffix)) => {
                        canonical_group_levels(&areas).get(&suffix).copied()
                    }
                    _ => None,
                };
                // For a summary object, carry its definition index (dedup identity for
                // `<SummaryFields>`) and result value type (from the indexed `0x7e` def's child).
                let summary_code = matches!(kind, FieldRefKind::Summary)
                    .then(|| code.map(u16::from))
                    .flatten();
                let value_type = summary_code
                    .and_then(|c| sum_defs.get(c as usize))
                    .map(|d| d.2)
                    .unwrap_or_default();
                open_object(
                    &mut areas,
                    ReportObjectKind::Field(FieldObject {
                        data_source: field_data_source(
                            kind,
                            &raw,
                            groups,
                            group_display,
                            code,
                            group_no,
                        ),
                        ref_kind: kind,
                        value_type,
                        summary_code,
                        ..Default::default()
                    }),
                );
            }
            RdRecord::OpenShape => {
                // The `0xa9` opener carries the drawing object's authoritative geometry as absolute
                // twips: after a u16 object index come two coordinates — Right then Bottom (the
                // bottom-right corner) — each in the variable-width `read_coord` encoding (4 bytes
                // when > 0x7fff, else 2). A trailing 2-byte block follows, whose second byte is the
                // EnableExtendToBottomOfSection flag. For boxes this geometry overrides the
                // (occasionally inflated) Width/Height in the `0x9e` name record; lines agree.
                let b = node.leaf_bytes(logical);
                let shape = (|| {
                    let (right, n1) = read_coord(&b, 2)?;
                    let (bottom, n2) = read_coord(&b, n1)?;
                    Some(crate::model::DrawingShape {
                        right: Twips(right),
                        bottom: Twips(bottom),
                        extend_to_bottom_of_section: b.get(n2 + 1).copied().unwrap_or(0) != 0,
                        ..Default::default()
                    })
                })()
                .unwrap_or_default();
                open_object(
                    &mut areas,
                    ReportObjectKind::Line(crate::model::LineShape {
                        shape,
                        ..Default::default()
                    }),
                );
            }
            RdRecord::BlobFieldRef => {
                // The wrapper's own leaf holds the bound field reference (`table.field`); stash it
                // for the picture opener that follows. (The opener's `ObjectName` child is excluded
                // by `object_data_string`, which reads only the node's own leaf bytes.)
                pending_blob_ds = object_data_string(node, logical);
            }
            RdRecord::OpenPicture => {
                // A picture wrapped by a `0xb1` is a blob-field object bound to a database field;
                // open it as such (carrying the field reference) so it can be counted toward
                // `Field.UseCount`. An unwrapped picture is a static image or a chart placeholder.
                match pending_blob_ds.take() {
                    Some(raw) if !raw.is_empty() => open_object(
                        &mut areas,
                        ReportObjectKind::BlobField(crate::model::BlobFieldObject {
                            data_source: format!("{{{raw}}}"),
                        }),
                    ),
                    _ => open_object(
                        &mut areas,
                        ReportObjectKind::Picture(crate::model::PictureObject::default()),
                    ),
                }
            }
            // The static/OLE picture's image bytes are not in this stream: they live in the
            // top-level `Embedding N` storage's `CONTENTS` stream. This record carries that
            // ordinal `N` (leaf `[0..4]`, big-endian); stash it on the just-opened picture so
            // `io` can resolve the bytes into `PictureObject.data` once the container is known.
            RdRecord::OleObjectItem => {
                if let Some(ReportObjectKind::Picture(pic)) =
                    current_object(&mut areas).map(|o| &mut o.kind)
                {
                    if let Some(ord) = u32_be(&node.leaf_bytes(logical), 0) {
                        pic.ole_ordinal = Some(ord);
                    }
                }
            }
            // The subreport's friendly name (SubreportName) lives in the backing `Subdocument N`
            // (its report-header record), not here; the opener's leaf bytes `[0..4]` (big-endian)
            // give that index `N`, which `io` uses to resolve the name after subreports are decoded.
            RdRecord::OpenSubreport => {
                let subdoc_index = u32_be(&node.leaf_bytes(logical), 0).unwrap_or(0);
                // Byte 7 of the opener is the EnableOnDemand flag (1 = on-demand, 0 = in-place).
                let on_demand = node.leaf_bytes(logical).get(7).is_some_and(|&b| b != 0);
                open_object(
                    &mut areas,
                    ReportObjectKind::Subreport(crate::model::SubreportObject {
                        subdoc_index,
                        on_demand,
                        ..Default::default()
                    }),
                );
            }
            RdRecord::OpenCrossTab => {
                // The `0xb8` opener carries the grid's row/column/summary bindings (decoded by
                // the derived analytics for UseCount); here it opens a typed marker so the following name,
                // geometry, border and format records decorate it like any other object.
                open_object(
                    &mut areas,
                    ReportObjectKind::CrossTab(crate::model::CrossTabObject::default()),
                );
            }
            RdRecord::Name => {
                if let Some(obj) = current_object(&mut areas) {
                    let (name, width, height) = raise_object_name(node, logical);
                    obj.name = name;
                    obj.bounds.width = Twips(width);
                    obj.bounds.height = Twips(height);
                }
            }
            RdRecord::Position => {
                if let Some(obj) = current_object(&mut areas) {
                    let (left, top) = raise_object_pos(node, logical).unwrap_or((0, 0));
                    obj.bounds.left = Twips(left);
                    obj.bounds.top = Twips(top);
                }
            }
            RdRecord::Format => {
                let lb = node.leaf_bytes(logical);
                if let (Some(obj), Some(align)) = (current_object(&mut areas), lb.get(2).copied()) {
                    obj.format.horizontal_alignment = Alignment::from_code(i32::from(align));
                    // Byte 1 is EnableSuppress, stored inverted (0 = suppressed, 1 = shown).
                    obj.format.suppress.value = lb.get(1).is_some_and(|&b| b == 0);
                    // Byte 5 is EnableKeepTogether (1 = keep together), byte 9 EnableCanGrow. A
                    // cross-tab ignores the object-level flag (byte5=1 like others, yet the engine
                    // reports False), so leave its keep-together at the default (false).
                    if !matches!(obj.kind, ReportObjectKind::CrossTab(_)) {
                        obj.format.keep_together = lb.get(5).is_none_or(|&b| b != 0);
                    }
                    obj.format.can_grow = lb.get(9).is_some_and(|&b| b != 0);
                    obj.format.hyperlink = decode_hyperlink(&lb);
                }
            }
            RdRecord::ObjectCondition => {
                let resolved = resolve_conditions(&condition_refs(node, logical), &conditions);
                if let Some(obj) = current_object(&mut areas) {
                    obj.format.condition_formulas.extend(resolved);
                }
            }
            RdRecord::AreaSectionFormat if in_aux_area => {
                // Format record belonging to a folded-away auxiliary detail-pair area; drop it so it
                // doesn't overwrite the real Detail area's (or its section's) format.
            }
            RdRecord::AreaSectionFormat => {
                let lb = node.leaf_bytes(logical);
                // Byte 4 discriminates: 0x01 = the current section's format, else the current area's.
                if lb.get(4).copied() == Some(0x01) {
                    if let Some(sec) = current_section(&mut areas) {
                        sec.format = decode_section_format(&lb);
                    }
                } else if let Some(area) = areas.last_mut() {
                    let group = area.format.group; // GroupAreaFormat is not in this record; keep it.
                    area.format = decode_area_format(&lb);
                    area.format.group = group;
                }
            }
            RdRecord::FontCondition => {
                let resolved = resolve_conditions(&condition_refs(node, logical), &conditions);
                if let Some(fc) = current_object(&mut areas).and_then(font_color_mut) {
                    fc.condition_formulas.extend(resolved);
                }
            }
            RdRecord::SectionCondition => {
                let resolved = resolve_conditions(&condition_refs(node, logical), &conditions);
                if let Some(sec) = current_section(&mut areas) {
                    sec.condition_formulas.extend(resolved);
                }
            }
            RdRecord::BorderCondition => {
                // The `0xed` wrapper's leaf names the border's colour condition formulas; stash them
                // for the `0xec` child that follows (which rebuilds the border).
                pending_border_conditions =
                    resolve_conditions(&condition_refs(node, logical), &conditions);
            }
            RdRecord::Border => {
                let lb = node.leaf_bytes(logical);
                // For drawing objects the `0xec` record doubles as the line-properties carrier:
                // byte 21 is the line thickness in twips (0 = hairline), and byte 25 authoritatively
                // types the `0xa9` shape — `1` = box, `2` = line. This is what distinguishes a box
                // from a line when their geometry is identical (e.g. a zero-height box vs a horizontal
                // line both have height 0); geometry alone cannot tell them apart.
                let line_thickness =
                    crate::model::Twips(i32::from(lb.get(21).copied().unwrap_or(0)));
                let shape_type = DrawingShapeKind::from_byte(lb.get(25).copied().unwrap_or(0));
                let mut border = raise_border(node, logical);
                border.condition_formulas = std::mem::take(&mut pending_border_conditions);
                // The box's own section height (to detect a box that spans past it, below).
                let section_height = areas
                    .last()
                    .and_then(|a| a.sections.last())
                    .map(|s| s.height.0)
                    .unwrap_or(0);
                if let Some(obj) = current_object(&mut areas) {
                    // Reclassify a freshly-opened drawing object (always opened as a Line at `0xa9`)
                    // to a Box when byte 25 says so, carrying over its drawing properties. A box's
                    // true geometry is the opener's absolute bottom-right corner (the `0x9e` size can
                    // be inflated), so derive Width/Height from it.
                    if shape_type == DrawingShapeKind::Box {
                        if let ReportObjectKind::Line(l) = &obj.kind {
                            let shape = l.shape;
                            obj.kind = ReportObjectKind::Box(crate::model::BoxShape {
                                shape,
                                end_section_name: l.end_section_name.clone(),
                                ..Default::default()
                            });
                            if shape.right.0 > 0 {
                                obj.bounds.width = Twips(shape.right.0 - obj.bounds.left.0);
                                // A cross-section box (spanning into a later section) keeps the
                                // `0x9e` height (the true span); its end section is resolved in a
                                // post-pass. Detected when the opener bottom is above the top, or the
                                // box's bottom edge (top + span) extends past its own section. A box
                                // that fits one section instead takes the opener span as its height.
                                let top = obj.bounds.top.0;
                                let spans = shape.bottom.0 < top
                                    || top + obj.bounds.height.0 > section_height;
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
            }
            RdRecord::FontColor => {
                let color = raise_colorref(&node.leaf_bytes(logical));
                if let Some(fc) = current_object(&mut areas).and_then(font_color_mut) {
                    fc.color = color;
                }
            }
            RdRecord::Font => {
                if let Some(font) = raise_font(node, logical) {
                    // Per-run font: a `0x08` streams right after the run (`0xc2`/`0xc4`) it styles, so
                    // it belongs to the current text object's last run.
                    if let Some(ReportObjectKind::Text(t)) =
                        current_object(&mut areas).map(|o| &mut o.kind)
                    {
                        if let Some(run) = t.paragraphs.last_mut().and_then(|p| p.runs.last_mut()) {
                            run.font = Some(font.clone());
                        }
                    }
                    // Object-level font: first run wins — a multi-run text object keeps the font of its
                    // first run for the `<Font>` the exporter emits.
                    if !font_set {
                        if let Some(fc) = current_object(&mut areas).and_then(font_color_mut) {
                            fc.font = font;
                            font_set = true;
                        }
                    }
                }
            }
            RdRecord::TextObjectFormat => {
                // Each `0xc0` opens one line (paragraph) of the text object: `0xc0`,`0xc2`(text),
                // `0x08`(font) repeat per line. The first opens line 1; every subsequent `0xc0`
                // within the same object is a line break, rendered as `\n` in `<Text>`.
                if let Some(ReportObjectKind::Text(t)) =
                    current_object(&mut areas).map(|o| &mut o.kind)
                {
                    if !t.display.is_empty() {
                        t.display.push('\n');
                    }
                    // Start a new paragraph in the structured run tree (one per `0x00c0`).
                    t.paragraphs.push(crate::model::Paragraph::default());
                }
                // Text and field-heading objects carry their authoritative horizontal alignment in
                // byte 12 of this `0xc0` record (the `0xfc` value is a conditional override). It
                // streams after `0xfc`, so it correctly supersedes it; field objects have no `0xc0`.
                let lb = node.leaf_bytes(logical);
                if let (Some(obj), Some(&a)) = (current_object(&mut areas), lb.get(12)) {
                    obj.format.horizontal_alignment = Alignment::from_code(i32::from(a));
                }
            }
            RdRecord::EmbeddedField => {
                // A `0x00c4` record is a field reference embedded in the current text object, with the
                // same layout as a field object's opener: `[u32-BE len][ref NUL-term][kind byte @ p]…
                // [special-field code @ p+2]`. Render it inline for `display` exactly as the engine
                // renders a DataSource (via the shared `field_data_source`: db/formula/param → `{ref}`,
                // a special field → its code name like `PrintDate`), then record the raw `alias.name` /
                // `@formula` / `?param` reference for UseCount.
                let raw = object_data_string(node, logical)
                    .or_else(|| first_string(node, logical))
                    .unwrap_or_default();
                if !raw.is_empty() {
                    let lb = node.leaf_bytes(logical);
                    let p = lb.get(3).map(|len| 4 + *len as usize).unwrap_or(0);
                    let kind = lb
                        .get(p)
                        .map(|c| FieldRefKind::from_code(*c))
                        .unwrap_or_default();
                    let code = lb.get(p + 2).copied();
                    let rendered = field_data_source(kind, &raw, groups, None, code, None);
                    if let Some(ReportObjectKind::Text(t)) =
                        current_object(&mut areas).map(|o| &mut o.kind)
                    {
                        t.display.push_str(&rendered);
                        push_text_run(
                            t,
                            crate::model::TextRun {
                                text: rendered,
                                field_ref: Some(raw.clone()),
                                font: None,
                            },
                        );
                        t.embedded_fields.push(raw);
                    }
                }
            }
            RdRecord::TextContent => {
                let text = object_data_string(node, logical).unwrap_or_default();
                if let Some(ReportObjectKind::Text(t)) =
                    current_object(&mut areas).map(|o| &mut o.kind)
                {
                    t.display.push_str(&text);
                    push_text_run(
                        t,
                        crate::model::TextRun {
                            text: text.clone(),
                            field_ref: None,
                            font: None,
                        },
                    );
                }
                if let Some(obj) = current_object(&mut areas) {
                    set_object_text(&mut obj.kind, text);
                }
            }
            RdRecord::FieldFormatBlock => {
                // The wrapper (odd rtype) parents its value child (even rtype = wrapper − 1). Decode
                // the byte-derived children into the current field object's FieldFormat. Every field
                // opener is followed by the full block, so `f.format` becomes `Some` for every field.
                // The Numeric child streams twice; the second one is authoritative, so it overwrites.
                if let Some(child) = node.children.first() {
                    let lb = child.leaf_bytes(logical);
                    if let Some(ReportObjectKind::Field(f)) =
                        current_object(&mut areas).map(|o| &mut o.kind)
                    {
                        let ff = f.format.get_or_insert_with(Default::default);
                        match child.rtype {
                            FF_COMMON_VALUE => ff.common = decode_common_format(&lb),
                            FF_NUMERIC_VALUE => ff.numeric = decode_numeric_format(&lb),
                            FF_BOOLEAN_VALUE => ff.boolean = decode_boolean_format(&lb),
                            // The `0x00f2` date leaf carries the per-field stored day/month/year
                            // format enums (varies per field); decode them. The engine reports them
                            // verbatim only for a date-valued non-system-default field — the
                            // effective resolution lives in the XML exporter's field_format derivation. The time
                            // (`0x00f6`) / datetime leaves stay runtime/locale-resolved; the string
                            // sub-format is decoded elsewhere / not emitted.
                            FF_DATE_VALUE => ff.date = decode_date_format(&lb),
                            _ => {}
                        }
                    }
                }
            }
            RdRecord::Other => {}
        }
    }
    demote_orphan_headings(&mut areas);
    reclassify_picture_openers(&mut areas);
    attach_grid_bindings(tree, logical, &mut areas);
    sort_areas_canonical(&mut areas);
    resolve_cross_section_boxes(&mut areas);
    ReportDefinition { areas }
}
