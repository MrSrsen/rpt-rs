//! `ParameterFieldUsage` derivation: the `InUse` / `DataFetching` flags for each parameter, from the
//! parameters the report's query and bodies reference (transitively through formulas).

use super::add_formula_mentions;
use super::font_color_of;
use super::formula;
use super::ParameterUsage;
use rpt::model::{FieldKindData, ParameterType, Report, ReportObjectKind, SubreportLink};
use std::collections::{HashMap, HashSet};

/// Derive the `InUse` / `DataFetching` usage flags for each parameter defined at this report level.
/// `incoming_links` are the subreport links that target this level: a parameter fed by a link (the
/// engine auto-names it `Pm-` + the linked main-report field) is `InUse` even if this level's own
/// formulas/query never reference it.
pub fn parameter_usage(
    report: &Report,
    incoming_links: &[SubreportLink],
) -> HashMap<String, ParameterUsage> {
    let (query, all) = parameter_reference_text(report);
    let linked: HashSet<String> = incoming_links
        .iter()
        .map(|l| format!("Pm-{}", l.main_report_field))
        .collect();
    let mut out = HashMap::new();
    for fd in &report.data_definition.field_definitions {
        if let FieldKindData::Parameter(p) = &fd.kind {
            let data_fetching = p.parameter_type == ParameterType::StoreProcedureParameter
                || query.contains(&fd.name);
            let in_use = data_fetching || all.contains(&fd.name) || linked.contains(&fd.name);
            out.insert(
                fd.name.clone(),
                ParameterUsage {
                    in_use,
                    data_fetching,
                },
            );
        }
    }
    out
}

/// Parameter usage is a derived aggregation (the engine computes it; it is not stored in the file).
/// Returns `(query_params, all_params)` as sets of parameter **names**: `query_params` are referenced
/// by the data query (table commands + record/group selection, plus formulas the query names) — these
/// drive `DataFetching`; `all_params` adds every formula body, field-object data source, embedded text
/// field and conditional-format formula — a parameter named anywhere is `InUse`. Reference extraction
/// is tokenizer-driven (see [`formula::refs::parameter_names`] / [`formula::refs::formula_names`]).
fn parameter_reference_text(report: &Report) -> (HashSet<String>, HashSet<String>) {
    let mut query: HashSet<String> = HashSet::new();
    let mut all: HashSet<String> = HashSet::new();
    // Formula names the data query references — seeds the transitive expansion below.
    let mut query_formula_mentions: HashSet<String> = HashSet::new();
    collect_param_refs(report, &mut query, &mut all, &mut query_formula_mentions);
    for s in &report.subreports {
        collect_param_refs(&s.report, &mut query, &mut all, &mut query_formula_mentions);
    }
    // A parameter referenced inside a formula `{@f}` the query uses also drives `DataFetching`;
    // expand with the parameters of formulas the query (transitively) names.
    expand_formula_refs(report, &query_formula_mentions, &mut query);
    // A query parameter is, by definition, also referenced — so it is `InUse`.
    for p in &query {
        all.insert(p.clone());
    }
    (query, all)
}

/// Add to `query` the parameters of every formula the query (transitively) names via `{@name}`,
/// starting from `initial_mentions` (formula names the query corpus references). A deterministic
/// transitive closure: each formula expands at most once, iterating to a fixpoint.
fn expand_formula_refs(
    report: &Report,
    initial_mentions: &HashSet<String>,
    query: &mut HashSet<String>,
) {
    let formulas: Vec<(&str, &str)> = std::iter::once(report)
        .chain(report.subreports.iter().map(|s| s.report.as_ref()))
        .flat_map(|r| &r.data_definition.field_definitions)
        .filter_map(|fd| match &fd.kind {
            FieldKindData::Formula(ff) => Some((fd.name.as_str(), ff.text.0.as_str())),
            _ => None,
        })
        .collect();
    let mut mentioned = initial_mentions.clone();
    let mut expanded: HashSet<&str> = HashSet::new();
    loop {
        let mut changed = false;
        for (name, body) in &formulas {
            if mentioned.contains(*name) && !expanded.contains(name) {
                expanded.insert(name);
                add_param_refs(body, query);
                add_formula_mentions(body, &mut mentioned);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

fn collect_param_refs(
    r: &Report,
    query: &mut HashSet<String>,
    all: &mut HashSet<String>,
    query_formula_mentions: &mut HashSet<String>,
) {
    for t in &r.database.tables {
        if let Some(c) = &t.command_text {
            add_param_refs(c, query);
            add_formula_mentions(c, query_formula_mentions);
        }
    }
    for f in [
        &r.data_definition.record_selection,
        &r.data_definition.group_selection,
    ]
    .into_iter()
    .flatten()
    {
        add_param_refs(&f.0, query);
        add_formula_mentions(&f.0, query_formula_mentions);
    }
    for fd in &r.data_definition.field_definitions {
        if let FieldKindData::Formula(ff) = &fd.kind {
            add_param_refs(&ff.text.0, all);
        }
    }
    for body in &r.data_definition.condition_formula_bodies {
        add_param_refs(body, all);
    }
    for area in &r.report_definition.areas {
        for sec in &area.sections {
            for (_, body) in &sec.condition_formulas {
                add_param_refs(body, all);
            }
            for obj in &sec.objects {
                match &obj.kind {
                    ReportObjectKind::Field(f) => add_param_refs(&f.data_source, all),
                    ReportObjectKind::Text(t) => {
                        add_param_refs(&t.text, all);
                        for ef in &t.embedded_fields {
                            // Embedded field refs are stored brace-less (e.g. `?param`); wrap them so
                            // the tokenizer recognises the `{?name}` reference form.
                            add_param_refs(&format!("{{{ef}}}"), all);
                        }
                    }
                    _ => {}
                }
                for (_, body) in &obj.format.condition_formulas {
                    add_param_refs(body, all);
                }
                if let Some(fc) = font_color_of(obj) {
                    for (_, body) in &fc.condition_formulas {
                        add_param_refs(body, all);
                    }
                }
            }
        }
    }
}

/// Add every parameter (`{?name}`) that `body` references to `set` (tokenizer-driven).
fn add_param_refs(body: &str, set: &mut HashSet<String>) {
    for n in formula::refs::parameter_names(body) {
        set.insert(n);
    }
}
