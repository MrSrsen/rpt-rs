//! Grouping: value-partition and boolean/sequential group construction, plus date/time period
//! bucketing of a group key.

use crate::context::{DataContext, FormulaRegistry, Parameters};
use crate::diagnostics::{DiagnosticKind, DiagnosticSink, EvalDiagnostic};
use crate::source::Row;
use crate::value_order::{compare_values, value_key};
use crate::GroupInstance;
use rpt_formula::eval::{Date, Time, Value};
use rpt_model::{Group, GroupCondition, SortDirection};

use super::aggregate::{summarize, SummaryDef};
use super::sort::apply_group_topn;

/// Recursively group `rows` by `groups[level..]`, computing summaries at each level.
pub(super) fn build_groups(
    rows: &[Row],
    groups: &[Group],
    level: usize,
    summaries: &[SummaryDef],
    formulas: &FormulaRegistry,
    params: &Parameters,
    sink: Option<&dyn DiagnosticSink>,
) -> Vec<GroupInstance> {
    let Some(group) = groups.get(level) else {
        return Vec::new();
    };
    if let Some(cond) = group.date_condition {
        // A boolean condition is order-sensitive (a transition / look-ahead break over the ordered
        // rows), not a value bucket, so it takes a dedicated sequential path rather than the
        // value-partition one below.
        if is_boolean_condition(cond) {
            return arrange_hierarchy(
                build_boolean_groups(
                    rows, groups, level, group, cond, summaries, formulas, params, sink,
                ),
                group,
            );
        }
        // An unrecognized ordinal is neither a value-bucketing period nor a boolean condition, so
        // rows fall back to grouping by raw value — flag it once per level.
        if !condition_is_bucketable(cond) {
            if let Some(sink) = sink {
                sink.report(
                    EvalDiagnostic::new(
                        DiagnosticKind::UnsupportedGroupCondition,
                        format!(
                            "group condition {cond:?} is not a value-bucketing period; \
                             rows grouped by raw value"
                        ),
                    )
                    .from_source(&group.condition_field),
                );
            }
        }
    }
    // Partition rows by the group's condition-field value, preserving first-seen order.
    let mut order: Vec<String> = Vec::new();
    let mut buckets: std::collections::HashMap<String, (Value, Vec<Row>)> =
        std::collections::HashMap::new();
    for row in rows {
        let key_val = date_bucket(
            group_key(row, &group.condition_field, formulas, params, sink),
            group.date_condition,
        );
        let key_str = value_key(&key_val);
        buckets
            .entry(key_str.clone())
            .or_insert_with(|| {
                order.push(key_str.clone());
                (key_val, Vec::new())
            })
            .1
            .push(row.clone());
    }

    // Sort the group instances by the group's sort direction (on the key). `a`/`b` come from
    // `order`, which only ever holds keys inserted into `buckets`, so the lookups cannot miss.
    order.sort_by(|a, b| {
        let ka = &buckets.get(a).expect("order key is a bucket key").0;
        let kb = &buckets.get(b).expect("order key is a bucket key").0;
        let ord = compare_values(ka, kb);
        match group.sort.direction {
            SortDirection::DescendingOrder => ord.reverse(),
            _ => ord,
        }
    });

    let instances: Vec<GroupInstance> = order
        .into_iter()
        .map(|key_str| {
            let (key, bucket) = buckets
                .remove(&key_str)
                .expect("order holds only keys inserted into buckets");
            let subgroups = build_groups(
                &bucket,
                groups,
                level + 1,
                summaries,
                formulas,
                params,
                sink,
            );
            let group_summaries = summarize(&bucket, summaries, formulas, params);
            // A leaf group owns its rows outright — move the bucket in rather than clone it.
            let details = if subgroups.is_empty() {
                bucket
            } else {
                Vec::new()
            };
            GroupInstance {
                level,
                condition_field: group.condition_field.clone(),
                key,
                date_condition: group.date_condition,
                summaries: group_summaries,
                subgroups,
                details,
                hierarchy_children: Vec::new(),
            }
        })
        .collect();
    arrange_hierarchy(
        apply_group_topn(instances, group, level, summaries, formulas, params, sink),
        group,
    )
}

/// Rearrange a hierarchically grouped level's flat instance list into the parent/child tree Crystal
/// walks when *Hierarchical Group Sorting* is on, leaving any other group untouched.
///
/// Each instance's `InstanceIDField` value identifies it; its `ParentIDField` value names its parent.
/// The result is the depth-first pre-order walk from the roots — the instances whose parent value is
/// null or matches no instance — with siblings keeping the order the grouping stage already put them
/// in (the group's own sort). Children become the parent's
/// [`hierarchy_children`](crate::GroupInstance::hierarchy_children), so the engine's nesting of the
/// group's header and footer bands around a whole subtree is reproduced.
///
/// Malformed hierarchies are laid out rather than rejected: a self-parenting row and a row whose
/// parent is absent both become roots, and an instance reachable only from inside a parent cycle is
/// emitted as a root after the well-formed trees. Every instance therefore appears exactly once and
/// the walk always terminates — losing or duplicating one would silently change the record counts the
/// report prints.
fn arrange_hierarchy(instances: Vec<GroupInstance>, group: &Group) -> Vec<GroupInstance> {
    let Some(opts) = group.hierarchical_options.as_ref().filter(|o| o.enabled) else {
        return instances;
    };
    let n = instances.len();

    // Index each instance by its instance-ID value, then hang each instance off the one its
    // parent-ID names. First occurrence wins a duplicated instance ID.
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, g) in instances.iter().enumerate() {
        index
            .entry(value_key(&hierarchy_key(g, &opts.instance_id_field, true)))
            .or_insert(i);
    }
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut roots: Vec<usize> = Vec::new();
    for (i, g) in instances.iter().enumerate() {
        let parent = hierarchy_key(g, &opts.parent_id_field, false);
        match index.get(&value_key(&parent)) {
            Some(&p) if !matches!(parent, Value::Null) && p != i => children[p].push(i),
            _ => roots.push(i),
        }
    }

    let mut slots: Vec<Option<GroupInstance>> = instances.into_iter().map(Some).collect();
    let mut visited = vec![false; n];
    let mut out = Vec::with_capacity(roots.len());
    for &r in &roots {
        out.push(take_subtree(r, &mut slots, &children, &mut visited));
    }
    // Anything still unvisited sits in a parent cycle no root leads into; emit it as a root.
    for i in 0..n {
        if !visited[i] {
            out.push(take_subtree(i, &mut slots, &children, &mut visited));
        }
    }
    out
}

/// Move instance `i` and, recursively, its not-yet-emitted children out of `slots`. Marking visited
/// on entry is what stops a parent cycle from recursing forever.
fn take_subtree(
    i: usize,
    slots: &mut [Option<GroupInstance>],
    children: &[Vec<usize>],
    visited: &mut [bool],
) -> GroupInstance {
    visited[i] = true;
    let mut inst = slots[i]
        .take()
        .expect("an instance is taken once: visited is set before recursing");
    for &c in &children[i] {
        if !visited[c] {
            inst.hierarchy_children
                .push(take_subtree(c, slots, children, visited));
        }
    }
    inst
}

/// The value of a hierarchy ID field for a group instance, read from its first record. `fall_back`
/// uses the group's own key when the field is absent from the row — the instance ID is normally the
/// group's condition field, so the key already holds it.
fn hierarchy_key(g: &GroupInstance, field: &str, fall_back: bool) -> Value {
    match first_row(g).and_then(|r| r.get(field)) {
        Some(v) => v.clone(),
        None if fall_back => g.key.clone(),
        None => Value::Null,
    }
}

/// The first record of a group instance, looked up through its subgroups when it is not a leaf.
fn first_row(g: &GroupInstance) -> Option<&crate::source::Row> {
    g.details
        .first()
        .or_else(|| g.subgroups.iter().find_map(first_row))
}

/// Group `rows` for a **boolean** group condition — an order-sensitive transition / look-ahead break
/// over the ordered rows (see [`boolean_starts_new_group`]), unlike the value-partition path. Rows
/// keep their incoming order (the record sort has already run); runs are consecutive, so the group
/// instances are emitted in sequence order and are NOT re-sorted by key. Each run's `key` is the
/// boolean value of its first row (its `GroupName` operand). A null / non-boolean condition value
/// counts as `false`.
///
/// The six conditions follow the documented SDK semantics (`ISCRBooleanGroupOptions.BooleanCondition`);
/// the exact engine partition and the GroupName/summary operand are conjectural.
#[allow(clippy::too_many_arguments)]
fn build_boolean_groups(
    rows: &[Row],
    groups: &[Group],
    level: usize,
    group: &Group,
    cond: GroupCondition,
    summaries: &[SummaryDef],
    formulas: &FormulaRegistry,
    params: &Parameters,
    sink: Option<&dyn DiagnosticSink>,
) -> Vec<GroupInstance> {
    // Resolve each row's boolean condition value once (null / non-boolean → false).
    let bits: Vec<bool> = rows
        .iter()
        .map(|r| bool_of(group_key(r, &group.condition_field, formulas, params, sink)))
        .collect();

    // Cut the ordered rows into consecutive runs at each group boundary; each run carries the boolean
    // of its first row (its `GroupName` identity).
    let mut runs: Vec<(bool, Vec<Row>)> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        if i == 0 || boolean_starts_new_group(cond, &bits, i) {
            runs.push((bits[i], Vec::new()));
        }
        // A run is pushed at i == 0, so `last_mut` is always present.
        if let Some((_, run)) = runs.last_mut() {
            run.push(row.clone());
        }
    }

    runs.into_iter()
        .map(|(leading, bucket)| {
            let key = Value::Bool(leading);
            let subgroups = build_groups(
                &bucket,
                groups,
                level + 1,
                summaries,
                formulas,
                params,
                sink,
            );
            let group_summaries = summarize(&bucket, summaries, formulas, params);
            let details = if subgroups.is_empty() {
                bucket
            } else {
                Vec::new()
            };
            GroupInstance {
                level,
                condition_field: group.condition_field.clone(),
                key,
                date_condition: Some(cond),
                summaries: group_summaries,
                subgroups,
                details,
                hierarchy_children: Vec::new(),
            }
        })
        .collect()
}

/// The boolean value of a group-condition key: `true` only for `Bool(true)`; a `false`, null, or
/// non-boolean value counts as `false` (a null boolean groups with the `false` rows, matching
/// Crystal's treatment of a null boolean).
fn bool_of(v: Value) -> bool {
    matches!(v, Value::Bool(true))
}

/// Whether row `i` (`i >= 1`) begins a new group under the boolean condition `cond`, given every
/// row's resolved boolean in `bits`. Row 0 always opens the first group. The six conditions are three
/// families that differ in *which side* of the triggering row the boundary falls:
///
/// * `ToYes` / `ToNo` — a *transition* break: a new group at each `false→true` (`ToYes`) or
///   `true→false` (`ToNo`) change. A run of consecutive matching values does not re-split — only the
///   leading edge triggers. The transitioning row is the first row of the new group.
/// * `NextIsYes` / `NextIsNo` — an *open* (look-ahead) trigger: the named-value row is the **first**
///   row of a new group, so the break falls **before** it (`bits[i]` is the looked-for value). Every
///   such row opens a group, so leading rows of the opposite value attach *backward* to the previous
///   group.
/// * `EveryYes` / `EveryNo` — a *close* trigger: the named-value row is the **last** row of its group,
///   so the break falls **after** it — i.e. row `i` opens a new group when the *previous* row was the
///   named value (`bits[i - 1]`). Trailing rows of the opposite value attach *forward* to the next
///   group. This mirror of `NextIs*` is what keeps the two families distinct (they would otherwise be
///   the same partition for every input).
///
/// The transition family (`ToYes`/`ToNo`) is high-confidence: the "change to" wording is unambiguous.
/// The open-vs-close mirror for `Every*`/`NextIs*` is the best-supported inference — they must differ,
/// and this is the boundary-side that distinguishes them — but the exact phrase→side mapping is
/// conjectural.
fn boolean_starts_new_group(cond: GroupCondition, bits: &[bool], i: usize) -> bool {
    use GroupCondition::*;
    let prev = bits[i - 1];
    let cur = bits[i];
    match cond {
        ToYes => !prev && cur,
        ToNo => prev && !cur,
        // Close trigger: break *after* a named-value row → row i opens a group when row i-1 matched.
        EveryYes => prev,
        EveryNo => !prev,
        // Open trigger: break *before* a named-value row.
        NextIsYes => cur,
        NextIsNo => !cur,
        // Only boolean conditions reach here (dispatched from `build_groups`); nothing else can.
        Daily | Weekly | BiWeekly | SemiMonthly | Monthly | Quarterly | SemiAnnually | Annually
        | BySecond | ByMinute | ByHour | ByAMPM | Other(_) => false,
    }
}

/// Whether `cond` is one of the six order-sensitive boolean group conditions (as opposed to a
/// date/time period or an unknown ordinal).
fn is_boolean_condition(cond: GroupCondition) -> bool {
    use GroupCondition::*;
    matches!(
        cond,
        ToYes | ToNo | EveryYes | EveryNo | NextIsYes | NextIsNo
    )
}

/// Collapse a date/datetime group key into its period bucket per the group's
/// [`GroupCondition`], so a date group partitions rows *by period* rather than by the raw timestamp —
/// e.g. a monthly group on a `DateTime` field puts every row of a calendar month in one bucket
/// instead of one bucket per distinct timestamp. A calendar period's bucket is the period's start
/// date (the day itself, the week-start, or the first of the month), which also becomes the group's
/// `GroupName` value; a time-of-day period buckets the time component and keeps the date. A boolean
/// condition or an unrecognized ordinal is order-sensitive / not a value-bucketing period, so the key
/// passes through unchanged (rows group by raw value); `condition_is_bucketable` tells the caller
/// when to flag that. A non-date/time key or an absent condition also passes through unchanged.
pub fn date_bucket(val: Value, condition: Option<GroupCondition>) -> Value {
    use GroupCondition::*;
    let Some(cond) = condition else { return val };
    match cond {
        // Calendar periods bucket the date component (a DateTime collapses to its bucketed day).
        Daily | Weekly | BiWeekly | SemiMonthly | Monthly | Quarterly | SemiAnnually | Annually => {
            let date = match &val {
                Value::Date(d) => *d,
                Value::DateTime(d, _) => *d,
                _ => return val,
            };
            Value::Date(date_period_bucket(date, cond))
        }
        // Time-of-day periods bucket the time, keeping the date (a `DateTime` stays a `DateTime`; a
        // bare `Time` stays a `Time`). A non-time key passes through.
        BySecond | ByMinute | ByHour | ByAMPM => {
            let Some(time) = time_of(&val) else {
                return val;
            };
            let bucket = time_period_bucket(time, cond);
            match val {
                Value::DateTime(d, _) => Value::DateTime(d, bucket),
                _ => Value::Time(bucket),
            }
        }
        // Boolean transition/look-ahead grouping is order-sensitive, not a value-bucketing op, so it
        // is not reproduced here (rows partition by the raw boolean value); an unknown ordinal is
        // likewise passed through. The caller flags these via [`condition_is_bucketable`].
        ToYes | ToNo | EveryYes | EveryNo | NextIsYes | NextIsNo | Other(_) => val,
    }
}

/// Whether [`date_bucket`] actually re-buckets a key for this condition (the twelve date/time
/// periods), as opposed to passing it through (boolean / unknown conditions). Drives the pipeline's
/// [`DiagnosticKind::UnsupportedGroupCondition`] flag.
pub(super) fn condition_is_bucketable(cond: GroupCondition) -> bool {
    use GroupCondition::*;
    match cond {
        Daily | Weekly | BiWeekly | SemiMonthly | Monthly | Quarterly | SemiAnnually | Annually
        | BySecond | ByMinute | ByHour | ByAMPM => true,
        ToYes | ToNo | EveryYes | EveryNo | NextIsYes | NextIsNo | Other(_) => false,
    }
}

/// The start date of the calendar period `cond` containing `date` — also the group's `GroupName`
/// value. Only the eight calendar variants bucket; the time-of-day, boolean, and unknown variants
/// (never routed here by [`date_bucket`]) return `date` unchanged.
fn date_period_bucket(date: Date, cond: GroupCondition) -> Date {
    use GroupCondition::*;
    match cond {
        // One bucket per calendar day.
        Daily => date,
        // Week-start. `day_of_week` is 1 = Sunday, Crystal's default first day of week; a
        // locale-specific first day of week is not modelled.
        Weekly => week_start(date),
        // A two-week period, aligned on week boundaries: fortnights align to even week-indices
        // from the civil epoch (an arbitrary but stable anchor — biweekly grouping has no
        // canonical starting date).
        BiWeekly => {
            let ws = week_start(date);
            let even_week = ws.to_days().div_euclid(7).rem_euclid(2);
            Date::from_days(ws.to_days() - even_week * 7)
        }
        // Two buckets per month: the 1st (days 1–15) and the 16th (days 16–end).
        SemiMonthly => Date::new(date.year, date.month, if date.day <= 15 { 1 } else { 16 }),
        // First of the month.
        Monthly => Date::new(date.year, date.month, 1),
        // First day of the calendar quarter.
        Quarterly => Date::new(date.year, (date.month - 1) / 3 * 3 + 1, 1),
        // First day of the half-year (Jan 1 or Jul 1).
        SemiAnnually => Date::new(date.year, if date.month <= 6 { 1 } else { 7 }, 1),
        // Jan 1 of the year.
        Annually => Date::new(date.year, 1, 1),
        // Not a calendar period — never routed here by `date_bucket`; pass the date through.
        BySecond | ByMinute | ByHour | ByAMPM | ToYes | ToNo | EveryYes | EveryNo | NextIsYes
        | NextIsNo | Other(_) => date,
    }
}

/// The start of `date`'s week (Sunday, matching Crystal's default first day of week).
fn week_start(date: Date) -> Date {
    Date::from_days(date.to_days() - i64::from(date.day_of_week() - 1))
}

/// The start time of the time-of-day period `cond` containing `time`. Only the four time-of-day
/// variants bucket; every other variant (never routed here by [`date_bucket`]) returns `time`.
fn time_period_bucket(time: Time, cond: GroupCondition) -> Time {
    use GroupCondition::*;
    match cond {
        BySecond => time,
        ByMinute => Time::new(time.hour, time.minute, 0),
        ByHour => Time::new(time.hour, 0, 0),
        // Two buckets per day: AM (before noon → 00:00) and PM (noon on → 12:00).
        ByAMPM => Time::new(if time.hour < 12 { 0 } else { 12 }, 0, 0),
        // Not a time-of-day period — never routed here by `date_bucket`; pass the time through.
        Daily | Weekly | BiWeekly | SemiMonthly | Monthly | Quarterly | SemiAnnually | Annually
        | ToYes | ToNo | EveryYes | EveryNo | NextIsYes | NextIsNo | Other(_) => time,
    }
}

/// The time component of a `Time` or `DateTime` value, else `None`.
fn time_of(val: &Value) -> Option<Time> {
    match val {
        Value::Time(t) => Some(*t),
        Value::DateTime(_, t) => Some(*t),
        _ => None,
    }
}

/// The value a row groups by (a `{@formula}` condition field resolves through the registry).
fn group_key(
    row: &Row,
    field: &str,
    formulas: &FormulaRegistry,
    params: &Parameters,
    sink: Option<&dyn DiagnosticSink>,
) -> Value {
    if let Some(name) = field.strip_prefix('@') {
        let mut ctx = DataContext::new(row, formulas).with_params(params);
        if let Some(sink) = sink {
            ctx = ctx.with_diagnostics(sink);
        }
        return ctx_formula(&ctx, name);
    }
    row.get(field).cloned().unwrap_or(Value::Null)
}

pub(super) fn ctx_formula(ctx: &DataContext, name: &str) -> Value {
    use rpt_formula::eval::EvalContext;
    ctx.resolve(rpt_formula::RefKind::Formula, name)
        .unwrap_or(Value::Null)
}
