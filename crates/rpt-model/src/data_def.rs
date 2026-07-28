//! Data definition & fields (SDK: `IDataDefinition`, `IField` + kinds).
//!
//! Modelled from the records: `Contents` stores only the fields a report references, not the full
//! table schema.

use super::enums::{
    DiscreteOrRangeKind, EvaluationConditionType, FieldValueType, FormulaVariableScope,
    LovSourceKind, ParameterType, ParameterValueKind, RangeBoundType, ResetConditionType,
    SortDirection, SortKind, SummaryOperation,
};
use super::primitives::{Formula, Twips};

/// SDK: `IDataDefinition` — the data half of the report.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataDefinition {
    /// SDK `RecordSelectionFormula` — the boolean condition that filters individual detail records.
    pub record_selection: Option<Formula>,
    /// SDK `GroupSelectionFormula` — the boolean condition that filters whole groups.
    pub group_selection: Option<Formula>,
    /// SDK saved-data selection formula — re-filters an already-fetched saved-data rowset without a
    /// re-query of the live database.
    pub saved_data_filter: Option<Formula>,
    /// SDK `Groups` — the report's group levels, outermost first.
    pub groups: Vec<Group>,
    /// SDK `SortFields` (record-level entries) — the detail-record sort order.
    pub record_sorts: Vec<Sort>,
    /// SDK `FieldDefinitions` — formula/parameter/summary/running-total/database fields unified.
    pub field_definitions: Vec<FieldDef>,
    /// Bodies of conditional/auxiliary formulas (running-total eval/reset conditions, section/object
    /// conditional formulas) that are not field definitions. Retained as decoded formula text: they
    /// are real stored formula bodies, and they carry field/parameter references nothing else records.
    pub condition_formula_bodies: Vec<String>,
    /// Bodies of running-total **condition** formulas only (named `"… Condition Formula"`: a running
    /// total's evaluate/reset condition). Held separately from `condition_formula_bodies` because
    /// these are never attached to a section/object, so a consumer walking the report's objects
    /// would otherwise miss the fields they name (see `rpt_query::used_database_fields`).
    pub running_total_condition_formulas: Vec<String>,
    /// The summarized database/formula field of every **summary definition** in the data-definition
    /// region — the summary definitions that precede the report layout, one per `ISummaryField`,
    /// excluding running totals and the chart/cross-tab data bindings that live inside the layout.
    /// Most map 1:1 to a *placed* summary; the surplus are **orphan** summary definitions with no
    /// placed object, whose summarized fields are reachable nowhere else in the model.
    pub summary_binding_fields: Vec<String>,
    /// The report's persisted formula-language variables — the `Global`/`Shared` variables declared in
    /// its formulas. STRUCTURAL: the Crystal
    /// SDK exposes no typed accessor for these (only each formula's raw `Text`/`Syntax`, already
    /// emitted), so they are not exported. Decoded as a stored fact for completeness and
    /// for the `crystal-formula` VM, which can pre-register a formula's shared/global variables.
    pub formula_variables: Vec<FormulaVariable>,
    /// The field-pool census — the engine's own tally of the report's field manager. Redundant with
    /// the decoded [`field_definitions`](Self::field_definitions) (it is a cross-check), and
    /// STRUCTURAL: no SDK accessor exposes it. `None` if it was not recorded (older/edge formats).
    pub field_manager_census: Option<FieldManagerCensus>,
    /// SDK `CustomFunctionController.GetCustomFunctions()` — the report's reusable custom functions
    /// (named `Function (args) …` declarations callable from any formula). Stored as formula records
    /// (`0x76`/`0x71`) whose body opens with the reserved `Function` header; decoded here as distinct
    /// definitions rather than [`field_definitions`](Self::field_definitions), since the engine lists
    /// them under `CustomFunctions`, not the formula-field collection.
    pub custom_functions: Vec<CustomFunction>,
}

/// SDK: `CrystalReports.CustomFunction` (RAS `ISCRCustomFunction`) — a named, reusable formula
/// function stored in the report. Its argument list and return type are declared inside the `Text`
/// body (`Function (StringVar x, …) …`); RAS exposes only name, syntax, and body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CustomFunction {
    /// SDK `CustomFunction.Name` — the identifier a formula calls it by (e.g. `Concatenate3Strings`).
    pub name: String,
    /// SDK `CustomFunction.Syntax` — the authoring dialect of the function body.
    pub syntax: FormulaSyntax,
    /// SDK `CustomFunction.Text` — the full `Function (args) …` declaration, body included.
    pub text: String,
}

/// The field-pool census: a compact tally of the report's field manager that mirrors the decoded
/// [`field_definitions`](DataDefinition::field_definitions); modeled as a cross-check. STRUCTURAL —
/// internal (not exported).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FieldManagerCensus {
    /// Number of database-field definitions — matches the decoded db-field count exactly.
    pub database_fields: u32,
    /// Total number of compiled **formula bodies** — every formula in the report (user formula fields
    /// *plus* the internal condition / selection / group-order / running-total formulas), not just the
    /// user `FormulaFieldDefinition`s.
    pub formula_bodies: u16,
}

impl DataDefinition {
    /// SDK-shaped, per-kind field views over the unified [`field_definitions`](Self::field_definitions)
    /// vector — the reading experience of the SDK's separate typed collections
    /// (`DataDefinition.FormulaFields`, `.ParameterFields`, …) without a second source of truth.
    /// Each yields `(&FieldDef, &<payload>)` in the file's original field order (an ordering the
    /// SDK's split collections don't obviously preserve across kinds).
    ///
    /// SDK: `IDataDefinition.FormulaFields`.
    pub fn formula_fields(&self) -> impl Iterator<Item = (&FieldDef, &FormulaField)> {
        self.field_definitions.iter().filter_map(|f| match &f.kind {
            FieldKindData::Formula(x) => Some((f, x)),
            _ => None,
        })
    }

    /// SDK: `IDataDefinition.ParameterFields`.
    pub fn parameter_fields(&self) -> impl Iterator<Item = (&FieldDef, &ParameterField)> {
        self.field_definitions.iter().filter_map(|f| match &f.kind {
            FieldKindData::Parameter(x) => Some((f, x.as_ref())),
            _ => None,
        })
    }

    /// SDK: `IDataDefinition.DatabaseFields` — the database fields the report *references*
    /// (distinct from the full table schema in [`Database`](super::Database)).
    pub fn database_fields(&self) -> impl Iterator<Item = (&FieldDef, &DbField)> {
        self.field_definitions.iter().filter_map(|f| match &f.kind {
            FieldKindData::Database(x) => Some((f, x)),
            _ => None,
        })
    }

    /// SDK: `IDataDefinition.SummaryFields`.
    pub fn summary_fields(&self) -> impl Iterator<Item = (&FieldDef, &SummaryField)> {
        self.field_definitions.iter().filter_map(|f| match &f.kind {
            FieldKindData::Summary(x) => Some((f, x)),
            _ => None,
        })
    }

    /// SDK: `IDataDefinition.RunningTotalFields`.
    pub fn running_total_fields(&self) -> impl Iterator<Item = (&FieldDef, &RunningTotalField)> {
        self.field_definitions.iter().filter_map(|f| match &f.kind {
            FieldKindData::RunningTotal(x) => Some((f, x)),
            _ => None,
        })
    }

    /// SDK: `IDataDefinition.GroupNameFields`.
    pub fn group_name_fields(&self) -> impl Iterator<Item = (&FieldDef, &GroupNameField)> {
        self.field_definitions.iter().filter_map(|f| match &f.kind {
            FieldKindData::GroupName(x) => Some((f, x)),
            _ => None,
        })
    }

    /// SDK: `IDataDefinition.SQLExpressionFields`.
    pub fn sql_expression_fields(&self) -> impl Iterator<Item = (&FieldDef, &SqlExpressionField)> {
        self.field_definitions.iter().filter_map(|f| match &f.kind {
            FieldKindData::SqlExpression(x) => Some((f, x)),
            _ => None,
        })
    }

    /// Special fields (page number, print date, …). NOTE: this is an rpt-rs convenience, **not** an
    /// SDK `DataDefinition` collection — the SDK reaches special fields only via layout/`FieldObject`
    /// dispatch, not a data-definition-level collection.
    pub fn special_fields(&self) -> impl Iterator<Item = (&FieldDef, &SpecialField)> {
        self.field_definitions.iter().filter_map(|f| match &f.kind {
            FieldKindData::Special(x) => Some((f, x)),
            _ => None,
        })
    }
}

/// SDK: `IField` base + subtype data (interface inheritance → base struct + `kind` enum).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FieldDef {
    /// SDK `IField.Name` — the field's identifier as referenced from formulas.
    pub name: String,
    /// SDK `IField.Type` — the field's value type.
    pub value_type: FieldValueType,
    /// SDK `Length`.
    pub length: i32,
    /// SDK `FormulaForm` — the `{table.field}` reference.
    pub formula_form: Option<String>,
    /// SDK `HeadingText` — the field's default column-heading text.
    pub heading_text: Option<String>,
    /// SDK `Description` — the field's author-supplied description.
    pub description: Option<String>,
    /// SDK `LongName` — the fully qualified name (e.g. `table.field`).
    pub long_name: Option<String>,
    /// SDK `ShortName` — the unqualified field name (e.g. `field`).
    pub short_name: Option<String>,
    /// The field's per-kind payload (dispatched by the SDK `FieldKind`).
    pub kind: FieldKindData,
}

/// SDK: `FieldKind` + the per-kind extra members.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FieldKindData {
    /// A field bound to a source-database table column.
    Database(DbField),
    /// A user-authored formula field.
    Formula(FormulaField),
    /// A report/stored-procedure input parameter.
    Parameter(Box<ParameterField>),
    /// A summary (aggregate) field.
    Summary(SummaryField),
    /// A running-total field.
    RunningTotal(RunningTotalField),
    /// A group-name field (the value of a group's condition field).
    GroupName(GroupNameField),
    /// A SQL Expression field (a snippet of raw SQL evaluated by the database).
    SqlExpression(SqlExpressionField),
    /// A built-in special field (page number, print date, …).
    Special(SpecialField),
    /// No payload could be decoded for this field's kind.
    #[default]
    Unknown,
}

/// SDK: `FieldKind` (`CrystalDecisions.CrystalReports.Engine.FieldKind`) — a field definition's
/// type discriminant. This is the value-only form of [`FieldKindData`] (which also holds the
/// per-kind payload); [`FieldKindData::field_kind`] projects one to the other. Values match the SDK
/// enum (1-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FieldKind {
    /// A field bound to a source-database table column.
    DatabaseField = 1,
    /// A user-authored formula field.
    FormulaField = 2,
    /// A summary (aggregate) field.
    SummaryField = 3,
    /// A built-in special field (page number, print date, …).
    SpecialVarField = 4,
    /// A group-name field.
    GroupNameField = 5,
    /// A report/stored-procedure input parameter.
    ParameterField = 6,
    /// A running-total field.
    RunningTotalField = 7,
    /// A SQL Expression field.
    SqlExpressionField = 8,
}

impl FieldKindData {
    /// Project this field definition's payload to its [`FieldKind`] discriminant. (`Unknown` has no
    /// SDK kind; it reports `DatabaseField`, the default, but unknown defs are never surfaced
    /// through this path.)
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
    /// The alias of the source table this field is read from.
    pub table_alias: String,
    /// The field's stable identifier within the table schema, distinct from its display name.
    pub unique_id: String,
}

/// SDK: `IFormulaField`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FormulaField {
    /// SDK `IFormulaField.Text` — the formula's source text (and parsed body, when decoded).
    pub text: Formula,
    /// Raw formula option bitflags (SDK `FormulaFieldDefinition.Options`). A multi-bit field stored
    /// in the `0x76` record trailer (e.g. `0x10` at trailer + 7, `0x40` at trailer + 11) whose full
    /// bit semantics track runtime formula-dependency (IsRecurring / page-dependence). Not decoded —
    /// the value is engine-recomputed at load and forgiven in the parity gate; left `0`.
    pub options: i32,
    /// The byte size of the formula's result (`NumberOfBytes`): the value type's intrinsic length
    /// for fixed types, or twice the maximum character count for a string result.
    pub number_of_bytes: i32,
    /// SDK `IFormulaField.Syntax` — the formula's authoring dialect.
    pub syntax: FormulaSyntax,
    /// SDK `IFormulaField.FormulaNullTreatment` — the per-formula editor setting for how a null
    /// database-field operand is handled (default value vs. exception). Decoded from the `0x76`
    /// record trailer.
    pub null_treatment: super::enums::FormulaNullTreatment,
}

/// A persisted formula-language variable — a `Global`/`Shared` variable declared in the report's
/// formulas. Crystal formulas can share state through such variables; the engine writes the table of
/// the report's persisted (non-`Local`) ones. STRUCTURAL: no SDK accessor exposes these (they surface
/// only as text inside each formula's body), so this is not exported — decoded for
/// completeness / formula-VM use.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FormulaVariable {
    /// The variable's identifier as written in the formula (e.g. `rowCounter`).
    pub name: String,
    /// The variable's declared value type (its FL result kind, mapped to [`FieldValueType`]).
    pub value_type: FieldValueType,
    /// The variable's declared scope (`Global`/`Shared`).
    pub scope: FormulaVariableScope,
}

/// SDK: `CrFormulaSyntaxEnum` — a formula's authoring dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FormulaSyntax {
    /// Crystal syntax.
    #[default]
    Crystal,
    /// Basic (VB-like) syntax.
    Basic,
}

/// SDK: `IParameterField`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ParameterField {
    /// SDK `ParameterFieldType` — whether this is an ordinary report parameter or a stored-procedure
    /// input parameter.
    pub parameter_type: ParameterType,
    /// SDK `ParameterValueRangeKind` — the parameter's value type (string/number/currency/…).
    pub value_kind: ParameterValueKind,
    /// SDK `PromptText` — the text shown to the user when prompting for a value.
    pub prompt_text: Option<String>,
    /// The name of the (sub)report that declares this parameter, when it differs from the current one.
    pub report_name: Option<String>,
    /// SDK `EditMask` — the input mask applied to the prompt's text entry.
    pub edit_mask: Option<String>,
    /// SDK `EnableAllowEditingDefaultValues`-adjacent flag — the user may type a value not present in
    /// the pick list.
    pub allow_custom_values: bool,
    /// SDK `EnableAllowEditingDefaultValue` — the default value may be edited when prompting.
    pub allow_editing_default_value: bool,
    /// SDK `EnableAllowMultipleValue` — the parameter accepts more than one selected value.
    pub allow_multiple_values: bool,
    /// SDK `EnableNullValue` — the parameter accepts a null (no value) selection.
    pub allow_null_value: bool,
    /// A saved last-used ("current") value is present for this parameter (SDK
    /// `HasCurrentValue`). True for every parameter of a *main* report that carries saved data;
    /// always False for sub-report parameters (the engine only records current values per saved
    /// sub-result, which is not recoverable from the definition).
    pub has_current_value: bool,
    /// SDK `EnableOptionalPrompt` — the user may skip supplying a value for this parameter.
    pub optional_prompt: bool,
    /// The parameter is shown on the parameter-entry panel (as opposed to being formula/link-driven
    /// only).
    pub show_on_panel: bool,
    /// The parameter's value is editable from the parameter-entry panel.
    pub editable_on_panel: bool,
    /// SDK `DefaultValues` — the pick list / default value(s) offered when prompting.
    pub default_values: Vec<ParameterValue>,
    /// SDK `CurrentValues` — the last-used value(s), present only when [`has_current_value`](Self::has_current_value) is set.
    pub current_values: Vec<ParameterValue>,
    /// SDK `InitialValues` — the value(s) the parameter is initialized with before any user input.
    pub initial_values: Vec<ParameterValue>,
    /// SDK `@DefaultValueDisplayType`: how the default-value pick list is displayed
    /// (`DescriptionAndValue` / `Description`).
    pub default_value_display_type: super::enums::ParameterDisplayType,
    /// SDK `@DefaultValueSortOrder`: the sort applied to the default-value pick list
    /// (`NoSort` / `AlphabeticalAscending`).
    pub default_value_sort_order: super::enums::ParameterSortOrder,
    /// SDK `@DiscreteOrRangeKind` — whether the parameter accepts discrete values, a range value,
    /// or both. Recovered from the decoded value structure (a parameter carrying a range value is a
    /// `RangeValue`), as its stored `0x007a` byte is unlocated.
    pub discrete_or_range_kind: DiscreteOrRangeKind,
    /// SDK `PromptGroupRef` (PromptManager XML) — the GUID of the prompt group this parameter
    /// belongs to. A **cascading** (parent→child, e.g. country→state→city) prompt group shares one
    /// group GUID across its ordered levels; an ordinary parameter has its own auto-generated
    /// singleton group. `None` when the PromptManager entry omits it (e.g. an orphan formula-only
    /// parameter).
    pub prompt_group: Option<String>,
    /// SDK `Boolean_PartOfGroup` (PromptManager property) — the parameter is a member of a
    /// multi-parameter prompt group (a cascading or mutually-exclusive group). `false` for a
    /// standalone parameter.
    pub part_of_group: bool,
    /// SDK `Boolean_MutuallyExclusiveGroup` (PromptManager property) — the members of this
    /// parameter's prompt group are mutually exclusive.
    pub mutually_exclusive_group: bool,
    /// The dynamic (database-sourced) list-of-values binding for a dynamic parameter, when present.
    /// STRUCTURAL: the LOV data-source binding is not exposed by any SDK accessor and is not
    /// stored in any decodable `Contents` / `PromptManager` location, so this stays `None`; the
    /// field exists so a reader that recovers one can represent it.
    pub dynamic_lov: Option<DynamicLovBinding>,
}

/// SDK: `IParameterFieldValue`. A parameter value is either **discrete** (`range == None`, the
/// scalar in `value`) or a **range** (`range == Some`, with `value` holding the range's lower/start
/// bound and [`ParameterRange`] the upper bound + each end's inclusivity).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ParameterValue {
    /// The discrete value, or — when [`range`](Self::range) is `Some` — the range's lower (start)
    /// bound, formatted like a discrete value (empty for an open lower end).
    pub value: String,
    /// The value's display description in the pick list, when distinct from the raw value.
    pub description: Option<String>,
    /// When `Some`, this value is a **range** rather than a discrete value; carries the upper bound
    /// and the inclusivity of both ends. [`value`](Self::value) holds the lower bound.
    pub range: Option<ParameterRange>,
}

/// The upper bound and bound inclusivity of a **range** [`ParameterValue`]. The lower bound is the
/// value's [`ParameterValue::value`]. SDK: `IRangeValue` (`BeginValue` / `EndValue` +
/// `LowerBoundType` / `UpperBoundType`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ParameterRange {
    /// The range's upper (end) bound, formatted like a discrete value; empty for an open upper end.
    pub end_value: String,
    /// SDK `LowerBoundType` — inclusivity of the lower (start) bound.
    pub lower_bound: RangeBoundType,
    /// SDK `UpperBoundType` — inclusivity of the upper (end) bound.
    pub upper_bound: RangeBoundType,
}

/// A **dynamic** parameter's list-of-values (LOV) data-source binding: the pick list is read live
/// from a database object rather than stored in the report. STRUCTURAL — no SDK accessor
/// exposes it; modeled so a reader that decodes the binding (and, separately, a data path that
/// resolves the LOV against a live database) can represent it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DynamicLovBinding {
    /// The kind of database object the pick list is sourced from.
    pub source_kind: LovSourceKind,
    /// The source object name (table / view / stored procedure) or the SQL command text.
    pub source: String,
    /// The column supplying each pick-list value.
    pub value_field: String,
    /// The column supplying each value's description (empty when the LOV has no description column).
    pub description_field: String,
}

/// SDK: `ISummaryField`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SummaryField {
    /// SDK `IField.SummaryInfo.SummaryType` — the aggregate operation (Sum, Average, …).
    pub operation: SummaryOperation,
    /// The name of the database/formula field being summarized.
    pub summarized_field: String,
    /// SDK `ISummaryField.SecondarySummarizedField` — the second operand of a *two-field* summary
    /// operation: the paired field for `Correlation`/`Covariance`, or the weight field for
    /// `WeightedAvg`. Empty for every single-field operation (the overwhelmingly common case) and for
    /// a report that carries no two-field summary. Stored as the raw field reference (`Table.field`
    /// or `@formula`), like [`summarized_field`](Self::summarized_field).
    ///
    /// UNVERIFIED: two-field summaries are unobserved, so the `0x7e` leaf byte offset this decodes
    /// from is provisional (see the decoder).
    pub secondary_summarized_field: String,
    /// The operation's extra numeric parameter (e.g. the N in NthLargest/NthSmallest/Percentile).
    pub operation_parameter: i32,
    /// The index of the group this summary is scoped to; `None` for a grand-total (report-level)
    /// summary.
    pub group_index: Option<i32>,
    /// SDK `ISummaryField.IsPercentageSummary` — the summary is shown as a percentage of a group
    /// total (`PercentOf<Op>`) rather than the raw aggregate. The base [`operation`](Self::operation)
    /// still reports the underlying aggregate (`Sum`); the percentage is a display mode, not a
    /// distinct operation.
    pub is_percentage_summary: bool,
}

/// SDK: `IRunningTotalField`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RunningTotalField {
    /// SDK `IRunningTotalFieldController.Operation` — the aggregate operation (Sum, Average, …).
    pub operation: SummaryOperation,
    /// The name of the database/formula field being accumulated.
    pub summarized_field: String,
    /// SDK `IRunningTotalField.SecondarySummarizedField` — the second operand of a *two-field*
    /// running-total operation (`Correlation`/`Covariance`/`WeightedAvg`), analogous to
    /// [`SummaryField::secondary_summarized_field`]. Empty for every single-field running total.
    /// UNVERIFIED — two-field running totals are unobserved.
    pub secondary_summarized_field: String,
    /// The operation's extra numeric parameter (e.g. the N in NthLargest/NthSmallest/Percentile).
    pub operation_parameter: i32,
    /// SDK `EvaluationCondition` — when a new record is included in the running total.
    pub evaluation: EvaluationConditionType,
    /// SDK `ResetCondition` — when the accumulator is reset back to its starting value.
    pub reset: ResetConditionType,
    /// The database/formula field whose change drives an `OnChangeOfField` evaluate/reset condition.
    /// Empty unless `evaluation` or `reset` is `OnChangeOfField`.
    pub on_change_field: String,
}

/// SDK: `IGroupNameField`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GroupNameField {
    /// The index of the group whose name this field displays.
    pub group_index: i32,
    /// The name of that group's condition field, whose value is rendered.
    pub group_name_field_name: String,
}

/// SDK: `ISQLExpressionField`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SqlExpressionField {
    /// The raw SQL expression text, evaluated by the database rather than the report engine.
    pub text: String,
}

/// SDK: `ISpecialField` (page number, print date, …).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SpecialField {
    /// Which built-in special field this is (page number, print date, …).
    pub special_type: super::enums::SpecialFieldType,
}

/// SDK: `IGroup`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Group {
    /// The name of the database/formula field the group breaks on.
    pub condition_field: String,
    /// The group's sort direction/kind over [`condition_field`](Self::condition_field).
    pub sort: Sort,
    /// SDK `IGroupOptions` for this group (currently no decoded members; see [`GroupOptions`]).
    pub options: GroupOptions,
    /// The group's date/time/boolean grouping condition (see [`GroupCondition`]). `None` for a
    /// discrete ("for each value") grouping or a non-date/time/boolean field — the common case.
    /// `Some` drives the engine's `GroupName`/summary data-source operands and the render pipeline's
    /// period bucketing.
    pub date_condition: Option<GroupCondition>,
    /// The group's `<GroupAreaFormat>` flags (KeepTogether / RepeatHeader / VisibleGroupsPerPage).
    /// The outermost group is not described by one, so it keeps the defaults.
    pub area_format: super::report_def::GroupAreaFormat,
    /// For a **specified-order** (hierarchical) group, the ordered list of named group values and the
    /// condition-formula that defines each. Empty for an ordinary "for each value"/ascending/descending
    /// group. STRUCTURAL: there is no reader for the specified-order value list, so it is not
    /// exported; decoded as a stored fact for completeness.
    pub hierarchical: Vec<HierarchicalGroupValue>,
    /// Crystal's **Hierarchical Grouping** (Report ▸ Hierarchical Grouping Options): an org-tree
    /// layout where each group instance nests under its parent by matching a parent-id field to an
    /// instance-id field. `Some` only when enabled. Distinct from
    /// [`hierarchical`](Self::hierarchical), which is the unrelated "in specified order" grouping.
    pub hierarchical_options: Option<HierarchicalGroupOptions>,
}

/// Crystal's **Hierarchical Grouping** options for a group (SDK `IArea`
/// `EnableHierarchicalGroupSorting` / `ParentIDField` / `InstanceIDField` / `GroupIndent`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HierarchicalGroupOptions {
    /// `EnableHierarchicalGroupSorting` — always `true` when this struct is present.
    pub enabled: bool,
    /// The field whose value identifies each row's **parent** instance (`ParentIDField`), stored as a
    /// bare `table.field` reference (no braces).
    pub parent_id_field: String,
    /// The field whose value identifies each row's **own** instance (`InstanceIDField`, normally the
    /// group's own condition field), stored as a bare `table.field` reference.
    pub instance_id_field: String,
    /// Left indent applied per hierarchy level, in twips (`GroupIndent`; from the `0x0088`
    /// GroupAreaFormat record). `0` when unset.
    pub group_indent: Twips,
}

/// One named value of a **specified-order** (hierarchical) group. Crystal's "in specified order"
/// grouping lets the author name each bucket (e.g. `"High"`, `"Medium"`, `"Low"`) and give the
/// boolean condition-formula that assigns rows to it. STRUCTURAL — no SDK accessor, not on any output
/// surface.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HierarchicalGroupValue {
    /// The bucket's display name (e.g. `"High"`).
    pub value_name: String,
    /// The condition-formula that assigns rows to this bucket
    /// (e.g. `{Command.some_field} = "X"`).
    pub condition: String,
}

/// SDK: `IGroupOptions` (date condition, keep-together, … — deferred detail).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GroupOptions {}

/// A group's date / time-of-day / boolean grouping condition — Crystal's *polymorphic* group
/// condition. In the RAS model this is not one enum but two, chosen by the runtime type of the
/// group's `Options` object (which follows the group field's value type):
///
/// * `CrDateConditionEnum` (a Date/Time/DateTime field) — eight calendar periods (ordinals `0..=7`)
///   then four time-of-day periods (ordinals `8..=11`).
/// * `CrBooleanConditionEnum` (a Boolean field) — six transition / look-ahead conditions
///   (ordinals `1..=6`; the enum has no `0`).
///
/// There is **no** "any value" / discrete member on either enum: a discrete ("for each value")
/// grouping carries no condition object at all, modeled here as [`Group::date_condition`] `== None`.
///
/// The calendar and time-of-day variants are the group analogue of the chart-side
/// [`ChartCategoryPeriod`](crate::ChartCategoryPeriod); `Daily`/`Weekly`/`Monthly` are established,
/// the other calendar/time periods follow the SDK ordering. The six boolean conditions are modeled
/// from the SDK enum and the native designer's own option strings and are **provisional**; they are
/// decoded only for a Boolean group field, so they cannot misfire on the date path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum GroupCondition {
    /// `CrDateConditionEnum` 0 — one bucket per calendar day.
    Daily,
    /// `CrDateConditionEnum` 1 — one bucket per week (week-start Sunday, Crystal's default).
    Weekly,
    /// `CrDateConditionEnum` 2 — one bucket per two weeks.
    BiWeekly,
    /// `CrDateConditionEnum` 3 — two buckets per month (1st–15th, 16th–end).
    SemiMonthly,
    /// `CrDateConditionEnum` 4 — one bucket per calendar month.
    Monthly,
    /// `CrDateConditionEnum` 5 — one bucket per calendar quarter.
    Quarterly,
    /// `CrDateConditionEnum` 6 — one bucket per half-year. (SDK spells this `SemiAnnually`.)
    SemiAnnually,
    /// `CrDateConditionEnum` 7 — one bucket per calendar year.
    Annually,
    /// `CrDateConditionEnum` 8 — one bucket per second (keeps the date).
    BySecond,
    /// `CrDateConditionEnum` 9 — one bucket per minute (keeps the date).
    ByMinute,
    /// `CrDateConditionEnum` 10 — one bucket per hour (keeps the date).
    ByHour,
    /// `CrDateConditionEnum` 11 — two buckets per day, AM / PM (keeps the date).
    ByAMPM,
    /// `CrBooleanConditionEnum` 1 — a new group starts where the value transitions False→True.
    ToYes,
    /// `CrBooleanConditionEnum` 2 — a new group starts where the value transitions True→False.
    ToNo,
    /// `CrBooleanConditionEnum` 3 — every True-valued row starts its own group.
    EveryYes,
    /// `CrBooleanConditionEnum` 4 — every False-valued row starts its own group.
    EveryNo,
    /// `CrBooleanConditionEnum` 5 — a look-ahead break one row before the value becomes True.
    NextIsYes,
    /// `CrBooleanConditionEnum` 6 — a look-ahead break one row before the value becomes False.
    NextIsNo,
    /// A condition ordinal with no named variant, preserved so an unrecognized value round-trips
    /// (and is surfaced) instead of silently collapsing to a discrete group.
    Other(i32),
}

impl GroupCondition {
    /// Decode a `CrDateConditionEnum` ordinal for a Date/Time/DateTime group field. Ordinals `1..=11`
    /// map to the eleven non-daily periods; ordinal `0` returns `None` — it is both `Daily` *and* the
    /// value on a discrete date group, so daily is resolved by the caller from the legacy daily flag
    /// (gated on field type) to avoid a false positive on a discrete group. An ordinal `>= 12` is out
    /// of range for the date enum and surfaces as [`Other`](Self::Other).
    pub fn from_date_ordinal(ordinal: u8) -> Option<Self> {
        Some(match ordinal {
            0 => return None,
            1 => Self::Weekly,
            2 => Self::BiWeekly,
            3 => Self::SemiMonthly,
            4 => Self::Monthly,
            5 => Self::Quarterly,
            6 => Self::SemiAnnually,
            7 => Self::Annually,
            8 => Self::BySecond,
            9 => Self::ByMinute,
            10 => Self::ByHour,
            11 => Self::ByAMPM,
            other => Self::Other(i32::from(other)),
        })
    }

    /// Map the legacy internal date-group `<code>` (the byte after the `@Group #N Order` marker,
    /// structure `01 00 <code> ff ff`) to its calendar period. `0x01` = daily, `0x03` = monthly,
    /// `0x06`/`0x08` = weekly (two week codes — week-of-year vs. a fixed-weekday week — both weekly).
    /// This `<code>` is *not* the SDK ordinal; it is a fallback for reports that leave the SDK-ordinal
    /// byte at `0`. Returns `None` for an unknown code.
    pub fn from_legacy_date_code(code: u8) -> Option<Self> {
        match code {
            0x01 => Some(Self::Daily),
            0x03 => Some(Self::Monthly),
            0x06 | 0x08 => Some(Self::Weekly),
            _ => None,
        }
    }

    /// Decode a `CrBooleanConditionEnum` ordinal for a Boolean group field. The boolean enum starts at
    /// `1`; ordinal `0` is a discrete boolean group → `None`. Ordinals `1..=6` map to the six
    /// conditions; anything else surfaces as [`Other`](Self::Other). UNVERIFIED — boolean groups are
    /// unobserved; gated on Boolean field type so it never touches the date path.
    pub fn from_boolean_ordinal(ordinal: u8) -> Option<Self> {
        Some(match ordinal {
            0 => return None,
            1 => Self::ToYes,
            2 => Self::ToNo,
            3 => Self::EveryYes,
            4 => Self::EveryNo,
            5 => Self::NextIsYes,
            6 => Self::NextIsNo,
            other => Self::Other(i32::from(other)),
        })
    }

    /// The lowercase canonical token for this condition — the token exported to KDL and (for the
    /// date/time conditions) the exact string the engine renders as the third operand of a
    /// group-scoped summary reference (`DistinctCount ({x}, {g}, "daily")`). `Other` reports
    /// `"unknown"`.
    pub fn token(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::BiWeekly => "biweekly",
            Self::SemiMonthly => "semimonthly",
            Self::Monthly => "monthly",
            Self::Quarterly => "quarterly",
            Self::SemiAnnually => "semiannually",
            Self::Annually => "annually",
            Self::BySecond => "bysecond",
            Self::ByMinute => "byminute",
            Self::ByHour => "byhour",
            Self::ByAMPM => "byampm",
            Self::ToYes => "toyes",
            Self::ToNo => "tono",
            Self::EveryYes => "everyyes",
            Self::EveryNo => "everyno",
            Self::NextIsYes => "nextisyes",
            Self::NextIsNo => "nextisno",
            Self::Other(_) => "unknown",
        }
    }

    /// The inverse of [`token`](Self::token): map a lowercase canonical token back to its condition.
    /// Used where a period is carried as a token string (e.g. the chart category-axis period, which
    /// has its own [`ChartCategoryPeriod`](crate::ChartCategoryPeriod) vocabulary sharing these
    /// tokens). Returns `None` for an unrecognized token (including `"unknown"`).
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "daily" => Self::Daily,
            "weekly" => Self::Weekly,
            "biweekly" => Self::BiWeekly,
            "semimonthly" => Self::SemiMonthly,
            "monthly" => Self::Monthly,
            "quarterly" => Self::Quarterly,
            "semiannually" => Self::SemiAnnually,
            "annually" => Self::Annually,
            "bysecond" => Self::BySecond,
            "byminute" => Self::ByMinute,
            "byhour" => Self::ByHour,
            "byampm" => Self::ByAMPM,
            "toyes" => Self::ToYes,
            "tono" => Self::ToNo,
            "everyyes" => Self::EveryYes,
            "everyno" => Self::EveryNo,
            "nextisyes" => Self::NextIsYes,
            "nextisno" => Self::NextIsNo,
            _ => return None,
        })
    }

    /// The SDK spelling the engine renders inside `GroupName ({field}, "…")` and as a group-scoped
    /// summary's period operand — the `CrDateConditionEnum` / `DateTimeCondition` name (`Monthly`,
    /// `SemiAnnually`, `BySecond`, …). Casing is authoritative, not a title-case transform
    /// (`SemiAnnually` and the `By…` forms are not plain title-case). `Some` only for the twelve
    /// date/time conditions; a boolean condition and [`Other`](Self::Other) return `None` — the
    /// engine's GroupName operand for a boolean group is unknown, so the bare
    /// `GroupName ({field})` form is emitted rather than a guessed operand.
    pub fn date_time_operand(self) -> Option<&'static str> {
        Some(match self {
            Self::Daily => "Daily",
            Self::Weekly => "Weekly",
            Self::BiWeekly => "Biweekly",
            Self::SemiMonthly => "Semimonthly",
            Self::Monthly => "Monthly",
            Self::Quarterly => "Quarterly",
            Self::SemiAnnually => "SemiAnnually",
            Self::Annually => "Annually",
            Self::BySecond => "BySecond",
            Self::ByMinute => "ByMinute",
            Self::ByHour => "ByHour",
            Self::ByAMPM => "ByAMPM",
            // Boolean conditions and unknown ordinals have no known GroupName/summary operand.
            Self::ToYes
            | Self::ToNo
            | Self::EveryYes
            | Self::EveryNo
            | Self::NextIsYes
            | Self::NextIsNo
            | Self::Other(_) => return None,
        })
    }
}

/// SDK: `ISort`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sort {
    /// The name of the database/formula field this sort orders by.
    pub field: String,
    /// SDK `SortDirection` — ascending/descending/unsorted (or Top N/Bottom N for a group sort).
    pub direction: SortDirection,
    /// SDK `@SortType` — whether this sort came from the record-sort or the group-sort collection.
    pub kind: SortKind,
    /// Top N / Bottom N group-sort options (SDK: `TopBottomNSortField`). `Some` only for a
    /// **summary-based** group sort — one where the group is sorted by a summary expression
    /// (`Sum (…, …)`), which is the only kind the engine exposes as a `TopBottomNSortField`.
    /// A plain group-field sort or a record sort leaves this `None` (no Top N attrs emitted).
    pub topn: Option<TopBottomNSort>,
}

/// SDK: `TopBottomNSortField` — the Top N / Bottom N options carried by a summary-based group sort.
/// `number_of_groups` is the group's Top N limit (`0` = no limit), `not_in_topn_name` is the
/// "Others"-bucket name (default `"Others"`). The two option flags follow that name in the `0xe5`
/// record as `[u8 WithTies][u8 DiscardOthers]`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TopBottomNSort {
    /// SDK `NumberOfNGroups` — the Top N / Bottom N group limit (`0` = no limit).
    pub number_of_groups: u16,
    /// SDK `EnableDiscardOtherGroups` — omit the "Others" bucket for groups outside the Top/Bottom N.
    /// Decoded from the option byte at the Top N name-end + 3 (`1` = set).
    pub discard_others: bool,
    /// SDK `TextForOther` — the display name of the "Others" bucket (default `"Others"`).
    pub not_in_topn_name: String,
    /// SDK `EnableWithTies` — include groups tied with the Nth for the last Top/Bottom slot. Decoded
    /// from the option byte at the Top N name-end + 2 (`1` = set). Unobserved: no stored Top N group
    /// has been seen to set it.
    pub with_ties: bool,
}

#[cfg(test)]
mod accessor_tests {
    use super::*;

    fn field(name: &str, kind: FieldKindData) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            kind,
            ..Default::default()
        }
    }

    #[test]
    fn per_kind_views_filter_and_preserve_order() {
        let dd = DataDefinition {
            field_definitions: vec![
                field("db1", FieldKindData::Database(DbField::default())),
                field("@f1", FieldKindData::Formula(FormulaField::default())),
                field("?p1", FieldKindData::Parameter(Box::default())),
                field("@f2", FieldKindData::Formula(FormulaField::default())),
                field(
                    "#rt1",
                    FieldKindData::RunningTotal(RunningTotalField::default()),
                ),
            ],
            ..Default::default()
        };

        // Formula view yields both formulas, in file order, with typed payloads.
        let formulas: Vec<&str> = dd.formula_fields().map(|(f, _)| f.name.as_str()).collect();
        assert_eq!(formulas, vec!["@f1", "@f2"]);
        // Other views isolate their kind.
        assert_eq!(dd.database_fields().count(), 1);
        assert_eq!(dd.parameter_fields().count(), 1);
        assert_eq!(dd.running_total_fields().count(), 1);
        assert_eq!(dd.summary_fields().count(), 0);
        // The unified vec is still the single source of truth.
        assert_eq!(dd.field_definitions.len(), 5);
    }
}
