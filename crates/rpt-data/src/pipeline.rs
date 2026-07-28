//! The record pipeline: selection → sort → grouping → summaries, producing the [`Dataset`]
//! instance tree the layout engine iterates.

mod aggregate;
mod group;
mod select;
mod sort;

use crate::context::{CompiledFormula, DataContext, DateTimeSpecials, FormulaRegistry, Parameters};
use crate::diagnostics::{DiagnosticKind, DiagnosticSink, EvalDiagnostic};
use crate::source::{Row, RowSource};
use crate::Dataset;
use crystal_formula::eval::{vm, Date, Time, Value};
use crystal_formula::{parse, Syntax};
use rpt_model::{DataDefinition, FieldKindData, SortDirection};

use self::aggregate::{apply_cumulative, collect_summaries, summarize};
use self::group::build_groups;
use self::select::{apply_group_selection, selection_detail};
use self::sort::compare_sort_keys;

pub use self::group::date_bucket;

/// Build the [`Dataset`] for a report from a row source and its data definition.
///
/// The pipeline is **fail-open**: a formula or selection that errors is swallowed. Use
/// [`build_dataset_with_diagnostics`] to surface those swallowed failures. This entry point resolves
/// no report parameters — a record-selection or grouping formula that references `{?Param}` will fail
/// to resolve it; use [`build_dataset_with_params`] when the report has parameters.
pub fn build_dataset(source: &dyn RowSource, data_def: &DataDefinition) -> Dataset {
    build_dataset_inner(source, data_def, &Parameters::new(), &[], None, None)
}

/// Like [`build_dataset`], but resolves `{?Param}` references in the record-selection and grouping
/// formulas against `params`. A report whose record selection filters on a parameter (the common
/// case) needs this — without the parameter values every row's selection formula errors on the
/// unresolved reference and is dropped fail-open, yielding an empty dataset.
pub fn build_dataset_with_params(
    source: &dyn RowSource,
    data_def: &DataDefinition,
    params: &Parameters,
) -> Dataset {
    build_dataset_inner(source, data_def, params, &[], None, None)
}

/// Like [`build_dataset_with_params`], but attaches the render's as-of date/time specials so a
/// record-selection or grouping formula reading `CurrentDate`/`Today`/`CurrentDateTime`/`CurrentTime`
/// resolves against a single, caller-supplied instant (deterministic across the render).
pub fn build_dataset_with_params_at(
    source: &dyn RowSource,
    data_def: &DataDefinition,
    params: &Parameters,
    datetime: DateTimeSpecials,
) -> Dataset {
    build_dataset_inner(source, data_def, params, &[], None, Some(datetime))
}

/// Like [`build_dataset`], but reports every swallowed formula/selection failure to `sink` before
/// applying the fail-open fallback (dropping a row, keeping a group, resolving a `{@formula}` to
/// `Null`). The resulting [`Dataset`] is identical to [`build_dataset`]'s — the sink only observes.
pub fn build_dataset_with_diagnostics(
    source: &dyn RowSource,
    data_def: &DataDefinition,
    sink: &dyn DiagnosticSink,
) -> Dataset {
    build_dataset_inner(source, data_def, &Parameters::new(), &[], Some(sink), None)
}

/// A structural equality filter applied on top of the record-selection formula: keep only rows whose
/// `field` equals `value`. Used for a subreport's direct field link (`SubreportLink` with no
/// `linked_parameter`), where the parent row's value is matched against a subreport field rather than
/// routed through a parameter. Applied by value — never as synthesized formula text — so no
/// quoting/locale hazards and no re-parse per row.
#[derive(Debug, Clone)]
pub struct FieldFilter {
    /// The subreport field to match (`table.field`; short-name fallback via [`Row::get`]).
    pub field: String,
    /// The parent value the field must equal.
    pub value: Value,
}

/// Like [`build_dataset_with_params`], but additionally keeps only rows matching every [`FieldFilter`]
/// in `extra` (ANDed with the record-selection formula). This is the per-instance subreport path: the
/// enclosing row's link values are merged into `params` (parameter-routed links) and passed as `extra`
/// (direct field links), so each subreport instance renders the parent-linked subset of its rows.
pub fn build_dataset_with(
    source: &dyn RowSource,
    data_def: &DataDefinition,
    params: &Parameters,
    extra: &[FieldFilter],
) -> Dataset {
    build_dataset_inner(source, data_def, params, extra, None, None)
}

/// Like [`build_dataset_with`], but attaches the render's as-of date/time specials so a subreport's
/// record-selection/grouping formulas resolve `CurrentDate`/… against the parent render's instant.
pub fn build_dataset_with_at(
    source: &dyn RowSource,
    data_def: &DataDefinition,
    params: &Parameters,
    extra: &[FieldFilter],
    datetime: DateTimeSpecials,
) -> Dataset {
    build_dataset_inner(source, data_def, params, extra, None, Some(datetime))
}

/// Everything the record pipeline can be told, for callers that need more than one option at a time.
///
/// The narrower `build_dataset*` functions each cover one common combination; this is the general
/// form, and the only way to combine a [`DiagnosticSink`] with parameters, link filters, or an as-of
/// instant — which the render path needs all together.
#[derive(Debug, Clone, Copy, Default)]
pub struct DatasetOptions<'a> {
    /// Report parameter values, for `{?Param}` references in selection and grouping formulas.
    pub params: Option<&'a Parameters>,
    /// Structural equality filters ANDed with the record selection (the subreport link path).
    pub extra: &'a [FieldFilter],
    /// Where to report swallowed fail-open failures. `None` keeps the pipeline silent.
    pub sink: Option<&'a dyn DiagnosticSink>,
    /// The render's as-of instant for `CurrentDate`/`Today`/`CurrentDateTime`/`CurrentTime`.
    pub datetime: Option<DateTimeSpecials>,
}

/// Build a [`Dataset`] with any combination of options — the general entry point.
///
/// Prefer this over the narrower `build_dataset*` functions when more than one option is in play, in
/// particular whenever a [`DiagnosticSink`] is attached: without a sink the pipeline is silent about
/// every failure it swallows.
pub fn build_dataset_opts(
    source: &dyn RowSource,
    data_def: &DataDefinition,
    opts: DatasetOptions<'_>,
) -> Dataset {
    let empty = Parameters::new();
    build_dataset_inner(
        source,
        data_def,
        opts.params.unwrap_or(&empty),
        opts.extra,
        opts.sink,
        opts.datetime,
    )
}

fn build_dataset_inner(
    source: &dyn RowSource,
    data_def: &DataDefinition,
    params: &Parameters,
    extra: &[FieldFilter],
    sink: Option<&dyn DiagnosticSink>,
    datetime: Option<DateTimeSpecials>,
) -> Dataset {
    let formulas = match datetime {
        Some(dt) => compile_formulas_reporting(data_def, sink).with_datetime(dt),
        None => compile_formulas_reporting(data_def, sink),
    };
    let mut rows = source.rows();

    // A cell that would not parse as its declared type was silently given a different one. Reported
    // once per column (with the affected row count and an example), before anything downstream —
    // because it is the explanation for a date group that sorts alphabetically or a summary that
    // comes out zero.
    if let Some(sink) = sink {
        for c in source.coercions() {
            sink.report(
                EvalDiagnostic::new(DiagnosticKind::TypeCoercion, c.describe())
                    .from_source(&c.column),
            );
        }
    }

    // Structural link filter (subreport per-instance): keep only rows matching every parent link
    // value. ANDed with the record-selection formula below; applied by value, not re-parsed per row.
    if !extra.is_empty() {
        rows.retain(|row| extra.iter().all(|f| row.get(&f.field) == Some(&f.value)));
    }

    // 1. Record selection — keep rows whose selection formula evaluates true (absent = keep all).
    if let Some(sel) = &data_def.record_selection {
        if !sel.0.trim().is_empty() {
            let (ast, diagnostics) = parse(&sel.0, Syntax::Crystal);
            crate::diagnostics::report_parse_diagnostics(
                sink,
                "the record-selection formula",
                &sel.0,
                &diagnostics,
            );
            let chunk = vm::compile(&ast);
            // `retain` visits rows in order, so a counter is the row's 0-based index — which is what
            // tells "one bad row" apart from "this formula fails on everything".
            let mut record_index = 0u64;
            // Counted so the summary below can distinguish "the selection filtered everything out"
            // (legitimate) from "the selection could not be evaluated at all" (a broken report).
            let mut failed = 0u64;
            let offered = rows.len();
            let mut first_failure = None;
            rows.retain(|row| {
                let index = record_index;
                record_index += 1;
                let mut ctx = DataContext::new(row, &formulas).with_params(params);
                if let Some(sink) = sink {
                    ctx = ctx.with_diagnostics(sink);
                }
                match vm::run(&chunk, &ctx) {
                    Ok(Value::Bool(true)) => true,
                    // A clean `false` is ordinary filtering, not a failure — never a diagnostic.
                    Ok(Value::Bool(false)) => false,
                    // An error or a non-boolean result drops the row (fail-open); report it first.
                    other => {
                        failed += 1;
                        let detail = selection_detail(&other);
                        if first_failure.is_none() {
                            first_failure = Some(detail.clone());
                        }
                        if let Some(sink) = sink {
                            sink.report(
                                EvalDiagnostic::new(DiagnosticKind::RecordSelection, detail)
                                    .at_record(index),
                            );
                        }
                        false
                    }
                }
            });
            // The high-signal case: the selection kept nothing AND at least one row was dropped
            // because the formula *failed*, not because it cleanly returned false. That is a broken
            // report rendering as an empty one, and it is indistinguishable from "no rows matched"
            // unless said outright. Reported after the per-row diagnostics so it reads as the summary.
            if let (Some(sink), true) = (sink, rows.is_empty() && offered > 0) {
                sink.report(if failed > 0 {
                    // Failures, not filtering: the report is broken, not empty.
                    let detail = first_failure.unwrap_or_default();
                    EvalDiagnostic::new(
                        DiagnosticKind::RecordSelection,
                        format!(
                            "0 of {offered} row(s) kept: the record-selection formula FAILED on \
                             {failed} of them, first as {detail}. Check that every field and \
                             parameter it references exists — compare `rpt saved <file>` (or `rpt \
                             sql <file>`) and `rpt inputs <file>` against the formula"
                        ),
                    )
                } else {
                    // Legitimate filtering, but still the answer to "why is my report empty?" —
                    // most often an unsupplied parameter leaving the criteria matching nothing.
                    EvalDiagnostic::new(
                        DiagnosticKind::AllRowsExcluded,
                        format!(
                            "0 of {offered} row(s) kept: the record-selection formula excluded \
                             every row. If the report expects parameters, check their values (`rpt \
                             inputs <file>` lists them) — a defaulted parameter often selects nothing"
                        ),
                    )
                });
            }
        }
    }

    // Stamp read-order index (source order after selection, before sort) so the render pass can map
    // a printed record back to its read-order slot for evaluation-time scheduling.
    for (i, row) in rows.iter_mut().enumerate() {
        row.set_read_index(i as u64);
    }

    // 2. Record sort — one stable sort across all record-sort fields via a precomputed composite
    // key, so each comparison reads the decorated keys instead of cloning both Values on every
    // comparison (the previous per-field sort passes cloned O(n log n) times per field).
    if !data_def.record_sorts.is_empty() {
        let dirs: Vec<SortDirection> = data_def.record_sorts.iter().map(|s| s.direction).collect();
        let mut decorated: Vec<(Vec<Value>, Row)> = rows
            .into_iter()
            .map(|row| {
                let key: Vec<Value> = data_def
                    .record_sorts
                    .iter()
                    .map(|s| row.get(&s.field).cloned().unwrap_or(Value::Null))
                    .collect();
                (key, row)
            })
            .collect();
        // Lexicographic by field in order, each field honoring its own direction; stable on full
        // ties (preserving read order) — identical ordering to the prior per-field reverse-order
        // sort passes.
        decorated.sort_by(|(ka, _), (kb, _)| compare_sort_keys(ka, kb, &dirs));
        rows = decorated.into_iter().map(|(_, row)| row).collect();
    }

    // 3. Summaries to compute at each level (declared summary fields).
    let summary_defs = collect_summaries(data_def);

    // 4. Grouping — nest by each Group in definition order; deepest level holds the detail rows.
    let groups = &data_def.groups;
    let (tree, grand) = if groups.is_empty() {
        (
            Vec::new(),
            summarize(&rows, &summary_defs, &formulas, params),
        )
    } else {
        let mut tree = build_groups(&rows, groups, 0, &summary_defs, &formulas, params, sink);
        // No-reset running totals accumulate across the top-level groups.
        apply_cumulative(&mut tree, &summary_defs);
        // Group selection (HAVING-like): drop groups the group-selection formula rejects.
        apply_group_selection(&mut tree, data_def, sink);
        let grand = summarize(&rows, &summary_defs, &formulas, params);
        (tree, grand)
    };

    Dataset {
        columns: source.columns().to_vec(),
        row_count: rows.len(),
        details: if groups.is_empty() { rows } else { Vec::new() },
        groups: tree,
        grand_total: grand,
        params: Default::default(),
    }
}

/// Like [`compile_formulas`], but attaches the render's as-of date/time specials so every context
/// built from the registry resolves `CurrentDate`/`Today`/`CurrentDateTime`/`CurrentTime`.
pub fn compile_formulas_at(
    data_def: &DataDefinition,
    datetime: DateTimeSpecials,
) -> FormulaRegistry {
    compile_formulas(data_def).with_datetime(datetime)
}

/// Parse every formula field's body once, keyed by lowercase name.
///
/// Parse diagnostics are discarded. Use [`compile_formulas_reporting`] to see them: a formula with a
/// syntax error is compiled from the parser's partial recovery AST and evaluated anyway, so it yields
/// a meaningless value with nothing said about it.
pub fn compile_formulas(data_def: &DataDefinition) -> FormulaRegistry {
    compile_formulas_reporting(data_def, None)
}

/// Like [`compile_formulas`], but reports each formula's parse diagnostics to `sink`.
///
/// Compile time, so this is once per formula rather than once per row — cheap, and it is the class of
/// failure the report's author can fix by editing the formula.
pub fn compile_formulas_reporting(
    data_def: &DataDefinition,
    sink: Option<&dyn DiagnosticSink>,
) -> FormulaRegistry {
    let mut reg = FormulaRegistry::new();
    for f in &data_def.field_definitions {
        // Every field contributes its type default, so a formula reading a null field under
        // DefaultValue null-treatment can substitute the right default for the field's type.
        reg.set_field_default(&f.name, type_default(f.value_type));
        if let FieldKindData::Formula(ff) = &f.kind {
            let syntax = match ff.syntax {
                rpt_model::FormulaSyntax::Basic => Syntax::Basic,
                _ => Syntax::Crystal,
            };
            let (ast, diagnostics) = parse(&ff.text.0, syntax);
            crate::diagnostics::report_parse_diagnostics(
                sink,
                &format!("formula \"{}\"", f.name),
                &ff.text.0,
                &diagnostics,
            );
            reg.insert(
                f.name.to_lowercase(),
                CompiledFormula {
                    chunk: vm::compile(&ast),
                    null_treatment: null_treatment(ff.null_treatment),
                },
            );
        }
    }
    reg
}

/// Map the stored [`FormulaNullTreatment`](rpt_model::FormulaNullTreatment) onto the evaluator's
/// [`NullTreatment`](crystal_formula::NullTreatment). Any unrecognized value keeps the engine
/// default (exception on null).
fn null_treatment(t: rpt_model::FormulaNullTreatment) -> crystal_formula::NullTreatment {
    match t {
        rpt_model::FormulaNullTreatment::DefaultValue => {
            crystal_formula::NullTreatment::DefaultValue
        }
        _ => crystal_formula::NullTreatment::Exception,
    }
}

/// The value Crystal substitutes for a null field of the given type under "default values for nulls":
/// `0` for the numeric types, `""` for text, `False` for boolean, and the zero date/time serial for
/// the calendar types (`1899-12-30` / `00:00:00`).
fn type_default(value_type: rpt_model::FieldValueType) -> Value {
    use rpt_model::FieldValueType as T;
    match value_type {
        T::Int8s | T::Int16s | T::Int32s | T::Int32u | T::Number => Value::Number(0.0),
        T::Currency => Value::Currency(0.0),
        T::Boolean => Value::Bool(false),
        T::Date => Value::Date(Date::from_ole_days(0)),
        T::Time => Value::Time(Time::from_seconds(0)),
        T::DateTime => Value::DateTime(Date::from_ole_days(0), Time::from_seconds(0)),
        _ => Value::Str(String::new()),
    }
}

#[cfg(test)]
mod agg_tests {
    use super::aggregate::aggregate;
    use super::group::condition_is_bucketable;
    use super::*;
    use rpt_model::{GroupCondition, SummaryOperation};

    /// Aggregate over raw-column rows with no formulas/parameters (the summarized field is always a
    /// database column here) — the [`aggregate`] signature the summary-over-formula work extended.
    fn agg(
        rows: &[Row],
        op: SummaryOperation,
        field: &str,
        param: i32,
        secondary: Option<&str>,
    ) -> Value {
        aggregate(
            rows,
            op,
            field,
            param,
            secondary,
            &FormulaRegistry::new(),
            &Parameters::new(),
        )
    }

    fn rows(field: &str, vals: &[f64]) -> Vec<Row> {
        vals.iter()
            .map(|&n| {
                let mut r = Row::default();
                r.insert(field, Value::Number(n));
                r
            })
            .collect()
    }

    fn num(v: Value) -> f64 {
        match v {
            Value::Number(n) | Value::Currency(n) => n,
            other => panic!("expected number, got {other:?}"),
        }
    }

    #[test]
    fn sql_expression_field_is_never_compiled_as_a_crystal_formula() {
        use rpt_model::{FieldDef, FieldKindData, FormulaField, SqlExpressionField};
        // A SQL Expression field carries a SQL body (server-evaluated), NOT a Crystal formula. Its
        // text must never reach the crystal-formula parser/VM; the DB computes it and it arrives as a
        // fetched column. `compile_formulas` proves this: it registers only Formula fields.
        // A body that is valid SQL but not valid Crystal — if it were parsed as Crystal it would be
        // mangled; it must be passed through opaquely instead.
        let sql_field = FieldDef {
            name: "SqlExpr1".to_string(),
            kind: FieldKindData::SqlExpression(SqlExpressionField {
                text: "CASE WHEN amount > 0 THEN 'pos' ELSE 'neg' END".to_string(),
            }),
            ..FieldDef::default()
        };
        let formula = FieldDef {
            name: "Formula1".to_string(),
            kind: FieldKindData::Formula(FormulaField {
                text: rpt_model::Formula("1 + 1".to_string()),
                ..FormulaField::default()
            }),
            ..FieldDef::default()
        };

        let data_def = DataDefinition {
            field_definitions: vec![sql_field, formula],
            ..DataDefinition::default()
        };

        let reg = compile_formulas(&data_def);
        // The Crystal formula is compiled; the SQL Expression field is not present in the registry,
        // so it is never handed to crystal-formula.
        assert!(reg.contains_key("formula1"));
        assert!(!reg.contains_key("sqlexpr1"));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn date_group_buckets_a_datetime_key_by_period() {
        use crystal_formula::eval::Time;
        use GroupCondition::*;
        let noon = Time::new(12, 0, 0);
        let dt = |y, m, d| Value::DateTime(Date::new(y, m, d), noon);

        // Monthly: every day of a month collapses to the first of that month; distinct months differ.
        assert_eq!(
            date_bucket(dt(2024, 1, 3), Some(Monthly)),
            Value::Date(Date::new(2024, 1, 1))
        );
        assert_eq!(
            date_bucket(dt(2024, 1, 28), Some(Monthly)),
            Value::Date(Date::new(2024, 1, 1))
        );
        assert_eq!(
            date_bucket(dt(2024, 2, 1), Some(Monthly)),
            Value::Date(Date::new(2024, 2, 1))
        );

        // Daily: a DateTime collapses to its calendar day (time dropped), regardless of the time.
        assert_eq!(
            date_bucket(dt(2024, 1, 3), Some(Daily)),
            Value::Date(Date::new(2024, 1, 3))
        );
        assert_eq!(
            date_bucket(
                Value::DateTime(Date::new(2024, 1, 3), Time::new(23, 59, 59)),
                Some(Daily)
            ),
            Value::Date(Date::new(2024, 1, 3))
        );

        // Weekly: keyed by the week-start Sunday. 2024-01-03 is a Wednesday → the prior Sunday
        // 2023-12-31; the following Sunday 2024-01-07 keys itself.
        assert_eq!(
            date_bucket(dt(2024, 1, 3), Some(Weekly)),
            Value::Date(Date::new(2023, 12, 31))
        );
        assert_eq!(
            date_bucket(dt(2024, 1, 7), Some(Weekly)),
            Value::Date(Date::new(2024, 1, 7))
        );

        // No condition, or a non-date key, passes through unchanged.
        assert_eq!(date_bucket(dt(2024, 1, 3), None), dt(2024, 1, 3));
        assert_eq!(
            date_bucket(Value::Number(5.0), Some(Monthly)),
            Value::Number(5.0)
        );
    }

    #[test]
    fn date_bucket_extended_calendar_periods() {
        use GroupCondition::*;
        let d = |y, m, dd| Value::Date(Date::new(y, m, dd));
        // Semimonthly: 1st–15th → the 1st; 16th–end → the 16th.
        assert_eq!(
            date_bucket(d(2024, 3, 15), Some(SemiMonthly)),
            d(2024, 3, 1)
        );
        assert_eq!(
            date_bucket(d(2024, 3, 16), Some(SemiMonthly)),
            d(2024, 3, 16)
        );
        // Quarterly: first day of the calendar quarter.
        assert_eq!(date_bucket(d(2024, 2, 29), Some(Quarterly)), d(2024, 1, 1));
        assert_eq!(date_bucket(d(2024, 8, 10), Some(Quarterly)), d(2024, 7, 1));
        // Semiannually: Jan 1 or Jul 1.
        assert_eq!(
            date_bucket(d(2024, 6, 30), Some(SemiAnnually)),
            d(2024, 1, 1)
        );
        assert_eq!(
            date_bucket(d(2024, 7, 1), Some(SemiAnnually)),
            d(2024, 7, 1)
        );
        // Annually: Jan 1 of the year.
        assert_eq!(date_bucket(d(2024, 11, 5), Some(Annually)), d(2024, 1, 1));
        // Biweekly aligns to a week-start and is stable within a fortnight (the fortnight starting
        // Sun 2024-01-07 spans through Sat 2024-01-20; the next fortnight starts 2024-01-21).
        let bw = |v| date_bucket(v, Some(BiWeekly));
        assert_eq!(bw(d(2024, 1, 7)), bw(d(2024, 1, 20))); // same fortnight
        assert_ne!(bw(d(2024, 1, 7)), bw(d(2024, 1, 21))); // next fortnight
    }

    #[test]
    fn date_bucket_time_of_day_periods() {
        use GroupCondition::*;
        let dt = |h, mi, s| Value::DateTime(Date::new(2024, 1, 3), Time::new(h, mi, s));
        let day = Date::new(2024, 1, 3);
        // Time periods keep the date and truncate the time; a DateTime stays a DateTime.
        assert_eq!(
            date_bucket(dt(9, 41, 30), Some(ByHour)),
            Value::DateTime(day, Time::new(9, 0, 0))
        );
        assert_eq!(
            date_bucket(dt(9, 41, 30), Some(ByMinute)),
            Value::DateTime(day, Time::new(9, 41, 0))
        );
        assert_eq!(
            date_bucket(dt(9, 41, 30), Some(ByAMPM)),
            Value::DateTime(day, Time::new(0, 0, 0))
        );
        assert_eq!(
            date_bucket(dt(14, 5, 0), Some(ByAMPM)),
            Value::DateTime(day, Time::new(12, 0, 0))
        );
        // A bare Time value stays a Time.
        assert_eq!(
            date_bucket(Value::Time(Time::new(14, 5, 30)), Some(ByHour)),
            Value::Time(Time::new(14, 0, 0))
        );
    }

    #[test]
    fn boolean_and_unknown_conditions_pass_the_key_through() {
        use GroupCondition::*;
        // A boolean transition/look-ahead condition and an unknown ordinal are not value-bucketing
        // periods, so the key is returned unchanged (rows group by raw value) and the condition is
        // reported as not bucketable.
        let d = Value::DateTime(Date::new(2024, 1, 3), Time::new(9, 41, 30));
        for cond in [
            ToYes,
            ToNo,
            EveryYes,
            EveryNo,
            NextIsYes,
            NextIsNo,
            Other(99),
        ] {
            assert_eq!(date_bucket(d.clone(), Some(cond)), d, "{cond:?}");
            assert!(!condition_is_bucketable(cond), "{cond:?}");
        }
        // Every date/time period IS bucketable.
        for cond in [
            Daily,
            Weekly,
            BiWeekly,
            SemiMonthly,
            Monthly,
            Quarterly,
            SemiAnnually,
            Annually,
            BySecond,
            ByMinute,
            ByHour,
            ByAMPM,
        ] {
            assert!(condition_is_bucketable(cond), "{cond:?}");
        }
    }

    #[test]
    fn monthly_group_buckets_thirty_timestamps_into_six_groups() {
        use crate::source::RowData;
        use crystal_formula::eval::Time;
        use rpt_model::{Group, Sort};
        // Thirty distinct timestamps across six calendar months (five per month), like the parking
        // `orders.created_at` seed. A monthly group must collapse them into six buckets, not thirty.
        let mut rows: Vec<Row> = Vec::new();
        for month in 1..=6u8 {
            for day in 1..=5u8 {
                let mut r = Row::default();
                r.insert(
                    "orders.created_at",
                    Value::DateTime(Date::new(2024, month, day), Time::new(day, 0, 0)),
                );
                rows.push(r);
            }
        }
        assert_eq!(rows.len(), 30);

        let data_def = DataDefinition {
            groups: vec![Group {
                condition_field: "orders.created_at".into(),
                sort: Sort::default(),
                date_condition: Some(GroupCondition::Monthly),
                ..Group::default()
            }],
            ..DataDefinition::default()
        };
        let dataset = build_dataset(&RowData::new(Vec::new(), rows), &data_def);
        assert_eq!(
            dataset.groups.len(),
            6,
            "30 timestamps across 6 months must bucket into 6 monthly groups"
        );
    }

    /// Group an ordered boolean sequence by a boolean group condition, returning `(group sizes, group
    /// leading-key booleans)` — a compact view of the sequential partition and each run's `GroupName`.
    fn bool_group(seq: &[bool], cond: GroupCondition) -> (Vec<usize>, Vec<bool>) {
        use crate::source::RowData;
        use rpt_model::{Group, Sort};
        let rows: Vec<Row> = seq
            .iter()
            .map(|&b| {
                let mut r = Row::default();
                r.insert("flag", Value::Bool(b));
                r
            })
            .collect();
        let data_def = DataDefinition {
            groups: vec![Group {
                condition_field: "flag".into(),
                sort: Sort::default(),
                date_condition: Some(cond),
                ..Group::default()
            }],
            ..DataDefinition::default()
        };
        let ds = build_dataset(&RowData::new(Vec::new(), rows), &data_def);
        let sizes = ds.groups.iter().map(|g| g.details.len()).collect();
        let keys = ds
            .groups
            .iter()
            .map(|g| matches!(g.key, Value::Bool(true)))
            .collect();
        (sizes, keys)
    }

    #[test]
    fn boolean_group_to_yes_breaks_on_false_to_true() {
        use GroupCondition::*;
        // [T,F,F,T,T,F]: ToYes breaks at the single false->true edge (before index 3).
        let seq = [true, false, false, true, true, false];
        assert_eq!(bool_group(&seq, ToYes), (vec![3, 3], vec![true, true]));
        // Leading run before the first True still forms its own group.
        assert_eq!(
            bool_group(&[false, false, true, false], ToYes),
            (vec![2, 2], vec![false, true])
        );
    }

    #[test]
    fn boolean_group_to_no_breaks_on_true_to_false() {
        use GroupCondition::*;
        // [T,F,F,T,T,F]: ToNo breaks at each true->false edge (before index 1 and index 5).
        let seq = [true, false, false, true, true, false];
        assert_eq!(
            bool_group(&seq, ToNo),
            (vec![1, 4, 1], vec![true, false, false])
        );
    }

    #[test]
    fn boolean_group_every_yes_no_are_close_triggers() {
        use GroupCondition::*;
        // Close trigger: the named-value row is the LAST of its group (break after it). Worked example
        // [T,F,F,T,T,F]: EveryYes -> {1}{2,3,4}{5}{6}, EveryNo -> {1,2}{3}{4,5,6}.
        let seq = [true, false, false, true, true, false];
        assert_eq!(
            bool_group(&seq, EveryYes),
            (vec![1, 3, 1, 1], vec![true, false, true, false])
        );
        assert_eq!(
            bool_group(&seq, EveryNo),
            (vec![2, 1, 3], vec![true, false, true])
        );
    }

    #[test]
    fn boolean_group_next_is_yes_no_are_open_triggers() {
        use GroupCondition::*;
        // Open trigger: the named-value row is the FIRST of a new group (break before it). Worked
        // example [T,F,F,T,T,F]: NextIsYes -> {1,2,3}{4}{5,6}, NextIsNo -> {1}{2}{3,4,5}{6}.
        // Distinct from Every* (the mirror-image boundary side) — verifies the two families differ.
        let seq = [true, false, false, true, true, false];
        assert_eq!(
            bool_group(&seq, NextIsYes),
            (vec![3, 1, 2], vec![true, true, true])
        );
        assert_eq!(
            bool_group(&seq, NextIsNo),
            (vec![1, 1, 3, 1], vec![true, false, false, false])
        );
        // The two families are genuinely different partitions, not relabelings of one.
        assert_ne!(bool_group(&seq, NextIsYes), bool_group(&seq, EveryYes));
        assert_ne!(bool_group(&seq, NextIsNo), bool_group(&seq, EveryNo));
    }

    #[test]
    fn boolean_group_single_value_and_all_same() {
        use GroupCondition::*;
        // A single row is one group regardless of condition.
        assert_eq!(bool_group(&[true], ToYes), (vec![1], vec![true]));
        // All-True under ToYes never sees a false->true edge → one group.
        assert_eq!(
            bool_group(&[true, true, true], ToYes),
            (vec![3], vec![true])
        );
        // All-True under EveryYes → one group per row.
        assert_eq!(
            bool_group(&[true, true, true], EveryYes),
            (vec![1, 1, 1], vec![true, true, true])
        );
    }

    #[test]
    fn variance_and_stddev_sample_vs_population() {
        // {2,4,4,4,5,5,7,9}: pop variance 4, sample variance 32/7.
        let rs = rows("x", &[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
        let pv = num(agg(&rs, SummaryOperation::PopVariance, "x", 0, None));
        assert!((pv - 4.0).abs() < 1e-9, "pop variance {pv}");
        let ps = num(agg(
            &rs,
            SummaryOperation::PopStandardDeviation,
            "x",
            0,
            None,
        ));
        assert!((ps - 2.0).abs() < 1e-9, "pop stddev {ps}");
        let sv = num(agg(&rs, SummaryOperation::SampleVariance, "x", 0, None));
        assert!((sv - 32.0 / 7.0).abs() < 1e-9, "sample variance {sv}");
        let ss = num(agg(
            &rs,
            SummaryOperation::SampleStandardDeviation,
            "x",
            0,
            None,
        ));
        assert!(
            (ss - (32.0f64 / 7.0).sqrt()).abs() < 1e-9,
            "sample stddev {ss}"
        );
    }

    #[test]
    fn sample_variance_needs_two_values() {
        let rs = rows("x", &[5.0]);
        assert_eq!(
            agg(&rs, SummaryOperation::SampleVariance, "x", 0, None),
            Value::Null
        );
        // Population variance of one value is 0.
        assert_eq!(
            num(agg(&rs, SummaryOperation::PopVariance, "x", 0, None)),
            0.0
        );
    }

    #[test]
    fn median_odd_and_even() {
        assert_eq!(
            num(agg(
                &rows("x", &[3.0, 1.0, 2.0]),
                SummaryOperation::Median,
                "x",
                0,
                None
            )),
            2.0
        );
        // Even count → mean of the two middle values (2 and 3 → 2.5).
        assert_eq!(
            num(agg(
                &rows("x", &[1.0, 2.0, 3.0, 4.0]),
                SummaryOperation::Median,
                "x",
                0,
                None
            )),
            2.5
        );
    }

    #[test]
    fn percentile_interpolates() {
        let rs = rows("x", &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(
            num(agg(&rs, SummaryOperation::Percentile, "x", 0, None)),
            1.0
        );
        assert_eq!(
            num(agg(&rs, SummaryOperation::Percentile, "x", 100, None)),
            4.0
        );
        assert_eq!(
            num(agg(&rs, SummaryOperation::Percentile, "x", 50, None)),
            2.5
        );
    }

    #[test]
    fn nth_largest_smallest_are_one_based() {
        let rs = rows("x", &[10.0, 30.0, 20.0, 40.0]);
        assert_eq!(
            num(agg(&rs, SummaryOperation::NthLargest, "x", 1, None)),
            40.0
        );
        assert_eq!(
            num(agg(&rs, SummaryOperation::NthLargest, "x", 2, None)),
            30.0
        );
        assert_eq!(
            num(agg(&rs, SummaryOperation::NthSmallest, "x", 1, None)),
            10.0
        );
        assert_eq!(
            num(agg(&rs, SummaryOperation::NthSmallest, "x", 2, None)),
            20.0
        );
        // Out of range → Null.
        assert_eq!(
            agg(&rs, SummaryOperation::NthLargest, "x", 9, None),
            Value::Null
        );
    }

    #[test]
    fn mode_and_nth_most_frequent() {
        // 20 appears 3x, 10 twice, 30 once.
        let rs = rows("x", &[10.0, 20.0, 20.0, 30.0, 10.0, 20.0]);
        assert_eq!(num(agg(&rs, SummaryOperation::Mode, "x", 0, None)), 20.0);
        assert_eq!(
            num(agg(&rs, SummaryOperation::NthMostFrequent, "x", 2, None)),
            10.0
        );
        assert_eq!(
            num(agg(&rs, SummaryOperation::NthMostFrequent, "x", 3, None)),
            30.0
        );
    }

    #[test]
    fn two_field_ops_without_secondary_are_null() {
        // WeightedAvg / Correlation / Covariance need a second field. With no `secondary` they
        // aggregate to Null (unavailable) — never a plausible-but-wrong number. WeightedAvg in
        // particular must NOT silently return the plain Average (here 2.0) of the single field.
        let rs = rows("x", &[1.0, 2.0, 3.0]);
        assert_eq!(
            agg(&rs, SummaryOperation::Average, "x", 0, None),
            Value::Number(2.0)
        );
        for op in [
            SummaryOperation::WeightedAvg,
            SummaryOperation::Correlation,
            SummaryOperation::Covariance,
        ] {
            assert_eq!(agg(&rs, op, "x", 0, None), Value::Null, "{op:?}");
        }
    }

    /// Rows carrying a paired (x, y) numeric sample in two columns.
    fn xy_rows(x: &str, y: &str, pairs: &[(f64, f64)]) -> Vec<Row> {
        pairs
            .iter()
            .map(|&(xv, yv)| {
                let mut r = Row::default();
                r.insert(x, Value::Number(xv));
                r.insert(y, Value::Number(yv));
                r
            })
            .collect()
    }

    #[test]
    fn weighted_average_is_sum_xw_over_sum_w() {
        // WeightedAvg(value X, weight W) = Σ(Xi·Wi)/Σ(Wi).
        // (10·1 + 20·2 + 30·3)/(1+2+3) = (10+40+90)/6 = 140/6 = 23.333…
        let rs = xy_rows("x", "w", &[(10.0, 1.0), (20.0, 2.0), (30.0, 3.0)]);
        let got = num(agg(&rs, SummaryOperation::WeightedAvg, "x", 0, Some("w")));
        assert!((got - 140.0 / 6.0).abs() < 1e-9, "weighted avg {got}");
        // All weights equal → the plain mean (20).
        let eq = xy_rows("x", "w", &[(10.0, 2.0), (20.0, 2.0), (30.0, 2.0)]);
        let m = num(agg(&eq, SummaryOperation::WeightedAvg, "x", 0, Some("w")));
        assert!((m - 20.0).abs() < 1e-9, "equal weights {m}");
        // Zero total weight → Null (no division by zero).
        let zero = xy_rows("x", "w", &[(10.0, 0.0), (20.0, 0.0)]);
        assert_eq!(
            agg(&zero, SummaryOperation::WeightedAvg, "x", 0, Some("w")),
            Value::Null
        );
    }

    #[test]
    fn covariance_uses_sample_divisor() {
        // X={1,2,3,4}, Y={2,4,6,8}=2X. Deviations: X−X̄ = {−1.5,−0.5,0.5,1.5}, Y−Ȳ = 2× that.
        // Σ cross = 2·(1.5²+0.5²+0.5²+1.5²) = 2·5 = 10. Sample cov = 10/(4−1) = 3.3333…
        let rs = xy_rows("x", "y", &[(1.0, 2.0), (2.0, 4.0), (3.0, 6.0), (4.0, 8.0)]);
        let cov = num(agg(&rs, SummaryOperation::Covariance, "x", 0, Some("y")));
        assert!((cov - 10.0 / 3.0).abs() < 1e-9, "sample covariance {cov}");
        // Fewer than two pairs → Null (sample form undefined).
        let one = xy_rows("x", "y", &[(1.0, 2.0)]);
        assert_eq!(
            agg(&one, SummaryOperation::Covariance, "x", 0, Some("y")),
            Value::Null
        );
    }

    #[test]
    fn correlation_is_one_for_a_perfect_positive_line_and_invariant_to_divisor() {
        // Y = 2X + 0 (perfect positive linear relation) → Pearson r = 1, regardless of the n vs n−1
        // choice (the divisor cancels in the correlation formula).
        let rs = xy_rows("x", "y", &[(1.0, 2.0), (2.0, 4.0), (3.0, 6.0), (4.0, 8.0)]);
        let r = num(agg(&rs, SummaryOperation::Correlation, "x", 0, Some("y")));
        assert!((r - 1.0).abs() < 1e-9, "correlation {r}");
        // Perfect negative line → −1.
        let neg = xy_rows("x", "y", &[(1.0, 8.0), (2.0, 6.0), (3.0, 4.0), (4.0, 2.0)]);
        let rn = num(agg(&neg, SummaryOperation::Correlation, "x", 0, Some("y")));
        assert!((rn + 1.0).abs() < 1e-9, "neg correlation {rn}");
        // A constant field has zero variance → Null (undefined correlation, no NaN).
        let flat = xy_rows("x", "y", &[(1.0, 5.0), (2.0, 5.0), (3.0, 5.0)]);
        assert_eq!(
            agg(&flat, SummaryOperation::Correlation, "x", 0, Some("y")),
            Value::Null
        );
    }

    #[test]
    fn correlation_matches_a_hand_checked_non_trivial_case() {
        // X={1,2,3,4,5}, Y={2,4,5,4,5}. X̄=3, Ȳ=4. Σ(dx·dy)=(−2)(−2)+(−1)(0)+0+1·0+2·1=4+0+0+0+2=6.
        // Σdx²=4+1+0+1+4=10, Σdy²=4+0+1+0+1=6. r = 6/√(10·6) = 6/√60 = 0.774596…
        let rs = xy_rows(
            "x",
            "y",
            &[(1.0, 2.0), (2.0, 4.0), (3.0, 5.0), (4.0, 4.0), (5.0, 5.0)],
        );
        let r = num(agg(&rs, SummaryOperation::Correlation, "x", 0, Some("y")));
        assert!((r - 6.0 / 60.0_f64.sqrt()).abs() < 1e-9, "correlation {r}");
    }
}
