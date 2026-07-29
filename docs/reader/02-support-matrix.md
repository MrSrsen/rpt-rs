# Support matrix

What `rpt-rs` does and does not handle. Everything not decoded is still preserved verbatim in the
[record layer](../format/04-record-tree.md), so reading is always lossless even where modelling is incomplete.

Legend: ✅ decoded · ◐ partial · ○ recognized but not decoded (passed through).

## Read and modelled

Two things are measured on this page, and they are independent: whether the reader **reads** a value out of the bytes,
and whether that reading is **carried onto the model**. A value can be read exactly — named, typed, its byte range
accounted for — and still stop at the decoder, because nothing assigns it to a model field. `rpt dump` shows a record's
own reading and so shows such a value; the model field keeps its default, and the JSON dump reports that default rather
than the stored value.

This page calls that **read but not modelled**. *Not decoded* is the stronger statement: the bytes themselves are
unread, so `rpt dump` shows the hex and nothing names it.

## Pipeline

| Stage                          | Status | Notes                                                  |
|--------------------------------|:------:|--------------------------------------------------------|
| CFB / OLE container            |   ✅   | via the `cfb` crate; reads all observed versions       |
| Stream classification          |   ✅   | every stream identified or kept with a stable identity |
| Stream header (`0xFFFF`)       |   ✅   | version + per-stream IV                                |
| AES-128-CFB decryption         |   ✅   | fixed-key files; pure-Rust cipher                      |
| zlib decompression             |   ✅   | standard deflate                                       |
| Flat record split              |   ✅   | consumes every logical byte                            |
| Record tree (nesting + mask)   |   ✅   | full recursive tree, lossless                          |
| Subreports                     |   ✅   | recurse into `Subdocument N` storages                  |
| Lossless round-trip of records |   ✅   | unknown records preserved verbatim                     |

## Streams

| Stream                                    | Status | Notes                                                                                                                                                                                                              |
|-------------------------------------------|:------:|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `Contents`                                |   ✅   | the report definition                                                                                                                                                                                              |
| `QESession`                               |   ✅   | connections, tables, fields, indexes, bind parameters, joins, SQL                                                                                                                                                  |
| `SummaryInformation`                      |   ✅   | OLE property set (title/author/timestamps/app)                                                                                                                                                                     |
| `ReportParametersStream`                  |   ✅   | saved parameter current values; a TSLV record stream like `Contents`                                                                                                                                               |
| `PromptManager`                           |   ◐    | the parameter objects only — [what the ◐ excludes](#the-three-partial-streams)                                                                                                                                     |
| `DataSourceManager`                       |   ◐    | the saved-data substrate only — [what the ◐ excludes](#the-three-partial-streams)                                                                                                                                  |
| `SavedRecordsStream` / `MemoValuesStream` |   ◐    | stored rows (saved data) — [what the ◐ excludes](#the-three-partial-streams)                                                                                                                                       |
| `ReportInfo`                              |   ○    | 58-byte plaintext capability cache (chart/map flags); derivable, not modelled                                                                                                                                      |
| `CrystalReportDesignerStream`             |   ○    | 114-byte design-time editor state (design-time reports only); not modelled                                                                                                                                         |
| `Embedding N`                             |   ◐    | the `Ole` stream is digested into `Report.embeds` and `CONTENTS` supplies a picture's image bytes; the OLE payload itself is not parsed                                                                            |
| `CHART N` / `zlibBLOB N`                  |   ○    | embedded payloads, not modelled                                                                                                                                                                                    |
| Engine side caches                        |   ○    | `TotallerStream` / `AnalysisGridsStream` / `ConstantRecordsStream` / `FormulaRecordsStream` / `ViewInformationStream` / `DataViewSortIndex` / `DataViewRecordFilter` — derivable from the definition, not modelled |
| `ExportFormatOptionsStream`               |   ○    | classified by the reader; absent from every report in the corpus                                                                                                                                                   |

### The three partial streams

A `◐` above means the reader reads part of the stream and leaves a known remainder. What that remainder is, per stream:

**`PromptManager`.** The stream inflates to a *sequence* of `<CRMetaObjects>` documents, and only the **first** is
decrypted and parsed. That first document holds the `Type=Parameter` objects, from which the reader takes the
parameter's name, value type, prompt-group reference, viewer-panel visibility and the group membership flags, plus the
initial values; everything else a `ParameterField` carries comes from the `Contents` `0x007a` records and the
`ReportParametersStream`. **Excluded:** every later document — the `Type=PromptGroup` objects — so the prompt-group
definitions are entirely unmodelled: the group's own name and hidden flag, the ordered prompt list that expresses a
cascading prompt, and the per-prompt edit mask, null/custom/discrete-vs-range flags and pick-list mirror. Four
`ParameterField` fields consequently keep their default on every report — `edit_mask`, `allow_null_value`,
`dynamic_lov` and `report_name`. Three of them are read from nowhere; `edit_mask` is
[read but not modelled](#read-and-modelled), off the `0x007a` record's own bytes.

**`DataSourceManager`.** Read only for what saved-data decoding needs: the batch directory (`0x2d` for the persistent
record width; `0x6d` for a batch's row count, record width, stream span and, for a packed batch, its column boundaries)
and the **database-field** slot catalog (`0x07` › `0x41` › `0x40` → field name, record offset, memo flag). **Excluded:**
the parallel slot catalogs for formula, summary/running-total and special fields — so a saved column is never a formula
or summary column — and every other record type the stream carries. Only the types listed above are named in the record
registry; the rest are unread. Because the stream is written in its own **`Catalog`** vocabulary and read through the
saved-data path rather than through the record walk, none of that appears as an unknown record or an uncovered byte in
`rpt streams`.

**`SavedRecordsStream` / `MemoValuesStream`.** Two stored row layouts decode — the memo-heap class (variable-length
values in an external `MemoValuesStream`, resolved per row through the memo descriptors) and the memo-less class
(strings inline, in fixed slots or packed per batch). **Excluded:** a record that is packed *and* carries a memo column,
which neither path handles; any batch whose stored metadata does not yield a valid decryption IV, which surfaces as
`saved_data: null` on the model and is told apart from a report with no saved data by
`Rpt::saved_data_status` (also printed by `rpt streams` and `rpt saved`); and a subreport's own saved batch, which is
never read. What *is* decoded is the **stored** rowset rather than the engine's presented one — no projection, ordering,
grouping, formula evaluation or formatting, and dates and times stay raw serials.
See [saved data](../format/05-saved-data.md).

## Record types

### Decoded — the structural core

These are the record types that build a report's skeleton. The additional decoded types (field sub-formats, chart and
cross-tab detail, designer state, authoring provenance) are enumerated under
[Record-type coverage](#record-type-coverage) below.

| Code     | Name                             | Code              | Name                   |
|----------|----------------------------------|-------------------|------------------------|
| `0xFFFF` | StreamHeader                     | `0x008a`          | Area                   |
| `0x0064` | ReportRoot                       | `0x008c`          | Section                |
| `0x0002` | QeConnection ᵠ                   | `0x008d`–`0x0099` | Band markers           |
| `0x0003` | PrinterInfo / QeTable ᵠ          | `0x009e`          | ObjectName             |
| `0x0004` | QeField ᵠ                        | `0x009f`          | FieldObject            |
| `0x0007` | PaperSize / QeCommandParameter ᵠ | `0x00a3`          | SubreportObject        |
| `0x0008` | Font / QeIndex ᵠ                 | `0x00a5`          | TextObjectContainer    |
| `0x000a` | QeTableLink ᵠ                    | `0x00a9`          | DrawingObject          |
| `0x0029` | RecordSortField                  | `0x00ae`          | PictureObject          |
| `0x0031` | CurrentValueRecord ᵖ             | `0x00b1`          | BlobFieldWrapper       |
| `0x0061` | SavedData (descriptor)           | `0x00be`          | ObjectPosition         |
| `0x0066` | PageSetup                        | `0x00c0`          | TextObjectFormat       |
| `0x006c` | MultiColumnFormat                | `0x00c2`          | TextObject             |
| `0x0071` | NamedValue                       | `0x00c4`          | TextEmbeddedField      |
| `0x0073` | FieldDef                         | `0x00ec`          | ObjectBorder           |
| `0x0076` | Formula / CustomFunction         | `0x00fc`          | ObjectFormat           |
| `0x007a` | ParameterRecord                  | `0x00fd`          | ObjectConditionFormat  |
| `0x007e` | SummaryFieldDefinition           | `0x00fe`          | AreaSectionFormat      |
| `0x0080` | RunningTotalField                | `0x00ff`          | SectionConditionFormat |
| `0x0081` | SqlExpressionField               | `0x0100`          | FontColor              |
| `0x0088` | GroupAreaFormat                  | `0x0101`          | FontConditionFormat    |
| `0x00e5` | Group                            | `0x0106`          | SubreportLink          |
| `0x0160` | ReportOptions                    | `0x0166`          | FieldHeadingLink       |
| `0x018e` | PaperRect                        |                   |                        |

Unmarked names are `Contents` records; ᵠ is `QESession` and ᵖ the `ReportParametersStream`. These are the registry's own
spellings, which is what `rpt dump --type` accepts — a name is resolved to a type *number*, and `--stream` then decides
which record of that number you get.

See the [block catalog](../format/06-block-catalog.md) for each one's meaning and layout.

### Record-type coverage

Practically every record type that occurs in the corpus is **identified and named** in the record registry
(`RecordTag::name(dialect)`, which takes the stream's record vocabulary because a type number means different records in
`Contents`, `QESession`, the saved-data catalog and the saved parameter values). Across the committed corpus — 154
reports: the 128 decode fixtures under `tests/fixtures/reports/` plus the 26 under `tests/meridian/` — the `Contents`
and `QESession` streams of every report, main and subreport alike, are fully named, and a single record type anywhere in
the streams walked as records is still `Unknown`: **`0x32` in the `ReportParametersStream`**, 2 records in 2 reports.
(The `DataSourceManager`
catalog is read by another route and is not measured here — see [above](#the-three-partial-streams).) A corpus sweep in
`rpt-reader` holds that exception list exactly, so a report introducing an unidentified type fails a test rather than
widening the gap unnoticed. The named types include the rare codes `0x017e`/`0x017f`
(`CrossTabCustomMembersBegin`/`CrossTabCustomMembersEnd`, the bracket around a cross-tab's custom-group-members
collection; the opener states how many members the bracket holds and every one in the corpus states none, which is why
the per-member detail records `0x0180`/`0x0181` do not occur there). Naming these types is format-completeness work,
measured by that sweep rather than by the decode baselines — nothing about them reaches the model yet, so a baseline
cannot see them.

Which walk a count came from decides what it can claim, and the two here differ. The `rpt-reader` sweep re-decodes each
subreport's own streams, which is what lets the paragraph above say "main and subreport alike". `rpt streams`
does not: only a top-level `Contents` is tiled, so a `Subdocument N/Contents` reports zero records. Over the same 154
reports it sees **55,829 outermost records, 106,950 in the trees, 2 unrecognized, 0 bytes covered by no record** — a
floor on the sweep's reach, not the whole of it.

Coverage comes in two grades:

- **Decoded into the model** — typed field sub-formats (`0x00ee`–`0x00fb`, stored attrs; the effective date/time/numeric
  display is runtime-derived and excluded like `NumberOfBytes`), object hyperlinks (text + type, from the `0x00fc`
  record), hierarchical grouping (`0x00e9`), formula variables (`0x0116`/`0x0118` → name/type/scope), save metadata
  (`0x0178`), subreport re-import (`0x0142`), the field-manager census (`0x006e`), an area's type and group nesting
  level (`0x009b`/`0x009c`), designer guidelines/connections coordinates (`0x010c`/`0x0111`), the cross-tab object /
  dimensions / grid formats (`0x00b8`/`0x00b9`/`0x00cb`/`0x00ce`/`0x00d2`/`0x00d6`/`0x0143`/`0x0145`), the chart
  binding / data records (`0x00b4`/`0x007f`), the chart definition's styling block (`0x0121`) and chart data-value
  labels (`0x011f`), the OLE embedding ordinal (`0x00bd`), the object border-color condition (`0x00ed`), container
  references (`0x018d`), and the parameter sort/display flags (`ParameterFieldDefinition`). Also decoded: report-level
  custom functions (`0x0076` bodies opening with the reserved `Function (…)` header), summary OperationParameter +
  secondary-field layout (`0x007e`), string-format indent longs (`0x00fa`), the numeric currency position, lead-zero /
  reverse-sign / per-page-symbol flags and zero-value string (`0x00f8`), the date era / calendar / day-of-week position,
  enclosure and the five date separators (`0x00f2`), the whole time record — clock base, designator format and text, and
  both element separators (`0x00f6`) — the report kind (`0x0066`), the group date/boolean condition (`0x00e5` →
  `GroupCondition`), the report's canned formatting style (`0x0066`), the per-page paging limits (records per page on
  the Detail `0x00fe`, groups per page on the group area-pair `0x0088`), the report file's creation / last-saved /
  last-printed timestamps (`SummaryInformation`), the special-field type codes (`SpecialFieldType`), the section a box
  or line's far corner lies in (`0x00a9` → `DrawingShape::end_section_index`), and the extended parameter attributes
  (type, on-panel flags, allow-custom-values, default-value sort order).
- **Named for recognition** — the many open/close bracket and wrapper/terminator records (e.g. `FieldManagerEnd`,
  `ReportRootEnd`, the section-band and area-pair ends, chart/cross-tab/ruler/guideline/history ends), which are
  structurally redundant with the content they bracket, and the designer/IDE state whose semantics carry nothing a
  reader needs. The query engine's index record (`QESession` `0x0008`) sits between the two grades — it is
  [read but not modelled](#read-and-modelled): the index name, its primary-key and uniqueness flags and the columns it
  covers are all read, and nothing about an index reaches the model, since no report behaviour depends on one.

**Undecoded families** are named at the **family** level for recognition only; their byte layouts are not read and
nothing about them reaches the model. Maps are the one that *does* occur — a single corpus report carries all six map
records (`0x00b6`/`0x00b7` object, `0x0119`–`0x011b` definition, `0x012a` layer) — so decoding them needs only the work,
not a new report. The full OLAP grid, dimension selection, alerts, Flash/Xcelsius and XML/XSLT export defs do **not**
occur in the corpus, so those need sample reports authored in a Crystal Reports designer first. (Cross-tab is partially
present and named.)

## Feature areas

| Feature                                                                           | Status | Notes                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
|-----------------------------------------------------------------------------------|:------:|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Data sources / tables / fields                                                    |   ✅   | connections, tables, command (SQL) tables, joins                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Parameters                                                                        |   ✅   | definitions, types, default/current values; on-panel flags, allow-custom-values, sort order. The discrete-vs-range kind is **not** decoded — the engine reports one, but the byte that stores it is unlocated                                                                                                                                                                                                                                                                 |
| Formulas                                                                          |   ✅   | bodies and references; report-level custom functions (main report only)                                                                                                                                                                                                                                                                                                                                                                                                       |
| Record / group selection formulas                                                 |   ✅   |                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Groups & sorting                                                                  |   ◐    | groups (with typed date/boolean grouping condition), sort fields, and Top N / Bottom N summary sorts decoded. WithTies is **not** decoded — no byte of the group record names it, so the model field is always `false`. DiscardOthers is read from the group record, but it is set on every corpus record, including groups with no Top N sort, so nothing witnesses the reading. The per-page group limit is decoded from the group area record, not the group record        |
| Summaries & running totals                                                        |   ✅   |                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Sections & areas                                                                  |   ✅   | with formatting and conditional formatting                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Report objects (field, text, line, box, picture, subreport)                       |   ✅   | placement, formatting, fonts, borders                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Object hyperlinks                                                                 |   ✅   | hyperlink text + type decoded from the object-format record; the renderer does not act on them — a PDF carries no link annotation                                                                                                                                                                                                                                                                                                                                             |
| Subreports & subreport links                                                      |   ✅   | including value passing between reports                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Page setup / print options                                                        |   ✅   | paper size, orientation, margins, page rectangle, multi-column detail layout (`0x006c`)                                                                                                                                                                                                                                                                                                                                                                                       |
| SQL Expression fields                                                             |   ✅   | `0x0081` — the pushed-down SQL text + name/type; listed by `rpt sql` with the generated query and stored SQL Commands                                                                                                                                                                                                                                                                                                                                                         |
| Charts / graphs                                                                   |   ◐    | object + analytic layout + data-value labels decoded, and the `0x0121` definition's styling block with them — the shape/size/colour enums, the legend layout, four gridline modes, and each of the three value/series axes' bounds, number format, auto-range/scale and divisions. A per-element *point size* is not stored anywhere in `Contents`; the OLAP/map families are still undecoded                                                                                 |
| Cross-tabs / OLAP grids                                                           |   ◐    | cross-tab records named/structured (object, dimensions, grid formats); full OLAP grid absent from corpus                                                                                                                                                                                                                                                                                                                                                                      |
| Hierarchical grouping                                                             |   ✅   | `0x00e9` group-value name + defining condition-formula decoded                                                                                                                                                                                                                                                                                                                                                                                                                |
| Maps                                                                              |   ○    | the six map records (`0x00b6`/`0x00b7` object, `0x0119`–`0x011b` definition, `0x012a` layer) are named and **do** occur in one corpus report, but no byte layout is decoded and nothing reaches the model                                                                                                                                                                                                                                                                     |
| Alerts, Flash/Xcelsius, XML/XSLT export                                           |   ○    | named at family level for recognition only; absent from corpus, decode pending a report that uses one                                                                                                                                                                                                                                                                                                                                                                         |
| Typed field sub-formats (number/date/currency/time/boolean/string masks)          |   ✅   | stored format attrs decoded (model structs populated) **and consumed by the renderer** — `rpt-layout`'s format module merges the stored format record with the render locale, arbitrated by `use_system_defaults` (see [format resolution](../rendering/05-format-resolution.md)). The engine's runtime-resolved effective display format is excluded from parity (like `NumberOfBytes`)                                                                                      |
| Formula variables (Global / Shared)                                               |   ✅   | name, result type, and scope decoded                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Designer / IDE state (rulers, guidelines, connections, history, interactive sort) |   ◐    | recognized and geometry decoded; parity-inert (no SDK read surface)                                                                                                                                                                                                                                                                                                                                                                                                           |
| Writing / editing `.rpt` files                                                    |   ◐    | a byte-faithful re-encoder ships (`Rpt::reencode`, `patch_record_field`, `patch_record_bytes`/`patch_record_bytes_resize`; the `rpt reencode`/`patch` CLI commands): the records round-trip, and one field of one decoded record can be changed by name. `Rpt::anonymize` (`rpt anonymize`) is the one *semantic* edit built on it — it strips authoring metadata. There is **no** general model→records lowering: you cannot mutate the semantic model and serialize it back |

## Lossless guarantee

Regardless of the above, every record read from a file is preserved with its exact bytes. Decoding adds typed meaning on
top; it never discards what it does not understand. The `rpt streams` command reports, per stream, how many records
carry a type the registry does not **name**, and how many logical bytes belong to no record of the linear walk. Naming
is not modelling — see [Read and modelled](#read-and-modelled).

---

← [The semantic model](01-semantic-model.md) · [Index](README.md) · **Next:** [The `rpt` CLI](03-cli.md) →
