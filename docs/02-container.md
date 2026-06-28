# The container

A `.rpt` file is a **Microsoft Compound File Binary (CFB/OLE2)** — the same structured-storage container used by legacy
Microsoft Office documents (`.doc`, `.xls`). It is a well-documented, general-purpose format ([MS-CFB]), independent of
anything Crystal-specific. `rpt-rs` reads it with the [`cfb`] crate, so enumerating, extracting, and replacing whole
streams is a solved problem.

Every `.rpt` begins with the CFB magic:

```
D0 CF 11 E0 A1 B1 1A E1
```

## Streams and storages

A compound file is a tree. Its leaves are **streams** (byte blobs, like files) and its internal nodes are **storages**
(like directories). A `.rpt` uses this tree to hold the report definition plus everything attached to it.

### Report streams

These streams carry the report itself. The big ones (`Contents`, `QESession`, `PromptManager`) are encrypted and
compressed; see [Stream decoding](03-stream-decoding.md).

| Stream               | What it holds                                                                                                                                                                                                          |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Contents`           | **The report definition** — the layout, formulas, groups, sections, and report objects. The primary target of decoding.                                                                                                |
| `QESession`          | The "Query Engine" session: data-source connections, tables, fields, joins, and SQL commands.                                                                                                                          |
| `PromptManager`      | Parameter prompting definitions, stored as an XML object set.                                                                                                                                                          |
| `ReportInfo`         | A small fixed-size header/flags block.                                                                                                                                                                                 |
| `SummaryInformation` | A standard OLE property set: title, author, revision, timestamps, and the creating application. Large values include an embedded preview thumbnail. Parseable with [MS-OLEPS] without touching the proprietary layers. |

### Subreports

A **subreport** is a complete nested report. It lives in its own storage named `Subdocument N`, with its own `Contents`,
`QESession`, and `PromptManager` streams. Because a subreport is just another report, the same decoder recurses into it
directly. A report's subreport count tracks its complexity.

### Embedded objects and other entries

| Entry                                                   | What it is                                                                                               |
| ------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `Embedding N` (with inner `CompObj`, `Ole`, `CONTENTS`) | An embedded OLE object — an image, logo, or chart. The inner `CONTENTS` is often itself a nested format. |
| `CHART N`                                               | A chart definition stream.                                                                               |
| `zlibBLOB N`                                            | A zlib-compressed payload.                                                                               |
| `CrystalReportDesignerStream`                           | Designer-only metadata (not always present).                                                             |
| `ExportFormatOptionsStream N`                           | Saved export options.                                                                                    |

## How `rpt-rs` uses the container

The library opens the compound file, identifies each entry by name (a fixed classification: any unrecognized entry still
has a stable identity), and hands the report streams to the stream decoder. Streams the library does not model are still
enumerable — the [`rpt streams`](08-usage.md) command reports every stream and its record coverage.

The next layer down is [Stream decoding](03-stream-decoding.md): turning an encrypted, compressed stream into a flat
sequence of records.

[MS-CFB]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cfb/
[MS-OLEPS]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-oleps/
[`cfb`]: https://crates.io/crates/cfb
