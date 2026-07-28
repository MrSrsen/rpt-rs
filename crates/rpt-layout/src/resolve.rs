//! Resolve a report object's bound value(s) to display strings, in a given record/group context.
//!
//! Field objects carry a `data_source` reference and a [`FieldRefKind`]; this turns that into a
//! [`Value`] (via the formula evaluator) and then a formatted string. Text objects with embedded
//! `{…}` references get per-reference substitution. The display format is resolved by
//! [`crate::format`], merging the render locale with the field's stored `FieldFormat` leaf.

use crate::format::{field_format_spec, render_value, render_value_default};
use crate::{push_diag, DiagSink};
use crystal_formula::eval::{EvalContext, EvalError, Value};
use crystal_formula::token::{brace_groups, short_name, split_reference, strip_braces};
use crystal_formula::{parse, Node, RefKind, Syntax};
use rpt_data::{
    DataContext, FormulaRegistry, Row, RunningTotals, ScheduledValues, SharedState, Summary,
};
use rpt_format_value::Locale;
use rpt_model::{
    field_object_value_type, Color, FieldObject, FieldRefKind, Report, SummaryOperation, TextObject,
};
use rpt_pages::{Diagnostic, DiagnosticKind};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    /// Memoized parses of inline formula bodies and brace-refs. The resolve hot path evaluates the
    /// same conditional-format bodies and `{table.field}` references once per record; parsing is pure
    /// in the source text, so caching the AST by that text turns per-record lexing+parsing into a hash
    /// lookup. Keyed by the exact source (every site parses with [`Syntax::Crystal`]). Lives for the
    /// process (bounded by the distinct formula bodies a run encounters).
    static AST_CACHE: RefCell<HashMap<String, Rc<ParsedFormula>>> = RefCell::new(HashMap::new());
}

/// Parse `src` as a Crystal formula, memoized by the source text (see [`AST_CACHE`]).
///
/// The diagnostics are cached with the AST rather than dropped, so a caller with a sink can report
/// them ([`parse_cached_reporting`]) without the memoization silently swallowing every occurrence
/// after the first.
fn parse_cached(src: &str) -> Rc<ParsedFormula> {
    AST_CACHE.with(|c| {
        if let Some(parsed) = c.borrow().get(src) {
            return Rc::clone(parsed);
        }
        let (node, diagnostics) = parse(src, Syntax::Crystal);
        let parsed = Rc::new(ParsedFormula { node, diagnostics });
        c.borrow_mut().insert(src.to_string(), Rc::clone(&parsed));
        parsed
    })
}

/// [`parse_cached`], reporting any parse diagnostic to `diag` under `label`.
///
/// A formula that does not parse is evaluated from the parser's partial recovery AST, so its value is
/// meaningless — worth saying, and fixable by the report's author. Reported on every call rather than
/// once per cache miss; identical diagnostics are collapsed where they are presented.
fn parse_cached_reporting(src: &str, diag: &DiagSink, label: &str) -> Rc<ParsedFormula> {
    let parsed = parse_cached(src);
    for d in &parsed.diagnostics {
        push_diag(
            diag,
            Diagnostic::warn(
                DiagnosticKind::FormulaParse,
                format!(
                    "{}: {} at byte {}; evaluated from a partial parse, so the value is not meaningful",
                    label, d.message, d.start
                ),
            )
            .with_source(label)
            .at_span(d.start..d.end),
        );
    }
    parsed
}

/// A parsed formula and whatever the parser had to say about it.
struct ParsedFormula {
    node: Node,
    diagnostics: Vec<crystal_formula::Diagnostic>,
}

/// The group/summary state a resolver needs beyond the current row.
#[derive(Debug, Clone, Default)]
pub struct ResolveState {
    /// The nearest enclosing group's key (for `GroupName` fields).
    pub group_key: Option<Value>,
    /// Summaries in scope (the nearest group's, else the grand total) for summary-field lookup.
    ///
    /// Shared (`Rc`) rather than owned so every per-detail-row [`ResolveState`] is a cheap refcount
    /// bump of the group-constant summary vec, not a deep clone per record.
    pub summaries: Rc<Vec<Summary>>,
    /// Each enclosing group's `(condition field, summaries)`, outermost first. A
    /// **group-scoped** (2-argument) summary `Op ({field}, {group condition field})` resolves against
    /// the summaries of the group whose condition field is the 2nd argument, rather than the nearest.
    ///
    /// Shared (`Rc`) rather than owned so the formatter builds this projection once per group-stack
    /// change and every per-record [`ResolveState`] is a cheap refcount bump, not a deep clone of
    /// every enclosing group's summary vec.
    pub group_summaries: Rc<Vec<(String, Vec<Summary>)>>,
    /// The report grand-total summaries. A 1-argument summary — `Sum({field})` with no group operand
    /// — is always the report total, so it resolves here rather than against the innermost group's
    /// [`summaries`](Self::summaries) (which is the nearest scope, correct only at the report footer).
    ///
    /// Shared (`Rc`) so every per-record [`ResolveState`] is a refcount bump, not a deep clone.
    pub grand_summaries: Rc<Vec<Summary>>,
    /// Print-state specials for the current position (page number, record number, …).
    pub page_number: i64,
    pub total_pages: i64,
    pub record_number: i64,
}

/// The in-scope summaries resolve a summary function inside a formula body the same way a placed
/// summary object resolves — by operation, summarized field, and group scope.
impl rpt_data::SummaryScope for ResolveState {
    fn resolve_summary(&self, op: &str, field: &str, group: Option<&str>) -> Value {
        resolve_summary_in_scope(Some(op), field, group, self)
    }
}

/// Build a [`DataContext`] for `row` carrying the standard specials from `state`, the report
/// parameter values (`{?Name}` resolution), the print-order running totals (`{#name}`), the in-scope
/// summaries (so a summary function in a formula body resolves), and any pre-scheduled formula values.
#[allow(clippy::too_many_arguments)]
pub fn context<'a>(
    row: &'a Row,
    formulas: &'a FormulaRegistry,
    params: &'a rpt_data::Parameters,
    state: &'a ResolveState,
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
        .with_summaries(state)
        .with_special("recordnumber", Value::Number(state.record_number as f64))
        .with_special("pagenumber", Value::Number(state.page_number as f64))
        .with_special("totalpagecount", Value::Number(state.total_pages as f64))
}

/// Resolve a field object to a [`Value`] in the given context, recording any runtime formula error
/// into `diag`. `ctx` is `None` for a band with no data row (a report footer): the row-independent
/// kinds (`Summary`/`Special`/`GroupName`) still resolve from `state`, while the row-bound kinds
/// (`DatabaseField`/`Formula`/`RunningTotal`/parameter) yield [`Value::Null`].
pub fn field_value(
    obj: &FieldObject,
    ctx: Option<&DataContext>,
    state: &ResolveState,
    diag: &DiagSink,
) -> Value {
    match obj.ref_kind {
        FieldRefKind::DatabaseField => ctx
            .map(|c| eval_ref(&brace(&obj.data_source), c, diag, &obj.data_source))
            .unwrap_or(Value::Null),
        FieldRefKind::Formula => {
            let name = split_reference(strip_braces(&obj.data_source)).1;
            ctx.and_then(|c| c.resolve(RefKind::Formula, name))
                .unwrap_or(Value::Null)
        }
        FieldRefKind::GroupName => state.group_key.clone().unwrap_or(Value::Null),
        FieldRefKind::Summary => summary_value(&obj.data_source, state),
        FieldRefKind::Special => special_value(&obj.data_source, state),
        // A running total (`{#name}`) resolves to the print-order value accumulated up to the current
        // record; the layout advances it per record before the band is emitted.
        FieldRefKind::RunningTotal => {
            let name = split_reference(strip_braces(&obj.data_source)).1;
            ctx.and_then(|c| c.resolve(RefKind::RunningTotal, name))
                .unwrap_or(Value::Null)
        }
        // Parameter / SQL-expression resolution lands with their owning layers.
        _ => ctx
            .map(|c| eval_ref(&brace(&obj.data_source), c, diag, &obj.data_source))
            .unwrap_or(Value::Null),
    }
}

/// The formatted display string for a field object: the field's **effective** value type + stored
/// [`rpt_model::FieldFormat`] leaf are merged with the render `locale` to pick the effective format
/// (integer types → 0 decimals; explicit stored decimals/negative/currency/date-forms win over the
/// locale defaults; names/separators always come from the locale — see [`crate::format`]).
///
/// The effective value type comes from [`field_object_value_type`]: database/summary objects carry
/// it on the object, but formula / parameter / running-total / SQL-expression / special objects leave
/// the object's `value_type` as `Unknown` and resolve it from the referenced field *definition*, so a
/// formula field honours its own stored numeric/date format leaf rather than falling through to bare
/// string/number formatting.
pub fn field_text(
    report: &Report,
    obj: &FieldObject,
    ctx: Option<&DataContext>,
    state: &ResolveState,
    loc: &Locale,
    diag: &DiagSink,
) -> String {
    let value = field_value(obj, ctx, state, diag);
    let value_type = field_object_value_type(report, obj);
    let spec = field_format_spec(obj.format.as_ref(), value_type, loc);
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
        Some(c) if !obj.embedded_fields.is_empty() && brace_groups(src).next().is_some() => {
            substitute_braces(src, c, state, loc, diag)
        }
        _ => src.clone(),
    }
}

/// Stored conditional-format formula names, keyed exactly as `rpt` decodes them from the record
/// (the reserved `@`-slot names — see `is_modeled_condition` in the reader). A condition list is a
/// `Vec<(name, body)>`; [`cond_color`]/[`cond_bool`] match on these names, so they must be the
/// *stored* names, not the SDK display names (`Back_Color`, not `BackgroundColor`).
pub mod cond {
    /// Background fill color of an object/border (`@Back_Color`).
    pub const BACK_COLOR: &str = "Back_Color";
    /// Line color of an object's border (`@Fore_Color`).
    pub const FORE_COLOR: &str = "Fore_Color";
    /// Font color of a text/field object (`@Font_Color`).
    pub const FONT_COLOR: &str = "Font_Color";
    /// Object visibility / suppress flag (`@Object_Visibility`).
    pub const OBJECT_VISIBILITY: &str = "Object_Visibility";
    /// Section visibility / suppress flag (`@Section_Visibility`).
    pub const SECTION_VISIBILITY: &str = "Section_Visibility";
    /// A section's own background-fill color, stored under one of several reserved names across
    /// engine versions. All map to the same section-background condition.
    pub const SECTION_BACK_COLORS: &[&str] =
        &["Section_Back_Color", "Background_Color", "Back_Color"];
}

/// Evaluate the first matching color condition among several candidate reserved names (used for the
/// section-background condition, stored under one of a few names across engine versions).
pub fn cond_color_any(
    conditions: &[(String, String)],
    keys: &[&str],
    ctx: Option<&DataContext>,
) -> Option<Color> {
    keys.iter().find_map(|k| cond_color(conditions, k, ctx))
}

/// Evaluate a conditional-format formula body (e.g. a border's `Back_Color` formula) in the
/// current record context, decoding the resulting Crystal COLORREF number to a [`Color`]. Returns
/// `None` when there is no context, no such formula, or the formula yields `crNoColor` (`-1`).
pub fn cond_color(
    conditions: &[(String, String)],
    key: &str,
    ctx: Option<&DataContext>,
) -> Option<Color> {
    let ctx = ctx?;
    let body = conditions.iter().find(|(k, _)| k == key).map(|(_, b)| b)?;
    let ast = parse_cached(body);
    let value = crystal_formula::eval::eval(&ast.node, ctx).ok()?;
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
    let ast = parse_cached(body);
    match crystal_formula::eval::eval(&ast.node, ctx).ok()? {
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
    let ast = parse_cached_reporting(expr, diag, label);
    // `eval_spanned` costs nothing extra and yields the byte range of the failing sub-expression —
    // which is the difference between "formula error: type mismatch" and being able to point at the
    // offending operator.
    match crystal_formula::eval::eval_spanned(&ast.node, ctx) {
        Ok(v) => v,
        Err(e) => {
            record_eval_error(diag, label, &e.error, Some(e.span.start..e.span.end));
            Value::Null
        }
    }
}

/// Record a formula evaluation error as a diagnostic, distinguishing an unimplemented builtin/feature
/// ([`EvalError::Unsupported`]) from an ordinary runtime error. `span`, when known, is the byte range
/// within the formula text that failed.
fn record_eval_error(
    diag: &DiagSink,
    label: &str,
    err: &EvalError,
    span: Option<std::ops::Range<usize>>,
) {
    let (kind, msg) = match err {
        EvalError::Unsupported(what) => (
            DiagnosticKind::UnsupportedFormula,
            format!("unsupported in formula: {what}"),
        ),
        e => (DiagnosticKind::FormulaError, format!("formula error: {e}")),
    };
    let mut d = Diagnostic::warn(kind, msg).with_source(label);
    // A `0..0` span means the failing op had no source origin — no location is better than a wrong one.
    if let Some(span) = span.filter(|s| s.end > s.start) {
        d = d.at_span(span);
    }
    push_diag(diag, d);
}

/// Evaluate a bare field/formula reference (`Table.field` or `@formula`) to a [`Value`] in `ctx`,
/// reporting a failure to `diag` under `label` rather than swallowing it.
///
/// The cross-tab and chart pivots call this per cell. A silent `unwrap_or(Null)` here made a whole
/// cross-tab column read as empty with nothing to say why, even while the surrounding code was
/// emitting diagnostics for every other formula failure.
pub(crate) fn eval_field_ref_reported(
    reference: &str,
    ctx: &DataContext,
    diag: &DiagSink,
    label: &str,
) -> Value {
    let ast = parse_cached_reporting(&brace(reference), diag, label);
    match crystal_formula::eval::eval_spanned(&ast.node, ctx) {
        Ok(v) => v,
        Err(e) => {
            record_eval_error(diag, label, &e.error, Some(e.span.start..e.span.end));
            Value::Null
        }
    }
}

/// Evaluate a bare field/formula reference with no diagnostics.
///
/// Prefer [`eval_field_ref_reported`]: this exists only for call sites with no diagnostic sink in
/// scope, and every such site is a place where a failure goes unreported.
pub(crate) fn eval_field_ref(reference: &str, ctx: &DataContext) -> Value {
    let ast = parse_cached(&brace(reference));
    crystal_formula::eval::eval(&ast.node, ctx).unwrap_or(Value::Null)
}

/// Resolve a blob field's bound reference to its runtime value, or `None` when null or empty. A
/// bytes-capable datasource (live DB) delivers the blob as [`Value::Bytes`]; saved data or a
/// text-only backend delivers it as a [`Value::Str`] (raw bytes in a lossy string, or a Postgres
/// `\x` hex-escape). The caller turns either into image bytes.
pub(crate) fn blob_value(data_source: &str, ctx: &DataContext) -> Option<Value> {
    match eval_field_ref(data_source, ctx) {
        Value::Bytes(b) if !b.is_empty() => Some(Value::Bytes(b)),
        Value::Str(s) if !s.is_empty() => Some(Value::Str(s)),
        _ => None,
    }
}

/// Ensure a reference is brace-wrapped for parsing (`table.field` → `{table.field}`).
fn brace(reference: &str) -> String {
    let r = reference.trim();
    if brace_groups(r).next().is_some() {
        r.to_string()
    } else {
        format!("{{{r}}}")
    }
}

/// Look up a summary field's value from the in-scope summaries by its operation and summarized field.
///
/// The data source is `Op ({summarized})` (report-level, grand total) or, for a **group-scoped**
/// summary, `Op ({summarized}, {group condition field})`. Only an index
/// is stored on the object; the group is recovered from context — the 2nd argument names the group's
/// condition field, so we resolve against **that group's** computed summaries (from
/// [`ResolveState::group_summaries`]) rather than the nearest group's. A 1-argument summary, or a 2nd
/// argument that matches no enclosing group, falls back to the nearest in-scope summaries.
fn summary_value(data_source: &str, state: &ResolveState) -> Value {
    let (field_arg, group_arg) = parse_summary_args(data_source);
    let op = summary_op_token(data_source);
    resolve_summary_in_scope(op, &field_arg, group_arg.as_deref(), state)
}

/// Resolve a summary by its operation token, summarized field, and optional group scope against the
/// report's computed summaries — the shared core of a placed summary object ([`summary_value`]) and a
/// summary function inside a formula body (the [`rpt_data::SummaryScope`] impl). Returns [`Value::Null`]
/// when no summary matches, so a missing summary renders blank rather than failing the whole formula.
fn resolve_summary_in_scope(
    op: Option<&str>,
    field: &str,
    group: Option<&str>,
    state: &ResolveState,
) -> Value {
    // Pick the summaries to search. A 2-argument summary is scoped to the group whose condition field
    // is the 2nd argument; a 1-argument summary is the report grand total. Match the **full** field
    // name first — a report grouped on `a.name` / `b.name` / `c.name` has several levels sharing the
    // short name `name`, and matching by short name alone would collapse every level onto the
    // outermost group. The short-name match is kept only as a fallback (an aliased/qualified 2nd
    // argument that differs textually); a 2nd argument matching no enclosing group falls back to the
    // nearest in-scope summaries.
    let summaries = match group {
        Some(g) => {
            let groups = state.group_summaries.iter();
            groups
                .clone()
                .find(|(cond, _)| full_field_eq(cond, g))
                .or_else(|| {
                    groups
                        .clone()
                        .find(|(cond, _)| short_name(cond) == short_name(g))
                })
                .map(|(_, s)| s.as_slice())
                .unwrap_or(state.summaries.as_slice())
        }
        None => state.grand_summaries.as_slice(),
    };
    // A field can carry several summaries of different operations (Sum and Avg of the same field), so
    // match on operation *and* field; fall back to a field-only match so an operation token we do not
    // map still resolves to its (single) summary rather than nothing.
    let field_matches = |s: &&Summary| {
        let f = strip_braces(&s.field);
        f == field || short_name(f) == short_name(field)
    };
    summaries
        .iter()
        .find(|s| field_matches(s) && op.is_some_and(|t| op_token_matches(s.operation, t)))
        .or_else(|| summaries.iter().find(field_matches))
        .map(|s| s.value.clone())
        .unwrap_or(Value::Null)
}

/// The operation token of a summary expression — the text before the first `(` (`"Sum ({x})"` →
/// `"Sum"`). `None` when the source has no operator prefix.
fn summary_op_token(data_source: &str) -> Option<&str> {
    let op = data_source.split('(').next()?.trim();
    (!op.is_empty()).then_some(op)
}

/// Whether a summary's [`SummaryOperation`] is the one named by an expression's operation token. The
/// token comes from a stored/authored expression, so a few operations have more than one accepted
/// spelling (`Avg`/`Average`, `Max`/`Maximum`, `StdDev`/`SampleStdDev`, …); the comparison is
/// case-insensitive.
fn op_token_matches(op: SummaryOperation, token: &str) -> bool {
    use SummaryOperation as Op;
    let t = token.to_ascii_lowercase();
    let accepted: &[&str] = match op {
        Op::Sum => &["sum"],
        Op::Average => &["avg", "average"],
        Op::Count => &["count"],
        Op::DistinctCount => &["distinctcount"],
        Op::Maximum => &["max", "maximum"],
        Op::Minimum => &["min", "minimum"],
        Op::SampleVariance => &["variance", "samplevariance"],
        Op::SampleStandardDeviation => &["stddev", "samplestddev", "samplestandarddeviation"],
        Op::PopVariance => &["popvariance", "populationvariance"],
        Op::PopStandardDeviation => &[
            "popstddev",
            "populationstddev",
            "populationstandarddeviation",
        ],
        Op::Correlation => &["correlation"],
        Op::Covariance => &["covariance"],
        Op::WeightedAvg => &["weightedavg", "weightedaverage"],
        Op::Median => &["median"],
        Op::Percentile => &["percentile", "pthpercentile"],
        Op::NthLargest => &["nthlargest"],
        Op::NthSmallest => &["nthsmallest"],
        Op::Mode => &["mode"],
        Op::NthMostFrequent => &["nthmostfrequent"],
        Op::Other(_) => &[],
    };
    accepted.contains(&t.as_str())
}

/// Whether two field references name the same field, comparing the full (brace-stripped)
/// name case-insensitively — so `cat_class.name` and `cat_group.name` are distinguished (unlike
/// [`short_name`], which reduces both to `name`).
fn full_field_eq(a: &str, b: &str) -> bool {
    strip_braces(a).eq_ignore_ascii_case(strip_braces(b))
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
    let clean = |s: &str| strip_braces(s).to_string();
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
        "pagenofm" => Value::Str(format!(
            "Page {} of {}",
            state.page_number, state.total_pages
        )),
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
    let mut rest = src;
    // Scan by byte index: copy the literal run up to each `{`, then resolve the `{…}` slice (braces
    // included, as `resolve_embedded` expects). Both braces are ASCII, so `find` returns char
    // boundaries and the string slices are valid.
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open..];
        match after.find('}') {
            Some(close) => {
                out.push_str(&resolve_embedded(&after[..=close], ctx, state, loc, diag));
                rest = &after[close + 1..];
            }
            // An unmatched `{`: the remainder is literal.
            None => {
                rest = after;
                break;
            }
        }
    }
    out.push_str(rest);
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
    let inner = strip_braces(reference);
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
        summ_op(SummaryOperation::Sum, field, n)
    }

    fn summ_op(operation: SummaryOperation, field: &str, n: f64) -> Summary {
        Summary {
            operation,
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
            // Nearest in-scope summaries (the innermost group): Sum(amt) = 50.
            summaries: Rc::new(vec![summ("t.amt", 50.0)]),
            // Report grand total: Sum(amt) = 500.
            grand_summaries: Rc::new(vec![summ("t.amt", 500.0)]),
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
        // 1-arg (grand total) → the grand-total summaries, never the innermost group → 500.
        assert_eq!(summary_value("Sum ({t.amt})", &state), Value::Number(500.0));
        // 2-arg group that isn't in scope falls back to the nearest summaries (fail-safe).
        assert_eq!(
            summary_value("Sum ({t.amt}, {t.unknown})", &state),
            Value::Number(50.0)
        );
    }

    /// A summary field carrying several operations of the same field resolves by operation, not just
    /// field — `Sum` and `Avg` of one field must each pick their own value.
    #[test]
    fn summary_value_disambiguates_by_operation() {
        let state = ResolveState {
            group_summaries: Rc::new(vec![(
                "shipment_mode.name".to_string(),
                vec![
                    summ_op(SummaryOperation::Sum, "shipment.freight_cost", 324_265.64),
                    summ_op(SummaryOperation::Average, "shipment.freight_cost", 1_080.88),
                ],
            )]),
            ..ResolveState::default()
        };
        assert_eq!(
            summary_value(
                "Sum ({shipment.freight_cost}, {shipment_mode.name})",
                &state
            ),
            Value::Number(324_265.64)
        );
        assert_eq!(
            summary_value(
                "Avg ({shipment.freight_cost}, {shipment_mode.name})",
                &state
            ),
            Value::Number(1_080.88)
        );
    }

    /// A summary function inside a formula body resolves through [`rpt_data::SummaryScope`] exactly as
    /// a placed summary object does — group (2-arg) and grand-total (1-arg) each pick the right scope.
    #[test]
    fn summary_scope_impl_matches_placed_object_resolution() {
        use rpt_data::SummaryScope;
        let state = ResolveState {
            grand_summaries: Rc::new(vec![summ_op(
                SummaryOperation::Count,
                "shipment.shipment_id",
                100.0,
            )]),
            group_summaries: Rc::new(vec![(
                "shipment_mode.name".to_string(),
                vec![summ_op(
                    SummaryOperation::Count,
                    "shipment.shipment_id",
                    40.0,
                )],
            )]),
            ..ResolveState::default()
        };
        assert_eq!(
            state.resolve_summary("count", "shipment.shipment_id", Some("shipment_mode.name")),
            Value::Number(40.0)
        );
        assert_eq!(
            state.resolve_summary("count", "shipment.shipment_id", None),
            Value::Number(100.0)
        );
    }

    /// Groups sharing a short name (`a.name`, `b.name`, `c.name`) must resolve by their full field
    /// name — matching by short name alone would collapse every level onto the outermost group.
    #[test]
    fn group_scoped_summary_disambiguates_shared_short_names() {
        let state = ResolveState {
            summaries: Rc::new(vec![summ("product.pid", 800.0)]),
            group_summaries: Rc::new(vec![
                (
                    "cat_division.name".to_string(),
                    vec![summ("product.pid", 90.0)],
                ),
                (
                    "cat_group.name".to_string(),
                    vec![summ("product.pid", 30.0)],
                ),
                (
                    "cat_class.name".to_string(),
                    vec![summ("product.pid", 25.0)],
                ),
            ]),
            ..ResolveState::default()
        };
        // Each level's 2-arg summary resolves to its own group despite the shared `name` short name.
        assert_eq!(
            summary_value("Count ({product.pid}, {cat_class.name})", &state),
            Value::Number(25.0)
        );
        assert_eq!(
            summary_value("Count ({product.pid}, {cat_group.name})", &state),
            Value::Number(30.0)
        );
        assert_eq!(
            summary_value("Count ({product.pid}, {cat_division.name})", &state),
            Value::Number(90.0)
        );
    }

    /// A report-footer grand-total object has no data row (`ctx = None`); a `Summary` field must
    /// still resolve from the in-scope grand totals, while a row-bound `DatabaseField` yields null.
    #[test]
    fn rowless_band_resolves_summary_but_not_database_field() {
        let state = ResolveState {
            grand_summaries: Rc::new(vec![summ("product.product_id", 800.0)]),
            ..ResolveState::default()
        };
        let diag: DiagSink = std::cell::RefCell::new(Vec::new());

        let grand_total = FieldObject {
            data_source: "Count ({product.product_id})".to_string(),
            ref_kind: FieldRefKind::Summary,
            ..Default::default()
        };
        assert_eq!(
            field_value(&grand_total, None, &state, &diag),
            Value::Number(800.0)
        );

        let db_field = FieldObject {
            data_source: "product.name".to_string(),
            ref_kind: FieldRefKind::DatabaseField,
            ..Default::default()
        };
        assert_eq!(field_value(&db_field, None, &state, &diag), Value::Null);
    }
}
