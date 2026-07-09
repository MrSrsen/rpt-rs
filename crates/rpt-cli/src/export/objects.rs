//! The `<ReportDefinition>` areas/sections/objects and their format children.

use std::fmt::Write as _;

use rpt::model::{Area, Node, ReportObject, ReportObjectKind, Section, Unknown, Value};

use crate::export::colors::{write_color, BLACK};
use crate::export::util::{b, escape, escape_text};

/// `<ObjectFormatConditionFormulas>` attribute order (subset of the SDK enum we decode).
const OBJECT_COND_ORDER: &[&str] = &["EnableSuppress", "DisplayString", "GraphicLocation"];
/// `<SectionAreaConditionFormulas>` attribute order (SDK enum order).
const SECTION_COND_ORDER: &[&str] = &["EnableSuppress", "EnableNewPageAfter", "BackgroundColor"];
/// `<FontColorConditionFormulas>` attribute order (SDK enum order).
const FONT_COND_ORDER: &[&str] = &["Color", "Style"];
/// `<BorderConditionFormulas>` attribute order (SDK enum order).
const BORDER_COND_ORDER: &[&str] = &["BackgroundColor", "BorderColor"];

/// Emit a `<…ConditionFormulas>` element: an empty self-closing tag when no conditional-format
/// formula is attached, otherwise a self-closing tag carrying one attribute per present property
/// (in `order`), the value being the escaped formula text.
fn write_condition_formulas(
    o: &mut String,
    pad: &str,
    tag: &str,
    order: &[&str],
    formulas: &[(String, String)],
) {
    if formulas.is_empty() {
        let _ = writeln!(o, "{pad}<{tag} />");
        return;
    }
    let _ = write!(o, "{pad}<{tag}");
    for &name in order {
        if let Some((_, text)) = formulas.iter().find(|(a, _)| a == name) {
            let _ = write!(o, " {name}=\"{}\"", escape(text));
        }
    }
    let _ = writeln!(o, " />");
}

pub(crate) fn write_area(o: &mut String, report: &rpt::model::Report, area: &Area, depth: usize) {
    let pad = "  ".repeat(depth);
    let _ = writeln!(
        o,
        "{pad}<Area Kind=\"{:?}\" Name=\"{}\">",
        area.kind,
        escape(&area.name)
    );
    // AreaFormat — decoded boolean flags. Group *header* areas additionally nest a
    // <GroupAreaFormat> child (decoded from the group's `0x0088` record; the outermost group has
    // none, so it keeps defaults); group footers do not.
    use rpt::model::AreaSectionKind::GroupHeader;
    let is_group = matches!(area.kind, GroupHeader);
    let af = &area.format;
    let area_format = format!(
        "AreaFormat EnableHideForDrillDown=\"{}\" EnableKeepTogether=\"{}\" EnableNewPageAfter=\"{}\" EnableNewPageBefore=\"{}\" EnablePrintAtBottomOfPage=\"{}\" EnableResetPageNumberAfter=\"{}\" EnableSuppress=\"{}\"",
        b(af.hide_for_drill_down),
        b(af.base.keep_together),
        b(af.base.new_page_after),
        b(af.base.new_page_before),
        b(af.base.print_at_bottom_of_page),
        b(af.base.reset_page_number_after),
        b(af.base.suppress),
    );
    if is_group {
        let g = af.group.unwrap_or_default();
        let _ = writeln!(o, "{pad}  <{area_format}>");
        let _ = writeln!(
            o,
            "{pad}    <GroupAreaFormat EnableKeepGroupTogether=\"{}\" EnableRepeatGroupHeader=\"{}\" VisibleGroupNumberPerPage=\"{}\" />",
            b(g.keep_group_together),
            b(g.repeat_group_header),
            g.visible_groups_per_page,
        );
        let _ = writeln!(o, "{pad}  </AreaFormat>");
    } else {
        let _ = writeln!(o, "{pad}  <{area_format} />");
    }
    let _ = writeln!(o, "{pad}  <Sections>");
    for section in &area.sections {
        write_section(o, report, section, depth + 2);
    }
    let _ = writeln!(o, "{pad}  </Sections>");
    let _ = writeln!(o, "{pad}</Area>");
}

pub(crate) fn write_section(
    o: &mut String,
    report: &rpt::model::Report,
    section: &Section,
    depth: usize,
) {
    let pad = "  ".repeat(depth);
    // Section Name + Height are decoded from the section record; the SectionFormat flags default.
    let _ = writeln!(
        o,
        "{pad}<Section Height=\"{}\" Kind=\"{:?}\" Name=\"{}\">",
        section.height.0,
        section.kind,
        escape(&section.name)
    );
    let sf = &section.format;
    let _ = writeln!(
        o,
        "{pad}  <SectionFormat CssClass=\"\" EnableKeepTogether=\"{}\" EnableNewPageAfter=\"{}\" EnableNewPageBefore=\"{}\" EnablePrintAtBottomOfPage=\"{}\" EnableResetPageNumberAfter=\"{}\" EnableSuppress=\"{}\" EnableSuppressIfBlank=\"{}\" EnableUnderlaySection=\"{}\" PageOrientation=\"{:?}\">",
        b(sf.base.keep_together),
        b(sf.base.new_page_after),
        b(sf.base.new_page_before),
        b(sf.base.print_at_bottom_of_page),
        b(sf.base.reset_page_number_after),
        b(sf.base.suppress),
        b(sf.suppress_if_blank),
        b(sf.underlay_section),
        // Section.SectionFormat.PageOrientation is a per-section override that inherits the report
        // orientation when unset; our decode never populates the field, so `None` resolves to the
        // engine's `DefaultPaperOrientation`.
        sf.page_orientation
            .unwrap_or(rpt::model::PaperOrientation::DefaultPaperOrientation),
    );
    write_condition_formulas(
        o,
        &format!("{pad}    "),
        "SectionAreaConditionFormulas",
        SECTION_COND_ORDER,
        &section.condition_formulas,
    );
    write_color(
        o,
        &format!("{pad}    "),
        "BackgroundColor",
        sf.background_color
            .as_ref()
            .unwrap_or(&rpt::model::Color::WHITE),
    );
    let _ = writeln!(o, "{pad}  </SectionFormat>");
    let _ = writeln!(o, "{pad}  <ReportObjects>");
    for obj in &section.objects {
        // The ReportObjects collection never exposes an unnamed object: the cell objects inside a
        // cross-tab/grid carry no ObjectName record and are folded into the CrossTabObject, not
        // emitted standalone. The file still contains them (rpt decodes them); skip any object with
        // no name.
        if obj.name.is_empty() {
            continue;
        }
        write_object(o, report, obj, &section.name, depth + 2);
    }
    let _ = writeln!(o, "{pad}  </ReportObjects>");
    let _ = writeln!(o, "{pad}</Section>");
}

/// Emit one `<…Object>` element. A clean dispatch over the object kind: the shared geometry/head
/// and alignment are resolved once, then each kind is written by its own function.
pub(crate) fn write_object(
    o: &mut String,
    report: &rpt::model::Report,
    obj: &ReportObject,
    section_name: &str,
    depth: usize,
) {
    let align = object_align(obj);
    match &obj.kind {
        ReportObjectKind::Text(t) => write_text_object(o, report, obj, t, &align, depth),
        ReportObjectKind::Field(f) => write_field_object(o, report, obj, f, &align, depth),
        ReportObjectKind::FieldHeading(h) => {
            write_field_heading_object(o, report, obj, h, &align, depth)
        }
        ReportObjectKind::Line(_) | ReportObjectKind::Box(_) => {
            write_line_or_box_object(o, obj, section_name, depth)
        }
        ReportObjectKind::Subreport(s) => write_subreport_object(o, obj, s, &align, depth),
        ReportObjectKind::Picture(_) => write_plain_object(
            o,
            "PictureObject",
            &object_head(obj, "PictureObject"),
            obj,
            &align,
            depth,
        ),
        ReportObjectKind::Chart(_) => write_plain_object(
            o,
            "ChartObject",
            &object_head(obj, "ChartObject"),
            obj,
            &align,
            depth,
        ),
        ReportObjectKind::BlobField(_) => write_plain_object(
            o,
            "BlobFieldObject",
            &object_head(obj, "BlobFieldObject"),
            obj,
            &align,
            depth,
        ),
        ReportObjectKind::CrossTab(_)
        | ReportObjectKind::OlapGrid
        | ReportObjectKind::Map
        | ReportObjectKind::Flash => write_grid_like_object(o, obj, &align, depth),
        other => {
            let pad = "  ".repeat(depth);
            let _ = writeln!(o, "{pad}<ReportObject Kind=\"{other:?}\" />");
        }
    }
}

/// The opening attributes every report object shares, in order: Name, Kind, then the geometry
/// (twips) from the ObjectName (0x9e) + position (0xbe) records.
fn object_head(obj: &ReportObject, kind: &str) -> String {
    let r = &obj.bounds;
    format!(
        "Name=\"{}\" Kind=\"{}\" Top=\"{}\" Left=\"{}\" Width=\"{}\" Height=\"{}\"",
        escape(&obj.name),
        kind,
        r.top.0,
        r.left.0,
        r.width.0,
        // A line drawn bottom-to-top stores a negative (signed `0x9e`) height; the engine reports
        // the magnitude. Every other object has a non-negative height, so `abs` is a no-op there.
        r.height.0.abs()
    )
}

/// The `HorizontalAlignment` value for an object. A plain text object's default alignment renders
/// as LeftAlign; every other kind emits its stored alignment verbatim (a field keeps DefaultAlign,
/// and a field heading's alignment is already resolved in `raise` from the field's value type).
fn object_align(obj: &ReportObject) -> String {
    let is_text = matches!(&obj.kind, ReportObjectKind::Text(_));
    match obj.format.horizontal_alignment {
        rpt::model::Alignment::DefaultAlign if is_text => "LeftAlign".to_string(),
        a => format!("{a:?}"),
    }
}

fn write_text_object(
    o: &mut String,
    report: &rpt::model::Report,
    obj: &ReportObject,
    t: &rpt::model::TextObject,
    align: &str,
    depth: usize,
) {
    let pad = "  ".repeat(depth);
    let _ = writeln!(
        o,
        "{pad}<TextObject {} MaxNumberOfLines=\"0\">",
        object_head(obj, "TextObject")
    );
    // `display` carries the full content (literals + inline `{field}` refs in order); fall back to
    // `text` only if no runs were recorded.
    let content = if t.display.is_empty() {
        &t.text
    } else {
        &t.display
    };
    let _ = writeln!(o, "{pad}  <Text>{}</Text>", escape_text(content));
    write_object_format(
        o,
        report,
        &t.font_color,
        &obj.border,
        align,
        &obj.format,
        None,
        depth + 1,
    );
    let _ = writeln!(o, "{pad}</TextObject>");
}

fn write_field_object(
    o: &mut String,
    report: &rpt::model::Report,
    obj: &ReportObject,
    f: &rpt::model::FieldObject,
    align: &str,
    depth: usize,
) {
    let pad = "  ".repeat(depth);
    let _ = writeln!(
        o,
        "{pad}<FieldObject {} DataSource=\"{}\">",
        object_head(obj, "FieldObject"),
        escape(&f.data_source)
    );
    write_object_format(
        o,
        report,
        &f.font_color,
        &obj.border,
        align,
        &obj.format,
        Some(f),
        depth + 1,
    );
    let _ = writeln!(o, "{pad}</FieldObject>");
}

fn write_field_heading_object(
    o: &mut String,
    report: &rpt::model::Report,
    obj: &ReportObject,
    h: &rpt::model::FieldHeadingObject,
    align: &str,
    depth: usize,
) {
    let pad = "  ".repeat(depth);
    let _ = writeln!(
        o,
        "{pad}<FieldHeadingObject {} FieldObjectName=\"{}\" MaxNumberOfLines=\"0\">",
        object_head(obj, "FieldHeadingObject"),
        escape(&h.field_object_name)
    );
    let _ = writeln!(o, "{pad}  <Text>{}</Text>", escape_text(&h.text));
    write_object_format(
        o,
        report,
        &h.font_color,
        &obj.border,
        align,
        &obj.format,
        None,
        depth + 1,
    );
    let _ = writeln!(o, "{pad}</FieldHeadingObject>");
}

/// `0xa9` is both line and box (distinguished in `raise` by geometry); they share the same element
/// shape: degenerate-rectangle attrs plus shape format children.
fn write_line_or_box_object(o: &mut String, obj: &ReportObject, section_name: &str, depth: usize) {
    let pad = "  ".repeat(depth);
    let r = &obj.bounds;
    let is_box = matches!(&obj.kind, ReportObjectKind::Box(_));
    let tag = if is_box { "BoxObject" } else { "LineObject" };
    let right = r.left.0 + r.width.0;
    // A cross-section box uses its resolved end section, and its Bottom is the opener's
    // end-relative value (not top+height, which would span sections). A normal box/line ends
    // in its own section with Bottom = top + height.
    let (end_section, bottom) = match &obj.kind {
        ReportObjectKind::Box(bx) if !bx.end_section_name.is_empty() => {
            (bx.end_section_name.as_str(), bx.shape.bottom.0)
        }
        // A bottom-to-top line (negative height) ends at its anchor top, so its Bottom is
        // the top; `max(0)` keeps the normal `top + height` for non-negative heights.
        _ => (section_name, r.top.0 + r.height.0.max(0)),
    };
    // LineThickness (twips) is decoded from `0xec` byte 21; line and box are separate
    // model variants so the shape is read from whichever applies.
    let (line_thickness, extend_to_bottom) = match &obj.kind {
        ReportObjectKind::Line(l) => (
            l.shape.line_thickness.0,
            l.shape.extend_to_bottom_of_section,
        ),
        ReportObjectKind::Box(bx) => (
            bx.shape.line_thickness.0,
            bx.shape.extend_to_bottom_of_section,
        ),
        _ => (0, false),
    };
    let _ = writeln!(
        o,
        "{pad}<{tag} {} Bottom=\"{bottom}\" EnableExtendToBottomOfSection=\"{}\" EndSectionName=\"{}\" LineStyle=\"{}\" LineThickness=\"{line_thickness}\" Right=\"{right}\">",
        object_head(obj, tag),
        b(extend_to_bottom),
        escape(end_section),
        shape_line_style(&obj.border),
    );
    if is_box {
        write_shape_format(o, &obj.border, depth + 1);
    } else {
        // A line's stroke is reported on the single border edge matching its orientation:
        // a horizontal line (wider than tall) on Top, a vertical line on Left.
        use rpt::model::LineStyle::NoLine;
        let style = shape_line_style_enum(&obj.border);
        let horizontal = r.height.0 <= r.width.0;
        let mut line_border = obj.border.clone();
        line_border.top = if horizontal { style } else { NoLine };
        line_border.left = if horizontal { NoLine } else { style };
        line_border.bottom = NoLine;
        line_border.right = NoLine;
        write_shape_format(o, &line_border, depth + 1);
    }
    let _ = writeln!(o, "{pad}</{tag}>");
}

fn write_subreport_object(
    o: &mut String,
    obj: &ReportObject,
    s: &rpt::model::SubreportObject,
    align: &str,
    depth: usize,
) {
    let pad = "  ".repeat(depth);
    // The placeholder for an embedded subreport; the nested report itself is emitted under
    // <SubReports>. SubreportName is not stored in Contents, so it is left empty.
    let _ = writeln!(
        o,
        "{pad}<SubreportObject {} SubreportName=\"{}\" EnableOnDemand=\"{}\">",
        object_head(obj, "SubreportObject"),
        escape(&s.subreport_name),
        b(s.on_demand),
    );
    write_border(o, &obj.border, depth + 1);
    let _ = writeln!(
        o,
        "{pad}  <ObjectFormat CssClass=\"\" EnableCanGrow=\"True\" EnableCloseAtPageBreak=\"True\" EnableKeepTogether=\"True\" EnableSuppress=\"False\" HorizontalAlignment=\"{align}\" />"
    );
    // A subreport object can carry conditional-format formulas (e.g. a conditional Suppress
    // keyed on a parameter); emit them like any other object rather than a fixed empty list.
    write_condition_formulas(
        o,
        &format!("{pad}  "),
        "ObjectFormatConditionFormulas",
        OBJECT_COND_ORDER,
        &obj.format.condition_formulas,
    );
    let _ = writeln!(o, "{pad}</SubreportObject>");
}

/// Grid/map/flash objects share the field/box body shape: border, the decoded ObjectFormat (these
/// carry their own CanGrow/KeepTogether flags, unlike the plain picture/chart objects), then the
/// object-format condition formulas. The internal bindings are not modelled here.
fn write_grid_like_object(o: &mut String, obj: &ReportObject, align: &str, depth: usize) {
    let pad = "  ".repeat(depth);
    let tag = match &obj.kind {
        ReportObjectKind::CrossTab(_) => "CrossTabObject",
        ReportObjectKind::OlapGrid => "OlapGridObject",
        ReportObjectKind::Map => "MapObject",
        _ => "FlashObject",
    };
    let _ = writeln!(o, "{pad}<{tag} {}>", object_head(obj, tag));
    write_border(o, &obj.border, depth + 1);
    write_object_format_element(o, &format!("{pad}  "), align, &obj.format);
    write_condition_formulas(
        o,
        &format!("{pad}  "),
        "ObjectFormatConditionFormulas",
        OBJECT_COND_ORDER,
        &obj.format.condition_formulas,
    );
    let _ = writeln!(o, "{pad}</{tag}>");
}

/// Emit a picture / chart / blob-field object: geometry attributes plus the shared border and
/// object-format children (these three `0xae`-family kinds carry no text, colour or extra attrs).
fn write_plain_object(
    o: &mut String,
    tag: &str,
    head: &str,
    obj: &ReportObject,
    align: &str,
    depth: usize,
) {
    let pad = "  ".repeat(depth);
    let _ = writeln!(o, "{pad}<{tag} {head}>");
    write_border(o, &obj.border, depth + 1);
    // EnableCanGrow is decoded (a dynamic/CanGrow picture sets it True); the rest stay at the
    // plain-object defaults.
    let _ = writeln!(
        o,
        "{pad}  <ObjectFormat CssClass=\"\" EnableCanGrow=\"{}\" EnableCloseAtPageBreak=\"True\" EnableKeepTogether=\"True\" EnableSuppress=\"False\" HorizontalAlignment=\"{align}\" />",
        b(obj.format.can_grow)
    );
    // A PictureObject's dynamic graphic-location formula (and any other decoded object-format
    // condition) is carried here; empty for plain pictures/charts.
    write_condition_formulas(
        o,
        &format!("{pad}  "),
        "ObjectFormatConditionFormulas",
        OBJECT_COND_ORDER,
        &obj.format.condition_formulas,
    );
    let _ = writeln!(o, "{pad}</{tag}>");
}

/// Emit an object's format children — font colour, font, border (with background/border colours)
/// and object format. Colours and border line styles are decoded from the records; alignment from
/// the object-format record.
// The emitter mirrors one object's several independent format record groups (font colour, font,
// border, background, object-format); grouping them into a struct would only relocate the arity.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_object_format(
    o: &mut String,
    report: &rpt::model::Report,
    fc: &rpt::model::FontColor,
    border: &rpt::model::Border,
    align: &str,
    format: &rpt::model::ObjectFormat,
    field: Option<&rpt::model::FieldObject>,
    depth: usize,
) {
    let pad = "  ".repeat(depth);
    write_color(o, &pad, "Color", &fc.color);
    write_font(o, &fc.font, depth);
    write_condition_formulas(
        o,
        &pad,
        "FontColorConditionFormulas",
        FONT_COND_ORDER,
        &fc.condition_formulas,
    );
    if let Some(f) = field {
        write_field_format(o, report, &pad, f);
    }
    write_border(o, border, depth);
    write_object_format_element(o, &pad, align, format);
    write_condition_formulas(
        o,
        &pad,
        "ObjectFormatConditionFormulas",
        OBJECT_COND_ORDER,
        &format.condition_formulas,
    );
}

/// Emit a field object's `<FieldFormat>` (SDK `FieldObject.FieldFormat`), streamed between the
/// `<FontColorConditionFormulas>` and `<Border>` of the object body.
///
/// The engine returns all six sub-formats for every field. The **byte-derived** ones — Common,
/// Numeric and Boolean — come straight from the decoded [`rpt::model::FieldFormat`]. The **Date**
/// sub-format is the field's stored `0x00f2` day/month/year for a date-valued non-system-default
/// field, the "active" (numeric day/month, long year) form for a system-default date field, and the
/// generic (leading-zero / short-year) default for a non-date field — resolved in
/// `crate::export::analysis::field_format`. The **Time** sub-format stays host-locale gated (selected by the
/// [`RenderLocale`](crate::export::analysis::field_format::RenderLocale)); `EnableUseLeadingZero` is likewise
/// value-type-derived (true for the numeric types).
fn write_field_format(
    o: &mut String,
    report: &rpt::model::Report,
    pad: &str,
    field: &rpt::model::FieldObject,
) {
    let Some(ff) = &field.format else { return };
    // The engine does not store a per-field date/time format nor the numeric defaults for a
    // non-numeric field: it *resolves* them at runtime from the field's effective value type (and
    // host locale). The `analysis` module reconstructs that resolution — the derive-layer counterpart
    // to the stored decode in `rpt`. This resolves the value type of formula/parameter/special/… field
    // objects (whose object `value_type` is `Unknown` — only their definition carries it).
    let eff = crate::export::analysis::field_format::effective_field_format(
        report,
        field,
        &crate::export::analysis::field_format::RenderLocale::default(),
    );
    let d = eff.date;
    let t = eff.time;
    let date_el = |p: &str| {
        format!(
            "{p}<DateFieldFormat DayFormat=\"{:?}\" MonthFormat=\"{:?}\" YearFormat=\"{:?}\" />",
            d.day, d.month, d.year
        )
    };
    let time_el = |p: &str| {
        format!(
            "{p}<TimeFieldFormat HourFormat=\"{}\" MinuteFormat=\"{}\" SecondFormat=\"{}\" AMPMFormat=\"{}\" AMString=\"{}\" PMString=\"{}\" HourMinuteSeparator=\"{}\" MinuteSecondSeparator=\"{}\" TimeBase=\"{}\" />",
            t.hour, t.minute, t.second, t.ampm, t.am_string, t.pm_string, t.hour_minute_sep, t.minute_second_sep, t.time_base
        )
    };
    let p1 = format!("{pad}  ");
    let p2 = format!("{pad}    ");
    let _ = writeln!(o, "{pad}<FieldFormat>");
    let _ = writeln!(
        o,
        "{p1}<CommonFieldFormat EnableSuppressIfDuplicated=\"{}\" EnableUseSystemDefaults=\"{}\" />",
        b(ff.common.suppress_if_duplicated),
        b(ff.common.use_system_defaults),
    );
    let _ = writeln!(
        o,
        "{p1}<NumericFieldFormat DecimalPlaces=\"{}\" RoundingFormat=\"{:?}\" NegativeFormat=\"{:?}\" CurrencySymbolFormat=\"{:?}\" EnableUseLeadingZero=\"{}\" />",
        eff.numeric.decimal_places,
        eff.numeric.rounding,
        eff.numeric.negative,
        eff.numeric.currency_symbol,
        b(eff.numeric.leading_zero),
    );
    let _ = writeln!(o, "{}", date_el(&p1));
    let _ = writeln!(o, "{}", time_el(&p1));
    let _ = writeln!(o, "{p1}<DateTimeFieldFormat DateTimeSeparator=\"  \">");
    let _ = writeln!(o, "{}", date_el(&p2));
    let _ = writeln!(o, "{}", time_el(&p2));
    let _ = writeln!(o, "{p1}</DateTimeFieldFormat>");
    let _ = writeln!(
        o,
        "{p1}<BooleanFieldFormat OutputType=\"{:?}\" />",
        ff.boolean.output_type,
    );
    let _ = writeln!(o, "{pad}</FieldFormat>");
}

/// Emit the bare `<ObjectFormat … />` element from a decoded [`rpt::model::ObjectFormat`].
/// `EnableCloseAtPageBreak` is held constant `True`.
fn write_object_format_element(
    o: &mut String,
    pad: &str,
    align: &str,
    format: &rpt::model::ObjectFormat,
) {
    let _ = writeln!(
        o,
        "{pad}<ObjectFormat CssClass=\"\" EnableCanGrow=\"{}\" EnableCloseAtPageBreak=\"True\" EnableKeepTogether=\"{}\" EnableSuppress=\"{}\" HorizontalAlignment=\"{align}\" />",
        b(format.can_grow),
        b(format.keep_together),
        b(format.suppress.value),
    );
}

/// Emit a `<Border>` element with its line styles, condition formulas, background and border
/// colours — shared by text/field objects and line/box shapes.
fn write_border(o: &mut String, border: &rpt::model::Border, depth: usize) {
    let pad = "  ".repeat(depth);
    let pad2 = "  ".repeat(depth + 1);
    let _ = writeln!(
        o,
        "{pad}<Border BottomLineStyle=\"{:?}\" HasDropShadow=\"{}\" LeftLineStyle=\"{:?}\" RightLineStyle=\"{:?}\" TopLineStyle=\"{:?}\">",
        border.bottom, b(border.has_drop_shadow), border.left, border.right, border.top
    );
    write_condition_formulas(
        o,
        &format!("{pad}  "),
        "BorderConditionFormulas",
        BORDER_COND_ORDER,
        &border.condition_formulas,
    );
    write_color(
        o,
        &pad2,
        "BackgroundColor",
        border
            .background_color
            .as_ref()
            .unwrap_or(&rpt::model::Color::WHITE),
    );
    write_color(
        o,
        &pad2,
        "BorderColor",
        border.border_color.as_ref().unwrap_or(&BLACK),
    );
    let _ = writeln!(o, "{pad}</Border>");
}

/// Emit the format children of a line/box shape — `<LineColor>`, `<Border>`, `<ObjectFormat>`
/// and `<ObjectFormatConditionFormulas>`. Shapes carry no font/colour text.
fn write_shape_format(o: &mut String, border: &rpt::model::Border, depth: usize) {
    let pad = "  ".repeat(depth);
    write_color(
        o,
        &pad,
        "LineColor",
        border.border_color.as_ref().unwrap_or(&BLACK),
    );
    write_border(o, border, depth);
    let _ = writeln!(
        o,
        "{pad}<ObjectFormat CssClass=\"\" EnableCanGrow=\"False\" EnableCloseAtPageBreak=\"True\" EnableKeepTogether=\"True\" EnableSuppress=\"False\" HorizontalAlignment=\"DefaultAlign\" />"
    );
    let _ = writeln!(o, "{pad}<ObjectFormatConditionFormulas />");
}

/// The shape's own line style: the first non-`NoLine` border side (a box uses one style all round).
/// When every side is `NoLine` the shape has no line, so it reports `NoLine` (a filled box with a
/// background but no border, or a hidden line). All real line objects carry their style on the edge
/// matching their orientation, so the fallback only applies to genuinely border-less boxes.
fn shape_line_style_enum(border: &rpt::model::Border) -> rpt::model::LineStyle {
    use rpt::model::LineStyle::NoLine;
    [border.top, border.bottom, border.left, border.right]
        .into_iter()
        .find(|&s| s != NoLine)
        .unwrap_or(NoLine)
}

fn shape_line_style(border: &rpt::model::Border) -> String {
    format!("{:?}", shape_line_style_enum(border))
}

pub(crate) fn write_font(o: &mut String, f: &rpt::model::Font, depth: usize) {
    if f.name.is_empty() {
        return;
    }
    let pad = "  ".repeat(depth);
    let size = f.size_pt as i32;
    // Emits a System.Drawing.Font's properties. The rendering environment resolves every typeface
    // to Tahoma, so Name/FontFamily are "Tahoma" while OriginalFontName preserves the requested
    // name; Height is the resolved font's line spacing in pixels (the Tahoma GDI metric, a fixed
    // function of Size). GdiCharSet/Unit/etc. are constants.
    let _ = writeln!(
        o,
        "{pad}<Font Bold=\"{bold}\" FontFamily=\"{family}\" GdiCharSet=\"1\" GdiVerticalFont=\"False\" Height=\"{height}\" IsSystemFont=\"False\" Italic=\"{italic}\" Name=\"{family}\" OriginalFontName=\"{orig}\" Size=\"{size}\" SizeinPoints=\"{size}\" Strikeout=\"{strike}\" Style=\"{style}\" SystemFontName=\"\" Underline=\"{under}\" Unit=\"Point\" />",
        bold = b(f.bold),
        family = RESOLVED_FONT.name,
        height = font_height(size),
        italic = b(f.italic),
        orig = escape(&f.name),
        strike = b(f.strikethrough),
        style = font_style(f),
        under = b(f.underline),
    );
}

/// The `Style` attribute — the .NET `FontStyle` flags enum's `ToString` (comma-joined Bold,
/// Italic, Underline, Strikeout in that order; `Regular` when none are set).
fn font_style(f: &rpt::model::Font) -> String {
    let mut parts = Vec::new();
    if f.bold {
        parts.push("Bold");
    }
    if f.italic {
        parts.push("Italic");
    }
    if f.underline {
        parts.push("Underline");
    }
    if f.strikethrough {
        parts.push("Strikeout");
    }
    if parts.is_empty() {
        "Regular".to_string()
    } else {
        parts.join(", ")
    }
}

/// `System.Drawing.Font.Height` — the line spacing in pixels. Not stored in the report; GDI
/// derives it from the *resolved* typeface as `ceil(pointSize · lineSpacing/unitsPerEm · dpi/72)`.
/// Every font resolves to [`RESOLVED_FONT`] (the same reason `Name`/`FontFamily` are that face),
/// so this is the GDI formula instantiated with that face's hhea metrics at the standard 96 DPI.
fn font_height(size_pt: i32) -> i32 {
    let numerator = size_pt * RESOLVED_FONT.line_spacing * SCREEN_DPI;
    let denominator = RESOLVED_FONT.units_per_em * 72;
    (numerator + denominator - 1) / denominator // ceil
}

/// Display DPI the renderer uses (System.Drawing's default).
const SCREEN_DPI: i32 = 96;

/// The typeface substituted for every requested font, with the hhea metrics GDI uses to compute
/// line spacing. One source of truth for the `Name` / `FontFamily` attributes and [`font_height`].
struct ResolvedFont {
    name: &'static str,
    line_spacing: i32, // hhea ascent + descent (Tahoma: 2049 + 423)
    units_per_em: i32, // Tahoma: 2048
}
const RESOLVED_FONT: ResolvedFont = ResolvedFont {
    name: "Tahoma",
    line_spacing: 2472,
    units_per_em: 2048,
};

/// `--full`: emit a record node — the XML element is the node's DTO type.
pub(crate) fn write_node(o: &mut String, node: &Node, depth: usize) {
    match node {
        Node::FieldDef(f) => {
            let pad = "  ".repeat(depth);
            let _ = writeln!(o, "{pad}<FieldDef Name=\"{}\" />", escape(&f.name));
        }
        Node::Unknown(u) => write_unknown(o, u, depth),
    }
}

pub(crate) fn write_unknown(o: &mut String, u: &Unknown, depth: usize) {
    let pad = "  ".repeat(depth);
    let name = u.type_name();
    if u.children.is_empty() && u.values.is_empty() {
        let _ = writeln!(o, "{pad}<{name} subtype=\"{}\" />", u.subtype);
        return;
    }
    let _ = writeln!(o, "{pad}<{name} subtype=\"{}\">", u.subtype);
    for v in &u.values {
        match v {
            Value::Text(t) => {
                let _ = writeln!(o, "{pad}  <Text>{}</Text>", escape_text(t));
            }
            Value::Int(n) => {
                let _ = writeln!(o, "{pad}  <Int>{n}</Int>");
            }
            Value::Bytes(_) => {}
        }
    }
    for child in &u.children {
        write_node(o, child, depth + 1);
    }
    let _ = writeln!(o, "{pad}</{name}>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_node_emits_generic_element_with_children() {
        // Every record type that occurs in the corpus has a registry name, so to exercise the
        // generic `<Type_0xNNNN>` fallback we use reserved out-of-range types that `RecordTag::name()`
        // does not map (named types render under their registry name instead). `0xfffe`/`0xfffd` are
        // not real record types — they only drive the fallback branch here.
        debug_assert!(
            rpt::raw::RecordTag(0xFFFE).name().is_none()
                && rpt::raw::RecordTag(0xFFFD).name().is_none()
        );
        let u = Unknown {
            rtype: 0xFFFE,
            subtype: 7,
            values: vec![Value::Text("hi".into())],
            children: vec![Node::Unknown(Unknown {
                rtype: 0xFFFD,
                subtype: 7,
                values: vec![],
                children: vec![],
            })],
        };
        let mut out = String::new();
        write_unknown(&mut out, &u, 0);
        assert!(out.contains("<Type_0xfffe subtype=\"7\">"));
        assert!(out.contains("<Text>hi</Text>"));
        assert!(out.contains("<Type_0xfffd subtype=\"7\" />"));
    }
}
