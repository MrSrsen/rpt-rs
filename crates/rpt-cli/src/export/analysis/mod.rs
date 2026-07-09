//! Derived report analytics computed on top of the decoded [`rpt`] model.
//!
//! Values here are not stored in the `.rpt` file; they are computed by walking the decoded model the
//! way the Crystal engine's object model exposes them — field use counts and parameter usage flags.
//!
//! The single entry point is [`analyze`], which returns a [`ReportAnalysis`] for one report level.
//! A main report aggregates references from its subreports (a field/parameter used only inside a
//! subreport still counts against the main report); a subreport covers itself and its own nested
//! subreports.

pub(crate) mod field_format;

mod parameter_usage;
mod summary_fields;
mod usecount;

pub(crate) use parameter_usage::parameter_usage;
pub(crate) use summary_fields::summary_fields;
pub(crate) use usecount::field_use_counts;

/// Alias for the standalone [`crystal_formula`] crate — the formula language has no dependency on the
/// `rpt` decode crate.
pub(crate) use crystal_formula as formula;

use std::collections::{HashMap, HashSet};

use rpt::model::{FieldKindData, FontColor, Report, ReportObject, ReportObjectKind, SubreportLink};

/// Derived analytics for one report level.
#[derive(Debug, Clone, Default)]
pub(crate) struct ReportAnalysis {
    /// `UseCount` per database-field reference key (`"{table.field}"`), matching the engine's
    /// `IField.UseCount`.
    pub field_use_counts: HashMap<String, i32>,
    /// Usage flags per parameter name (the engine's `ParameterFieldUsage`).
    pub parameter_usage: HashMap<String, ParameterUsage>,
    /// The deduplicated, ordered `<SummaryFields>` list for this report level.
    pub summary_fields: Vec<SummaryFieldDef>,
    /// Names of formulas the engine fails to compile at load because they reference an undefined
    /// `{?parameter}` or `{@formula}` — such a formula is reported as `UnknownField`/0. (Unresolved
    /// `{field}` refs and `GroupName()`-without-groups are already resolved to `Unknown`/0 by the
    /// `rpt` decoder; this covers the param/formula-reference case it cannot see, since parameters
    /// are decoded after the formulas.) See [`stale_formulas`].
    pub stale_formulas: HashSet<String>,
}

/// One entry of the derived `<SummaryFields>` list — a placed summary field (`ISummaryField`),
/// reconstructed from its field object and `0x7e` definition. See [`mod@summary_fields`].
#[derive(Debug, Clone)]
pub(crate) struct SummaryFieldDef {
    /// `FormulaName`/`Name`: the rendered summary expression, e.g. `Sum ({t.f}, {t.g})`.
    pub formula_name: String,
    /// `Operation`: Sum / Count / DistinctCount / Minimum / Maximum / Average.
    pub operation: String,
    /// `ValueType`, e.g. `CurrencyField` (the result type, read from the definition).
    pub value_type: String,
    /// `NumberOfBytes`: intrinsic size of the result type (a string result uses the summarized
    /// field's declared length).
    pub number_of_bytes: i32,
    /// `SummarizedField`: the .NET type name of the summarized field object.
    pub summarized_field_type: String,
    /// Whether the summary is group-scoped (emits the `Group` attribute) vs a grand total.
    pub grouped: bool,
    /// Whether the summary object has a name. Cross-tab cell summaries have none: they still count
    /// toward `UseCount` but are owned by the `CrossTabObject`, so they are not emitted as a
    /// report-level `<SummaryFieldDefinition>`.
    pub named: bool,
}

/// The engine's derived `ParameterFieldUsage` flags for one parameter.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ParameterUsage {
    /// Referenced anywhere in the report (`InUse` vs `NotInUse`).
    pub in_use: bool,
    /// Feeds the data query — a table command or the record/group selection (`DataFetching`).
    pub data_fetching: bool,
}

/// Compute the derived analytics for a report level. `incoming_links` are the subreport links that
/// target this report level (empty for the main report); they mark link-fed parameters as `InUse`.
pub(crate) fn analyze(report: &Report, incoming_links: &[SubreportLink]) -> ReportAnalysis {
    ReportAnalysis {
        field_use_counts: field_use_counts(report, incoming_links),
        parameter_usage: parameter_usage(report, incoming_links),
        // Emit only named summaries; cross-tab cell summaries are owned by the CrossTabObject (they
        // still count toward UseCount via `summary_fields` directly in `count_report`).
        summary_fields: summary_fields(report)
            .into_iter()
            .filter(|s| s.named)
            .collect(),
        stale_formulas: stale_formulas(report),
    }
}

/// The formula names at this report level that the engine reports as `UnknownField`/0 because they
/// reference an **undefined** `{?parameter}` or `{@formula}` — a load-time compile failure.
///
/// The engine recompiles every formula when the report loads; a reference to a name that no longer
/// exists (a parameter or formula deleted after the formula was written) fails to type-check, so the
/// engine reports the formula as `UnknownField` with `NumberOfBytes` 0, overriding the type/length
/// still persisted in the `0x71` record. The `rpt` decoder already handles the *field*-reference and
/// `GroupName()`-without-groups cases; it cannot see this one because parameters are decoded from
/// `PromptManager` *after* the formulas. References are extracted with the tokenizer
/// ([`formula::references`]) so a name inside a comment or string literal is correctly ignored.
pub(crate) fn stale_formulas(report: &Report) -> HashSet<String> {
    let defined_params: HashSet<String> = report
        .data_definition
        .field_definitions
        .iter()
        .filter(|fd| matches!(fd.kind, FieldKindData::Parameter(_)))
        .map(|fd| fd.name.to_ascii_lowercase())
        .collect();
    let defined_formulas: HashSet<String> = report
        .data_definition
        .field_definitions
        .iter()
        .filter(|fd| matches!(fd.kind, FieldKindData::Formula(_)))
        .map(|fd| fd.name.to_ascii_lowercase())
        .collect();

    let mut stale = HashSet::new();
    for fd in &report.data_definition.field_definitions {
        let FieldKindData::Formula(ff) = &fd.kind else {
            continue;
        };
        for r in formula::references(&ff.text.0) {
            let unresolved = match r.kind {
                formula::RefKind::Parameter => {
                    !defined_params.contains(&r.name.to_ascii_lowercase())
                }
                formula::RefKind::Formula => {
                    !defined_formulas.contains(&r.name.to_ascii_lowercase())
                }
                _ => false,
            };
            if unresolved {
                stale.insert(fd.name.clone());
                break;
            }
        }
    }
    stale
}

/// Add every formula (`{@name}`) that `body` references to `set`. Routed through the tokenizer, so a
/// `{@name}` inside a `//` comment or a string literal is correctly NOT treated as a mention.
pub(crate) fn add_formula_mentions(body: &str, set: &mut HashSet<String>) {
    for n in formula::refs::formula_names(body) {
        set.insert(n);
    }
}

/// The font colour of a text / field / heading object (other object kinds have none).
pub(crate) fn font_color_of(obj: &ReportObject) -> Option<&FontColor> {
    match &obj.kind {
        ReportObjectKind::Text(t) => Some(&t.font_color),
        ReportObjectKind::Field(f) => Some(&f.font_color),
        ReportObjectKind::FieldHeading(h) => Some(&h.font_color),
        _ => None,
    }
}
