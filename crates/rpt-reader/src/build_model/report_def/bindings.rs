//! Chart / cross-tab field-binding collection — the shared binding-scope walk skeleton and the
//! per-object grid bindings decoded from the report's binding region, plus the post-walk pass that
//! hands each object to the file named for its record family ([`super::chart`] / [`super::crosstab`]).

use super::chart::ChartAttach;
use super::crosstab::CrossTabAttach;
use crate::build_model::record_values::is_field_ref;
use crate::build_model::row_of;
use crate::build_model::tree_search::flatten;
use crate::codec::RecordNode;
use crate::field_table::table::Row;
use crate::field_table::tables as ft;
use crate::model::{area_objects_mut, Area, Group, ReportObjectKind};
use crate::records::rtype::*;

/// Attach each chart / cross-tab object's decoded detail onto it: the field bindings decoded by
/// name from the separate binding region (see [`collect_grid_bindings`]), and whatever the file
/// named for that object kind decodes for it. Runs after picture openers are reclassified so the
/// chart / cross-tab kinds are settled.
pub(super) fn attach_grid_bindings(
    tree: &[RecordNode],
    logical: &[u8],
    areas: &mut [Area],
    groups: &[Group],
    field_types: &std::collections::HashMap<String, crate::model::FieldValueType>,
) {
    let bindings = collect_grid_bindings(tree, logical);
    let mut charts = ChartAttach::new(tree, logical, groups, field_types);
    let mut crosstabs = CrossTabAttach::new(tree, logical);
    for obj in area_objects_mut(areas) {
        let refs = bindings.get(&obj.name);
        match &mut obj.kind {
            ReportObjectKind::Chart(c) => charts.attach(&obj.name, c, refs),
            ReportObjectKind::CrossTab(c) => crosstabs.attach(&obj.name, c, refs),
            _ => {}
        }
    }
}

/// Collect each chart / cross-tab object's persistent field bindings from the report's binding
/// region (a flat run of sibling records that follows the layout), keyed by object name.
///
/// The binding records reuse the generic group machinery, so each is scoped precisely:
/// - A **chart** binding block starts with `0xb4` (whose `ObjectName` is a descendant rather than a
///   child, reached through the `0xb3` analytic object and its `0xae` graphic base); its data
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
pub(super) fn collect_grid_bindings(
    tree: &[RecordNode],
    logical: &[u8],
) -> std::collections::HashMap<String, GridBindings> {
    let mut out: std::collections::HashMap<String, GridBindings> = std::collections::HashMap::new();
    // A chart binds a field in one of two roles the engine counts differently: a data ("show value")
    // field versus a category / cross-tab dimension (a grid group). Only field-shaped references are
    // kept; each role also captures its per-binding metadata (data → operation, category → period).
    // A chart owns a data ("show value") role; a cross-tab owns only categories. Track which opener
    // set the current scope so a `SUMMARY_FIELD_WRAPPER` record is only read inside a chart block.
    let mut is_chart = false;
    for (current, node) in binding_scopes(tree, logical, &[CHART_OBJECT, CROSSTAB_WRAPPER]) {
        match node.rtype {
            CHART_OBJECT => is_chart = true,
            CROSSTAB_WRAPPER => is_chart = false,
            // A chart's data field: the summary the `0x7f` wrapper contains names it. Only inside a
            // chart block.
            SUMMARY_FIELD_WRAPPER if is_chart && current.is_some() => {
                let summary = node
                    .children
                    .iter()
                    .find(|c| c.rtype == SUMMARY_FIELD_DEFINITION)
                    .map(|c| row_of(c, logical, &ft::SUMMARY_FIELD_DEFINITION))
                    .unwrap_or_default();
                let field = summary.text("operand");
                // The summary's operation travels with the field, for the summary form a binding is
                // reported in; keep the two vecs parallel.
                if let (Some(name), true) = (&current, is_field_ref(field)) {
                    let b = out.entry(name.clone()).or_default();
                    b.data.push(field.to_owned());
                    b.data_ops.push(crate::model::SummaryOperation::from_code(
                        summary.u("operation") as i32,
                    ));
                }
            }
            // A grid group is a chart category / cross-tab dimension binding (identified by marker).
            GROUP if current.is_some() => {
                let group = row_of(node, logical, &ft::GROUP);
                if !is_grid_group(&group) {
                    continue;
                }
                let f = group.text("condition_field").to_owned();
                // Capture the category's explicit grouping period alongside its field, for the RAS
                // `DataFields` summary form; keep the two vecs parallel.
                if let (Some(name), true) = (&current, is_field_ref(&f)) {
                    let b = out.entry(name.clone()).or_default();
                    b.category.push(f.clone());
                    // A cross-tab dimension grid group also carries its axis in the marker string
                    // (`@Column #N Order` / `@Row #N Order`), the authoritative axis-tagged field
                    // reference. Designer-authored cross-tabs omit the field ref from the `0x00cb`
                    // level record, so this is the only stored place the real (non-grand-total)
                    // level's field reference survives.
                    match grid_group_axis(&group) {
                        Some(CrosstabAxis::Column) => {
                            b.crosstab_columns.push(f);
                            b.crosstab_column_periods.push(grid_group_condition(&group));
                            b.crosstab_column_suppress.push(grid_group_suppress(&group));
                        }
                        Some(CrosstabAxis::Row) => {
                            b.crosstab_rows.push(f);
                            b.crosstab_row_periods.push(grid_group_condition(&group));
                            b.crosstab_row_suppress.push(grid_group_suppress(&group));
                        }
                        None => {}
                    }
                }
                // For a chart category that is a date field grouped by a period, decode the period
                // from the grid group's SDK-ordinal byte (same encoding as a report group). Keep the
                // first category's period per chart (the "on change of" axis).
                if is_chart {
                    if let (Some(name), Some(period)) = (&current, grid_group_period(&group)) {
                        out.entry(name.clone())
                            .or_default()
                            .category_period
                            .get_or_insert(period);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// One chart/cross-tab object's field bindings, split by the role the engine binds them in: `data`
/// are a chart's "show value" data fields; `category` are chart "on change of" categories and
/// cross-tab row/column dimensions (the `0xe5` grid groups). A cross-tab has only `category`
/// bindings.
#[derive(Default)]
pub(super) struct GridBindings {
    pub(super) data: Vec<String>,
    pub(super) category: Vec<String>,
    /// The chart's "on change of `<date>`" category period, decoded from the first date-periodic
    /// category grid group (see [`grid_group_period`]). `None` for a discrete category or a
    /// cross-tab.
    pub(super) category_period: Option<crate::model::ChartCategoryPeriod>,
    /// The summary operation of each chart data ("show value") binding, parallel to [`data`](Self::data)
    /// — read from the `0x7e` child's operation byte. Used to build the RAS `DataFields` summary form.
    pub(super) data_ops: Vec<crate::model::SummaryOperation>,
    /// A cross-tab's column-axis dimension field references, in level order (real, non-grand-total
    /// levels only), read from the `@Column #N Order` grid groups. Fills the `0x00cb` levels'
    /// field references for designer-authored cross-tabs, which omit them from the level record.
    pub(super) crosstab_columns: Vec<String>,
    /// A cross-tab's row-axis dimension field references, in level order — the `@Row #N Order` groups.
    pub(super) crosstab_rows: Vec<String>,
    /// A cross-tab's column-axis dimension grouping periods, parallel to [`crosstab_columns`](Self::crosstab_columns)
    /// — the date/time interval each column level buckets by, decoded from the same grid group's SDK
    /// `CrGroupConditionEnum` ordinal byte. `None` for a discrete (non-periodic) level.
    pub(super) crosstab_column_periods: Vec<Option<crate::model::GroupCondition>>,
    /// A cross-tab's row-axis dimension grouping periods, parallel to [`crosstab_rows`](Self::crosstab_rows).
    pub(super) crosstab_row_periods: Vec<Option<crate::model::GroupCondition>>,
    /// A cross-tab's column-axis `(suppress_subtotal, suppress_label)` pairs, parallel to
    /// [`crosstab_columns`](Self::crosstab_columns) — see [`grid_group_suppress`].
    pub(super) crosstab_column_suppress: Vec<(bool, bool)>,
    /// A cross-tab's row-axis `(suppress_subtotal, suppress_label)` pairs, parallel to
    /// [`crosstab_rows`](Self::crosstab_rows).
    pub(super) crosstab_row_suppress: Vec<(bool, bool)>,
}

/// The axis a cross-tab dimension grid group binds, from its marker string.
enum CrosstabAxis {
    Column,
    Row,
}

/// The cross-tab axis of a `0x00e5` grid group, from its `@Column #N Order` / `@Row #N Order` order
/// marker. Returns `None` for a chart-category grid group (`… Grid #N`), which has no axis.
fn grid_group_axis(group: &Row) -> Option<CrosstabAxis> {
    let marker = group.text("order_marker");
    if marker.starts_with("@Column #") {
        Some(CrosstabAxis::Column)
    } else if marker.starts_with("@Row #") {
        Some(CrosstabAxis::Row)
    } else {
        None
    }
}

/// Decode a chart-category grid `0xe5` group's date-grouping period from its SDK condition ordinal,
/// via [`ChartCategoryPeriod::from_sdk_ordinal`](crate::model::ChartCategoryPeriod::from_sdk_ordinal).
/// This is the identical encoding `data_def::build_group` reads for a report group's period; a
/// discrete (non-periodic) category stores ordinal `0` and returns `None`.
fn grid_group_period(group: &Row) -> Option<crate::model::ChartCategoryPeriod> {
    crate::model::ChartCategoryPeriod::from_sdk_ordinal(group.u("condition_ordinal") as u8)
}

/// Decode a cross-tab dimension grid `0x00e5` group's date grouping period from its SDK
/// `CrGroupConditionEnum` condition ordinal. This is the identical encoding [`grid_group_period`]
/// reads for a chart category and `data_def::build_group` reads for a report group; ordinal `0`
/// (discrete or a daily axis) returns `None`, so the render side keys the axis by raw value.
fn grid_group_condition(group: &Row) -> Option<crate::model::GroupCondition> {
    crate::model::GroupCondition::from_date_ordinal(group.u("condition_ordinal") as u8)
}

/// A cross-tab dimension grid `0x00e5` group's two axis-level suppress flags —
/// `(suppress_subtotal, suppress_label)`, the RAS `ICrossTabGroup.EnableSuppressSubtotal` /
/// `.EnableSuppressLabel` pair — two of the four adjacent words between the group's order reference
/// and its name formula (see the record's table for the other two).
/// A group with no axis marker is not a cross-tab dimension level and yields `(false, false)`.
fn grid_group_suppress(group: &Row) -> (bool, bool) {
    if grid_group_axis(group).is_none() {
        return (false, false);
    }
    (
        group.i("suppress_subtotal") != 0,
        group.i("suppress_label") != 0,
    )
}

/// Whether a `0xe5` record is a chart-category / cross-tab-dimension "grid" group (rather than one
/// of the report's own group levels), from the localized name of the order field it generates.
fn is_grid_group(group: &Row) -> bool {
    let marker = group.text("order_marker");
    marker.contains(" Grid #") || marker.starts_with("@Column #") || marker.starts_with("@Row #")
}

/// The shared binding-collector walk skeleton: flatten the tree and yield `(current_object_name,
/// node)` for every node, where `current_object_name` is the chart / cross-tab named by the most
/// recent opener record (any rtype in `openers`, via [`descendant_object_name`]) and is reset to
/// `None` at each `AREA`/`SECTION` layout marker. Each binding collector supplies its opener set and
/// keeps its own record-specific state in the loop body.
pub(super) fn binding_scopes<'a>(
    tree: &'a [RecordNode],
    logical: &'a [u8],
    openers: &'a [u16],
) -> impl Iterator<Item = (Option<String>, &'a RecordNode)> + 'a {
    flatten(tree)
        .into_iter()
        .scan(None::<String>, move |current, node| {
            if openers.contains(&node.rtype) {
                *current = descendant_object_name(node, logical);
            } else if matches!(node.rtype, AREA | SECTION) {
                *current = None;
            }
            Some((current.clone(), node))
        })
}

/// The set of object names that own a chart (each is nested in a `0xb4 CHART_OBJECT` block). This is
/// the authoritative signal that a layout picture opener is a chart — the engine auto-names charts
/// `Graph…`, but a user-renamed chart (e.g. `Chart1`) has no such prefix, so a name-based test alone
/// misroutes it to a blob field. The binding block always carries the chart's name regardless of what
/// the user renamed it to.
pub(super) fn collect_chart_object_names(
    tree: &[RecordNode],
    logical: &[u8],
) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for node in flatten(tree) {
        if node.rtype == CHART_OBJECT {
            if let Some(name) = descendant_object_name(node, logical) {
                out.insert(name);
            }
        }
    }
    out
}

/// The object name nested in a chart/cross-tab wrapper: the name field of the first `OBJECT_NAME`
/// (`0x9e`) descendant. Both halves are load-bearing — the wrapper's own field bytes can decode a spurious
/// short string, and within the `0x9e` record the name must be taken at its position rather than by
/// scanning, since the record's leading size words can themselves decode as a string.
fn descendant_object_name(node: &RecordNode, logical: &[u8]) -> Option<String> {
    let mut found = None;
    node.walk(&mut |n| {
        if found.is_none() && n.rtype == OBJECT_NAME {
            let name = row_of(n, logical, &ft::OBJECT_NAME).text("name").to_owned();
            found = (!name.is_empty()).then_some(name);
        }
    });
    found
}
