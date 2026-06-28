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

| Stream                                   | Status | Notes                                           |
| ---------------------------------------- | :----: | ----------------------------------------------- |
| `Contents`                               |   ✅   | the report definition                           |
| `QESession`                              |   ✅   | connections, tables, fields, joins, SQL         |
| `SummaryInformation`                     |   ✅   | OLE property set (title/author/timestamps/app)  |
| `PromptManager`                          |   ◐    | parameter prompting metadata used during decode |
| `ReportInfo`                             |   ○    | fixed-size header, not modelled                 |
| `Embedding N` / `CHART N` / `zlibBLOB N` |   ○    | embedded payloads, not modelled                 |

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
| `0x00e5` | Group                  | `0x00ff` | SectionConditionFormulas |
| `0x0100` | FontColor              | `0x0101` | FontConditionFormulas    |
| `0x0106` | SubReportLink          | `0x0166` | FieldHeadingLink         |
| `0x018e` | PaperRect              |          |                          |

See the [block catalog](06-block-catalog.md) for each one's meaning and layout.

### Recognized but not decoded

These types appear in real reports and are preserved verbatim, but are not yet interpreted into the model:

```
0x0000 0x0001 0x0005 0x0009 0x006c 0x006e 0x006f 0x0077 0x0079 0x007f 0x0082 0x0084
0x0086 0x0088 0x008d 0x008f 0x0091 0x0093 0x0095 0x0097 0x0099 0x009b 0x009c 0x009d
0x00aa 0x00ac 0x00af 0x00b1 0x00b3 0x00b4 0x00bd 0x00ca 0x00e7 0x00e9 0x00ed 0x00ee
0x00ef 0x00f0 0x00f1 0x00f2 0x00f3 0x00f4 0x00f5 0x00f6 0x00f7 0x00f8 0x00f9 0x00fa
0x00fb 0x0103 0x0104 0x0107 0x0108 0x010a 0x010c 0x010d 0x010f 0x0111 0x0112 0x0116
0x0118 0x011c 0x011f 0x0120 0x0121 0x0126 0x0127 0x0128 0x013f 0x0140 0x0142 0x015f
0x0160 0x0165 0x016a 0x016d 0x0178 0x0179 0x017b 0x0189 0x018b 0x018d
```

(`0x00b1` and `0x00b4` are partially used during decode — the blob-field wrapper and chart data block — but are not
fully modelled.)

## Feature areas

| Feature                                                     | Status | Notes                                                        |
| ----------------------------------------------------------- | :----: | ------------------------------------------------------------ |
| Data sources / tables / fields                              |   ✅   | connections, tables, command (SQL) tables, joins             |
| Parameters                                                  |   ✅   | definitions, types, default/current values                   |
| Formulas                                                    |   ✅   | bodies and references                                        |
| Record / group selection formulas                           |   ✅   |                                                              |
| Groups & sorting                                            |   ◐    | groups and sort fields decoded; some group options pending   |
| Summaries & running totals                                  |   ✅   |                                                              |
| Sections & areas                                            |   ✅   | with formatting and conditional formatting                   |
| Report objects (field, text, line, box, picture, subreport) |   ✅   | placement, formatting, fonts, borders                        |
| Subreports & subreport links                                |   ✅   | including value passing between reports                      |
| Page setup / print options                                  |   ✅   | paper size, orientation, margins, page rectangle             |
| Charts / graphs                                             |   ◐    | placeholder object only; chart data model not decoded        |
| Cross-tabs / OLAP grids                                     |   ○    | not modelled                                                 |
| Maps, alerts, hierarchical grouping                         |   ○    | not modelled                                                 |
| Typed field sub-formats (number/date/currency masks)        |   ○    | not modelled                                                 |
| Writing / editing `.rpt` files                              |   ○    | the substrate round-trips; a public write API is future work |

## Lossless guarantee

Regardless of the above, every record read from a file is preserved with its exact bytes. Decoding adds typed meaning on
top; it never discards what it does not understand. The `rpt streams` command reports, per stream, how many records
remain undecoded for a given file.
