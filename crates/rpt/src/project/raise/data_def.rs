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
                pending_group_format = Some(decode_group_area_format(&node.leaf_bytes(logical)));
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

/// Decode the field-pool census from the `0x006e` `FieldManagerEntry` record (20-byte leaf, writer
/// `fldmgrbs`, one per report): `[u32 BE database_fields][u16 BE formula-body-count-less-3][…]`.
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
/// STRUCTURAL — no SDK accessor exposes them, so they are not on the XML surface.
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
/// (`table.field` or `@formula`) of each is returned, in document order. The UseCount counter
/// reconciles these against the placed summaries to recover orphan summary definitions (see
/// `DataDefinition.summary_binding_fields`).
pub(super) fn raise_summary_bindings(tree: &[RecordNode], logical: &[u8]) -> Vec<String> {
    let nodes = flatten(tree);
    let mut out = Vec::new();
    for i in 0..nodes.len() {
        let node = nodes[i];
        // The layout region begins at the first area marker; summary definitions all precede it.
        if node.rtype == AREA_MARKER {
            break;
        }
        if node.rtype != SUMMARY_DEF {
            continue;
        }
        // A running total is a `0x7e` immediately preceded by its `0x80` reset record — not a summary.
        if i > 0 && nodes[i - 1].rtype == RT_RESET {
            continue;
        }
        // The summarized field is the first field-shaped length-prefixed string in the record's own
        // leaf (`table.field` or `@formula`); the operation byte precedes it and the name is a child.
        if let Some(f) = own_lp_strings(node, logical)
            .into_iter()
            .find(|s| s.contains('.') || s.starts_with('@'))
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
    /// conditional formulas) that are not user field definitions — kept for usage aggregation.
    condition_formula_bodies: Vec<String>,
    /// Subset of `condition_formula_bodies`: only the running-total **condition** formulas (names
    /// ending `" Condition Formula"`). Kept separately because, unlike the section/object conditional
    /// formulas, these are *not* attached to any section/object — so the UseCount counter must scan
    /// them on their own to count their DB-field references without double-counting the attached ones.
    running_total_condition_formulas: Vec<String>,
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
    let mut pending: Option<(String, crate::model::FormulaSyntax)> = None;
    for n in nodes {
        if n.rtype == FORMULA {
            let leaf = n.leaf_bytes(logical);
            pending = Some((formula_body(n, logical), formula_syntax(&leaf)));
            continue;
        }
        // NAMED_VALUE: names the pending body, if any (db-field/parameter names have none).
        let Some((body, syntax)) = pending.take() else {
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
            // Running-total eval/reset condition formulas and section/object conditional formulas:
            // not user field definitions, but their bodies are real stored formula text (and may
            // reference parameters), so keep them for the export layer's usage aggregation.
            n if n.ends_with(" Condition Formula") || SECTION_FORMULA_NAMES.contains(&n) => {
                if !body.is_empty() {
                    if n.ends_with(" Condition Formula") {
                        out.running_total_condition_formulas.push(body.clone());
                    }
                    out.condition_formula_bodies.push(body);
                }
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
        let summarized_field = leaf
            .get(4..)
            .and_then(read_lp_string)
            .map(|(s, _)| s)
            .unwrap_or_default();
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
        // `0x80` record's own leaf (a field-shaped LP string, e.g. `table.field`). The engine holds it
        // as a persistent field reference (it counts toward that field's UseCount); an
        // `OnChangeOfGroup`/`OnFormula`/`NoCondition` condition has no such direct field ref here.
        let on_change_field = if reset == ResetConditionType::OnChangeOfField
            || evaluation == crate::model::EvaluationConditionType::OnChangeOfField
        {
            own_lp_strings(reset_node, logical)
                .into_iter()
                .find(|s| s.contains('.') || s.starts_with('@'))
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
                operation_parameter: 0,
                evaluation,
                reset,
                on_change_field,
            }),
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
/// that follows the field reference. `discard_others` has no located byte (always the default
/// `false`), see [`crate::model::TopBottomNSort`].
fn decode_group_topn(
    node: &RecordNode,
    logical: &[u8],
    number_of_groups: u16,
) -> crate::model::TopBottomNSort {
    let bytes = node.leaf_bytes(logical);
    let not_in_topn_name = read_lp_string(&bytes)
        .and_then(|(_, consumed)| read_lp_string(bytes.get(consumed + E5_OTHERS_NAME_OFFSET..)?))
        .map(|(name, _)| name)
        .unwrap_or_default();
    crate::model::TopBottomNSort {
        number_of_groups,
        discard_others: false,
        not_in_topn_name,
        // The WithTies byte has not been located, so it defaults to false (see `TopBottomNSort`).
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

/// Decode a `0x0088` GroupAreaFormat record (24 bytes; describes the *next* group): byte 1 is
/// RepeatGroupHeader, byte 3 KeepGroupTogether, byte 15 a VisibleGroupNumberPerPage>0 flag (only
/// 0/1 is currently decoded; the full integer location is unknown).
pub(super) fn decode_group_area_format(lb: &[u8]) -> crate::model::GroupAreaFormat {
    let flag = |i: usize| lb.get(i).copied().unwrap_or(0) != 0;
    crate::model::GroupAreaFormat {
        repeat_group_header: flag(1),
        keep_group_together: flag(3),
        visible_groups_per_page: i32::from(flag(15)),
    }
}

/// Map the legacy internal date-group period `<code>` (the byte after the `@Group #N Order` marker,
/// structure `01 00 <code> ff ff`) to the lowercase period name the engine renders as the second
/// `GroupName` operand (`GroupName ({field}, "Monthly")`, title-cased on emit). Returns `None` for an
/// unknown code (never panics).
///
/// `0x01` = daily, `0x03` = monthly, `0x06`/`0x08` = weekly (there are two week codes — week-of-year
/// vs. a fixed-weekday week — that both render "Weekly").
///
/// This internal `<code>` is *not* the SDK `CrGroupConditionEnum` ordinal — that ordinal lives in a
/// separate byte ([`sdk_period_name`], `used + 3`). Some reports leave this `<code>` at `0` and carry
/// the period only in the SDK-ordinal byte, so both are consulted.
fn date_period_name(code: u8) -> Option<&'static str> {
    match code {
        0x01 => Some("daily"),
        0x03 => Some("monthly"),
        0x06 | 0x08 => Some("weekly"),
        _ => None,
    }
}

/// Map the SDK `CrGroupConditionEnum` ordinal — stored in the `0xe5` leaf at `used + 3` (the byte
/// after the group's field reference) — to the lowercase period name. This byte is populated
/// consistently (unlike the legacy [`date_period_name`] `<code>`, which some reports leave at `0`),
/// so it is the primary period source.
///
/// Ordinal `1` = weekly, `4` = monthly. Ordinal `0` = daily, but `0` is also the value on
/// non-periodic groups, so daily is *not* mapped here (to avoid a false positive on a discrete group)
/// and is detected via the legacy paths instead. Ordinals `2`/`3`/`5`/`6`/`7` (biweekly /
/// semimonthly / quarterly / semiannually / annually) are the remaining *date* periods; `8`–`11`
/// (per second / minute / hour / AM-PM) are the *time-of-day* periods a Time/DateTime field also
/// offers — appended after the eight date periods. These extra ordinals follow the SDK enum ordering
/// and never collide with the no-period state, so they are safe to map (they only ever fire on a
/// genuine periodic group).
///
/// Returns the lowercase canonical token stored on [`Group::date_condition`]; the exact title-cased
/// `GroupName` operand string (e.g. `SemiAnnually`, `BySecond`) is produced at emit time, since this
/// same lowercase token is also matched by the render pipeline's date bucketer.
fn sdk_period_name(ordinal: u8) -> Option<&'static str> {
    match ordinal {
        1 => Some("weekly"),
        2 => Some("biweekly"),
        3 => Some("semimonthly"),
        4 => Some("monthly"),
        5 => Some("quarterly"),
        6 => Some("semiannually"),
        7 => Some("annually"),
        8 => Some("bysecond"),
        9 => Some("byminute"),
        10 => Some("byhour"),
        11 => Some("byampm"),
        _ => None,
    }
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
    // Date-grouping condition. The 6-byte blob that follows the field reference carries a condition
    // code at byte 4 (`used + 4`): `0x02` = grouped "for each day" (daily), `0x00` = "for each
    // value" (discrete). Crystal only renders a condition for date/time/boolean fields — the same
    // byte is non-zero on plain discrete fields too (it doubles as a sort attribute), so the field
    // type gates it. Only `daily` is decoded via this flag; other codes are left undecoded rather
    // than guessed. `condition_field` is `Alias.name`; look the type up case-insensitively.
    use crate::model::FieldValueType::*;
    // The date-grouping period has two encodings in the `0xe5` leaf, consulted in order:
    //  1. The SDK `CrGroupConditionEnum` ordinal at `used + 3` (the byte after the field reference) —
    //     consistently populated (see [`sdk_period_name`]); ordinals >= 1 are the non-daily periods
    //     and never collide with the no-period state.
    //  2. The legacy internal `<code>` after the `@Group #N Order` marker (structure `01 00 <code>
    //     ff ff`, see [`date_period_name`]), plus the older `used + 4 == 0x02` daily flag — used for
    //     daily and for pre-SDK-ordinal reports. Discrete date grouping leaves all of these clear.
    let sdk_ordinal = bytes.get(used + 3).copied();
    let period_code = lp_scan(&bytes, Scan::Consume)
        .find(|(_, s, _)| s.starts_with("@Group #") && s.ends_with(" Order"))
        .and_then(|(i, _, consumed)| bytes.get(i + consumed + 2).copied());
    let date_condition = field_types
        .get(&field.to_lowercase())
        .filter(|t| matches!(t, Date | Time | DateTime | Boolean))
        .and_then(|_| {
            sdk_ordinal
                .and_then(sdk_period_name)
                .or_else(|| period_code.and_then(date_period_name))
                .map(str::to_string)
                .or_else(|| {
                    (bytes.get(used + 4).copied() == Some(0x02)).then(|| "daily".to_string())
                })
        });
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
    use super::date_period_name;

    #[test]
    fn confirmed_date_period_codes() {
        assert_eq!(date_period_name(0x01), Some("daily"));
        assert_eq!(date_period_name(0x03), Some("monthly"));
        assert_eq!(date_period_name(0x06), Some("weekly"));
        assert_eq!(date_period_name(0x08), Some("weekly"));
    }

    #[test]
    fn unknown_date_period_codes_fall_back_to_none() {
        // Codes with no confirmed meaning (incl. the SDK ordinals that would collide with the
        // confirmed codes if naively applied) must NOT be mapped — they fall back to discrete.
        for code in [0x00u8, 0x02, 0x04, 0x05, 0x07, 0xff] {
            assert_eq!(date_period_name(code), None, "code {code:#04x}");
        }
    }

    #[test]
    fn with_ties_defaults_false() {
        // WithTies has no located byte; it must default to false.
        assert!(!crate::model::TopBottomNSort::default().with_ties);
    }
}
