# Support matrix

What `rpt-rs` does and does not handle. Everything not decoded is still preserved verbatim in the
[record substrate](04-record-tree.md), so reading is always lossless even where modelling is incomplete.

Legend: ✅ decoded · ◐ partial · ○ recognized but not decoded (passed through).

## Pipeline

| Stage                          | Status | Notes                                                  |
|--------------------------------|:------:|--------------------------------------------------------|
| CFB / OLE container            |   ✅   | via the `cfb` crate; reads all observed versions       |
| Stream classification          |   ✅   | every stream identified or kept with a stable identity |
| Stream header (`0xFFFF`)       |   ✅   | version + per-stream IV                                |
| AES-128-CFB decryption         |   ✅   | fixed-key files; pure-Rust cipher                      |
| zlib decompression             |   ✅   | standard deflate                                       |
| Record tiling                  |   ✅   | consumes every logical byte                            |
| Record tree (nesting + mask)   |   ✅   | full recursive tree, lossless substrate                |
| Subreports                     |   ✅   | recurse into `Subdocument N` storages                  |
| Lossless round-trip of records |   ✅   | unknown records preserved verbatim                     |

## Streams

| Stream                                    | Status | Notes                                                                                  |
|-------------------------------------------|:------:|----------------------------------------------------------------------------------------|
| `Contents`                                |   ✅   | the report definition                                                                  |
| `QESession`                               |   ✅   | connections, tables, fields, joins, SQL                                                |
| `SummaryInformation`                      |   ✅   | OLE property set (title/author/timestamps/app)                                         |
| `ReportParametersStream`                  |   ✅   | saved parameter current values; a TSLV record stream like `Contents`                   |
| `PromptManager`                           |   ◐    | parameter prompting metadata used during decode                                        |
| `DataSourceManager`                       |   ◐    | saved-data batch directory + field catalog (QE record dialect)                         |
| `SavedRecordsStream` / `MemoValuesStream` |   ◐    | stored rows (saved data); two batch classes decode ([details](06-saved-data.md))       |
| `ReportInfo`                              |   ○    | 58-byte plaintext capability cache (chart/map flags); derivable, not modelled          |
| `CrystalReportDesignerStream`             |   ○    | 114-byte design-time editor state (design-time reports only); not modelled             |
| `Embedding N` / `CHART N` / `zlibBLOB N`  |   ○    | embedded payloads, not modelled                                                        |
| Engine side caches                        |   ○    | `TotallerStream` / `AnalysisGridsStream` / `ConstantRecordsStream` / `FormulaRecordsStream` / `ViewInformationStream` — derivable from the definition, not modelled |
| `ExportFormatOptionsStream`               |   ○    | classified by the reader; absent from every report in the corpus                       |

## Record types

### Decoded — the structural core

These are the record types that build a report's skeleton. The additional decoded types (field sub-formats, chart and
cross-tab detail, designer state, authoring provenance) are enumerated under
[Record-type coverage](#record-type-coverage) below.

| Code            | Name                          | Code     | Name                     |
|-----------------|-------------------------------|----------|--------------------------|
| `0xFFFF`        | StreamHeader                  | `0x008a` | Area                     |
| `0x0064`        | ReportRoot                    | `0x008c` | Section                  |
| `0x0002`        | QE_Connection                 | `0x008d`–`0x0099` | Band markers    |
| `0x0003`        | PrinterInfo / QE_Table        | `0x009e` | ObjectName               |
| `0x0004`        | QE_Field                      | `0x009f` | FieldObject              |
| `0x0007`        | PaperSize / QE_CommandParam   | `0x00a3` | SubreportObject          |
| `0x0008`        | Font                          | `0x00a5` | TextObject               |
| `0x000a`        | QE_TableLink                  | `0x00a9` | LineObject / Box         |
| `0x0029`        | RecordSortField               | `0x00ae` | PictureObject            |
| `0x0031`        | CurrentValueRecord            | `0x00b1` | BlobFieldRef             |
| `0x0061`        | SavedData (descriptor)        | `0x00be` | ObjectPosition           |
| `0x0066`        | PageSetup                     | `0x00c0` | TextObjectFormat         |
| `0x006c`        | MultiColumnFormat             | `0x00c2` | TextContent              |
| `0x0071`        | NamedValue                    | `0x00c4` | TextEmbeddedField        |
| `0x0073`        | FieldDef                      | `0x00ec` | ObjectBorder             |
| `0x0076`        | Formula / CustomFunction      | `0x00fc` | ObjectFormat             |
| `0x007a`        | ParamRecord                   | `0x00fd` | ObjectConditionFormulas  |
| `0x007e`        | SummaryDef                    | `0x00fe` | AreaSectionFormat        |
| `0x0080`        | RunningTotalReset             | `0x00ff` | SectionConditionFormulas |
| `0x0081`        | SQL Expression field          | `0x0100` | FontColor                |
| `0x0088`        | GroupAreaFormat               | `0x0101` | FontConditionFormulas    |
| `0x00e5`        | Group                         | `0x0106` | SubReportLink            |
| `0x0160`        | Data-source options           | `0x0166` | FieldHeadingLink         |
| `0x018e`        | PaperRect                     |          |                          |

See the [block catalog](07-block-catalog.md) for each one's meaning and layout.

### Record-type coverage

Practically every record type that occurs in the corpus is **identified and named** in the record registry
(`RecordTag::name()`). Measured across the 218 reports the repository can read (the public demo sets, the render
fixtures, and the Meridian corpus), `rpt streams` reports **163,212 records, of which 2 are still `Unknown`** — both in
the `ReportParametersStream` of two fixtures; the `Contents` and `QESession` streams of every corpus report are fully
named. That includes the rare codes `0x017e`/`0x017f`
(`CrossTabCustomMembersBegin`/`CrossTabCustomMembersEnd`, the bracket around a cross-tab's custom-group-members
collection; the per-member detail records `0x0180`/`0x0181` do not occur in the corpus). Naming these types is
format-completeness work, measured by `rpt streams` rather than by the decode baselines — nothing about them reaches
the model yet, so a baseline cannot see them.

Coverage comes in two grades:

- **Decoded into the model** — typed field sub-formats (`0x00ee`–`0x00fb`, stored attrs; the effective date/time/numeric
  display is runtime-derived and excluded like `NumberOfBytes`), object hyperlinks (text + type, from the `0x00fc`
  leaf), hierarchical grouping (`0x00e9`), formula variables (`0x0116`/`0x0118` → name/type/scope), save metadata
  (`0x0178`), subreport re-import (`0x0142`), the field-manager census (`0x006e`), section codes / area-type (`0x009b`–
  `0x009d`), designer guidelines/connections coordinates (`0x010c`/`0x0111`), the cross-tab object / dimensions / grid
  formats (`0x00b8`/`0x00b9`/`0x00cb`/`0x00ce`/`0x00d2`/`0x0143`/`0x0145`), the chart binding / data records (`0x00b4`/
  `0x007f`) and chart data-value labels (`0x011f`), the OLE embedding ordinal (`0x00bd`), the object border-colour
  condition (`0x00ed`), container references (`0x018d`), and the parameter sort/display flags
  (`ParameterFieldDefinition`). Also decoded: report-level custom functions (`0x0076` bodies with a `0xFF80` sentinel),
  summary OperationParameter + secondary-field layout (`0x007e`), string-format indent longs (`0x00fa`), numeric
  currency position (`0x00f8`), the group date/boolean condition (`0x00e5` → `GroupCondition`), the group area-pair
  visible-groups-per-page count (`0x0088`), the special-field type codes (`SpecialFieldType`), and the extended
  parameter attributes (type, on-panel flags, allow-custom-values, default-value sort order, discrete-or-range kind).
- **Named for recognition** — the many open/close bracket and wrapper/terminator records (e.g. `FieldManagerEnd`,
  `ReportRootEnd`, the section-band and area-pair ends, chart/cross-tab/ruler/guideline/history ends), which are
  structurally redundant with the content they bracket, plus opaque render state (the chart styling blob `0x0121`) and
  the designer/IDE state whose semantics carry nothing a reader needs.

**Absent-from-corpus families** (cross-tab is partially present and named; full OLAP grid, maps, dimension selection,
alerts, Flash/Xcelsius, and XML/XSLT export defs do **not** occur in the corpus) are named at the **family** level for
recognition only, but their byte layouts are not decoded — that needs sample reports authored in
a Crystal Reports designer.

## Feature areas

| Feature                                                                           | Status | Notes                                                                                                                                                                                                                                                                                                                       |
|-----------------------------------------------------------------------------------|:------:|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Data sources / tables / fields                                                    |   ✅   | connections, tables, command (SQL) tables, joins                                                                                                                                                                                                                                                                            |
| Parameters                                                                        |   ✅   | definitions, types, default/current values; on-panel flags, allow-custom-values, sort order, discrete/range kind                                                                                                                                                                                                            |
| Formulas                                                                          |   ✅   | bodies and references; report-level custom functions (main report only)                                                                                                                                                                                                                                     |
| Record / group selection formulas                                                 |   ✅   |                                                                                                                                                                                                                                                                                                                             |
| Groups & sorting                                                                  |   ◐    | groups (with typed date/boolean grouping condition), sort fields, and Top N / Bottom N summary sorts (incl. DiscardOthers and visible-groups-per-page) decoded; Top-N WithTies is not stored in the file (runtime property)                                                                                                 |
| Summaries & running totals                                                        |   ✅   |                                                                                                                                                                                                                                                                                                                             |
| Sections & areas                                                                  |   ✅   | with formatting and conditional formatting                                                                                                                                                                                                                                                                                  |
| Report objects (field, text, line, box, picture, subreport)                       |   ✅   | placement, formatting, fonts, borders                                                                                                                                                                                                                                                                                       |
| Object hyperlinks                                                                 |   ✅   | hyperlink text + type decoded from the object-format leaf                                                                                                                                                                                                                                   |
| Subreports & subreport links                                                      |   ✅   | including value passing between reports                                                                                                                                                                                                                                                                                     |
| Page setup / print options                                                        |   ✅   | paper size, orientation, margins, page rectangle, multi-column detail layout (`0x006c`)                                                                                                                                                                                                                                     |
| SQL Expression fields                                                             |   ✅   | `0x0081` — the pushed-down SQL text + name/type; listed by `rpt sql` with the generated query and stored SQL Commands                                                                                                                                                                                                        |
| Charts / graphs                                                                   |   ◐    | object + analytic layout + data-value labels decoded; styling blob named but opaque                                                                                                                                                                                                                                         |
| Cross-tabs / OLAP grids                                                           |   ◐    | cross-tab records named/structured (object, dimensions, grid formats); full OLAP grid absent from corpus                                                                                                                                                                                                                    |
| Hierarchical grouping                                                             |   ✅   | `0x00e9` group-value name + defining condition-formula decoded                                                                                                                                                                                                                                                              |
| Maps, alerts, Flash/Xcelsius, XML/XSLT export                                     |   ○    | named at family level for recognition only; absent from corpus, decode pending samples                                                                                                                                                                                                                                      |
| Typed field sub-formats (number/date/currency/time/boolean/string masks)          |   ✅   | stored format attrs decoded (model structs populated) **and consumed by the renderer** — `rpt-layout`'s format module merges the stored leaf with the render locale, arbitrated by `use_system_defaults` (see [format resolution](12-rendering.md#format-resolution)). The engine's runtime-resolved effective display format is excluded from parity (like `NumberOfBytes`)                                                                       |
| Formula variables (Global / Shared)                                               |   ✅   | name, result type, and scope decoded                                                                                                                                                                                                                                                                                        |
| Designer / IDE state (rulers, guidelines, connections, history, interactive sort) |   ◐    | recognized and geometry decoded; parity-inert (no SDK read surface)                                                                                                                                                                                                                                                         |
| Writing / editing `.rpt` files                                                    |   ◐    | a byte-faithful re-encoder ships (`Rpt::reencode`, `patch_record_leaf`/`patch_record_leaf_resize`; the `rpt reencode`/`patch` CLI commands): the substrate round-trips and a decoded record's leaf can be byte-patched. `Rpt::anonymize` (`rpt anonymize`) is the one *semantic* edit built on it — it strips authoring metadata. There is **no** general model→records lowering: you cannot mutate the semantic model and serialize it back |

## Lossless guarantee

Regardless of the above, every record read from a file is preserved with its exact bytes. Decoding adds typed meaning on
top; it never discards what it does not understand. The `rpt streams` command reports, per stream, how many records
remain undecoded for a given file.

---

← [Block catalog](07-block-catalog.md) · [Index](README.md) · **Next:** [Endianness](09-endianness.md) →
