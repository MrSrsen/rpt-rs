//! Chart / cross-tab field-binding collection — the shared binding-scope walk skeleton and the
//! per-object grid bindings decoded from the report's binding region, plus the post-walk pass that
//! attaches every decoded chart / cross-tab detail onto its object.

use super::*;
use crate::model::area_objects_mut;

/// Attach each chart / cross-tab object's decoded field bindings (decoded by name from the separate
/// binding region; see [`collect_grid_bindings`]). Runs after picture openers are reclassified so
/// the chart / cross-tab kinds are settled.
pub(super) fn attach_grid_bindings(
    tree: &[RecordNode],
    logical: &[u8],
    areas: &mut [Area],
    groups: &[Group],
    field_types: &std::collections::HashMap<String, crate::model::FieldValueType>,
) {
    let bindings = collect_grid_bindings(tree, logical);
    // A chart data summary over a date "on change of" category carries a `"daily"` third operand
    // (the summary's grouping condition), which the engine reports for every date-valued category
    // regardless of its visual grouping period. A category is a date field when either the report
    // groups on it with a date condition, or — for a pure single-chart date axis with no report
    // group — its datasource field type is Date/DateTime (looked up by the lowercase `alias.field`
    // reference in `field_types`, threaded from the QESession schema).
    let date_group_fields: std::collections::HashSet<&str> = groups
        .iter()
        .filter(|g| g.date_condition.is_some())
        .map(|g| g.condition_field.as_str())
        .collect();
    let is_date_scope = |cat: &str| -> bool {
        date_group_fields.contains(cat)
            || matches!(
                field_types.get(&cat.to_lowercase()),
                Some(crate::model::FieldValueType::Date | crate::model::FieldValueType::DateTime)
            )
    };
    // The report's deduped summary defs `(operation, field)` — the data source of a report-group
    // ("Top-N") chart, whose own `0xb4` block carries no bindings and which instead charts the
    // report's summary field(s) over the report's group(s). Non-running-total summaries only (the
    // running-total defs are `0x80`-reset and excluded by `collect_summary_defs`); field-shaped
    // operands only; duplicates (the engine writes one summary def per placed summary object, so the
    // same `Op (field)` recurs) collapsed to one, preserving order.
    let report_summaries: Vec<(crate::model::SummaryOperation, String)> = {
        let mut seen: Vec<(crate::model::SummaryOperation, String)> = Vec::new();
        for (op, field, _vt) in collect_summary_defs(tree, logical) {
            if is_field_ref(&field) && !seen.contains(&(op, field.clone())) {
                seen.push((op, field));
            }
        }
        seen
    };
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
                // A chart binds its fields one of two ways. A *grid-group* chart carries its own
                // bindings in its `0xb4` block (data `0x7f`/`0x7e`, category grid `0xe5`). A
                // *report-group* ("Top-N") chart has an empty `0xb4` block and instead charts the
                // report's own group(s) over the report's summary field(s). Treat a missing or empty
                // grid binding as the report-group case.
                let grid = bindings
                    .get(&obj.name)
                    .filter(|r| !r.data.is_empty() || !r.category.is_empty());
                if let Some(refs) = grid {
                    c.data_refs = refs.data.clone();
                    c.category_refs = refs.category.clone();
                    // Set the category period after the definition is assigned (which replaces it),
                    // so the grid-group-decoded period survives.
                    c.definition.category_period = refs.category_period;
                    // Compose the RAS `DataFields` / `ConditionFields` summary-form strings. The
                    // scoping (innermost) category is the last one; its explicit period (if any)
                    // becomes the summary's third operand.
                    let data: Vec<_> = refs
                        .data_ops
                        .iter()
                        .copied()
                        .zip(refs.data.iter().cloned())
                        .collect();
                    // The summary's grouping period is `"daily"` when the scoping (innermost)
                    // category is a date field, else absent (see `is_date_scope`).
                    let scoping_is_date = refs.category.last().is_some_and(|c| is_date_scope(c));
                    let scoping_period = scoping_is_date.then_some("daily");
                    let (data_forms, cond_forms) =
                        compose_chart_refs(&data, &refs.category, scoping_period);
                    c.definition.data_refs = data_forms;
                    c.definition.category_refs = cond_forms;
                } else if !groups.is_empty() && !report_summaries.is_empty() {
                    // Report-group ("Top-N") chart: category = each report group's condition field
                    // (outer→inner); data = each report summary summarized over the innermost group,
                    // `Op ({field}, {innermost group}[, "period"])`. Only the DataDefinition
                    // (`ChartDefinition`) refs are set here — the bare `ChartObject.data_refs` /
                    // `category_refs` are left untouched, as a report-group chart references the
                    // report's own group/summary fields, not new ones.
                    let categories: Vec<String> =
                        groups.iter().map(|g| g.condition_field.clone()).collect();
                    let scoping_is_date = categories.last().is_some_and(|c| is_date_scope(c));
                    let scoping_period = scoping_is_date.then_some("daily");
                    let (data_forms, cond_forms) =
                        compose_chart_refs(&report_summaries, &categories, scoping_period);
                    c.definition.data_refs = data_forms;
                    c.definition.category_refs = cond_forms;
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
                    // Fill each real (non-grand-total) level's field reference from the axis-tagged
                    // grid groups when the `0x00cb` level record omits it (designer-authored cross-
                    // tabs store the field only in the `0x00e5` grid group, not the level). The first
                    // level of each axis is the grand-total level (legitimately empty); the remaining
                    // levels are the real dimensions, in the same order as the grid groups.
                    if let Some(b) = bindings.get(&obj.name) {
                        let columns = b
                            .crosstab_columns
                            .iter()
                            .zip(b.crosstab_column_periods.iter())
                            .zip(b.crosstab_column_suppress.iter());
                        for (level, ((field, period), suppress)) in
                            c.columns.iter_mut().skip(1).zip(columns)
                        {
                            if level.field_ref.is_empty() {
                                level.field_ref = field.clone();
                            }
                            // The grid group is the only place a dimension's grouping period and its
                            // two suppress flags are stored (the `0x00cb` level record carries
                            // neither), so always take them.
                            level.period = *period;
                            (level.suppress_subtotal, level.suppress_label) = *suppress;
                        }
                        let rows = b
                            .crosstab_rows
                            .iter()
                            .zip(b.crosstab_row_periods.iter())
                            .zip(b.crosstab_row_suppress.iter());
                        for (level, ((field, period), suppress)) in
                            c.rows.iter_mut().skip(1).zip(rows)
                        {
                            if level.field_ref.is_empty() {
                                level.field_ref = field.clone();
                            }
                            level.period = *period;
                            (level.suppress_subtotal, level.suppress_label) = *suppress;
                        }
                    }
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
                // report; every summary is attributed to the cross-tab — see the collector).
                c.measures = crosstab_measures.clone();
                // The RAS `CrossTabFormat.CrossTabStyle` view mirrors `options`, but reflects the
                // grand-total colours as concrete engine COLORREF colours: the "auto" default
                // (stored `0xFFFFFFFF`, decoded to `None` on `options`) surfaces as white.
                const WHITE: crate::model::Color = crate::model::Color {
                    a: 255,
                    r: 255,
                    g: 255,
                    b: 255,
                };
                c.grid_format.style = crate::model::CrossTabGridOptions {
                    row_grand_total_color: Some(c.options.row_grand_total_color.unwrap_or(WHITE)),
                    column_grand_total_color: Some(
                        c.options.column_grand_total_color.unwrap_or(WHITE),
                    ),
                    ..c.options.clone()
                };
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
    // A chart binds a field in one of two roles the engine counts differently: a data ("show value")
    // field versus a category / cross-tab dimension (a grid group). Only field-shaped references are
    // kept; each role also captures its per-binding metadata (data → operation, category → period).
    // A chart owns a data ("show value") role; a cross-tab owns only categories. Track which opener
    // set the current scope so a `CHART_DATA` record is only read inside a chart block.
    let mut is_chart = false;
    for (current, node) in binding_scopes(tree, logical, &[CHART_BINDING, CROSSTAB_WRAPPER]) {
        match node.rtype {
            CHART_BINDING => is_chart = true,
            CROSSTAB_WRAPPER => is_chart = false,
            // A chart's data field (`0x7f` → `0x7e` field ref); only inside a chart block.
            CHART_DATA if is_chart && current.is_some() => {
                let field = first_string(node, logical);
                // Capture the summary operation (the `0x7e` child's byte 0) alongside the field, for
                // the RAS `DataFields` summary form; keep the two vecs parallel.
                if let (Some(name), Some(f)) = (&current, field.clone().filter(|s| is_field_ref(s)))
                {
                    let b = out.entry(name.clone()).or_default();
                    b.data.push(f);
                    b.data_ops.push(chart_data_op(node, logical));
                }
            }
            // A grid group is a chart category / cross-tab dimension binding (identified by marker).
            GROUP if current.is_some() && is_grid_group(node, logical) => {
                let field = first_string(node, logical);
                // Capture the category's explicit grouping period alongside its field, for the RAS
                // `DataFields` summary form; keep the two vecs parallel.
                if let (Some(name), Some(f)) = (&current, field.clone().filter(|s| is_field_ref(s)))
                {
                    let b = out.entry(name.clone()).or_default();
                    b.category.push(f.clone());
                    // A cross-tab dimension grid group also carries its axis in the marker string
                    // (`@Column #N Order` / `@Row #N Order`), the authoritative axis-tagged field
                    // reference. Designer-authored cross-tabs omit the field ref from the `0x00cb`
                    // level record, so this is the only stored place the real (non-grand-total)
                    // level's field reference survives.
                    match grid_group_axis(node, logical) {
                        Some(CrosstabAxis::Column) => {
                            b.crosstab_columns.push(f);
                            b.crosstab_column_periods
                                .push(grid_group_condition(node, logical));
                            b.crosstab_column_suppress
                                .push(grid_group_suppress(node, logical));
                        }
                        Some(CrosstabAxis::Row) => {
                            b.crosstab_rows.push(f);
                            b.crosstab_row_periods
                                .push(grid_group_condition(node, logical));
                            b.crosstab_row_suppress
                                .push(grid_group_suppress(node, logical));
                        }
                        None => {}
                    }
                }
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

/// The cross-tab axis of a `0x00e5` grid group, from its `@Column #N Order` / `@Row #N Order` marker
/// string. Returns `None` for a chart-category grid group (`… Grid #N`), which has no axis.
fn grid_group_axis(node: &RecordNode, logical: &[u8]) -> Option<CrosstabAxis> {
    all_strings(node, logical).iter().find_map(|s| {
        if s.starts_with("@Column #") {
            Some(CrosstabAxis::Column)
        } else if s.starts_with("@Row #") {
            Some(CrosstabAxis::Row)
        } else {
            None
        }
    })
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

/// Decode a cross-tab dimension grid `0x00e5` group's date grouping period from its SDK
/// `CrGroupConditionEnum` ordinal byte — the byte at `used + 3`, where `used` is the length of the
/// leading dimension-field reference. This is the identical encoding [`grid_group_period`] reads for
/// a chart category and `data_def::raise_group` reads for a report group; ordinal `0` (discrete or a
/// daily axis) returns `None`, so the render side keys the axis by raw value.
fn grid_group_condition(node: &RecordNode, logical: &[u8]) -> Option<crate::model::GroupCondition> {
    let leaf = node.leaf_bytes(logical);
    let (_, used) = read_lp_string(&leaf)?;
    let ordinal = leaf.get(used + 3).copied()?;
    crate::model::GroupCondition::from_date_ordinal(ordinal)
}

/// A cross-tab dimension grid `0x00e5` group's two axis-level suppress flags —
/// `(suppress_subtotal, suppress_label)`, the RAS `ICrossTabGroup.EnableSuppressSubtotal` /
/// `.EnableSuppressLabel` pair. Both are big-endian `u32` booleans in the record's fixed tail,
/// anchored on the end of the axis marker string (`@Column #N Order` / `@Row #N Order`) — the
/// record's last string — at `+5` and `+9`. Anchoring on the marker rather than an absolute offset
/// is required: the leading field reference and the two `Others` strings are variable-length.
/// A group with no axis marker is not a cross-tab dimension level and yields `(false, false)`.
fn grid_group_suppress(node: &RecordNode, logical: &[u8]) -> (bool, bool) {
    let leaf = node.leaf_bytes(logical);
    let Some(end) = axis_marker_end(&leaf) else {
        return (false, false);
    };
    let flag = |at: usize| u32_be(&leaf, at).is_some_and(|v| v != 0);
    (flag(end + 5), flag(end + 9))
}

/// The offset just past the axis marker string (`@Column #N Order` / `@Row #N Order`) in a cross-tab
/// dimension grid group's leaf. Scans every offset rather than walking the strings back-to-back —
/// the record interleaves fixed scalar fields between them, so they are not contiguous.
fn axis_marker_end(leaf: &[u8]) -> Option<usize> {
    lp_scan(leaf, Scan::Consume).find_map(|(off, s, used)| {
        (s.starts_with("@Column #") || s.starts_with("@Row #")).then_some(off + used)
    })
}

/// A chart data (`0x7f`) binding's summary operation — the operation byte (leaf byte 0) of its
/// `0x7e SUMMARY_DEF` child. Defaults to `Sum` (code 0) if the child is absent.
fn chart_data_op(node: &RecordNode, logical: &[u8]) -> crate::model::SummaryOperation {
    let code = node
        .children
        .iter()
        .find(|c| c.rtype == SUMMARY_DEF)
        .and_then(|c| c.leaf_bytes(logical).first().copied())
        .unwrap_or(0);
    crate::model::SummaryOperation::from_code(i32::from(code))
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

/// The set of object names that own a chart (each is nested in a `0xb4 CHART_BINDING` block). This is
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
        if node.rtype == CHART_BINDING {
            if let Some(name) = descendant_object_name(node, logical) {
                out.insert(name);
            }
        }
    }
    out
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

/// Whether a `0xe5` group record is a chart-category / cross-tab-dimension "grid" group (rather than
/// a report group), identified by its localized order-marker string.
fn is_grid_group(node: &RecordNode, logical: &[u8]) -> bool {
    all_strings(node, logical)
        .iter()
        .any(|s| s.contains(" Grid #") || s.starts_with("@Column #") || s.starts_with("@Row #"))
}
