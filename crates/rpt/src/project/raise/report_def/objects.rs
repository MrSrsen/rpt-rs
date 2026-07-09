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
/// is picture/chart/blob-field (the engine auto-names charts `Graph…`, static pictures `Picture…`,
/// and binds blob fields to their database field name). Line-vs-box is already resolved at the
/// `0xec` border record via its byte-25 shape type (see [`super::DrawingShapeKind`]).
pub(super) fn reclassify_picture_openers(areas: &mut [Area]) {
    for obj in area_objects_mut(areas) {
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
}

/// Resolve the end section of every cross-section box. Walk the stacked sections (canonical layout
/// order, across areas) from the box's section, accumulating each section's height from the box's
/// top until the total reaches the box height — that section holds the box's bottom edge. Must run
/// after `sort_areas_canonical` so the stacking order matches the rendered layout.
pub(super) fn resolve_cross_section_boxes(areas: &mut [Area]) {
    let flat: Vec<(String, i32)> = areas
        .iter()
        .flat_map(|a| &a.sections)
        .map(|s| (s.name.clone(), s.height.0))
        .collect();
    for (start, sec) in areas.iter_mut().flat_map(|a| &mut a.sections).enumerate() {
        for obj in &mut sec.objects {
            let top = obj.bounds.top.0;
            let height = obj.bounds.height.0;
            if let ReportObjectKind::Box(bx) = &mut obj.kind {
                // Cross-section signature: the opener's (end-relative) bottom is above the top, or the
                // box's bottom edge (top + the true span) extends past its own section.
                if bx.shape.bottom.0 < top || top + height > flat[start].1 {
                    // The bottom edge sits `bx.shape.bottom` twips into the end section, so the end
                    // section's top lies `height - bottom` below the box top. Walk stacked sections
                    // (from the box top) until the cumulative height reaches that point; that section
                    // holds the bottom edge.
                    let target = (height - bx.shape.bottom.0).max(0);
                    let mut acc = flat[start].1 - top;
                    let mut end = start;
                    while acc < target && end + 1 < flat.len() {
                        end += 1;
                        acc += flat[end].1;
                    }
                    bx.end_section_name = flat[end].0.clone();
                }
            }
        }
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
