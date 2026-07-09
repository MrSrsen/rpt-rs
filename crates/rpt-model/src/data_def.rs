//! Data definition & fields (SDK: `IDataDefinition`, `IField` + kinds).
//!
//! Modelled from the records: `Contents` stores only the fields a report references, not the full
//! table schema.

use super::enums::{
    DiscreteOrRangeKind, EvaluationConditionType, FieldValueType, FormulaVariableScope,
    LovSourceKind, ParameterType, ParameterValueKind, RangeBoundType, ResetConditionType,
    SortDirection, SortKind, SummaryOperation,
};
use super::primitives::Formula;

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
    /// conditional formulas) that are not field definitions. Retained as decoded formula text so the
    /// export layer can account for the field/parameter references they contain.
    pub condition_formula_bodies: Vec<String>,
    /// Bodies of running-total **condition** formulas only (named `"… Condition Formula"`: a running
    /// total's evaluate/reset condition). Each is a distinct
    /// persistent formula, so every database field it names contributes one to that field's
    /// `IField.UseCount`. These are never attached to a section/object (unlike the entries in
    /// `condition_formula_bodies`), so the UseCount counter scans this list separately to avoid
    /// double-counting.
    pub running_total_condition_formulas: Vec<String>,
    /// The summarized database/formula field of every **summary definition** in the data-definition
    /// region — the summary definitions that precede the report layout, one per `ISummaryField`,
    /// excluding running totals and the chart/cross-tab data bindings that live inside the layout.
    /// Most map 1:1 to a *placed* summary
    /// (counted from its field object); the surplus are **orphan** summary definitions with no placed
    /// object that the engine still refcounts. The UseCount counter adds the orphans (per-field
    /// surplus over the placed summaries).
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
}

/// The field-pool census: a compact tally of the report's field manager that mirrors the decoded
/// [`field_definitions`](DataDefinition::field_definitions); modeled as a cross-check. STRUCTURAL —
/// internal (not exported).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FieldManagerCensus {
    /// Number of database-field definitions — matches the decoded db-field count exactly across the
    /// corpus.
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
    /// SDK `IField.Name` — the field's identifier as referenced from formulas/XML.
    pub name: String,
    /// SDK `IField.Type` — the field's value type.
    pub value_type: FieldValueType,
    /// SDK `Length` (XML `@NumberOfBytes`).
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

/// SDK: `FieldKind` (`CrystalDecisions.CrystalReports.Engine.FieldKind`) — the discriminant tag a
/// field definition carries in the `Kind=` XML attribute. This is the value-only form of
/// [`FieldKindData`] (which also holds the per-kind payload); [`FieldKindData::field_kind`] projects
/// one to the other. Values match the SDK enum (1-based); [`FieldKind::name`] returns the SDK
/// `ToString()`.
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
    /// Raw formula option bitflags (subtype not yet decoded).
    pub options: i32,
    /// The byte size of the formula's result (`NumberOfBytes`): the value type's intrinsic length
    /// for fixed types, or twice the maximum character count for a string result.
    pub number_of_bytes: i32,
    /// SDK `IFormulaField.Syntax` — the formula's authoring dialect.
    pub syntax: FormulaSyntax,
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
    /// or both. `DiscreteValue` for every observed corpus report.
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
    /// STRUCTURAL: the LOV data-source binding is not exposed by the SDK / RptToXml and is not
    /// stored in a decodable `Contents` / `PromptManager` location in the observed corpus, so this
    /// stays `None` there; the field exists so a reader that recovers one can represent it.
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
/// from a database object rather than stored in the report. STRUCTURAL — no SDK / RptToXml accessor
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
    /// The operation's extra numeric parameter (e.g. the N in NthLargest/NthSmallest/Percentile).
    pub operation_parameter: i32,
    /// The index of the group this summary is scoped to; `None` for a grand-total (report-level)
    /// summary.
    pub group_index: Option<i32>,
}

/// SDK: `IRunningTotalField`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RunningTotalField {
    /// SDK `IRunningTotalFieldController.Operation` — the aggregate operation (Sum, Average, …).
    pub operation: SummaryOperation,
    /// The name of the database/formula field being accumulated.
    pub summarized_field: String,
    /// The operation's extra numeric parameter (e.g. the N in NthLargest/NthSmallest/Percentile).
    pub operation_parameter: i32,
    /// SDK `EvaluationCondition` — when a new record is included in the running total.
    pub evaluation: EvaluationConditionType,
    /// SDK `ResetCondition` — when the accumulator is reset back to its starting value.
    pub reset: ResetConditionType,
    /// The database/formula field whose change drives an `OnChangeOfField` evaluate/reset condition.
    /// Empty unless `evaluation` or `reset` is `OnChangeOfField`. Not emitted to XML; it is a
    /// persistent field reference, so it contributes to that field's `UseCount`.
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

/// SDK: `IGroup` (XML `<Group>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Group {
    /// The name of the database/formula field the group breaks on.
    pub condition_field: String,
    /// The group's sort direction/kind over [`condition_field`](Self::condition_field).
    pub sort: Sort,
    /// SDK `IGroupOptions` for this group (currently no decoded members; see [`GroupOptions`]).
    pub options: GroupOptions,
    /// For a date/time/boolean group condition, the engine's date-grouping token rendered in
    /// `GroupName`/summary data sources (e.g. `daily` -> `GroupName ({fld}, "Daily")`). `None` for a
    /// discrete ("for each value") grouping or a non-date field. Stored lowercase; `GroupName`
    /// title-cases it, summaries keep it lowercase.
    pub date_condition: Option<String>,
    /// The group's `<GroupAreaFormat>` flags (KeepTogether / RepeatHeader / VisibleGroupsPerPage).
    /// The outermost group is not described by one, so it keeps the defaults.
    pub area_format: super::report_def::GroupAreaFormat,
    /// For a **specified-order** (hierarchical) group, the ordered list of named group values and the
    /// condition-formula that defines each. Empty for an ordinary "for each value"/ascending/descending
    /// group. STRUCTURAL: there is no reader for the specified-order value list, so it is not
    /// exported; decoded as a stored fact for completeness.
    pub hierarchical: Vec<HierarchicalGroupValue>,
}

/// One named value of a **specified-order** (hierarchical) group. Crystal's "in specified order"
/// grouping lets the author name each bucket (e.g. `"High"`, `"Medium"`, `"Low"`) and give the
/// boolean condition-formula that assigns rows to it. STRUCTURAL — no SDK accessor, not on the XML
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

/// SDK: `ISort` (XML `<SortField>`).
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
/// "Others"-bucket name (default `"Others"`). `discard_others` (SDK `EnableDiscardOtherGroups`) is
/// `false` everywhere in the corpus and cannot be located from it, so it is decoded as `false` until
/// a non-default sample surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TopBottomNSort {
    /// SDK `NumberOfNGroups` — the Top N / Bottom N group limit (`0` = no limit).
    pub number_of_groups: u16,
    /// SDK `EnableDiscardOtherGroups` — omit the "Others" bucket for groups outside the Top/Bottom N.
    pub discard_others: bool,
    /// SDK `TextForOther` — the display name of the "Others" bucket (default `"Others"`).
    pub not_in_topn_name: String,
    /// SDK `EnableWithTies` — include groups tied with the Nth for the last Top/Bottom slot. Not
    /// exported (absent from every corpus report) and `false` for every Top N group in the
    /// corpus, so — like `discard_others` — its stored byte cannot be located/verified from the
    /// corpus. Decoded as `false` (unverified: no non-default corpus sample) until one surfaces.
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
