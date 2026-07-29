//! Report-object cursors and post-walk object-tree transforms — appending objects to the current
//! section, and the fix-ups that need the whole tree (heading demotion, picture reclassification).

use crate::model::{
    area_objects, area_objects_mut, Area, Rect, ReportObject, ReportObjectKind, TextObject,
};

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
/// the engine's object type.
///
/// A live heading keeps its own stored alignment byte — including `DefaultAlign`, which the engine
/// resolves at paint time from the headed field (see `rpt-layout`'s alignment resolution).
pub(super) fn demote_orphan_headings(areas: &mut [Area]) {
    // The names of the field objects a heading can legitimately head. A heading can head a regular
    // field OR a blob/picture field (e.g. a signature `BlobFieldObject`); both are live targets, so
    // only a truly orphaned link degrades.
    let live_fields: std::collections::HashSet<String> = area_objects(areas)
        .filter(|o| {
            matches!(
                o.kind,
                ReportObjectKind::Field(_) | ReportObjectKind::BlobField(_)
            )
        })
        .map(|o| o.name.clone())
        .collect();
    for obj in area_objects_mut(areas) {
        if let ReportObjectKind::FieldHeading(h) = &obj.kind {
            if !live_fields.contains(h.field_object_name.as_str()) {
                obj.kind = ReportObjectKind::Text(TextObject {
                    text: h.text.clone(),
                    max_lines: h.max_lines,
                    font_color: h.font_color.clone(),
                    ..Default::default()
                });
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
