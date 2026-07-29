//! The record-type registry — the numeric ↔ symbolic mapping for TSLV record types.
//!
//! **A record type number is per stream, not global**, so every lookup here takes the stream's
//! [`Dialect`] alongside the number. The report definition and the query-engine session both write a
//! `0x0003`, for unrelated records; answering from one vocabulary for a record read out of the other
//! names it after something it is not. Each dialect has its own table below, and a type unmapped in
//! its own is still a first-class [`RecordTag`], just without a name.
//!
//! This table maps numbers to names and nothing else. A record's **byte layout** is documented at the
//! decoder that reads it and in `docs/format/06-block-catalog.md`; a name here does not imply the record's
//! contents are modelled. Comments below cover only what the name itself cannot say: how a record
//! nests, which stream disambiguates a shared number, and where a non-obvious one is decoded.
//!
//! Two conventions run through the whole format and explain most of the table:
//!
//! - **Open/close bracketing.** The stream is a stack of brackets, and a closing record is almost
//!   always `opener + 1` (`Area` `0x8a` / `AreaEnd` `0x8b`). Such pairs are named but store no
//!   field bytes of their own.
//! - **Number reuse across streams.** A few numbers mean different things in `Contents` than in
//!   `QESession` or `ReportParametersStream`. Each decoder keys off the record type within its own
//!   stream, so a name here is only ever for display; the collisions are called out at their entries.

use crate::codec::Dialect;

/// A TSLV record type. Always carries the raw numeric type; a human name is attached for the
/// types we have identified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecordTag(pub u16);

impl RecordTag {
    /// The type-`0xffff` stream header record.
    pub const STREAM_HEADER: RecordTag = RecordTag(0xFFFF);

    /// The raw numeric record type.
    pub fn value(self) -> u16 {
        self.0
    }

    /// The symbolic name for this record type in `dialect`, if identified there.
    ///
    /// The dialect is the stream the record was read from ([`crate::raw::RecordStream::dialect`]).
    /// The same number is a different record in each, so a name is only meaningful once the stream
    /// is named too.
    pub fn name(self, dialect: Dialect) -> Option<&'static str> {
        match dialect {
            Dialect::Contents => self.contents_name(),
            Dialect::QeSession => self.qe_session_name(),
            Dialect::Catalog => self.catalog_name(),
            Dialect::ReportParameters => self.report_parameters_name(),
        }
    }

    /// True if this record type is identified in `dialect` (has a name there).
    pub fn is_known(self, dialect: Dialect) -> bool {
        self.name(dialect).is_some()
    }

    /// The record type a symbolic `name` identifies, and the vocabulary that names it — the inverse
    /// of [`name`](Self::name), matched case-insensitively.
    ///
    /// A caller that has a name and not a stream needs this: the name is the whole selector, and it
    /// identifies its own vocabulary, so `QeIndex` answers `0x0008` however the caller arrived at
    /// it. Asking here rather than per dialect is what keeps a vocabulary from being left out of the
    /// question — the search runs over [`Dialect::ALL`], which is declared with the variants and so
    /// cannot omit one.
    ///
    /// The registry is a `match` rather than a table, so the answer is found by asking every type
    /// number in every vocabulary. That is a few hundred thousand jump-table lookups for a one-shot
    /// selector, and it is the only form that cannot fall out of step with the names themselves: a
    /// reverse table would be a second statement of them.
    ///
    /// Where a name is used in more than one vocabulary it names the same record type in each (a
    /// stream header is `0xffff` everywhere), so the vocabulary returned is the first in
    /// `Dialect::ALL` that names it and the type is unambiguous.
    pub fn from_name(name: &str) -> Option<(RecordTag, Dialect)> {
        Dialect::ALL.iter().find_map(|&dialect| {
            (0u16..=0xffff)
                .map(RecordTag)
                .find(|tag| {
                    tag.name(dialect)
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
                .map(|tag| (tag, dialect))
        })
    }

    /// This record type's `Name(0x00nn)` label in `dialect`, or bare hex where it has no name.
    pub fn label(self, dialect: Dialect) -> String {
        match self.name(dialect) {
            Some(name) => format!("{name}({:#06x})", self.0),
            None => format!("{:#06x}", self.0),
        }
    }

    /// The query engine's own record vocabulary — the `QESession` stream and every subreport's own.
    ///
    /// The type numbers overlap the report definition's almost entirely and mean nothing in common
    /// with them. A number absent here is left unnamed rather than borrowed from another stream.
    fn qe_session_name(self) -> Option<&'static str> {
        match self.0 {
            // The session's object model: the session record is the stream's single root and holds
            // the connections, a connection holds its tables, a table its fields, indexes and
            // command parameters, and a link joins two tables by a field pair. All but the root are
            // read by the field table of the same name.
            0x0001 => Some("QeSession"),
            0x0002 => Some("QeConnection"),
            0x0003 => Some("QeTable"),
            0x0004 => Some("QeField"),
            0x0007 => Some("QeCommandParameter"),
            0x0008 => Some("QeIndex"),
            0x0009 => Some("QeLogonProperty"),
            0x000a => Some("QeTableLink"),
            // The last flat record of every session stream; the records above are nested children.
            0x001e => Some("DataSourceState"),
            0xFFFF => Some("StreamHeader"),
            _ => None,
        }
    }

    /// The saved-data catalog's vocabulary — the `DataSourceManager` stream.
    ///
    /// The catalog's tree is scanned rather than declared, so much of what it yields is field data
    /// that happened to frame as a record. Only the types the saved-data decoder itself reads are
    /// named; the rest stay hex, which is what a scan artifact should look like.
    fn catalog_name(self) -> Option<&'static str> {
        match self.0 {
            0x0007 => Some("SavedFieldContainer"),
            0x002d => Some("SavedRecordsStructure"),
            0x0040 => Some("SavedFieldDescriptor"),
            0x0041 => Some("SavedFieldHeader"),
            0x006d => Some("SavedBatchEntry"),
            0xFFFF => Some("StreamHeader"),
            _ => None,
        }
    }

    /// The parameter-values stream's vocabulary — the `ReportParametersStream`.
    ///
    /// Framed like the report definition and versioned in the same series, but numbered in a
    /// vocabulary of its own: `0x0030`, `0x0031` and `0x003b` are the records of a parameter's
    /// saved entry here and unrelated report-definition records there. A number absent here is left
    /// unnamed rather than borrowed from the report definition's table.
    ///
    /// Per-parameter shape: `DataSourceParameterEntry(0x3b) · DataSourceParameterValue(0x30) ·
    /// CurrentValueRecord(0x31) · FormulaFieldWrapper(0x77) · DataSourceParameterValueEnd(0x33) ·
    /// DataSourceParameterEntryEnd(0x3c)`, bracketed by the stream's header and footer. The
    /// `CurrentValueRecord` is written only for a parameter that has a saved value.
    fn report_parameters_name(self) -> Option<&'static str> {
        match self.0 {
            0xFFFF => Some("StreamHeader"),
            0x012f => Some("DataSourceParametersHeader"),
            0x0130 => Some("DataSourceParametersFooter"),
            0x003b => Some("DataSourceParameterEntry"),
            0x003c => Some("DataSourceParameterEntryEnd"),
            0x0030 => Some("DataSourceParameterValue"),
            0x0033 => Some("DataSourceParameterValueEnd"),
            0x0031 => Some("CurrentValueRecord"),
            // The report definition's own formula wrapper, streamed here by the archive machinery
            // the two streams share. A parameter entry carries an empty one: no formula body, only
            // the runtime cache words its `Contents` counterpart ends with.
            0x0077 => Some("FormulaFieldWrapper"),
            _ => None,
        }
    }

    /// The report definition's vocabulary — the `Contents` stream and every subreport's own.
    fn contents_name(self) -> Option<&'static str> {
        match self.0 {
            0xFFFF => Some("StreamHeader"),
            0x0064 => Some("ReportRoot"),
            0x0003 => Some("PrinterInfo"),
            0x0007 => Some("PaperSize"),
            // The field-definition base every kind of field is built on: the definition's name, the
            // type of the value it yields, that value's byte length, and an alternative name the
            // writer leaves empty when it matches the first.
            0x0071 => Some("NamedValue"),
            // Wraps exactly one `0x0071`, as `0x0077` wraps a formula body and `0x007f` a summary
            // definition.
            0x0072 => Some("NamedValueWrapper"),
            0x0073 => Some("FieldDef"),
            0x0076 => Some("Formula"),
            // Wraps exactly one `0x0076` formula body, decoded by `data_def::build_formulas`. Its
            // 4-byte field is a formula-cache slot populated only at runtime, never a field index.
            0x0077 => Some("FormulaFieldWrapper"),
            0x0078 => Some("ReportProperty"),
            // Secondary field definition streamed for summary/running-total "show value" bindings,
            // alongside the `0x007e` SummaryDef. Parents a `0x0071` NamedValue.
            0x0079 => Some("FieldDefinition2"),
            // Wraps exactly one `0x007e` summary / running-total / chart "show value" definition; its
            // u32 field is a summary-slot id correlating with sibling `0x0079` records. The decoders
            // calls the chart-scoped occurrences `CHART_DATA`.
            0x007f => Some("SummaryFieldWrapper"),
            // Owns the report's field pool; parents the `0x006e` entry. Its own field bytes are all zero.
            0x006f => Some("FieldManager"),
            // The field-pool census, one per `0x006f` — redundant with the decoded field defs, which
            // is what makes it a useful cross-check (`data_def::build_field_manager_census`).
            0x006e => Some("FieldManagerEntry"),
            // Area-pair openers. The report body is a tree of nested header/footer *area pairs*, each
            // opened by one signature record; the kind is the record type, not its bytes.
            // `PageAreaPair` is top-level only — subreports have no page areas. `GroupAreaFormat` is the per-group
            // pair and additionally carries the group's KeepTogether/RepeatHeader/VisibleGroups flags.
            // The Detail pair doubles as the group-list terminator.
            0x0082 => Some("ReportAreaPair"),
            0x0084 => Some("PageAreaPair"),
            0x0086 => Some("DetailAreaPair"),
            0x0088 => Some("GroupAreaFormat"),
            0x008a => Some("Area"),
            0x008c => Some("Section"),
            // Section bands: per-section containers parenting the `0x008c` Section. The record type
            // selects the band's area kind. `GroupHeaderBand`/`GroupFooterBand` occur only in reports
            // that have groups.
            0x008d => Some("ReportHeaderBand"),
            0x008f => Some("ReportFooterBand"),
            0x0091 => Some("PageHeaderBand"),
            0x0093 => Some("PageFooterBand"),
            0x0095 => Some("DetailBand"),
            0x0097 => Some("GroupHeaderBand"),
            0x0099 => Some("GroupFooterBand"),
            // SectionCode family, nested `SectionCode` → `SectionCodeHeaderFooter` →
            // `SectionCodeAreaType`. The area-type code uses the same encoding as the `0x00fe`
            // area-format byte 0.
            0x009b => Some("SectionCodeAreaType"),
            0x009c => Some("SectionCodeHeaderFooter"),
            0x009d => Some("SectionCode"),
            0x009e => Some("ObjectName"),
            0x009f => Some("FieldObject"),
            0x00c2 => Some("TextObject"),
            0x0008 => Some("Font"),
            // Parents the `0xec` ObjectBorder and carries the border's conditioned color slots
            // (`@Fore_Color` → BorderColor, `@Back_Color` → BackgroundColor), decoded as
            // `BorderCondition`.
            0x00ed => Some("ObjectAdornment"),
            // Zero-byte marker streamed once after each `ObjectName` and each page-setup (`0x0066`).
            0x0165 => Some("ObjectMarker"),
            // One length-prefixed key/value pair of save-time environment metadata per record, grouped
            // per save event. Decoded into `Report::save_metadata`.
            0x0178 => Some("SaveMetadata"),
            // Typed field-format family: each wrapper (odd) carries conditioned-value slots and
            // parents its value child (even), streamed after every `0x9f` field opener.
            0x00ee => Some("BooleanFieldFormat"),
            0x00ef => Some("BooleanFieldFormatWrapper"),
            0x00f0 => Some("CommonFieldFormat"),
            0x00f1 => Some("CommonFieldFormatWrapper"),
            0x00f2 => Some("DateFieldFormat"),
            0x00f3 => Some("DateFieldFormatWrapper"),
            0x00f4 => Some("DateTimeFieldFormat"),
            0x00f5 => Some("DateTimeFieldFormatWrapper"),
            0x00f6 => Some("TimeFieldFormat"),
            0x00f7 => Some("TimeFieldFormatWrapper"),
            0x00f8 => Some("NumericFieldFormat"),
            0x00f9 => Some("NumericFieldFormatWrapper"),
            0x00fa => Some("StringFieldFormat"),
            0x00fb => Some("StringFieldFormatWrapper"),
            // One `HierarchicalGroupingOptions` per specified/hierarchical group value, carrying the
            // value name and its defining condition-formula. The Top N limit and NotInTopBottomNName
            // live in the `0xe5` Group record, not here (see `data_def::decode_group_topn`).
            0x00e7 => Some("GroupOptions"),
            0x00e9 => Some("HierarchicalGroupingOptions"),
            // The report-document structural header (object-id ranges, counts, geometry bounds), once
            // each near the ReportRoot; `0x0000` nests under `ReportRoot` with no children, the
            // others are top-level singletons. COLLISION: in `QESession` these numbers are QE-dialect markers.
            0x0000 => Some("ReportDocument"),
            0x0001 => Some("ReportDocumentInfo"),
            0x0005 => Some("ReportDocumentFlags"),
            0x0009 => Some("ReportDocumentBounds"),
            // "Format with Multiple Columns" detail layout, decoded into `PrintOptions.multi_column`.
            0x006c => Some("MultiColumnFormat"),
            // Draw-object sub-records, each parenting a `0xa9` geometry opener.
            0x00aa => Some("LineDrawObject"),
            0x00ac => Some("BoxDrawObject"),
            // A picture object is opened by the `0xae` graphic base and wrapped by one of these two,
            // selected by the picture's source: `PictureWrapper` for a static/OLE-embedded image
            // (paired 1:1 with an `OleObjectItem`), `BlobFieldWrapper` for one bound to a database blob
            // field, which names that field and the stream its picture is cached in.
            0x00af => Some("PictureWrapper"),
            0x00b1 => Some("BlobFieldWrapper"),
            // One per static/OLE picture; its leading u32 field is the 1-based OLE item ordinal
            // linking to the report's `Embedding N` storage.
            0x00bd => Some("OleObjectItem"),
            0x00ca => Some("ParagraphFormat"),
            // Its field bytes are a fixed header followed by a length-prefixed
            // `<CrystalReports.PropertyBag …>` XML document: the report's data-connection property bag.
            0x015f => Some("DataInterface"),
            // The Contents-stream companion to the `PromptManager` stream (parameter prompt layout).
            0x016d => Some("PromptManagerRecord"),
            // Subreport-link framing. One collection per linked subreport object (`0xa3`): the
            // collection opens, writes its count child, closes, then the link items follow and a
            // `0x0105` terminator ends the run — `0xa3 · 0x0104{0x0103} · 0x0106×N`. The links
            // themselves are decoded from the `0x0106` records (`io::subreport_links`); the count is a
            // u16 BE equal to the number of those records.
            0x0104 => Some("SubreportLinkCollection"),
            0x0103 => Some("SubreportLinkCount"),
            // Where a report/subreport was imported from, for the designer's "re-import subreport when
            // opening" feature: a length-prefixed source `.rpt` path then two compound
            // `(Julian-day, time-fraction)` import timestamps separated by a re-import enum byte.
            0x0142 => Some("SubreportReimportInfo"),
            // 5B field block = `[enum flag 1B][u32 BE raw value]`, a report-document scalar — NOT a subreport
            // field/param index. (`0x016a` is separately used as hex shorthand for a parameter index
            // inside a subreport-link head record: different bytes, not this record type.)
            0x016a => Some("ReportDocumentContainer"),
            // ── Designer/IDE state ────────────────────────────────────────────────────────────
            // The report designer's on-canvas editing state: ruler ticks, snap guidelines,
            // object-connection edges, edit history, interactive sorts, container references. Present
            // in every report whether or not it has subreports, and high-volume — the `0x0111` connect
            // edges and `0x010f` guideline collections are the most numerous of the group.
            //
            // Rulers: `RulerEntry` (a tick/subdivision value) is a child of the two ruler containers,
            // the horizontal and vertical rulers (both parent `0x0107`; the H/V roles are not
            // distinguished).
            0x0107 => Some("RulerEntry"),
            0x0108 => Some("RulerDefinition"),
            0x010a => Some("RulerScale"),
            // Guidelines: `GuidelineEntry` (a twip canvas position plus flags) is a child of the two
            // guideline collections, the horizontal and vertical guideline sets.
            0x010c => Some("GuidelineEntry"),
            0x010d => Some("GuidelineList"),
            0x010f => Some("GuidelineCollection"),
            // The designer's object-connection graph: a collection parenting one `ObjectConnection` per
            // edge, whose field bytes hold the source and destination layout-object node indices.
            0x0111 => Some("ObjectConnection"),
            0x0112 => Some("ObjectConnectionCollection"),
            // Crystal formulas can declare `Global`/`Shared` variables that persist with the report.
            // `FormulaVariableTable` heads the run with the count of persisted (non-Local) variables,
            // followed by that many `FormulaVariable` records and a `0x0117` terminator. Decoded by
            // `data_def::build_formula_variables`.
            0x0116 => Some("FormulaVariableTable"),
            0x0118 => Some("FormulaVariable"),
            // `HistoryInfo` heads the designer's modification history with the count of `HistoryEntry`
            // records that follow.
            0x0179 => Some("HistoryEntry"),
            0x017b => Some("HistoryInfo"),
            // `InteractiveSort` is the manager opener; one `InteractiveSortEntry` per binding.
            0x0189 => Some("InteractiveSort"),
            0x018b => Some("InteractiveSortEntry"),
            // ── Chart / graph model ───────────────────────────────────────────────────────────
            // A chart's DATA bindings are NOT in these records: the on-change-of group is an ordinary
            // `0xe5` Group and the "show value" summary an ordinary `0x007f`→`0x007e` wrapper, both
            // following the chart object in the section. These carry the analytic layout and styling
            // around those bindings. See `docs/format/06-block-catalog.md` for the record flow.
            //
            // The placeholder object. Past its one child it carries the analytic's render extent as
            // a `TwipSize`; the actual on-page placement is in the object's `0xfd`/`0xfe` format
            // record like any other.
            0x00b4 => Some("ChartObject"),
            // Sits between the ChartObject and the `0xae` graphic base it draws through.
            0x00b3 => Some("ChartAnalyticObject"),
            // The chart's definition, and a pure container: it stores no field bytes of its own,
            // and what it holds is the `0x0121` styling record and the grid definition nested inside it. Closed by
            // `0x0129`. A chart object that carries no definition closes at `0x00b5` instead, which
            // is what makes the record optional rather than a missing one.
            0x0128 => Some("ChartDefinition"),
            // Bytes 2 and 4 are small per-chart enum codes; byte 2 is the chart layout
            // (`ChartLayoutType`), byte 4 an unmapped chart-type/subtype selector.
            0x011c => Some("ChartAnalyticHeader"),
            // The labeled-value analytic variant: its field bytes carry the chart's data-value
            // description. The binding it labels is the sibling summary/group records.
            0x011f => Some("ChartDataValue"),
            0x0120 => Some("ChartDataValueEnd"),
            // The grouped analytic variant: a fixed 28B block of axis counts/flags bracketing the
            // chart's summary/group bindings and any series records.
            0x0126 => Some("ChartDataLayout"),
            0x0127 => Some("ChartDataLayoutEnd"),
            // One pair per chart series/riser inside the `0x0126` data-layout; a single-series chart
            // has none.
            0x013f => Some("ChartDataSeries"),
            0x0140 => Some("ChartDataSeriesEnd"),
            // The chart definition's **style** member, and a second-generation class of its own
            // rather than a second version of the record above: type/subtype enums, the title and
            // axis-title strings, then a fixed-schema legend/gridline/font/color region. Decoded by
            // `report_def::chart::parse_chart_definition2`.
            0x0121 => Some("ChartDefinition2"),
            // References a contained layout object BY NAME, with an ordinal.
            0x018d => Some("ContainerReference"),
            // ── Per-object format & geometry (decoded by `report_def::build_report_definition`) ──
            0x00be => Some("ObjectPosition"), // Left/Top position (variable-width coords)
            0x00ec => Some("ObjectBorder"),   // border line styles + border/background colors
            0x00fc => Some("ObjectFormat"),   // object format flags (CanGrow/Suppress/…)
            0x00fd => Some("ObjectConditionFormat"), // object conditional-format formula slots
            0x00fe => Some("AreaSectionFormat"), // the area/section format block (byte0 = area kind)
            0x00ff => Some("SectionConditionFormat"), // section/area conditional-format formula slots
            0x0100 => Some("FontColor"),              // font color record
            0x0101 => Some("FontConditionFormat"),    // font color/style conditional-format slots
            // One Highlighting Expert condition — the no-formula way to color/border a field by
            // comparing its value to a bound. Emits the comparison enum, an optional value-endpoint
            // pair, an operator/format enum, and two numeric bounds.
            0x00bf => Some("HighlightCondition"),
            // ── Text objects ──────────────────────────────────────────────────────────────────
            0x00a5 => Some("TextObjectContainer"), // text object opener (parents 0xc2 TextObject content)
            0x00a7 => Some("TextObjectSubRecord"), // text object sub-record
            0x00c0 => Some("TextObjectFormat"),    // text object paragraph/format record
            0x00c4 => Some("TextEmbeddedField"),   // a field reference embedded in the text object
            // ── Object openers / field-object sub-records ─────────────────────────────────────
            0x00a1 => Some("FieldObjectSubRecord"), // field object sub-record
            0x00a3 => Some("SubreportObject"),      // subreport placement object
            0x00a9 => Some("DrawingObject"),        // line/box drawing object opener
            0x00ae => Some("PictureObject"),        // picture object base opener
            0x0166 => Some("FieldHeadingLink"),     // field-heading → field-object link
            0x0043 => Some("FormatObject"),         // format-object record
            // ── Data definition / layout (`data_def`, `print_options`) ────────────────────────
            0x00e5 => Some("Group"),                  // the group definition
            0x007e => Some("SummaryFieldDefinition"), // summary / running-total field def
            0x0029 => Some("RecordSortField"),        // record/group sort field
            // A running total: the two conditions that drive it, around the `0x007e` it contains.
            0x0080 => Some("RunningTotalField"),
            // A SQL Expression field: the SQL text and the fields it names, after its `0x0071`.
            0x0081 => Some("SqlExpressionField"),
            // A field-definition-family record: the shared FieldDefinition header, then
            // formula/expression strings and a data-interface string array. The subclass is not
            // pinned, so this is a FAMILY-level name rather than decoded semantics.
            0x014f => Some("FieldDefinition4"),
            0x007a => Some("ParameterRecord"), // parameter descriptor (0x7a) — drives ParameterField
            0x0061 => Some("SavedData"), // the saved-data batch descriptor (codec::saved_data)
            0x0066 => Some("PageSetup"), // page-setup record
            0x018e => Some("PaperRect"), // paper rectangle / margins
            // The document's report-wide option bag, in the report-definition tail: one per
            // `Contents` stream, all scalars, no nested record.
            0x0160 => Some("ReportOptions"),
            0x0106 => Some("SubreportLink"), // a single subreport link (io::subreport_links)
            // ── Cross-tab / OLAP grid family ──────────────────────────────────────────────────
            // The object nests `0xb9`→`0xb8`→ObjectName, dimensions as `0xce`→`0xcc`→`0xcb{field ref}`,
            // and per-cell grid formats as `0x0143`/`0x0145`. Field-level decode lives in
            // `build_model::report_def::crosstab`.
            0x00b8 => Some("CrossTabObject"),
            0x00b9 => Some("CrossTabObjectWrapper"),
            0x00cb => Some("CrossTabDimensionField"), // carries the dimension's {table.field} ref
            0x00cc => Some("CrossTabDimensionGroup"),
            0x00cd => Some("CrossTabRecordDimension"), // nests a CrossTabDimensionField under a CrossTabRecord (0xd2)
            0x00ce => Some("CrossTabDimension"),
            0x00d2 => Some("CrossTabRecord"),
            0x00d6 => Some("CrossTabGridObject"),
            0x00d7 => Some("CrossTabSummaryRecord"),
            0x00db => Some("CrossTabFieldGrid"),
            0x00dc => Some("CrossTabFieldGridEntry"),
            0x0143 => Some("CrossTabGridFormat"), // (opener)
            0x0145 => Some("CrossTabGridCellFormat"), // (per-cell, 11B)
            // Brackets a cross-tab's custom-group-member collection, one per cross-tab; the begin
            // record's 4B field is the u32 member count, and each member is itself bracketed by the
            // inner pair.
            0x017e => Some("CrossTabCustomMembersBegin"),
            0x017f => Some("CrossTabCustomMembersEnd"),
            0x0180 => Some("CrossTabCustomMemberBegin"),
            0x0181 => Some("CrossTabCustomMemberEnd"),
            // A chart analytic pair distinct from the `0x0128`/`0x011c`/`0x011f`/`0x0121` flow above.
            0x0122 => Some("ChartAnalyticRecord"),
            0x0123 => Some("ChartAnalyticRecordEnd"),
            // ── Bracket partners of the openers named above ───────────────────────────────────
            // Each object's format prologue is `FontConditionFormat(0x101) · Font(0x08) · 0x0102`, then
            // for value/field objects the typed-format block `0x00ea … typed-wrappers … 0x00eb`.
            0x0102 => Some("FontColorEnd"), // closes the FontConditionFormat(0x101) font block
            0x00ea => Some("FieldFormat"),  // opens the per-field typed-format block (1/field)
            0x00eb => Some("FieldFormatEnd"), // closes it after StringFieldFormatWrapper(0xfb)
            // Object body = `<opener> · pos(0xbe) · cond(0xfd) · adorn(0xed) · <format> · <End>`.
            0x00a0 => Some("FieldObjectEnd"), // 0x9f+1, terminates each field object
            0x00a2 => Some("FieldObjectDefinition"), // rare field-object def variant (crosstab/grid-embedded fields)
            0x00a4 => Some("SubreportObjectEnd"), // 0xa3+1, closes a subreport object (follows the 0x0105 link terminator)
            0x00a6 => Some("TextObjectEnd"), // terminates text objects and field-heading objects
            0x00a8 => Some("TextObjectDefinition"), // rare text-object def variant (crosstab/grid-embedded text)
            0x00ab => Some("LineDrawObjectEnd"),    // 0xaa+1
            0x00ad => Some("BoxDrawObjectEnd"),     // 0xac+1
            0x00b0 => Some("PictureWrapperEnd"),    // 0xaf+1
            0x00b2 => Some("BlobFieldWrapperEnd"),  // 0xb1+1
            // A text object holds paragraphs, each holding runs. A literal run is
            // `TextObject(0xc2) · <font block> · 0x00c3`; a field run is
            // `TextEmbeddedField(0xc4) · <font block> · <field-format block> · 0x00c5`; the paragraph
            // closes with `0x00c1`, the whole object with `0x00a6`.
            0x00c1 => Some("ParagraphEnd"), // 0xc0+1, closes a paragraph's run list
            0x00c3 => Some("TextRunEnd"),   // 0xc2+1, closes a literal text run
            0x00c5 => Some("TextFieldRunEnd"), // 0xc4+1, closes an embedded-field run
            // Nested area pairs, e.g.
            // `PageAreaPair[ Area[ …PageHeaderBand… ] 0x8b Area[ …PageFooterBand… ] 0x8b ] 0x85`.
            0x0083 => Some("ReportAreaPairEnd"),   // 0x82+1
            0x0085 => Some("PageAreaPairEnd"),     // 0x84+1
            0x0087 => Some("DetailAreaPairEnd"),   // 0x86+1
            0x0089 => Some("GroupAreaFormatEnd"),  // 0x88+1
            0x008b => Some("AreaEnd"),             // 0x8a+1
            0x008e => Some("ReportHeaderBandEnd"), // 0x8d+1
            0x0090 => Some("ReportFooterBandEnd"), // 0x8f+1
            0x0092 => Some("PageHeaderBandEnd"),   // 0x91+1
            0x0094 => Some("PageFooterBandEnd"),   // 0x93+1
            0x0096 => Some("DetailBandEnd"),       // 0x95+1
            0x0098 => Some("GroupHeaderBandEnd"),  // 0x97+1
            0x009a => Some("GroupFooterBandEnd"),  // 0x99+1
            0x00e6 => Some("GroupEnd"), // 0xe5+1, immediately follows the Group definition
            0x00e8 => Some("GroupOptionsRecord"), // secondary group-options record
            // Nested: `RulerDefinition[ GuidelineCollection[ ObjectConnection… ] 0x0110 ] 0x0109` and
            // `RulerScale[ GuidelineList[ ObjectConnection… ] 0x010e ] 0x010b`.
            0x0109 => Some("RulerDefinitionEnd"),     // 0x108+1
            0x010b => Some("RulerScaleEnd"),          // 0x10a+1
            0x010e => Some("GuidelineListEnd"),       // 0x10d+1
            0x0110 => Some("GuidelineCollectionEnd"), // 0x10f+1
            // `HistoryInfo(0x17b)[ HistoryEntry(0x179)·SaveMetadata×N·0x017a … ] 0x017c`.
            0x017a => Some("HistoryEntryEnd"), // 0x179+1, closes each entry and its SaveMetadata group
            0x017c => Some("HistoryInfoEnd"),  // 0x17b+1
            0x018a => Some("InteractiveSortEnd"), // 0x189+1
            0x018c => Some("InteractiveSortEntryEnd"), // 0x18b+1
            0x00b5 => Some("ChartObjectEnd"),  // 0xb4+1
            0x011d => Some("ChartAnalyticHeaderEnd"), // 0x11c+1
            0x0129 => Some("ChartDefinitionEnd"), // closes ChartDefinition(0x128)
            // e.g. `CrossTabGridFormat[ …cells… ]0x0144 · CrossTabDimension·0x00df·Group·0x00e0 …
            // ]0x00cf · CrossTabRecord …]0x00d3 · CrossTabSummaryRecord·CrossTabFieldGrid·…·0x00d8`.
            0x00cf => Some("CrossTabDimensionEnd"), // 0xce+1
            0x00d3 => Some("CrossTabRecordEnd"),    // 0xd2+1
            0x00d8 => Some("CrossTabSummaryRecordEnd"), // 0xd7+1, closes a summary/field-grid block
            0x00df => Some("CrossTabDataBinding"),  // opens a dimension/record group binding
            0x00e0 => Some("CrossTabDataBindingEnd"), // 0xdf+1, wraps the on-change Group
            0x00ba => Some("CrossTabObjectRecord"), // 1/crosstab, near the field-grid; role unpinned
            0x0144 => Some("CrossTabGridFormatEnd"), // 0x143+1
            0x0105 => Some("SubreportLinkCollectionEnd"), // 0x104+1
            0x0114 => Some("ReportFormatInterface"), // opens the tail block wrapping the formula-variable table
            0x0115 => Some("ReportFormatInterfaceEnd"), // closes it
            0x0117 => Some("FormulaVariableTableEnd"), // 0x116 terminator
            0x016b => Some("ReportDocumentContainerEnd"), // 0x16a+1
            0x0070 => Some("FieldManagerEnd"), // 0x6f+1, closes the field/formula/param/summary def block
            0x0067 => Some("ReportDefinitionTrailer"), // tail singleton before ReportFormatInterface
            0x0065 => Some("ReportRootEnd"),           // 0x64+1, the final Contents record
            0x018f => Some("PaperRectEnd"),            // 0x18e+1
            0x0044 => Some("FormatObjectEnd"),         // 0x43+1, precedes the saved-data batch
            0x0062 => Some("InstanceManager"), // saved-data instance manager, follows the SavedData batch
            // The last flat record of every QESession stream. The QE connection/table/field records are
            // nested children rather than flat records, so they never reach this table — which is why
            // the `0x0002`/`0x0004` names below are safe despite the collision.
            0x001e => Some("DataSourceState"),
            // Report-document header slots. COLLISION: in `QESession` these numbers are the nested QE
            // connection/field records (see `0x001e`).
            0x0002 => Some("DataSourceManagerHeader"),
            0x0004 => Some("ReportDocument5Header"),
            0x0006 => Some("ReportDocument5HeaderAlt"),
            // ── Feature families identified at the type level only ────────────────────────────
            // Byte layouts for these are not decoded, and their `opener`/`opener + 1` pairing is
            // assumed from the surrounding format rather than established. Treat the names as family
            // labels, not decoded semantics — they exist so a report using one of these features shows
            // an identified type instead of raw hex.
            //
            // Maps:
            0x00b6 => Some("MapObject"),
            0x00b7 => Some("MapObjectData"),
            0x0119 => Some("MapDefinition"),
            0x011a => Some("MapDefinitionData"),
            0x011b => Some("MapDefinitionEnd"),
            0x012a => Some("MapLayerDefinition"),
            0x012b => Some("MapLayerData"),
            0x012c => Some("MapLayerStyle"),
            0x012d => Some("MapLayerBinding"),
            0x012e => Some("MapDefinitionTrailer"),
            // OLAP grid:
            0x00d0 => Some("OlapGridRow"),
            0x00d1 => Some("OlapGridRowEnd"),
            0x00d4 => Some("OlapGridColumn"),
            0x00d5 => Some("OlapGridColumnEnd"),
            0x00d9 => Some("OlapGridData"),
            0x00da => Some("OlapGridDataEnd"),
            0x00dd => Some("OlapGridObject"),
            0x00de => Some("OlapGridObjectEnd"),
            0x0161 => Some("OlapGridDefinition"),
            0x0162 => Some("OlapGridDefinitionEnd"),
            0x0163 => Some("OlapDimensionSelectInfo"),
            0x0164 => Some("OlapDimensionSelectInfoEnd"),
            0x0146 => Some("OlapGridSectionHeader"),
            0x0147 => Some("OlapGridSectionHeaderData"),
            0x0148 => Some("OlapGridSectionHeaderStyle"),
            0x0149 => Some("OlapGridSectionHeaderBinding"),
            0x014d => Some("OlapGridSectionHeaderExtra"),
            0x014e => Some("OlapGridSectionHeaderEnd"),
            // Dimension selection / query condition:
            0x00e1 => Some("DimensionSelect"),
            0x00e2 => Some("DimensionSelectField"),
            0x00e3 => Some("DimensionSelectEnd"),
            0x00e4 => Some("QueryDimensionCondition"),
            0x016c => Some("QueryDimensionConditionData"),
            // Alerts:
            0x0150 => Some("AlertCondition"),
            // XML / XSLT export defs:
            0x0151 => Some("XmlDefinition"),
            0x0152 => Some("XmlDefinitionData"),
            0x0153 => Some("XmlDefinitionEnd"),
            0x0186 => Some("XsltDefinition"),
            0x0187 => Some("XsltDefinitionData"),
            0x0188 => Some("XsltDefinitionEnd"),
            // Flash / Xcelsius objects:
            0x0182 => Some("FlashObject"),
            0x0183 => Some("FlashObjectData"),
            0x017d => Some("FlashDataDescriptor"),
            0x0184 => Some("FlashDataDescriptorField"),
            0x0185 => Some("FlashDataDescriptorEnd"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field_table::tables;

    /// A number is named per stream. `0x0008` is the query engine's index record and the report
    /// definition's font; naming either from the other's vocabulary describes the wrong record.
    #[test]
    fn a_shared_number_is_named_per_dialect() {
        for (rtype, contents, qe) in [
            (0x0008u16, "Font", "QeIndex"),
            (0x0003, "PrinterInfo", "QeTable"),
            (0x0004, "ReportDocument5Header", "QeField"),
        ] {
            let tag = RecordTag(rtype);
            assert_eq!(tag.name(Dialect::Contents), Some(contents));
            assert_eq!(tag.name(Dialect::QeSession), Some(qe));
        }
    }

    /// The parameter-values stream's numbers are its own. Answered from the report definition's
    /// table they name a `Contents` record after a record that stream does not hold, and — since a
    /// type is identified exactly when it has a name — count it as identified in its coverage.
    #[test]
    fn a_parameter_values_number_is_not_named_in_the_report_definition() {
        for (rtype, name) in [
            (0x0031u16, "CurrentValueRecord"),
            (0x0030, "DataSourceParameterValue"),
            (0x0033, "DataSourceParameterValueEnd"),
            (0x003b, "DataSourceParameterEntry"),
            (0x003c, "DataSourceParameterEntryEnd"),
            (0x012f, "DataSourceParametersHeader"),
            (0x0130, "DataSourceParametersFooter"),
        ] {
            let tag = RecordTag(rtype);
            assert_eq!(tag.name(Dialect::ReportParameters), Some(name));
            assert_eq!(tag.name(Dialect::Contents), None);
            assert!(!tag.is_known(Dialect::Contents));
        }
    }

    /// A name identifies its own vocabulary, so the lookup takes one and needs no stream. Each name
    /// below is exclusive to one vocabulary: every one of them resolving is what says the search
    /// reaches them all, which a hand-written list of vocabularies is what usually fails to do.
    #[test]
    fn a_name_resolves_in_whichever_vocabulary_holds_it() {
        for (name, rtype, dialect) in [
            ("Font", 0x0008u16, Dialect::Contents),
            ("QeIndex", 0x0008, Dialect::QeSession),
            ("qetable", 0x0003, Dialect::QeSession),
            ("SavedBatchEntry", 0x006d, Dialect::Catalog),
            ("CurrentValueRecord", 0x0031, Dialect::ReportParameters),
            (
                "datasourceparameterentry",
                0x003b,
                Dialect::ReportParameters,
            ),
        ] {
            assert_eq!(
                RecordTag::from_name(name),
                Some((RecordTag(rtype), dialect)),
                "{name}"
            );
        }
        assert_eq!(RecordTag::from_name("NotARecordName"), None);
    }

    /// No name names two record types. [`RecordTag::from_name`] answers with the first vocabulary
    /// that holds the name, which is only safe while that is true — a name reused for a different
    /// number in another vocabulary would resolve to whichever happened to be searched first.
    #[test]
    fn a_name_names_one_record_type() {
        let mut seen: std::collections::BTreeMap<&'static str, (u16, Dialect)> = Default::default();
        for &dialect in Dialect::ALL {
            for rtype in 0u16..=0xffff {
                let Some(name) = RecordTag(rtype).name(dialect) else {
                    continue;
                };
                if let Some(&(prev, prev_dialect)) = seen.get(name) {
                    assert_eq!(
                        prev, rtype,
                        "{name} is {prev:#06x} in {prev_dialect:?} and {rtype:#06x} in {dialect:?}"
                    );
                } else {
                    seen.insert(name, (rtype, dialect));
                }
            }
        }
        assert!(seen.len() > 200, "only {} name(s) read", seen.len());
    }

    /// The catalog's tree is scanned, so a number it yields is as likely to be field data as a
    /// record. Naming only what its decoder reads keeps the rest looking like what it is.
    #[test]
    fn the_catalog_names_only_what_it_decodes() {
        assert_eq!(
            RecordTag(0x0041).name(Dialect::Catalog),
            Some("SavedFieldHeader")
        );
        assert_eq!(RecordTag(0x00e5).name(Dialect::Contents), Some("Group"));
        assert_eq!(RecordTag(0x00e5).name(Dialect::Catalog), None);
    }

    /// The one record type the corpus contains that the registry does not name, as
    /// `(dialect, type)`. The list is the claim: naming coverage is complete apart from these.
    const UNNAMED_IN_CORPUS: &[(Dialect, u16)] = &[(Dialect::ReportParameters, 0x0032)];

    /// Naming coverage of the record streams, stated as an exhaustive list of what is *not* named.
    ///
    /// A type left unnamed is a record whose very identity is unknown, so the useful claim is the
    /// negative one — and only a sweep can hold it, since the registry cannot see which of its
    /// numbers a report actually uses. Stating the exception list rather than a record total keeps
    /// the claim stable as fixtures come and go: a report that adds records changes nothing, and a
    /// report that introduces an unidentified type is the regression this exists to catch.
    ///
    /// The catalog dialect is out of scope: it is a scanned tree read through the saved-data path,
    /// where an unnamed number is as likely to be field data as a record.
    #[test]
    fn the_corpus_uses_no_unnamed_record_type_but_the_listed_ones() {
        let mut walked = 0usize;
        let mut observed: Vec<(Dialect, u16, String)> = Vec::new();
        crate::field_table::corpus::for_each_record(|dialect, node, _, path| {
            if dialect == Dialect::Catalog {
                return;
            }
            walked += 1;
            if !RecordTag(node.rtype).is_known(dialect) {
                let file = path.file_name().unwrap_or_default().to_string_lossy();
                observed.push((dialect, node.rtype, file.into_owned()));
            }
        });

        let unexpected: Vec<_> = observed
            .iter()
            .filter(|(d, t, _)| !UNNAMED_IN_CORPUS.contains(&(*d, *t)))
            .collect();
        assert!(
            unexpected.is_empty(),
            "{} record(s) of a type the registry does not name: {:?}",
            unexpected.len(),
            &unexpected[..unexpected.len().min(10)]
        );

        // A sweep that walked nothing proves nothing, and this one silently skips a report it
        // cannot open. The floor sits well under the corpus so adding or removing fixtures does not
        // move it, and well above what any single report contributes.
        assert!(
            walked > 20_000,
            "only {walked} record(s) swept — the corpus walk found nothing to measure"
        );

        // An exception that no longer occurs is an excuse waiting to cover an unrelated record, so
        // the list must earn each of its entries every run.
        for &(dialect, rtype) in UNNAMED_IN_CORPUS {
            assert!(
                observed.iter().any(|&(d, t, _)| (d, t) == (dialect, rtype)),
                "{rtype:#06x} no longer occurs unnamed in {dialect:?} — drop it from the list"
            );
        }
    }

    /// The registry and the field tables name the same record types, so a table added under one
    /// name and looked up under another cannot pass unnoticed.
    #[test]
    fn every_table_is_named_the_same_here() {
        for dialect in [
            Dialect::QeSession,
            Dialect::Catalog,
            Dialect::ReportParameters,
        ] {
            for table in tables::set(dialect) {
                assert_eq!(
                    RecordTag(table.rtype).name(dialect),
                    Some(table.name),
                    "table {} and the registry disagree on {:#06x} in {dialect:?}",
                    table.name,
                    table.rtype
                );
            }
        }
    }
}
