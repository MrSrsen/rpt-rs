//! Data definition — the data half of the report: the fields it can reference, the groups it
//! breaks on, and the sorts over both. This module root holds the walk that assembles
//! [`DataDefinition`] from records scattered across the whole tree; the submodules own the raising
//! detail:
//!
//! - [`fields`] — the field definitions: database fields, formulas and custom functions, running
//!   totals, summaries and SQL expressions, over the `0x0071 NamedValue` base under all of them.
//! - [`groups`] — the group levels and the sort records: the grouping condition, the group sort and
//!   its Top N options, the group-area format, and the hierarchical-grouping trailer.

use crate::build_model::row_of;
use crate::codec::RecordNode;
use crate::field_table::tables as ft;
use crate::model::{DataDefinition, Formula, Sort};
use crate::records::rtype::*;

mod fields;
mod groups;

use fields::{
    build_field_manager_census, build_formula_variables, build_formulas, build_running_totals,
    build_sql_expressions, build_summaries, build_summary_bindings,
};
use groups::{
    build_group, build_sort, decode_group_area_format, decode_group_indent, decode_group_topn,
    decode_hierarchical_value, group_sort_direction, render_group_sort_summary, SortRecord,
};

// The `0x0071 NamedValue` base and the database-field definition built on it are also read by the
// report-definition decoders (a sibling module), so re-export them up to the parent `build_model`.
pub(in crate::build_model) use fields::{build_field, named_value};

/// SDK `DataDefinition`: the referenced database fields (`0x73` records), found anywhere in the
/// record tree. Formula/parameter/summary field *definitions* are not stored as plain records in
/// `Contents` the way db fields are, so they are not fabricated here; the raw records are still
/// visible in `--full` export.
pub(super) fn build_data_definition(
    tree: &[RecordNode],
    logical: &[u8],
    field_types: &std::collections::HashMap<String, crate::model::FieldValueType>,
) -> DataDefinition {
    let mut field_definitions = Vec::new();
    let mut groups = Vec::new();
    let mut record_sort_fields = Vec::new();
    // Each group's GroupAreaFormat is the `0x0088` record that immediately *precedes* its `0xe5`
    // (including the outermost group — its `0x0088` sits before the first `0xe5`). Stage every
    // `0x0088` across the pre-order walk; the one in effect when a group appears (the immediately
    // preceding one) is that group's format.
    let mut pending_group_format: Option<crate::model::GroupAreaFormat> = None;
    // The `0x0088` GroupAreaFormat also carries the group's per-level `GroupIndent` (bytes `[6..8]`),
    // which belongs to a group's hierarchical-grouping options; staged alongside the format.
    let mut pending_group_indent: Option<crate::model::Twips> = None;
    // Group summary sorts (a `0x29` record with a `0x02` marker) are emitted, in group order,
    // before their groups' `0xe5` records — queue each and bind it to the next raised group (FIFO).
    let mut pending_group_sorts: std::collections::VecDeque<(String, u8)> =
        std::collections::VecDeque::new();
    for root in tree {
        root.walk(&mut |node| match node.rtype {
            FIELD_DEFINITION => {
                if let Some(f) = build_field(node, logical) {
                    field_definitions.push(f);
                }
            }
            GROUP => {
                let group = row_of(node, logical, &ft::GROUP);
                if let Some(mut g) = build_group(&group, field_types) {
                    g.area_format = pending_group_format.take().unwrap_or_default();
                    // Attach the group's per-level indent (from its `0x0088`) to its hierarchical
                    // options, if this is a hierarchical group.
                    if let (Some(h), Some(indent)) =
                        (g.hierarchical_options.as_mut(), pending_group_indent.take())
                    {
                        h.group_indent = indent;
                    }
                    // A queued summary sort replaces the group's default field sort: the sort field
                    // becomes the group-scoped summary expression, its direction resolved from the
                    // group's Top N limit. It is not also emitted as a record sort.
                    if let Some((operand, dir_byte)) = pending_group_sorts.pop_front() {
                        g.sort.field = render_group_sort_summary(&operand, &g.condition_field);
                        let n = group.i("topn_limit") as u16;
                        g.sort.direction = group_sort_direction(dir_byte, n);
                        // Only a summary-based group sort is a `TopBottomNSortField`; a plain
                        // group-field sort keeps `topn = None` and emits no Top N attrs.
                        g.sort.topn = Some(decode_group_topn(&group, n));
                    }
                    groups.push(g);
                }
            }
            GROUP_AREA_FORMAT => {
                let row = row_of(node, logical, &ft::GROUP_AREA_FORMAT);
                pending_group_format = Some(decode_group_area_format(&row));
                pending_group_indent = Some(decode_group_indent(&row));
            }
            // A `0x00e9` specified-order group value follows its group's `0xe5` (flat siblings), so it
            // binds to the most-recently-decoded report group. Grid `0xe5` records decode to `None`, so
            // `groups.last_mut()` is always a real report group.
            HIERARCHICAL_GROUP_VALUE => {
                let row = row_of(node, logical, &ft::HIERARCHICAL_GROUP_VALUE);
                if let (Some(g), Some(v)) = (groups.last_mut(), decode_hierarchical_value(&row)) {
                    g.hierarchical.push(v);
                }
            }
            RECORD_SORT_FIELD => match build_sort(node, logical) {
                Some(SortRecord::GroupSummary { operand, dir_byte }) => {
                    pending_group_sorts.push_back((operand, dir_byte));
                }
                Some(SortRecord::Record(s)) => record_sort_fields.push(s),
                None => {}
            },
            _ => {}
        });
    }
    // Group sorts are listed first (one per group, `GroupSortField`), then the record-level sorts
    // (from the `0x29` records) in document order. A `0x29` sort whose field is itself a group field
    // is reported as a `GroupSortField` (it is that group's sort), not a record sort.
    let mut record_sorts: Vec<Sort> = groups.iter().map(|g| g.sort.clone()).collect();
    for mut s in record_sort_fields {
        if groups.iter().any(|g| g.condition_field == s.field) {
            s.kind = crate::model::SortKind::GroupSortField;
        }
        record_sorts.push(s);
    }

    let formulas = build_formulas(tree, logical);
    field_definitions.extend(formulas.user_formulas);
    field_definitions.extend(build_running_totals(tree, logical));
    field_definitions.extend(build_summaries(tree, logical));
    field_definitions.extend(build_sql_expressions(tree, logical));
    DataDefinition {
        field_definitions,
        groups,
        record_sorts,
        record_selection: formulas.record_selection.map(Formula),
        group_selection: formulas.group_selection.map(Formula),
        saved_data_filter: formulas.saved_data_filter.map(Formula),
        condition_formula_bodies: formulas.condition_formula_bodies,
        running_total_condition_formulas: formulas.running_total_condition_formulas,
        summary_binding_fields: build_summary_bindings(tree, logical),
        formula_variables: build_formula_variables(tree, logical),
        field_manager_census: build_field_manager_census(tree, logical),
        custom_functions: formulas.custom_functions,
    }
}
