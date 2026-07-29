//! Field definitions — every kind of field the data definition can reference: database fields
//! (`0x73`), formulas and custom functions (`0x76`), running totals (`0x80`), summaries (`0x7e`)
//! and SQL expressions (`0x81`). All of them are built on the same `0x0071 NamedValue` base, which
//! names the definition and fixes the type and width of the value it produces, so that base is
//! decoded here too.

use crate::build_model::record_values::is_field_ref;
use crate::build_model::tree_search::{nodes_where, summary_def_nodes};
use crate::build_model::{row_of, MAX_STRING_BYTES};
use crate::codec::RecordNode;
use crate::field_table::table::{Cell, Row};
use crate::field_table::tables as ft;
use crate::model::{
    DbField, FieldDef, FieldKindData, FieldValueType, Formula, FormulaField, FormulaVariable,
    FormulaVariableScope, ResetConditionType, RunningTotalField, SummaryField, SummaryOperation,
};
use crate::records::rtype::*;

/// The `0x0071 NamedValue` a field definition carries: the definition's name, the type of the value
/// it produces, and that value's length in bytes.
pub(in crate::build_model) struct NamedValue {
    /// The definition's own name. Empty where the definition has none — a summary stores an empty
    /// string here, not an absent field.
    pub name: String,
    pub value_type: FieldValueType,
    /// The value's length **in bytes**.
    pub length: i32,
}

/// Read a `0x0071 NamedValue` record.
///
/// The record stores its length twice: a narrow form that counts characters for a string-typed
/// value and saturates at 255, and a wide form that is the byte count for every type. The wide form
/// supersedes the narrow one wherever the record carries it. The wide form is signed — a value of no
/// fixed width, a blob, stores `-1` — and a string field cannot exceed 32767 characters, so a wider
/// stored width means "unbounded" and is reported at that ceiling.
pub(in crate::build_model) fn named_value(node: &RecordNode, logical: &[u8]) -> NamedValue {
    named_value_of(&row_of(node, logical, &ft::NAMED_VALUE))
}

/// [`named_value`] over an already-read row.
fn named_value_of(row: &Row) -> NamedValue {
    let code = row.u("value_type") as i32;
    let narrow = row.u("narrow_length") as i32;
    // A string-typed value's narrow length is a character count; every other type's is already
    // bytes.
    let narrow_bytes = if code == STRING_VALUE_TYPE_CODE {
        narrow * 2
    } else {
        narrow
    };
    let length = row.get("length").and_then(Cell::i).unwrap_or(narrow_bytes);
    NamedValue {
        name: row.text("name").to_owned(),
        value_type: FieldValueType::from_code(code),
        length: length.min(MAX_STRING_BYTES),
    }
}

/// The `CrFieldValueTypeEnum` code for a string value, whose stored narrow length counts characters
/// rather than bytes.
const STRING_VALUE_TYPE_CODE: i32 = 11;

/// Decode a database field definition (`0x0073`): a `0x0072` wrapping the `0x0071` base, which is
/// where the field's name, value type and length live.
pub(in crate::build_model) fn build_field(node: &RecordNode, logical: &[u8]) -> Option<FieldDef> {
    let mut base = None;
    node.walk(&mut |n| {
        if base.is_none() && n.rtype == NAMED_VALUE {
            base = Some(named_value(n, logical));
        }
    });
    let nv = base?;
    if nv.name.is_empty() {
        return None;
    }
    Some(FieldDef {
        name: nv.name.clone(),
        value_type: nv.value_type,
        length: nv.length,
        short_name: Some(nv.name),
        kind: FieldKindData::Database(DbField::default()),
        ..Default::default()
    })
}

/// Decode the field-pool census from the report's one `0x006e` `FieldManagerEntry` record.
/// Returns `None` when the record is absent. The stored formula-body count omits the three built-in
/// formulas, so it is reconstructed here (`+ 3`), matching the `0x0076` record count exactly.
pub(super) fn build_field_manager_census(
    tree: &[RecordNode],
    logical: &[u8],
) -> Option<crate::model::FieldManagerCensus> {
    const BUILTIN_FORMULAS: u16 = 3;
    let node = nodes_where(tree, |n| n.rtype == FIELD_MANAGER_ENTRY)
        .into_iter()
        .next()?;
    let census = row_of(node, logical, &ft::FIELD_MANAGER_ENTRY);
    Some(crate::model::FieldManagerCensus {
        database_fields: census.u("database_fields"),
        formula_bodies: (census.u("formula_bodies") as u16).saturating_add(BUILTIN_FORMULAS),
    })
}

/// The report's persisted formula-language variables (`Global`/`Shared`), one per `0x0118` record.
/// Each carries the variable's declared result kind and scope; `Local` variables are not
/// persisted, so only the two outer scopes appear. The `0x0116` header that precedes the run counts
/// them, which the records themselves already say.
pub(super) fn build_formula_variables(tree: &[RecordNode], logical: &[u8]) -> Vec<FormulaVariable> {
    let mut out = Vec::new();
    for root in tree {
        root.walk(&mut |node| {
            if node.rtype != FORMULA_VARIABLE {
                return;
            }
            let var = row_of(node, logical, &ft::FORMULA_VARIABLE);
            let Some(name) = var.get("name").and_then(|v| v.text()) else {
                return;
            };
            out.push(FormulaVariable {
                name: name.to_owned(),
                value_type: var
                    .get("value_type")
                    .and_then(|v| v.u())
                    .map(|v| FieldValueType::from_result_kind(v as i32))
                    .unwrap_or_default(),
                scope: var
                    .get("scope")
                    .and_then(|v| v.u())
                    .map(|v| FormulaVariableScope::from_code(v as i32))
                    .unwrap_or_default(),
            });
        });
    }
    out
}

/// The summarized field of every **summary definition** (`ISummaryField`) in the data-definition
/// region. These are the `0x7e` summary records (each wrapped in a `0x7f`) that appear *before* the
/// report layout (the first `0x8a` area marker). Running totals (a `0x7e` its `0x80` record
/// contains) are excluded — they are decoded separately — and so are the chart/cross-tab data bindings,
/// which live inside the layout (after the first area marker). Only the field-shaped summarized field
/// (`table.field` or `@formula`) of each is returned, in document order. Reconciling these against
/// the placed summaries is what recovers the orphan summary definitions (see
/// `DataDefinition.summary_binding_fields`).
pub(super) fn build_summary_bindings(tree: &[RecordNode], logical: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for node in summary_def_nodes(tree, true) {
        let row = row_of(node, logical, &ft::SUMMARY_FIELD_DEFINITION);
        let field = row.text("operand");
        if is_field_ref(field) {
            out.push(field.to_owned());
        }
    }
    out
}

/// The classified outcome of pairing formula bodies (`0x76`) with their following name (`0x71`).
#[derive(Default)]
pub(super) struct Formulas {
    pub user_formulas: Vec<FieldDef>,
    pub record_selection: Option<String>,
    pub group_selection: Option<String>,
    pub saved_data_filter: Option<String>,
    /// Bodies of conditional/auxiliary formulas (running-total eval/reset conditions, section/object
    /// conditional formulas) that are not user field definitions.
    pub condition_formula_bodies: Vec<String>,
    /// Subset of `condition_formula_bodies`: only the running-total **condition** formulas (names
    /// ending `" Condition Formula"`). Kept separately because, unlike the section/object conditional
    /// formulas, these are *not* attached to any section/object, so a consumer walking the report's
    /// objects reaches them only through this list.
    pub running_total_condition_formulas: Vec<String>,
    /// Custom functions: formula records whose body opens with the reserved `Function (args) …`
    /// header. Not formula fields — the engine lists them under `CustomFunctions` — so they are
    /// collected here rather than in `user_formulas`.
    pub custom_functions: Vec<crate::model::CustomFunction>,
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
    // only in a field-format formula) and Fore/font-Color formula. These are reserved engine names
    // a user formula field cannot take; they attach to the object, not the formula list.
    "Display_String",
    "Fore_Color",
    // A PictureObject's dynamic graphic-location formula; reserved, attaches to the object, not the
    // formula list.
    "Graphic_Location",
    "Record_Selection",
    "Group_Selection",
];

/// Pair each formula body (`0x76`) with the **named value** (`0x71`) it carries — the definition
/// nested at the head of its content, which the pre-order walk reaches immediately after it.
/// Classify by the name: the report's selection formulas, the per-group display formulas (skipped —
/// synthesised as `GroupNameFieldDefinition`s), section conditional formulas (skipped), and the
/// user formula fields (`{@name}`).
pub(super) fn build_formulas(tree: &[RecordNode], logical: &[u8]) -> Formulas {
    let nodes = nodes_where(tree, |n| n.rtype == FORMULA || n.rtype == NAMED_VALUE);
    let mut out = Formulas::default();
    let mut pending: Option<(
        String,
        crate::model::FormulaSyntax,
        crate::model::FormulaNullTreatment,
    )> = None;
    for n in nodes {
        if n.rtype == FORMULA {
            let formula = row_of(n, logical, &ft::FORMULA);
            pending = Some((
                formula.text("text").to_owned(),
                formula_syntax(&formula),
                formula_null_treatment(&formula),
            ));
            continue;
        }
        // NAMED_VALUE: names the pending body, if any (db-field/parameter names have none).
        let Some((body, syntax, null_treatment)) = pending.take() else {
            continue;
        };
        let nv = named_value(n, logical);
        // A definition with no name of its own — a summary's — names no formula.
        if nv.name.is_empty() {
            continue;
        }
        let name = nv.name;
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
                // A user formula field. Its result type and length are the values the `0x71` record
                // carries: the engine re-compiles every formula at load time and may report a
                // different type/length, but that recompute is runtime-gated (it depends on the live
                // datasource binding) and not reproducible from the file alone, so `rpt-reader` emits
                // the stored fact. The recompute model lives in `rpt_formula::string_max_bytes` for the
                // eval/LSP paths that have runtime context.
                let value_type = nv.value_type;
                // NumberOfBytes is the engine-persisted `IField.Length` (RAS DispId 7): a fixed type
                // uses its intrinsic size; a `String` result uses the record's stored byte width —
                // the last-saved length as it sits in the file.
                let number_of_bytes = value_type.byte_length().unwrap_or(nv.length);
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

/// The formula's authoring dialect: `1` is the Basic-syntax editor, anything else Crystal's own.
/// A record that ended before the field carries the default, which is Crystal.
fn formula_syntax(formula: &Row) -> crate::model::FormulaSyntax {
    use crate::model::FormulaSyntax;
    match formula.u("syntax") {
        1 => FormulaSyntax::Basic,
        _ => FormulaSyntax::Crystal,
    }
}

/// How the formula treats a null field value: `1` substitutes the value type's default, anything
/// else raises. A record that ended before the field carries the default, which is to raise.
fn formula_null_treatment(formula: &Row) -> crate::model::FormulaNullTreatment {
    use crate::model::FormulaNullTreatment;
    match formula.u("null_treatment") {
        1 => FormulaNullTreatment::DefaultValue,
        _ => FormulaNullTreatment::Exception,
    }
}

/// The `SecondarySummarizedField` of a `0x7e` summary / running-total record, or an empty string
/// when there is none.
///
/// Each summarized field is serialized as a length-prefixed reference followed by a fixed 3-byte
/// value descriptor (`[type][00][bytes]`, e.g. `01 00 04` for a Number formula, `00 00 03` for a
/// database field). The record always stores a second such reference; a definition with no second
/// field stores it empty, which `is_field_ref` rejects.
///
/// This is a stored fact, not gated on the operation: a running total added through
/// `RunningTotalFieldController.Add` self-mirrors its secondary field to equal the primary, so the
/// second reference is present and equals the first; one added in the designer, and a plain
/// summary, leave it empty.
fn secondary_field(row: &Row) -> String {
    let s = row.text("secondary_operand");
    if is_field_ref(s) {
        s.to_owned()
    } else {
        String::new()
    }
}

/// Raise running-total field definitions. Each is a `0x80` record: the two conditions that drive
/// the accumulator, around the `0x7e` summary definition it contains. That `0x7e` carries the
/// operation and the summarized field, and its own `0x71` child names the running total and gives
/// its value type and byte length. A `0x7e` no `0x80` contains is a summary, handled elsewhere.
pub(super) fn build_running_totals(tree: &[RecordNode], logical: &[u8]) -> Vec<FieldDef> {
    let mut out = Vec::new();
    for node in nodes_where(tree, |n| n.rtype == RUNNING_TOTAL_FIELD) {
        let record = row_of(node, logical, &ft::RUNNING_TOTAL_FIELD);
        let reset = ResetConditionType::from_code(record.u("reset_kind") as i32);
        let evaluation =
            crate::model::EvaluationConditionType::from_code(record.u("evaluate_kind") as i32);
        // Each condition names its own driver, so the field whose change drives the running total
        // is the one named by whichever condition is on change of a field. The model carries a
        // single name for both, and the reset condition is stored first, so it wins.
        let on_change_field = if reset == ResetConditionType::OnChangeOfField {
            record.text("reset_field").to_owned()
        } else if evaluation == crate::model::EvaluationConditionType::OnChangeOfField {
            record.text("evaluate_field").to_owned()
        } else {
            String::new()
        };
        // The operation the running total accumulates is its nested summary definition.
        let Some(summary_node) = node
            .children
            .iter()
            .find(|c| c.rtype == SUMMARY_FIELD_DEFINITION)
        else {
            continue;
        };
        let row = row_of(summary_node, logical, &ft::SUMMARY_FIELD_DEFINITION);
        let operation = SummaryOperation::from_code(row.u("operation") as i32);
        let operation_parameter = row.u("operation_parameter") as i32;
        let summarized_field = row.text("operand").to_owned();
        // A running total added through the controller API self-mirrors its secondary summarized
        // field to the primary; one added in the designer, and a plain summary, store it empty.
        let secondary_summarized_field = secondary_field(&row);
        // The `0x71` child: the running total's name, its result type and its byte length.
        let Some(child) = summary_node
            .children
            .iter()
            .find(|c| c.rtype == NAMED_VALUE)
        else {
            continue;
        };
        let nv = named_value(child, logical);
        if nv.name.is_empty() {
            continue;
        }
        let name = nv.name;
        // A running total always reports its result as a plain number; the engine widens a Currency
        // summarized field (the stored type byte is Currency) to NumberField.
        let value_type = match nv.value_type {
            FieldValueType::Currency => FieldValueType::Number,
            other => other,
        };
        let length = nv.length;
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

/// Raise **summary** field definitions (`ISummaryField`). Each is a standalone `0x7e` record (one no
/// `0x80` running total contains) appearing before the report layout: the aggregate operation, then
/// the summarized field as a reference, and a `0x71` child giving the result's value type and byte
/// length. Unlike a running total, a summary carries no stored name, and its own
/// group scope is not in the record — two definitions differing only in scope are byte-identical —
/// so `group_index` is left `None` and the placement recovers the scope. The *denominator* scope of a
/// percentage summary is stored, in the record's tail.
pub(super) fn build_summaries(tree: &[RecordNode], logical: &[u8]) -> Vec<FieldDef> {
    let mut out = Vec::new();
    for node in summary_def_nodes(tree, true) {
        let row = row_of(node, logical, &ft::SUMMARY_FIELD_DEFINITION);
        let operation = SummaryOperation::from_code(row.u("operation") as i32);
        let operation_parameter = row.u("operation_parameter") as i32;
        let Some(summarized_field) = row.get("operand").and_then(|v| v.text()).map(str::to_owned)
        else {
            continue;
        };
        // A two-field summary stores its second field reference where a single-field one stores an
        // empty string. See [`secondary_field`].
        let secondary_summarized_field = secondary_field(&row);
        // The tail past the field refs carries the IsPercentageSummary flag and, in the percentage
        // form only, the group whose total the percentage is taken of (`0` = the report grand
        // total, which is no group).
        let is_percentage_summary = row.i("is_percentage") != 0;
        let second_group_for_percentage = is_percentage_summary
            .then(|| row.i("percentage_base_group"))
            .filter(|&g| g != 0);
        // The `0x71` child fixes the summary's result type and byte length. A summary has no name of
        // its own, so that child stores an empty one.
        let (value_type, length) = node
            .children
            .iter()
            .find(|c| c.rtype == NAMED_VALUE)
            .map(|c| named_value(c, logical))
            .map(|nv| (nv.value_type, nv.length))
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
                second_group_for_percentage,
            }),
            ..Default::default()
        });
    }
    out
}

/// SQL Expression field definitions (`0x81` records). A SQL Expression is a snippet of raw SQL
/// evaluated by the database and referenced from the report as `{%Name}`.
///
/// The `0x71` `NamedValue` child comes first and carries the field name, value type and byte
/// length; the expression **text** follows it, empty for an unbound or blank expression.
pub(super) fn build_sql_expressions(tree: &[RecordNode], logical: &[u8]) -> Vec<FieldDef> {
    let mut out = Vec::new();
    for node in nodes_where(tree, |n| n.rtype == SQL_EXPRESSION_FIELD) {
        let text = row_of(node, logical, &ft::SQL_EXPRESSION_FIELD)
            .text("text")
            .to_owned();
        let Some(child) = node.children.iter().find(|c| c.rtype == NAMED_VALUE) else {
            continue;
        };
        let nv = named_value(child, logical);
        if nv.name.is_empty() {
            continue;
        }
        out.push(FieldDef {
            name: nv.name.clone(),
            value_type: nv.value_type,
            length: nv.length,
            short_name: Some(nv.name),
            kind: FieldKindData::SqlExpression(crate::model::SqlExpressionField { text }),
            ..Default::default()
        });
    }
    out
}

#[cfg(test)]
mod named_value_tests {
    use super::{ft, named_value_of, FieldValueType};
    use crate::field_table::cursor::{Piece, RecordContent, StringFormat};
    use crate::field_table::table::read_strings;

    /// A `u32`-BE NUL-terminated length-prefixed string.
    fn lp(s: &str) -> Vec<u8> {
        let mut v = ((s.len() + 1) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v.push(0);
        v
    }

    /// Assemble a `0x0071` run: name, a value-type enum already encoded, the narrow length, the
    /// second name, and the wide length.
    fn field_run(name: &str, value_type: &[u8], narrow: u16, wide: Option<u32>) -> Vec<u8> {
        let mut v = lp(name);
        v.extend_from_slice(value_type);
        v.extend_from_slice(&narrow.to_be_bytes());
        v.extend(lp(""));
        if let Some(w) = wide {
            v.extend_from_slice(&w.to_be_bytes());
        }
        v
    }

    /// A record built here carries no header to declare a string form, so the reading names the
    /// enhanced form — the one the record-tree reader admits — rather than leaving it assumed.
    fn decode(run: Vec<u8>) -> super::NamedValue {
        let content = RecordContent {
            rtype: 0x0071,
            schema: 0x0700,
            pieces: vec![Piece::Run(run)],
        };
        let reading = read_strings(&ft::NAMED_VALUE, &content, StringFormat::Enhanced);
        assert!(reading.exact(), "the table accounts for the whole record");
        named_value_of(&reading.row)
    }

    /// A definition with no name of its own stores an **empty** name, not an absent one — so the
    /// value type sits after a five-byte empty string block, exactly where it sits after a real
    /// name. One shape, read one way.
    #[test]
    fn an_unnamed_definition_stores_an_empty_name() {
        let nv = decode(field_run("", &[0x07], 8, Some(8)));
        assert_eq!(nv.name, "");
        assert_eq!(nv.value_type, FieldValueType::Currency);
        assert_eq!(nv.length, 8);
    }

    /// The narrow length counts **characters** for a string value; the wide length that follows the
    /// second name is the byte count, and it is the one to report.
    #[test]
    fn a_string_length_is_the_byte_count() {
        let nv = decode(field_run("", &[0x0b], 41, Some(82)));
        assert_eq!(nv.value_type, FieldValueType::String);
        assert_eq!(nv.length, 82);
    }

    /// The narrow length saturates at 255, so it stops being a length at all past that; the wide
    /// one still carries the real width.
    #[test]
    fn a_saturated_narrow_length_does_not_bound_the_byte_count() {
        let nv = decode(field_run("Wide", &[0x0b], 255, Some(1024)));
        assert_eq!(nv.length, 1024);
    }

    /// A value type of `0x80` or more is two bytes, not one — read as one, every field after it
    /// lands a byte early.
    #[test]
    fn a_value_type_past_the_narrowing_threshold_is_two_bytes() {
        let nv = decode(field_run("Unknown", &[0x80, 0xff], 4, Some(4)));
        assert_eq!(nv.value_type, FieldValueType::from_code(0xff));
        assert_eq!(nv.length, 4);
    }

    /// Without the wide form the narrow one stands, doubled for a string value.
    #[test]
    fn the_narrow_length_stands_when_the_record_stops() {
        let nv = decode(field_run("Short", &[0x0b], 10, None));
        assert_eq!(nv.length, 20);
    }

    /// The wide length is signed: a value of no fixed width stores `-1`, not a number just under
    /// 2^32 — and certainly not the `65535` its low half looks like.
    #[test]
    fn a_value_of_no_fixed_width_stores_minus_one() {
        let nv = decode(field_run("photo", &[0x0e], 255, Some(0xffff_ffff)));
        assert_eq!(nv.value_type, FieldValueType::Blob);
        assert_eq!(nv.length, -1);
    }
}

#[cfg(test)]
mod field_definition_tests {
    use super::{build_field, FieldValueType, NAMED_VALUE};
    use crate::codec::RecordNode;

    /// A `0x0073` database field: a `0x0072` wrapping the `0x0071` base, whose field bytes are the one
    /// given. Only the nesting and the base record's bytes matter to `build_field`. Each record is
    /// headed by its own declaration of the enhanced string form, as a stored record is.
    fn db_field(base: &[u8]) -> (Vec<u8>, RecordNode) {
        let mut logical = vec![0u8; 16];
        for (rtype, offset) in [(0x0073u16, 0usize), (0x0072, 4), (NAMED_VALUE, 10)] {
            logical[offset] = crate::build_model::enhanced_header_byte(rtype, 0);
        }
        let start = logical.len();
        logical.extend_from_slice(base);
        let end = logical.len();
        let named_value = RecordNode {
            rtype: NAMED_VALUE,
            schema: 0x0700,
            offset: start.saturating_sub(6),
            content_start: start,
            content_end: end,
            mask: 0,
            children: Vec::new(),
        };
        let wrapper = RecordNode {
            rtype: 0x0072,
            schema: 0x0700,
            offset: start.saturating_sub(12),
            content_start: start,
            content_end: end,
            mask: 0,
            children: vec![named_value],
        };
        let node = RecordNode {
            rtype: 0x0073,
            schema: 0x0700,
            offset: start.saturating_sub(18),
            content_start: start,
            content_end: end,
            mask: 0,
            children: vec![wrapper],
        };
        (logical, node)
    }

    /// A blob column has no fixed width, and the base record says so with `-1`. Read from the low
    /// half of that word it becomes `65535`, a width no field has.
    #[test]
    fn a_blob_column_has_no_fixed_length() {
        let base = [
            0, 0, 0, 6, b'p', b'h', b'o', b't', b'o', 0,    // name
            0x0e, // value type: blob
            0x00, 0xff, // narrow length
            0, 0, 0, 1, 0, // second name, empty
            0xff, 0xff, 0xff, 0xff, // wide length
        ];
        let (logical, node) = db_field(&base);
        let f = build_field(&node, &logical).expect("a named base record is a field");
        assert_eq!(f.name, "photo");
        assert_eq!(f.value_type, FieldValueType::Blob);
        assert_eq!(f.length, -1);
    }
}
