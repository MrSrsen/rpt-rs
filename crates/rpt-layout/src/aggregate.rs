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

use crate::resolve::eval_field_ref;
use rpt_model::{self as m};

/// One chart series: its name and one aggregated value per category.
type ChartSeries = (String, Vec<f64>);
/// A chart's category-axis labels paired with its series — `(categories, series)`.
type CategorySeries = (Vec<String>, Vec<ChartSeries>);

/// Compute the cross-tab pivot: for each detail row, evaluate the row/column dimension refs and
/// the measure, accumulating the aggregate for each (row, column) cell.
pub(crate) fn crosstab_pivot(
    dataset: &Dataset,
    formulas: &FormulaRegistry,
    locale: &Locale,
    row_field: &str,
    col_field: &str,
    measure: &m::CrossTabMeasure,
) -> crosstab::Grid {
    use std::collections::HashMap;

    // Ordered-distinct dimension values (first-seen), and a per-cell aggregator. The cell uses the
    // same shared reducer as group summaries, so a cross-tab cell and a group summary of the same
    // field/op agree.
    let mut row_keys: Vec<(String, Value)> = Vec::new();
    let mut col_keys: Vec<(String, Value)> = Vec::new();
    let mut cells: HashMap<(String, String), rpt_data::SummaryAccumulator> = HashMap::new();

    for row in dataset.iter_detail_rows() {
        let ctx = rpt_data::DataContext::new(row, formulas);
        let rk = eval_field_ref(row_field, &ctx);
        let ck = eval_field_ref(col_field, &ctx);
        let mv = eval_field_ref(&measure.field, &ctx);
        let (rks, cks) = (value_key(&rk), value_key(&ck));
        if !row_keys.iter().any(|(k, _)| k == &rks) {
            row_keys.push((rks.clone(), rk));
        }
        if !col_keys.iter().any(|(k, _)| k == &cks) {
            col_keys.push((cks.clone(), ck));
        }
        cells.entry((rks, cks)).or_default().fold(&mv);
    }

    let nf = locale.number_format();
    let label = |v: &Value| crate::format::render_value_default(v, locale);
    let cell_rows = row_keys
        .iter()
        .map(|(rk, _)| {
            col_keys
                .iter()
                .map(|(ck, _)| {
                    cells
                        .get(&(rk.clone(), ck.clone()))
                        .map(|a| {
                            let v = a.value(measure.operation).unwrap_or(Value::Null);
                            rpt_format_value::format_number(v.as_number().unwrap_or(0.0), &nf)
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .collect();

    crosstab::Grid {
        corner: String::new(),
        col_headers: col_keys.iter().map(|(_, v)| label(v)).collect(),
        row_headers: row_keys.iter().map(|(_, v)| label(v)).collect(),
        cells: cell_rows,
    }
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
    let period = chart_period_token(chart);
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
    let buckets = chart_category_buckets(dataset, locale, category_ref, chart_period_token(chart));
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
                    acc.value(op).and_then(|v| v.as_number()).unwrap_or(0.0)
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
    let period = chart_period_token(chart);

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
        let pb = rpt_data::date_bucket(praw.clone(), Some(period));
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
        acc.value(op).and_then(|v| v.as_number())
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
    chart_category_buckets(dataset, locale, cref, chart_period_token(chart))
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
    let f = field.trim_matches(['{', '}']);
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
            return s.value.as_number();
        }
    }
    if let (Some(f), Some(op)) = (field, op) {
        let mut acc = rpt_data::SummaryAccumulator::new();
        collect_group_values(g, f, &mut acc);
        if let Some(v) = acc.value(op).and_then(|v| v.as_number()) {
            return Some(v);
        }
    }
    g.summaries.first().and_then(|s| s.value.as_number())
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
    period: &str,
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
            let value = acc.value(op).and_then(|v| v.as_number()).unwrap_or(0.0);
            (label, value)
        })
        .collect()
}

/// Partition a dataset's detail rows into ordered category buckets on `category_ref`, for charts whose
/// "on change of" category is independent of the report grouping. A temporal category buckets by the
/// chart's decoded `period` (`"weekly"`/`"monthly"`/…, the same vocabulary as a report group's
/// `date_condition`); every other category buckets by exact value. Ordered temporally-ascending for a
/// date category, else in first-seen order.
fn chart_category_buckets<'a>(
    dataset: &'a Dataset,
    locale: &Locale,
    category_ref: &str,
    period: &str,
) -> Vec<(String, Vec<&'a Row>)> {
    use std::collections::HashMap;
    let cat = inner_field(category_ref);
    let mut order: Vec<String> = Vec::new();
    let mut buckets: HashMap<String, (Value, Vec<&Row>)> = HashMap::new();
    let mut temporal = false;
    for row in dataset.iter_detail_rows() {
        let Some(raw) = row.get(&cat) else { continue };
        let bucket = rpt_data::date_bucket(raw.clone(), Some(period));
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

/// The chart's category-bucketing period token (the same vocabulary as a report group's
/// `date_condition`), resolved from the decoded [`m::ChartCategoryPeriod`]. An undecoded period
/// defaults to monthly. Biweekly is not distinguishable from weekly in the stored data, so it is
/// treated as weekly rather than approximated.
fn chart_period_token(chart: &m::ChartObject) -> &'static str {
    match chart.definition.category_period {
        Some(m::ChartCategoryPeriod::Biweekly) => "weekly",
        Some(p) => p.as_token(),
        None => "monthly",
    }
}

/// Format a chart category label for the bucketing `period`, matching the engine's compact
/// (no-leading-zero) category-axis date style: a monthly bucket reads as `M/YYYY` (e.g. "1/2024"),
/// every other date period as `M/d/YYYY` (e.g. a weekly bucket's week-start "1/7/2024"). This is
/// distinct from a field's system short-date default, which zero-pads. Non-temporal values format
/// through the locale default.
fn format_period_label(bucket: &Value, locale: &Locale, period: &str) -> String {
    let (year, month, day) = match bucket {
        Value::Date(d) => (d.year, d.month, d.day),
        Value::DateTime(d, _) => (d.year, d.month, d.day),
        other => return crate::format::render_value_default(other, locale),
    };
    if period == "monthly" {
        format!("{month}/{year}")
    } else {
        format!("{month}/{day}/{year}")
    }
}

/// Format a report-group category label. A monthly bucket — a date on the first of the month, the
/// signature of a monthly report date group — reads as the engine's `M/YYYY` (no leading zeros, e.g.
/// "1/2024") rather than a full localized date, so the category-axis and legend labels match
/// Crystal's. A finer-grained date (not the 1st) and every non-temporal value format through the
/// locale default. (Chart-owned period buckets use [`format_period_label`] instead, which keys off
/// the decoded period rather than the day-of-month heuristic.)
pub(crate) fn format_category_label(bucket: &Value, locale: &Locale) -> String {
    match bucket {
        Value::Date(d) if d.day == 1 => format!("{}/{}", d.month, d.year),
        Value::DateTime(d, _) if d.day == 1 => format!("{}/{}", d.month, d.year),
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
