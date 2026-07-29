//! Group levels and sorts — the `0xe5` group record (the field it breaks on, its grouping
//! condition and its hierarchical-grouping trailer), the `0x0088` group-area format that precedes
//! it, the `0x00e9` specified-order values that follow it, and the `0x29` sort records that carry
//! both the record-level sorts and the group summary sorts.

use crate::build_model::record_values::summary_op_full;
use crate::build_model::row_of;
use crate::codec::RecordNode;
use crate::field_table::table::{Cell, Row};
use crate::field_table::tables as ft;
use crate::model::{Group, Sort};

/// The sort direction a legacy report writes on a date group it groups by day, in place of the
/// condition ordinal it leaves at `0`.
const LEGACY_DAILY_DIRECTION: u32 = 0x02;

/// The field-handle kind a group's summary sort states; any other kind is a plain record sort.
const GROUP_SUMMARY_FIELD_KIND: u32 = 2;

/// Raise one report group from its `0xe5` record: the field it breaks on, the sort over that field,
/// the grouping condition the field's value type selects, and the hierarchical-grouping trailer.
///
/// `None` for a `0xe5` that names no field, and for one that belongs to a chart or cross-tab grid
/// rather than to the report's own group levels.
pub(super) fn build_group(
    group: &Row,
    field_types: &std::collections::HashMap<String, crate::model::FieldValueType>,
) -> Option<Group> {
    let field = group.text("condition_field").to_owned();
    if field.is_empty() {
        return None;
    }
    // A `0xe5` record also encodes chart / cross-tab "grid" groups, which are scoped to their
    // object — not the report's `DataDefinition.Groups`. A real report group's order marker reads
    // `@Group #N Order`; a grid group's reads `@… Grid #N Order` or `@Column #N` / `@Row #N`.
    if !is_report_group(group) {
        return None;
    }
    let direction = crate::model::SortDirection::from_code(group.u("direction") as i32);
    // Grouping condition. Crystal's group condition is polymorphic (see `GroupCondition`), selected
    // by the group field's value type; only a Date/Time/DateTime or Boolean field carries one, so the
    // field type gates it (`condition_field` is `Alias.name`, looked up case-insensitively).
    //
    // A Boolean field reads the condition ordinal through the `CrBooleanConditionEnum` table; a
    // date/time field through `CrDateConditionEnum`, with two legacy fallbacks for reports that
    // leave that ordinal at `0`: the low byte of the group-order formula reference's own pool
    // index, and [`LEGACY_DAILY_DIRECTION`]. Discrete grouping leaves all of these clear. (Splitting
    // Boolean out of the date table also fixes a misread where a boolean ordinal 1–6 was decoded as a
    // date period.)
    use crate::model::{FieldValueType, GroupCondition};
    let sdk_ordinal = group.u("condition_ordinal") as u8;
    let date_condition = match field_types.get(&field.to_lowercase()) {
        Some(FieldValueType::Boolean) => GroupCondition::from_boolean_ordinal(sdk_ordinal),
        Some(FieldValueType::Date | FieldValueType::Time | FieldValueType::DateTime) => {
            GroupCondition::from_date_ordinal(sdk_ordinal)
                .or_else(|| {
                    GroupCondition::from_legacy_date_code(group.u("order_formula_index") as u8)
                })
                .or_else(|| {
                    (group.u("direction") == LEGACY_DAILY_DIRECTION)
                        .then_some(GroupCondition::Daily)
                })
        }
        _ => None,
    };
    Some(Group {
        sort: Sort {
            field: field.clone(),
            direction,
            kind: crate::model::SortKind::GroupSortField,
            topn: None,
        },
        condition_field: field,
        date_condition,
        options: Default::default(),
        // Populated by the off-by-one `0x0088` pass in `build_data_definition`.
        area_format: Default::default(),
        // Populated by the `0x00e9` pass in `build_data_definition` (specified-order groups only).
        hierarchical: Vec::new(),
        // The Hierarchical-Grouping trailer is part of this same `0xe5` record; its `group_indent`
        // is filled from the group's `0x0088` record in `build_data_definition`.
        hierarchical_options: decode_hierarchical_options(group),
    })
}

/// Whether a `0xe5` record is one of the report's own group levels, from the localized name of the
/// order field it generates: a report group's reads `@Group #N Order`, a chart category's
/// `@… Grid #N Order`, and a cross-tab dimension's `@Column #N Order` / `@Row #N Order`.
fn is_report_group(group: &Row) -> bool {
    let marker = group.text("order_marker");
    marker.starts_with("@Group #") && marker.ends_with(" Order")
}

/// Decode the **Hierarchical Grouping** trailer of a `0xe5` group record (SDK `IArea`
/// `EnableHierarchicalGroupSorting` / `ParentIDField` / `InstanceIDField`) — an enable flag then the
/// two `table.field` references, inside the record's fixed trailer. Returns `None` for a plain
/// group, which stores the flag clear and both references empty. `group_indent` is filled later
/// from the group's `0x0088` record (not in this record).
fn decode_hierarchical_options(group: &Row) -> Option<crate::model::HierarchicalGroupOptions> {
    if group.i("hierarchical_enabled") != 1 {
        return None;
    }
    let parent_id_field = group.text("parent_id_field").to_owned();
    let instance_id_field = group.text("instance_id_field").to_owned();
    if parent_id_field.is_empty() || instance_id_field.is_empty() {
        return None;
    }
    Some(crate::model::HierarchicalGroupOptions {
        enabled: true,
        parent_id_field,
        instance_id_field,
        group_indent: crate::model::Twips(0),
    })
}

/// One value of a group's specified order, from a `0x00e9` record: the value's display name and
/// the condition that defines it. `None` for a record that ended before its name, which carries no
/// value at all.
pub(super) fn decode_hierarchical_value(row: &Row) -> Option<crate::model::HierarchicalGroupValue> {
    let value_name = row.get("value_name").and_then(Cell::text)?.to_owned();
    Some(crate::model::HierarchicalGroupValue {
        value_name,
        condition: row.text("condition").to_owned(),
    })
}

/// Decode a `0x0088` GroupAreaFormat record from its field-table reading: the two area flags and
/// the per-page group limit that sits past the record's nested child.
pub(super) fn decode_group_area_format(row: &Row) -> crate::model::GroupAreaFormat {
    crate::model::GroupAreaFormat {
        repeat_group_header: row.i("repeat_group_header") != 0,
        keep_group_together: row.i("keep_group_together") != 0,
        // `VisibleGroupNumberPerPage` ("keep N groups together per page"; `0` = off). The engine
        // clamps a negative to zero on load.
        visible_groups_per_page: row.i("visible_groups_per_page").max(0),
    }
}

/// The per-level `GroupIndent`, in twips (`0` for a non-hierarchical group). The engine clamps a
/// negative to zero on load.
pub(super) fn decode_group_indent(row: &Row) -> crate::model::Twips {
    crate::model::Twips(row.i("group_indent").max(0))
}

/// The two shapes a `0x29` sort record can take.
pub(super) enum SortRecord {
    /// A plain record-level sort (`RecordSortField`).
    Record(Sort),
    /// A group's summary-based sort: `operand` is the summary display form (`Sum of {field}`) bound
    /// to the owning group. `dir_byte` is the raw direction byte; its meaning (TopN/BottomN vs
    /// Descending/Ascending) depends on the group's Top N limit, resolved when bound to the group.
    GroupSummary { operand: String, dir_byte: u8 },
}

/// Raise a `0x29` sort record. Its field handle's kind says which of the two shapes it is: a
/// [`GROUP_SUMMARY_FIELD_KIND`] handle is a group's summary sort, whose direction depends on the
/// group's Top N limit and so is resolved when it binds to the group rather than here; anything
/// else is a plain record sort (direction `0` ascending, `1` descending).
///
/// The Top N / Bottom N options themselves are not in this record: they live in the group's `0xe5`,
/// which is why the sort raised here leaves `topn` unset (see [`decode_group_topn`]).
pub(super) fn build_sort(node: &RecordNode, logical: &[u8]) -> Option<SortRecord> {
    let sort = row_of(node, logical, &ft::RECORD_SORT_FIELD);
    let field = sort.text("field").to_owned();
    if field.is_empty() {
        return None;
    }
    let dir_byte = sort.u("direction") as u8;
    if sort.u("field_kind") == GROUP_SUMMARY_FIELD_KIND {
        return Some(SortRecord::GroupSummary {
            operand: field,
            dir_byte,
        });
    }
    Some(SortRecord::Record(Sort {
        field,
        direction: crate::model::SortDirection::from_code(i32::from(dir_byte)),
        kind: crate::model::SortKind::RecordSortField,
        topn: None,
    }))
}

/// Decode a summary-based group sort's Top N / Bottom N options (SDK `TopBottomNSortField`) from the
/// group's `0xe5` record. `number_of_groups` is the group's Top N limit, which the record stores
/// twice — beside the "Others"-bucket name and again in its trailer.
///
/// `DiscardOthers` (SDK `EnableDiscardOtherGroups`) is the short that follows the limit. Unpinned:
/// that short reads `1` even on groups with no Top-N sort at all, so nothing here distinguishes it
/// from a structural constant.
///
/// `WithTies` (the designer "Include ties" option) has no located byte in this record and is left
/// `false`; see the `with_ties_defaults_false` test for the rationale.
pub(super) fn decode_group_topn(
    group: &Row,
    number_of_groups: u16,
) -> crate::model::TopBottomNSort {
    crate::model::TopBottomNSort {
        number_of_groups,
        discard_others: group.i("discard_others") == 1,
        not_in_topn_name: group.text("not_in_topn_name").to_owned(),
        with_ties: false,
    }
}

/// Resolve a group summary sort's direction from its direction byte and Top N limit: limited
/// (`N > 0`) → TopN (`1`) / BottomN (`0`); unlimited (`N == 0`) → Descending (`1`) / Ascending (`0`).
pub(super) fn group_sort_direction(dir_byte: u8, topn_limit: u16) -> crate::model::SortDirection {
    use crate::model::SortDirection::*;
    match (topn_limit > 0, dir_byte) {
        (true, 0) => BottomNOrder,
        (true, _) => TopNOrder,
        (false, 0) => AscendingOrder,
        (false, _) => DescendingOrder,
    }
}

/// Render a group's Top N / Bottom N sort field: the display form `Op of {operand}` becomes the
/// engine expression `Op ({operand}, {group field})` (e.g. `Sum of X` → `Sum ({X}, {group})`).
/// `Max`/`Min` expand to `Maximum`/`Minimum`, matching `data_source::field_data_source`.
pub(super) fn render_group_sort_summary(operand: &str, group_field: &str) -> String {
    match operand.split_once(" of ") {
        Some((op, summed)) => {
            let op = summary_op_full(op);
            format!("{op} ({{{summed}}}, {{{group_field}}})")
        }
        None => operand.to_string(),
    }
}

#[cfg(test)]
mod hierarchical_tests {
    use super::{decode_hierarchical_value, ft, Row};
    use crate::field_table::cursor::{Piece, RecordContent, StringFormat};
    use crate::field_table::table::read_strings;
    use crate::field_table::table::write_as;

    /// A `u32`-BE NUL-terminated length-prefixed string.
    fn lp(s: &str) -> Vec<u8> {
        let mut v = ((s.len() + 1) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v.push(0);
        v
    }

    /// A `0x00e9` record carrying `strings`, read through its field table.
    ///
    /// Only a group that declares value ranges is followed by such records, so the reading is
    /// exercised on bytes of this test's own making — held to the same bar as every other table:
    /// the record is accounted for exactly and re-emits byte for byte.
    ///
    /// A record built here carries no header to declare a string form, so the reading and the
    /// re-emission both name the enhanced form — the one the record-tree reader admits — rather
    /// than leaving it assumed.
    fn e9_row(strings: &[&str]) -> Row {
        let content = RecordContent {
            rtype: 0x00e9,
            schema: 0x0700,
            pieces: match strings.iter().flat_map(|s| lp(s)).collect::<Vec<u8>>() {
                v if v.is_empty() => Vec::new(),
                v => vec![Piece::Run(v)],
            },
        };
        let reading = read_strings(
            &ft::HIERARCHICAL_GROUP_VALUE,
            &content,
            StringFormat::Enhanced,
        );
        assert!(reading.exact(), "the table accounts for the record exactly");
        assert_eq!(
            write_as(
                &ft::HIERARCHICAL_GROUP_VALUE,
                &reading.row,
                0x0700,
                StringFormat::Enhanced
            ),
            content.pieces,
            "the row re-emits the record it was read from"
        );
        reading.row
    }

    #[test]
    fn decodes_value_name_and_condition() {
        let v = decode_hierarchical_value(&e9_row(&["X", "{Command.some_field} = \"X\""]))
            .expect("a named value");
        assert_eq!(v.value_name, "X");
        assert_eq!(v.condition, "{Command.some_field} = \"X\"");
    }

    #[test]
    fn missing_condition_yields_empty() {
        let v = decode_hierarchical_value(&e9_row(&["Low"])).expect("a named value");
        assert_eq!(v.value_name, "Low");
        assert_eq!(v.condition, "");
    }

    /// A stored empty name is a value with an empty name, not the absence of one — the two are the
    /// same length-prefixed string with and without characters in it.
    #[test]
    fn an_empty_name_is_still_a_value() {
        let v = decode_hierarchical_value(&e9_row(&["", "cond"])).expect("a value");
        assert_eq!((v.value_name.as_str(), v.condition.as_str()), ("", "cond"));
    }

    /// A record with no content at all never reaches its name, and carries no value.
    #[test]
    fn a_record_without_a_name_carries_no_value() {
        assert!(decode_hierarchical_value(&e9_row(&[])).is_none());
    }
}

#[cfg(test)]
mod hierarchical_options_tests {
    //! A record built here carries no header to declare a string form, so every reading names the
    //! enhanced form — the one the record-tree reader admits — rather than leaving it assumed.

    use super::{decode_group_indent, decode_hierarchical_options, ft, Row};
    use crate::field_table::cursor::{Piece, RecordContent, StringFormat};
    use crate::field_table::table::read_strings;

    /// A `u32`-BE NUL-terminated length-prefixed string.
    fn lp(s: &str) -> Vec<u8> {
        let mut v = ((s.len() + 1) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v.push(0);
        v
    }

    /// An unset field reference: an empty name, then a `(kind, index)` handle of `(0, 0xffff)`.
    fn empty_ref() -> Vec<u8> {
        let mut v = lp("");
        v.extend([0x00, 0xff, 0xff]);
        v
    }

    /// Build a `0xe5` record in its real shape: the condition field reference, the grouping ordinal
    /// and sort direction, the Top-N "Others" bucket and its limit,
    /// the two generated field references, and the trailer — whose Hierarchical-Grouping block
    /// (an enable word then the parent-ID and instance-ID references) is always written, empty and
    /// flag-clear on a plain group.
    fn e5_row(cond: &str, hier: Option<(&str, &str, u8)>) -> Row {
        let (parent, instance, flag) = hier.unwrap_or(("", "", 0x00));
        let mut v = lp(cond);
        v.extend([0x00, 0x00, 0x00]); // condition-field handle
        v.extend([0x00, 0x00, 0x00]); // ordinal, direction, the discarded enum
        v.extend(lp("Others"));
        v.extend([0x00, 0x00, 0x00, 0x01]); // Top-N limit, DiscardOthers
        v.extend(lp("Others"));
        v.extend(lp("Group #1 Name"));
        v.extend([0x04, 0x00, 0x00]);
        v.extend(lp("@Group #1 Order"));
        v.extend([0x01, 0x00, 0x00]);
        v.extend([0xff, 0xff]); // the -1 default
        v.extend([0u8; 8]); // the four words, suppress flags among them
        v.extend([0x00, 0x00]);
        v.extend(empty_ref()); // the group-name formula
        v.extend([0x00, flag]);
        v.extend(lp(parent));
        v.extend([0x00, 0xff, 0xff]); // parent-ID field handle
        v.extend(lp(instance));
        v.extend([0x00, 0xff, 0xff]); // instance-ID field handle
        v.push(0x00);
        v.extend([0u8; 8]);
        v.extend([0x00, 0x00]); // no specified-order pairs
        v.extend([0x00, 0x00]);
        v.extend(empty_ref()); // the Top-N value formula
        v.extend([0x00, 0x00]);
        v.extend(empty_ref()); // the group-sort-order formula
        v.extend([0x00, 0x00]);
        v.extend([0x00, 0x00]); // the Top-N limit, repeated
        v.extend([0u8; 8]);
        v.push(0x00);
        let content = RecordContent {
            rtype: 0x00e5,
            schema: 0x0700,
            pieces: vec![Piece::Run(v)],
        };
        let reading = read_strings(&ft::GROUP, &content, StringFormat::Enhanced);
        assert!(reading.exact(), "the synthetic record is table-shaped");
        reading.row
    }

    #[test]
    fn decodes_enabled_parent_and_instance() {
        let row = e5_row(
            "employee.employee_id",
            Some(("employee.manager_id", "employee.employee_id", 0x01)),
        );
        let h = decode_hierarchical_options(&row).expect("hierarchical");
        assert!(h.enabled);
        assert_eq!(h.parent_id_field, "employee.manager_id");
        assert_eq!(h.instance_id_field, "employee.employee_id");
        // `group_indent` is filled later from the `0x0088` record, not this record.
        assert_eq!(h.group_indent.0, 0);
    }

    #[test]
    fn plain_group_has_no_options() {
        // Flag clear and both references empty → not a hierarchical group.
        assert!(decode_hierarchical_options(&e5_row("region.name", None)).is_none());
    }

    #[test]
    fn flag_clear_is_not_hierarchical() {
        // The trailing field references exist but the enable flag is 0 → treated as non-hierarchical.
        let row = e5_row("t.f", Some(("t.parent", "t.f", 0x00)));
        assert!(decode_hierarchical_options(&row).is_none());
    }

    /// A `0x0088` record: the two flags and the indent, the nested `XmlDefinition`, then the
    /// per-page group limit and the formula that can override it.
    pub(super) fn group_area_format_row(indent: i32, visible: i32) -> Row {
        let mut head = vec![0x00, 0x00, 0x00, 0x00];
        head.extend(indent.to_be_bytes());
        let mut tail = visible.to_be_bytes().to_vec();
        tail.extend([0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xff, 0xff]);
        let content = RecordContent {
            rtype: 0x0088,
            schema: 0x0700,
            pieces: vec![
                Piece::Run(head),
                Piece::Child(crate::field_table::cursor::ChildRef {
                    rtype: 0x0151,
                    schema: 0x0700,
                    framed_len: 8,
                }),
                Piece::Run(tail),
            ],
        };
        let reading = read_strings(&ft::GROUP_AREA_FORMAT, &content, StringFormat::Enhanced);
        assert!(reading.exact(), "the synthetic record is table-shaped");
        reading.row
    }

    #[test]
    fn group_indent_is_the_word_before_the_nested_definition() {
        assert_eq!(decode_group_indent(&group_area_format_row(300, 0)).0, 300);
        // Plain group: the indent is zero.
        assert_eq!(decode_group_indent(&group_area_format_row(0, 0)).0, 0);
    }
}

#[cfg(test)]
mod group_option_tests {
    use crate::model::GroupCondition;

    #[test]
    fn legacy_date_codes_map_to_their_conditions() {
        assert_eq!(
            GroupCondition::from_legacy_date_code(0x01),
            Some(GroupCondition::Daily)
        );
        assert_eq!(
            GroupCondition::from_legacy_date_code(0x03),
            Some(GroupCondition::Monthly)
        );
        assert_eq!(
            GroupCondition::from_legacy_date_code(0x06),
            Some(GroupCondition::Weekly)
        );
        assert_eq!(
            GroupCondition::from_legacy_date_code(0x08),
            Some(GroupCondition::Weekly)
        );
    }

    #[test]
    fn unknown_legacy_date_codes_fall_back_to_none() {
        // Codes with no known meaning must NOT be mapped — they fall back to discrete.
        for code in [0x00u8, 0x02, 0x04, 0x05, 0x07, 0xff] {
            assert_eq!(
                GroupCondition::from_legacy_date_code(code),
                None,
                "code {code:#04x}"
            );
        }
    }

    #[test]
    fn sdk_date_ordinal_zero_is_discrete_nonzero_are_periods() {
        // Ordinal 0 is ambiguous (daily vs. discrete) and must not map here — the caller resolves
        // daily from the legacy flag. 1..=11 are the eleven non-daily periods.
        assert_eq!(GroupCondition::from_date_ordinal(0), None);
        assert_eq!(
            GroupCondition::from_date_ordinal(1),
            Some(GroupCondition::Weekly)
        );
        assert_eq!(
            GroupCondition::from_date_ordinal(4),
            Some(GroupCondition::Monthly)
        );
        assert_eq!(
            GroupCondition::from_date_ordinal(11),
            Some(GroupCondition::ByAMPM)
        );
        assert_eq!(
            GroupCondition::from_date_ordinal(12),
            Some(GroupCondition::Other(12))
        );
    }

    #[test]
    fn boolean_ordinals_use_the_boolean_table() {
        // The boolean enum starts at 1; ordinal 0 is a discrete boolean group. Crucially, ordinal 1
        // is ToYes here, NOT Weekly (the date table) — the field-type-gated split fixes that misread.
        assert_eq!(GroupCondition::from_boolean_ordinal(0), None);
        assert_eq!(
            GroupCondition::from_boolean_ordinal(1),
            Some(GroupCondition::ToYes)
        );
        assert_eq!(
            GroupCondition::from_boolean_ordinal(6),
            Some(GroupCondition::NextIsNo)
        );
        assert_eq!(
            GroupCondition::from_boolean_ordinal(9),
            Some(GroupCondition::Other(9))
        );
    }

    #[test]
    fn with_ties_defaults_false() {
        // WithTies (the designer "Include ties" option) is a real RAS property but has no located
        // storage in the `0xe5` group record: the only Top-N flag scalar there is the DiscardOthers
        // short. It is left false.
        assert!(!crate::model::TopBottomNSort::default().with_ties);
    }

    /// `VisibleGroupNumberPerPage` is the word past the record's nested definition, and `0` is "no
    /// limit" — what every report that does not set one stores.
    #[test]
    fn group_area_format_reads_the_visible_group_limit() {
        use super::hierarchical_options_tests::group_area_format_row;
        let f = super::decode_group_area_format(&group_area_format_row(0, 2));
        assert_eq!(f.visible_groups_per_page, 2);
        assert_eq!(
            super::decode_group_area_format(&group_area_format_row(0, 0)).visible_groups_per_page,
            0
        );
    }
}
