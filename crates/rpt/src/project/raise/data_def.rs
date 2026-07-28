//! Data definition — formula/summary/running-total fields, groups, sorts (the data half).

use super::*;

/// SDK `DataDefinition`: the referenced database fields (`0x73` records), found anywhere in the
/// record tree. Formula/parameter/summary field *definitions* are not stored as plain records in
/// `Contents` the way db fields are, so they are not fabricated here; the raw records are still
/// visible in `--full` export.
pub(super) fn raise_data_definition(
    tree: &[RecordNode],
    logical: &[u8],
    known_db_fields: &std::collections::HashSet<String>,
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
    // The `0x0088` GroupAreaFormat also carries the group's per-level `GroupIndent` (leaf `[6..8]`),
    // which belongs to a group's hierarchical-grouping options; staged alongside the format.
    let mut pending_group_indent: Option<crate::model::Twips> = None;
    // Group summary sorts (a `0x29` record with a `0x02` marker) are emitted, in group order,
    // before their groups' `0xe5` records — queue each and bind it to the next raised group (FIFO).
    let mut pending_group_sorts: std::collections::VecDeque<(String, u8)> =
        std::collections::VecDeque::new();
    for root in tree {
        root.walk(&mut |node| match node.rtype {
            FIELD_DEF => {
                if let Some(f) = raise_field(node, logical) {
                    field_definitions.push(f);
                }
            }
            GROUP => {
                if let Some(mut g) = raise_group(node, logical, field_types) {
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
                        let n = group_topn_limit(node, logical);
                        g.sort.direction = group_sort_direction(dir_byte, n);
                        // Only a summary-based group sort is a `TopBottomNSortField`; a plain
                        // group-field sort keeps `topn = None` and emits no Top N attrs.
                        g.sort.topn = Some(decode_group_topn(node, logical, n));
                    }
                    groups.push(g);
                }
            }
            GROUP_OPTIONS => {
                let lb = node.leaf_bytes(logical);
                pending_group_format = Some(decode_group_area_format(&lb));
                pending_group_indent = Some(decode_group_indent(&lb));
            }
            // A `0x00e9` specified-order group value follows its group's `0xe5` (flat siblings), so it
            // binds to the most-recently-raised report group. Grid `0xe5` records raise to `None`, so
            // `groups.last_mut()` is always a real report group.
            HIER_GROUP => {
                if let (Some(g), Some(v)) = (
                    groups.last_mut(),
                    decode_hierarchical_value(&node.leaf_bytes(logical)),
                ) {
                    g.hierarchical.push(v);
                }
            }
            RECORD_SORT_FIELD => match raise_sort(node, logical) {
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

    let formulas = raise_formulas(tree, logical, known_db_fields, &groups);
    field_definitions.extend(formulas.user_formulas);
    field_definitions.extend(raise_running_totals(tree, logical));
    field_definitions.extend(raise_summaries(tree, logical));
    field_definitions.extend(raise_sql_expressions(tree, logical));
    DataDefinition {
        field_definitions,
        groups,
        record_sorts,
        record_selection: formulas.record_selection.map(Formula),
        group_selection: formulas.group_selection.map(Formula),
        saved_data_filter: formulas.saved_data_filter.map(Formula),
        condition_formula_bodies: formulas.condition_formula_bodies,
        running_total_condition_formulas: formulas.running_total_condition_formulas,
        summary_binding_fields: raise_summary_bindings(tree, logical),
        formula_variables: raise_formula_variables(tree, logical),
        field_manager_census: raise_field_manager_census(tree, logical),
        custom_functions: formulas.custom_functions,
    }
}

/// Decode a `0x00e9` `HierarchicalGroupingOptions` leaf: two length-prefixed strings (`u32` big-endian
/// byte count including the trailing NUL) — the specified-order group value's display name then its
/// defining condition-formula. Returns `None` if the value name is empty/unparseable.
pub(super) fn decode_hierarchical_value(
    leaf: &[u8],
) -> Option<crate::model::HierarchicalGroupValue> {
    let (value_name, consumed) = read_lp_string(leaf)?;
    let condition = read_lp_string(&leaf[consumed..])
        .map(|(s, _)| s)
        .unwrap_or_default();
    Some(crate::model::HierarchicalGroupValue {
        value_name,
        condition,
    })
}

/// Decode the field-pool census from the `0x006e` `FieldManagerEntry` record (20-byte leaf, one per
/// report): `[u32 BE database_fields][u16 BE formula-body-count-less-3][…]`.
/// Returns `None` when the record is absent. The stored formula-body count omits the three built-in
/// formulas, so it is reconstructed here (`+ 3`), matching the `0x0076` record count exactly.
pub(super) fn raise_field_manager_census(
    tree: &[RecordNode],
    logical: &[u8],
) -> Option<crate::model::FieldManagerCensus> {
    const BUILTIN_FORMULAS: u16 = 3;
    let node = nodes_where(tree, |n| n.rtype == FIELD_MANAGER_ENTRY)
        .into_iter()
        .next()?;
    let leaf = node.leaf_bytes(logical);
    let database_fields = u32_be(&leaf, 0)?;
    let formula_bodies = u16_be(&leaf, 4)?.saturating_add(BUILTIN_FORMULAS);
    Some(crate::model::FieldManagerCensus {
        database_fields,
        formula_bodies,
    })
}

/// The report's persisted formula-language variables (`Global`/`Shared`), decoded from the `0x0118`
/// records (the preceding `0x0116` header holds the count). Each
/// `0x0118` leaf is `[u32 BE namelen (incl NUL)][name + NUL][type byte][scope byte]`, where `type` is
/// the variable's declared FL result kind (mapped to [`FieldValueType`]) and
/// `scope` its `FLScope` (`0`=Shared, `1`=Global; `Local` variables are not persisted). One
/// [`FormulaVariable`] is materialised per `0x0118` found (the `0x0116` count is redundant). These are
/// STRUCTURAL — no SDK accessor exposes them, so they are not on any output surface.
pub(super) fn raise_formula_variables(tree: &[RecordNode], logical: &[u8]) -> Vec<FormulaVariable> {
    let mut out = Vec::new();
    for root in tree {
        root.walk(&mut |node| {
            if node.rtype != FORMULA_VARIABLE {
                return;
            }
            let leaf = node.leaf_bytes(logical);
            let Some((name, consumed)) = read_lp_string(&leaf) else {
                return;
            };
            let value_type = leaf
                .get(consumed)
                .map(|&b| FieldValueType::from_result_kind(i32::from(b)))
                .unwrap_or_default();
            let scope = leaf
                .get(consumed + 1)
                .map(|&b| FormulaVariableScope::from_code(i32::from(b)))
                .unwrap_or_default();
            out.push(FormulaVariable {
                name,
                value_type,
                scope,
            });
        });
    }
    out
}

/// The summarized field of every **summary definition** (`ISummaryField`) in the data-definition
/// region. These are the `0x7e` summary records (each wrapped in a `0x7f`) that appear *before* the
/// report layout (the first `0x8a` area marker). Running totals (`0x7e` preceded by a `0x80` reset
/// record) are excluded — they are decoded separately — and so are the chart/cross-tab data bindings,
/// which live inside the layout (after the first area marker). Only the field-shaped summarized field
/// (`table.field` or `@formula`) of each is returned, in document order. Reconciling these against
/// the placed summaries is what recovers the orphan summary definitions (see
/// `DataDefinition.summary_binding_fields`).
pub(super) fn raise_summary_bindings(tree: &[RecordNode], logical: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for node in summary_def_nodes(tree, true) {
        // The summarized field is the first field-shaped length-prefixed string in the record's own
        // leaf (`table.field` or `@formula`); the operation byte precedes it and the name is a child.
        if let Some(f) = own_lp_strings(node, logical)
            .into_iter()
            .find(|s| is_field_ref(s))
        {
            out.push(f);
        }
    }
    out
}

/// The classified outcome of pairing formula bodies (`0x76`) with their following name (`0x71`).
#[derive(Default)]
pub(super) struct Formulas {
    user_formulas: Vec<FieldDef>,
    record_selection: Option<String>,
    group_selection: Option<String>,
    saved_data_filter: Option<String>,
    /// Bodies of conditional/auxiliary formulas (running-total eval/reset conditions, section/object
    /// conditional formulas) that are not user field definitions.
    condition_formula_bodies: Vec<String>,
    /// Subset of `condition_formula_bodies`: only the running-total **condition** formulas (names
    /// ending `" Condition Formula"`). Kept separately because, unlike the section/object conditional
    /// formulas, these are *not* attached to any section/object, so a consumer walking the report's
    /// objects reaches them only through this list.
    running_total_condition_formulas: Vec<String>,
    /// Custom functions: formula records whose body opens with the reserved `Function (args) …`
    /// header. Not formula fields — the engine lists them under `CustomFunctions` — so they are
    /// collected here rather than in `user_formulas`.
    custom_functions: Vec<crate::model::CustomFunction>,
}

/// Whether a formula body is a Crystal *custom function* declaration (opens with the reserved
/// `Function` keyword then `(`). The keyword can only begin a custom-function declaration — a report
/// formula body never starts with it — so the body-shape check is exact.
fn is_custom_function_body(body: &str) -> bool {
    body.trim_start()
        .strip_prefix("Function")
        .is_some_and(|rest| rest.trim_start().starts_with('('))
}

/// Crystal's reserved section/object conditional-formula names — these are *not* user formula
/// fields (they attach to sections/objects, not `FormulaFieldDefinitions`).
const SECTION_FORMULA_NAMES: &[&str] = &[
    "New_Page_After",
    "New_Page_Before",
    "Reset_Page_Number_After",
    "Keep_Together",
    "Suppress",
    "Underlay_Following_Sections",
    "Print_at_Bottom_of_Page",
    "Show_Area",
    "Hide_for_Drilldown",
    "Suppress_if_Blank",
    "Background_Color",
    "Section_Height",
    "Can_Grow",
    "Section_Visibility",
    "Object_Visibility",
    "Back_Color",
    "Section_Back_Color",
    // Object-level conditional-format formulas and the internal selection-condition duplicates
    // (the real Record/Group selection formulas use space-separated names, handled above).
    "Font_Color",
    "Font_Style",
    // A field object's Display-String formula (its body uses the `currentfieldvalue` keyword, valid
    // only in a field-format formula) and Fore/font-Colour formula. These are reserved engine names
    // a user formula field cannot take; they attach to the object, not the formula list.
    "Display_String",
    "Fore_Color",
    // A PictureObject's dynamic graphic-location formula; reserved, attaches to the object, not the
    // formula list.
    "Graphic_Location",
    "Record_Selection",
    "Group_Selection",
];

/// Whether the engine will type-check this formula body as `UnknownField`/length 0 at load time:
/// (A) it references a `{alias.field}` not in the live database schema (case-insensitive), or
/// (B) it calls `GroupName(` while the report defines no groups. Either makes the persisted type in
/// the `0x71` record stale.
pub(super) fn formula_is_stale(
    body: &str,
    known_db_fields: &std::collections::HashSet<String>,
    groups: &[Group],
) -> bool {
    // Condition A — a database-field reference (`{alias.field}`, not a `{?param}`/`{@formula}`)
    // that the current schema no longer contains.
    let mut rest = body;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('}') else { break };
        let token = &rest[..close];
        rest = &rest[close + 1..];
        if !token.starts_with('?')
            && !token.starts_with('@')
            && token.contains('.')
            && !known_db_fields.contains(&token.to_lowercase())
        {
            return true;
        }
    }
    // Condition B — `GroupName(` with no groups defined (the call has no active group to name).
    if groups.is_empty() {
        let low = body.to_lowercase();
        let mut from = 0;
        while let Some(g) = low[from..].find("groupname") {
            from += g + "groupname".len();
            if low[from..].trim_start().starts_with('(') {
                return true;
            }
        }
    }
    false
}

/// Pair each formula body (`0x76`) with the **named value** (`0x71`) that immediately follows it
/// in document order — the engine stores a formula as `[body][name]`. Classify by the name:
/// the report's selection formulas, the per-group display formulas (skipped — synthesised as
/// `GroupNameFieldDefinition`s), section conditional formulas (skipped), and the user formula
/// fields (`{@name}`).
pub(super) fn raise_formulas(
    tree: &[RecordNode],
    logical: &[u8],
    known_db_fields: &std::collections::HashSet<String>,
    groups: &[Group],
) -> Formulas {
    let nodes = nodes_where(tree, |n| n.rtype == FORMULA || n.rtype == NAMED_VALUE);
    let mut out = Formulas::default();
    let mut pending: Option<(
        String,
        crate::model::FormulaSyntax,
        crate::model::FormulaNullTreatment,
    )> = None;
    for n in nodes {
        if n.rtype == FORMULA {
            let leaf = n.leaf_bytes(logical);
            pending = Some((
                formula_body(n, logical),
                formula_syntax(&leaf),
                formula_null_treatment(&leaf),
            ));
            continue;
        }
        // NAMED_VALUE: names the pending body, if any (db-field/parameter names have none).
        let Some((body, syntax, null_treatment)) = pending.take() else {
            continue;
        };
        let Some((name, after)) = read_lp_string(&n.leaf_bytes(logical)) else {
            continue;
        };
        match name.as_str() {
            "Record Selection" | "Record_Selection" => out.record_selection = Some(body),
            "Group Selection" | "Group_Selection" => out.group_selection = Some(body),
            "Saved Data Selection" => out.saved_data_filter = Some(body),
            // Group/grid order records ("Group #1 Order", "… Grid #3 Order") — match the full
            // pattern (a `#N` index and the " Order" suffix), not merely " #", so a user formula
            // legitimately named with a trailing " #" is not dropped.
            n if n.contains(" #") && n.ends_with(" Order") => {}
            // A cross-tab with no summarized field carries an empty-bodied "No Summarized Field"
            // placeholder in the formula stream — an internal sentinel, not a user formula field (the
            // engine omits it from its formula collection), so drop it. Gated on an empty body so a
            // real user formula that happened to take this name is not lost.
            "No Summarized Field" if body.is_empty() => {}
            // Running-total eval/reset condition formulas and section/object conditional formulas:
            // not user field definitions, but their bodies are real stored formula text (and may
            // reference parameters), so keep them — they are stored facts nothing else records.
            n if n.ends_with(" Condition Formula") || SECTION_FORMULA_NAMES.contains(&n) => {
                if !body.is_empty() {
                    if n.ends_with(" Condition Formula") {
                        out.running_total_condition_formulas.push(body.clone());
                    }
                    out.condition_formula_bodies.push(body);
                }
            }
            // A custom function: its body opens with the reserved `Function (args) …` header. The
            // engine lists these under `CustomFunctions`, not the formula-field collection — the
            // `0x71` name is the function's identifier and the body carries its arg list + return type.
            _ if is_custom_function_body(&body) => {
                out.custom_functions.push(crate::model::CustomFunction {
                    name,
                    syntax,
                    text: body,
                });
            }
            _ => {
                // A user formula field. The engine re-compiles every formula at load time; one
                // that references a database field no longer in the schema, or calls `GroupName()`
                // with no groups defined, fails to type-check and is reported as UnknownField/0 —
                // overriding the (now stale) type and length the `0x71` record still carries.
                let (value_type, number_of_bytes) =
                    if formula_is_stale(&body, known_db_fields, groups) {
                        (FieldValueType::Unknown, 0)
                    } else {
                        let leaf = n.leaf_bytes(logical);
                        // Value type is the u16 (LE) right after the name.
                        let value_type = u16_le(&leaf, after)
                            .map(|v| FieldValueType::from_code(i32::from(v)))
                            .unwrap_or_default();
                        // NumberOfBytes is the engine-persisted `IField.Length` (RAS DispId 7): a fixed
                        // type uses its intrinsic size; a `String` result uses the record's **stored**
                        // width (trailing big-endian u32 at `after + 8`, past vt(2) + charCount(4) +
                        // flag(2)) — the last-saved length as it sits in the file. The engine sometimes
                        // *recomputes* this at load, but that recompute is runtime-gated and not
                        // reproducible from the file alone, so `rpt` emits the stored fact. The recompute
                        // model lives in `crystal_formula::string_max_bytes` for the eval/LSP paths
                        // that have runtime context. Capped at 32767 chars → 65534.
                        let number_of_bytes = if let Some(n) = value_type.byte_length() {
                            n
                        } else {
                            i32_be(&leaf, after + 8).unwrap_or(0).min(MAX_STRING_BYTES)
                        };
                        (value_type, number_of_bytes)
                    };
                out.user_formulas.push(FieldDef {
                    name,
                    value_type,
                    kind: FieldKindData::Formula(FormulaField {
                        text: Formula(body),
                        options: 0,
                        number_of_bytes,
                        syntax,
                        null_treatment,
                    }),
                    ..Default::default()
                });
            }
        }
    }
    // A formula name is unique in a report, but the engine stores the compiled body once per use, so
    // the same `{@name}` can appear several times in the stream. The SDK exposes each formula field
    // once — dedupe by name, keeping the first occurrence (preserves the engine's emit order).
    {
        let mut seen = std::collections::HashSet::new();
        out.user_formulas.retain(|f| seen.insert(f.name.clone()));
    }
    out
}

/// Raise running-total field definitions. Each is a `0x7e` record (byte 0 = operation, then the
/// summarized-field reference) immediately preceded by its `0x80` reset record (byte 0 = reset
/// condition); the `0x7e`'s `0x71` child names it and gives its value type + byte length. A
/// standalone `0x7e` (no preceding `0x80`) is a summary, handled elsewhere.
/// The `SecondarySummarizedField` of a `0x7e` summary / running-total record, or an empty string when
/// there is none. `off` is the leaf offset immediately after the primary field reference.
///
/// Each summarized field is serialized as a length-prefixed field reference followed by a fixed 3-byte
/// value descriptor (`[type][00][bytes]`, e.g. `01 00 04` for a Number formula, `00 00 03` for a
/// database field). A record that carries a second summarized field writes a full second field
/// serialization right after the primary's descriptor — so the secondary field reference begins at
/// `off + 3`. A record with no second field instead writes the record's fixed no-field trailer there
/// (`00 00 00 01 00 00 ff ff …`), whose leading `u32` reads as an empty/short non-field string that
/// `is_field_ref` rejects.
///
/// This is a stored fact, not gated on the operation: a running total added through
/// `RunningTotalFieldController.Add` self-mirrors its secondary field to equal the primary, so the
/// second serialization is present and equals the first; one added in the designer, and a plain
/// summary, omit it. Reading the actual second field reproduces both cases (emit `==primary` vs
/// omit) directly from the bytes.
fn read_secondary_field(leaf: &[u8], off: usize) -> String {
    leaf.get(off + 3..)
        .and_then(read_lp_string)
        .map(|(s, _)| s)
        .filter(|s| is_field_ref(s))
        .unwrap_or_default()
}

pub(super) fn raise_running_totals(tree: &[RecordNode], logical: &[u8]) -> Vec<FieldDef> {
    let nodes = flatten(tree);
    let mut out = Vec::new();
    for i in 0..nodes.len() {
        let node = nodes[i];
        if node.rtype != SUMMARY_DEF {
            continue;
        }
        // A running total is the operation record preceded by its reset record.
        let Some(reset_node) = i.checked_sub(1).map(|p| nodes[p]) else {
            continue;
        };
        if reset_node.rtype != RT_RESET {
            continue;
        }
        let leaf = node.leaf_bytes(logical);
        let operation = SummaryOperation::from_code(i32::from(leaf.first().copied().unwrap_or(0)));
        // Leaf byte 0 = operation, byte 1 = a constant `0` separator, bytes 2..4 = the operation
        // parameter (`u16` BE, the archive's scalar convention): the N of NthLargest/NthSmallest/
        // NthMostFrequent or the target percentile of Percentile; 0 for every other operation.
        let operation_parameter = u16_be(&leaf, 2).map_or(0, i32::from);
        let (summarized_field, primary_consumed) =
            leaf.get(4..).and_then(read_lp_string).unwrap_or_default();
        // A running total added through the controller API self-mirrors its secondary summarized field
        // to the primary, storing a full second field serialization after the primary's 3-byte value
        // descriptor; one added in the designer, and a plain summary, omit it. See
        // [`read_secondary_field`].
        let secondary_summarized_field = read_secondary_field(&leaf, 4 + primary_consumed);
        // The `0x71` child: name + value-type code (at the byte after the name) + NumberOfBytes.
        let Some(child) = node.children.iter().find(|c| c.rtype == NAMED_VALUE) else {
            continue;
        };
        let cb = child.leaf_bytes(logical);
        let Some((name, used)) = read_lp_string(&cb) else {
            continue;
        };
        let value_type = FieldValueType::from_code(i32::from(cb.get(used).copied().unwrap_or(0)));
        // A running total always reports its result as a plain number; the engine widens a Currency
        // summarized field (the stored type byte is Currency) to NumberField.
        let value_type = match value_type {
            FieldValueType::Currency => FieldValueType::Number,
            other => other,
        };
        let length = i32::from(cb.get(used + 2).copied().unwrap_or(0));
        // `0x80`: byte 0 is the reset condition, byte 3 the evaluation condition (same coding).
        let reset_bytes = reset_node.leaf_bytes(logical);
        let reset =
            ResetConditionType::from_code(i32::from(reset_bytes.first().copied().unwrap_or(0)));
        // A formula- or field-driven evaluation stores no code at byte 3: it embeds the driver as a
        // length-prefixed reference at byte 2, whose length prefix overruns byte 3 (reading 0 =
        // NoCondition). When reset is NoCondition and such a reference is present, its kind picks the
        // condition: an `@`-prefixed formula → `OnFormula`; a `table.field` reference →
        // `OnChangeOfField`. Otherwise byte 3 holds the code directly.
        use crate::model::EvaluationConditionType as Eval;
        let ref_at_2 = reset_bytes
            .get(2..)
            .and_then(read_lp_string)
            .map(|(s, _)| s);
        let evaluation = match ref_at_2 {
            Some(s) if reset == ResetConditionType::NoCondition && s.starts_with('@') => {
                Eval::OnFormula
            }
            Some(s)
                if reset == ResetConditionType::NoCondition
                    && s.contains('.')
                    && !s.starts_with('@') =>
            {
                Eval::OnChangeOfField
            }
            _ => Eval::from_code(i32::from(reset_bytes.get(3).copied().unwrap_or(0))),
        };
        // An `OnChangeOfField` evaluate/reset condition names the field whose change drives it in the
        // `0x80` record's own leaf (a field-shaped LP string, e.g. `table.field`). An
        // `OnChangeOfGroup`/`OnFormula`/`NoCondition` condition has no such direct field ref here.
        let on_change_field = if reset == ResetConditionType::OnChangeOfField
            || evaluation == crate::model::EvaluationConditionType::OnChangeOfField
        {
            own_lp_strings(reset_node, logical)
                .into_iter()
                .find(|s| is_field_ref(s))
                .unwrap_or_default()
        } else {
            String::new()
        };
        out.push(FieldDef {
            name,
            value_type,
            length,
            kind: FieldKindData::RunningTotal(RunningTotalField {
                operation,
                summarized_field,
                secondary_summarized_field,
                operation_parameter,
                evaluation,
                reset,
                on_change_field,
            }),
            ..Default::default()
        });
    }
    out
}

/// Raise **summary** field definitions (`ISummaryField`). Each is a standalone `0x7e` record (not
/// preceded by a `0x80` running-total reset) appearing before the report layout: byte 0 is the
/// aggregate operation, the summarized field is a length-prefixed reference at byte 4, and the
/// `0x71` child is the fixed header `00 00 00 01 00 <vt> 00 <nbytes>` (value-type code at offset 5,
/// byte length at offset 7). Unlike a running total, a summary carries no stored name, and its group
/// scope is not in the record — the same summary definition is stored once per placement (group
/// footer / report footer), so `group_index` is left `None`; the placement recovers the scope.
pub(super) fn raise_summaries(tree: &[RecordNode], logical: &[u8]) -> Vec<FieldDef> {
    let mut out = Vec::new();
    for node in summary_def_nodes(tree, true) {
        let leaf = node.leaf_bytes(logical);
        let operation = SummaryOperation::from_code(i32::from(leaf.first().copied().unwrap_or(0)));
        // Leaf byte 0 = operation, byte 1 = a constant `0` separator, bytes 2..4 = the operation
        // parameter (`u16` BE): the N of NthLargest/NthSmallest/NthMostFrequent or the target
        // percentile of Percentile; 0 for every other operation.
        let operation_parameter = u16_be(&leaf, 2).map_or(0, i32::from);
        let Some((summarized_field, primary_consumed)) = leaf.get(4..).and_then(read_lp_string)
        else {
            continue;
        };
        // A two-field summary stores its second field reference after the primary's 3-byte value
        // descriptor; a single-field summary stores the no-field trailer there instead. See
        // [`read_secondary_field`].
        let secondary_summarized_field = read_secondary_field(&leaf, 4 + primary_consumed);
        // A fixed tail follows the field ref(s): `… ff ff 00 <flag> …`, where the byte 12 past the end
        // of the primary field ref is the IsPercentageSummary flag (0 = raw aggregate, 1 = shown as a
        // percentage of a group total). A percentage summary carries two extra trailing bytes.
        let is_percentage_summary = leaf.get(4 + primary_consumed + 12).is_some_and(|&b| b != 0);
        // The `0x71` child fixes the value type (offset 5) and byte length (offset 7).
        let (value_type, length) = node
            .children
            .iter()
            .find(|c| c.rtype == NAMED_VALUE)
            .map(|c| c.leaf_bytes(logical))
            .map(|cb| {
                (
                    FieldValueType::from_code(i32::from(cb.get(5).copied().unwrap_or(0))),
                    i32::from(cb.get(7).copied().unwrap_or(0)),
                )
            })
            .unwrap_or_default();
        out.push(FieldDef {
            value_type,
            length,
            kind: FieldKindData::Summary(SummaryField {
                operation,
                summarized_field,
                secondary_summarized_field,
                operation_parameter,
                group_index: None,
                is_percentage_summary,
            }),
            ..Default::default()
        });
    }
    out
}

/// SQL Expression field definitions (`0x81` records). A SQL Expression is a snippet of raw SQL
/// evaluated by the database and referenced from the report as `{%Name}`.
///
/// Layout: the record's own leaf leads with the SQL expression **text** as a length-prefixed string
/// (empty for an unbound/blank expression). Its `0x71` `NamedValue` child carries the field **name**
/// (length-prefixed), then the value-type code (`u16` LE immediately after the name) and the byte
/// **length** (`u16` BE in the child's final two bytes) — the same trailing type/length shape as a
/// summary's named-value child.
pub(super) fn raise_sql_expressions(tree: &[RecordNode], logical: &[u8]) -> Vec<FieldDef> {
    let mut out = Vec::new();
    for node in nodes_where(tree, |n| n.rtype == SQL_EXPRESSION) {
        let leaf = node.leaf_bytes(logical);
        let text = read_lp_string(&leaf).map(|(s, _)| s).unwrap_or_default();
        let Some(child) = node.children.iter().find(|c| c.rtype == NAMED_VALUE) else {
            continue;
        };
        let cb = child.leaf_bytes(logical);
        let Some((name, after)) = read_lp_string(&cb) else {
            continue;
        };
        let value_type = u16_le(&cb, after)
            .map(|v| FieldValueType::from_code(i32::from(v)))
            .unwrap_or_default();
        let length = if cb.len() >= 2 {
            u16_be(&cb, cb.len() - 2).map_or(0, i32::from)
        } else {
            0
        };
        out.push(FieldDef {
            name: name.clone(),
            value_type,
            length,
            short_name: Some(name),
            kind: FieldKindData::SqlExpression(crate::model::SqlExpressionField { text }),
            ..Default::default()
        });
    }
    out
}

/// The body text of a formula record (`0x76`), read structurally past the dependency list (see
/// [`parse_formula_record`]). Falls back to the longest expression-like string when the
/// structure does not parse (older/atypical records). Empty when the slot has no formula.
pub(super) fn formula_body(node: &RecordNode, logical: &[u8]) -> String {
    let bytes = node.leaf_bytes(logical);
    if let Some((body, _)) = parse_formula_record(&bytes) {
        return body;
    }
    let strings = all_strings(node, logical);
    // A byte-presence heuristic, not reference parsing — `rpt` is pure I/O and does not depend on
    // `crystal-formula`, so this stays a local check rather than using the shared reference walker.
    let is_expr = |s: &&String| {
        s.contains('{')
            || s.contains(" & ")
            || s.contains('\n')
            || s.contains('(')
            || s.contains('"')
    };
    strings
        .iter()
        .filter(is_expr)
        .max_by_key(|s| s.len())
        .cloned()
        .unwrap_or_default()
}

/// Parse a `0x76` formula record structurally:
/// `[u16-BE ref-count N][N × (LP field-ref + 3-byte separator)][LP body][trailer]`.
/// Returns the body and the trailer offset, or `None` when the layout is implausible (so
/// callers can fall back).
pub(super) fn parse_formula_record(bytes: &[u8]) -> Option<(String, usize)> {
    let mut c = Cursor::new(bytes);
    let n = c.u16_be()? as usize;
    // A real dependency list cannot exceed the record; reject absurd counts (mis-parse / not 0x76).
    if n > bytes.len() / 5 {
        return None;
    }
    for _ in 0..n {
        c.lp_string()?;
        c.skip(3); // 3-byte inter-reference separator
    }
    let body = c.lp_string()?;
    Some((body, c.pos()))
}

/// The formula's authoring dialect. In a `0x76` record, byte 16 of the trailer (after the dependency
/// list and body string) is `1` for Basic, else Crystal. Defaults to Crystal if the layout doesn't parse.
pub(super) fn formula_syntax(bytes: &[u8]) -> crate::model::FormulaSyntax {
    use crate::model::FormulaSyntax;
    match parse_formula_record(bytes) {
        Some((_, trailer_start)) if bytes.get(trailer_start + 16) == Some(&1) => {
            FormulaSyntax::Basic
        }
        _ => FormulaSyntax::Crystal,
    }
}

/// The formula's null-treatment editor setting (SDK `FormulaNullTreatment`). In a `0x76` record,
/// byte 17 of the trailer is `1` when the formula treats a null field value as its type's default
/// (`crTreatNullAsDefaultValue`); otherwise the engine default `crTreatNullAsException` applies.
pub(super) fn formula_null_treatment(bytes: &[u8]) -> crate::model::FormulaNullTreatment {
    use crate::model::FormulaNullTreatment;
    match parse_formula_record(bytes) {
        Some((_, trailer_start)) if bytes.get(trailer_start + 17) == Some(&1) => {
            FormulaNullTreatment::DefaultValue
        }
        _ => FormulaNullTreatment::Exception,
    }
}

/// A group record (`0xe5`): its first length-prefixed string is the group's condition field
/// (`Table.column`). Each group carries a group sort, ascending by default.
/// The two shapes a `0x29` sort record can take.
pub(super) enum SortRecord {
    /// A plain record-level sort (`RecordSortField`).
    Record(Sort),
    /// A group's summary-based sort: `operand` is the summary display form (`Sum of {field}`) bound
    /// to the owning group. `dir_byte` is the raw direction byte; its meaning (TopN/BottomN vs
    /// Descending/Ascending) depends on the group's Top N limit, resolved when bound to the group.
    GroupSummary { operand: String, dir_byte: u8 },
}

/// Raise a `0x29` sort record: a length-prefixed field reference then a trailer whose first byte is
/// a marker — `0x00` = plain record sort (dir 0 asc / 1 desc); `0x02` = group summary sort (its
/// direction depends on the group's Top N limit, so it is resolved later, not here).
pub(super) fn raise_sort(node: &RecordNode, logical: &[u8]) -> Option<SortRecord> {
    let bytes = node.leaf_bytes(logical);
    let (field, consumed) = read_lp_string(&bytes)?;
    if field.is_empty() {
        return None;
    }
    let dir_byte = bytes.last().copied().unwrap_or(0);
    if bytes.get(consumed).copied() == Some(0x02) {
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

/// A group's Top N / Bottom N limit `N`: a big-endian `u16` 11 bytes from the end of its `0xe5`
/// record. `N > 0` = ordered Top N / Bottom N; `N == 0` = ordered by summary asc/desc (or by the
/// group field). Returns 0 when the tail is too short.
pub(super) fn group_topn_limit(node: &RecordNode, logical: &[u8]) -> u16 {
    let bytes = node.leaf_bytes(logical);
    bytes
        .len()
        .checked_sub(11)
        .and_then(|i| u16_be(&bytes, i))
        .unwrap_or(0)
}

/// Number of bytes between the end of a `0xe5` group's field reference and its Top N "Others"-bucket
/// name: `[u32 group ordinal][u16 pad]`, then the length-prefixed `NotInTopBottomNName`.
const E5_OTHERS_NAME_OFFSET: usize = 6;

/// Decode a summary-based group sort's Top N / Bottom N options (SDK `TopBottomNSortField`) from the
/// group's `0xe5` record. `number_of_groups` is the group's Top N limit (already resolved by the
/// caller via [`group_topn_limit`]); `not_in_topn_name` is the length-prefixed "Others"-bucket name
/// that follows the field reference. Just past that name the record carries `[u16 BE N][u16 BE
/// DiscardOthers]`: `N` (a duplicate of the limit) then a `DiscardOthers` flag short (SDK
/// `EnableDiscardOtherGroups`, `1` = set), read here from the short's low byte at name-end + 3.
///
/// `WithTies` (the designer "Include ties" option) has no located byte in this record and is left
/// `false`; see the `with_ties_defaults_false` test for the rationale.
fn decode_group_topn(
    node: &RecordNode,
    logical: &[u8],
    number_of_groups: u16,
) -> crate::model::TopBottomNSort {
    let bytes = node.leaf_bytes(logical);
    let (not_in_topn_name, discard_others) = (|| {
        let (_, fieldref_consumed) = read_lp_string(&bytes)?;
        let name_start = fieldref_consumed + E5_OTHERS_NAME_OFFSET;
        let (name, name_consumed) = read_lp_string(bytes.get(name_start..)?)?;
        // Flags sit just past the name: a u16 N duplicate, then the DiscardOthers flag short.
        let flags_at = name_start + name_consumed;
        let discard_others = bytes.get(flags_at + 3).copied() == Some(1);
        Some((name, discard_others))
    })()
    .unwrap_or_default();
    crate::model::TopBottomNSort {
        number_of_groups,
        discard_others,
        not_in_topn_name,
        with_ties: false,
    }
}

/// Resolve a group summary sort's direction from its direction byte and Top N limit: limited
/// (`N > 0`) → TopN (`1`) / BottomN (`0`); unlimited (`N == 0`) → Descending (`1`) / Ascending (`0`).
fn group_sort_direction(dir_byte: u8, topn_limit: u16) -> crate::model::SortDirection {
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
fn render_group_sort_summary(operand: &str, group_field: &str) -> String {
    match operand.split_once(" of ") {
        Some((op, summed)) => {
            let op = summary_op_full(op);
            format!("{op} ({{{summed}}}, {{{group_field}}})")
        }
        None => operand.to_string(),
    }
}

/// Decode a `0x0088` GroupAreaFormat record (the group's AreaPair). The engine serializes big-endian
/// scalars up front: a `u16` RepeatGroupHeader flag at leaf `[0..2]`, a `u16` KeepGroupTogether flag
/// at leaf `[2..4]`, and a `u16` VisibleGroupNumberPerPage ("keep N groups together per page"; `0` =
/// off) at leaf `[4..6]`. Leaf `[6..8]` is a separate (unemitted) group property, so the count must
/// not span it.
pub(crate) fn decode_group_area_format(lb: &[u8]) -> crate::model::GroupAreaFormat {
    let flag = |off: usize| u16_be(lb, off).unwrap_or(0) != 0;
    crate::model::GroupAreaFormat {
        repeat_group_header: flag(0),
        keep_group_together: flag(2),
        visible_groups_per_page: i32::from(u16_be(lb, 4).unwrap_or(0)),
    }
}

/// Decode the **Hierarchical Grouping** block appended to a `0xe5` group leaf (SDK `IArea`
/// `EnableHierarchicalGroupSorting` / `ParentIDField` / `InstanceIDField`). The block sits after the
/// group's `@Group #N Order` marker + boilerplate as `[flag 0x01][LP ParentIDField][00 00 01]
/// [LP InstanceIDField]`, where each field is a bare `table.field` (NUL-terminated, `u32`-BE length).
/// Its absolute offset varies with the condition-field name length, so it is anchored on the marker
/// string rather than a fixed offset. Returns `None` for a plain (non-hierarchical) group, which
/// carries no such trailing field references. `group_indent` is filled later from the group's
/// `0x0088` record (not in this leaf).
pub(super) fn decode_hierarchical_options(
    leaf: &[u8],
) -> Option<crate::model::HierarchicalGroupOptions> {
    let strings: Vec<(usize, String, usize)> = lp_scan(leaf, Scan::Consume).collect();
    // The hierarchical field references follow the `@Group #N Order` marker.
    let order_pos = strings
        .iter()
        .position(|(_, s, _)| s.starts_with("@Group #") && s.ends_with(" Order"))?;
    // Bare `table.field` references after the marker: ParentIDField then InstanceIDField.
    let mut refs = strings[order_pos + 1..]
        .iter()
        .filter(|(_, s, _)| !s.is_empty() && s.contains('.') && !s.contains('{'));
    let parent = refs.next()?;
    let instance = refs.next()?;
    // Enable flag: the byte immediately before the ParentIDField length prefix (`0x01` = enabled).
    let enabled = parent.0.checked_sub(1).and_then(|i| leaf.get(i)).copied() == Some(0x01);
    if !enabled {
        return None;
    }
    Some(crate::model::HierarchicalGroupOptions {
        enabled: true,
        parent_id_field: parent.1.clone(),
        instance_id_field: instance.1.clone(),
        group_indent: crate::model::Twips(0),
    })
}

/// The per-level `GroupIndent`, in twips: a big-endian `u16` at leaf `[6..8]` of the `0x0088`
/// GroupAreaFormat record (`0` for a non-hierarchical group). This slot is deliberately excluded
/// from [`decode_group_area_format`], which stops at `[6]`.
pub(crate) fn decode_group_indent(lb: &[u8]) -> crate::model::Twips {
    crate::model::Twips(i32::from(u16_be(lb, 6).unwrap_or(0)))
}

pub(super) fn raise_group(
    node: &RecordNode,
    logical: &[u8],
    field_types: &std::collections::HashMap<String, crate::model::FieldValueType>,
) -> Option<Group> {
    // The `0xe5` leaf begins with the condition-field reference, then `[u32 order-id][00][dir]`
    // where `dir` is the group's sort direction (0 ascending, 1 descending, 2 unsorted).
    let bytes = node.leaf_bytes(logical);
    let (field, used) = read_lp_string(&bytes)?;
    if field.is_empty() {
        return None;
    }
    // A `0xe5` record also encodes chart / cross-tab "grid" groups, which are scoped to their
    // object — not the report's `DataDefinition.Groups`. A real report group carries an
    // `@Group #N Order` marker string; grid groups carry `@… Grid #N Order` instead.
    let is_report_group = all_strings(node, logical)
        .iter()
        .any(|s| s.starts_with("@Group #") && s.ends_with(" Order"));
    if !is_report_group {
        return None;
    }
    let direction = crate::model::SortDirection::from_code(i32::from(
        bytes.get(used + 5).copied().unwrap_or(0),
    ));
    // Grouping condition. Crystal's group condition is polymorphic (see `GroupCondition`), selected
    // by the group field's value type; only a Date/Time/DateTime or Boolean field carries one, so the
    // field type gates it (`condition_field` is `Alias.name`, looked up case-insensitively).
    //
    // The condition ordinal lives in the `0xe5` leaf at `used + 3` (the byte after the field
    // reference). A Boolean field reads it through the `CrBooleanConditionEnum` table; a date/time
    // field through `CrDateConditionEnum`, with two legacy fallbacks for reports that leave that byte
    // at `0`:
    //  * the internal `<code>` after the `@Group #N Order` marker (`01 00 <code> ff ff`), and
    //  * the older `used + 4 == 0x02` daily flag.
    // Discrete grouping leaves all of these clear. (Splitting Boolean out of the date table also fixes
    // a misread where a boolean ordinal 1–6 was decoded as a date period.)
    use crate::model::{FieldValueType, GroupCondition};
    let sdk_ordinal = bytes.get(used + 3).copied();
    let date_condition = match field_types.get(&field.to_lowercase()) {
        Some(FieldValueType::Boolean) => sdk_ordinal.and_then(GroupCondition::from_boolean_ordinal),
        Some(FieldValueType::Date | FieldValueType::Time | FieldValueType::DateTime) => {
            let period_code = lp_scan(&bytes, Scan::Consume)
                .find(|(_, s, _)| s.starts_with("@Group #") && s.ends_with(" Order"))
                .and_then(|(i, _, consumed)| bytes.get(i + consumed + 2).copied());
            sdk_ordinal
                .and_then(GroupCondition::from_date_ordinal)
                .or_else(|| period_code.and_then(GroupCondition::from_legacy_date_code))
                .or_else(|| {
                    (bytes.get(used + 4).copied() == Some(0x02)).then_some(GroupCondition::Daily)
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
        // Populated by the off-by-one `0x0088` pass in `raise_data_definition`.
        area_format: Default::default(),
        // Populated by the `0x00e9` pass in `raise_data_definition` (specified-order groups only).
        hierarchical: Vec::new(),
        // The Hierarchical-Grouping block is appended to this same `0xe5` leaf; its `group_indent`
        // is filled from the group's `0x0088` record in `raise_data_definition`.
        hierarchical_options: decode_hierarchical_options(&bytes),
    })
}

#[cfg(test)]
mod hierarchical_tests {
    use super::decode_hierarchical_value;

    /// A `0x00e9` leaf is two `u32`-BE-length-prefixed (NUL-terminated) strings.
    fn lp(s: &str) -> Vec<u8> {
        let mut v = ((s.len() + 1) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v.push(0);
        v
    }

    #[test]
    fn decodes_value_name_and_condition() {
        let mut leaf = lp("X");
        leaf.extend(lp("{Command.some_field} = \"X\""));
        let v = decode_hierarchical_value(&leaf).expect("parse");
        assert_eq!(v.value_name, "X");
        assert_eq!(v.condition, "{Command.some_field} = \"X\"");
    }

    #[test]
    fn missing_condition_yields_empty() {
        let v = decode_hierarchical_value(&lp("Low")).expect("parse");
        assert_eq!(v.value_name, "Low");
        assert_eq!(v.condition, "");
    }
}

#[cfg(test)]
mod hierarchical_options_tests {
    use super::{decode_group_indent, decode_hierarchical_options};

    /// A `u32`-BE NUL-terminated length-prefixed string.
    fn lp(s: &str) -> Vec<u8> {
        let mut v = ((s.len() + 1) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v.push(0);
        v
    }

    /// Build a `0xe5`-style leaf: the condition field, the `@Group #1 Order` marker, then (optionally)
    /// the appended Hierarchical-Grouping block `[flag][LP parent][00 00 01][LP instance]`. Boilerplate
    /// separators (`01 00 00 ff ff`, padding) sit between the parts, as in the real record — the
    /// decoder anchors on the marker, so their exact bytes only need to keep the marker findable.
    fn e5_leaf(cond: &str, hier: Option<(&str, &str, u8)>) -> Vec<u8> {
        let mut v = lp(cond);
        v.extend([0x01, 0x00, 0x00, 0xff, 0xff]); // boilerplate before the marker
        v.extend(lp("@Group #1 Order"));
        v.extend([0x01, 0x00, 0x00, 0xff, 0xff, 0x00, 0x00, 0x00]); // trailing boilerplate
        if let Some((parent, instance, flag)) = hier {
            v.push(flag);
            v.extend(lp(parent));
            v.extend([0x00, 0x00, 0x01]);
            v.extend(lp(instance));
        }
        v
    }

    #[test]
    fn decodes_enabled_parent_and_instance() {
        let leaf = e5_leaf(
            "employee.employee_id",
            Some(("employee.manager_id", "employee.employee_id", 0x01)),
        );
        let h = decode_hierarchical_options(&leaf).expect("hierarchical");
        assert!(h.enabled);
        assert_eq!(h.parent_id_field, "employee.manager_id");
        assert_eq!(h.instance_id_field, "employee.employee_id");
        // `group_indent` is filled later from the `0x0088` record, not this leaf.
        assert_eq!(h.group_indent.0, 0);
    }

    #[test]
    fn plain_group_has_no_options() {
        // No appended block → not a hierarchical group.
        assert!(decode_hierarchical_options(&e5_leaf("region.name", None)).is_none());
    }

    #[test]
    fn flag_clear_is_not_hierarchical() {
        // The trailing field references exist but the enable flag is 0 → treated as non-hierarchical.
        let leaf = e5_leaf("t.f", Some(("t.parent", "t.f", 0x00)));
        assert!(decode_hierarchical_options(&leaf).is_none());
    }

    #[test]
    fn group_indent_reads_big_endian_u16_at_offset_6() {
        // `0x0088` leaf: RepeatHeader/KeepTogether/VisiblePerPage occupy [0..6]; GroupIndent is [6..8].
        let lb = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x2c, 0x39, 0x51];
        assert_eq!(decode_group_indent(&lb).0, 300);
        // Plain group: indent slot is zero.
        assert_eq!(decode_group_indent(&[0u8; 8]).0, 0);
    }
}

/// Decode a field-definition record: the nested string leaf holds the name followed by the
/// value-type and length attributes — `name(lp-string) value_type(u16 LE) … length(u16 BE)`.
pub(super) fn raise_field(node: &RecordNode, logical: &[u8]) -> Option<FieldDef> {
    // The name + attributes live in the record's deepest (string) leaf.
    let mut leaf = None;
    node.walk(&mut |n| {
        if leaf.is_none() && n.is_leaf() {
            let bytes = n.leaf_bytes(logical);
            if read_lp_string(&bytes).is_some() {
                leaf = Some(bytes);
            }
        }
    });
    let bytes = leaf?;
    let (short_name, after) = read_lp_string(&bytes)?;

    // Trailing attributes: value_type (u16 LE) at the start, byte length (u16 BE) at the end.
    let attrs = &bytes[after..];
    let value_type = u16_le(attrs, 0)
        .map(|v| FieldValueType::from_code(i32::from(v)))
        .unwrap_or_default();
    let length = if attrs.len() >= 12 {
        u16_be(attrs, attrs.len() - 2).map_or(0, i32::from)
    } else {
        0
    };

    Some(FieldDef {
        name: short_name.clone(),
        value_type,
        length,
        short_name: Some(short_name),
        kind: FieldKindData::Database(DbField::default()),
        ..Default::default()
    })
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
        // short. It is left false until a report that exercises it (WithTies=true) is available.
        assert!(!crate::model::TopBottomNSort::default().with_ties);
    }
}
