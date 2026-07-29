//! What a report groups and summarises: the group records, and the fields whose value a group
//! or the report as a whole produces.

use super::*;

/// Whether a summary definition stores its percentage base group: the engine reads it only on a
/// definition whose percentage flag is set.
fn is_percentage(c: &Ctx<'_>) -> bool {
    c.row.i("is_percentage") != 0
}

/// `0x007e SummaryFieldDefinition` — one summary or running-total operation.
///
/// The nested `NamedValue` carrying the result type comes **first**, ahead of the record's own
/// fields. Two **field references** follow the operation — the summarized field and a second one
/// (stored empty unless the definition was built through the controller API) — each a name plus its
/// pool handle, so the percentage tail's position follows both names' lengths.
///
/// The tail is three guarded groups, and the base group is inside the first: the engine reads it
/// **only when the percentage flag is set**. Read unconditionally it lands on the two trailing
/// enums instead, which is a number, just not that one.
pub(crate) const SUMMARY_FIELD_DEFINITION: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x007e,
    name: "SummaryFieldDef",
    fields: &[
        Field::new("value", Kind::Child(0x0071)),
        Field::new("operation", Kind::VarU16),
        // Read by the engine and thrown away.
        Field::new("_u0", Kind::VarU16),
        Field::new("operation_parameter", Kind::U16Be),
        Field::new("operand", Kind::Str),
        Field::new("operand_kind", Kind::VarU16),
        Field::new("operand_index", Kind::U16Be),
        Field::new("secondary_operand", Kind::Str),
        Field::new("secondary_operand_kind", Kind::VarU16),
        Field::new("secondary_operand_index", Kind::U16Be),
        Field::new("is_percentage", Kind::I16Be),
        Field::when("percentage_base_group", Kind::I16Be, is_percentage),
        Field::new("_u1", Kind::VarU16),
        Field::new("_u2", Kind::VarU16),
        // The same slot the base group uses, written again when the second trailing enum is set.
        Field::when("_u3", Kind::I16Be, |c| c.row.u("_u2") != 0),
    ],
};

/// Whether a running total's condition names a field: the condition is a kind followed by whatever
/// that kind names, and only the change-of-field and formula kinds name anything referable.
fn condition_names_a_field(c: &Ctx<'_>, kind: &'static str) -> bool {
    matches!(c.row.u(kind), 1 | 3)
}

/// Whether a running total's condition names a group, which is stored as a plain number.
fn condition_names_a_group(c: &Ctx<'_>, kind: &'static str) -> bool {
    c.row.u(kind) == 2
}

/// `0x0080 RunningTotalField` — a running total: the summary operation it accumulates, and the two
/// conditions that drive it.
///
/// The record **contains** its summary definition rather than preceding it. The `0x007e` is the
/// record's first content, exactly as a formula's `0x0071` is, so a running total is a summary plus
/// two conditions and not a pair of adjacent records — and the two are only ever adjacent in the
/// stream because a parent's header immediately precedes its first child's.
///
/// Each condition is a kind followed by what that kind names, so its length follows its kind: a
/// change-of-field names a field reference, a formula names the formula's reference, a
/// change-of-group names the group's number as a bare word, and no-condition names nothing at all.
/// The evaluate condition therefore sits wherever the reset condition ends — read at a fixed offset
/// it lands inside the reset condition's field name on every record that has one.
///
/// The two kinds share one coding: `0` no condition, `1` on change of field, `2` on change of
/// group, `3` on a formula. The trailing word is written only while the record still has content.
pub(crate) const RUNNING_TOTAL_FIELD: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0080,
    name: "RunningTotalField",
    fields: &[
        Field::new("summary", Kind::Child(0x007e)),
        Field::new("reset_kind", Kind::VarU16),
        Field::when("reset_field", Kind::FieldRef, |c| {
            condition_names_a_field(c, "reset_kind")
        }),
        Field::when("reset_group", Kind::U16Be, |c| {
            condition_names_a_group(c, "reset_kind")
        }),
        Field::new("evaluate_kind", Kind::VarU16),
        Field::when("evaluate_field", Kind::FieldRef, |c| {
            condition_names_a_field(c, "evaluate_kind")
        }),
        Field::when("evaluate_group", Kind::U16Be, |c| {
            condition_names_a_group(c, "evaluate_kind")
        }),
        Field::optional("_u0", Kind::I16Be),
    ],
};

/// One field a SQL expression's text refers to.
const SQL_EXPRESSION_REFERENCE: &[Field] = &[Field::new("field", Kind::FieldRef)];

/// `0x0081 SqlExpressionField` — a SQL Expression field: a snippet of raw SQL the database
/// evaluates, referenced from the report as `{%Name}`.
///
/// The nested `NamedValue` comes **first** and carries the expression's name, result type and
/// stored width; the SQL text follows it, then a word, then the fields the text names — a count and
/// one reference each. The text is a stored value even when it is empty, so a record that carries
/// no expression still carries its framing and everything after it keeps its place.
pub(crate) const SQL_EXPRESSION_FIELD: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0081,
    name: "SqlExpressionField",
    fields: &[
        Field::new("value", Kind::Child(0x0071)),
        Field::new("text", Kind::Str),
        Field::new("_u0", Kind::I16Be),
        Field::new("reference_count", Kind::U16Be),
        Field::new(
            "references",
            Kind::Repeat {
                count: Count::FromField("reference_count"),
                body: SQL_EXPRESSION_REFERENCE,
            },
        ),
    ],
};

/// One entry of a group's specified-order pair array.
const ORDER_PAIR: &[Field] = &[Field::new("a", Kind::U32Be), Field::new("b", Kind::U32Be)];

/// `0x00e5 Group` — one grouping level: a report group, a chart category, or a cross-tab dimension
/// (the three are told apart by `order_marker`, the localized name of the group's generated order
/// field).
///
/// The record is a straight line of **seven field references** — the condition field, the generated
/// name and order fields, the group-name formula, the parent-ID and instance-ID fields, and the
/// Top-N and group-sort formulas — with the scalars between them. Each reference is a name, then
/// the pool and index that resolve it, so every one of them is variable-length even though a
/// non-hierarchical group stores its two hierarchy references (and its three formulas) empty.
///
/// The grouping period and the sort direction are the two enums after the condition field. A third
/// enum follows them that the engine loads and never reads back; it holds the same value as the
/// direction, so reading the direction one byte late would go undetected by comparing output alone.
///
/// Four adjacent words sit between the order reference and the group-name formula. `suppress_label`
/// and `suppress_subtotal` are a cross-tab dimension level's two suppress flags (a report group and
/// a chart category store both clear); `has_value_ranges` is not a suppress flag at all — the engine
/// reads the group's value-range-list records after the record iff it is set. Which of the other
/// two carries the label flag rests on `has_value_ranges` having its own use, not on either ever
/// being non-zero.
///
/// The tail past the hierarchy block is a run of guarded groups: an enum, a double, a
/// count-prefixed array of `(u32, u32)` specified-order pairs, the Top-N value formula, the
/// group-sort-order formula, and a final trio that the engine defaults from the Top-N limit, the
/// double and the direction when the record ends before it. The array is ordinarily empty; a
/// non-empty one moves everything after it.
pub(crate) const GROUP: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00e5,
    name: "Group",
    fields: &[
        Field::new("condition_field", Kind::Str),
        Field::new("condition_field_kind", Kind::VarU16),
        Field::new("condition_field_index", Kind::U16Be),
        Field::new("condition_ordinal", Kind::VarU16),
        Field::new("direction", Kind::VarU16),
        // Loaded and thrown away by the engine, which never reads it back.
        Field::new("_discarded", Kind::VarU16),
        Field::new("not_in_topn_name", Kind::Str),
        Field::new("topn_limit", Kind::I16Be),
        Field::new("discard_others", Kind::I16Be),
        Field::new("_others_name", Kind::Str),
        Field::new("group_name_field", Kind::Str),
        Field::new("group_name_field_kind", Kind::VarU16),
        Field::new("group_name_field_index", Kind::U16Be),
        Field::new("order_marker", Kind::Str),
        Field::new("order_formula_kind", Kind::VarU16),
        Field::new("order_formula_index", Kind::U16Be),
        Field::new("_u0", Kind::I16Be),
        Field::new("_u1", Kind::I16Be),
        Field::new("suppress_subtotal", Kind::I16Be),
        Field::new("has_value_ranges", Kind::I16Be),
        Field::new("suppress_label", Kind::I16Be),
        Field::new("_u2", Kind::I16Be),
        Field::new("group_name_formula", Kind::Str),
        Field::new("group_name_formula_kind", Kind::VarU16),
        Field::new("group_name_formula_index", Kind::U16Be),
        Field::new("hierarchical_enabled", Kind::I16Be),
        Field::new("parent_id_field", Kind::Str),
        Field::new("parent_id_kind", Kind::VarU16),
        Field::new("parent_id_index", Kind::U16Be),
        Field::new("instance_id_field", Kind::Str),
        Field::new("instance_id_kind", Kind::VarU16),
        Field::new("instance_id_index", Kind::U16Be),
        Field::new("_u3", Kind::VarU16),
        Field::new("_u4", Kind::F64Be),
        Field::new("order_pair_count", Kind::U16Be),
        Field::new(
            "order_pairs",
            Kind::Repeat {
                count: Count::FromField("order_pair_count"),
                body: ORDER_PAIR,
            },
        ),
        Field::new("_u5", Kind::I16Be),
        Field::new("topn_value_formula", Kind::Str),
        Field::new("topn_value_formula_kind", Kind::VarU16),
        Field::new("topn_value_formula_index", Kind::U16Be),
        Field::new("_u6", Kind::I16Be),
        Field::new("group_sort_formula", Kind::Str),
        Field::new("group_sort_formula_kind", Kind::VarU16),
        Field::new("group_sort_formula_index", Kind::U16Be),
        Field::new("_u7", Kind::I16Be),
        Field::new("topn_limit_repeat", Kind::I16Be),
        Field::new("_u8", Kind::F64Be),
        Field::new("_u9", Kind::VarU16),
    ],
};

/// `0x00e9 HierarchicalGroupingOptions` — one value of a group's specified order: the value's
/// display name, then the condition that decides which records fall under it.
///
/// The two strings are the whole record and the whole of what the format stores for such a value —
/// nothing else follows them. The records appear in a run after the `0x00e5` group whose
/// `has_value_ranges` is set, between the `0x00e7` that counts them and the `0x00e8` that closes
/// the run.
///
/// The layout rests on the format's own writer, which emits exactly these two strings and ends
/// the record.
pub(crate) const HIERARCHICAL_GROUP_VALUE: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00e9,
    name: "HierarchicalGroupingOptions",
    fields: &[
        Field::new("value_name", Kind::Str),
        Field::new("condition", Kind::Str),
    ],
};
