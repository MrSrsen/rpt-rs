//! The record-type registry — the numeric ↔ symbolic mapping for TSLV record types.
//!
//! The registry is flat (`u16` keyed); sub-documents reuse the same vocabulary. Any unmapped
//! type is still a first-class [`RecordTag`], just without a name.

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
            // Formula-body wrapper. One `0x0077` opens each `0x0076` formula body, with exactly one
            // `0x0076` child and no others. The 4-byte leaf is always zero (a reserved
            // formula-cache/handle slot, populated only at runtime) — it is *not* a field index. The
            // wrapped `0x0076` is decoded by `data_def::raise_formulas`; the wrapper itself is
            // structural and carries nothing extra, so it is named for coverage only.
            0x0077 => Some("FormulaFieldWrapper"),
            0x0078 => Some("ReportProperty"),
            // Secondary field definition. Streamed for
            // summary/running-total "show value" bindings (the neighbour of the `0x007e` SummaryDef
            // decoded elsewhere); parents a `0x0071` NamedValue. Structural-only.
            0x0079 => Some("FieldDefinition2"),
            // Summary-field wrapper. One `0x007f` opens each `0x007e` SummaryDef/running-total/chart
            // "show value" def, with exactly one `0x007e` child. The leaf u32 is a summary-slot id
            // (correlates with the id embedded in sibling `0x0079` records). The wrapped `0x007e` is
            // decoded by the summary/RT/chart raises; the wrapper is named for coverage. (Alias of
            // the `CHART_DATA` const in the raise layer, which only cares about the chart-scoped
            // occurrences.)
            0x007f => Some("SummaryFieldWrapper"),
            // Field-manager collection. Parents the `0x006e` FieldManagerEntry. The manager owns the
            // report's field pool; its own leaf is all-zero. Structural-only.
            0x006f => Some("FieldManager"),
            // Field-manager entry (20B leaf, one per `0x006f`). The
            // leaf is a field-pool census: bytes[0..4] u32 BE = number of `0x0073` database field
            // defs, bytes[4..6] u16 BE = formula-body count less three built-in formulas, bytes[6..8]
            // u16 BE = 18 (a constant kind/version marker), and the remaining u16s are further
            // per-kind counts (group / parameter / summary). Redundant with the already-decoded field
            // defs, so named for coverage.
            0x006e => Some("FieldManagerEntry"),
            // Field-definitions block header. A fixed-layout header of field-pool counts/flags
            // that precedes the per-field records. Structural-only.
            0x0160 => Some("FieldDefinitionsHeader"),
            // Area-pair openers: the report body is a tree of nested
            // header/footer *area pairs*, each opened by one signature record carrying the constant
            // `0x395107` tag. Kind is encoded by the record type, not the leaf. `PageAreaPair`
            // (0x84) is top-level only — subreports have no page areas; the others appear in every
            // Contents stream. `GroupAreaFormat` (0x88) is the per-group pair and additionally
            // carries the group's KeepTogether/RepeatHeader/VisibleGroups flags. The
            // Detail pair (0x86) doubles as the group-list terminator.
            0x0082 => Some("ReportAreaPair"),
            0x0084 => Some("PageAreaPair"),
            0x0086 => Some("DetailAreaPair"),
            0x0088 => Some("GroupAreaFormat"),
            0x008a => Some("Area"),
            0x008c => Some("Section"),
            // Section bands: a per-section container that parents the
            // `0x008c` Section record. The record type selects the band's area kind — confirmed
            // across the corpus by joining each band to its section's decoded `Kind`:
            // 0x8d=ReportHeader, 0x8f=ReportFooter, 0x91=PageHeader, 0x93=PageFooter, 0x95=Detail,
            // 0x97=GroupHeader, 0x99=GroupFooter. 0x97/0x99 occur only in reports that have groups,
            // matching the kind mapping.
            0x008d => Some("ReportHeaderBand"),
            0x008f => Some("ReportFooterBand"),
            0x0091 => Some("PageHeaderBand"),
            0x0093 => Some("PageFooterBand"),
            0x0095 => Some("DetailBand"),
            0x0097 => Some("GroupHeaderBand"),
            0x0099 => Some("GroupFooterBand"),
            // SectionCode family.
            // `SectionCodeAreaType` (0x9b, 3B): leaf[0] = area-type code — 01=Page, 02=Report,
            // 03=Group, 04=Detail (same encoding as the `0x00fe` area-format byte 0), leaf[2] =
            // group level for group areas. `SectionCodeHeaderFooter` (0x9c, 2B): leaf[1] = the
            // header/footer discriminator, parents a `0x9b`. `SectionCode` (0x9d, 2B): the
            // enclosing container, parents a `0x9c`.
            0x009b => Some("SectionCodeAreaType"),
            0x009c => Some("SectionCodeHeaderFooter"),
            0x009d => Some("SectionCode"),
            0x009e => Some("ObjectName"),
            0x009f => Some("FieldObject"),
            0x00c2 => Some("TextObject"),
            0x0008 => Some("Font"),
            // Per-object adornment wrapper: parents the `0xec` ObjectBorder and
            // carries the border's conditioned colour slots (`@Fore_Color` → BorderColor,
            // `@Back_Color` → BackgroundColor). Decoded by the report-def raise as `BorderCondition`.
            0x00ed => Some("ObjectAdornment"),
            // Zero-byte structural marker, streamed once after each `ObjectName`
            // (`0x009e`) and each page-setup (`0x0066`). Carries no leaf content.
            0x0165 => Some("ObjectMarker"),
            // Save-time environment metadata: one length-prefixed key/value
            // pair per record (`Saved Date`, `Build Version`, `Print Engine`, `OS`, `Architecture`),
            // grouped per save event. Decoded into `Report::save_metadata`; not exported.
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
            // Group-options records.
            // `GroupOptions` (0xe7, 3B) is a small group-option enum. `HierarchicalGroupingOptions`
            // (0xe9, ~47–64B) carries a length-prefixed group-value name plus its length-prefixed
            // defining condition-formula (one record per specified/hierarchical group value, e.g.
            // "X" → `{Command.some_field} = "X"`). Neither is exported
            // (structural-only). NOTE: the Top N limit and NotInTopBottomNName live in the `0xe5`
            // Group record, not here (see `data_def::decode_group_topn`).
            0x00e7 => Some("GroupOptions"),
            0x00e9 => Some("HierarchicalGroupingOptions"),
            // Report-document block. These low-numbered codes carry the
            // report-document's structural header (object-id ranges, counts, geometry bounds). In
            // `Contents` they appear once each near the ReportRoot; `0x0000` specifically is a leaf
            // child of `ReportRoot` (`0x0064`) while the others are top-level singletons.
            // NAMESPACE COLLISION: in the `QESession` stream `0x0000`/`0x0001`/`0x0009` are
            // QE-dialect markers, not these report-document blocks — the numeric type is shared but
            // the meaning differs by stream. The QE decode keys off record type directly and does not
            // consult this name table, so the shared name is cosmetic there.
            0x0000 => Some("ReportDocument"),
            0x0001 => Some("ReportDocumentInfo"),
            0x0005 => Some("ReportDocumentFlags"),
            0x0009 => Some("ReportDocumentBounds"),
            // "Format with Multiple Columns" detail layout. Decoded into `PrintOptions.multi_column` by
            // `raise::print_options` (column width, inter-column gaps, and flow direction; a zero
            // column width means a single column). Render-only — internal (not exported).
            0x006c => Some("MultiColumnFormat"),
            // Draw-object sub-records. Each parents a `0xa9`
            // geometry opener (Right/Bottom coords). `0x00aa` is the Line draw object, `0x00ac` the
            // Box draw object. Structural-only.
            0x00aa => Some("LineDrawObject"),
            0x00ac => Some("BoxDrawObject"),
            // Graphic / picture object records. A picture object is opened
            // by the base graphic sub-record `0xae` (OpenPicture in the raise layer), wrapped by one
            // of two outer records selected by the picture's source:
            // `PictureWrapper` (0xaf) — a static image / OLE-embedded picture.
            // Its 4B leaf is a reserved graphic-type/handle slot (all-zero across
            // the corpus). One per non-blob picture; paired 1:1 with an `OleObjectItem` (0xbd).
            // `BlobFieldWrapper` (0xb1) — a picture bound to a database blob
            // field. Its leaf leads with the length-prefixed bound field reference
            // (`{table.field}`) followed by an id/extent trailer; the field reference is decoded
            // by the report-def raise (`RdRecord::BlobFieldRef`) into `BlobFieldObject.data_source`
            // and counted toward `Field.UseCount`. Private-corpus-only.
            // Neither is exported beyond what the picture opener already drives.
            0x00af => Some("PictureWrapper"),
            0x00b1 => Some("BlobFieldWrapper"),
            // Embedded OLE item detail.
            // One per static/OLE `PictureWrapper` picture. Leaf `[0..4]` u32 BE = the 1-based OLE item
            // ordinal (links to the report's `Embedding N` storage, emitted separately), `[4..8]` = 0,
            // `[8..12]` = `00 01 00 00` constant marker. Structural-only.
            0x00bd => Some("OleObjectItem"),
            // Paragraph / tab-stop sub-record. Very rare (one corpus
            // occurrence). Structural-only.
            0x00ca => Some("ParagraphFormat"),
            // Data-interface block. Its leaf is a
            // fixed header followed by a length-prefixed `<CrystalReports.PropertyBag …>` XML
            // document (the report's data-connection property bag). Structural-only.
            0x015f => Some("DataInterface"),
            // PromptManager record — the
            // Contents-stream companion to the `PromptManager` stream (parameter prompt layout as a
            // set of u32 BE fields). Structural-only.
            0x016d => Some("PromptManagerRecord"),
            // Subreport-link collection wrapper. One per subreport object (`0xa3`) that has
            // links: it opens `0x0104` (empty leaf), writes the `0x0103` count child, closes, then
            // emits the `0x0106` link items, then an empty `0x0105` terminator. So the on-disk shape
            // around each linked subreport is `0xa3 · 0x0104{0x0103} · 0x0106×N`. Structural — the
            // links themselves are decoded from the `0x0106` records (see `io::subreport_links`);
            // this wrapper carries nothing beyond the count.
            0x0104 => Some("SubreportLinkCollection"),
            // Subreport-link count. The 2B leaf
            // is `store((ushort) CSObArray::size(linkArray))` — a u16 BE count of the link items,
            // i.e. the number of `0x0106` records that follow, always equal to the count of following
            // `0x0106` records. Redundant with the decoded links, so named for coverage.
            0x0103 => Some("SubreportLinkCount"),
            // Subreport re-import descriptor. Records where a
            // report/subreport was imported from, for the designer's "re-import subreport when
            // opening" feature. Leaf layout:
            // `[u32 BE L][path: L bytes][ts1a u32][ts1b u32][enum flag 1B][ts2a u32][ts2b u32]`
            // — an ATL length-prefixed source `.rpt` path (`L==1`/single NUL when none), then a
            // fixed 17-byte trailer of two compound `(JDN, time-fraction)` import timestamps
            // separated by a 1-byte re-import enum. NOT exported (SubreportController exposes no
            // re-import accessor); structural.
            0x0142 => Some("SubreportReimportInfo"),
            // Report-document container record. 5B leaf = `[enum flag 1B][u32 BE raw value]` written from a single
            // report-document field: the writer stores a clamped enum
            // (`-1`, `0`, or `0xc9`=201) followed by the raw int. Across the corpus the raw value is
            // 0 or 999 (constant per dataset), and the enum resolves to 0 — it is a report-document
            // scalar, NOT a subreport field/param index. (`0x016a` is also used elsewhere as hex
            // shorthand for a parameter *index* value stored in a subreport-link head record — a
            // different set of bytes, not record type 0x016a.)
            // Structural.
            0x016a => Some("ReportDocumentContainer"),
            // ── Designer/IDE state (internal; no SDK read API) ─────────────────────────────
            // These carry the report designer's on-canvas editing state — ruler ticks, snap
            // guidelines, object-connection edges, edit history, interactive-sort bindings and
            // container references. None are exported (the SDK exposes no reader
            // for them); they are named here for record-type coverage only. High volume — the
            // `0x0111` connect edges and `0x010f` guideline collections dominate the "unknown"
            // record census.
            //
            // This `0x0107–0x0112` cluster is per-report designer state, present in every report
            // regardless of whether it has subreports (`0x010c`'s u32 is a twip COORDINATE — a
            // guideline position on the design surface; `0x0111`'s two leading u16s are small
            // layout-object node indices with a `0x0002` kind and an all-`0xff` null-anchor trailer —
            // an object-connection edge). It is unrelated to `Field.UseCount`, which is the live
            // add/release reference count the engine maintains, reconstructed in the
            // derived analytics.
            //
            // Rulers: the design-surface ruler definitions.
            // `RulerEntry` (0x0107, 2B u16 tick/subdivision value) is a child of the two ruler
            // containers `RulerDefinition` (0x0108, empty) and `RulerScale` (0x010a, empty) — the
            // horizontal and vertical rulers (exact H/V role not distinguished; both parent 0x0107).
            0x0107 => Some("RulerEntry"),
            0x0108 => Some("RulerDefinition"),
            0x010a => Some("RulerScale"),
            // Guidelines: the designer's snap guidelines.
            // `GuidelineEntry` (0x010c, 6B `[u32 BE position-twips][u16 flags]`) is a child of the
            // two guideline collections `GuidelineList` (0x010d, 8B all-zero header) and
            // `GuidelineCollection` (0x010f, empty; the high-volume one, ~4304 corpus-wide) — the
            // horizontal and vertical guideline sets. Positions are twip coordinates on the canvas.
            0x010c => Some("GuidelineEntry"),
            0x010d => Some("GuidelineList"),
            0x010f => Some("GuidelineCollection"),
            // Object connections: the designer's object-connection graph — a
            // collection (`ObjectConnectionCollection`, 0x0112, empty top-level) parenting one
            // `ObjectConnection` (0x0111, 22B) per edge. The 22B leaf is
            // `[u16 src-node][u16 dst-node][8×00][u16 kind=0x0002][8×ff null-anchor]`
            // where src/dst are small layout-object node indices (0–0x14). Present in every report
            // regardless of subreports. Highest-volume unknown record.
            0x0111 => Some("ObjectConnection"),
            0x0112 => Some("ObjectConnectionCollection"),
            // Formula-language variable declarations. Crystal
            // formulas can declare `Global`/`Shared` variables that persist with the report; these
            // records carry that table. `FormulaVariableTable` (0x0116, 2B u16 BE, 1/report) is the
            // header — its value is the count of persisted (non-Local) variables. It is followed by
            // that many `FormulaVariable` (0x0118) records, each `[u32 BE namelen incl NUL][name+NUL]
            // [type][scope]`, then a 0x0117 empty terminator. `type` = the variable's declared FL
            // result kind (1=Number … 7=String); `scope` = FLScope (0=Shared, 1=Global,
            // 2=Local, Local asserted-absent). Decoded by `data_def::raise_formula_variables`;
            // structural (the SDK exposes no typed accessor, so not exported).
            0x0116 => Some("FormulaVariableTable"),
            0x0118 => Some("FormulaVariable"),
            // Report edit history: `HistoryInfo` (0x017b, 4B u32 BE, 1/report)
            // is the history header whose value is the number of `HistoryEntry` (0x0179, 4B u32 BE)
            // records that follow. Designer modification-history state; not exported.
            0x0179 => Some("HistoryEntry"),
            0x017b => Some("HistoryInfo"),
            // Interactive sort: the interactive-sort manager. `InteractiveSort`
            // (0x0189, empty, 1/report) is the manager opener; `InteractiveSortEntry` (0x018b, 4B
            // u32 BE) is one per interactive-sort binding. Designer/runtime UI state; not on the XML
            // surface (the SDK exposes interactive sorts only via the runtime report object).
            0x0189 => Some("InteractiveSort"),
            0x018b => Some("InteractiveSortEntry"),
            // Container reference: a
            // reference from a container to a contained layout object BY NAME. Leaf layout is
            // `[u32 BE L][name: L bytes incl. trailing NUL][u16 ordinal]`
            // (e.g. "Text35"→ordinal 4, "Text34"→3, "Text2"→6). It names layout objects, confirming
            // the designer-state (not field-dependency) reading of this whole cluster. Structural.
            // ── Chart / graph model (not exported) ───────────────────
            // A chart ("graph") object and its analytic data model. Co-occur in the 21 chart-bearing
            // corpus reports (private + worrall; 48 charts). Not exported,
            // and the SDK's ChartObject.ChartDefinition/.ChartStyle member surface is undocumented, so
            // none of these are exported — named here for record-type coverage only. The
            // chart's actual DATA bindings are NOT carried by these records: the on-change-of group is
            // the ordinary `0xe5` Group record and the "show value" summary is the ordinary
            // `0x007f`→`0x007e` SummaryFieldWrapper that immediately follow the chart object in the
            // section. These records carry the analytic layout + styling around those bindings.
            //
            // On-disk shape of a chart, in section order (all flat siblings except the `0xb4` sub-tree):
            // 0xb4 ChartObject [4B] -- the placeholder object
            // └ 0xb3 ChartAnalyticObject (empty)
            // └ 0xae graphic base → 0x9e ObjectName ("GraphN") (chart draws as a picture)
            // <standard object format: 0xbe, 0xfd/0xfc, 0xed, 0x0009>
            // 0x0128 ChartDefinition (empty) -- chart-definition opener
            // 0x011c ChartAnalyticHeader [5B] -- analytic header
            // <analytic data section — ONE of two variants, bracketed open/close:>
            // 0x011f ChartDataValue […] · 0x0120 ChartDataValueEnd (labeled-value variant)
            // or
            // 0x0126 ChartDataLayout[28B] · <summary/group + 0x013f/0x0140 series> · 0x0127 End
            // 0x0121 ChartDefinition2 [~426–466B] -- v2 styling blob
            //
            // ChartObject. 4B leaf = two u16 BE =
            // the chart's default analytic render extent in twips (constant `05 a0 05 a0` = 1440×1440 =
            // 1"×1" across the whole corpus); the actual on-page placement/size is in the object's
            // `0xfd/0xfe` format record like any other object.
            0x00b4 => Some("ChartObject"),
            // Chart analytic-object wrapper. Sits between the ChartObject and the graphic base (`0xae`) it draws through.
            0x00b3 => Some("ChartAnalyticObject"),
            // Chart-definition opener. Opens
            // the chart-definition block that the analytic records + `0x0121` styling blob describe.
            0x0128 => Some("ChartDefinition"),
            // Chart analytic header. bytes[2] and bytes[4] are small enum codes that vary per chart (b2 ∈ {0,1,2};
            // b4 ∈ {1,2,5,0x15,0x16}) — a chart data-type / chart-type-or-subtype selector (exact
            // mapping to the SDK ChartType/DataType enums not confirmed; not exported).
            0x011c => Some("ChartAnalyticHeader"),
            // Chart data-value descriptor (open/close pair).
            // The labeled-value analytic variant. `ChartDataValue` (0x011f) leaf =
            // `[6B header][u32 BE strlen][value label][7B trailer]`
            // where the label is the chart's data-value description (e.g. "Count of Command.some_field").
            // `ChartDataValueEnd` (0x0120, empty) closes it. Structural; the binding it labels is the
            // sibling summary/group records.
            0x011f => Some("ChartDataValue"),
            0x0120 => Some("ChartDataValueEnd"),
            // Chart data-layout descriptor (open/close pair).
            // The grouped analytic variant. `ChartDataLayout` (0x0126) leaf is a fixed 28B block of
            // axis counts/flags (constant `00 00 00 01 …00 00 ff ff` across the corpus); it brackets
            // the chart's summary/group bindings and any `0x013f/0x0140` series, closed by
            // `ChartDataLayoutEnd` (0x0127, empty). Structural.
            0x0126 => Some("ChartDataLayout"),
            0x0127 => Some("ChartDataLayoutEnd"),
            // Chart data-series descriptor (open/close pair).
            // One per chart series/riser inside the `0x0126` data-layout (0..N per chart — e.g. 2 in a
            // multi-series bar chart, 0 in a single-series chart). `ChartDataSeries` (0x013f)
            // leaf is a 2B flag/count (`00 00`); `ChartDataSeriesEnd` (0x0140, empty) closes it.
            // Structural.
            0x013f => Some("ChartDataSeries"),
            0x0140 => Some("ChartDataSeriesEnd"),
            // Chart-definition v2 styling blob. Leaf leads with the chart title as `[2B header][u32 BE titlelen][title]`, then an
            // opaque fixed-schema render-style blob (axis/legend/colour/marker state). Only the title
            // prefix is legible; the remainder is opaque chart render styling (not decoded — no SDK
            // read surface and not exported). Named + layout documented for coverage.
            0x0121 => Some("ChartDefinition2"),
            0x018d => Some("ContainerReference"),
            // ── Completeness sweep: record types decoded elsewhere in the raise layer but not
            // previously present in this name table (so they scored as `Unknown` in `rpt streams`
            // despite being fully parsed). Named here so the coverage meter reflects real decode
            // coverage. Each is decoded/consumed at the cited raise site
            //
            // Per-object format/geometry records (decoded by `report_def::raise_report_definition`):
            0x00be => Some("ObjectPosition"), // Left/Top position (variable-width coords)
            0x00ec => Some("ObjectBorder"),   // border line styles + border/background colours
            0x00fc => Some("ObjectFormat"),   // object format flags (CanGrow/Suppress/…)
            0x00fd => Some("ObjectConditionFormat"), // object conditional-format formula slots
            0x00fe => Some("AreaSectionFormat"), // the area/section format block (byte0 = area kind)
            0x00ff => Some("SectionConditionFormat"), // section/area conditional-format formula slots
            0x0100 => Some("FontColor"),              // font colour record
            0x0101 => Some("FontConditionFormat"),    // font colour/style conditional-format slots
            // Text-object records (text object opener + its format/embedded-field children):
            0x00a5 => Some("TextObjectContainer"), // text object opener (parents 0xc2 TextObject content)
            0x00a7 => Some("TextObjectSubRecord"), // text object sub-record
            0x00c0 => Some("TextObjectFormat"),    // text object paragraph/format record
            0x00c4 => Some("TextEmbeddedField"),   // a field reference embedded in the text object
            // Object openers / field-object sub-records:
            0x00a1 => Some("FieldObjectSubRecord"), // field object sub-record
            0x00a3 => Some("SubreportObject"),      // subreport placement object
            0x00a9 => Some("DrawingObject"),        // line/box drawing object opener
            0x00ae => Some("PictureObject"),        // picture object base opener
            0x0166 => Some("FieldHeadingLink"),     // field-heading → field-object link
            0x0043 => Some("FormatObject"),         // format-object record (FRObj)
            // Data-definition / layout records decoded in `data_def` / `print_options`:
            0x00e5 => Some("Group"),                  // the group definition
            0x007e => Some("SummaryFieldDefinition"), // summary / running-total field def
            0x0029 => Some("RecordSortField"),        // record/group sort field
            0x0080 => Some("RunningTotalReset"),      // running-total reset condition
            0x007a => Some("ParameterRecord"), // parameter descriptor (0x7a) — drives ParameterField
            0x0031 => Some("CurrentValueRecord"), // parameter current-value record
            0x0061 => Some("SavedData"), // the saved-data batch descriptor (codec::saved_data)
            0x0066 => Some("PageSetup"), // page-setup record
            0x018e => Some("PaperRect"), // paper rectangle / margins
            0x0106 => Some("SubreportLink"), // a single subreport link (io::subreport_links)
            // ── Cross-tab / OLAP grid family (writers crostab1/crosstab/gridobj/fldgrid/grdfmtop).
            // Not exported to XML (RptToXml never exports cross-tabs); named here for record-tree
            // coverage. Field-level decode (dimensions, measures, grid format) lives in
            // `raise::report_def::crosstab`, not in this naming table. The object nests
            // `0xb9→0xb8→ObjectName "CrossTabN"`, dimensions as `0xce→0xcc→0xcb{field ref}`, and
            // per-cell grid formats as `0x0143`/`0x0145`.
            0x00b8 => Some("CrossTabObject"),
            0x00b9 => Some("CrossTabObjectWrapper"),
            0x00cb => Some("CrossTabDimensionField"), // carries the dimension's {table.field} ref
            0x00cc => Some("CrossTabDimensionGroup"),
            0x00cd => Some("CrossTabRecordDimension"), // per-record dimension wrapper nesting a CrossTabDimensionField under a CrossTabRecord (0xd2)
            0x00ce => Some("CrossTabDimension"),
            0x00d2 => Some("CrossTabRecord"),
            0x00d6 => Some("CrossTabGridObject"),
            0x00d7 => Some("CrossTabSummaryRecord"),
            0x00db => Some("CrossTabFieldGrid"),
            0x00dc => Some("CrossTabFieldGridEntry"),
            0x0143 => Some("CrossTabGridFormat"), // (opener)
            0x0145 => Some("CrossTabGridCellFormat"), // (per-cell, 11B)
            // Cross-tab column-group / total records; appear only in the
            // cross-tab B1Budget corpus reports alongside 0xcd. Named for coverage.
            0x017e => Some("CrossTabColumnGroupIndex"), // 4B leaf, column-group index/count
            0x017f => Some("CrossTabTotalValue"), // 0B marker for the per-column total binding
            // Chart analytic open/close pair, distinct from the `0x0128`/`0x011c`/`0x011f`/`0x0121`
            // flow documented above:
            0x0122 => Some("ChartAnalyticRecord"),
            0x0123 => Some("ChartAnalyticRecordEnd"),

            // ══════════════════════════════════════════════════════════════════════════════════
            // Completeness sweep — WAVE 2: the paired partners (terminator / close / content) of
            // already-named opener records. Every close below was verified by tracing the linear
            // record order across the corpus (`rpt --example flat`): the on-disk stream is a stack
            // of open/close brackets where a close is almost always `opener_type + 1`. Each entry
            // names the opener it pairs with. All are structural (they add nothing to the export) —
            // named for record-type coverage only.
            //
            // ── Font-colour + field-format block (per field/text object) ───────────────────────
            // Each object's format prologue is `FontConditionFormat(0x101) · Font(0x08) · 0x0102`,
            // then (for value/field objects) the typed-format block `0x00ea … typed-wrappers … 0x00eb`.
            0x0102 => Some("FontColorEnd"), // closes the FontConditionFormat(0x101) font block
            0x00ea => Some("FieldFormat"),  // opens the per-field typed-format block (1/field)
            0x00eb => Some("FieldFormatEnd"), // closes it after StringFieldFormatWrapper(0xfb)
            // ── Report-object terminators (each object's record group ends with its close) ─────
            // Object body = `<opener> · pos(0xbe) · cond(0xfd) · adorn(0xed) · <format> · <End>`.
            0x00a0 => Some("FieldObjectEnd"), // 0x9f+1, terminates each field object
            0x00a2 => Some("FieldObjectDefinition"), // rare field-object def variant (crosstab/grid-embedded fields)
            0x00a4 => Some("SubreportObjectEnd"), // 0xa3+1, closes a subreport object (follows the 0x0105 link terminator)
            0x00a6 => Some("TextObjectEnd"), // terminates text objects and field-heading objects
            0x00a8 => Some("TextObjectDefinition"), // rare text-object def variant (crosstab/grid-embedded text)
            0x00ab => Some("LineDrawObjectEnd"),    // 0xaa+1, closes a LineDrawObject
            0x00ad => Some("BoxDrawObjectEnd"),     // 0xac+1, closes a BoxDrawObject
            0x00b0 => Some("PictureWrapperEnd"),    // 0xaf+1, closes a static/OLE PictureWrapper
            0x00b2 => Some("BlobFieldWrapperEnd"),  // 0xb1+1, closes a BlobFieldWrapper picture
            // ── Text-object paragraphs & runs ───────────────────────────────────
            // A text object holds paragraphs; each paragraph holds runs. A literal run is
            // `TextObject(0xc2) · <font block> · 0x00c3`; a field run is
            // `TextEmbeddedField(0xc4) · <font block> · <field-format block> · 0x00c5`; the
            // paragraph closes with `0x00c1`, the whole object with `0x00a6`.
            0x00c1 => Some("ParagraphEnd"), // 0xc0+1, closes a paragraph's run list
            0x00c3 => Some("TextRunEnd"),   // 0xc2+1, closes a literal text run (TextObject)
            0x00c5 => Some("TextFieldRunEnd"), // 0xc4+1, closes an embedded-field run (TextEmbeddedField)
            // ── Area pairs & their closes ───────────────────────────────────────
            // The report body is nested area pairs; each opener (0x82/0x84/0x86/0x88) + the Area
            // (0x8a) closes with opener+1, e.g.:
            // PageAreaPair[ Area[ …PageHeaderBand… ] 0x8b Area[ …PageFooterBand… ] 0x8b ] 0x85 …
            0x0083 => Some("ReportAreaPairEnd"),  // 0x82+1
            0x0085 => Some("PageAreaPairEnd"),    // 0x84+1
            0x0087 => Some("DetailAreaPairEnd"),  // 0x86+1
            0x0089 => Some("GroupAreaFormatEnd"), // 0x88+1
            0x008b => Some("AreaEnd"),            // 0x8a+1, closes an Area
            // ── Section-band closes ──────────────────────────────────────────────
            // Each band opener (0x8d..0x99, odd) closes with opener+1 (even). Verified by joining
            // each even close to its section's decoded band kind across the corpus.
            0x008e => Some("ReportHeaderBandEnd"), // 0x8d+1
            0x0090 => Some("ReportFooterBandEnd"), // 0x8f+1
            0x0092 => Some("PageHeaderBandEnd"),   // 0x91+1
            0x0094 => Some("PageFooterBandEnd"),   // 0x93+1
            0x0096 => Some("DetailBandEnd"),       // 0x95+1
            0x0098 => Some("GroupHeaderBandEnd"),  // 0x97+1
            0x009a => Some("GroupFooterBandEnd"),  // 0x99+1
            // ── Group definition close + rare group-options record ──────────────
            0x00e6 => Some("GroupEnd"), // 0xe5+1, closes a Group definition (immediately follows Group)
            0x00e8 => Some("GroupOptionsRecord"), // secondary group-options record
            // ── Designer rulers & guidelines closes ──────────────
            // Nested: RulerDefinition[ GuidelineCollection[ ObjectConnection… ] 0x0110 ] 0x0109 and
            // RulerScale[ GuidelineList[ ObjectConnection… ] 0x010e ] 0x010b (verified in trace).
            0x0109 => Some("RulerDefinitionEnd"),     // 0x108+1
            0x010b => Some("RulerScaleEnd"),          // 0x10a+1
            0x010e => Some("GuidelineListEnd"),       // 0x10d+1
            0x0110 => Some("GuidelineCollectionEnd"), // 0x10f+1 (high-volume, pairs the 0x010f collection)
            // ── Report edit-history closes ───────────────────────────────────────
            // HistoryInfo(0x17b)[ HistoryEntry(0x179)·SaveMetadata×N·0x017a … ] 0x017c.
            0x017a => Some("HistoryEntryEnd"), // 0x179+1, closes each HistoryEntry (+ its SaveMetadata group)
            0x017c => Some("HistoryInfoEnd"),  // 0x17b+1, closes the history block
            // ── Interactive-sort closes ─────────────────────────────────────────
            0x018a => Some("InteractiveSortEnd"), // 0x189+1, closes the InteractiveSort manager
            0x018c => Some("InteractiveSortEntryEnd"), // 0x18b+1, closes each InteractiveSortEntry
            // ── Chart-object closes ───────────────────
            0x00b5 => Some("ChartObjectEnd"), // 0xb4+1, closes the chart object
            0x011d => Some("ChartAnalyticHeaderEnd"), // 0x11c+1, closes the analytic section
            0x0129 => Some("ChartDefinitionEnd"), // 0x128+1, closes the chart-definition block
            // ── Cross-tab / OLAP grid closes & bindings
            // e.g.: CrossTabGridFormat[ …cells… ]0x0144 · CrossTabDimension
            // ·0x00df·Group·0x00e0 … ]0x00cf · CrossTabRecord …]0x00d3 · CrossTabSummaryRecord·
            // CrossTabFieldGrid·…entry…·0x00d8.
            0x00cf => Some("CrossTabDimensionEnd"), // 0xce+1, closes a CrossTabDimension
            0x00d3 => Some("CrossTabRecordEnd"),    // 0xd2+1, closes a CrossTabRecord
            0x00d8 => Some("CrossTabSummaryRecordEnd"), // 0xd7+1, closes a summary/field-grid block
            0x00df => Some("CrossTabDataBinding"),  // opens a dimension/record group binding
            0x00e0 => Some("CrossTabDataBindingEnd"), // 0xdf+1, closes it (wraps the on-change Group)
            0x00ba => Some("CrossTabObjectRecord"), // 1/crosstab, near the field-grid (exact role unpinned)
            0x0144 => Some("CrossTabGridFormatEnd"), // 0x143+1, closes CrossTabGridFormat
            // ── Misc singleton closes / wrappers ───────────────────────────────────────────────
            0x0105 => Some("SubreportLinkCollectionEnd"), // 0x104+1 (the empty terminator noted at 0x0104)
            0x0114 => Some("ReportFormatInterface"), // opens the tail block wrapping the formula-variable table
            0x0115 => Some("ReportFormatInterfaceEnd"), // closes it
            0x0117 => Some("FormulaVariableTableEnd"), // 0x116 terminator (noted at 0x0116/0x0118)
            0x016b => Some("ReportDocumentContainerEnd"), // 0x16a+1, closes ReportDocumentContainer
            0x0070 => Some("FieldManagerEnd"), // 0x6f+1, closes the field/formula/param/summary def block
            0x0067 => Some("ReportDefinitionTrailer"), // tail singleton (1/report) before ReportFormatInterface
            0x0065 => Some("ReportRootEnd"), // 0x64+1, the final Contents record (closes ReportRoot)
            0x018f => Some("PaperRectEnd"),  // 0x18e+1, companion/terminator following PaperRect
            0x0044 => Some("FormatObjectEnd"), // 0x43+1, closes FormatObject (precedes the saved-data batch)
            0x0062 => Some("InstanceManager"), // the saved-data instance manager (follows the SavedData batch)
            // QESession tail state: last flat record of every QESession stream, 1/stream.
            // (The QE connection/table/field records are nested children, not flat records, so they
            // are NOT in this coverage meter — no namespace collision on 0x02/0x04 here.)
            0x001e => Some("DataSourceState"),
            // ── Report-document header slot. NAMESPACE COLLISION: in a
            // QESession stream 0x0002/0x0004 are the (nested) QE connection/field records; those
            // do not reach this flat name table (see the 0x001e note), so these Contents names are
            // safe for the meter and only cosmetic if ever shown for a QE node.
            0x0002 => Some("DataSourceManagerHeader"),
            0x0004 => Some("ReportDocument5Header"),
            0x0006 => Some("ReportDocument5HeaderAlt"), // /
            // ── ReportParametersStream framing (parameter current values;).
            // Per-parameter shape: `DataSourceParameterEntry(0x3b) · DataSourceParameterValue(0x30)
            // · CurrentValueRecord(0x31) · FormulaFieldWrapper(0x77) · DataSourceParameterValueEnd
            // (0x33) · DataSourceParameterEntryEnd(0x3c)`, bracketed by the stream header/footer.
            // COLLISION: the FR* runtime objects (frfieldobject/frtextobject) reuse 0x30/0x3b/0x3c
            // but are never serialised to a .rpt; and 0x012f/0x0130 are rptdoc records in Contents.
            // In the corpus these codes appear only in ReportParametersStream, so name for that role.
            0x012f => Some("DataSourceParametersHeader"),
            0x0130 => Some("DataSourceParametersFooter"),
            0x003b => Some("DataSourceParameterEntry"),
            0x0030 => Some("DataSourceParameterValue"),
            0x0033 => Some("DataSourceParameterValueEnd"),
            0x003c => Some("DataSourceParameterEntryEnd"),
            // ── Absent-from-corpus feature families ─────────────────────────────────────────────
            // These record types are IDENTIFIED at the type level but byte layouts are not decoded.
            // They are named here at the FAMILY level for recognition only, so a report that DOES
            // use these features renders identified record types instead of raw hex. Adjacent code
            // pairs follow the usual `opener` / `opener+1` open/close convention seen throughout
            // the format, but that pairing is unverified for these (no corpus instance) — treat the
            // names as family labels, not decoded semantics.
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
