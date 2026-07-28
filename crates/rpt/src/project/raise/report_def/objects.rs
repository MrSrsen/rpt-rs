//! Report-object cursors and post-walk object-tree transforms — appending objects to the current
//! section, and the fix-ups that need the whole tree (heading demotion, picture reclassification,
//! cross-section box resolution, heading-alignment inheritance).

use super::*;
use crate::model::{area_objects, area_objects_mut};

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

/// Set the literal text of a text or field-heading object (a no-op for other kinds).
pub(super) fn set_object_text(kind: &mut ReportObjectKind, text: String) {
    match kind {
        ReportObjectKind::Text(t) => t.text = text,
        ReportObjectKind::FieldHeading(h) => h.text = text,
        _ => {}
    }
}

/// Append a run to a text object's structured paragraph tree, opening an implicit first paragraph if
/// the run arrives before any `0x00c0` paragraph marker (a single-line text object with no explicit
/// paragraph opener).
pub(super) fn push_text_run(t: &mut crate::model::TextObject, run: crate::model::TextRun) {
    if t.paragraphs.is_empty() {
        t.paragraphs.push(crate::model::Paragraph::default());
    }
    if let Some(p) = t.paragraphs.last_mut() {
        p.runs.push(run);
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

/// A text object with a field-heading link is only a FieldHeadingObject if the FieldObject it names
/// still exists; an orphaned link (the field was removed) degrades to a plain TextObject, matching
/// the engine's object type. Also inherits a heading's alignment from the field it heads: a heading
/// left at `DefaultAlign` takes the explicit alignment of its field; the value-type-based default
/// (for a field that is itself `DefaultAlign`) is resolved later in `resolve_heading_alignment`,
/// once the database has supplied each field object's value type.
pub(super) fn demote_orphan_headings(areas: &mut [Area]) {
    // Each FieldObject's stored alignment, keyed by object name.
    let field_align: BTreeMap<String, Alignment> = area_objects(areas)
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
    for obj in area_objects_mut(areas) {
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
}

/// Resolve the opener kinds that can only be told apart once the object's name is known: `0xae`
/// is picture/chart/blob-field. A chart is identified by its name appearing in a `0xb4 CHART_BINDING`
/// block (`chart_names`) — the engine auto-names charts `Graph…`, but a user-renamed chart carries no
/// such prefix, so the binding block is the authoritative signal (the `Graph…` prefix is kept only as
/// a defensive fallback). Static pictures are named `Picture…`; blob fields bind to their database
/// field name. Line-vs-box is already resolved at the `0xec` border record via its byte-25 shape type
/// (see [`super::DrawingShapeKind`]).
pub(super) fn reclassify_picture_openers(
    areas: &mut [Area],
    chart_names: &std::collections::HashSet<String>,
) {
    for obj in area_objects_mut(areas) {
        match &obj.kind {
            ReportObjectKind::Picture(_)
                if chart_names.contains(&obj.name) || obj.name.starts_with("Graph") =>
            {
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
}

/// Finalize every drawing object (line/box) once the whole area tree exists and is in canonical
/// stacking order. Three fix-ups that all need the resolved section layout:
///
/// 1. **End section** (SDK `EndSectionName`). A box that spans past its own section has its end
///    section resolved geometrically: walk the stacked sections (canonical layout order, across
///    areas) from the box's section, accumulating each section's height from the box's top until the
///    total reaches the box height — that section holds the box's bottom edge. Every non-spanning box
///    and every line ends in its own section, so its end section is just the owning section's name.
/// 2. **Line second corner** (SDK `Bottom`). The `0xa9` opener's second coordinate is stale for a
///    line, so its bottom-corner Y is derived from the box: `Top` for an upward/flat line (stored
///    height ≤ 0, i.e. the second corner is at the top) and `Top + height` for a downward one.
/// 3. **Stroke/fill redirection** — the authoritative stroke style/colour and fill live in the
///    object's [`Border`](crate::model::Border) (decoded from `0xec`); the drawing shape's own
///    `line_style`/`line_color` and the box `fill_color` are mirrors kept for consumers that read the
///    shape directly, so populate them from the border here.
///
/// Must run after `sort_areas_canonical` (stacking order) and after the border records are decoded.
pub(super) fn resolve_cross_section_boxes(areas: &mut [Area]) {
    let flat: Vec<(String, i32)> = areas
        .iter()
        .flat_map(|a| &a.sections)
        .map(|s| (s.name.clone(), s.height.0))
        .collect();
    for (start, sec) in areas.iter_mut().flat_map(|a| &mut a.sections).enumerate() {
        let sec_name = sec.name.clone();
        for obj in &mut sec.objects {
            let top = obj.bounds.top.0;
            let height = obj.bounds.height.0;
            // The authoritative stroke/fill live in the border; copy them into the shape mirrors.
            let border_style = border_stroke_style(&obj.border);
            let border_color = obj.border.border_color;
            let border_fill = obj.border.background_color;
            match &mut obj.kind {
                ReportObjectKind::Line(l) => {
                    // Second-corner Y: the anchor top for an upward/flat line (stored height ≤ 0),
                    // else the box bottom. `max(0)` keeps `Top` when the stored height is negative.
                    l.shape.bottom = Twips(top + height.max(0));
                    if l.end_section_name.is_empty() {
                        l.end_section_name = sec_name.clone();
                    }
                    redirect_stroke(&mut l.shape, border_style, border_color);
                }
                ReportObjectKind::Box(bx) => {
                    // Cross-section signature: the opener's (end-relative) bottom is above the top, or
                    // the box's bottom edge (top + the true span) extends past its own section.
                    if bx.shape.bottom.0 < top || top + height > flat[start].1 {
                        // The bottom edge sits `bx.shape.bottom` twips into the end section, so the end
                        // section's top lies `height - bottom` below the box top. Walk stacked sections
                        // (from the box top) until the cumulative height reaches that point; that
                        // section holds the bottom edge.
                        let target = (height - bx.shape.bottom.0).max(0);
                        let mut acc = flat[start].1 - top;
                        let mut end = start;
                        while acc < target && end + 1 < flat.len() {
                            end += 1;
                            acc += flat[end].1;
                        }
                        bx.end_section_name = flat[end].0.clone();
                    } else if bx.end_section_name.is_empty() {
                        bx.end_section_name = sec_name.clone();
                    }
                    redirect_stroke(&mut bx.shape, border_style, border_color);
                    if bx.fill_color.is_none() {
                        bx.fill_color = border_fill;
                    }
                }
                _ => {}
            }
        }
    }
}

/// The stroke style a drawing object renders with: the first non-`NoLine` border edge (a box's edges
/// are uniform; a line carries its style on the single edge matching its orientation).
fn border_stroke_style(b: &crate::model::Border) -> LineStyle {
    [b.top, b.bottom, b.left, b.right]
        .into_iter()
        .find(|s| !matches!(s, LineStyle::NoLine))
        .unwrap_or(LineStyle::NoLine)
}

/// Copy the authoritative border stroke into a drawing shape's mirror fields, leaving the shape's
/// defaults in place when the border carries no style/colour.
fn redirect_stroke(shape: &mut crate::model::DrawingShape, style: LineStyle, color: Option<Color>) {
    if !matches!(style, LineStyle::NoLine) {
        shape.line_style = style;
    }
    if let Some(c) = color {
        shape.line_color = c;
    }
}

/// Populate each display field object's value type from the database schema, then resolve any field
/// heading still left at `DefaultAlign` over a `DefaultAlign` field: the engine right-aligns the
/// heading when the underlying field is numeric and left-aligns it otherwise. (Headings that
/// inherit an explicit field alignment were already resolved while building the report definition.)
pub(in crate::project::raise) fn resolve_heading_alignment(report: &mut Report) {
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

    for obj in report.objects_mut() {
        if let ReportObjectKind::Field(f) = &mut obj.kind {
            if let Some(&vt) = field_types.get(&f.data_source) {
                f.value_type = vt;
            }
        }
    }

    // Resolve the value type of every field object that references a *non-database* definition
    // (formula / SQL-expression / running-total / group-name / special field). Each type is a stored
    // fact recoverable within the file: a formula's and a SQL-expression's result type is decoded into
    // its `FieldDef.value_type` (the `0x71`/`0x7e` value-type code); a group name is always a String;
    // a special field's type follows its kind; a running total takes its summarized field's type.
    // Database-field objects were resolved above and summaries at construction, so only objects still
    // left `Unknown` are filled here (existing types are never clobbered).

    // Reference form (`{@f}` / `{%f}`) → value type, for formula / SQL-expression objects.
    let ref_types: BTreeMap<String, FieldValueType> = report
        .data_definition
        .field_definitions
        .iter()
        .filter_map(|fd| {
            let prefix = match &fd.kind {
                FieldKindData::Formula(_) => '@',
                FieldKindData::SqlExpression(_) => '%',
                _ => return None,
            };
            Some((format!("{{{prefix}{}}}", fd.name), fd.value_type))
        })
        .collect();

    // Running-total defs by reference form (`{#name}`) → (operation, summarized field, stored type).
    let rt_defs: BTreeMap<String, (SummaryOperation, String, FieldValueType)> = report
        .data_definition
        .field_definitions
        .iter()
        .filter_map(|fd| match &fd.kind {
            FieldKindData::RunningTotal(rt) => Some((
                format!("{{#{}}}", fd.name),
                (rt.operation, rt.summarized_field.clone(), fd.value_type),
            )),
            _ => None,
        })
        .collect();

    // Bare-form (unbraced) reference → value type, for resolving a running total's summarized field,
    // which the engine stores unwrapped (`Orders.Order Amount`, `@Line_Sum`).
    let bare_types: BTreeMap<String, FieldValueType> =
        report
            .database
            .tables
            .iter()
            .flat_map(|t| {
                t.data_fields
                    .iter()
                    .map(move |f| (format!("{}.{}", t.alias, f.name), f.value_type))
            })
            .chain(report.data_definition.field_definitions.iter().filter_map(
                |fd| match &fd.kind {
                    FieldKindData::Formula(_) => Some((format!("@{}", fd.name), fd.value_type)),
                    _ => None,
                },
            ))
            .collect();

    for obj in report.objects_mut() {
        if let ReportObjectKind::Field(f) = &mut obj.kind {
            if f.value_type != FieldValueType::Unknown {
                continue;
            }
            f.value_type = match f.ref_kind {
                FieldRefKind::Formula | FieldRefKind::SqlExpression => {
                    ref_types.get(&f.data_source).copied().unwrap_or_default()
                }
                FieldRefKind::GroupName => FieldValueType::String,
                FieldRefKind::Special => special_field_value_type(&f.data_source),
                FieldRefKind::RunningTotal => rt_defs
                    .get(&f.data_source)
                    .map(|(op, sf, stored)| running_total_value_type(*op, sf, *stored, &bare_types))
                    .unwrap_or_default(),
                _ => f.value_type,
            };
        }
    }

    // With value types resolved, pick the numeric-format slot the engine reports: a Currency-valued
    // field surfaces its stored currency-format slot, every other field the number-format slot.
    for obj in report.objects_mut() {
        if let ReportObjectKind::Field(f) = &mut obj.kind {
            if f.value_type == FieldValueType::Currency {
                if let Some(ff) = f.format.as_mut() {
                    ff.numeric = ff.currency_numeric.clone();
                }
            }
        }
    }

    // Field objects now carry their value type; index it by object name for the heading links.
    let field_vt: BTreeMap<String, FieldValueType> = report
        .objects()
        .filter_map(|o| match &o.kind {
            ReportObjectKind::Field(f) => Some((o.name.clone(), f.value_type)),
            _ => None,
        })
        .collect();

    for obj in report.objects_mut() {
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

/// The value type of a special field, keyed by its data-source name (the canonical/English kind name,
/// e.g. `PrintDate`, `PageNumber`, `DataDate`). Date/time kinds resolve to `Date`/`Time`; the
/// page/record/group counters to `Int32u`; the textual kinds (page-of-M, report title/comments, …)
/// to `String`. An unmapped kind stays `Unknown`.
fn special_field_value_type(name: &str) -> FieldValueType {
    crate::model::SpecialFieldType::from_name(name)
        .map(|k| k.value_type())
        .unwrap_or(FieldValueType::Unknown)
}

/// The value type of a running-total field. A counting operation (`Count`/`DistinctCount`) yields an
/// integer count regardless of the summarized field, which the running total's stored value type
/// already reflects. A value-preserving aggregate (`Sum`/`Maximum`/`Minimum`) instead takes the
/// summarized field's own type — the engine promotes e.g. `Sum` of a `Currency` field to `Currency`,
/// which the running total's generic stored `Number` does not capture. Any other operation, or an
/// unresolvable summarized field, keeps the stored type.
fn running_total_value_type(
    operation: SummaryOperation,
    summarized_field: &str,
    stored: FieldValueType,
    bare_types: &BTreeMap<String, FieldValueType>,
) -> FieldValueType {
    match operation {
        SummaryOperation::Sum | SummaryOperation::Maximum | SummaryOperation::Minimum => bare_types
            .get(summarized_field)
            .copied()
            .filter(|vt| *vt != FieldValueType::Unknown)
            .unwrap_or(stored),
        _ => stored,
    }
}
