//! Report object → KDL node mapping (every [`ReportObjectKind`] variant).
//!
//! Each object becomes one node named after its kind, with the object name as the first argument,
//! its bounds as geometry properties, the kind-specific data, and finally the shared
//! [`ObjectFormat`]/border surface applied uniformly by [`apply_common`]. Every object struct is
//! destructured without `..` so a new model field fails to compile until it is emitted or skipped.

use kdl::KdlValue;
use rpt_model::{
    Alignment, Border, BoxShape, ChartDefinition, ChartGraphType, ChartLegendPosition, ChartObject,
    CommonFieldFormat, CrossTabDimension, CrossTabGridOptions, CrossTabObject, DrawingShape,
    FieldHeadingObject, FieldObject, FieldRefKind, FieldValueType, Hyperlink, LineShape,
    ObjectFormat, Paragraph, PictureObject, ReadingOrder, Rect, ReportObject, ReportObjectKind,
    SubreportObject, TextObject, TextRotationAngle, Twips, VerticalAlignment,
};

use crate::build::{color, int, twips, Node};
use crate::enums;
use crate::format;

/// Map one report object to its KDL node.
pub(crate) fn object_node(obj: &ReportObject) -> Node {
    let ReportObject {
        name,
        bounds,
        border,
        format,
        kind,
        // A back-reference to the record this object was decoded from.
        origin: _,
    } = obj;
    let base = match kind {
        ReportObjectKind::Field(f) => field_node(name, bounds, f),
        ReportObjectKind::Text(t) => text_node(name, bounds, t),
        ReportObjectKind::FieldHeading(h) => field_heading_node(name, bounds, h),
        ReportObjectKind::Line(l) => line_node(name, bounds, l),
        ReportObjectKind::Box(b) => box_node(name, bounds, b),
        ReportObjectKind::Picture(p) => picture_node(name, bounds, p),
        ReportObjectKind::Subreport(s) => subreport_object_node(name, bounds, s),
        ReportObjectKind::BlobField(bf) => xywh(Node::new("blob-field").arg(name.as_str()), bounds)
            .prop("src", bf.data_source.as_str()),
        ReportObjectKind::Chart(c) => chart_node(name, bounds, c),
        ReportObjectKind::CrossTab(ct) => crosstab_node(name, bounds, ct),
        ReportObjectKind::OlapGrid => marker_node(name, bounds, "olap-grid"),
        ReportObjectKind::Map => marker_node(name, bounds, "map"),
        ReportObjectKind::Flash => marker_node(name, bounds, "flash"),
        ReportObjectKind::Deferred(code) => {
            xywh(Node::new("raw").arg(name.as_str()), bounds).prop("kind", int(*code))
        }
        ReportObjectKind::Unknown => marker_node(name, bounds, "unknown"),
    };
    apply_common(base, border, format)
}

/// Append `x=`/`y=`/`w=`/`h=` bounds geometry (raw twips).
fn xywh(n: Node, r: &Rect) -> Node {
    n.prop("x", twips(r.left))
        .prop("y", twips(r.top))
        .prop("w", twips(r.width))
        .prop("h", twips(r.height))
}

/// A `raw`-node placeholder for a typed-but-unproduced marker object kind.
fn marker_node(name: &str, bounds: &Rect, kind: &str) -> Node {
    xywh(Node::new("raw").arg(name), bounds).prop("kind", kind)
}

fn field_node(name: &str, bounds: &Rect, f: &FieldObject) -> Node {
    let FieldObject {
        data_source,
        ref_kind,
        value_type,
        font_color,
        format,
        summary_code,
    } = f;
    let mut n = xywh(Node::new("field").arg(name), bounds)
        .prop("src", data_source.as_str())
        .prop_if(
            *ref_kind != FieldRefKind::default(),
            "kind",
            enums::field_ref_kind(*ref_kind),
        )
        .prop_if(
            *value_type != FieldValueType::default(),
            "value-type",
            enums::field_value_type(*value_type),
        )
        .prop_if(
            summary_code.is_some(),
            "summary-code",
            int(summary_code.unwrap_or(0)),
        );
    if let Some(fmt) = format {
        let CommonFieldFormat {
            suppress_if_duplicated,
            use_system_defaults,
        } = &fmt.common;
        n = n
            .flag("suppress-if-duplicated", *suppress_if_duplicated)
            .flag("use-system-defaults", *use_system_defaults)
            .children(format::field_format_nodes(fmt));
    }
    n.child_opt(format::font_node(font_color))
}

fn text_node(name: &str, bounds: &Rect, t: &TextObject) -> Node {
    let TextObject {
        // The last literal run, superseded by the flattened `display`.
        text: _,
        max_lines,
        font_color,
        reading_order,
        // The embedded field references, already present in `paragraphs` / `display`.
        embedded_fields: _,
        display,
        paragraphs,
    } = t;
    let mut n = xywh(Node::new("text").arg(name), bounds)
        .str_if("content", display)
        .prop_if(*max_lines != 0, "max-lines", int(*max_lines))
        .prop_if(
            *reading_order != ReadingOrder::default(),
            "reading-order",
            enums::reading_order(*reading_order),
        )
        .child_opt(format::font_node(font_color));
    // The flattened `content` is enough unless a run carries its own reference or a font that differs
    // from the object's base font.
    let base = &font_color.font;
    if paragraphs.iter().any(|p| paragraph_has_detail(p, base)) {
        n = n.children(paragraphs.iter().map(|p| paragraph_node(p, base)));
    }
    n
}

fn run_font_override<'a>(
    r: &'a rpt_model::TextRun,
    base: &rpt_model::Font,
) -> Option<&'a rpt_model::Font> {
    r.font.as_ref().filter(|f| *f != base)
}

fn paragraph_has_detail(p: &Paragraph, base: &rpt_model::Font) -> bool {
    p.runs.iter().any(|r| {
        r.field_ref.is_some()
            || run_font_override(r, base).is_some()
            || r.character_spacing != Twips(0)
    })
}

fn paragraph_node(p: &Paragraph, base: &rpt_model::Font) -> Node {
    Node::new("paragraph").children(p.runs.iter().map(|r| {
        let mut run = Node::new("run").arg(r.text.as_str());
        if let Some(reference) = &r.field_ref {
            run = run.prop("ref", reference.as_str());
        }
        run = run.prop_if(
            r.character_spacing != Twips(0),
            "char-spacing",
            twips(r.character_spacing),
        );
        if let Some(font) = run_font_override(r, base) {
            run = run.child_opt(format::font_node(&rpt_model::FontColor {
                font: font.clone(),
                ..Default::default()
            }));
        }
        run
    }))
}

fn field_heading_node(name: &str, bounds: &Rect, h: &FieldHeadingObject) -> Node {
    let FieldHeadingObject {
        field_object_name,
        text,
        max_lines,
        font_color,
        reading_order,
    } = h;
    xywh(Node::new("field-heading").arg(name), bounds)
        .prop("for", field_object_name.as_str())
        .str_if("text", text)
        .prop_if(*max_lines != 0, "max-lines", int(*max_lines))
        .prop_if(
            *reading_order != ReadingOrder::default(),
            "reading-order",
            enums::reading_order(*reading_order),
        )
        .child_opt(format::font_node(font_color))
}

/// Append a [`DrawingShape`]'s end-point geometry and stroke, shared by line and box.
fn apply_drawing_shape(n: Node, s: &DrawingShape) -> Node {
    let DrawingShape {
        right,
        bottom,
        line_thickness,
        extend_to_bottom_of_section,
        end_section_index,
    } = s;
    n.prop("x2", twips(*right))
        .prop("y2", twips(*bottom))
        .prop_if(
            *end_section_index != 0,
            "end-section-index",
            int(i64::from(*end_section_index)),
        )
        .prop_if(line_thickness.0 != 0, "thickness", twips(*line_thickness))
        .flag("extend-to-bottom", *extend_to_bottom_of_section)
}

fn line_node(name: &str, bounds: &Rect, l: &LineShape) -> Node {
    let LineShape { shape } = l;
    let n = Node::new("line")
        .arg(name)
        .prop("x", twips(bounds.left))
        .prop("y", twips(bounds.top));
    apply_drawing_shape(n, shape)
}

fn box_node(name: &str, bounds: &Rect, b: &BoxShape) -> Node {
    let BoxShape {
        shape,
        corner_ellipse_width,
        corner_ellipse_height,
    } = b;
    let n = Node::new("box")
        .arg(name)
        .prop("x", twips(bounds.left))
        .prop("y", twips(bounds.top));
    apply_drawing_shape(n, shape)
        .prop_if(
            corner_ellipse_width.0 != 0,
            "corner-width",
            twips(*corner_ellipse_width),
        )
        .prop_if(
            corner_ellipse_height.0 != 0,
            "corner-height",
            twips(*corner_ellipse_height),
        )
}

fn picture_node(name: &str, bounds: &Rect, p: &PictureObject) -> Node {
    let PictureObject {
        picture_type,
        // The binary payload never enters the KDL: `source` references a sidecar file (see `assets`).
        data,
        ole_ordinal,
        location_formula,
        crop_top,
        crop_bottom,
        crop_left,
        crop_right,
    } = p;
    let mut n = xywh(Node::new("picture").arg(name), bounds)
        .prop_if(
            *picture_type != rpt_model::PictureType::default(),
            "picture-type",
            enums::picture_type(*picture_type),
        )
        .prop_if(crop_top.0 != 0, "crop-top", twips(*crop_top))
        .prop_if(crop_bottom.0 != 0, "crop-bottom", twips(*crop_bottom))
        .prop_if(crop_left.0 != 0, "crop-left", twips(*crop_left))
        .prop_if(crop_right.0 != 0, "crop-right", twips(*crop_right));
    if let Some(reference) = crate::assets::picture_reference(p, name) {
        n = n
            .prop("source", reference)
            .prop("bytes", int(data.len() as i64));
    }
    if let Some(ord) = ole_ordinal {
        n = n.prop("ole-ordinal", int(*ord as i64));
    }
    if let Some(f) = location_formula {
        n = n.prop("location-formula", f.0.as_str());
    }
    n
}

fn subreport_object_node(name: &str, bounds: &Rect, s: &SubreportObject) -> Node {
    let SubreportObject {
        subreport_name,
        on_demand,
        is_imported,
        // `enable_reimport` is unpinned (always false); the reimport policy is not a KDL surface.
        enable_reimport: _,
        // Internal `Subdocument N` storage ordinal; the subreport is referenced by name.
        subdoc_index: _,
        links,
    } = s;
    xywh(Node::new("subreport-object").arg(name), bounds)
        .prop("target", subreport_name.as_str())
        .flag("on-demand", *on_demand)
        .flag("imported", *is_imported)
        .children(links.iter().map(|l| {
            Node::new("link")
                .prop("main", l.main_report_field.as_str())
                .prop("sub", l.subreport_field.as_str())
                .opt_str("param", l.linked_parameter.as_deref())
        }))
}

fn chart_node(name: &str, bounds: &Rect, c: &ChartObject) -> Node {
    let ChartObject {
        data_refs,
        category_refs,
        definition,
    } = c;
    let ChartDefinition {
        layout_type,
        graph_type,
        graph_subtype,
        title,
        subtitle,
        footnote,
        group_axis_title,
        data_axis_title,
        data_label,
        category_period,
        legend_visible,
        legend_position,
        group_axis_gridlines,
        value_axis_gridlines,
        data_labels_show_value,
        series_colors,
        view_angle,
        // RAS `DataFields`/`ConditionFields` summary-form strings are not emitted to KDL — the chart's
        // bare data/category refs (from `ChartObject` above) are the KDL surface.
        ..
    } = definition;
    let mut n = xywh(Node::new("chart").arg(name), bounds)
        .prop_if(
            *layout_type != rpt_model::ChartLayoutType::default(),
            "layout-type",
            enums::chart_layout_type(*layout_type),
        )
        .prop_if(
            *graph_type != ChartGraphType::default(),
            "graph-type",
            enums::chart_graph_type(*graph_type),
        )
        .prop_if(*graph_subtype != 0, "subtype", int(*graph_subtype))
        .str_if("title", title)
        .str_if("subtitle", subtitle)
        .str_if("footnote", footnote)
        .str_if("group-axis-title", group_axis_title)
        .str_if("data-axis-title", data_axis_title)
        .str_if("data-label", data_label)
        .flag("legend", *legend_visible)
        .prop_if(
            *legend_position != ChartLegendPosition::default(),
            "legend-position",
            enums::chart_legend_position(*legend_position),
        )
        .prop_if(
            *group_axis_gridlines != rpt_model::ChartGridType::default(),
            "group-gridlines",
            enums::chart_grid_type(*group_axis_gridlines),
        )
        .prop_if(
            *value_axis_gridlines != rpt_model::ChartGridType::default(),
            "value-gridlines",
            enums::chart_grid_type(*value_axis_gridlines),
        )
        .flag("show-data-labels", *data_labels_show_value)
        .prop_if(
            *view_angle != rpt_model::ChartViewAngle::default(),
            "view-angle",
            enums::chart_view_angle(*view_angle),
        );
    if let Some(period) = category_period {
        n = n.prop("category-period", enums::chart_category_period(*period));
    }
    n.children(data_refs.iter().map(|r| Node::new("data").arg(r.as_str())))
        .children(
            category_refs
                .iter()
                .map(|r| Node::new("category").arg(r.as_str())),
        )
        .children(
            series_colors
                .iter()
                .map(|c| Node::new("series-color").arg(color(*c))),
        )
}

fn crosstab_node(name: &str, bounds: &Rect, ct: &CrossTabObject) -> Node {
    let CrossTabObject {
        // The row/column bindings, redundant with the `columns` / `rows` axis-split view below.
        field_refs: _,
        // The combined dimension list, a superset of `columns` + `rows`.
        dimensions: _,
        columns,
        rows,
        // Engine-internal grid-region format template; not a render/authoring fact.
        grid_format: _,
        column_level_count,
        row_level_count,
        options,
    } = ct;
    let CrossTabGridOptions {
        show_grid,
        show_cell_margins,
        keep_columns_together,
        repeat_row_labels,
        suppress_empty_rows,
        suppress_empty_columns,
        suppress_row_grand_totals,
        suppress_column_grand_totals,
        row_grand_total_color,
        column_grand_total_color,
    } = options;
    let mut n = xywh(Node::new("crosstab").arg(name), bounds)
        .flag("show-grid", *show_grid)
        .flag("show-cell-margins", *show_cell_margins)
        .flag("keep-columns-together", *keep_columns_together)
        .flag("repeat-row-labels", *repeat_row_labels)
        .flag("suppress-empty-rows", *suppress_empty_rows)
        .flag("suppress-empty-columns", *suppress_empty_columns)
        .flag("suppress-row-grand-totals", *suppress_row_grand_totals)
        .flag(
            "suppress-column-grand-totals",
            *suppress_column_grand_totals,
        )
        .prop_if(
            *column_level_count != 0,
            "column-level-count",
            int(*column_level_count),
        )
        .prop_if(
            *row_level_count != 0,
            "row-level-count",
            int(*row_level_count),
        );
    if let Some(c) = row_grand_total_color {
        n = n.prop("row-grand-total-color", color(*c));
    }
    if let Some(c) = column_grand_total_color {
        n = n.prop("column-grand-total-color", color(*c));
    }
    let dim = |node_name: &'static str| {
        move |d: &CrossTabDimension| {
            Node::new(node_name)
                .arg(d.field_ref.as_str())
                .flag("suppress-subtotal", d.suppress_subtotal)
                .flag("suppress-label", d.suppress_label)
        }
    };
    // No measure children: a cross-tab's measures are the report's own summary definitions, which
    // this document already states as `summary` field definitions. Repeating them here would assert
    // an attribution the file does not record.
    n.children(
        rows.iter()
            .filter(|d| !d.field_ref.is_empty())
            .map(dim("row")),
    )
    .children(
        columns
            .iter()
            .filter(|d| !d.field_ref.is_empty())
            .map(dim("column")),
    )
}

/// Append the shared [`ObjectFormat`] / border surface every object carries.
fn apply_common(n: Node, border: &Border, fmt: &ObjectFormat) -> Node {
    let ObjectFormat {
        suppress,
        can_grow,
        keep_together,
        close_at_page_break,
        horizontal_alignment,
        vertical_alignment,
        css_class,
        hyperlink,
        tooltip_text,
        text_rotation,
        condition_formulas,
    } = fmt;
    let mut n = n
        .flag("can-grow", *can_grow)
        .flag("keep-together", *keep_together)
        .flag("close-at-page-break", *close_at_page_break)
        .prop_if(
            *horizontal_alignment != Alignment::default(),
            "align",
            enums::alignment(*horizontal_alignment),
        )
        .prop_if(
            *vertical_alignment != VerticalAlignment::default(),
            "valign",
            enums::vertical_alignment(*vertical_alignment),
        )
        .opt_str("css-class", css_class.as_deref())
        .opt_str("tooltip", tooltip_text.as_deref());
    if let Some(rot) = rotation(*text_rotation) {
        n = n.prop("rotate", rot);
    }
    // The suppress property carries its conditional formula as an adjacent `when=` (Conditioned<T>).
    if suppress.value || suppress.formula.is_some() {
        n = n.prop("suppress", suppress.value);
        if let Some(f) = &suppress.formula {
            n = n.prop("when", f.0.as_str());
        }
    }
    if let Some(h) = hyperlink {
        let Hyperlink { text, kind } = h;
        n = n.child(
            Node::new("hyperlink")
                .arg(text.as_str())
                .prop("type", enums::hyperlink_type(*kind)),
        );
    }
    n.child_opt(format::border_node(border))
        .children(format::condition_formula_nodes(condition_formulas))
}

/// The text-rotation angle as a bare-integer degree value, or `None` when unrotated (the default).
fn rotation(r: TextRotationAngle) -> Option<KdlValue> {
    match r {
        TextRotationAngle::Rotate0 => None,
        TextRotationAngle::Rotate90 => Some(int(90)),
        TextRotationAngle::Rotate270 => Some(int(270)),
        TextRotationAngle::Other(c) => Some(int(c)),
    }
}
