//! Resolve a report object's bound value(s) to display strings, in a given record/group context.
//!
//! Field objects carry a `data_source` reference and a [`FieldRefKind`]; this turns that into a
//! [`Value`] (via the formula evaluator) and then a formatted string. Text objects with embedded
//! `{…}` references get per-reference substitution. The display format is resolved by
//! [`crate::format`], merging the render locale with the field's stored `FieldFormat` leaf.

use crate::format::{field_format_spec, render_value, render_value_default};
use crate::{push_diag, DiagSink};
use crystal_formula::eval::{EvalContext, EvalError, Value};
use crystal_formula::token::short_name;
use crystal_formula::{parse, RefKind, Syntax};
use rpt_data::{
    DataContext, FormulaRegistry, Row, RunningTotals, ScheduledValues, SharedState, Summary,
};
use rpt_format_value::Locale;
use rpt_model::{Color, FieldObject, FieldRefKind, TextObject};
use rpt_pages::{Diagnostic, DiagnosticKind};
use std::rc::Rc;

/// The group/summary state a resolver needs beyond the current row.
#[derive(Debug, Clone, Default)]
pub struct ResolveState {
    /// The nearest enclosing group's key (for `GroupName` fields).
    pub group_key: Option<Value>,
    /// Summaries in scope (the nearest group's, else the grand total) for summary-field lookup.
    pub summaries: Vec<Summary>,
    /// Each enclosing group's `(condition field, summaries)`, outermost first. A
    /// **group-scoped** (2-argument) summary `Op ({field}, {group condition field})` resolves against
    /// the summaries of the group whose condition field is the 2nd argument, rather than the nearest.
    ///
    /// Shared (`Rc`) rather than owned so the formatter builds this projection once per group-stack
    /// change and every per-record [`ResolveState`] is a cheap refcount bump, not a deep clone of
    /// every enclosing group's summary vec.
    pub group_summaries: Rc<Vec<(String, Vec<Summary>)>>,
    /// Print-state specials for the current position (page number, record number, …).
    pub page_number: i64,
    pub total_pages: i64,
    pub record_number: i64,
}

/// Build a [`DataContext`] for `row` carrying the standard specials from `state`, the report
/// parameter values (`{?Name}` resolution), the print-order running totals (`{#name}`),
/// and any pre-scheduled formula values for this record.
#[allow(clippy::too_many_arguments)]
pub fn context<'a>(
    row: &'a Row,
    formulas: &'a FormulaRegistry,
    params: &'a rpt_data::Parameters,
    state: &ResolveState,
    state_vars: &'a SharedState,
    running: &'a RunningTotals,
    scheduled: &'a ScheduledValues,
) -> DataContext<'a> {
    let scheduled_row = row.read_index().and_then(|i| scheduled.record(i));
    let before = (!scheduled.before.is_empty()).then_some(&scheduled.before);
    DataContext::new(row, formulas)
        .with_params(params)
        .with_state(state_vars)
        .with_running_totals(running)
        .with_scheduled(before, scheduled_row)
        .with_special("recordnumber", Value::Number(state.record_number as f64))
        .with_special("pagenumber", Value::Number(state.page_number as f64))
        .with_special("totalpagecount", Value::Number(state.total_pages as f64))
}

/// Resolve a field object to a [`Value`] in the given context, recording any runtime formula error
/// into `diag`.
pub fn field_value(
    obj: &FieldObject,
    ctx: &DataContext,
    state: &ResolveState,
    diag: &DiagSink,
) -> Value {
    match obj.ref_kind {
        FieldRefKind::DatabaseField => {
            eval_ref(&brace(&obj.data_source), ctx, diag, &obj.data_source)
        }
        FieldRefKind::Formula => {
            let name = obj
                .data_source
                .trim_start_matches(['{', '@'])
                .trim_end_matches('}');
            ctx.resolve(RefKind::Formula, name).unwrap_or(Value::Null)
        }
        FieldRefKind::GroupName => state.group_key.clone().unwrap_or(Value::Null),
        FieldRefKind::Summary => summary_value(&obj.data_source, state),
        FieldRefKind::Special => special_value(&obj.data_source, state),
        // A running total (`{#name}`) resolves to the print-order value accumulated up to the current
        // record; the layout advances it per record before the band is emitted.
        FieldRefKind::RunningTotal => {
            let name = obj.data_source.trim().trim_matches(['{', '#', '}']);
            ctx.resolve(RefKind::RunningTotal, name)
                .unwrap_or(Value::Null)
        }
        // Parameter / SQL-expression resolution lands with their owning layers.
        _ => eval_ref(&brace(&obj.data_source), ctx, diag, &obj.data_source),
    }
}

/// The formatted display string for a field object: the field's declared value type + stored
/// [`rpt_model::FieldFormat`] leaf are merged with the render `locale` to pick the effective format
/// (integer types → 0 decimals; explicit stored decimals/negative/currency/date-forms win over the
/// locale defaults; names/separators always come from the locale — see [`crate::format`]).
pub fn field_text(
    obj: &FieldObject,
    ctx: &DataContext,
    state: &ResolveState,
    loc: &Locale,
    diag: &DiagSink,
) -> String {
    let value = field_value(obj, ctx, state, diag);
    let spec = field_format_spec(obj.format.as_ref(), obj.value_type, loc);
    render_value(&value, &spec, loc)
}

/// Render a text object: its full literal content (the `display` string, which keeps every run —
/// e.g. a two-line `"Numeric\nCode"` label — unlike `text`, which holds only the last run), with any
/// embedded `{ref}` substituted by its resolved value. Works **without** a row (`ctx = None`): static
/// labels in page/report headers and footers have no data row, so we must still use `display` there
/// rather than falling back to the last-run-only `text`.
pub fn text_display(
    obj: &TextObject,
    ctx: Option<&DataContext>,
    state: &ResolveState,
    loc: &Locale,
    diag: &DiagSink,
) -> String {
    let src = if obj.display.is_empty() {
        &obj.text
    } else {
        &obj.display
    };
    match ctx {
        Some(c) if !obj.embedded_fields.is_empty() && src.contains('{') => {
            substitute_braces(src, c, state, loc, diag)
        }
        _ => src.clone(),
    }
}

/// Evaluate a conditional-format formula body (e.g. a border's `BackgroundColor` formula) in the
/// current record context, decoding the resulting Crystal COLORREF number to a [`Color`]. Returns
/// `None` when there is no context, no such formula, or the formula yields `crNoColor` (`-1`).
pub fn cond_color(
    conditions: &[(String, String)],
    key: &str,
    ctx: Option<&DataContext>,
) -> Option<Color> {
    let ctx = ctx?;
    let body = conditions.iter().find(|(k, _)| k == key).map(|(_, b)| b)?;
    let (ast, _) = parse(body, Syntax::Crystal);
    let value = crystal_formula::eval::eval(&ast, ctx).ok()?;
    color_from_colorref(&value)
}

/// Evaluate a named conditional-format formula to a boolean (e.g. an object's `EnableSuppress`).
/// `None` when there is no context, no such formula, or it does not yield a `Bool`.
pub fn cond_bool(
    conditions: &[(String, String)],
    key: &str,
    ctx: Option<&DataContext>,
) -> Option<bool> {
    let ctx = ctx?;
    let body = conditions.iter().find(|(k, _)| k == key).map(|(_, b)| b)?;
    let (ast, _) = parse(body, Syntax::Crystal);
    match crystal_formula::eval::eval(&ast, ctx).ok()? {
        Value::Bool(b) => Some(b),
        _ => None,
    }
}

/// Decode a Crystal COLORREF number (`r + g·256 + b·65536`) to an opaque [`Color`]; `None` for a
/// negative value (`crNoColor`).
fn color_from_colorref(value: &Value) -> Option<Color> {
    let n = value.as_number()? as i64;
    if n < 0 {
        return None;
    }
    Some(Color {
        a: 255,
        r: (n & 0xFF) as u8,
        g: ((n >> 8) & 0xFF) as u8,
        b: ((n >> 16) & 0xFF) as u8,
    })
}

/// Evaluate a brace-wrapped reference expression (`{table.field}`) to a Value, recording any runtime
/// error into `diag` under `label` (deduped) and yielding `Null`.
fn eval_ref(expr: &str, ctx: &DataContext, diag: &DiagSink, label: &str) -> Value {
    let (ast, _) = parse(expr, Syntax::Crystal);
    match crystal_formula::eval::eval(&ast, ctx) {
        Ok(v) => v,
        Err(e) => {
            record_eval_error(diag, label, &e);
            Value::Null
        }
    }
}

/// Record a formula evaluation error as a diagnostic, distinguishing an unimplemented builtin/feature
/// ([`EvalError::Unsupported`]) from an ordinary runtime error.
fn record_eval_error(diag: &DiagSink, label: &str, err: &EvalError) {
    let (kind, msg) = match err {
        EvalError::Unsupported(what) => (
            DiagnosticKind::UnsupportedFormula,
            format!("unsupported in formula: {what}"),
        ),
        e => (DiagnosticKind::FormulaError, format!("formula error: {e}")),
    };
    push_diag(diag, Diagnostic::warn(kind, msg).with_source(label));
}

/// Evaluate a bare field/formula reference (`Table.field` or `@formula`) to a [`Value`] in `ctx`,
/// with no diagnostics — used by the cross-tab pivot.
pub(crate) fn eval_field_ref(reference: &str, ctx: &DataContext) -> Value {
    let (ast, _) = parse(&brace(reference), Syntax::Crystal);
    crystal_formula::eval::eval(&ast, ctx).unwrap_or(Value::Null)
}

/// Ensure a reference is brace-wrapped for parsing (`table.field` → `{table.field}`).
fn brace(reference: &str) -> String {
    let r = reference.trim();
    if r.starts_with('{') {
        r.to_string()
    } else {
        format!("{{{r}}}")
    }
}

/// Look up a summary field's value from the in-scope summaries by its summarized field name.
///
/// The data source is `Op ({summarized})` (report-level, grand total) or, for a **group-scoped**
/// summary, `Op ({summarized}, {group condition field})`. Only an index
/// is stored on the object; the group is recovered from context — the 2nd argument names the group's
/// condition field, so we resolve against **that group's** computed summaries (from
/// [`ResolveState::group_summaries`]) rather than the nearest group's. A 1-argument summary, or a 2nd
/// argument that matches no enclosing group, falls back to the nearest in-scope summaries.
fn summary_value(data_source: &str, state: &ResolveState) -> Value {
    let (field_arg, group_arg) = parse_summary_args(data_source);
    // Pick the summaries to search: the group whose condition field is the 2nd argument, else the
    // nearest in-scope summaries.
    let summaries = group_arg
        .as_deref()
        .and_then(|g| {
            state
                .group_summaries
                .iter()
                .find(|(cond, _)| short_name(cond) == short_name(g))
                .map(|(_, s)| s.as_slice())
        })
        .unwrap_or(state.summaries.as_slice());
    summaries
        .iter()
        .find(|s| {
            let field = s.field.trim_matches(['{', '}']);
            field == field_arg || short_name(field) == short_name(&field_arg)
        })
        .map(|s| s.value.clone())
        .unwrap_or(Value::Null)
}

/// Split a summary data source `Op ({arg0}[, {arg1}])` into its summarized-field argument and the
/// optional group-condition-field argument (both brace-stripped). Splits on the top-level comma
/// inside the outer parentheses so a `table.field` name is never mistaken for the separator.
fn parse_summary_args(data_source: &str) -> (String, Option<String>) {
    let inner = data_source
        .split_once('(')
        .and_then(|(_, rest)| rest.rsplit_once(')').map(|(f, _)| f))
        .unwrap_or(data_source);
    // Find the top-level comma (brace depth 0).
    let mut depth = 0i32;
    let mut split_at = None;
    for (i, ch) in inner.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                split_at = Some(i);
                break;
            }
            _ => {}
        }
    }
    let clean = |s: &str| s.trim().trim_matches(['{', '}']).trim().to_string();
    match split_at {
        Some(i) => (clean(&inner[..i]), Some(clean(&inner[i + 1..]))),
        None => (clean(inner), None),
    }
}

/// Resolve a special field by its (spaceless) name.
fn special_value(data_source: &str, state: &ResolveState) -> Value {
    let key = data_source.to_lowercase().replace(['{', '}', ' '], "");
    match key.as_str() {
        "pagenumber" => Value::Number(state.page_number as f64),
        "totalpagecount" => Value::Number(state.total_pages as f64),
        "recordnumber" => Value::Number(state.record_number as f64),
        "pagenofm" => Value::Str(format!("{} of {}", state.page_number, state.total_pages)),
        // Other specials (dates/times) need the print-run clock — deferred to the orchestrator.
        _ => Value::Null,
    }
}

/// Replace each `{ref}` run in `src` with its resolved formatted value.
fn substitute_braces(
    src: &str,
    ctx: &DataContext,
    state: &ResolveState,
    loc: &Locale,
    diag: &DiagSink,
) -> String {
    let mut out = String::with_capacity(src.len());
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            if let Some(close) = chars[i..].iter().position(|&c| c == '}') {
                let inner: String = chars[i..=i + close].iter().collect(); // includes braces
                let value = resolve_embedded(&inner, ctx, state, loc, diag);
                out.push_str(&value);
                i += close + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Resolve one embedded `{…}` reference (field/formula/param) to its display string, formatted with
/// the locale's system defaults (an embedded ref carries no per-field format leaf of its own).
fn resolve_embedded(
    reference: &str,
    ctx: &DataContext,
    state: &ResolveState,
    loc: &Locale,
    diag: &DiagSink,
) -> String {
    let inner = reference.trim_matches(['{', '}']);
    let value = if let Some(name) = inner.strip_prefix('@') {
        ctx.resolve(RefKind::Formula, name).unwrap_or(Value::Null)
    } else if let Some(name) = inner.strip_prefix('#') {
        // A running total embedded in a text object.
        ctx.resolve(RefKind::RunningTotal, name)
            .unwrap_or(Value::Null)
    } else if inner.starts_with('?') {
        Value::Null // parameters resolved by a higher layer
    } else {
        let _ = state;
        eval_ref(&brace(inner), ctx, diag, inner)
    };
    render_value_default(&value, loc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_model::SummaryOperation;

    fn summ(field: &str, n: f64) -> Summary {
        Summary {
            operation: SummaryOperation::Sum,
            field: field.to_string(),
            value: Value::Number(n),
        }
    }

    #[test]
    fn parse_summary_args_one_and_two_arg() {
        assert_eq!(
            parse_summary_args("Sum ({Command.total})"),
            ("Command.total".to_string(), None)
        );
        assert_eq!(
            parse_summary_args("Sum ({@90+}, {Command.cost_center})"),
            ("@90+".to_string(), Some("Command.cost_center".to_string()))
        );
    }

    /// A 2-argument, group-scoped summary resolves against the named group's summaries, not the
    /// nearest in-scope ones.
    #[test]
    fn group_scoped_summary_picks_the_named_group() {
        let state = ResolveState {
            // Nearest / grand-total scope: Sum(amt) = 50.
            summaries: vec![summ("t.amt", 50.0)],
            // Two enclosing groups; the region group's Sum(amt) = 30.
            group_summaries: Rc::new(vec![
                ("t.year".to_string(), vec![summ("t.amt", 999.0)]),
                ("t.region".to_string(), vec![summ("t.amt", 30.0)]),
            ]),
            ..ResolveState::default()
        };
        // 2-arg form scoped to the region group → 30.
        assert_eq!(
            summary_value("Sum ({t.amt}, {t.region})", &state),
            Value::Number(30.0)
        );
        // 1-arg (grand total) → the nearest summaries → 50.
        assert_eq!(summary_value("Sum ({t.amt})", &state), Value::Number(50.0));
        // 2-arg group that isn't in scope falls back to the nearest summaries (fail-safe).
        assert_eq!(
            summary_value("Sum ({t.amt}, {t.unknown})", &state),
            Value::Number(50.0)
        );
    }
}
