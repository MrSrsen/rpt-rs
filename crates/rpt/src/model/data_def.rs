//! Data definition & fields (SDK: `IDataDefinition`, `IField` + kinds).
//!
//! Modelled from the records: `Contents` stores only the fields a report references, not the full
//! table schema.

use super::enums::{
    EvaluationConditionType, FieldValueType, ParameterType, ParameterValueKind, ResetConditionType,
    SortDirection, SortKind, SummaryOperation,
};
use super::primitives::Formula;

/// SDK: `IDataDefinition` — the data half of the report.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct DataDefinition {
    pub record_selection: Option<Formula>,
    pub group_selection: Option<Formula>,
    pub saved_data_filter: Option<Formula>,
    pub groups: Vec<Group>,
    pub record_sorts: Vec<Sort>,
    /// SDK `FieldDefinitions` — formula/parameter/summary/running-total/database fields unified.
    pub field_definitions: Vec<FieldDef>,
    /// Bodies of conditional/auxiliary formulas (running-total eval/reset conditions, section/object
    /// conditional formulas) that are not field definitions. Retained as decoded formula text so the
    /// export layer can account for the field/parameter references they contain.
    pub condition_formula_bodies: Vec<String>,
    /// Bodies of running-total **condition** formulas only (the `0x77` records named
    /// `"… Condition Formula"`: a running total's evaluate/reset condition). Each is a distinct
    /// persistent formula, so every database field it names contributes one to that field's
    /// `IField.UseCount`. These are never attached to a section/object (unlike the entries in
    /// `condition_formula_bodies`), so the UseCount counter scans this list separately to avoid
    /// double-counting.
    pub running_total_condition_formulas: Vec<String>,
    /// The summarized database/formula field of every **summary definition** in the data-definition
    /// region — the `0x7e` summary records (each wrapped in a `0x7f`) that precede the report layout,
    /// one per `ISummaryField`, excluding running totals (a `0x7e` preceded by a `0x80`) and the
    /// chart/cross-tab data bindings that live inside the layout. Most map 1:1 to a *placed* summary
    /// (counted from its field object); the surplus are **orphan** summary definitions with no placed
    /// object that the engine still refcounts. The UseCount counter adds the orphans (per-field
    /// surplus over the placed summaries).
    pub summary_binding_fields: Vec<String>,
}

/// SDK: `IField` base + subtype data (interface inheritance → base struct + `kind` enum).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct FieldDef {
    pub name: String,
    pub value_type: FieldValueType,
    /// SDK `Length` (XML `@NumberOfBytes`).
    pub length: i32,
    /// SDK `FormulaForm` — the `{table.field}` reference.
    pub formula_form: Option<String>,
    pub heading_text: Option<String>,
    pub description: Option<String>,
    pub long_name: Option<String>,
    pub short_name: Option<String>,
    pub kind: FieldKindData,
}

/// SDK: `FieldKind` + the per-kind extra members.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FieldKindData {
    Database(DbField),
    Formula(FormulaField),
    Parameter(Box<ParameterField>),
    Summary(SummaryField),
    RunningTotal(RunningTotalField),
    GroupName(GroupNameField),
    SqlExpression(SqlExpressionField),
    Special(SpecialField),
    #[default]
    Unknown,
}

/// SDK: `FieldKind` (`CrystalDecisions.CrystalReports.Engine.FieldKind`) — the discriminant tag a
/// field definition carries in the `Kind=` XML attribute. This is the value-only form of
/// [`FieldKindData`] (which also holds the per-kind payload); [`FieldKindData::field_kind`] projects
/// one to the other. Values match the SDK enum (1-based); [`FieldKind::name`] returns the SDK
/// `ToString()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FieldKind {
    DatabaseField = 1,
    FormulaField = 2,
    SummaryField = 3,
    SpecialVarField = 4,
    GroupNameField = 5,
    ParameterField = 6,
    RunningTotalField = 7,
    SqlExpressionField = 8,
}

impl FieldKind {
    /// The SDK `FieldKind.ToString()` string written in `Kind=`. (Note the SQL acronym is
    /// upper-cased in the SDK name, unlike the Rust variant.)
    pub fn name(self) -> &'static str {
        match self {
            FieldKind::DatabaseField => "DatabaseField",
            FieldKind::FormulaField => "FormulaField",
            FieldKind::SummaryField => "SummaryField",
            FieldKind::SpecialVarField => "SpecialVarField",
            FieldKind::GroupNameField => "GroupNameField",
            FieldKind::ParameterField => "ParameterField",
            FieldKind::RunningTotalField => "RunningTotalField",
            FieldKind::SqlExpressionField => "SQLExpressionField",
        }
    }
}

impl FieldKindData {
    /// Project this field definition's payload to its [`FieldKind`] discriminant — the typed driver
    /// for the `Kind=` XML attribute. (`Unknown` has no SDK kind; it reports `DatabaseField`, the
    /// default, but unknown defs are never emitted through this path.)
    pub fn field_kind(&self) -> FieldKind {
        match self {
            FieldKindData::Database(_) => FieldKind::DatabaseField,
            FieldKindData::Formula(_) => FieldKind::FormulaField,
            FieldKindData::Parameter(_) => FieldKind::ParameterField,
            FieldKindData::Summary(_) => FieldKind::SummaryField,
            FieldKindData::RunningTotal(_) => FieldKind::RunningTotalField,
            FieldKindData::GroupName(_) => FieldKind::GroupNameField,
            FieldKindData::SqlExpression(_) => FieldKind::SqlExpressionField,
            FieldKindData::Special(_) => FieldKind::SpecialVarField,
            FieldKindData::Unknown => FieldKind::DatabaseField,
        }
    }
}

/// SDK: `IDBField`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DbField {
    pub table_alias: String,
    pub unique_id: String,
}

/// SDK: `IFormulaField`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FormulaField {
    pub text: Formula,
    pub options: i32,
    /// The byte size of the formula's result (`NumberOfBytes`): the value type's intrinsic length
    /// for fixed types, or twice the maximum character count for a string result.
    pub number_of_bytes: i32,
    /// SDK `IFormulaField.Syntax` — the formula's authoring dialect.
    pub syntax: FormulaSyntax,
}

/// SDK: `CrFormulaSyntaxEnum` — a formula's authoring dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FormulaSyntax {
    /// `crFormulaSyntaxCrystal`.
    #[default]
    Crystal,
    /// `crFormulaSyntaxBasic`.
    Basic,
}

impl FormulaSyntax {
    /// The SDK `Syntax=` attribute string.
    pub fn name(self) -> &'static str {
        match self {
            FormulaSyntax::Crystal => "crFormulaSyntaxCrystal",
            FormulaSyntax::Basic => "crFormulaSyntaxBasic",
        }
    }
}

/// SDK: `IParameterField` (XML `<ParameterFieldDefinition>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ParameterField {
    pub parameter_type: ParameterType,
    pub value_kind: ParameterValueKind,
    pub prompt_text: Option<String>,
    pub report_name: Option<String>,
    pub edit_mask: Option<String>,
    pub allow_custom_values: bool,
    pub allow_editing_default_value: bool,
    pub allow_multiple_values: bool,
    pub allow_null_value: bool,
    /// A saved last-used ("current") value is present for this parameter (SDK
    /// `HasCurrentValue`). True for every parameter of a *main* report that carries saved data;
    /// always False for sub-report parameters (the engine only records current values per saved
    /// sub-result, which is not recoverable from the definition).
    pub has_current_value: bool,
    pub optional_prompt: bool,
    pub show_on_panel: bool,
    pub editable_on_panel: bool,
    pub default_values: Vec<ParameterValue>,
    pub current_values: Vec<ParameterValue>,
    pub initial_values: Vec<ParameterValue>,
}

/// SDK: `IParameterFieldValue` (discrete value; range values deferred).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ParameterValue {
    pub value: String,
    pub description: Option<String>,
}

/// SDK: `ISummaryField`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SummaryField {
    pub operation: SummaryOperation,
    pub summarized_field: String,
    pub operation_parameter: i32,
    pub group_index: Option<i32>,
}

/// SDK: `IRunningTotalField`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RunningTotalField {
    pub operation: SummaryOperation,
    pub summarized_field: String,
    pub operation_parameter: i32,
    pub evaluation: EvaluationConditionType,
    pub reset: ResetConditionType,
    /// The database/formula field whose change drives an `OnChangeOfField` evaluate/reset condition
    /// (the field named in the `0x80` reset record's own leaf). Empty unless `evaluation` or `reset`
    /// is `OnChangeOfField`. Not emitted to XML; it is a persistent field reference, so it
    /// contributes to that field's `UseCount`.
    pub on_change_field: String,
}

/// SDK: `IGroupNameField`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GroupNameField {
    pub group_index: i32,
    pub group_name_field_name: String,
}

/// SDK: `ISQLExpressionField`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SqlExpressionField {
    pub text: String,
}

/// SDK: `ISpecialField` (page number, print date, …).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SpecialField {
    pub special_type: super::enums::SpecialFieldType,
}

/// SDK: `IGroup` (XML `<Group>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Group {
    pub condition_field: String,
    pub sort: Sort,
    pub options: GroupOptions,
    /// For a date/time/boolean group condition, the engine's date-grouping token rendered in
    /// `GroupName`/summary data sources (e.g. `daily` -> `GroupName ({fld}, "Daily")`). `None` for a
    /// discrete ("for each value") grouping or a non-date field. Stored lowercase; `GroupName`
    /// title-cases it, summaries keep it lowercase.
    pub date_condition: Option<String>,
    /// The group's `<GroupAreaFormat>` flags (KeepTogether / RepeatHeader / VisibleGroupsPerPage),
    /// decoded from the `0x0088` record. The outermost group is not described by one, so it keeps
    /// the defaults.
    pub area_format: super::report_def::GroupAreaFormat,
}

/// SDK: `IGroupOptions` (date condition, keep-together, … — deferred detail).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct GroupOptions {}

/// SDK: `ISort` (XML `<SortField>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Sort {
    pub field: String,
    pub direction: SortDirection,
    pub kind: SortKind,
}
