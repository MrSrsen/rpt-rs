//! Formula **evaluator** — a bytecode VM ([`vm`]) over the parsed [`Node`] AST, with a tree-walking
//! reference implementation for differential testing.
//!
//! Design: the native engine compiles formulas to bytecode and caches values per evaluation context;
//! the VM is the equivalent, and its shared value semantics (the `ops` module) are reused by a tree-walking
//! `Evaluator` so both agree byte for byte. Values are pulled through [`EvalContext`], which is where
//! record/page state and cross-formula resolution live. Evaluation is single-formula and stateless
//! across calls except for its variable store.
//!
//! Unimplemented builtins and constructs fail loudly ([`EvalError::Unsupported`]) — never
//! silently wrong. Null propagates through operators and most builtins (the engine's
//! "convert null values" report options are a later concern, handled by the caller/context).

mod builtins;
mod context;
mod lazy;
mod ops;
#[cfg(any(test, feature = "differential"))]
mod tree;
mod value;
pub mod vm;

pub use builtins::{is_print_state_special, is_record_nav};
pub use context::{
    EmptyContext, EvalContext, EvalError, MapContext, NullTreatment, SpannedEvalError,
};
#[cfg(any(test, feature = "differential"))]
pub use tree::Evaluator;
pub use value::{format_number, Date, Time, Value};

// Shared value semantics reachable at `crate::eval::…` for the builtins and parser.
pub(crate) use ops::{compare, parse_date_literal};

use crate::ast::Node;

/// Evaluate a parsed formula against a context by compiling it to bytecode and running it on the
/// [`vm`]. This is the sole production path; the tree-walking `Evaluator` remains as the
/// differential-test reference (gated behind `cfg(test)` / the `differential` feature). For a
/// formula evaluated many times (once per row), compile once with
/// [`vm::compile`] and reuse the [`vm::Chunk`] rather than calling this per evaluation.
///
/// # Errors
///
/// [`EvalError`] if the formula references an unknown name, applies an operator or builtin to the
/// wrong type, divides by zero, calls a builtin with bad arguments, uses something unimplemented, or
/// trips an internal invariant ([`EvalError::Internal`]).
pub fn eval(node: &Node, ctx: &dyn EvalContext) -> Result<Value, EvalError> {
    vm::run(&vm::compile(node), ctx)
}

/// [`eval`] that, on failure, reports the source [`Span`](crate::token::Span) of the sub-expression
/// that raised the error — for an LSP/playground that wants to underline the exact failing node. The
/// success value and the underlying [`EvalError`] are identical to [`eval`]; only the span is added.
///
/// # Errors
///
/// As [`eval`], wrapped in a [`SpannedEvalError`] carrying the failing node's span.
pub fn eval_spanned(node: &Node, ctx: &dyn EvalContext) -> Result<Value, SpannedEvalError> {
    vm::run_spanned(&vm::compile(node), ctx)
}

/// [`eval`] with an explicit [`NullTreatment`] — under [`NullTreatment::DefaultValue`], a null
/// `{...}` field is replaced by [`EvalContext::null_default`] before it enters the computation.
///
/// # Errors
///
/// As [`eval`].
pub fn eval_with(
    node: &Node,
    ctx: &dyn EvalContext,
    nulls: NullTreatment,
) -> Result<Value, EvalError> {
    vm::run_with(&vm::compile(node), ctx, nulls)
}

#[cfg(test)]
mod summary_call_tests {
    //! A summary function over a `{...}` reference resolves through [`EvalContext::resolve_summary`]
    //! against the report's computed summaries, not by reducing the current row — while the
    //! array-literal form stays an ordinary inline aggregate.

    use super::{eval, EmptyContext, EvalContext, EvalError, Value};
    use crate::token::RefKind;
    use crate::{parse, Syntax};

    /// A context that answers summary functions from a fixed `(op, field, group) → value` table and
    /// resolves nothing else.
    #[derive(Default)]
    struct SummaryContext {
        entries: Vec<(String, String, Option<String>, Value)>,
    }

    impl SummaryContext {
        fn with(mut self, op: &str, field: &str, group: Option<&str>, v: Value) -> Self {
            self.entries
                .push((op.into(), field.into(), group.map(Into::into), v));
            self
        }
    }

    impl EvalContext for SummaryContext {
        fn resolve(&self, _kind: RefKind, _name: &str) -> Option<Value> {
            None
        }
        fn resolve_summary(&self, op: &str, field: &str, group: Option<&str>) -> Option<Value> {
            // Present-but-unmatched resolves to Null (the facility exists), matching the render path.
            Some(
                self.entries
                    .iter()
                    .find(|(o, f, g, _)| o == op && f == field && g.as_deref() == group)
                    .map(|(_, _, _, v)| v.clone())
                    .unwrap_or(Value::Null),
            )
        }
    }

    fn run(src: &str, ctx: &dyn EvalContext) -> Result<Value, EvalError> {
        let (ast, diags) = parse(src, Syntax::Crystal);
        assert!(diags.is_empty(), "parse diagnostics for `{src}`: {diags:?}");
        eval(&ast, ctx)
    }

    #[test]
    fn group_and_grand_summary_functions_resolve_from_context() {
        let ctx = SummaryContext::default()
            .with(
                "count",
                "shipment.id",
                Some("mode.name"),
                Value::Number(4.0),
            )
            .with("count", "shipment.id", None, Value::Number(10.0));
        // The 2-arg (group) form and the 1-arg (grand total) form pick out their own summary.
        assert_eq!(
            run("Count({shipment.id}, {mode.name})", &ctx),
            Ok(Value::Number(4.0))
        );
        assert_eq!(run("Count({shipment.id})", &ctx), Ok(Value::Number(10.0)));
        // A ModePct-style expression over the two resolves end to end.
        assert_eq!(
            run(
                "100 * Count({shipment.id}, {mode.name}) / Count({shipment.id})",
                &ctx
            ),
            Ok(Value::Number(40.0))
        );
    }

    #[test]
    fn a_formula_summarized_field_keeps_its_sigil() {
        let ctx = SummaryContext::default().with(
            "avg",
            "@TaskCost",
            Some("project.id"),
            Value::Number(69.0),
        );
        assert_eq!(
            run("Avg({@TaskCost}, {project.id})", &ctx),
            Ok(Value::Number(69.0))
        );
    }

    #[test]
    fn modepct_body_resolves() {
        let ctx = SummaryContext::default()
            .with(
                "count",
                "shipment.shipment_id",
                Some("shipment_mode.name"),
                Value::Number(4.0),
            )
            .with("count", "shipment.shipment_id", None, Value::Number(10.0));
        let out = run(
            "ToText(100 * Count ({shipment.shipment_id}, {shipment_mode.name}) / Count ({shipment.shipment_id}), 1) & \"%\"",
            &ctx,
        );
        assert_eq!(out, Ok(Value::Str("40.0%".to_string())));
    }

    #[test]
    fn array_literal_form_is_still_an_inline_aggregate() {
        // An array argument is the ordinary aggregate, unaffected by summary resolution.
        assert_eq!(run("Sum([1, 2, 3])", &EmptyContext), Ok(Value::Number(6.0)));
    }

    #[test]
    fn record_set_form_without_a_context_is_unsupported() {
        // No summary facility → the record-set form is a clean Unsupported error, not a wrong value.
        assert!(matches!(
            run("Count({shipment.id})", &EmptyContext),
            Err(EvalError::Unsupported(_))
        ));
    }
}

#[cfg(test)]
mod group_name_call_tests {
    //! `GroupName({condition field})` in a formula body names a group and resolves through
    //! [`EvalContext::group_name`] to its key — in the group field's own type, not a pre-formatted
    //! label — so the formula may format it however it likes.

    use super::{eval, EmptyContext, EvalContext, EvalError, Value};
    use crate::eval::Date;
    use crate::token::RefKind;
    use crate::{parse, Syntax};

    /// A context that answers group names from a fixed `condition field → key` table, and resolves
    /// the same fields' per-row values to something deliberately different — so a test that reads the
    /// row value instead of the group key fails loudly.
    #[derive(Default)]
    struct GroupContext {
        keys: Vec<(String, Value)>,
    }

    impl GroupContext {
        fn with(mut self, cond: &str, key: Value) -> Self {
            self.keys.push((cond.into(), key));
            self
        }
    }

    impl EvalContext for GroupContext {
        fn resolve(&self, _kind: RefKind, _name: &str) -> Option<Value> {
            Some(Value::Str("the current row, not the group key".into()))
        }
        fn group_name(&self, field: &str) -> Option<Value> {
            // Present-but-unmatched resolves to Null (the facility exists), matching the render path.
            Some(
                self.keys
                    .iter()
                    .find(|(c, _)| c == field)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Value::Null),
            )
        }
    }

    fn run(src: &str, ctx: &dyn EvalContext) -> Result<Value, EvalError> {
        let (ast, diags) = parse(src, Syntax::Crystal);
        assert!(diags.is_empty(), "parse diagnostics for `{src}`: {diags:?}");
        eval(&ast, ctx)
    }

    #[test]
    fn a_group_name_resolves_to_its_group_key() {
        let ctx =
            GroupContext::default().with("city.structured_on", Value::Str("2019-01-01".into()));
        assert_eq!(
            run("GroupName({city.structured_on})", &ctx),
            Ok(Value::Str("2019-01-01".into()))
        );
    }

    #[test]
    fn a_string_group_key_formats_through_the_ordinary_conversions() {
        // The shape a report uses to print a group header its own way: parse the key, format it.
        let ctx =
            GroupContext::default().with("city.structured_on", Value::Str("2019-01-01".into()));
        assert_eq!(
            run(
                r#"ToText(CDate(GroupName({city.structured_on})), "yyyy, MMMM dd")"#,
                &ctx
            ),
            Ok(Value::Str("2019, January 01".into()))
        );
    }

    #[test]
    fn the_key_keeps_the_group_field_s_own_type() {
        // A date group's key arrives as a Date, so a date builtin applies to it directly.
        let ctx =
            GroupContext::default().with("order.placed_on", Value::Date(Date::new(2019, 3, 25)));
        assert_eq!(
            run("Year(GroupName({order.placed_on}))", &ctx),
            Ok(Value::Number(2019.0))
        );
    }

    #[test]
    fn the_named_group_need_not_be_the_nearest_one() {
        let ctx = GroupContext::default()
            .with("customer.region", Value::Str("West".into()))
            .with("order.year", Value::Number(2019.0));
        assert_eq!(
            run("GroupName({customer.region})", &ctx),
            Ok(Value::Str("West".into()))
        );
        assert_eq!(
            run("GroupName({order.year})", &ctx),
            Ok(Value::Number(2019.0))
        );
    }

    #[test]
    fn a_period_operand_is_ignored_in_favour_of_the_bucketed_key() {
        // The grouping pass has already bucketed the key to the period, so the second operand adds
        // nothing to resolve by.
        let ctx =
            GroupContext::default().with("order.placed_on", Value::Date(Date::new(2019, 1, 1)));
        assert_eq!(
            run(r#"GroupName({order.placed_on}, "monthly")"#, &ctx),
            Ok(Value::Date(Date::new(2019, 1, 1)))
        );
    }

    #[test]
    fn an_unmatched_condition_field_is_null_not_an_error() {
        let ctx = GroupContext::default().with("customer.region", Value::Str("West".into()));
        assert_eq!(run("GroupName({no.such_field})", &ctx), Ok(Value::Null));
    }

    #[test]
    fn without_a_group_facility_it_is_unsupported() {
        // No grouping → a clean Unsupported error rather than the row's own value for that field.
        assert!(matches!(
            run("GroupName({city.structured_on})", &EmptyContext),
            Err(EvalError::Unsupported(_))
        ));
    }
}

#[cfg(test)]
mod currency_tests {
    //! Currency is a full-precision `f64` with no value snap — number and currency share the
    //! engine's arithmetic substrate; the 2-decimal behaviour is display-only, not a snap-on-assignment
    //! as the naive model would suggest.

    use super::{eval, EmptyContext, Value};
    use crate::{parse, Syntax};

    fn cur(src: &str) -> f64 {
        let (ast, diags) = parse(src, Syntax::Crystal);
        assert!(diags.is_empty(), "parse diagnostics for `{src}`: {diags:?}");
        match eval(&ast, &EmptyContext) {
            Ok(Value::Currency(n)) => n,
            other => panic!("`{src}` → {other:?} (expected Currency)"),
        }
    }

    #[test]
    fn construction_keeps_full_precision() {
        // No 2dp snap (would be 1.23) and no 4dp snap (would be 1.2346): the raw value survives.
        assert_eq!(cur("CCur(1.23456)"), 1.23456);
        assert_eq!(cur("$9.87654"), 9.87654);
        // `$` and `CCur` are identical.
        assert_eq!(cur("$1.23456"), cur("CCur(1.23456)"));
    }

    #[test]
    fn no_construction_snap_collapses_tiny_amounts() {
        // If each CCur(0.001) snapped to 2dp it would be 0.00 and the sum 0.00; it does not.
        assert!((cur("CCur(0.001) + CCur(0.001) + CCur(0.001)") - 0.003).abs() < 1e-12);
    }

    #[test]
    fn arithmetic_matches_number_substrate() {
        // Division keeps full precision (no per-op currency re-round), like a plain Number would.
        let round_trip = cur("CCur(10) / 3 * 3");
        assert!((round_trip - 10.0).abs() < 1e-9, "got {round_trip}");
        // A currency operand promotes the result to Currency without changing the numeric value.
        assert_eq!(cur("CCur(2.5) + 1.005"), 3.505);
    }
}
