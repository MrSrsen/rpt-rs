//! Pure data-aggregation helpers for charts and cross-tabs, computed over the [`Dataset`] rather than
//! hosted on the formatter: the per-group / per-category series a chart plots, and the pivot grid a
//! cross-tab draws. Each takes the dataset (plus the render locale / formula registry) explicitly, so
//! the formatter calls them instead of owning the aggregation.

use crate::crosstab;
use crystal_formula::eval::Value;
use crystal_formula::token::{last_segment, strip_braces};
use rpt_data::{compare_values, value_key, Dataset, FormulaRegistry, GroupInstance, Row};
use rpt_format_value::Locale;
use rpt_model::SummaryOperation;

use crate::resolve::{eval_field_ref, eval_field_ref_reported};
use rpt_model::{self as m};

/// One chart series: its name and one aggregated value per category.
type ChartSeries = (String, Vec<f64>);
/// A chart's category-axis labels paired with its series — `(categories, series)`.
type CategorySeries = (Vec<String>, Vec<ChartSeries>);

/// Compute the cross-tab pivot: for each detail row, evaluate the row/column dimension refs and
/// the measure, accumulating the aggregate for each (row, column) cell. A dimension with a decoded
/// date/time grouping [`period`](m::CrossTabDimension::period) buckets its axis by that interval
/// (e.g. a monthly date column collapses to one bucket per calendar month, keyed and ordered by the
/// period-start date and labelled `M/YYYY`) instead of keying on the raw field value; a discrete
/// dimension keeps its first-seen value order.
/// `diag` is the diagnostic sink and the label to report under (the cross-tab object's name). Passing
/// `None` makes a per-cell formula failure silent, which is only appropriate in a unit test.
pub(crate) fn crosstab_pivot(
    dataset: &Dataset,
    formulas: &FormulaRegistry,
    locale: &Locale,
    row_dim: &m::CrossTabDimension,
    col_dim: &m::CrossTabDimension,
    measures: &[m::CrossTabMeasure],
    diag: Option<(&crate::DiagSink, &str)>,
) -> crosstab::Grid {
    use std::collections::HashMap;

    // A cross-tab draws every measure stacked in each data cell (e.g. a `Sum` line over a
    // `DistinctCount` line), so each intersection accumulates one reducer per measure. The reducer
    // is the same one group summaries use, so a cross-tab cell and a group summary of the same
    // field/op agree.
    let nm = measures.len().max(1);
    let new_accs = || vec![rpt_data::SummaryAccumulator::new(); nm];
    let mut row_keys: Vec<(String, Value)> = Vec::new();
    let mut col_keys: Vec<(String, Value)> = Vec::new();
    let mut cells: HashMap<(String, String), Vec<rpt_data::SummaryAccumulator>> = HashMap::new();
    // Grand-total accumulators, kept alongside the per-cell ones so a total re-aggregates the raw
    // rows (correct for Average/DistinctCount, not just Sum) rather than summing formatted cells.
    let mut row_acc: HashMap<String, Vec<rpt_data::SummaryAccumulator>> = HashMap::new();
    let mut col_acc: HashMap<String, Vec<rpt_data::SummaryAccumulator>> = HashMap::new();
    let mut grand_acc = new_accs();
    // Whether an axis buckets a temporal value (drives ascending period ordering below).
    let (mut row_temporal, mut col_temporal) = (false, false);

    // One resolver for every reference this pivot reads, so a failure in a dimension or a measure is
    // reported rather than quietly becoming an empty cell.
    let resolve = |reference: &str, ctx: &rpt_data::DataContext| match diag {
        Some((sink, label)) => eval_field_ref_reported(reference, ctx, sink, label),
        None => eval_field_ref(reference, ctx),
    };
    for row in dataset.iter_detail_rows() {
        let ctx = rpt_data::DataContext::new(row, formulas);
        let rk = rpt_data::date_bucket(resolve(&row_dim.field_ref, &ctx), row_dim.period);
        let ck = rpt_data::date_bucket(resolve(&col_dim.field_ref, &ctx), col_dim.period);
        row_temporal |= is_temporal(&rk);
        col_temporal |= is_temporal(&ck);
        let mvs: Vec<Value> = measures.iter().map(|m| resolve(&m.field, &ctx)).collect();
        let (rks, cks) = (value_key(&rk), value_key(&ck));
        if !row_keys.iter().any(|(k, _)| k == &rks) {
            row_keys.push((rks.clone(), rk));
        }
        if !col_keys.iter().any(|(k, _)| k == &cks) {
            col_keys.push((cks.clone(), ck));
        }
        let fold_all = |accs: &mut Vec<rpt_data::SummaryAccumulator>| {
            for (a, mv) in accs.iter_mut().zip(&mvs) {
                a.fold(mv);
            }
        };
        fold_all(
            cells
                .entry((rks.clone(), cks.clone()))
                .or_insert_with(new_accs),
        );
        fold_all(row_acc.entry(rks).or_insert_with(new_accs));
        fold_all(col_acc.entry(cks).or_insert_with(new_accs));
        fold_all(&mut grand_acc);
    }

    // A temporal (date/time-bucketed) axis is ordered by the period-start instant, matching the
    // engine's ascending calendar order; a discrete axis keeps its first-seen order.
    if row_temporal {
        row_keys.sort_by(|(_, a), (_, b)| compare_values(a, b));
    }
    if col_temporal {
        col_keys.sort_by(|(_, a), (_, b)| compare_values(a, b));
    }

    let nf = locale.number_format();
    let row_period = LabelPeriod::from_group(row_dim.period);
    let col_period = LabelPeriod::from_group(col_dim.period);
    let row_label = |v: &Value| format_period_label(v, locale, row_period);
    let col_label = |v: &Value| format_period_label(v, locale, col_period);
    // Format every measure for one cell, stacked as `\n`-joined lines (drawn as separate lines by the
    // grid renderer) — matching the engine's stacked measure values in each cross-tab cell. Each
    // measure uses its operation's natural format: a `Count`/`DistinctCount` is a whole number, every
    // other operation the locale's grouped 2-decimal default.
    let fmt = move |accs: &[rpt_data::SummaryAccumulator]| -> String {
        accs.iter()
            .zip(measures)
            .map(|(a, meas)| {
                let v = a.value(meas.operation).unwrap_or(Value::Null);
                let mut f = nf.clone();
                if matches!(
                    meas.operation,
                    SummaryOperation::Count | SummaryOperation::DistinctCount
                ) {
                    f.decimals = 0;
                }
                rpt_format_value::format_number(v.as_number().unwrap_or(0.0), &f)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    // An empty (row, column) intersection is each measure over zero rows — the engine draws it as the
    // formatted zero value (e.g. `0.00`), not a blank cell.
    let empty = new_accs();
    let cell_rows = row_keys
        .iter()
        .map(|(rk, _)| {
            col_keys
                .iter()
                .map(|(ck, _)| fmt(cells.get(&(rk.clone(), ck.clone())).unwrap_or(&empty)))
                .collect()
        })
        .collect();
    // Per-row totals (across every column) are drawn as the grand-total column; per-column totals
    // (across every row) as the grand-total row; `grand_total` is the corner of both.
    let row_totals = row_keys
        .iter()
        .map(|(rk, _)| row_acc.get(rk).map(|a| fmt(a)).unwrap_or_default())
        .collect();
    let col_totals = col_keys
        .iter()
        .map(|(ck, _)| col_acc.get(ck).map(|a| fmt(a)).unwrap_or_default())
        .collect();
    let grand_total = fmt(&grand_acc);

    crosstab::Grid {
        corner: String::new(),
        col_headers: col_keys.iter().map(|(_, v)| col_label(v)).collect(),
        row_headers: row_keys.iter().map(|(_, v)| row_label(v)).collect(),
        cells: cell_rows,
        row_totals,
        col_totals,
        grand_total,
    }
}

/// Whether a value is a date/time bucket (so its axis is ordered ascending by instant).
fn is_temporal(v: &Value) -> bool {
    matches!(v, Value::Date(_) | Value::DateTime(..) | Value::Time(_))
}

/// Build the `(category label, value)` series for a group chart. Fast path: one entry per dataset
/// group, the value taken from the group's summary of the charted field (else its first summary).
/// When the report carries no matching group section, fall back to building the chart's own
/// grouping from the detail rows (see [`chart_series_ungrouped`]).
pub(crate) fn chart_series(
    dataset: &Dataset,
    locale: &Locale,
    chart: &m::ChartObject,
) -> Vec<(String, f64)> {
    let field = chart.data_refs.first().map(|r| inner_field(r));
    // The chart's aggregation operation is not surfaced as a declared summary field; it is named
    // in the data-axis title ("Sum of id" / "Count of …") or the data-value label.
    let op = chart_summary_op(&chart.definition.data_axis_title)
        .or_else(|| chart_summary_op(&chart.definition.data_label));
    // Fast path: the chart's category mirrors a report group section, so read one entry per group
    // instance. Kept byte-identical for already-grouped reports.
    let series: Vec<(String, f64)> = dataset
        .groups
        .iter()
        .filter_map(|g| {
            let label = format_category_label(&g.key, locale);
            Some((label, chart_group_value(g, field.as_deref(), op)?))
        })
        .collect();
    if !series.is_empty() {
        return series;
    }
    // Fallback: the chart groups by its own "on change of" field, which need not match any report
    // group (e.g. a chart in the Report Header of an ungrouped report). Build the chart's own
    // category buckets from the detail rows.
    let Some(category_ref) = chart.category_refs.first() else {
        return series;
    };
    let period = chart_label_period(chart);
    chart_series_ungrouped(dataset, locale, category_ref, field.as_deref(), op, period)
}

/// Build the categories and per-series values for a multi-series chart: the categories are the
/// dataset groups, and each data binding is one series carrying its value in every category (the
/// binding's own summary operation over each group's rows). Returns `(categories, series)` where
/// each series is `(name, value-per-category)`.
pub(crate) fn chart_series_multi(
    dataset: &Dataset,
    locale: &Locale,
    chart: &m::ChartObject,
) -> CategorySeries {
    // A chart bound to a SECOND category ("on change of") dimension with a single value field draws
    // one series per distinct secondary value (the series/z axis), each carrying the value per primary
    // category — not one series per value field. Takes precedence over the value-field path below.
    if let Some(res) = chart_series_second_group(dataset, locale, chart) {
        return res;
    }
    let chart_op = chart_summary_op(&chart.definition.data_axis_title)
        .or_else(|| chart_summary_op(&chart.definition.data_label));
    // Fast path: the report groups mirror the chart category. Kept byte-identical when populated.
    if !dataset.groups.is_empty() {
        let categories: Vec<String> = dataset
            .groups
            .iter()
            .map(|g| format_category_label(&g.key, locale))
            .collect();
        let series: Vec<(String, Vec<f64>)> = chart
            .data_refs
            .iter()
            .map(|r| {
                let field = inner_field(r);
                let op = chart_summary_op(r).or(chart_op);
                let vals = dataset
                    .groups
                    .iter()
                    .map(|g| chart_group_value(g, Some(&field), op).unwrap_or(0.0))
                    .collect();
                (field, vals)
            })
            .collect();
        return (categories, series);
    }
    // Fallback: build the chart's own category buckets from the detail rows, one value per data
    // binding (matching the single-series [`chart_series_ungrouped`] path).
    let Some(category_ref) = chart.category_refs.first() else {
        return (Vec::new(), Vec::new());
    };
    let buckets = chart_category_buckets(dataset, locale, category_ref, chart_label_period(chart));
    let categories: Vec<String> = buckets.iter().map(|(label, _)| label.clone()).collect();
    let series: Vec<(String, Vec<f64>)> = chart
        .data_refs
        .iter()
        .map(|r| {
            let field = inner_field(r);
            let op = chart_summary_op(r)
                .or(chart_op)
                .unwrap_or(SummaryOperation::Sum);
            let vals = buckets
                .iter()
                .map(|(_, rows)| {
                    let mut acc = rpt_data::SummaryAccumulator::new();
                    for row in rows {
                        if let Some(v) = row.get(&field) {
                            acc.fold(v);
                        }
                    }
                    acc.value(op)
                        .and_then(|v| v.as_number())
                        .map(finite)
                        .unwrap_or(0.0)
                })
                .collect();
            (field, vals)
        })
        .collect();
    (categories, series)
}

/// Build the categories and per-series values for a chart carrying a **second** category dimension
/// (a primary "on change of" on the category axis and a secondary one on the series/z axis) and a
/// single value field. Returns `None` when the chart has fewer than two category dimensions or more
/// than one value binding — those keep the single-dimension / multiple-value-field paths.
///
/// The primary dimension (`category_refs[0]`) forms the categories (temporal-bucketed like the
/// single-dimension path), the secondary (`category_refs[1]`) forms one series per distinct value,
/// and each cell aggregates the value field over the detail rows matching that (primary, secondary)
/// pair with the chart's summary operation.
fn chart_series_second_group(
    dataset: &Dataset,
    locale: &Locale,
    chart: &m::ChartObject,
) -> Option<CategorySeries> {
    use std::collections::HashMap;
    if chart.category_refs.len() < 2 || chart.data_refs.len() != 1 {
        return None;
    }
    let primary = inner_field(&chart.category_refs[0]);
    let secondary = inner_field(&chart.category_refs[1]);
    let value_field = inner_field(&chart.data_refs[0]);
    let op = chart_summary_op(&chart.data_refs[0])
        .or_else(|| chart_summary_op(&chart.definition.data_axis_title))
        .or_else(|| chart_summary_op(&chart.definition.data_label))
        .unwrap_or(SummaryOperation::Sum);
    let period = chart_label_period(chart);

    // Ordered-distinct primary categories (temporal-bucketed, sorted like `chart_category_buckets`)
    // and ordered-distinct secondary series (first-seen), with a per-(primary, secondary) aggregator.
    let mut prim_order: Vec<String> = Vec::new();
    let mut prim_vals: HashMap<String, Value> = HashMap::new();
    let mut prim_temporal = false;
    let mut sec_order: Vec<String> = Vec::new();
    let mut sec_vals: HashMap<String, Value> = HashMap::new();
    let mut cells: HashMap<(String, String), rpt_data::SummaryAccumulator> = HashMap::new();

    for row in dataset.iter_detail_rows() {
        let (Some(praw), Some(sraw)) = (row.get(&primary), row.get(&secondary)) else {
            continue;
        };
        let pb = rpt_data::date_bucket(praw.clone(), period.group_condition());
        prim_temporal |= matches!(pb, Value::Date(_) | Value::DateTime(..) | Value::Time(_));
        let pk = value_key(&pb);
        if !prim_vals.contains_key(&pk) {
            prim_order.push(pk.clone());
            prim_vals.insert(pk.clone(), pb);
        }
        let sk = value_key(sraw);
        if !sec_vals.contains_key(&sk) {
            sec_order.push(sk.clone());
            sec_vals.insert(sk.clone(), sraw.clone());
        }
        if let Some(v) = row.get(&value_field) {
            cells.entry((pk, sk)).or_default().fold(v);
        }
    }
    if prim_order.is_empty() || sec_order.is_empty() {
        return None;
    }
    if prim_temporal {
        prim_order.sort_by(|a, b| compare_values(&prim_vals[a], &prim_vals[b]));
    }
    let categories: Vec<String> = prim_order
        .iter()
        .map(|k| format_period_label(&prim_vals[k], locale, period))
        .collect();
    let series: Vec<(String, Vec<f64>)> = sec_order
        .iter()
        .map(|sk| {
            let name = format_category_label(&sec_vals[sk], locale);
            let vals = prim_order
                .iter()
                .map(|pk| {
                    cells
                        .get(&(pk.clone(), sk.clone()))
                        .and_then(|a| a.value(op))
                        .and_then(|v| v.as_number())
                        .map(finite)
                        .unwrap_or(0.0)
                })
                .collect();
            (name, vals)
        })
        .collect();
    Some((categories, series))
}

/// Build one [`crosstab`]-free [`crate::chart::StockPoint`] per category: high = the category's
/// maximum of the first value binding, low = the minimum of the second (or first). The OHLC subtype
/// (`graph_subtype == 101`, or four value bindings) additionally carries open/close ticks — from the
/// third/fourth bindings when present, else the low/high ends. Uses the report groups when they
/// mirror the chart category, else the chart's own "on change of" buckets.
pub(crate) fn chart_stock_series(
    dataset: &Dataset,
    locale: &Locale,
    chart: &m::ChartObject,
) -> Vec<crate::chart::StockPoint> {
    let ohlc = chart.definition.graph_subtype == 101 || chart.data_refs.len() >= 4;
    let hi_f = chart.data_refs.first().map(|r| inner_field(r));
    let lo_f = chart
        .data_refs
        .get(1)
        .or_else(|| chart.data_refs.first())
        .map(|r| inner_field(r));
    let open_f = chart.data_refs.get(2).map(|r| inner_field(r));
    let close_f = chart.data_refs.get(3).map(|r| inner_field(r));
    let agg = |field: &Option<String>, rows: &[&Row], op: SummaryOperation| -> Option<f64> {
        let f = field.as_ref()?;
        let mut acc = rpt_data::SummaryAccumulator::new();
        for r in rows {
            if let Some(v) = r.get(f) {
                acc.fold(v);
            }
        }
        acc.value(op).and_then(|v| v.as_number()).map(finite)
    };
    let build = |label: String, rows: &[&Row]| -> Option<crate::chart::StockPoint> {
        let high = agg(&hi_f, rows, SummaryOperation::Maximum)?;
        let low = agg(&lo_f, rows, SummaryOperation::Minimum)?;
        let (open, close) = if ohlc {
            let open = agg(&open_f, rows, SummaryOperation::Minimum).or(Some(low));
            let close = agg(&close_f, rows, SummaryOperation::Maximum).or(Some(high));
            (open, close)
        } else {
            (None, None)
        };
        Some(crate::chart::StockPoint {
            label,
            high,
            low,
            open,
            close,
        })
    };
    // Fast path: the report groups mirror the chart category.
    if !dataset.groups.is_empty() {
        return dataset
            .groups
            .iter()
            .filter_map(|g| {
                let rows = group_rows(g);
                build(format_category_label(&g.key, locale), &rows)
            })
            .collect();
    }
    // Fallback: bucket the detail rows on the chart's own "on change of" category.
    let Some(cref) = chart.category_refs.first() else {
        return Vec::new();
    };
    chart_category_buckets(dataset, locale, cref, chart_label_period(chart))
        .into_iter()
        .filter_map(|(label, rows)| build(label, &rows))
        .collect()
}

/// Build one [`crate::chart::GanttBar`] per detail record from the `start`/`end` date fields,
/// normalizing each span so `start <= end`. Records with neither a datable start nor end are
/// skipped. The row label is the chart's "on change of" category value when bound, else the 1-based
/// record number.
pub(crate) fn chart_gantt_series(
    dataset: &Dataset,
    locale: &Locale,
    chart: &m::ChartObject,
    start: &str,
    end: &str,
) -> Vec<crate::chart::GanttBar> {
    let cat_field = chart.category_refs.first().map(|r| inner_field(r));
    dataset
        .iter_detail_rows()
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let s = r.get(start).and_then(value_to_days);
            let e = r.get(end).and_then(value_to_days);
            // Draw a bar whenever at least one endpoint is datable; a missing endpoint collapses to
            // the other (a zero-width marker at the known instant).
            let (s, e) = match (s, e) {
                (Some(s), Some(e)) => (s.min(e), s.max(e)),
                (Some(s), None) => (s, s),
                (None, Some(e)) => (e, e),
                (None, None) => return None,
            };
            let label = match &cat_field {
                Some(f) => r
                    .get(f)
                    .map(|v| format_category_label(v, locale))
                    .unwrap_or_else(|| (i + 1).to_string()),
                None => (i + 1).to_string(),
            };
            Some(crate::chart::GanttBar {
                label,
                start: s,
                end: e,
            })
        })
        .collect()
}

/// Extract the bare field reference from a chart data binding: `"Sum of {Table.field}"` /
/// `"{Table.field}"` / `"Table.field"` → `"Table.field"`.
pub(crate) fn inner_field(data_ref: &str) -> String {
    let s = data_ref.trim();
    let s = s.split_once(" of ").map(|(_, r)| r).unwrap_or(s);
    strip_braces(s).to_string()
}

/// Whether a summary's field reference matches the wanted field (exact, or same trailing
/// `.`-segment — `"Orders.Amount"` matches `"Amount"`).
fn field_matches(field: &str, want: &str) -> bool {
    let f = strip_braces(field);
    f == want || last_segment(f).eq_ignore_ascii_case(last_segment(want))
}

/// The chart's value for one group: a declared group summary of the charted field if present (so the
/// chart agrees with the section formatter), else the chart's own operation computed over the group's
/// rows, else the group's first summary (legacy fallback).
fn chart_group_value(
    g: &GroupInstance,
    field: Option<&str>,
    op: Option<SummaryOperation>,
) -> Option<f64> {
    if let Some(f) = field {
        if let Some(s) = g.summaries.iter().find(|s| field_matches(&s.field, f)) {
            return s.value.as_number().map(finite);
        }
    }
    if let (Some(f), Some(op)) = (field, op) {
        let mut acc = rpt_data::SummaryAccumulator::new();
        collect_group_values(g, f, &mut acc);
        if let Some(v) = acc.value(op).and_then(|v| v.as_number()) {
            return Some(finite(v));
        }
    }
    g.summaries
        .first()
        .and_then(|s| s.value.as_number())
        .map(finite)
}

/// Sanitize a plotted value: a non-finite (NaN/±Inf) result — e.g. from a divide-by-zero formula or
/// an empty statistical summary — folds to `0.0` so no chart geometry is ever computed from NaN/Inf.
/// A finite value (including extreme magnitudes) passes through unchanged.
fn finite(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

/// Every detail row under `g`, recursing into subgroups — the flat row set a per-category chart
/// (e.g. the stock hi-lo range) aggregates its bound fields over.
fn group_rows(g: &GroupInstance) -> Vec<&Row> {
    let mut out: Vec<&Row> = g.details.iter().collect();
    for sub in &g.subgroups {
        out.extend(group_rows(sub));
    }
    out
}

/// Fold every detail row's `field` value under `g` (recursing into subgroups) into `acc`.
fn collect_group_values(g: &GroupInstance, field: &str, acc: &mut rpt_data::SummaryAccumulator) {
    for row in &g.details {
        if let Some(v) = row.get(field) {
            acc.fold(v);
        }
    }
    for sub in &g.subgroups {
        collect_group_values(sub, field, acc);
    }
}

/// Build the `(category label, value)` series for a chart whose own category is independent of the
/// report grouping: bucket the dataset's detail rows on the chart's "on change of" field and
/// aggregate the data field per bucket with the chart's summary operation (defaulting to `Sum`).
pub(crate) fn chart_series_ungrouped(
    dataset: &Dataset,
    locale: &Locale,
    category_ref: &str,
    field: Option<&str>,
    op: Option<SummaryOperation>,
    period: LabelPeriod,
) -> Vec<(String, f64)> {
    let op = op.unwrap_or(SummaryOperation::Sum);
    chart_category_buckets(dataset, locale, category_ref, period)
        .into_iter()
        .map(|(label, rows)| {
            let mut acc = rpt_data::SummaryAccumulator::new();
            for row in rows {
                match field {
                    // Fold the data field per row for the operation's aggregate.
                    Some(f) => {
                        if let Some(v) = row.get(f) {
                            acc.fold(v);
                        }
                    }
                    // No data binding: count the rows falling in the bucket.
                    None => acc.fold(&Value::Number(1.0)),
                }
            }
            let value = acc
                .value(op)
                .and_then(|v| v.as_number())
                .map(finite)
                .unwrap_or(0.0);
            (label, value)
        })
        .collect()
}

/// Partition a dataset's detail rows into ordered category buckets on `category_ref`, for charts whose
/// "on change of" category is independent of the report grouping. A temporal category buckets by the
/// chart's decoded [`LabelPeriod`] (its [`group_condition`](LabelPeriod::group_condition)); every
/// other category buckets by exact value. Ordered temporally-ascending for a date category, else in
/// first-seen order.
fn chart_category_buckets<'a>(
    dataset: &'a Dataset,
    locale: &Locale,
    category_ref: &str,
    period: LabelPeriod,
) -> Vec<(String, Vec<&'a Row>)> {
    use std::collections::HashMap;
    let cat = inner_field(category_ref);
    let mut order: Vec<String> = Vec::new();
    let mut buckets: HashMap<String, (Value, Vec<&Row>)> = HashMap::new();
    let mut temporal = false;
    for row in dataset.iter_detail_rows() {
        let Some(raw) = row.get(&cat) else { continue };
        let bucket = rpt_data::date_bucket(raw.clone(), period.group_condition());
        temporal |= matches!(
            bucket,
            Value::Date(_) | Value::DateTime(..) | Value::Time(_)
        );
        let key = value_key(&bucket);
        if !buckets.contains_key(&key) {
            order.push(key.clone());
            buckets.insert(key.clone(), (bucket, Vec::new()));
        }
        buckets
            .get_mut(&key)
            .expect("bucket just inserted")
            .1
            .push(row);
    }
    if temporal {
        order.sort_by(|a, b| compare_values(&buckets[a].0, &buckets[b].0));
    }
    order
        .into_iter()
        .filter_map(|k| buckets.remove(&k))
        .map(|(bucket, rows)| (format_period_label(&bucket, locale, period), rows))
        .collect()
}

/// The engine's compact (no-leading-zero) date style for a temporal category-axis label, keyed per
/// period by [`LabelPeriod::date_style`]. Distinct from a field's system short-date default, which
/// zero-pads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DateLabelStyle {
    /// `YYYY` — a year-granular bucket (annual).
    Year,
    /// `M/YYYY` (e.g. "1/2024") — a month-granular bucket (monthly; quarterly/semi-annually roll to
    /// their start month).
    MonthYear,
    /// `M/d/YYYY` (e.g. "1/7/2024") — a day-granular bucket (daily/weekly; semi-monthly).
    MonthDayYear,
}

impl DateLabelStyle {
    fn render(self, year: i32, month: u8, day: u8) -> String {
        match self {
            Self::Year => format!("{year}"),
            Self::MonthYear => format!("{month}/{year}"),
            Self::MonthDayYear => format!("{month}/{day}/{year}"),
        }
    }
}

/// The period a temporal category axis buckets and labels at — the render-side vocabulary shared by
/// the chart path (from a decoded [`m::ChartCategoryPeriod`]) and the cross-tab path (from a
/// [`m::GroupCondition`]). Matched exhaustively so every period has an explicit, intentional bucket
/// grain and label style rather than a string catch-all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LabelPeriod {
    Daily,
    Weekly,
    /// Bucketed on fortnight-aligned two-week boundaries ([`m::GroupCondition::BiWeekly`]) and
    /// labelled by day. A genuinely biweekly category axis is decoded and honoured as such.
    Biweekly,
    Semimonthly,
    Monthly,
    Quarterly,
    SemiAnnually,
    Annually,
    /// A non-date grouping (a boolean/time report group, or a discrete cross-tab dimension) — labelled
    /// through the locale default, never a date style.
    NonDate,
}

impl LabelPeriod {
    /// The chart category period from the decoded [`m::ChartCategoryPeriod`]; an undecoded period
    /// defaults to monthly (the engine's most common date grouping).
    fn from_chart(period: Option<m::ChartCategoryPeriod>) -> Self {
        use m::ChartCategoryPeriod as P;
        match period {
            Some(P::Daily) => Self::Daily,
            Some(P::Weekly) => Self::Weekly,
            Some(P::Biweekly) => Self::Biweekly,
            Some(P::Semimonthly) => Self::Semimonthly,
            Some(P::Monthly) => Self::Monthly,
            Some(P::Quarterly) => Self::Quarterly,
            Some(P::SemiAnnually) => Self::SemiAnnually,
            Some(P::Annually) => Self::Annually,
            None => Self::Monthly,
        }
    }

    /// The label period for a cross-tab dimension's grouping condition; a non-date (boolean/time)
    /// condition or an ungrouped discrete dimension is [`NonDate`](Self::NonDate).
    fn from_group(cond: Option<m::GroupCondition>) -> Self {
        use m::GroupCondition as G;
        match cond {
            Some(G::Daily) => Self::Daily,
            Some(G::Weekly) => Self::Weekly,
            Some(G::BiWeekly) => Self::Biweekly,
            Some(G::SemiMonthly) => Self::Semimonthly,
            Some(G::Monthly) => Self::Monthly,
            Some(G::Quarterly) => Self::Quarterly,
            Some(G::SemiAnnually) => Self::SemiAnnually,
            Some(G::Annually) => Self::Annually,
            _ => Self::NonDate,
        }
    }

    /// The bucketing condition the date bucketer groups rows by. `None` leaves values un-bucketed (a
    /// non-date period).
    fn group_condition(self) -> Option<m::GroupCondition> {
        use m::GroupCondition as G;
        Some(match self {
            Self::Daily => G::Daily,
            Self::Weekly => G::Weekly,
            Self::Biweekly => G::BiWeekly,
            Self::Semimonthly => G::SemiMonthly,
            Self::Monthly => G::Monthly,
            Self::Quarterly => G::Quarterly,
            Self::SemiAnnually => G::SemiAnnually,
            Self::Annually => G::Annually,
            Self::NonDate => return None,
        })
    }

    /// The date label style for this period, or `None` for a non-date period (labelled through the
    /// locale default). Monthly is `M/YYYY` and weekly `M/d/YYYY`; the quarterly/semi-annually/
    /// semi-monthly/daily styles follow each period's natural grain and are provisional.
    fn date_style(self) -> Option<DateLabelStyle> {
        Some(match self {
            Self::Daily | Self::Weekly | Self::Biweekly | Self::Semimonthly => {
                DateLabelStyle::MonthDayYear
            }
            Self::Monthly | Self::Quarterly | Self::SemiAnnually => DateLabelStyle::MonthYear,
            Self::Annually => DateLabelStyle::Year,
            Self::NonDate => return None,
        })
    }
}

/// The chart's category [`LabelPeriod`], resolved from the decoded [`m::ChartCategoryPeriod`].
fn chart_label_period(chart: &m::ChartObject) -> LabelPeriod {
    LabelPeriod::from_chart(chart.definition.category_period)
}

/// Format a temporal category label for the bucketing `period` in the engine's compact
/// (no-leading-zero) category-axis date style ([`DateLabelStyle`]). Non-temporal values, and a
/// non-date period, format through the locale default.
fn format_period_label(bucket: &Value, locale: &Locale, period: LabelPeriod) -> String {
    let (year, month, day) = match bucket {
        Value::Date(d) => (d.year, d.month, d.day),
        Value::DateTime(d, _) => (d.year, d.month, d.day),
        other => return crate::format::render_value_default(other, locale),
    };
    match period.date_style() {
        Some(style) => style.render(year, month, day),
        None => crate::format::render_value_default(bucket, locale),
    }
}

/// Format a report-group category label. A date on the first of the month — the signature of a
/// monthly report date group — reads as the engine's `M/YYYY` ([`DateLabelStyle::MonthYear`], no
/// leading zeros, e.g. "1/2024") rather than a full localized date, so the category-axis and legend
/// labels match Crystal's. A finer-grained date (not the 1st) and every non-temporal value format
/// through the locale default.
///
/// This infers the month grain from a 1st-of-month key because a [`GroupInstance`] carries only the
/// bucketed key, not the group's decoded period. The chart-owned period path uses
/// [`format_period_label`], which keys off the decoded [`LabelPeriod`] instead of this heuristic.
pub(crate) fn format_category_label(bucket: &Value, locale: &Locale) -> String {
    match bucket {
        Value::Date(d) if d.day == 1 => DateLabelStyle::MonthYear.render(d.year, d.month, d.day),
        Value::DateTime(d, _) if d.day == 1 => {
            DateLabelStyle::MonthYear.render(d.year, d.month, d.day)
        }
        other => crate::format::render_value_default(other, locale),
    }
}

/// A temporal value as a civil day-number (the Gantt date axis's unit): a `Date` is its day count, a
/// `DateTime` adds the time-of-day as a day fraction. `None` for a non-temporal value, so a record
/// with no datable endpoint is skipped.
fn value_to_days(v: &Value) -> Option<f64> {
    match v {
        Value::Date(d) => Some(d.to_days() as f64),
        Value::DateTime(d, t) => Some(d.to_days() as f64 + t.to_seconds() as f64 / 86_400.0),
        _ => None,
    }
}

/// Parse a chart value binding's leading operation ("Sum of id", "Count of {t.f}", "Distinct Count
/// of …") into a [`SummaryOperation`]. `None` when it carries no "<op> of …" prefix.
pub(crate) fn chart_summary_op(binding: &str) -> Option<SummaryOperation> {
    let (op, _) = binding.trim().split_once(" of ")?;
    Some(match op.trim().to_ascii_lowercase().as_str() {
        "sum" => SummaryOperation::Sum,
        "count" => SummaryOperation::Count,
        "distinct count" => SummaryOperation::DistinctCount,
        "average" | "avg" => SummaryOperation::Average,
        "maximum" | "max" => SummaryOperation::Maximum,
        "minimum" | "min" => SummaryOperation::Minimum,
        "median" => SummaryOperation::Median,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_data::{Dataset, FormulaRegistry, Row};
    use rpt_format_value::{Date, Locale};

    /// Every [`LabelPeriod`] maps to an explicit, intentional date-label style — the enum-driven
    /// replacement for the old monthly-vs-everything string catch-all. Monthly (`M/YYYY`) and weekly
    /// (`M/d/YYYY`) are established; annually reads year-only; biweekly folds to the weekly style
    /// and weekly bucket condition; the quarterly/semi/daily styles are the documented placeholders.
    #[test]
    fn period_label_styles_are_exhaustive_and_intentional() {
        use rpt_model::GroupCondition as G;
        let locale = Locale::default();
        // A mid-month day so the day-granular styles differ visibly from the month-granular ones.
        let d = Value::Date(Date::new(2024, 3, 7));
        let label = |p: LabelPeriod| format_period_label(&d, &locale, p);

        // Day- and month-grain styles.
        assert_eq!(label(LabelPeriod::Monthly), "3/2024");
        assert_eq!(label(LabelPeriod::Weekly), "3/7/2024");
        // Intentional (documented placeholder) styles.
        assert_eq!(label(LabelPeriod::Annually), "2024");
        assert_eq!(label(LabelPeriod::Quarterly), "3/2024");
        assert_eq!(label(LabelPeriod::SemiAnnually), "3/2024");
        assert_eq!(label(LabelPeriod::Daily), "3/7/2024");
        assert_eq!(label(LabelPeriod::Semimonthly), "3/7/2024");
        // Biweekly is labelled by day and bucketed on fortnight-aligned two-week boundaries.
        assert_eq!(label(LabelPeriod::Biweekly), "3/7/2024");
        assert_eq!(LabelPeriod::Biweekly.group_condition(), Some(G::BiWeekly));

        // A non-date period (e.g. a boolean cross-tab dimension) falls back to the locale default,
        // never a date style.
        assert!(LabelPeriod::from_group(Some(G::EveryYes))
            .date_style()
            .is_none());
        assert_eq!(LabelPeriod::from_group(None), LabelPeriod::NonDate);

        // An undecoded chart period defaults to monthly.
        assert_eq!(LabelPeriod::from_chart(None), LabelPeriod::Monthly);
        assert_eq!(
            LabelPeriod::from_chart(Some(rpt_model::ChartCategoryPeriod::Annually)),
            LabelPeriod::Annually
        );
    }

    /// Build a flat (ungrouped) dataset from `(cat, "YYYY-MM-DD", amount)` triples.
    fn dataset(rows: &[(&str, (i32, u8, u8), f64)]) -> Dataset {
        let details: Vec<Row> = rows
            .iter()
            .map(|(cat, (y, m, d), amt)| {
                let mut r = Row::default();
                r.insert("cat", Value::Str((*cat).to_string()));
                r.insert("t.d", Value::Date(Date::new(*y, *m, *d)));
                r.insert("amt", Value::Number(*amt));
                r
            })
            .collect();
        Dataset {
            columns: Vec::new(),
            row_count: details.len(),
            groups: Vec::new(),
            details,
            grand_total: Vec::new(),
            params: Default::default(),
        }
    }

    fn dim(field: &str, period: Option<m::GroupCondition>) -> m::CrossTabDimension {
        m::CrossTabDimension {
            field_ref: field.to_string(),
            period,
            ..Default::default()
        }
    }

    /// A monthly-period date column dimension buckets the raw dates into one column per calendar
    /// month (ascending, labelled `M/YYYY`) rather than one per distinct date, and the cells /
    /// totals re-aggregate over the buckets. An empty intersection reads as the formatted zero.
    #[test]
    fn crosstab_pivot_buckets_date_column_by_month() {
        // Dates span two months, fed out of order; category B has no February row.
        let ds = dataset(&[
            ("A", (2024, 2, 5), 100.0),
            ("A", (2024, 1, 10), 5.0),
            ("A", (2024, 1, 20), 15.0),
            ("B", (2024, 1, 15), 50.0),
        ]);
        let measure = m::CrossTabMeasure {
            operation: m::SummaryOperation::Sum,
            field: "amt".to_string(),
        };
        let grid = crosstab_pivot(
            &ds,
            &FormulaRegistry::new(),
            &Locale::default(),
            &dim("cat", None),
            &dim("t.d", Some(m::GroupCondition::Monthly)),
            &[measure],
            None,
        );
        // Two monthly columns, ascending, labelled M/YYYY (not one column per distinct date).
        assert_eq!(grid.col_headers, vec!["1/2024", "2/2024"]);
        assert_eq!(grid.row_headers, vec!["A", "B"]);
        // A: Jan 5+15=20, Feb 100. B: Jan 50, Feb empty → 0.00.
        assert_eq!(grid.cells[0], vec!["20.00", "100.00"]);
        assert_eq!(grid.cells[1], vec!["50.00", "0.00"]);
        assert_eq!(grid.row_totals, vec!["120.00", "50.00"]);
        assert_eq!(grid.col_totals, vec!["70.00", "100.00"]);
        assert_eq!(grid.grand_total, "170.00");
    }

    /// Without a decoded period, a date column dimension keys on the raw value — the pre-fix
    /// behaviour that exploded the grid to one column per distinct date. The guard: four distinct
    /// dates yield four columns, versus two when bucketed monthly (the test above).
    #[test]
    fn crosstab_pivot_no_period_keeps_raw_date_columns() {
        let ds = dataset(&[
            ("A", (2024, 2, 5), 100.0),
            ("A", (2024, 1, 10), 5.0),
            ("A", (2024, 1, 20), 15.0),
            ("B", (2024, 1, 15), 50.0),
        ]);
        let measure = m::CrossTabMeasure {
            operation: m::SummaryOperation::Sum,
            field: "amt".to_string(),
        };
        let grid = crosstab_pivot(
            &ds,
            &FormulaRegistry::new(),
            &Locale::default(),
            &dim("cat", None),
            &dim("t.d", None),
            &[measure],
            None,
        );
        assert_eq!(grid.col_headers.len(), 4, "raw dates are not bucketed");
    }

    /// A cross-tab with two measures stacks both formatted values (`\n`-joined) in every cell and
    /// total, each with its operation's natural format: `Sum` grouped-2dp, `DistinctCount` a whole
    /// number.
    #[test]
    fn crosstab_pivot_stacks_multiple_measures() {
        // Two categories × one column; ids 1,2 in A (2 distinct), id 3 twice in B (1 distinct).
        let details: Vec<Row> = [
            ("A", 1.0, 10.0),
            ("A", 2.0, 20.0),
            ("B", 3.0, 5.0),
            ("B", 3.0, 15.0),
        ]
        .iter()
        .map(|(cat, id, amt)| {
            let mut r = Row::default();
            r.insert("cat", Value::Str((*cat).to_string()));
            r.insert("col", Value::Str("X".to_string()));
            r.insert("id", Value::Number(*id));
            r.insert("amt", Value::Number(*amt));
            r
        })
        .collect();
        let ds = Dataset {
            columns: Vec::new(),
            row_count: details.len(),
            groups: Vec::new(),
            details,
            grand_total: Vec::new(),
            params: Default::default(),
        };
        let measures = [
            m::CrossTabMeasure {
                operation: m::SummaryOperation::Sum,
                field: "amt".to_string(),
            },
            m::CrossTabMeasure {
                operation: m::SummaryOperation::DistinctCount,
                field: "id".to_string(),
            },
        ];
        let grid = crosstab_pivot(
            &ds,
            &FormulaRegistry::new(),
            &Locale::default(),
            &dim("cat", None),
            &dim("col", None),
            &measures,
            None,
        );
        // Each cell stacks Sum (2dp) over DistinctCount (whole number).
        assert_eq!(grid.cells[0], vec!["30.00\n2"], "A: sum 30, 2 distinct ids");
        assert_eq!(grid.cells[1], vec!["20.00\n1"], "B: sum 20, 1 distinct id");
        // Grand total re-aggregates the raw rows: sum 50, 3 distinct ids (1,2,3).
        assert_eq!(grid.grand_total, "50.00\n3");
    }
}
