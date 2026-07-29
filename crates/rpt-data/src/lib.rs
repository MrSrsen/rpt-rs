//! # rpt-data — the record pipeline
//!
//! Turns a report's [`RowSource`] + [`DataDefinition`](rpt_model::DataDefinition) into a
//! [`Dataset`]: the grouped, summarized **instance tree** the layout engine walks. This is the
//! "push-built" half of the native pull-driven formatter: here we build the tree; `rpt-layout`
//! iterates it and pulls values.
//!
//! Stages (see [`build_dataset`]): record selection (evaluate the selection formula per row) →
//! record sort → grouping (nest by each [`Group`](rpt_model::Group)) → summaries at every group
//! level and a grand total. Formula/field resolution runs through [`DataContext`], which is the
//! evaluator's [`EvalContext`](rpt_formula::eval::EvalContext).

mod context;
mod diagnostics;
mod eval_time;
mod pipeline;
mod running_total;
mod schedule;
mod source;
mod summary;
mod value_order;

pub use context::{
    normalize_param_name, DataContext, DateTimeSpecials, FormulaRegistry, GroupNameScope,
    Parameters, SharedState, SummaryScope,
};
pub use diagnostics::{CollectingSink, DiagnosticKind, DiagnosticSink, EvalDiagnostic};
pub use eval_time::{classify_eval_time, EvalTime};
pub use pipeline::{
    build_dataset, build_dataset_opts, build_dataset_with, build_dataset_with_at,
    build_dataset_with_diagnostics, build_dataset_with_params, build_dataset_with_params_at,
    compile_formulas, compile_formulas_at, compile_formulas_reporting, date_bucket, DatasetOptions,
    FieldFilter,
};
pub use running_total::RunningTotals;
pub use schedule::{EvalSchedule, ScheduledValues};
pub use source::{
    cell_to_value, rows_from_cells, Cell, Column, DbError, EmptySource, Row, RowData, RowSource,
    SavedDataSource, ScopeData,
};
pub use summary::SummaryAccumulator;
pub use value_order::{compare_values, value_key};

use rpt_formula::eval::Value;
use rpt_model::SummaryOperation;

/// One computed summary: the operation, the field summarized, and the resulting value.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// The summary operation applied (sum, average, count, …).
    pub operation: SummaryOperation,
    /// The field (or `@formula`) being summarized.
    pub field: String,
    /// The computed summary value.
    pub value: Value,
}

/// One group instance in the tree. A non-leaf carries `subgroups`; a leaf carries `details`.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupInstance {
    /// 0-based nesting level (0 = outermost group).
    pub level: usize,
    /// The field (or `@formula`) this level groups by.
    pub condition_field: String,
    /// The group's key value (its `GroupName`).
    pub key: Value,
    /// The group's decoded [`rpt_model::GroupCondition`], carried through from its
    /// [`rpt_model::Group`]. A calendar period bucketed [`key`](Self::key) to the period's start
    /// date, which alone cannot say *which* period produced it — a 1st-of-month key is equally a
    /// monthly, semi-monthly, or daily bucket — so the label formatters read the grain from here
    /// rather than inferring it from the key.
    pub date_condition: Option<rpt_model::GroupCondition>,
    /// Summaries computed over this group's rows.
    pub summaries: Vec<Summary>,
    /// Deeper group instances (empty at the deepest level).
    pub subgroups: Vec<GroupInstance>,
    /// Detail rows (only populated at the deepest level).
    pub details: Vec<Row>,
    /// Instances of **this same group level** nested under this one by Crystal's *Hierarchical
    /// Grouping* (`HierarchicalGroupOptions`): the instances whose `ParentIDField` value matches this
    /// one's `InstanceIDField` value. Always empty for a group without hierarchical sorting.
    ///
    /// They share `level` with their parent, so they print through the same group header/footer
    /// bands — the tree is a print-order nesting, not a deeper grouping level.
    pub hierarchy_children: Vec<GroupInstance>,
}

/// The materialized result of the pipeline: the instance tree plus schema and grand totals.
#[derive(Debug, Clone, PartialEq)]
pub struct Dataset {
    /// The result schema: one [`Column`] per selected value, in result order.
    pub columns: Vec<Column>,
    /// Rows kept after selection (the count the report processed).
    pub row_count: usize,
    /// Top-level group instances (empty when the report has no groups).
    pub groups: Vec<GroupInstance>,
    /// Detail rows when the report has no grouping (flat list); empty when grouped.
    pub details: Vec<Row>,
    /// Report-level (grand total) summaries.
    pub grand_total: Vec<Summary>,
    /// Report parameter values supplied by the caller (`{?Name}` resolution). Empty by default.
    pub params: Parameters,
}

impl Dataset {
    /// Iterate the detail rows in report order, regardless of grouping — walks the tree depth-first.
    pub fn iter_detail_rows(&self) -> Vec<&Row> {
        if self.groups.is_empty() {
            return self.details.iter().collect();
        }
        let mut out = Vec::new();
        fn walk<'a>(g: &'a GroupInstance, out: &mut Vec<&'a Row>) {
            for row in &g.details {
                out.push(row);
            }
            for sub in &g.subgroups {
                walk(sub, out);
            }
            for child in &g.hierarchy_children {
                walk(child, out);
            }
        }
        for g in &self.groups {
            walk(g, &mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests;
