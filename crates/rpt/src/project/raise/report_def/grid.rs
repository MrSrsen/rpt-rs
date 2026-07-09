//! Chart / cross-tab field-binding collection — the shared binding-scope walk skeleton and the
//! per-object grid bindings decoded from the report's binding region, plus the post-walk pass that
//! attaches every decoded chart / cross-tab detail onto its object.

use super::*;
use crate::model::area_objects_mut;

/// Attach each chart / cross-tab object's decoded field bindings (decoded by name from the separate
/// binding region; see [`collect_grid_bindings`]). Runs after picture openers are reclassified so
/// the chart / cross-tab kinds are settled.
pub(super) fn attach_grid_bindings(tree: &[RecordNode], logical: &[u8], areas: &mut [Area]) {
    let bindings = collect_grid_bindings(tree, logical);
    let mut styles = collect_chart_styles(tree, logical);
    let mut crosstab_dims = collect_crosstab_dimensions(tree, logical);
    let crosstab_measures = collect_crosstab_measures(tree, logical);
    let mut crosstab_grid = collect_crosstab_grid(tree, logical);
    for obj in area_objects_mut(areas) {
        match &mut obj.kind {
            ReportObjectKind::Chart(c) => {
                if let Some(def) = styles.remove(&obj.name) {
                    c.definition = def;
                }
                if let Some(refs) = bindings.get(&obj.name) {
                    c.data_refs = refs.data.clone();
                    c.category_refs = refs.category.clone();
                    // Set the category period after the definition is assigned (which replaces it),
                    // so the grid-group-decoded period survives.
                    c.definition.category_period = refs.category_period;
                }
            }
            // Every cross-tab grid binding is a row/column dimension (no data role here).
            ReportObjectKind::CrossTab(c) => {
                if let Some(refs) = bindings.get(&obj.name) {
                    c.field_refs = refs.category.clone();
                }
                if let Some(s) = crosstab_dims.remove(&obj.name) {
                    c.dimensions = s.dimensions;
                    c.columns = s.columns;
                    c.rows = s.rows;
                    // Axes are cross-wired as the SDK exposes them: the column-axis grand-total
                    // level's colour is RAS `RowGrandTotalColor`, and vice versa.
                    c.options.row_grand_total_color = s.column_gt_color;
                    c.options.column_grand_total_color = s.row_gt_color;
                }
                if let Some(g) = crosstab_grid.remove(&obj.name) {
                    c.grid_format = g.grid_format;
                    c.column_axis_options = g.column_axis_options;
                    c.row_axis_options = g.row_axis_options;
                    c.options.show_grid = g.options.show_grid;
                    c.options.show_cell_margins = g.options.show_cell_margins;
                    c.options.keep_columns_together = g.options.keep_columns_together;
                    c.options.repeat_row_labels = g.options.repeat_row_labels;
                    c.options.suppress_empty_rows = g.options.suppress_empty_rows;
                    c.options.suppress_empty_columns = g.options.suppress_empty_columns;
                    c.options.suppress_row_grand_totals = g.options.suppress_row_grand_totals;
                    c.options.suppress_column_grand_totals = g.options.suppress_column_grand_totals;
                }
                // The data-cell measures are the report's pre-layout summary defs (shared across the
                // report; in the corpus every summary is a cross-tab measure — see the collector).
                c.measures = crosstab_measures.clone();
            }
            _ => {}
        }
    }
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
pub(super) fn collect_grid_bindings(
    tree: &[RecordNode],
    logical: &[u8],
) -> std::collections::HashMap<String, GridBindings> {
    let mut out: std::collections::HashMap<String, GridBindings> = std::collections::HashMap::new();
    // `is_category` selects which role the field is bound in: a chart's data ("show value") field
    // versus a category / cross-tab dimension (a grid group). The engine counts them differently.
    let push = |out: &mut std::collections::HashMap<String, GridBindings>,
                cur: &Option<String>,
                field: Option<String>,
                is_category: bool| {
        if let (Some(name), Some(f)) = (cur, field.filter(|s| is_field_ref(s))) {
            let b = out.entry(name.clone()).or_default();
            if is_category {
                b.category.push(f);
            } else {
                b.data.push(f);
            }
        }
    };
    // A chart owns a data ("show value") role; a cross-tab owns only categories. Track which opener
    // set the current scope so a `CHART_DATA` record is only read inside a chart block.
    let mut is_chart = false;
    for (current, node) in binding_scopes(tree, logical, &[CHART_BINDING, CROSSTAB_WRAPPER]) {
        match node.rtype {
            CHART_BINDING => is_chart = true,
            CROSSTAB_WRAPPER => is_chart = false,
            // A chart's data field (`0x7f` → `0x7e` field ref); only inside a chart block.
            CHART_DATA if is_chart && current.is_some() => {
                push(&mut out, &current, first_string(node, logical), false);
            }
            // A grid group is a chart category / cross-tab dimension binding (identified by marker).
            GROUP if current.is_some() && is_grid_group(node, logical) => {
                push(&mut out, &current, first_string(node, logical), true);
                // For a chart category that is a date field grouped by a period, decode the period
                // from the grid group's SDK-ordinal byte (same encoding as a report group). Keep the
                // first category's period per chart (the "on change of" axis).
                if is_chart {
                    if let (Some(name), Some(period)) = (&current, grid_group_period(node, logical))
                    {
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

/// One chart/cross-tab object's field bindings, split by the role the engine binds them in (it
/// references each role a different number of times for `Field.UseCount`): `data` are a chart's
/// "show value" data fields; `category` are chart "on change of" categories and cross-tab row/column
/// dimensions (the `0xe5` grid groups). A cross-tab has only `category` bindings.
#[derive(Default)]
pub(super) struct GridBindings {
    pub(super) data: Vec<String>,
    pub(super) category: Vec<String>,
    /// The chart's "on change of `<date>`" category period, decoded from the first date-periodic
    /// category grid group (see [`grid_group_period`]). `None` for a discrete category or a
    /// cross-tab.
    pub(super) category_period: Option<crate::model::ChartCategoryPeriod>,
}

/// Decode a chart-category grid `0xe5` group's date-grouping period from its SDK-ordinal byte — the
/// byte at `used + 3`, where `used` is the length of the leading category-field reference — via
/// [`ChartCategoryPeriod::from_sdk_ordinal`](crate::model::ChartCategoryPeriod::from_sdk_ordinal).
/// This is the identical encoding `data_def::raise_group` reads for a report group's period; a
/// discrete (non-periodic) category stores ordinal `0` and returns `None`.
fn grid_group_period(
    node: &RecordNode,
    logical: &[u8],
) -> Option<crate::model::ChartCategoryPeriod> {
    let leaf = node.leaf_bytes(logical);
    let (_, used) = read_lp_string(&leaf)?;
    let ordinal = leaf.get(used + 3).copied()?;
    crate::model::ChartCategoryPeriod::from_sdk_ordinal(ordinal)
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
            } else if matches!(node.rtype, AREA_MARKER | SECTION_MARKER) {
                *current = None;
            }
            Some((current.clone(), node))
        })
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
pub(super) fn is_field_ref(s: &str) -> bool {
    s.starts_with('@') || s.contains('.')
}

/// Whether a `0xe5` group record is a chart-category / cross-tab-dimension "grid" group (rather than
/// a report group), identified by its localized order-marker string.
fn is_grid_group(node: &RecordNode, logical: &[u8]) -> bool {
    all_strings(node, logical)
        .iter()
        .any(|s| s.contains(" Grid #") || s.starts_with("@Column #") || s.starts_with("@Row #"))
}
