//! The non-eager call forms: the lazy conditionals ([`LazyForm`]) and the summary-function
//! reference form ([`SummaryCall`]). Classified by name in one place so the tree-walker and the
//! bytecode compiler share a single source of truth; their (necessarily duplicated) dispatch then
//! keys off these classifications.

use crate::ast::{Node, NodeKind};
use crate::eval::EvalError;
use crate::token::RefKind;

/// The three lazy-evaluation call forms: only the selected branch is evaluated, so they cannot be
/// dispatched as ordinary (eager) builtins. Classified by name in one place ([`LazyForm::from_name`])
/// so the tree-walker and the bytecode compiler share a single source of truth for which names are
/// lazy; their (necessarily duplicated) branch-selection logic then keys off the variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LazyForm {
    IIf,
    Switch,
    Choose,
}

impl LazyForm {
    /// Classify a call name (case-insensitive) as a lazy form, if it is one.
    pub(super) fn from_name(name: &str) -> Option<LazyForm> {
        match name.to_lowercase().as_str() {
            "iif" => Some(LazyForm::IIf),
            "switch" => Some(LazyForm::Switch),
            "choose" => Some(LazyForm::Choose),
            _ => None,
        }
    }
}

/// A summary-function call form — `Op({field}[, {group}])` whose operand is a `{...}` reference, as
/// opposed to the array-aggregate form `Sum([1, 2, 3])`. The reference operand marks it as a
/// reference to an existing group/grand-total summary, resolved through
/// [`EvalContext::resolve_summary`](crate::eval::EvalContext::resolve_summary) rather than reduced
/// inline.
#[derive(Debug, Clone)]
pub(super) struct SummaryCall {
    /// The lowercased operation name as written (`"sum"`, `"count"`, `"avg"`, …).
    pub op: String,
    /// The summarized field (`table.field`, or `@formula` with its sigil restored).
    pub field: String,
    /// The group scope's condition field, present only for the 2-argument (group-scoped) form.
    pub group: Option<String>,
}

impl SummaryCall {
    /// Classify a call as a summary function, if its name is a summary operation and its first
    /// argument is a `{...}` reference (the array-literal form is a plain aggregate, not a summary).
    pub(super) fn from_call(name: &str, args: &[Node]) -> Option<SummaryCall> {
        let op = name.to_lowercase();
        if !is_summary_func(&op) {
            return None;
        }
        let field = reference_operand(args.first()?)?;
        let group = args.get(1).and_then(reference_operand);
        Some(SummaryCall { op, field, group })
    }
}

/// A `GroupName({condition field})` call — a reference to a group's key, resolved through
/// [`EvalContext::group_name`](crate::eval::EvalContext::group_name) rather than computed from the
/// current row. The operand names the group's *condition field*, so it is a name and not a value:
/// the argument is never evaluated.
#[derive(Debug, Clone)]
pub(super) struct GroupNameCall {
    /// The named group's condition field (`table.field`, or `@formula` with its sigil restored).
    pub field: String,
}

impl GroupNameCall {
    /// Classify a call as a group-name reference: the name is `GroupName` and its first argument is a
    /// `{...}` reference naming the group's condition field.
    ///
    /// The optional second argument is the group's date periodicity as a token (`"monthly"`, …). It
    /// is not consulted: the period buckets the group's *key*, which the grouping pass has already
    /// applied, so the key this resolves to is the period's own value either way. A form with no
    /// reference operand at all — a bare group number — carries no name to resolve and stays with the
    /// ordinary dispatch, which reports it as unsupported.
    pub(super) fn from_call(name: &str, args: &[Node]) -> Option<GroupNameCall> {
        if !name.eq_ignore_ascii_case("groupname") || args.is_empty() || args.len() > 2 {
            return None;
        }
        Some(GroupNameCall {
            field: reference_operand(args.first()?)?,
        })
    }
}

/// The error a group-name reference raises when the context cannot resolve it — a group's key is a
/// fact about the grouped record set, which a bare formula evaluation (no data pipeline) lacks.
pub(super) fn group_name_needs_context(field: &str) -> EvalError {
    EvalError::Unsupported(format!("`GroupName({{{field}}})` needs a data context"))
}

/// Whether a (lowercased) call name is a Crystal summary operation. Covers the aliases the engine
/// accepts in a formula body; used only to gate the reference-operand form (an array argument to the
/// same name is the ordinary aggregate builtin).
fn is_summary_func(op_lower: &str) -> bool {
    matches!(
        op_lower,
        "sum"
            | "average"
            | "avg"
            | "count"
            | "distinctcount"
            | "minimum"
            | "min"
            | "maximum"
            | "max"
            | "median"
            | "mode"
            | "stddev"
            | "samplestddev"
            | "variance"
            | "samplevariance"
            | "popstddev"
            | "populationstddev"
            | "popvariance"
            | "populationvariance"
            | "correlation"
            | "covariance"
            | "weightedaverage"
            | "weightedavg"
            | "nthlargest"
            | "nthsmallest"
            | "nthmostfrequent"
            | "pthpercentile"
            | "percentile"
    )
}

/// The error a summary function raises when the context cannot resolve it — the record-set form
/// needs a report's computed summaries, which a bare formula evaluation (no data pipeline) lacks.
pub(super) fn summary_needs_context(sc: &SummaryCall) -> EvalError {
    EvalError::Unsupported(format!(
        "summary function `{}({})` needs a data context",
        sc.op, sc.field
    ))
}

/// The reference-operand string of a summary-function argument, with its sigil restored (so a formula
/// operand round-trips to `@name`, matching how a summary keys its summarized field). `None` when the
/// argument is not a `{...}` reference — which is what distinguishes the summary form from the
/// array-aggregate form.
fn reference_operand(node: &Node) -> Option<String> {
    match &node.kind {
        NodeKind::Reference { kind, name } => Some(match kind {
            RefKind::Formula => format!("@{name}"),
            RefKind::RunningTotal => format!("#{name}"),
            RefKind::SqlExpr => format!("%{name}"),
            RefKind::Parameter => format!("?{name}"),
            RefKind::Field => name.clone(),
        }),
        _ => None,
    }
}
