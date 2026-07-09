# Support matrix

What `rpt-rs` does and does not handle. Everything not decoded is still preserved verbatim in the
[record substrate](04-record-tree.md), so reading is always lossless even where modelling is incomplete.

Legend: ✅ decoded · ◐ partial · ○ recognized but not decoded (passed through).

## Pipeline

| Stage                          | Status | Notes                                                  |
| ------------------------------ | :----: | ------------------------------------------------------ |
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
| ----------------------------------------- | :----: | -------------------------------------------------------------------------------------- |
| `Contents`                                |   ✅   | the report definition                                                                  |
| `QESession`                               |   ✅   | connections, tables, fields, joins, SQL                                                |
| `SummaryInformation`                      |   ✅   | OLE property set (title/author/timestamps/app)                                         |
| `PromptManager`                           |   ◐    | parameter prompting metadata used during decode                                        |
| `DataSourceManager`                       |   ◐    | saved-data batch directory + field catalog                                             |
| `SavedRecordsStream` / `MemoValuesStream` |   ◐    | stored rows (saved data); external-memo batch class only ([details](10-saved-data.md)) |
| `ReportInfo`                              |   ○    | fixed-size header, not modelled                                                        |
| `Embedding N` / `CHART N` / `zlibBLOB N`  |   ○    | embedded payloads, not modelled                                                        |

## Record types

### Decoded

| Code     | Name                   | Code     | Name                     |
| -------- | ---------------------- | -------- | ------------------------ |
| `0xFFFF` | StreamHeader           | `0x008a` | Area                     |
| `0x0064` | ReportRoot             | `0x008c` | Section                  |
| `0x0002` | QE_Connection          | `0x009e` | ObjectName               |
| `0x0003` | PrinterInfo / QE_Table | `0x009f` | FieldObject              |
| `0x0004` | QE_Field               | `0x00a3` | SubreportObject          |
| `0x0007` | PaperSize / DEVMODE    | `0x00a5` | TextObject               |
| `0x0008` | Font                   | `0x00a9` | LineObject / Box         |
| `0x000a` | QE_TableLink           | `0x00ae` | PictureObject            |
| `0x0029` | RecordSortField        | `0x00be` | ObjectPosition           |
| `0x0066` | PageSetup              | `0x00c0` | TextObjectFormat         |
| `0x0071` | NamedValue             | `0x00c2` | TextContent              |
| `0x0073` | FieldDef               | `0x00c4` | TextEmbeddedField        |
| `0x0076` | Formula                | `0x00ec` | ObjectBorder             |
| `0x007a` | ParamRecord            | `0x00fc` | ObjectFormat             |
| `0x007e` | SummaryDef             | `0x00fd` | ObjectConditionFormulas  |
| `0x0080` | RunningTotalReset      | `0x00fe` | AreaSectionFormat        |
| `0x0088` | GroupAreaFormat        | `0x00ff` | SectionConditionFormulas |
| `0x00e5` | Group                  |          |                          |
| `0x0100` | FontColor              | `0x0101` | FontConditionFormulas    |
| `0x0106` | SubReportLink          | `0x0166` | FieldHeadingLink         |
| `0x018e` | PaperRect              |          |                          |

See the [block catalog](06-block-catalog.md) for each one's meaning and layout.

### Record-type coverage

Every record type that occurs in the 283-report corpus is now **identified and named** in the record registry
(`RecordTag::name()`). The `rpt streams` undecoded count across the whole corpus fell from ~138,700 records to **0** —
every corpus record type is named, including the last two rare codes `0x017e`/`0x017f` (identified as
`CrossTabColumnGroupIndex`/`CrossTabTotalValue`). None of these types affect oracle parity — the reference exporter never
emits their content — so this was format-completeness work, measured by `rpt streams`, not `oraclematch.py`.

Coverage comes in two grades:

- **Decoded into the model** — typed field sub-formats (`0x00ee`–`0x00fb`, stored attrs; the effective date/time/numeric
  display is runtime-derived and excluded like `NumberOfBytes`), object hyperlinks (text + type, from the `0x00fc`
  leaf), hierarchical grouping (`0x00e9`), formula variables (`0x0116`/`0x0118` → name/type/scope), save metadata
  (`0x0178`), subreport re-import (`0x0142`), the field-manager census (`0x006e`), section codes / area-type
  (`0x009b`–`0x009d`), designer guidelines/connections coordinates (`0x010c`/`0x0111`), the cross-tab object /
  dimensions / grid formats (`0x00b8`/`0x00b9`/`0x00cb`/`0x00ce`/`0x00d2`/`0x0143`/`0x0145`), the chart binding / data
  records (`0x00b4`/`0x007f`) and chart data-value labels (`0x011f`), the OLE embedding ordinal (`0x00bd`), the object
  border-colour condition (`0x00ed`), container references (`0x018d`), and the parameter sort/display flags
  (`ParameterFieldDefinition`).
- **Named for recognition** — the many open/close bracket and wrapper/terminator records (e.g. `FieldManagerEnd`,
  `ReportRootEnd`, the section-band and area-pair ends, chart/cross-tab/ruler/guideline/history ends), which are
  structurally redundant with the content they bracket, plus opaque render state (the chart styling blob `0x0121`) and
  the designer/IDE state whose semantics carry nothing a reader needs.

**Absent-from-corpus families** (cross-tab is partially present and named; full OLAP grid, maps, dimension selection,
alerts, Flash/Xcelsius, and XML/XSLT export defs do **not** occur in the corpus) are named at the **family** level from
the crpe32 writer TU map for recognition, but their byte layouts are not decoded — that needs sample reports authored in
a Crystal Reports designer.

## Feature areas

| Feature                                                     | Status | Notes                                                                                       |
| ----------------------------------------------------------- | :----: | ------------------------------------------------------------------------------------------- |
| Data sources / tables / fields                              |   ✅   | connections, tables, command (SQL) tables, joins                                            |
| Parameters                                                  |   ✅   | definitions, types, default/current values                                                  |
| Formulas                                                    |   ✅   | bodies and references                                                                       |
| Record / group selection formulas                           |   ✅   |                                                                                             |
| Groups & sorting                                            |   ◐    | groups, sort fields, and Top N / Bottom N summary sorts decoded; some group options pending |
| Summaries & running totals                                  |   ✅   |                                                                                             |
| Sections & areas                                            |   ✅   | with formatting and conditional formatting                                                  |
| Report objects (field, text, line, box, picture, subreport) |   ✅   | placement, formatting, fonts, borders                                                       |
| Object hyperlinks                                           |   ✅   | hyperlink text + type decoded from the object-format leaf (not emitted by the XML export)    |
| Subreports & subreport links                                |   ✅   | including value passing between reports                                                     |
| Page setup / print options                                  |   ✅   | paper size, orientation, margins, page rectangle                                            |
| Charts / graphs                                             |   ◐    | object + analytic layout + data-value labels decoded; styling blob named but opaque |
| Cross-tabs / OLAP grids                                     |   ◐    | cross-tab records named/structured (object, dimensions, grid formats); full OLAP grid absent from corpus |
| Hierarchical grouping                                       |   ✅   | `0x00e9` group-value name + defining condition-formula decoded                              |
| Maps, alerts, Flash/Xcelsius, XML/XSLT export               |   ○    | named at family level from the writer TU map; absent from corpus, decode pending samples    |
| Typed field sub-formats (number/date/currency/time/boolean/string masks) | ✅ | stored format attrs decoded (model structs populated); runtime-resolved display format excluded from parity (like `NumberOfBytes`). Wiring these stored specs into the renderer is separate follow-on work — the layout engine currently formats with type defaults |
| Formula variables (Global / Shared)                         |   ✅   | name, result type, and scope decoded                                                        |
| Designer / IDE state (rulers, guidelines, connections, history, interactive sort) | ◐ | recognized and geometry decoded; parity-inert (no SDK read surface)              |
| Writing / editing `.rpt` files                              |   ◐    | a byte-faithful re-encoder ships (`Rpt::reencode`, `patch_record_leaf`/`patch_record_leaf_resize`; the `rpt reencode`/`patch` CLI commands): the substrate round-trips and a decoded record's leaf can be byte-patched. There is **no** model→records lowering — you cannot mutate the semantic model and serialize it back |

## Lossless guarantee

Regardless of the above, every record read from a file is preserved with its exact bytes. Decoding adds typed meaning on
top; it never discards what it does not understand. The `rpt streams` command reports, per stream, how many records
remain undecoded for a given file.
