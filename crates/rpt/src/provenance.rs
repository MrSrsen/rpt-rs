//! Binary-format provenance for the semantic model.
//!
//! The [`model`](crate::model) types (from the format-neutral [`rpt_model`] crate) describe *what* a
//! report contains, not *where* it is stored in the `.rpt` bytes. That byte-level provenance belongs
//! with the binary reader, so it lives here rather than cluttering the neutral model.
//!
//! This is a **reverse index**: given a model type, which `Contents` record it is decoded from.
//! `docs/07-block-catalog.md` is the same relation the other way round (given a record, what it holds
//! and how its bytes are laid out), and each record's **leaf layout** is documented at the
//! `project::raise` function that reads it. Offsets are deliberately not repeated here — a copy this
//! far from the parsing code has nothing to keep it honest.
//!
//! These modules are documentation only: they define no types and have no runtime effect. Records are
//! TSLV types in the `Contents` stream (a `u16` rtype); "leaf" is a record's demasked payload bytes.

/// Provenance for [`Report`](crate::model::Report) and its structural / designer members.
///
/// # [`Report`](crate::model::Report)
///
/// The report root. Most members are raised from dedicated record subtrees; the fields below carry
/// binary provenance that would otherwise sit on the neutral model:
///
/// - `version` — the `Contents` stream header's format-version word.
/// - `has_saved_data` — the saved-data block descriptor record `0x0061`.
/// - `saved_data` — the stored saved-data batch (`0x0061` and its rowset records).
/// - `save_metadata` — `Contents` record `0x0178`, one entry per record in stream order.
/// - `reimport` — the `0x0142` `SubreportReimportInfo` record (one per report even when it has no
///   subreports).
/// - `designer_state` — the `0x010c` snap-guideline and `0x0111` object-connection records.
///
/// The raw record substrate itself (the per-record DOM and its inventory) is not a model field; it
/// is projected on demand from the bytes via [`Rpt::record_dom`](crate::Rpt::record_dom) /
/// [`Rpt::inventory`](crate::Rpt::inventory).
///
/// # [`SaveMetadataEntry`](crate::model::SaveMetadataEntry)
///
/// One `Contents` record `0x0178`: a save-time environment key/value pair.
///
/// # [`SubreportReimportInfo`](crate::model::SubreportReimportInfo)
///
/// The `0x0142` record: a length-prefixed source `.rpt` path, then the two import timestamps and the
/// re-import enum.
///
/// # [`DesignerState`](crate::model::DesignerState) / [`Guideline`](crate::model::Guideline) / [`ObjectConnection`](crate::model::ObjectConnection)
///
/// The designer's on-canvas editing geometry, from records scattered across the `Contents` tree:
///
/// - `Guideline` — a `0x010c` `GuidelineEntry` record. The horizontal and vertical guides share the
///   record shape; the axis is implied by the parent collection.
/// - `ObjectConnection` — a `0x0111` `ObjectConnection` record.
pub mod report {}

/// Provenance for the data-definition types ([`DataDefinition`](crate::model::DataDefinition) and its
/// members).
///
/// # [`DataDefinition`](crate::model::DataDefinition)
///
/// - `running_total_condition_formulas` — the `0x77` condition-formula records named
///   `"… Condition Formula"`.
/// - `summary_binding_fields` — the pre-layout `0x7e` summary records (each wrapped in a `0x7f`),
///   excluding running totals (a `0x7e` preceded by a `0x80`).
/// - `formula_variables` — the `0x0118` formula-variable records (count in `0x0116`).
/// - `field_manager_census` — the `0x006e` `FieldManagerEntry` record.
///
/// # [`FieldManagerCensus`](crate::model::FieldManagerCensus)
///
/// The `0x006e` `FieldManagerEntry` record, one per report: `database_fields` counts `0x0073`
/// records, `formula_bodies` the `0x0076` records. See `data_def::raise_field_manager_census` for the
/// leaf and for the built-in-formula adjustment the stored count needs.
///
/// # Groups / sorts / parameters
///
/// - [`ParameterField`](crate::model::ParameterField) `default_value_display_type` /
///   `default_value_sort_order` — the `0x007a` parameter record, from two adjacent bytes past the
///   parameter-name string.
/// - [`GroupOptions`](crate::model::GroupOptions) — the `0x0088` record; a specified-order group's
///   named values come from the `0x00e9` `HierarchicalGroupingOptions` records following the group's
///   `0xe5`.
/// - [`HierarchicalGroupValue`](crate::model::HierarchicalGroupValue) — a `0x00e9`
///   `HierarchicalGroupingOptions` record.
/// - [`TopBottomNSort`](crate::model::TopBottomNSort) — all three values live in the group's `0xe5`
///   record, **not** the `0x29` sort record (see `data_def::decode_group_topn`).
/// - [`FormulaVariable`](crate::model::FormulaVariable) — the `0x0118` record.
pub mod data_def {}

/// Provenance for the field-format types ([`FieldFormat`](crate::model::FieldFormat) and its
/// sub-formats). The display-format sub-formats are the `0x00ee`..`0x00fb` record family — one
/// wrapper+child pair per sub-format, streamed after each `0x9f` field opener. Leaf layouts are
/// documented on the `report_def::formats` decoders.
///
/// - [`CommonFieldFormat`](crate::model::CommonFieldFormat) — `0x00f0`.
/// - [`NumericFieldFormat`](crate::model::NumericFieldFormat) — the **second** `0x00f8` record. Each
///   field emits two `0x00f9`/`0x00f8` pairs, a currency slot then a number slot; the engine surfaces
///   one based on the field's value type.
/// - [`BooleanFieldFormat`](crate::model::BooleanFieldFormat) — `0x00ee`.
/// - [`StringFieldFormat`](crate::model::StringFieldFormat) — `0x00fa`.
/// - [`DateFieldFormat`](crate::model::DateFieldFormat) — `0x00f2`.
/// - [`DateTimeFieldFormat`](crate::model::DateTimeFieldFormat) — `0x00f4`.
/// - [`TimeFieldFormat`](crate::model::TimeFieldFormat) — `0x00f6`. Most of the SDK time surface
///   (`TimeBase`, AM/PM strings, separators) is resolved at runtime from the host locale, so only the
///   three element-display enums are decoded.
/// - [`Border`](crate::model::Border) `attributes` — the `0xed` wrapper that parents the `0xec`
///   border.
pub mod format {}

/// Provenance for the enum types whose stored byte is worth locating. The `sdk_enum!` variants map a
/// stored byte to an SDK ordinal; the notes below record which record that byte lives in.
///
/// - [`FormulaVariableScope`](crate::model::FormulaVariableScope) — the `0x0118` formula-variable
///   record.
/// - [`RoundingFormat`](crate::model::RoundingFormat) — the second `0x00f8` numeric record. The
///   stored code encodes the decimal-place count as `11 - places`, so it is not a plain ordinal (see
///   [`RoundingFormat::from_code`](crate::model::RoundingFormat::from_code)).
/// - [`DayFormat`](crate::model::DayFormat) / [`MonthFormat`](crate::model::MonthFormat) /
///   [`YearFormat`](crate::model::YearFormat) / [`DateSystemDefaultType`](crate::model::DateSystemDefaultType) /
///   [`DayOfWeekFormat`](crate::model::DayOfWeekFormat) — the `0x00f2` date leaf.
/// - [`NegativeFormat`](crate::model::NegativeFormat) / [`CurrencySymbolFormat`](crate::model::CurrencySymbolFormat)
///   / [`CurrencyPosition`](crate::model::CurrencyPosition) — the second `0x00f8` numeric record.
/// - [`BooleanOutputType`](crate::model::BooleanOutputType) — the `0x00ee` boolean record.
/// - [`ParameterDisplayType`](crate::model::ParameterDisplayType) / [`ParameterSortOrder`](crate::model::ParameterSortOrder)
///   — the `0x007a` parameter record.
pub mod enums {}

/// Provenance for the report-object types ([`ReportObject`](crate::model::ReportObject) and kin). Each
/// object is opened by a type-specific `Contents` record; text/chart/cross-tab objects nest further
/// records.
///
/// - [`FieldObject`](crate::model::FieldObject) — its [`FieldRefKind`](crate::model::FieldRefKind) is
///   the type byte in the field-object opener record; a placed summary carries the `0x7e` summary
///   record's code.
/// - [`TextObject`](crate::model::TextObject) / [`Paragraph`](crate::model::Paragraph) /
///   [`TextRun`](crate::model::TextRun) — a paragraph opens with `0x00c0`; its runs are literal-text
///   `0x00c2` elements and embedded-reference `0x00c4` elements; a run's own font is a `0x08` record.
/// - [`PictureObject`](crate::model::PictureObject) — the bound field reference comes from the `0x00b1`
///   wrapper around the picture opener; OLE embedding is the `0xbd` `OleObjectItem` record.
/// - [`SubreportObject`](crate::model::SubreportObject) — `subdoc_index` names the backing
///   `Subdocument N` storage.
/// - [`ChartObject`](crate::model::ChartObject) / [`ChartDefinition`](crate::model::ChartDefinition) —
///   the `0x0121` `ChartDefinition2` record carries the type/subtype enums, the title and axis-title
///   strings, and the legend/gridline/data-label styling; the data-value label is the sibling `0x011f`
///   `ChartDataValue` record; the category period comes from the chart's `0xe5` grid-group record.
/// - [`CrossTabObject`](crate::model::CrossTabObject) — opener `0xb8`, wrapped by `0xb9`. Its
///   dimensions are `0x00cb` `CrossTabDimensionField` records (nested `0x00ce → 0x00cc → 0x00cb`),
///   column levels under `0x00ce` and row levels under `0x00d2`; measures are pre-layout `0x7e`
///   summaries counted by the `0x00db` `CrossTabFieldGrid` record.
pub mod objects {}

/// Provenance for the raw-record DOM ([`Node`](crate::raw::Node) / [`Unknown`](crate::raw::Unknown)),
/// projected on demand by [`Rpt::record_dom`](crate::Rpt::record_dom).
/// [`Unknown`](crate::raw::Unknown) already carries the raw `rtype`/`subtype` and decoded leaf
/// values — it *is* the raw substrate. [`Node::FieldDef`](crate::raw::Node::FieldDef) is the
/// modelled field-definition record `0x73`.
pub mod dom {}
