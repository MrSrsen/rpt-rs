//! Record sort (a stable composite-key comparison) and the summary-based Top N / Bottom N group sort.

use crate::context::{FormulaRegistry, Parameters};
use crate::diagnostics::{DiagnosticKind, DiagnosticSink, EvalDiagnostic};
use crate::source::Row;
use crate::value_order::compare_values;
use crate::GroupInstance;
use rpt_formula::eval::Value;
use rpt_formula::token::short_name;
use rpt_model::{Group, SortDirection};
use std::cmp::Ordering;

use super::aggregate::{summarize, SummaryDef};

/// Apply a **summary-based Top N / Bottom N** group sort to one level's group instances.
///
/// A group sorted by a summary expression (`Sum of {field}`) with a Top N limit is ranked by that
/// summary and cut to the top (or bottom) N groups; the groups outside the cut are either **discarded**
/// (SDK `EnableDiscardOtherGroups`) or **collapsed** into a single trailing "Others" group named by
/// `TextForOther`. Groups with no limit (`number_of_groups == 0`) keep their key ordering — this
/// handles only the truncating Top N / Bottom N case (`topn = None` for a plain group-field sort).
///
/// The collapsed "Others" group flattens the leftover groups' rows into its details (dropping any
/// nested subgroup structure) and re-summarizes over them, so its header/footer totals are correct.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_group_topn(
    mut instances: Vec<GroupInstance>,
    group: &Group,
    level: usize,
    summaries: &[SummaryDef],
    formulas: &FormulaRegistry,
    params: &Parameters,
    sink: Option<&dyn DiagnosticSink>,
) -> Vec<GroupInstance> {
    let Some(topn) = group.sort.topn.as_ref() else {
        return instances;
    };
    let n = topn.number_of_groups as usize;
    if n == 0 {
        return instances;
    }
    // Rank by the sort's summary. Its summed field is the first `{...}` of the sort expression.
    let Some(summed_field) = topn_summed_field(&group.sort.field) else {
        return instances;
    };
    // A ranking value must exist for every group, else the cut is arbitrary — leave key order.
    if instances
        .iter()
        .any(|g| topn_rank(g, &summed_field).is_none())
    {
        if let Some(sink) = sink {
            sink.report(
                EvalDiagnostic::new(
                    DiagnosticKind::Formula,
                    format!(
                        "Top N group sort on {:?} has no matching summary to rank by; \
                         groups left in key order",
                        group.sort.field
                    ),
                )
                .from_source(&group.condition_field),
            );
        }
        return instances;
    }
    // Top N orders largest-first; Bottom N smallest-first. A stable sort keeps ties in key order.
    let descending = group.sort.direction == SortDirection::TopNOrder;
    instances.sort_by(|a, b| {
        let ord = compare_values(
            &topn_rank(a, &summed_field).expect("checked above"),
            &topn_rank(b, &summed_field).expect("checked above"),
        );
        if descending {
            ord.reverse()
        } else {
            ord
        }
    });
    if instances.len() <= n {
        return instances;
    }
    let others = instances.split_off(n);
    if !topn.discard_others {
        let mut rows = Vec::new();
        for g in &others {
            flatten_group_rows(g, &mut rows);
        }
        instances.push(GroupInstance {
            level,
            condition_field: group.condition_field.clone(),
            key: Value::Str(topn.not_in_topn_name.clone()),
            date_condition: group.date_condition,
            summaries: summarize(&rows, summaries, formulas, params),
            subgroups: Vec::new(),
            details: rows,
            hierarchy_children: Vec::new(),
        });
    }
    instances
}

/// The summed field of a group summary-sort expression: the first `{...}` of `Sum ({field}, {group})`.
fn topn_summed_field(sort_field: &str) -> Option<String> {
    let start = sort_field.find('{')? + 1;
    let end = sort_field[start..].find('}')? + start;
    Some(sort_field[start..end].to_string())
}

/// The numeric ranking value for a group: the value of the summary keyed by `summed_field` (matched by
/// full or short name). `None` when the group has no such (numeric) summary.
fn topn_rank(g: &GroupInstance, summed_field: &str) -> Option<Value> {
    let target = summed_field.to_lowercase();
    let short = short_name(&target);
    g.summaries
        .iter()
        .find(|s| {
            let f = s.field.to_lowercase();
            f == target || short_name(&f) == short
        })
        .filter(|s| s.value.as_number().is_some())
        .map(|s| s.value.clone())
}

/// Flatten a group's rows (its own details plus every subgroup's, recursively) into `out`.
fn flatten_group_rows(g: &GroupInstance, out: &mut Vec<Row>) {
    out.extend(g.details.iter().cloned());
    for sg in &g.subgroups {
        flatten_group_rows(sg, out);
    }
}

/// Compare two decorated sort keys lexicographically, each field honoring its own direction. Returns
/// `Equal` only when every field ties, leaving the stable sort to preserve read order.
pub(super) fn compare_sort_keys(a: &[Value], b: &[Value], dirs: &[SortDirection]) -> Ordering {
    for ((av, bv), dir) in a.iter().zip(b).zip(dirs) {
        let ord = match dir {
            SortDirection::DescendingOrder => compare_values(av, bv).reverse(),
            _ => compare_values(av, bv),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}
