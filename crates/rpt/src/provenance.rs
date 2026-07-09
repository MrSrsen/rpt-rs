//! Binary-format provenance for the semantic model.
//!
//! The [`model`](crate::model) types (from the format-neutral [`rpt_model`] crate) describe *what* a
//! report contains, not *where* it is stored in the `.rpt` bytes. That byte-level provenance — which
//! `Contents` record a field is decoded from, and its leaf layout — belongs with the binary reader,
//! so it lives here rather than cluttering the neutral model.
//!
//! These modules are documentation only: they define no types and have no runtime effect. Each links
//! to a model type and records the on-disk origin of its stored fields. The decoding itself is
//! performed by the `project::raise` pass. Records are TSLV types in the `Contents` stream (a `u16`
//! rtype); "leaf" is a record's demasked payload bytes.

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
/// - `records` / `record_inventory` — the raw record substrate; every TSLV record is represented,
///   typed where decoded and [`Unknown`](crate::model::Unknown) otherwise.
///
/// # [`SaveMetadataEntry`](crate::model::SaveMetadataEntry)
///
/// One `Contents` record `0x0178`: a save-time environment key/value pair, length-prefixed key then
/// value.
///
/// # [`SubreportReimportInfo`](crate::model::SubreportReimportInfo)
///
/// The `0x0142` record. Leaf layout:
/// `[u32 BE L][source path: L bytes incl NUL][imported_at][enum 1B][source_saved_at]`, each timestamp
/// a compound `(Julian-day, time-fraction)` `u32` pair. The path is empty (`L == 1`, a lone NUL)
/// across the corpus.
///
/// # [`DesignerState`](crate::model::DesignerState) / [`Guideline`](crate::model::Guideline) / [`ObjectConnection`](crate::model::ObjectConnection)
///
/// The designer's on-canvas editing geometry, from records scattered across the `Contents` tree:
///
/// - `Guideline` — a `0x010c` `GuidelineEntry` record (`[u32 BE position-twips][u16 flags]`; the
///   horizontal and vertical guides share the record shape, the axis implied by the parent
///   collection).
/// - `ObjectConnection` — a `0x0111` `ObjectConnection` record
///   (`[u16 src][u16 dst][8×00][u16 kind][8×ff]`); the edge `kind` word read at the fixed offset 12 is
///   `0x0002` for every real edge in the corpus (a degenerate `0 → 0` root edge stores it shifted).
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
/// The `0x006e` `FieldManagerEntry` record (20-byte leaf, one per report). Leaf layout:
/// `[u32 BE database_fields][u16 BE formula-body-count-less-3][u16 BE = 18 marker]` then four further
/// `u16` per-kind counts left undecoded. `database_fields` counts `0x0073` records; `formula_bodies`
/// is the `0x0076` record count, stored **less the three built-in formulas** (so the decoder adds 3
/// back).
///
/// # Groups / sorts / parameters
///
/// - [`ParameterField`](crate::model::ParameterField) `default_value_display_type` /
///   `default_value_sort_order` — the `0x007a` parameter record (a byte at, respectively, offset 4
///   and 5 past the end of the parameter-name length-prefixed string).
/// - [`GroupOptions`](crate::model::GroupOptions) — the `0x0088` record; a specified-order group's
///   named values come from the `0x00e9` `HierarchicalGroupingOptions` records following the group's
///   `0xe5`.
/// - [`HierarchicalGroupValue`](crate::model::HierarchicalGroupValue) — a `0x00e9`
///   `HierarchicalGroupingOptions` record: two length-prefixed strings (`u32` BE byte count incl.
///   trailing NUL).
/// - [`TopBottomNSort`](crate::model::TopBottomNSort) — all three values live in the group's `0xe5`
///   record (not the `0x29` sort record): `number_of_groups` is a `u16` BE 11 bytes from the end of
///   the `0xe5` leaf, `not_in_topn_name` a length-prefixed string in the Top N tail.
/// - [`FormulaVariable`](crate::model::FormulaVariable) — the `0x0118` record.
pub mod data_def {}

/// Provenance for the field-format types ([`FieldFormat`](crate::model::FieldFormat) and its
/// sub-formats). The display-format sub-formats are the `0x00ee`..`0x00fb` record family — one
/// wrapper+child pair per sub-format, streamed after each `0x9f` field opener.
///
/// - [`FieldFormat`](crate::model::FieldFormat) — the family as a whole; the byte-derived sub-formats
///   are [`CommonFieldFormat`](crate::model::CommonFieldFormat) (`0x00f0`),
///   [`NumericFieldFormat`](crate::model::NumericFieldFormat) (the second `0x00f8`),
///   [`BooleanFieldFormat`](crate::model::BooleanFieldFormat) (`0x00ee`),
///   [`StringFieldFormat`](crate::model::StringFieldFormat) (`0x00fa`), and
///   [`DateFieldFormat`](crate::model::DateFieldFormat) (`0x00f2`). The time sub-format is `0x00f6`
///   (fully runtime-resolved, so not modelled).
/// - [`DateFieldFormat`](crate::model::DateFieldFormat) — the `0x00f2` leaf: its first bytes are, in
///   order, `dateOrder`, `yearType`, `monthType`, `dayType`, `dayOfWeekType` (byte 4),
///   `windowsDefaultType`, `eraType`, `calendarType`.
/// - [`CommonFieldFormat`](crate::model::CommonFieldFormat) — the `0x00f0` record:
///   `suppress_if_duplicated` = byte 0, `use_system_defaults` = byte 3.
/// - [`NumericFieldFormat`](crate::model::NumericFieldFormat) — the second `0x00f8` record: byte 2 =
///   negative, byte 8 = decimal places, byte 9 = rounding (stored as `11 - decimalPlaces`), byte 10 =
///   currency symbol. After a fixed 14-byte scalar header the leaf carries length-prefixed strings;
///   the first three are the decimal / thousand / currency symbols.
/// - [`BooleanFieldFormat`](crate::model::BooleanFieldFormat) — the `0x00ee` record, byte 0 = output
///   type.
/// - [`Border`](crate::model::Border) `attributes` — the `0xed` wrapper that parents the `0xec`
///   border.
pub mod format {}

/// Provenance for the enum types whose byte encoding is worth recording. The `sdk_enum!` variants map
/// a stored byte to an SDK ordinal; the notes below record where that byte lives.
///
/// - [`FormulaVariableScope`](crate::model::FormulaVariableScope) — the `0x0118` formula-variable
///   record.
/// - [`RoundingFormat`](crate::model::RoundingFormat) — the second `0x00f8` numeric record's rounding
///   byte, encoded as `11 - decimalPlaces`.
/// - [`DayFormat`](crate::model::DayFormat) / [`MonthFormat`](crate::model::MonthFormat) /
///   [`YearFormat`](crate::model::YearFormat) / [`DateSystemDefaultType`](crate::model::DateSystemDefaultType) /
///   [`DayOfWeekFormat`](crate::model::DayOfWeekFormat) — the `0x00f2` date leaf; the `dayOfWeekType`
///   is leaf byte 4.
/// - [`NegativeFormat`](crate::model::NegativeFormat) / [`CurrencySymbolFormat`](crate::model::CurrencySymbolFormat)
///   — the second `0x00f8` numeric record (bytes 2 and 10).
/// - [`BooleanOutputType`](crate::model::BooleanOutputType) — the `0x00ee` boolean record, byte 0.
/// - [`ParameterDisplayType`](crate::model::ParameterDisplayType) / [`ParameterSortOrder`](crate::model::ParameterSortOrder)
///   — the `0x007a` parameter record (a byte just past the parameter-name string).
pub mod enums {}

/// Provenance for the report-object types ([`ReportObject`](crate::model::ReportObject) and kin). Each
/// object is opened by a type-specific `Contents` record; text/chart/cross-tab objects nest further
/// records.
///
/// - [`CrossTabObject`](crate::model::CrossTabObject) — opener `0xb8`, wrapped by `0xb9`.
/// - [`FieldObject`](crate::model::FieldObject) — its [`FieldRefKind`](crate::model::FieldRefKind) is
///   the type byte in the field-object opener record; a placed summary carries the `0x7e` summary
///   record's code.
/// - [`TextObject`](crate::model::TextObject) / [`Paragraph`](crate::model::Paragraph) /
///   [`TextRun`](crate::model::TextRun) — a paragraph opens with `0x00c0`; its runs are literal-text
///   `0x00c2` elements and embedded-reference `0x00c4` elements; a run's own font is a `0x08` record.
/// - [`PictureObject`](crate::model::PictureObject) — the bound field reference comes from the `0x00b1`
///   wrapper around the picture opener; OLE embedding is the `0xbd` `OleObjectItem` record.
/// - [`SubreportObject`](crate::model::SubreportObject) — `subreport_index` is the `0xa3` opener's leaf
///   bytes `[0..4]` (big-endian), naming the backing `Subdocument N` storage.
/// - [`ChartObject`](crate::model::ChartObject) / [`ChartDefinition`](crate::model::ChartDefinition) —
///   the `0x0121` `ChartDefinition2` record: a `[enum +0x4c][enum +0x50]` header then length-prefixed
///   strings (title `+0x54`, subtitle `+0x58`, …); the data-value label is the sibling `0x011f`
///   `ChartDataValue` record; the category period is the chart's `0xe5` grid-group record; legend flags
///   live in the styling struct (`+0x410` legend short, `+0x4a8` data-labels).
/// - [`CrossTabObject`](crate::model::CrossTabObject) dimensions — `0x00cb` `CrossTabDimensionField`
///   records (nested `0x00ce → 0x00cc → 0x00cb`); column levels nest under `0x00ce`, row levels under
///   `0x00d2`; measures are pre-layout `0x7e` summaries counted by the `0x00db` `CrossTabFieldGrid`
///   record.
pub mod objects {}

/// Provenance for the raw-record DOM ([`Node`](crate::model::Node) / [`Unknown`](crate::model::Unknown)).
/// [`Unknown`](crate::model::Unknown) already carries the raw `rtype`/`subtype` and decoded leaf
/// values — it *is* the raw substrate. [`Node::FieldDef`](crate::model::Node::FieldDef) is the
/// modelled field-definition record `0x73`.
pub mod dom {}
