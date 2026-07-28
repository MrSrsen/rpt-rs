//! The record-type registry — the numeric ↔ symbolic mapping for TSLV record types.
//!
//! The registry is flat (`u16` keyed); sub-documents reuse the same vocabulary. Any unmapped type is
//! still a first-class [`RecordTag`], just without a name.
//!
//! This table maps numbers to names and nothing else. A record's **byte layout** is documented at the
//! decoder that reads it and in `docs/07-block-catalog.md`; a name here does not imply the record's
//! contents are modelled. Comments below cover only what the name itself cannot say: how a record
//! nests, which stream disambiguates a shared number, and where a non-obvious one is decoded.
//!
//! Two conventions run through the whole format and explain most of the table:
//!
//! - **Open/close bracketing.** The stream is a stack of brackets, and a closing record is almost
//!   always `opener + 1` (`Area` `0x8a` / `AreaEnd` `0x8b`). Such pairs are named but carry no leaf
//!   content of their own.
//! - **Number reuse across streams.** A few numbers mean different things in `Contents` than in
//!   `QESession` or `ReportParametersStream`. Each decoder keys off the record type within its own
//!   stream and never consults this table, so a shared number is only ever cosmetic here. The
//!   collisions are called out at their entries.

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

    /// The symbolic name for this record type, if identified.
    pub fn name(self) -> Option<&'static str> {
        match self.0 {
            0xFFFF => Some("StreamHeader"),
            0x0064 => Some("ReportRoot"),
            0x0003 => Some("PrinterInfo"),
            0x0007 => Some("PaperSize"),
            0x0071 => Some("NamedValue"),
            0x0073 => Some("FieldDef"),
            0x0076 => Some("Formula"),
            // Wraps exactly one `0x0076` formula body, decoded by `data_def::raise_formulas`. Its
            // 4-byte leaf is a formula-cache slot populated only at runtime, never a field index.
            0x0077 => Some("FormulaFieldWrapper"),
            0x0078 => Some("ReportProperty"),
            // Secondary field definition streamed for summary/running-total "show value" bindings,
            // alongside the `0x007e` SummaryDef. Parents a `0x0071` NamedValue.
            0x0079 => Some("FieldDefinition2"),
            // Wraps exactly one `0x007e` summary / running-total / chart "show value" definition; the
            // leaf u32 is a summary-slot id correlating with sibling `0x0079` records. The raise layer
            // calls the chart-scoped occurrences `CHART_DATA`.
            0x007f => Some("SummaryFieldWrapper"),
            // Owns the report's field pool; parents the `0x006e` entry. Its own leaf is all-zero.
            0x006f => Some("FieldManager"),
            // The field-pool census, one per `0x006f` — redundant with the decoded field defs, which
            // is what makes it a useful cross-check (`data_def::raise_field_manager_census`).
            0x006e => Some("FieldManagerEntry"),
            0x0160 => Some("FieldDefinitionsHeader"),
            // Area-pair openers. The report body is a tree of nested header/footer *area pairs*, each
            // opened by one signature record; the kind is the record type, not the leaf. `PageAreaPair`
            // is top-level only — subreports have no page areas. `GroupAreaFormat` is the per-group
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
            // Parents the `0xec` ObjectBorder and carries the border's conditioned colour slots
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
            // each near the ReportRoot; `0x0000` is a leaf child of `ReportRoot`, the others are
            // top-level singletons. COLLISION: in `QESession` these numbers are QE-dialect markers.
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
            // field, whose leaf leads with the field reference decoded into
            // `BlobFieldObject.data_source`.
            0x00af => Some("PictureWrapper"),
            0x00b1 => Some("BlobFieldWrapper"),
            // One per static/OLE picture; its leaf's leading u32 is the 1-based OLE item ordinal
            // linking to the report's `Embedding N` storage.
            0x00bd => Some("OleObjectItem"),
            0x00ca => Some("ParagraphFormat"),
            // Leaf is a fixed header followed by a length-prefixed
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
            // 5B leaf = `[enum flag 1B][u32 BE raw value]`, a report-document scalar — NOT a subreport
            // field/param index. (`0x016a` is separately used as hex shorthand for a parameter index
            // inside a subreport-link head record: different bytes, not this record type.)
            0x016a => Some("ReportDocumentContainer"),
            // ── Designer/IDE state ────────────────────────────────────────────────────────────
            // The report designer's on-canvas editing state: ruler ticks, snap guidelines,
            // object-connection edges, edit history, interactive sorts, container references. Present
            // in every report whether or not it has subreports, and high-volume — the `0x0111` connect
            // edges and `0x010f` guideline collections dominate the unnamed-record census.
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
            // edge, whose leaf holds the source and destination layout-object node indices.
            0x0111 => Some("ObjectConnection"),
            0x0112 => Some("ObjectConnectionCollection"),
            // Crystal formulas can declare `Global`/`Shared` variables that persist with the report.
            // `FormulaVariableTable` heads the run with the count of persisted (non-Local) variables,
            // followed by that many `FormulaVariable` records and a `0x0117` terminator. Decoded by
            // `data_def::raise_formula_variables`.
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
            // around those bindings. See `docs/07-block-catalog.md` for the record flow.
            //
            // The placeholder object. Its 4B leaf is a default analytic render extent in twips; the
            // actual on-page placement is in the object's `0xfd`/`0xfe` format record like any other.
            0x00b4 => Some("ChartObject"),
            // Sits between the ChartObject and the `0xae` graphic base it draws through.
            0x00b3 => Some("ChartAnalyticObject"),
            0x0128 => Some("ChartDefinition"),
            // Leaf bytes 2 and 4 are small per-chart enum codes; byte 2 is the chart layout
            // (`ChartLayoutType`), byte 4 an unmapped chart-type/subtype selector.
            0x011c => Some("ChartAnalyticHeader"),
            // The labeled-value analytic variant: the leaf carries the chart's data-value description.
            // The binding it labels is the sibling summary/group records.
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
            // The v2 styling blob: type/subtype enums, the title and axis-title strings, then a
            // fixed-schema legend/gridline/font/colour region. Decoded by
            // `report_def::chart::parse_chart_definition2`.
            0x0121 => Some("ChartDefinition2"),
            // References a contained layout object BY NAME, with an ordinal.
            0x018d => Some("ContainerReference"),
            // ── Per-object format & geometry (decoded by `report_def::raise_report_definition`) ──
            0x00be => Some("ObjectPosition"), // Left/Top position (variable-width coords)
            0x00ec => Some("ObjectBorder"),   // border line styles + border/background colours
            0x00fc => Some("ObjectFormat"),   // object format flags (CanGrow/Suppress/…)
            0x00fd => Some("ObjectConditionFormat"), // object conditional-format formula slots
            0x00fe => Some("AreaSectionFormat"), // the area/section format block (byte0 = area kind)
            0x00ff => Some("SectionConditionFormat"), // section/area conditional-format formula slots
            0x0100 => Some("FontColor"),              // font colour record
            0x0101 => Some("FontConditionFormat"),    // font colour/style conditional-format slots
            // One Highlighting Expert condition — the no-formula way to colour/border a field by
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
            0x0043 => Some("FormatObject"),         // format-object record (FRObj)
            // ── Data definition / layout (`data_def`, `print_options`) ────────────────────────
            0x00e5 => Some("Group"),                  // the group definition
            0x007e => Some("SummaryFieldDefinition"), // summary / running-total field def
            0x0029 => Some("RecordSortField"),        // record/group sort field
            0x0080 => Some("RunningTotalReset"),      // running-total reset condition
            // Field-definition-family records. Each emits the shared FieldDefinition header, then a
            // subclass-specific tail — a name, count and field-reference array for `FieldDefinition3`
            // (related to `RunningTotalReset`); formula/expression strings and a data-interface string
            // array for `FieldDefinition4` (related to `FieldDef`). The exact subclass of each is not
            // pinned, so both are FAMILY-level names rather than decoded semantics.
            0x0081 => Some("FieldDefinition3"),
            0x014f => Some("FieldDefinition4"),
            0x007a => Some("ParameterRecord"), // parameter descriptor (0x7a) — drives ParameterField
            0x0031 => Some("CurrentValueRecord"), // parameter current-value record
            0x0061 => Some("SavedData"), // the saved-data batch descriptor (codec::saved_data)
            0x0066 => Some("PageSetup"), // page-setup record
            0x018e => Some("PaperRect"), // paper rectangle / margins
            0x0106 => Some("SubreportLink"), // a single subreport link (io::subreport_links)
            // ── Cross-tab / OLAP grid family ──────────────────────────────────────────────────
            // The object nests `0xb9`→`0xb8`→ObjectName, dimensions as `0xce`→`0xcc`→`0xcb{field ref}`,
            // and per-cell grid formats as `0x0143`/`0x0145`. Field-level decode lives in
            // `raise::report_def::crosstab`.
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
            // record's 4B leaf is the u32 member count, and each member is itself bracketed by the
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
            0x0129 => Some("ChartDefinitionEnd"), // 0x128+1
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
            // ReportParametersStream framing. Per-parameter shape:
            // `DataSourceParameterEntry(0x3b) · DataSourceParameterValue(0x30) ·
            // CurrentValueRecord(0x31) · FormulaFieldWrapper(0x77) · DataSourceParameterValueEnd(0x33)
            // · DataSourceParameterEntryEnd(0x3c)`, bracketed by the stream header/footer.
            // COLLISION: the FR* runtime objects reuse 0x30/0x3b/0x3c but are never serialised to a
            // `.rpt`, and 0x012f/0x0130 are rptdoc records in Contents; these names are for the
            // ReportParametersStream role.
            0x012f => Some("DataSourceParametersHeader"),
            0x0130 => Some("DataSourceParametersFooter"),
            0x003b => Some("DataSourceParameterEntry"),
            0x0030 => Some("DataSourceParameterValue"),
            0x0033 => Some("DataSourceParameterValueEnd"),
            0x003c => Some("DataSourceParameterEntryEnd"),
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

    /// True if this record type has been identified (has a name).
    pub fn is_known(self) -> bool {
        self.name().is_some()
    }
}

impl std::fmt::Display for RecordTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.name() {
            Some(name) => write!(f, "{name}({:#06x})", self.0),
            None => write!(f, "{:#06x}", self.0),
        }
    }
}
