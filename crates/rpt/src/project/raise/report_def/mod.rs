//! Report definition — the area/section/object layout tree: the record walk that builds areas,
//! sections and objects, plus the cursor helpers and post-walk transforms.
//!
//! Submodules decode the leaf detail: [`formats`] (per-object/area/section format records),
//! [`conditions`] (conditional-format formula slots), [`data_source`] (field/object reference text).

use super::*;

mod conditions;
mod data_source;
mod formats;

use conditions::*;
use data_source::*;
use formats::*;

// `condition_formula_bodies` is also consumed by the data-definition raise (a sibling module), so
// re-export it up to the parent `raise` module.
pub(in crate::project::raise) use conditions::condition_formula_bodies;

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
    // Canonical group level per area suffix (first-appearing `GroupHeader` = outermost = 1), the
    // same mapping `sort_areas_canonical` builds. A summary/group-name object's group scope is this
    // level, not the raw area suffix — Crystal numbers area suffixes in UI-creation order, not
    // nesting order, so a footer's suffix need not equal its group's nesting index.
    let mut suffix_level: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut next_level = 1usize;
    // A text object stores one font per text run (`0x08`), interleaved with the run text (`0xc2`).
    // The engine reports the object's font as the FIRST run's font, so once an object has received a
    // font we ignore later runs. Reset on every object opener.
    let mut font_set = false;
    // True while the walk is inside a folded-away auxiliary detail-pair area (DetailHeader/Footer),
    // whose format records must not leak onto the real Detail area. Set per Area marker.
    let mut in_aux_area = false;
    // The ordered non-running-total summary definitions (`0x7e` not preceded by a `0x80` reset).
    // A localized field-object reference string fails the ASCII guard, so a Summary object's `code`
    // byte indexes into this list to recover its operation + summarized field.
    let sum_defs: Vec<(
        crate::model::SummaryOperation,
        String,
        crate::model::FieldValueType,
    )> = {
        let mut prev = 0u16;
        let mut out = Vec::new();
        for n in flatten(tree) {
            if n.rtype == SUMMARY_DEF && prev != RT_RESET {
                let lb = n.leaf_bytes(logical);
                let op = crate::model::SummaryOperation::from_code(i32::from(
                    lb.first().copied().unwrap_or(0),
                ));
                let operand = lb
                    .get(4..)
                    .and_then(read_lp_string)
                    .map(|(s, _)| s)
                    .unwrap_or_default();
                // The `0x71` child carries the summary's result value type: unlike a running total's
                // child (which leads with the field name), a summary's child is a fixed header
                // `00 00 00 01 00 <vt> 00 <nbytes> …` — the value-type code sits at offset 5.
                let value_type = n
                    .children
                    .iter()
                    .find(|c| c.rtype == NAMED_VALUE)
                    .and_then(|child| child.leaf_bytes(logical).get(5).copied())
                    .map(|code| crate::model::FieldValueType::from_code(i32::from(code)))
                    .unwrap_or_default();
                out.push((op, operand, value_type));
            }
            prev = n.rtype;
        }
        out
    };
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
                // Register each GroupHeader suffix → canonical level on first appearance (group
                // headers print outermost-first, so first-seen = level 1). Footers reuse the level.
                if let Some(a) = areas.last() {
                    if a.kind == AreaSectionKind::GroupHeader {
                        suffix_level
                            .entry(trailing_digits(&a.name))
                            .or_insert_with(|| {
                                let l = next_level;
                                next_level += 1;
                                l
                            });
                    }
                }
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
                // level (not the raw area suffix). Report/page bands yield none (a grand total).
                let group_no = areas.last().and_then(|a| match a.kind {
                    AreaSectionKind::GroupHeader | AreaSectionKind::GroupFooter => {
                        suffix_level.get(&trailing_digits(&a.name)).copied()
                    }
                    _ => None,
                });
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
            // The subreport's friendly name (SubreportName) lives in the backing `Subdocument N`
            // (its report-header record), not here; the opener's leaf bytes `[0..4]` (big-endian)
            // give that index `N`, which `io` uses to resolve the name after subreports are decoded.
            RdRecord::OpenSubreport => {
                let subdoc_index = node
                    .leaf_bytes(logical)
                    .get(0..4)
                    .and_then(|b| <[u8; 4]>::try_from(b).ok())
                    .map(u32::from_be_bytes)
                    .unwrap_or(0);
                open_object(
                    &mut areas,
                    ReportObjectKind::Subreport(crate::model::SubreportObject {
                        subdoc_index,
                        ..Default::default()
                    }),
                );
            }
            RdRecord::OpenCrossTab => {
                // The `0xb8` opener carries the grid's row/column/summary bindings (decoded by
                // rpt-engine for UseCount); here it opens a typed marker so the following name,
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
                    // Byte 5 is EnableKeepTogether (1 = keep together), byte 9 EnableCanGrow.
                    obj.format.keep_together = lb.get(5).is_none_or(|&b| b != 0);
                    obj.format.can_grow = lb.get(9).is_some_and(|&b| b != 0);
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
                                obj.bounds.height = Twips(shape.bottom.0 - obj.bounds.top.0);
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
                // First run wins: a multi-run text object keeps the font of its first run.
                if !font_set {
                    if let Some(font) = raise_font(node, logical) {
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
                }
                if let Some(obj) = current_object(&mut areas) {
                    set_object_text(&mut obj.kind, text);
                }
            }
            RdRecord::Other => {}
        }
    }
    // A text object with a field-heading link is only a FieldHeadingObject if the FieldObject it
    // names still exists; an orphaned link (the field was removed) degrades to a plain TextObject,
    // matching the engine's object type. Collect the live field-object names, then demote orphans.
    // Each FieldObject's stored alignment, keyed by object name. A heading left at `DefaultAlign`
    // inherits the explicit alignment of the field it heads; the value-type-based default (for a
    // field that is itself `DefaultAlign`) is resolved later in `resolve_heading_alignment`, once
    // the database has supplied each field object's value type.
    let field_align: BTreeMap<String, Alignment> = areas
        .iter()
        .flat_map(|a| &a.sections)
        .flat_map(|sec| &sec.objects)
        // A heading can head a regular field OR a blob/picture field (e.g. a `user_signature`
        // BlobFieldObject); both are live targets, so only a truly orphaned link degrades.
        .filter(|o| {
            matches!(
                o.kind,
                ReportObjectKind::Field(_) | ReportObjectKind::BlobField(_)
            )
        })
        .map(|o| (o.name.clone(), o.format.horizontal_alignment))
        .collect();
    for obj in areas
        .iter_mut()
        .flat_map(|a| &mut a.sections)
        .flat_map(|sec| &mut sec.objects)
    {
        if let ReportObjectKind::FieldHeading(h) = &obj.kind {
            match field_align.get(h.field_object_name.as_str()) {
                // The named field no longer exists — the heading degrades to a plain text object.
                None => {
                    obj.kind = ReportObjectKind::Text(TextObject {
                        text: h.text.clone(),
                        max_lines: h.max_lines,
                        font_color: h.font_color.clone(),
                        ..Default::default()
                    });
                }
                // A heading without its own alignment inherits the field's explicit alignment (so a
                // heading over a right-aligned number is itself right-aligned). When the field is
                // also `DefaultAlign`, the heading stays default here and is resolved by value type
                // in `resolve_heading_alignment` after the database is decoded.
                Some(&a)
                    if a != Alignment::DefaultAlign
                        && obj.format.horizontal_alignment == Alignment::DefaultAlign =>
                {
                    obj.format.horizontal_alignment = a;
                }
                _ => {}
            }
        }
    }

    // Resolve the opener kinds that can only be told apart once the object's name is known: `0xae`
    // is picture/chart/blob-field (the engine auto-names charts `Graph…`, static pictures `Picture…`,
    // and binds blob fields to their database field name). Line-vs-box is already resolved at the
    // `0xec` border record via its byte-25 shape type (see [`DrawingShapeKind`]).
    for obj in areas
        .iter_mut()
        .flat_map(|a| &mut a.sections)
        .flat_map(|sec| &mut sec.objects)
    {
        match &obj.kind {
            ReportObjectKind::Picture(_) if obj.name.starts_with("Graph") => {
                obj.kind = ReportObjectKind::Chart(Box::default());
            }
            ReportObjectKind::Picture(_) if !obj.name.starts_with("Picture") => {
                // Fallback: a picture that is neither a chart nor a static image but had no `0xb1`
                // wrapper to supply the bound field reference (so its data source stays empty).
                obj.kind = ReportObjectKind::BlobField(crate::model::BlobFieldObject::default());
            }
            _ => {}
        }
    }

    // Attach each chart / cross-tab object's decoded field bindings (decoded by name from the
    // separate binding region; see `collect_grid_bindings`), now that openers are reclassified.
    let bindings = collect_grid_bindings(tree, logical);
    for obj in areas
        .iter_mut()
        .flat_map(|a| &mut a.sections)
        .flat_map(|sec| &mut sec.objects)
    {
        let Some(refs) = bindings.get(&obj.name) else {
            continue;
        };
        match &mut obj.kind {
            ReportObjectKind::Chart(c) => {
                c.data_refs = refs.data.clone();
                c.category_refs = refs.category.clone();
            }
            // Every cross-tab grid binding is a row/column dimension (no data role here).
            ReportObjectKind::CrossTab(c) => c.field_refs = refs.category.clone(),
            _ => {}
        }
    }
    sort_areas_canonical(&mut areas);
    ReportDefinition { areas }
}

/// Collect each chart / cross-tab object's persistent field bindings from the report's binding
/// region (a flat run of sibling records that follows the layout), keyed by object name.
///
/// The binding records reuse the generic group machinery, so each is scoped precisely:
/// - A **chart** binding block starts with `0xb4` (which nests the chart's `ObjectName`); its data
///   ("show value") field is the `0x7e` child of the next `0x7f`, and its category ("on change of")
///   field is the next grid `0xe5`.
/// - A **cross-tab** block starts with `0xb9`/`0xb8` (nesting `CrossTabN`); each row/column
///   dimension is a grid `0xe5`.
///
/// A grid `0xe5` is told apart from a real report group (which `data_def` decodes into
/// `DataDefinition.groups`) by its localized order-marker string: a report group carries
/// `@Group #N Order`, a chart category `@… Grid #N Order`, and a cross-tab dimension
/// `@Column #N Order` / `@Row #N Order`. Only field-shaped references (`Table.field` or `@formula`)
/// are kept — grand-total dimension levels read `Others`. Cross-tab data-cell summaries
/// (`Sum of {Table.x}`) are NOT collected here (they are counted via `<SummaryFields>`).
fn collect_grid_bindings(
    tree: &[RecordNode],
    logical: &[u8],
) -> std::collections::HashMap<String, GridBindings> {
    let mut out: std::collections::HashMap<String, GridBindings> = std::collections::HashMap::new();
    // The chart/cross-tab whose binding records are currently being read: (object name, is_chart).
    let mut current: Option<(String, bool)> = None;
    // `is_category` selects which role the field is bound in: a chart's data ("show value") field
    // versus a category / cross-tab dimension (a grid group). The engine counts them differently.
    let push = |out: &mut std::collections::HashMap<String, GridBindings>,
                cur: &Option<(String, bool)>,
                field: Option<String>,
                is_category: bool| {
        if let (Some((name, _)), Some(f)) = (cur, field.filter(|s| is_field_ref(s))) {
            let b = out.entry(name.clone()).or_default();
            if is_category {
                b.category.push(f);
            } else {
                b.data.push(f);
            }
        }
    };
    for node in flatten(tree) {
        match node.rtype {
            CHART_BINDING => current = descendant_object_name(node, logical).map(|n| (n, true)),
            CROSSTAB_WRAPPER => {
                current = descendant_object_name(node, logical).map(|n| (n, false));
            }
            // A chart's data field (`0x7f` → `0x7e` field ref); only inside a chart block.
            CHART_DATA if matches!(current, Some((_, true))) => {
                push(&mut out, &current, first_string(node, logical), false);
            }
            // A grid group is a chart category / cross-tab dimension binding (identified by marker).
            GROUP if current.is_some() && is_grid_group(node, logical) => {
                push(&mut out, &current, first_string(node, logical), true);
            }
            // Leaving the binding scope (a real layout marker) clears the current object.
            AREA_MARKER | SECTION_MARKER => current = None,
            _ => {}
        }
    }
    out
}

/// One chart/cross-tab object's field bindings, split by the role the engine binds them in (it
/// references each role a different number of times for `Field.UseCount`): `data` are a chart's
/// "show value" data fields; `category` are chart "on change of" categories and cross-tab row/column
/// dimensions (the `0xe5` grid groups). A cross-tab has only `category` bindings.
#[derive(Default)]
struct GridBindings {
    data: Vec<String>,
    category: Vec<String>,
}

/// The object name nested in a chart/cross-tab wrapper: the first `OBJECT_NAME` (`0x9e`) descendant's
/// string. The wrapper's own leaf bytes can decode a spurious short string, so the name must be read
/// from the `0x9e` record specifically (not the first string anywhere in the subtree).
fn descendant_object_name(node: &RecordNode, logical: &[u8]) -> Option<String> {
    let mut found = None;
    node.walk(&mut |n| {
        if found.is_none() && n.rtype == OBJECT_NAME {
            found = first_string(n, logical);
        }
    });
    found
}

/// Whether a string is an engine field reference: a database field (`Table.field`) or a formula
/// (`@name`). Excludes literals like `Others` and localized order/name marker strings.
fn is_field_ref(s: &str) -> bool {
    s.starts_with('@') || s.contains('.')
}

/// Whether a `0xe5` group record is a chart-category / cross-tab-dimension "grid" group (rather than
/// a report group), identified by its localized order-marker string.
fn is_grid_group(node: &RecordNode, logical: &[u8]) -> bool {
    all_strings(node, logical)
        .iter()
        .any(|s| s.contains(" Grid #") || s.starts_with("@Column #") || s.starts_with("@Row #"))
}

/// Reorder areas into the canonical Crystal Reports band sequence —
/// `ReportHeader, PageHeader, GroupHeader[1..N], Detail, GroupFooter[N..1], ReportFooter,
/// PageFooter` — matching the order the SDK's `Areas` collection presents.
/// The native binary stores them in raw storage order (page/report bands first, then interleaved
/// group header/footer pairs, then detail), which is not the band order. Note ReportFooter prints
/// *before* PageFooter even though the enum value is larger, so the band rank is explicit.
///
/// Group nesting level (1 = outermost) is assigned by the order in which `GroupHeader` areas
/// appear in the binary; the matching `GroupFooter` is linked by the trailing-digit suffix shared
/// with its header (e.g. `GroupHeaderArea4` ↔ `GroupFooterArea4`).
pub(super) fn sort_areas_canonical(areas: &mut [Area]) {
    use std::collections::HashMap;

    let mut suffix_level: HashMap<String, usize> = HashMap::new();
    let mut next = 1usize;
    for area in areas.iter() {
        if area.kind == AreaSectionKind::GroupHeader {
            suffix_level
                .entry(trailing_digits(&area.name))
                .or_insert_with(|| {
                    let l = next;
                    next += 1;
                    l
                });
        }
    }
    let n = next - 1;

    areas.sort_by_key(|a| {
        use AreaSectionKind::*;
        let band: u8 = match a.kind {
            ReportHeader => 0,
            PageHeader => 1,
            GroupHeader => 2,
            Detail => 3,
            GroupFooter => 4,
            ReportFooter => 5,
            PageFooter => 6,
            _ => 7,
        };
        let sub: usize = match a.kind {
            GroupHeader => *suffix_level.get(&trailing_digits(&a.name)).unwrap_or(&0),
            GroupFooter => match suffix_level.get(&trailing_digits(&a.name)) {
                Some(&lv) => n + 1 - lv,
                None => 0,
            },
            _ => 0,
        };
        (band, sub)
    });
}

/// Populate each display field object's value type from the database schema, then resolve any field
/// heading still left at `DefaultAlign` over a `DefaultAlign` field: the engine right-aligns the
/// heading when the underlying field is numeric and left-aligns it otherwise. (Headings that
/// inherit an explicit field alignment were already resolved while building the report definition.)
pub(super) fn resolve_heading_alignment(report: &mut Report) {
    // Each database field's `{alias.name}` reference → its value type (the form a db field object's
    // DataSource takes).
    let field_types: BTreeMap<String, FieldValueType> = report
        .database
        .tables
        .iter()
        .flat_map(|t| {
            t.data_fields
                .iter()
                .map(move |f| (format!("{{{}.{}}}", t.alias, f.name), f.value_type))
        })
        .collect();

    for obj in report
        .report_definition
        .areas
        .iter_mut()
        .flat_map(|a| &mut a.sections)
        .flat_map(|s| &mut s.objects)
    {
        if let ReportObjectKind::Field(f) = &mut obj.kind {
            if let Some(&vt) = field_types.get(&f.data_source) {
                f.value_type = vt;
            }
        }
    }

    // Field objects now carry their value type; index it by object name for the heading links.
    let field_vt: BTreeMap<String, FieldValueType> = report
        .report_definition
        .areas
        .iter()
        .flat_map(|a| &a.sections)
        .flat_map(|s| &s.objects)
        .filter_map(|o| match &o.kind {
            ReportObjectKind::Field(f) => Some((o.name.clone(), f.value_type)),
            _ => None,
        })
        .collect();

    for obj in report
        .report_definition
        .areas
        .iter_mut()
        .flat_map(|a| &mut a.sections)
        .flat_map(|s| &mut s.objects)
    {
        let resolved = match &obj.kind {
            ReportObjectKind::FieldHeading(h)
                if obj.format.horizontal_alignment == Alignment::DefaultAlign =>
            {
                let numeric = field_vt
                    .get(h.field_object_name.as_str())
                    .is_some_and(|vt| vt.is_numeric());
                Some(if numeric {
                    Alignment::RightAlign
                } else {
                    Alignment::LeftAlign
                })
            }
            _ => None,
        };
        if let Some(a) = resolved {
            obj.format.horizontal_alignment = a;
        }
    }
}

/// Open an area (`0x8a`). The detail area-pair's auxiliary `DetailHeader` / `DetailFooter` halves
/// are folded into the single `Detail` area, so they are skipped (their objects, if any, attach to
/// the preceding area).
/// Returns `true` if a real area was opened, `false` if this was an auxiliary detail-pair
/// Header/Footer marker that the engine folds away (the caller suppresses its trailing records).
pub(super) fn open_area(areas: &mut Vec<Area>, node: &RecordNode, logical: &[u8]) -> bool {
    let name = first_string(node, logical).unwrap_or_default();
    if name.starts_with("DetailHeader") || name.starts_with("DetailFooter") {
        return false;
    }
    let kind = area_kind(&name);
    areas.push(Area {
        kind,
        name,
        sections: Vec::new(),
        ..Default::default()
    });
    true
}

/// Open a section (`0x8c`) in the current area, reading its Height (u32 BE twips) + Name.
pub(super) fn open_section(areas: &mut [Area], node: &RecordNode, logical: &[u8]) {
    let b = node.leaf_bytes(logical);
    let height = b
        .get(0..4)
        .map(|x| i32::from_be_bytes([x[0], x[1], x[2], x[3]]))
        .unwrap_or(0);
    let mut name = String::new();
    let mut i = 4;
    while i + 4 <= b.len() {
        if let Some((n, _)) = read_lp_string(&b[i..]) {
            name = n;
            break;
        }
        i += 1;
    }
    if let Some(area) = areas.last_mut() {
        let kind = area.kind;
        area.sections.push(Section {
            kind,
            height: Twips(height),
            name,
            ..Default::default()
        });
    }
}

/// Append an empty object of the given kind to the current section; its attribute records fill
/// the rest in. Objects only ever follow a section marker, so the current section exists.
pub(super) fn open_object(areas: &mut [Area], kind: ReportObjectKind) {
    if let Some(area) = areas.last_mut() {
        push_object(area, String::new(), Rect::default(), kind);
    }
}

/// The most-recently-opened object — the last object of the last section of the last area —
/// which the attribute records that follow an opener decorate.
pub(super) fn current_object(areas: &mut [Area]) -> Option<&mut ReportObject> {
    areas.last_mut()?.sections.last_mut()?.objects.last_mut()
}

pub(super) fn current_section(areas: &mut [Area]) -> Option<&mut crate::model::Section> {
    areas.last_mut()?.sections.last_mut()
}

/// Set the literal text of a text or field-heading object (a no-op for other kinds).
pub(super) fn set_object_text(kind: &mut ReportObjectKind, text: String) {
    match kind {
        ReportObjectKind::Text(t) => t.text = text,
        ReportObjectKind::FieldHeading(h) => h.text = text,
        _ => {}
    }
}

pub(super) fn push_object(area: &mut Area, name: String, bounds: Rect, kind: ReportObjectKind) {
    let obj = ReportObject {
        name,
        bounds,
        kind,
        ..Default::default()
    };
    if let Some(section) = area.sections.last_mut() {
        section.objects.push(obj);
    }
}

/// Map an area name (e.g. `PageHeaderArea1`, `DetailArea1`) to its [`AreaSectionKind`].
pub(super) fn area_kind(name: &str) -> AreaSectionKind {
    for (prefix, kind) in [
        ("ReportHeader", AreaSectionKind::ReportHeader),
        ("ReportFooter", AreaSectionKind::ReportFooter),
        ("PageHeader", AreaSectionKind::PageHeader),
        ("PageFooter", AreaSectionKind::PageFooter),
        ("GroupHeader", AreaSectionKind::GroupHeader),
        ("GroupFooter", AreaSectionKind::GroupFooter),
        ("Detail", AreaSectionKind::Detail),
    ] {
        if name.starts_with(prefix) {
            return kind;
        }
    }
    // Some reports name the five fixed bands generically (`Area1`..`Area5`) instead of by band.
    // They are numbered in canonical band order: 1=ReportHeader, 2=PageHeader, 3=Detail,
    // 4=ReportFooter, 5=PageFooter (group bands always carry explicit `GroupHeader/FooterArea`
    // names, so they never reach here).
    if let Some(suffix) = name.strip_prefix("Area") {
        return match suffix {
            "1" => AreaSectionKind::ReportHeader,
            "2" => AreaSectionKind::PageHeader,
            "3" => AreaSectionKind::Detail,
            "4" => AreaSectionKind::ReportFooter,
            "5" => AreaSectionKind::PageFooter,
            _ => AreaSectionKind::default(),
        };
    }
    AreaSectionKind::default()
}
