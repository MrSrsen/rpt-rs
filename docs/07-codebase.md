# The codebase

`rpt-rs` is a Cargo workspace of 20 crates in two layers: the **reader** (the load-bearing crates below) and a
**rendering & data pipeline** built on top of it. The split mirrors the decode-then-render flow and enforces two
load-bearing boundaries: **stored facts vs. derived analytics**, and **reader vs. render pipeline**. The latter is
compiler-enforced: the format-neutral `rpt-model` crate holds the semantic model as pure data, so the render stack
depends on it and links **no decoder** (no CFB, no inflate). The rendering layer has its own guided walkthrough —
[Rendering](11-rendering.md) — covering how the pieces compose and the public API for driving a render.

## Reader crates

| Crate        | Kind           | Responsibility                                                                                                                                         |
| ------------ | -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `rpt`        | library        | Read (and eventually write) the `.rpt` format: container → decryption → records → `rpt-model`. Decodes only what is _stored_ in the bytes. Re-exports the model as `rpt::model`; the byte-level provenance of the model's fields is documented in `rpt::provenance`. |
| `rpt-model`  | library        | The format-neutral, pure-data **semantic model** (the L1 IR): every model DTO and enum, no I/O, WASM-safe, `serde`-optional. Produced by `rpt` today (and by future non-binary readers); consumed directly by the render/data pipeline, so those crates never link the decoder. |
| `crystal-formula` | library   | The Crystal/Basic **formula language** as a standalone crate: lexer, recursive-descent parser, AST, type system, and bytecode evaluator. Depends only on `rpt-format-value` (no `rpt` decoder dependency), so it is reusable outside the binary reader. |
| `rpt-cli`    | binary (`rpt`) | The inspection/export CLI over the `rpt` library: read-only inspectors, the **`rpt xml-dump`** subcommand (serializes the model — plus its own derived analytics: field use counts, parameter usage, summary fields, effective field formats — to a structured XML document), and the byte-level write-path commands `reencode` / `patch`. |

## Rendering & data pipeline

Built on the reader, this layer turns a decoded report + a data source into paginated, rendered output. Every crate here
is pure, WASM-safe Rust (the exceptions, `rpt-db-postgres` and `rpt-db-sqlite`, are native-only and isolated behind the `RowSource` trait).

| Crate               | Responsibility                                                                                                    |
| ------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `rpt-format-value`  | Value → string formatting (number / currency / date / time), a dependency-free leaf crate.                        |
| `rpt-data`          | The record pipeline: a `RowSource` → record selection → sort → grouping → summaries, plus the formula evaluation context (`Global`/`Shared` variable persistence, per-record value cache). |
| `rpt-query`         | SQL generation for the live-DB path: builds a joined `SELECT` from the table/link graph, projecting only the fields the report references (`build_query_for_report` prunes unused tables/columns via `used_database_fields`) and pushing the translatable selection-formula subset into `WHERE`. |
| `rpt-layout`        | The layout & pagination engine: walks the dataset, places each object, paginates band-by-band, emits the Page IR. |
| `rpt-pages`         | The backend-agnostic Page IR (`serde` draw-ops) that every renderer consumes.                                     |
| `rpt-text`          | The text stack: under the default `cosmic` feature, a cosmic-text `TextLayout` (real font metrics + Unicode/CJK line-breaking); with `cosmic` off, just the fontdb-based `FontDb` face resolver the PDF/raster backends need. (The dependency-free `ApproxLayout` fallback lives in `rpt-layout`.) |
| `rpt-render`        | Orchestration facade (a `ReportDocument` + free functions) tying decode → data → layout → backend together. A library crate — the `rpt-render` binary lives in `rpt-render-cli`. |
| `rpt-render-cli`    | The `rpt-render` binary (`[[bin]] name = "rpt-render"`): resolves the five render inputs (report, datasource, locale, parameters, output) and drives the facade. |
| `rpt-render-util`   | Backend-serialization helpers shared by the four backends and the layout engine: twip↔unit constants, XML/HTML text escaping, stroke dash-pattern math — kept out of the frozen Page IR. WASM-safe (depends only on `rpt-pages`). |
| `rpt-render-html` / `-svg` / `-pdf` / `-raster` | The four output backends, each consuming the Page IR: HTML (RAS-shaped XHTML, images inlined as `data:` URIs), SVG (one file per page), PDF (`krilla` with real font-subset embedding, plus a zero-dep fallback writer), and raster (`tiny-skia` + `fontdue` → PNG per page). |
| `rpt-db-postgres`   | A live PostgreSQL `RowSource` (native-only) that executes the `rpt-query` SQL.                                     |
| `rpt-db-sqlite`     | A live SQLite `RowSource` (native-only); the zero-process CI / localhost data path.                                |

One supporting crate rounds out the workspace to 20: **`rpt-test-support`** is a dev-only crate of shared test helpers
(fixture-path resolution, hand-built saved-data batches), pulled in under `[dev-dependencies]` only.

### The stored-vs-derived boundary

The rule: **if a value is in the bytes, it is decoded in `rpt`; if it is computed or inferred, it lives in the derive
layer.** A derived value is never stored as a field on a core `rpt` model struct. This keeps the I/O layer a faithful
representation of the file and isolates inference (which can be wrong, or version-specific) in the consumer that needs
it. The canonical example is a field's use count: it is not stored in the file, so `rpt-cli` computes it (in its
private `export::analysis` module) by walking the model. The boundary is preserved even though the derivation now lives
inside the exporter — the derived values are still never written back onto an `rpt` model struct.

## Inside `rpt`

The library is a stack of layers, each in its own module, mirroring the decode pipeline. Today the write path is a
**phase-1 byte-faithful re-encoder**: it round-trips and byte-patches the raw record substrate but has **no
model→records lowering** — you cannot mutate the typed model and serialize it back (see the invertibility note below).

| Module                 | Layer   | Responsibility                                                                                                      |
| ---------------------- | ------- | ------------------------------------------------------------------------------------------------------------------- |
| `container`            | L0      | Open the CFB/OLE compound file; classify and read streams.                                                          |
| `codec`                | L0.5–L1 | The stream header, the cipher, decompression, record tiling and the recursive record tree (the masking lives here). |
| `records`              | L1      | The record model: the typed record stream, raw records, and the by-name record-type catalog the decoders match against (the `RecordTag` type itself is defined in `rpt-model`). |
| `bytes`                | L1–L2   | The crate's binary-decoding vocabulary: checked scalar reads, the sequential `Cursor`, and the length-prefixed-string scanner the `raise` decoders are built from. |
| `project`              | L2      | Raise the record tree into the typed model (`project::raise`, split by domain); the inverse today is byte-level (record-tree re-serialize / leaf patch), not a model→records lowering. |
| `model` (`rpt-model`)  | L3      | The typed report model (the object graph callers use) — the format-neutral `rpt-model` crate, re-exported as `rpt::model`, not a module inside `rpt`. |
| `provenance`           | —       | Public documentation module: the byte-level provenance of each model field (which `Contents` record it decodes from and its leaf layout), kept out of the format-neutral model. |
| `io`                   | —       | Orchestration: ties the layers together into `Rpt::open` and exposes the report and its streams.                    |
| `error`, `diagnostics` | —       | The error type, and the crash/backtrace hook used by the binaries.                                                  |

The **lossless substrate** is the foundation: layers L0–L1 round-trip every record byte-identically, including records
that are not yet understood. The typed model (L3) is a projection on top that can grow without ever risking the
round-trip.

The substrate is also **invertible** at the record level. `RecordStream::serialize_tree` rebuilds a stream's logical
(inflated) bytes from its record tree, and the `codec` layer runs the L1→L0 write pipeline (re-serialize → deflate →
AES-CFB encrypt with the stored header IV → CFB rewrite, other streams verbatim). `Rpt::reencode` produces a valid
`.rpt` that re-opens to byte-identical logical bytes (deflate is non-canonical, so only the inflated level round-trips),
and `Rpt::patch_record_leaf` changes an equal-length region of a decoded record's leaf and writes a new file — a
same-size-only writer (length-changing edits, which need a record-length recompute, are not yet supported). There is
still no model→bytes path; edits are made against the raw record tree.

### `rpt-model` submodules

The semantic model lives in the standalone `rpt-model` crate (re-exported as `rpt::model`), split by domain: `document`
(the top-level `Report` and summary info), `database` (connections, tables, fields, links), `data_def` (parameters,
formulas, groups, sorts, summaries), `report_def` (areas, sections, objects), `objects` (the report object kinds),
`format` (object and section formatting), `enums` (the SDK-style enumerations), `primitives` (shared value types like
`Twips`, `Color`, `Rect`, `Conditioned`), `dom` (the generic record-tree view: `Node`, `Value`, `Unknown`), `saved` (the
decoded saved-data model — cached columns and rows), `tag` (the `RecordTag` record-type registry: numeric ↔ symbolic
names), and `fit` (the integer-code → model-enum conversions the readers use to raise low-level codes).

### `project::raise` submodules

The projection code is organized to match the model: `database`, `data_def`, `report_def` (with `sections`, `objects`,
`conditions`, `data_source`, `formats`, `chart`, `crosstab`, `grid`, and `summary`), `parameters`, `print_options`,
`subreport` (raising each `Subdocument N` stream), `dom` (the generic record-tree view), and shared helpers in `common`.
This is where record bytes are interpreted into typed elements — the layouts documented in the
[block catalog](06-block-catalog.md) are implemented here.

## Derived analytics (inside `rpt-cli`)

The derived analytics — use-count computation (which fields and formulas are referenced, and how often), parameter
usage flags, the `<SummaryFields>` list, and the runtime-resolved effective field formats — live in `rpt-cli`'s
private `export::analysis` module. They are computed from a decoded `Report` and combined with the stored facts when the
XML is written; the `rpt xml-dump` export is their only consumer. The formula language they depend on lives in the
standalone [`crystal-formula`](#the-crystal-formula-crate) crate.

## The `crystal-formula` crate

The Crystal/Basic formula language — lexer, parser, AST, type system, and bytecode evaluator — is a **standalone crate**,
independent of the report reader. The rationale:

- **It is genuinely independent of the `.rpt` binary container.** A formula body is a text language; parsing and
  evaluating it has nothing to do with the CFB/OLE2 file layout, so it belongs behind its own crate boundary.
- **It has no dependency on the `rpt` decoder** — only on `rpt-format-value` (a dependency-free leaf, needed because a
  `Value` carries `Date`/`Time`). So it can be reused without pulling in the whole binary decoder: the planned Crystal
  LSP server, a WASM formula sandbox, and a standalone validator/playground can all depend on just `crystal-formula`.
- **Cross-boundary type mappings stay with their consumers.** `crystal-formula` exposes its own `ResultKind`; any code
  that needs to relate a formula's result kind to the `rpt` model's `FieldValueType` does so in the consumer that knows
  both types, never by coupling the formula crate to the model.

Every consumer that needs the formula engine (`rpt-data`, `rpt-layout`, `rpt-query`, `rpt-db-sqlite`, `rpt-render`,
`rpt-cli`) depends on `crystal-formula` directly.

## The binaries

- **`rpt-cli`** (`rpt`) is an inspector with ten subcommands (`inspect` / `inputs` / `tree` / `streams` / `dump` /
  `saved` / `sql` / `xml-dump` / `reencode` / `patch`) and a `--json` flag. Eight are read-only: `dump` is the byte-layout
  workbench for reverse-engineering a record (annotated hex of demasked leaf bytes + scalar probe + minimal-pair diff);
  `saved` decodes the saved-data rows (schema + cached rowset); `sql` lists every SQL the report can issue (the generated
  join query + stored SQL Commands + SQL Expression fields, recursively through subreports, with connection/table
  provenance — via `rpt-query`, no DB connection made); **`xml-dump`** walks the model (plus its own derived
  analytics) and emits the RptToXml-compatible XML — a default mode (the modelled report) and a `--full` mode that also
  dumps the complete record tree. The two write-path commands exercise the byte-faithful re-encoder: `reencode`
  round-trips the `Contents` stream to a fresh `.rpt` (a no-op writer proof), and `patch` overwrites a same-size region
  of one decoded record's demasked leaf and writes a new `.rpt` (there is no model→records lowering — see the
  invertibility note above). See [Usage](08-usage.md). Each subcommand lives in its own module under
  `crates/rpt-cli/src/` (the exporter under `export/`, the write path in `reencode`); `rpt <command> --help` prints
  scoped help.
- **`rpt-render`** renders a report end-to-end to HTML / SVG / PDF / PNG, from saved data or a live database (`--db`),
  with `--param`, `--locale`, and file-or-stdout output. See [Rendering](11-rendering.md) for the pipeline and the
  live-DB path, and [Render examples](12-render-examples.md) for driving the facade from code.

## Conventions

- The `rpt` crate forbids `unsafe` code.
- The minimum supported Rust version is 1.89.
- Dependencies are deliberately minimal: the CFB container, a zlib inflater, an XML writer, an error derive, and serde
  for the CLI's JSON output. The cipher is implemented in-crate with no cryptography dependency.
