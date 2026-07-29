//! Record- and group-selection filtering: the record-selection diagnostic detail and the
//! group-selection (HAVING-like) pruning of the group tree.

use crate::diagnostics::{DiagnosticKind, DiagnosticSink, EvalDiagnostic};
use crate::{GroupInstance, Summary};
use rpt_formula::eval::{vm, Value};
use rpt_formula::token::{short_name, strip_braces};
use rpt_formula::{parse, Syntax};
use rpt_model::{DataDefinition, SummaryOperation};

/// Describe why a selection formula's result was not a clean boolean, for a diagnostic: an
/// evaluation error, or a non-boolean value where a boolean was expected.
pub(super) fn selection_detail(result: &Result<Value, rpt_formula::eval::EvalError>) -> String {
    match result {
        Ok(value) => format!("selection formula returned a non-boolean value: {value:?}"),
        Err(err) => err.to_string(),
    }
}

/// Apply the `group_selection` formula as a HAVING-like filter on the group tree: evaluate it per
/// leaf group against that group's summaries and drop the groups it rejects, pruning ancestors that
/// become empty. **Fail-open**: filtering applies only when every reference in the selection resolves
/// reliably at the group level (running-total `{#name}` summaries and group condition fields); any
/// other reference, or a non-boolean / erroring result, keeps the group — so a running total or field
/// summary can never wrongly drop data this way.
pub(super) fn apply_group_selection(
    tree: &mut Vec<GroupInstance>,
    data_def: &DataDefinition,
    sink: Option<&dyn DiagnosticSink>,
) {
    let Some(sel) = &data_def.group_selection else {
        return;
    };
    let body = sel.0.trim();
    if body.is_empty() {
        return;
    }
    // Only filter when every reference resolves group-constantly — a running total, a group
    // condition field, or a field consumed by a group-scoped summary function (`Sum({x}, {g})`,
    // whose value is a computed group summary). Any other reference fails open (keep every group).
    use rpt_formula::refs::is_aggregation_function;
    use rpt_formula::{references, RefKind};
    let cond_fields: std::collections::HashSet<String> = data_def
        .groups
        .iter()
        .map(|g| short_name(&g.condition_field))
        .collect();
    let refs: Vec<_> = references(body).collect();
    let safe = !refs.is_empty()
        && refs.iter().all(|r| match r.kind {
            RefKind::RunningTotal => true,
            RefKind::Field => {
                cond_fields.contains(&short_name(&r.name))
                    || r.enclosing_fn
                        .as_deref()
                        .is_some_and(is_aggregation_function)
            }
            _ => false,
        });
    if !safe {
        return;
    }
    // The group level the selection filters at: the deepest group whose condition field the formula
    // names (as a group-scoped summary's group argument, or referenced directly). A summary like
    // `DistinctCount({x}, {carrier.name})` scopes to the carrier group, so the selection prunes whole
    // carrier groups — not the innermost leaf. Matched on the **full** condition field, since a report
    // grouped on `a.name`/`b.name`/… shares the short name across levels. Falls back to the leaf.
    let target_level = refs
        .iter()
        .filter_map(|r| {
            let name = strip_braces(&r.name);
            data_def
                .groups
                .iter()
                .rposition(|g| strip_braces(&g.condition_field).eq_ignore_ascii_case(name))
        })
        .max()
        .unwrap_or_else(|| data_def.groups.len().saturating_sub(1));
    let (ast, diagnostics) = parse(body, Syntax::Crystal);
    crate::diagnostics::report_parse_diagnostics(
        sink,
        "the group-selection formula",
        body,
        &diagnostics,
    );
    let chunk = vm::compile(&ast);
    filter_group_tree_at(tree, target_level, &chunk, sink);
}

/// Recursively retain groups the selection keeps: at the `target_level` a group is dropped when the
/// selection cleanly evaluates to `false` (against that group's computed summaries); an ancestor is
/// dropped once all its subgroups are gone.
fn filter_group_tree_at(
    groups: &mut Vec<GroupInstance>,
    target_level: usize,
    chunk: &vm::Chunk,
    sink: Option<&dyn DiagnosticSink>,
) {
    groups.retain_mut(|g| {
        if g.level >= target_level {
            keep_group(g, chunk, sink)
        } else {
            filter_group_tree_at(&mut g.subgroups, target_level, chunk, sink);
            !g.subgroups.is_empty()
        }
    });
}

/// Whether a group survives the group-selection formula. Keeps the group on anything but a clean
/// `false` (fail-open); an error or non-boolean result keeps the group but is reported to `sink`.
fn keep_group(g: &GroupInstance, chunk: &vm::Chunk, sink: Option<&dyn DiagnosticSink>) -> bool {
    let ctx = GroupFilterContext { group: g };
    match vm::run(chunk, &ctx) {
        // A clean `false` drops the group (ordinary HAVING filtering); a clean `true` keeps it.
        Ok(Value::Bool(false)) => false,
        Ok(Value::Bool(true)) => true,
        // An error or a non-boolean result keeps the group (fail-open); report it first.
        other => {
            if let Some(sink) = sink {
                sink.report(EvalDiagnostic::new(
                    DiagnosticKind::GroupSelection,
                    selection_detail(&other),
                ));
            }
            true
        }
    }
}

/// A minimal [`EvalContext`] for evaluating a group-selection formula against one group instance:
/// `{field}` resolves group-constantly (this group's condition field from its key, else its first
/// detail row), `{#name}` resolves from the group's computed `#name` summary, and a group-scoped
/// summary function (`DistinctCount({x}, {g})`) resolves from the group's computed summaries.
struct GroupFilterContext<'a> {
    group: &'a GroupInstance,
}

impl rpt_formula::eval::EvalContext for GroupFilterContext<'_> {
    fn resolve(&self, kind: rpt_formula::RefKind, name: &str) -> Option<Value> {
        use rpt_formula::RefKind;
        match kind {
            RefKind::Field => {
                // This group's own key when the reference names its condition field; otherwise the
                // first detail row (populated only at the innermost level).
                if short_name(&self.group.condition_field) == short_name(name) {
                    Some(self.group.key.clone())
                } else {
                    self.group
                        .details
                        .first()
                        .and_then(|r| r.get(name).cloned())
                }
            }
            RefKind::RunningTotal => {
                let want = name.trim_start_matches('#');
                self.group
                    .summaries
                    .iter()
                    .find(|s| s.field.trim_start_matches('#').eq_ignore_ascii_case(want))
                    .map(|s| s.value.clone())
            }
            _ => None,
        }
    }

    fn resolve_summary(&self, op: &str, field: &str, _group: Option<&str>) -> Option<Value> {
        // The context is already positioned at the group the selection filters, so the summary's
        // group argument is implicit — match the group's computed summary by operation and field.
        let field_eq = |s: &&Summary| {
            let f = strip_braces(&s.field);
            f.eq_ignore_ascii_case(strip_braces(field)) || short_name(f) == short_name(field)
        };
        self.group
            .summaries
            .iter()
            .find(|s| field_eq(s) && summary_op_matches(s.operation, op))
            .or_else(|| self.group.summaries.iter().find(field_eq))
            .map(|s| s.value.clone())
    }
}

/// Whether a computed summary's [`SummaryOperation`] is the one named by a summary function's
/// (lowercased) operation token — covering the aliases the engine accepts (`avg`/`average`, …).
fn summary_op_matches(op: SummaryOperation, token: &str) -> bool {
    use SummaryOperation as Op;
    let accepted: &[&str] = match op {
        Op::Sum => &["sum"],
        Op::Average => &["avg", "average"],
        Op::Count => &["count"],
        Op::DistinctCount => &["distinctcount"],
        Op::Maximum => &["max", "maximum"],
        Op::Minimum => &["min", "minimum"],
        Op::SampleVariance => &["variance", "samplevariance"],
        Op::SampleStandardDeviation => &["stddev", "samplestddev"],
        Op::PopVariance => &["popvariance", "populationvariance"],
        Op::PopStandardDeviation => &["popstddev", "populationstddev"],
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
    accepted.contains(&token.to_ascii_lowercase().as_str())
}
