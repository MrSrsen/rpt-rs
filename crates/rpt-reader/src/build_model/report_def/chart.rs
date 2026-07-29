//! Chart decode — each chart object's [`ChartDefinition`] (type/subtype, titles, legend, and the
//! styling block: shape and size enums, the four gridline modes, and the per-axis range, scaling
//! and division settings) parsed from the flat binding region, and the assembly of a decoded chart
//! onto its object.

use super::bindings::{binding_scopes, GridBindings};
use super::summary::collect_summary_defs;
use crate::build_model::record_values::is_field_ref;
use crate::build_model::row_of;
use crate::codec::RecordNode;
use crate::field_table::content_of;
use crate::field_table::cursor::{RecordContent, StringFormat};
use crate::field_table::table::{read_strings, Cell, Row};
use crate::field_table::tables::{
    CHART_ANALYTIC as CHART_ANALYTIC_TABLE, CHART_DATA_VALUE as CHART_DATA_VALUE_TABLE,
    CHART_DEFINITION2 as CHART_DEFINITION2_TABLE,
};
use crate::model::{ChartDefinition, ChartGraphType, ChartGridType, ChartLegendPosition, Group};
use crate::records::rtype::*;

/// The report-wide inputs every chart's assembly draws on: the decoded chart definitions, the
/// report's own groups and summaries (which are what a report-group chart charts), and the field
/// types that say whether a category is date-valued.
pub(super) struct ChartAttach<'a> {
    styles: std::collections::HashMap<String, ChartDefinition>,
    groups: &'a [Group],
    /// The report's deduped summary defs `(operation, field)` — the data source of a report-group
    /// ("Top-N") chart, whose own `0xb4` block carries no bindings and which instead charts the
    /// report's summary field(s) over the report's group(s). Non-running-total summaries only (the
    /// running-total defs sit inside a `0x80` and are excluded by `collect_summary_defs`);
    /// field-shaped operands only; duplicates (the engine writes one summary def per placed summary
    /// object, so the same `Op (field)` recurs) collapsed to one, preserving order.
    report_summaries: Vec<(crate::model::SummaryOperation, String)>,
    /// The condition fields the report groups on with a date condition.
    date_group_fields: std::collections::HashSet<&'a str>,
    field_types: &'a std::collections::HashMap<String, crate::model::FieldValueType>,
}

impl<'a> ChartAttach<'a> {
    pub(super) fn new(
        tree: &[RecordNode],
        logical: &[u8],
        groups: &'a [Group],
        field_types: &'a std::collections::HashMap<String, crate::model::FieldValueType>,
    ) -> Self {
        let mut report_summaries: Vec<(crate::model::SummaryOperation, String)> = Vec::new();
        for def in collect_summary_defs(tree, logical) {
            let entry = (def.operation, def.operand);
            if is_field_ref(&entry.1) && !report_summaries.contains(&entry) {
                report_summaries.push(entry);
            }
        }
        ChartAttach {
            styles: collect_chart_styles(tree, logical),
            report_summaries,
            date_group_fields: groups
                .iter()
                .filter(|g| g.date_condition.is_some())
                .map(|g| g.condition_field.as_str())
                .collect(),
            groups,
            field_types,
        }
    }

    /// Whether a category is a date field, and so carries the summary's `"daily"` third operand.
    ///
    /// A chart data summary over a date "on change of" category carries that operand for every
    /// date-valued category regardless of its visual grouping period. A category is a date field
    /// when either the report groups on it with a date condition, or — for a pure single-chart date
    /// axis with no report group — its datasource field type is Date/DateTime (looked up by the
    /// lowercase `alias.field` reference, threaded from the QESession schema).
    fn is_date_scope(&self, category: &str) -> bool {
        self.date_group_fields.contains(category)
            || matches!(
                self.field_types.get(&category.to_lowercase()),
                Some(crate::model::FieldValueType::Date | crate::model::FieldValueType::DateTime)
            )
    }

    /// Assemble one chart object from its decoded definition and field bindings.
    pub(super) fn attach(
        &mut self,
        name: &str,
        chart: &mut crate::model::ChartObject,
        bindings: Option<&GridBindings>,
    ) {
        if let Some(def) = self.styles.remove(name) {
            chart.definition = def;
        }
        // A chart binds its fields one of two ways. A *grid-group* chart carries its own bindings
        // in its `0xb4` block (data `0x7f`/`0x7e`, category grid `0xe5`). A *report-group* ("Top-N")
        // chart has an empty `0xb4` block and instead charts the report's own group(s) over the
        // report's summary field(s). A missing or empty grid binding is the report-group case.
        let grid = bindings.filter(|r| !r.data.is_empty() || !r.category.is_empty());
        if let Some(refs) = grid {
            chart.data_refs = refs.data.clone();
            chart.category_refs = refs.category.clone();
            // Set the category period after the definition is assigned (which replaces it), so the
            // grid-group-decoded period survives.
            chart.definition.category_period = refs.category_period;
            // Compose the RAS `DataFields` / `ConditionFields` summary-form strings. The scoping
            // (innermost) category is the last one; its explicit period (if any) becomes the
            // summary's third operand.
            let data: Vec<_> = refs
                .data_ops
                .iter()
                .copied()
                .zip(refs.data.iter().cloned())
                .collect();
            let (data_forms, cond_forms) = self.compose(&data, &refs.category);
            chart.definition.data_refs = data_forms;
            chart.definition.category_refs = cond_forms;
        } else if !self.groups.is_empty() && !self.report_summaries.is_empty() {
            // Report-group ("Top-N") chart: category = each report group's condition field
            // (outer→inner); data = each report summary summarized over the innermost group,
            // `Op ({field}, {innermost group}[, "period"])`. Only the DataDefinition
            // (`ChartDefinition`) refs are set here — the bare `ChartObject.data_refs` /
            // `category_refs` are left untouched, as a report-group chart references the report's
            // own group/summary fields, not new ones.
            let categories: Vec<String> = self
                .groups
                .iter()
                .map(|g| g.condition_field.clone())
                .collect();
            let (data_forms, cond_forms) = self.compose(&self.report_summaries, &categories);
            chart.definition.data_refs = data_forms;
            chart.definition.category_refs = cond_forms;
        }
    }

    /// Compose the summary forms for one chart: the summary's grouping period is `"daily"` when the
    /// scoping (innermost) category is a date field, else absent.
    fn compose(
        &self,
        data: &[(crate::model::SummaryOperation, String)],
        categories: &[String],
    ) -> (Vec<String>, Vec<String>) {
        let scoping_is_date = categories.last().is_some_and(|c| self.is_date_scope(c));
        compose_chart_refs(data, categories, scoping_is_date.then_some("daily"))
    }
}

/// Collect each chart's decoded [`ChartDefinition`] (type + titles + data label) from the flat
/// binding region, keyed by chart object name.
///
/// A chart is written as a contiguous group in section order — `0xb4 ChartObject` → its `0x9e`
/// name → `0x011c` analytic header → the analytic data section → `0x0121 ChartDefinition2` — so the
/// chart named by the most recent `0xb4` owns the `0x011f`/`0x0121` records until the next `0xb4`
/// (or an area/section marker) begins another. The `0x0121` record's own layout is stated by
/// [`CHART_DEFINITION2_TABLE`]. The `0x011f` record carries the data-value label after a 6-byte
/// header. See [`ChartDefinition`] for the status of this decode.
pub(super) fn collect_chart_styles(
    tree: &[RecordNode],
    logical: &[u8],
) -> std::collections::HashMap<String, ChartDefinition> {
    let mut out: std::collections::HashMap<String, ChartDefinition> =
        std::collections::HashMap::new();
    for (current, node) in binding_scopes(tree, logical, &[CHART_OBJECT]) {
        match node.rtype {
            CHART_ANALYTIC => {
                // The `0x011c` analytic header precedes the `0x0121` definition record, whose arm
                // below replaces the slot wholesale — so, like `data_label`, the layout type is
                // stashed on the slot here and restored across that replacement.
                if let Some(name) = &current {
                    let row = row_of(node, logical, &CHART_ANALYTIC_TABLE);
                    if let Some(code) = row.get("layout_type").and_then(Cell::u) {
                        out.entry(name.clone()).or_default().layout_type =
                            crate::model::ChartLayoutType::from_code(code as u8);
                    }
                }
            }
            CHART_DATA_VALUE => {
                if let Some(name) = &current {
                    let row = row_of(node, logical, &CHART_DATA_VALUE_TABLE);
                    // A chart may riser several values; the label is the first one's.
                    if let Some(first) = row.seq("values").first() {
                        out.entry(name.clone()).or_default().data_label =
                            first.text("summary").to_owned();
                    }
                }
            }
            CHART_DEFINITION2 => {
                if let Some(name) = &current {
                    let mut def = parse_chart_definition2(&content_of(node, logical));
                    // The `0x011f` record (which precedes this one) is the only source of the data
                    // label, and the `0x011c` header the only source of the layout type; `0x0121`
                    // carries neither, so preserve whatever was captured on the slot.
                    let slot = out.entry(name.clone()).or_default();
                    def.data_label = std::mem::take(&mut slot.data_label);
                    def.layout_type = slot.layout_type;
                    *slot = def;
                }
            }
            _ => {}
        }
    }
    out
}

/// The chart text elements, by the index each occupies in every per-element run the `0x0121` record
/// carries — the default flags, the style entries, and this decoder's `element_fonts`.
mod element {
    pub(super) const TITLE: usize = 0;
    pub(super) const SUBTITLE: usize = 1;
    pub(super) const FOOTNOTE: usize = 2;
    pub(super) const GROUP_TITLE: usize = 3;
    pub(super) const DATA_TITLE: usize = 4;
    pub(super) const SERIES_TITLE: usize = 5;
    pub(super) const LEGEND: usize = 6;
    pub(super) const GROUP_LABEL: usize = 7;
    pub(super) const DATA_LABEL: usize = 8;
    pub(super) const SERIES_LABEL: usize = 9;
}

/// The number of chart text elements the engine enumerates — the length of every run indexed by
/// [`element`].
const CHART_TEXT_ELEMENTS: usize = 10;

/// Which text element each of the eight face names in the `0x0121` string run belongs to.
///
/// The face names are written in a different order from the one the flag and style arrays use: the
/// run skips the two series elements, whose faces are the strings past the styling block
/// ([`FACE_TRAILING_ELEMENTS`]).
const FACE_RUN_ELEMENTS: [usize; 8] = [
    element::TITLE,
    element::SUBTITLE,
    element::FOOTNOTE,
    element::GROUP_TITLE,
    element::DATA_TITLE,
    element::LEGEND,
    element::GROUP_LABEL,
    element::DATA_LABEL,
];

/// Which text element each of the two face names past the styling block belongs to — the two the
/// string run skips.
const FACE_TRAILING_ELEMENTS: [usize; 2] = [element::SERIES_TITLE, element::SERIES_LABEL];

/// Which text element each of the nine per-element style entries belongs to. DataLabel has no
/// entry, so its weight and slant are not `Contents` facts and a consumer takes them from its
/// default table.
const STYLE_ELEMENTS: [usize; 9] = [
    element::TITLE,
    element::SUBTITLE,
    element::FOOTNOTE,
    element::GROUP_TITLE,
    element::DATA_TITLE,
    element::SERIES_TITLE,
    element::LEGEND,
    element::GROUP_LABEL,
    element::SERIES_LABEL,
];

/// The per-element flag word meaning "this text element is entirely at the engine's default font",
/// at the width the record stores it. Any other value marks an authored element (whose point size
/// lives outside `Contents`).
const ELEMENT_FONT_DEFAULT: u16 = 0x0110;

/// Parse a `0x0121 ChartDefinition2` record into the chart fields its bytes carry (type/subtype,
/// title/axis strings, legend placement, the styling block, per-element fonts), by walking
/// [`CHART_DEFINITION2_TABLE`].
///
/// Every field's position is a consequence of what precedes it — the two narrowing enums, the
/// sixteen length-prefixed strings, and the two family-conditional entries — so nothing here
/// computes an offset.
///
/// A record that ends early simply leaves the fields it did not reach unset, which is how a
/// truncated record reports every element as engine-default rather than half a table.
///
/// The parse is handed content rather than a node, so there is no header to take the string framing
/// from and it names the enhanced form — the one the record-tree reader admits a header for.
fn parse_chart_definition2(content: &RecordContent) -> ChartDefinition {
    let mut def = ChartDefinition::default();
    let row = read_strings(&CHART_DEFINITION2_TABLE, content, StringFormat::Enhanced).row;
    let Some(graph_type) = row.get("graph_type").and_then(Cell::u) else {
        return def;
    };
    def.graph_type = ChartGraphType::from_code(graph_type as i32);
    def.graph_subtype = row.u("graph_subtype") as i32;
    def.title = row.text("title").to_owned();
    def.subtitle = row.text("subtitle").to_owned();
    def.footnote = row.text("footnote").to_owned();
    def.group_axis_title = row.text("group_axis_title").to_owned();
    def.data_axis_title = row.text("data_axis_title").to_owned();
    // The engine defaults, reported unless the record reached the byte and said otherwise.
    def.legend_visible = true;
    def.is_vertical_bar = true;
    def.value_axis_auto_range = true;
    def.value_axis2_auto_range = true;
    def.series_axis_auto_range = true;

    // The eight faces of the string run, then everything past the styling block. Both are read
    // all-or-nothing: a run that stopped part-way names no element.
    let faces = row.seq("faces");
    if faces.len() != FACE_RUN_ELEMENTS.len() || faces.iter().any(|f| f.get("face").is_none()) {
        return def;
    }
    def.element_fonts = vec![crate::model::ChartElementFont::default(); CHART_TEXT_ELEMENTS];
    for (f, &element) in faces.iter().zip(&FACE_RUN_ELEMENTS) {
        def.element_fonts[element].name = f.text("face").to_owned();
        def.element_fonts[element].is_default = true;
    }
    read_trailing_element_fonts(&row, &mut def.element_fonts);

    if let (Some(flags), Some(pos)) = (
        row.get("legend_flags").and_then(Cell::num),
        row.get("legend_position").and_then(Cell::u),
    ) {
        def.legend_visible = flags & 0x01 != 0;
        def.legend_position = ChartLegendPosition::from_code(pos as u8);
    }
    read_styling(&row, &mut def);
    // The viewing angle is a field only the 3-D families carry; the table omits it for the rest, so
    // there is nothing to gate here.
    if let Some(va) = row.get("view_angle").and_then(Cell::u) {
        def.view_angle = crate::model::ChartViewAngle::from_stored(va as u8);
    }
    def
}

/// Apply the `0x0121` styling block: the shape/size enums, the four gridline modes, and the three
/// per-axis runs (range, number format, auto-range, auto-scale, division method and count) in the
/// axis order value, value-2, series.
///
/// Each field is applied only where the record reached it, so one that stops part-way keeps the
/// engine defaults set by the caller rather than reporting a zero the record never stored.
fn read_styling(row: &Row, def: &mut ChartDefinition) {
    use crate::model::{
        ChartBarSize, ChartColorMode, ChartDataPoint, ChartDivisionMethod, ChartLegendLayout,
        ChartMarkerShape, ChartMarkerSize, ChartNumberFormat, ChartPieSize, ChartSliceDetachment,
    };
    let byte = |name: &str| row.get(name).and_then(Cell::u).map(|v| v as u8);
    let flag = |name: &str| row.get(name).and_then(Cell::u).map(|v| v != 0);
    let count = |name: &str| row.get(name).and_then(Cell::u);
    let real = |name: &str| row.get(name).and_then(Cell::f);

    if let Some(v) = row
        .get("is_vertical_bar")
        .and_then(Cell::num)
        .map(|v| v != 0)
    {
        def.is_vertical_bar = v;
    }
    if let Some(v) = byte("bar_size") {
        def.bar_size = ChartBarSize::from_code(v);
    }
    if let Some(v) = row.get("pie_size").and_then(Cell::u) {
        def.pie_size = ChartPieSize::from_code(v as u8);
    }
    // Only the pie families carry it, so a chart of any other family keeps the default.
    if let Some(v) = byte("slice_detachment") {
        def.slice_detachment = ChartSliceDetachment::from_code(v);
    }
    if let Some(v) = byte("marker_size") {
        def.marker_size = ChartMarkerSize::from_code(v);
    }
    if let Some(v) = byte("marker_shape") {
        def.marker_shape = ChartMarkerShape::from_code(v);
    }
    if let Some(v) = byte("chart_color") {
        def.chart_color = ChartColorMode::from_code(v);
    }
    if let Some(v) = byte("legend_layout") {
        def.legend_layout = ChartLegendLayout::from_code(v);
    }
    if let Some(v) = byte("data_value_number_format") {
        def.data_value_number_format = ChartNumberFormat::from_code(v);
    }
    // The same byte twice: the whole enum, and the "writes its value" reading the render side takes.
    if let Some(v) = byte("data_labels") {
        def.data_point = ChartDataPoint::from_code(v);
        def.data_labels_show_value = v & 0x02 != 0;
    }
    // Every family stores the four gridline bytes at the same sequence position, the axis-less ones
    // (Pie, Doughnut, Gauge, Gantt, Funnel, Histogram) included. Whether a family draws an axis to
    // apply them to is the renderer's business, not the decoder's.
    if let Some(v) = byte("group_axis_gridlines") {
        def.group_axis_gridlines = ChartGridType::from_code(v);
    }
    if let Some(v) = byte("series_axis_gridlines") {
        def.series_axis_gridlines = ChartGridType::from_code(v);
    }
    if let Some(v) = byte("value_axis_gridlines") {
        def.value_axis_gridlines = ChartGridType::from_code(v);
    }
    if let Some(v) = byte("value_axis2_gridlines") {
        def.value_axis2_gridlines = ChartGridType::from_code(v);
    }
    if let Some(v) = real("value_axis_min") {
        def.value_axis_min = v;
    }
    if let Some(v) = real("value_axis_max") {
        def.value_axis_max = v;
    }
    if let Some(v) = real("value_axis2_min") {
        def.value_axis2_min = v;
    }
    if let Some(v) = real("value_axis2_max") {
        def.value_axis2_max = v;
    }
    if let Some(v) = real("series_axis_min") {
        def.series_axis_min = v;
    }
    if let Some(v) = real("series_axis_max") {
        def.series_axis_max = v;
    }
    if let Some(v) = byte("value_axis_number_format") {
        def.value_axis_number_format = ChartNumberFormat::from_code(v);
    }
    if let Some(v) = byte("value_axis2_number_format") {
        def.value_axis2_number_format = ChartNumberFormat::from_code(v);
    }
    if let Some(v) = byte("series_axis_number_format") {
        def.series_axis_number_format = ChartNumberFormat::from_code(v);
    }
    if let Some(v) = flag("value_axis_auto_range") {
        def.value_axis_auto_range = v;
    }
    if let Some(v) = flag("value_axis2_auto_range") {
        def.value_axis2_auto_range = v;
    }
    if let Some(v) = flag("series_axis_auto_range") {
        def.series_axis_auto_range = v;
    }
    if let Some(v) = flag("value_axis_auto_scale") {
        def.value_axis_auto_scale = v;
    }
    if let Some(v) = flag("value_axis2_auto_scale") {
        def.value_axis2_auto_scale = v;
    }
    if let Some(v) = flag("series_axis_auto_scale") {
        def.series_axis_auto_scale = v;
    }
    if let Some(v) = byte("value_axis_division_method") {
        def.value_axis_division_method = ChartDivisionMethod::from_code(v);
    }
    if let Some(v) = byte("value_axis2_division_method") {
        def.value_axis2_division_method = ChartDivisionMethod::from_code(v);
    }
    if let Some(v) = byte("series_axis_division_method") {
        def.series_axis_division_method = ChartDivisionMethod::from_code(v);
    }
    if let Some(v) = count("value_axis_divisions") {
        def.value_axis_divisions = v;
    }
    if let Some(v) = count("value_axis2_divisions") {
        def.value_axis2_divisions = v;
    }
    if let Some(v) = count("series_axis_divisions") {
        def.series_axis_divisions = v;
    }
}

/// Apply the part of the per-element font table that lives past the styling block: the two
/// remaining face names ([`FACE_TRAILING_ELEMENTS`]), the per-element default-font flags
/// ([`ELEMENT_FONT_DEFAULT`] marking an element left at the engine's font), and the per-element
/// weight and slant ([`STYLE_ELEMENTS`]).
///
/// Each group is all-or-nothing: a record that stops inside one leaves every element in that group
/// untouched, so a consumer reads "not stored" for all of them rather than for an arbitrary suffix.
fn read_trailing_element_fonts(row: &Row, fonts: &mut [crate::model::ChartElementFont]) {
    let trailing = row.seq("trailing_faces");
    let flags = row.seq("element_flags");
    if trailing.len() != FACE_TRAILING_ELEMENTS.len() || flags.len() != CHART_TEXT_ELEMENTS {
        return;
    }
    for (f, &element) in trailing.iter().zip(&FACE_TRAILING_ELEMENTS) {
        fonts[element].name = f.text("face").to_owned();
    }
    for (font, f) in fonts.iter_mut().zip(flags) {
        font.is_default = f.u("flag") == u32::from(ELEMENT_FONT_DEFAULT);
    }
    let styles = row.seq("element_styles");
    if styles.len() != STYLE_ELEMENTS.len() {
        return;
    }
    for (s, &element) in styles.iter().zip(&STYLE_ELEMENTS) {
        fonts[element].weight = s.u("weight") as u16;
        fonts[element].italic = s.u("italic") != 0;
    }
}

/// Compose a chart's RAS `DataFields` / `ConditionFields` `FormulaForm` lists (see
/// [`ChartDefinition::data_refs`]/[`ChartDefinition::category_refs`]) from the decoded data summaries
/// (`(operation, field)` pairs), the ordered category ("on change of") field references, and the
/// scoping (innermost) category's grouping-period token.
///
/// - `ConditionFields` = each category brace-wrapped (`{field}`), in axis order.
/// - `DataFields` = for each data summary, `Op ({field}, {scoping_category}[, "period"])`, or
///   `Op ({field})` when the chart has no category. The scoping category is the innermost (last)
///   axis; `scoping_period` is its explicit non-daily grouping period token, or `None` (a discrete
///   category, or a date category on its implicit daily default — see [`ChartDefinition::data_refs`]).
pub(super) fn compose_chart_refs(
    data: &[(crate::model::SummaryOperation, String)],
    categories: &[String],
    scoping_period: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    let condition = categories.iter().map(|c| format!("{{{c}}}")).collect();
    let scoping = categories.last();
    let data_forms = data
        .iter()
        .map(|(op, field)| {
            let name = op.full_name();
            match scoping {
                Some(cat) => match scoping_period {
                    Some(p) => format!("{name} ({{{field}}}, {{{cat}}}, \"{p}\")"),
                    None => format!("{name} ({{{field}}}, {{{cat}}})"),
                },
                None => format!("{name} ({{{field}}})"),
            }
        })
        .collect();
    (data_forms, condition)
}

#[cfg(test)]
mod chart_def2_tests {
    use super::{
        element, CHART_TEXT_ELEMENTS, ELEMENT_FONT_DEFAULT, FACE_TRAILING_ELEMENTS, STYLE_ELEMENTS,
    };
    use crate::field_table::cursor::{Piece, RecordContent};
    use crate::model::{ChartDefinition, ChartGraphType, ChartLegendPosition};

    /// Decode a synthetic run as a `0x0121` record with no nested children.
    fn parse_chart_definition2(run: &[u8]) -> ChartDefinition {
        super::parse_chart_definition2(&RecordContent {
            rtype: 0x0121,
            schema: 0x0700,
            pieces: vec![Piece::Run(run.to_vec())],
        })
    }

    /// The sixteen length-prefixed strings a `0x0121` record carries: title, subtitle, footnote, two
    /// format masks, the two axis titles, one empty separator, then eight per-element face names.
    const CHART_STRING_COUNT: usize = 16;

    /// The graph-type codes whose family carries extra bytes mid-styling-struct.
    const GRAPH_TYPE_PIE: u8 = 3;
    const GRAPH_TYPE_DOUGHNUT: u8 = 4;
    const GRAPH_TYPE_3D_RISER: u8 = 5;
    const GRAPH_TYPE_3D_SURFACE: u8 = 6;

    /// The styling struct's length for a chart family that carries no extra mid-struct field.
    const STYLING_STRUCT_LEN: usize = 110;
    /// The two enum bytes the pie families carry mid-struct on top of that.
    const PIE_FAMILY_EXTRA: usize = 2;
    /// The one the 3-D families carry.
    const THREE_D_FAMILY_EXTRA: usize = 1;

    /// The bytes between the end of the string run's separator and the two trailing face names, for
    /// a chart family. Stated here as a total so the synthetic runs place the trailing block where
    /// the record does; the field table reaches it by walking the fields instead.
    fn styling_struct_len(graph_type: u8) -> usize {
        STYLING_STRUCT_LEN
            + match graph_type {
                GRAPH_TYPE_PIE | GRAPH_TYPE_DOUGHNUT => PIE_FAMILY_EXTRA,
                GRAPH_TYPE_3D_RISER | GRAPH_TYPE_3D_SURFACE => THREE_D_FAMILY_EXTRA,
                _ => 0,
            }
    }

    /// One text element's weight and slant byte offsets, relative to the start of the per-element
    /// flag array.
    struct ElementStyleOffsets {
        element: usize,
        weight: usize,
        italic: usize,
    }

    /// Where each text element's weight and slant sit relative to the start of the per-element flag
    /// array. Entries run five bytes apart, except GroupTitle's, which is six; DataLabel has none
    /// and so is absent here. The synthetic runs below are built at these positions, so the field
    /// table has to land on them by walking its entries rather than by sharing the arithmetic.
    ///
    /// GroupTitle's extra byte sits between its weight and its slant, hence the gap of three where
    /// every other entry has two. The `ELEMENT_STYLE` body of the field tables is where that
    /// irregularity is stated.
    const ELEMENT_STYLE_OFFSETS: [ElementStyleOffsets; STYLE_ELEMENTS.len()] = [
        ElementStyleOffsets {
            element: element::TITLE,
            weight: 26,
            italic: 28,
        },
        ElementStyleOffsets {
            element: element::SUBTITLE,
            weight: 31,
            italic: 33,
        },
        ElementStyleOffsets {
            element: element::FOOTNOTE,
            weight: 36,
            italic: 38,
        },
        ElementStyleOffsets {
            element: element::GROUP_TITLE,
            weight: 41,
            italic: 44,
        },
        ElementStyleOffsets {
            element: element::DATA_TITLE,
            weight: 47,
            italic: 49,
        },
        ElementStyleOffsets {
            element: element::SERIES_TITLE,
            weight: 52,
            italic: 54,
        },
        ElementStyleOffsets {
            element: element::LEGEND,
            weight: 57,
            italic: 59,
        },
        ElementStyleOffsets {
            element: element::GROUP_LABEL,
            weight: 62,
            italic: 64,
        },
        ElementStyleOffsets {
            element: element::SERIES_LABEL,
            weight: 67,
            italic: 69,
        },
    ];

    /// One past the last byte the per-element style block occupies, from the flag array's start.
    const ELEMENT_STYLE_END: usize = 72;

    /// Build a minimal synthetic `0x0121` run: two enum bytes (`graph_type`, subtype `0`), then
    /// [`CHART_STRING_COUNT`] empty length-prefixed strings (len `1` = a lone NUL), a 1-byte
    /// separator, and the styling struct opened by the legend `short` (`legend_flags`,
    /// `legend_pos`). Padded so the data-labels byte (81 bytes past the legend short for an axis
    /// chart) is present and set from `data_label`.
    fn synth_run(graph_type: u8, legend_flags: u8, legend_pos: u8, data_label: u8) -> Vec<u8> {
        let mut v = vec![graph_type, 0];
        for _ in 0..CHART_STRING_COUNT {
            v.extend_from_slice(&[0, 0, 0, 1, 0]); // len=1, single NUL → empty string
        }
        v.push(0); // separator; legend short opens at off+1
        let flags_off = v.len();
        v.push(legend_flags);
        v.push(legend_pos);
        // Data-labels enum byte is a fixed 81-byte walk past the legend short (pie/doughnut add 2,
        // exercised separately). Pad the intervening styling bytes with zeros, then write it.
        let data_label_off = flags_off + 81;
        v.resize(data_label_off, 0);
        v.push(data_label);
        v
    }

    /// `graph_subtype` is byte 1 verbatim, and for the Bar / Line / Pie / Doughnut bands that
    /// byte is the SDK subtype ordinal itself — no compaction, unlike the Area band. The stored
    /// `(graph_type, graph_subtype)` pairs below are the ones an authored per-family subtype sweep
    /// produces, so a family's depth sibling (Bar `3`/`4`/`5`, Pie `31`) reaches
    /// `has_depth_effect` as a distinct value rather than colliding with its flat variant.
    #[test]
    fn graph_subtype_is_the_stored_byte_across_family_bands() {
        let parse = |graph_type: u8, subtype: u8| {
            let mut run = synth_run(graph_type, 0x01, 0, 0);
            run[1] = subtype;
            parse_chart_definition2(&run)
        };
        for (gt, sub, depth) in [
            (0u8, 0u8, false), // Bar SideBySide
            (0, 1, false),     // Bar Stacked
            (0, 2, false),     // Bar Percent
            (0, 3, true),      // Bar Faked3DSideBySide
            (0, 4, true),      // Bar Faked3DStacked
            (0, 5, true),      // Bar Faked3DPercent
            (1, 10, false),    // Line Regular … the Line band has no depth variant at all
            (1, 13, false),    // Line WithMarkers
            (3, 30, false),    // Pie Regular
            (3, 31, true),     // Pie Faked3DRegular
            (3, 33, false),    // Pie MultipleProportional
            (4, 40, false),    // Doughnut Regular
        ] {
            let def = parse(gt, sub);
            assert_eq!(
                def.graph_subtype,
                i32::from(sub),
                "stored subtype {gt}/{sub}"
            );
            assert_eq!(def.has_depth_effect(), depth, "depth {gt}/{sub}");
        }
        // The subtype is byte 1, not byte 2: byte 2 opens the string run, and a subtype written there
        // is read as a string length instead — the whole run then decodes as subtype 0.
        let mut misplaced = synth_run(0, 0x01, 0, 0);
        misplaced[2] = 4;
        let def = parse_chart_definition2(&misplaced);
        assert_eq!(def.graph_subtype, 0, "byte 2 is not the subtype slot");
        assert!(!def.has_depth_effect());
    }

    /// The decoder maps all four legend-position codes 0..=3 to Right/Left/BottomCenter/Custom, each
    /// (code 3 = a manually positioned legend, reported by the engine as `crLegendPositionCustom`).
    #[test]
    fn legend_position_decodes_all_four_codes() {
        for (code, want) in [
            (0u8, ChartLegendPosition::Right),
            (1, ChartLegendPosition::Left),
            (2, ChartLegendPosition::BottomCenter),
            (3, ChartLegendPosition::Custom),
        ] {
            let def = parse_chart_definition2(&synth_run(0, 0x01, code, 0));
            assert_eq!(def.legend_position, want, "legend position code {code}");
            assert!(def.legend_visible, "bit0 set → visible (code {code})");
        }
    }

    /// The styling block's per-axis runs are read in the axis order value, value-2, series: three
    /// big-endian `(min, max)` double pairs, then the number-format / auto-range / division-method
    /// triples, then the three division counts. Each slot is given a distinct value, so a swapped
    /// axis or a slipped offset shows up as the wrong axis carrying it.
    #[test]
    fn per_axis_runs_decode_in_value_value2_series_order() {
        use crate::model::{ChartDivisionMethod, ChartNumberFormat};
        let flags_off = 2 + CHART_STRING_COUNT * 5 + 1;
        // One byte past what `synth_run` reaches, so the data-value number format is present.
        let mut run = synth_run(0, 0x01, 0, 0);
        run.resize(flags_off + 83, 0);
        // Six doubles at +11, big-endian, one distinct value each.
        for (i, v) in [1.5f64, -2.25, 3.5, -4.75, 5.125, -6.0]
            .into_iter()
            .enumerate()
        {
            let at = flags_off + 11 + i * 8;
            run[at..at + 8].copy_from_slice(&v.to_be_bytes());
        }
        // The three enum triples, then the three division counts.
        for (i, v) in [1u8, 2, 3, 1, 0, 1, 1, 0, 1].into_iter().enumerate() {
            run[flags_off + 59 + i] = v;
        }
        for (i, v) in [7u32, 8, 9].into_iter().enumerate() {
            let at = flags_off + 68 + i * 4;
            run[at..at + 4].copy_from_slice(&v.to_be_bytes());
        }
        let def = parse_chart_definition2(&run);
        assert_eq!(
            (
                def.value_axis_min,
                def.value_axis_max,
                def.value_axis2_min,
                def.value_axis2_max,
                def.series_axis_min,
                def.series_axis_max
            ),
            (1.5, -2.25, 3.5, -4.75, 5.125, -6.0)
        );
        assert_eq!(def.value_axis_number_format, ChartNumberFormat::OneDecimal);
        assert_eq!(def.value_axis2_number_format, ChartNumberFormat::TwoDecimal);
        assert_eq!(
            def.series_axis_number_format,
            ChartNumberFormat::CurrencyNoDecimal
        );
        assert_eq!(
            (
                def.value_axis_auto_range,
                def.value_axis2_auto_range,
                def.series_axis_auto_range
            ),
            (true, false, true)
        );
        assert_eq!(def.value_axis_division_method, ChartDivisionMethod::Manual);
        assert_eq!(
            def.value_axis2_division_method,
            ChartDivisionMethod::Automatic
        );
        assert_eq!(def.series_axis_division_method, ChartDivisionMethod::Manual);
        assert_eq!(
            (
                def.value_axis_divisions,
                def.value_axis2_divisions,
                def.series_axis_divisions
            ),
            (7, 8, 9)
        );
        // The byte past the division counts is the colour mode, and the one past the data-point
        // enum the data-value number format — the two single slots that bracket it.
        run[flags_off + 80] = 1;
        run[flags_off + 82] = 5;
        let def = parse_chart_definition2(&run);
        assert_eq!(def.chart_color, crate::model::ChartColorMode::BlackAndWhite);
        assert_eq!(
            def.data_value_number_format,
            ChartNumberFormat::PercentNoDecimal
        );
    }

    /// The shape and size enums that open the styling block, in wire order: the bar orientation
    /// flag, the riser size, then the marker size and shape.
    #[test]
    fn shape_and_size_enums_open_the_styling_block() {
        use crate::model::{ChartBarSize, ChartMarkerShape, ChartMarkerSize};
        let flags_off = 2 + CHART_STRING_COUNT * 5 + 1;
        let mut run = synth_run(0, 0x01, 0, 0);
        run[flags_off + 3] = 0; // is_vertical_bar
        run[flags_off + 4] = 1; // bar size
        run[flags_off + 5] = 4; // marker size
        run[flags_off + 6] = 8; // marker shape
        let def = parse_chart_definition2(&run);
        assert!(!def.is_vertical_bar);
        assert_eq!(def.bar_size, ChartBarSize::Small);
        assert_eq!(def.marker_size, ChartMarkerSize::Large);
        assert_eq!(def.marker_shape, ChartMarkerShape::Triangle);
        // A code the SDK enum does not name is kept rather than folded into a neighbour.
        run[flags_off + 5] = 5;
        assert_eq!(
            parse_chart_definition2(&run).marker_size,
            ChartMarkerSize::Other(5)
        );
    }

    /// The per-axis gridline bytes decode from `flags_off + 7` (group/X axis) and `flags_off + 9`
    /// (value/Y axis), covering both stored configurations: the default `None`/`Major` and
    /// `Both`/`Both`. A pie-family record carries the same two fields two bytes later, because the
    /// family's extra mid-sequence pair precedes them — so the byte at the axis-family position is
    /// not a gridline mode there, and the shifted one is.
    #[test]
    fn axis_gridlines_decode_group_x_and_value_y() {
        use crate::model::ChartGridType;
        // flags_off matches synth_run: 2 enum bytes + the string block + a 1-byte separator.
        let flags_off = 2 + CHART_STRING_COUNT * 5 + 1;
        let with_grid = |graph_type: u8, at: usize, gx: u8, vy: u8| {
            let mut run = synth_run(graph_type, 0x01, 0, 0);
            run[flags_off + at] = gx;
            run[flags_off + at + 2] = vy;
            parse_chart_definition2(&run)
        };
        // Default axis chart: group axis None, value axis Major.
        let def = with_grid(0, 7, 0, 2);
        assert_eq!(def.group_axis_gridlines, ChartGridType::None);
        assert_eq!(def.value_axis_gridlines, ChartGridType::Major);
        // Both axes set to Both.
        let def = with_grid(0, 7, 3, 3);
        assert_eq!(def.group_axis_gridlines, ChartGridType::Both);
        assert_eq!(def.value_axis_gridlines, ChartGridType::Both);
        // Pie family (type 3): the same fields, two bytes on.
        let def = with_grid(3, 9, 0, 2);
        assert_eq!(def.group_axis_gridlines, ChartGridType::None);
        assert_eq!(def.value_axis_gridlines, ChartGridType::Major);
        // Written at the axis-family position instead, the pair lands one field early: the group
        // axis picks up what was meant for the value axis and the value axis reads past the pair.
        let def = with_grid(3, 7, 3, 3);
        assert_eq!(def.group_axis_gridlines, ChartGridType::Both);
        assert_eq!(def.value_axis_gridlines, ChartGridType::None);
    }

    /// `ChartGridType::from_code` maps the `CrGridTypeEnum` bitmask (bit0 minor, bit1 major).
    #[test]
    fn chart_grid_type_from_code() {
        use crate::model::ChartGridType;
        assert_eq!(ChartGridType::from_code(0), ChartGridType::None);
        assert_eq!(ChartGridType::from_code(1), ChartGridType::Minor);
        assert_eq!(ChartGridType::from_code(2), ChartGridType::Major);
        assert_eq!(ChartGridType::from_code(3), ChartGridType::Both);
    }

    /// `from_code` maps the raw legend-position enum byte independently of the field walk.
    #[test]
    fn legend_position_from_code() {
        assert_eq!(
            ChartLegendPosition::from_code(0),
            ChartLegendPosition::Right
        );
        assert_eq!(ChartLegendPosition::from_code(1), ChartLegendPosition::Left);
        assert_eq!(
            ChartLegendPosition::from_code(2),
            ChartLegendPosition::BottomCenter
        );
        assert_eq!(
            ChartLegendPosition::from_code(3),
            ChartLegendPosition::Custom
        );
        // Any unsampled code falls back to the engine default Right.
        assert_eq!(
            ChartLegendPosition::from_code(4),
            ChartLegendPosition::Right
        );
    }

    /// The legend-visible flag is bit0 of the legend `short`'s low byte.
    #[test]
    fn legend_visible_is_bit0() {
        assert!(parse_chart_definition2(&synth_run(0, 0x01, 0, 0)).legend_visible);
        assert!(!parse_chart_definition2(&synth_run(0, 0x00, 0, 0)).legend_visible);
    }

    /// The data-labels "show value" flag is bit1 of the data-labels enum byte, 81 bytes past the
    /// legend short for an axis (bar/line/area) chart.
    #[test]
    fn data_labels_show_value_bit1_axis() {
        assert!(!parse_chart_definition2(&synth_run(0, 0x01, 0, 0x00)).data_labels_show_value);
        assert!(parse_chart_definition2(&synth_run(0, 0x01, 0, 0x02)).data_labels_show_value);
        // bit0 alone (a different label mode) is not "show value".
        assert!(!parse_chart_definition2(&synth_run(0, 0x01, 0, 0x01)).data_labels_show_value);
    }

    /// Pie/doughnut charts (type 3/4) carry two extra mid-struct enum bytes, so the data-labels byte
    /// sits at +83 rather than +81. Build a pie run with the show-value bit at the shifted offset.
    #[test]
    fn data_labels_show_value_pie_family_shift() {
        // Manually build a pie run: same prefix as synth_run but data-labels at flags_off+83.
        let build = |gt: u8, dl_extra: usize| {
            let mut v = vec![gt, 0];
            for _ in 0..CHART_STRING_COUNT {
                v.extend_from_slice(&[0, 0, 0, 1, 0]);
            }
            v.push(0);
            let flags_off = v.len();
            v.push(0x01); // visible
            v.push(0x00); // right
            let dl_off = flags_off + 81 + dl_extra;
            v.resize(dl_off, 0);
            v.push(0x02); // show value
            v
        };
        // Pie (3): decoder must read at +83; the +81 slot is zero, so only the shift decodes true.
        let pie = parse_chart_definition2(&build(3, 2));
        assert_eq!(pie.graph_type, ChartGraphType::Pie);
        assert!(pie.data_labels_show_value, "pie reads data-labels at +83");
        // Doughnut (code 4) is a distinct pie-family type that shares the +2 shift.
        let doughnut = parse_chart_definition2(&build(4, 2));
        assert_eq!(doughnut.graph_type, ChartGraphType::Doughnut);
        assert!(
            doughnut.data_labels_show_value,
            "doughnut reads data-labels at +83"
        );
    }

    /// The 3-D camera preset decodes from the `+0x4cc` enum byte, two bytes past the data-labels
    /// byte (`+0x4a8`). Only the 3-D families (graph_type 5/6) interpret it; the stored value is the
    /// 1-based `CrViewingAngleEnum` ordinal (Standard = 1, DistortedView = 4).
    #[test]
    fn view_angle_decodes_for_3d_families() {
        use crate::model::ChartViewAngle;
        // Build a run whose view-angle byte (flags_off + 83 = data_label_off + 2) carries `code`.
        let build = |gt: u8, code: u8| {
            let mut v = vec![gt, 0];
            for _ in 0..CHART_STRING_COUNT {
                v.extend_from_slice(&[0, 0, 0, 1, 0]);
            }
            v.push(0);
            let flags_off = v.len();
            v.push(0x01); // legend visible
            v.push(0x00); // right
            let va_off = flags_off + 83; // = data_label_off (flags_off+81, non-pie) + 2 = +0x4cc
            v.resize(va_off, 0);
            v.push(code);
            v
        };
        // 3-D Riser (5) and 3-D Surface (6) both read the byte; the value is the 1-based ordinal.
        assert_eq!(
            parse_chart_definition2(&build(5, 1)).view_angle,
            ChartViewAngle::Standard
        );
        assert_eq!(
            parse_chart_definition2(&build(5, 4)).view_angle,
            ChartViewAngle::DistortedView
        );
        assert_eq!(
            parse_chart_definition2(&build(6, 15)).view_angle,
            ChartViewAngle::BirdsEyeView
        );
        // 0 (custom/unset) falls back to Standard.
        assert_eq!(
            parse_chart_definition2(&build(5, 0)).view_angle,
            ChartViewAngle::Standard
        );
        // A 2-D family (bar) never carries a meaningful view angle — the byte in that position is
        // not interpreted, so it stays at the default `Standard`.
        assert_eq!(
            parse_chart_definition2(&build(0, 4)).view_angle,
            ChartViewAngle::Standard
        );
    }

    /// The per-element font run decodes into `element_fonts` in element order with index 0 = the
    /// Title element, keeping verbatim face names; the GroupLabel and SeriesLabel faces and every
    /// element's default flag come from the block past the styling struct.
    #[test]
    fn element_fonts_decode() {
        // A length-prefixed string: 4-byte BE prefix = content length incl. trailing NUL.
        fn lp(s: &str) -> Vec<u8> {
            let mut v = ((s.len() + 1) as u32).to_be_bytes().to_vec();
            v.extend_from_slice(s.as_bytes());
            v.push(0);
            v
        }
        let build = |title_face: &str, flags: [u16; CHART_TEXT_ELEMENTS]| {
            let mut v = vec![0u8, 0]; // graph_type (bar), subtype
                                      // 8 leading strings (title/subtitle/footnote/2 masks/group/data axis + 1 separator),
                                      // all empty; the font run's element order is what this test exercises.
            for _ in 0..8 {
                v.extend_from_slice(&[0, 0, 0, 1, 0]);
            }
            // 8 contiguous per-element font names: index 0 = Title, the rest default Arial.
            v.extend_from_slice(&lp(title_face));
            for _ in 0..7 {
                v.extend_from_slice(&lp("Arial"));
            }
            // The styling struct, then the GroupLabel/SeriesLabel faces and the flag array.
            v.resize(v.len() + styling_struct_len(0), 0);
            v.extend_from_slice(&lp("Arial"));
            v.extend_from_slice(&lp("Arial"));
            for f in flags {
                v.extend_from_slice(&f.to_be_bytes());
            }
            v
        };
        let mut flags = [ELEMENT_FONT_DEFAULT; CHART_TEXT_ELEMENTS];
        flags[0] = 0x0100;
        flags[3] = 0x0122;
        let def = parse_chart_definition2(&build("Times New Roman", flags));
        assert_eq!(
            def.element_fonts.len(),
            CHART_TEXT_ELEMENTS,
            "ten text elements"
        );
        assert_eq!(
            def.element_fonts[0].name, "Times New Roman",
            "index 0 = Title"
        );
        for f in &def.element_fonts[1..] {
            assert_eq!(f.name, "Arial", "non-Title elements keep the default face");
        }
        let authored: Vec<usize> = def
            .element_fonts
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.is_default)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            authored,
            vec![0, 3],
            "only the non-0x0110 elements are authored"
        );

        // An all-default flag array marks every element as engine-default.
        let def =
            parse_chart_definition2(&build("Arial", [ELEMENT_FONT_DEFAULT; CHART_TEXT_ELEMENTS]));
        assert!(def.element_fonts.iter().all(|f| f.is_default));
    }

    /// Every element's weight and slant come from its own entry in the style block that follows the
    /// flag array, and the one element the block omits keeps the unstored weight `0`.
    #[test]
    fn element_style_decodes_per_element() {
        fn lp(s: &str) -> Vec<u8> {
            let mut v = ((s.len() + 1) as u32).to_be_bytes().to_vec();
            v.extend_from_slice(s.as_bytes());
            v.push(0);
            v
        }
        let mut v = vec![0u8, 0];
        for _ in 0..8 {
            v.extend_from_slice(&[0, 0, 0, 1, 0]);
        }
        for _ in 0..8 {
            v.extend_from_slice(&lp("Arial"));
        }
        v.resize(v.len() + styling_struct_len(0), 0);
        v.extend_from_slice(&lp("Arial"));
        v.extend_from_slice(&lp("Arial"));
        // The flag array, then a style block giving element `i` weight `400 + i` and italic on the
        // odd elements, so a swapped or off-by-one offset shows up as a wrong element.
        let flags_at = v.len();
        v.resize(flags_at + ELEMENT_STYLE_END, 0);
        for i in 0..CHART_TEXT_ELEMENTS {
            v[flags_at + i * 2..flags_at + i * 2 + 2]
                .copy_from_slice(&ELEMENT_FONT_DEFAULT.to_be_bytes());
        }
        for style in &ELEMENT_STYLE_OFFSETS {
            let at = flags_at + style.weight;
            v[at..at + 2].copy_from_slice(&(400 + style.element as u16).to_be_bytes());
            v[flags_at + style.italic] = u8::from(style.element % 2 == 1);
        }
        let def = parse_chart_definition2(&v);
        let got: Vec<(u16, bool)> = def
            .element_fonts
            .iter()
            .map(|f| (f.weight, f.italic))
            .collect();
        assert_eq!(
            got,
            vec![
                (400, false),
                (401, true),
                (402, false),
                (403, true),
                (404, false),
                (405, true),
                (406, false),
                (407, true),
                (0, false),
                (409, true),
            ],
            "each element reads its own entry; DataLabel (index 8) has none"
        );
    }

    /// A record that ends before the trailing font block keeps the faces from the string run at their
    /// element indices, and reports every element as engine-default with no stored weight rather than
    /// emitting half a table.
    #[test]
    fn element_fonts_without_trailing_block() {
        fn lp(s: &str) -> Vec<u8> {
            let mut v = ((s.len() + 1) as u32).to_be_bytes().to_vec();
            v.extend_from_slice(s.as_bytes());
            v.push(0);
            v
        }
        let mut v = vec![0u8, 0];
        for _ in 0..8 {
            v.extend_from_slice(&[0, 0, 0, 1, 0]);
        }
        for _ in 0..8 {
            v.extend_from_slice(&lp("Arial"));
        }
        let def = parse_chart_definition2(&v);
        assert_eq!(def.element_fonts.len(), CHART_TEXT_ELEMENTS);
        assert!(def.element_fonts.iter().all(|f| f.weight == 0));
        // The two elements whose faces live past the styling struct read as unnamed; the run's eight
        // are placed at their own element indices and marked engine-default.
        for (i, font) in def.element_fonts.iter().enumerate() {
            let trailing = FACE_TRAILING_ELEMENTS.contains(&i);
            assert_eq!(font.name.is_empty(), trailing, "element {i} face");
            assert_eq!(font.is_default, !trailing, "element {i} default flag");
        }
    }

    /// A short/truncated run must not panic and defaults sensibly (visible, no data labels).
    #[test]
    fn short_run_defaults() {
        let def = parse_chart_definition2(&[0x00, 0x00]);
        assert!(def.legend_visible);
        assert!(!def.data_labels_show_value);
    }
}
