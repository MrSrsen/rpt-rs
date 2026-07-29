# The codebase

`rpt-rs` is a Cargo workspace of 21 members in two layers: the **reader** (the load-bearing crates below) and a
**rendering & data pipeline** built on top of it. The split mirrors the decode-then-render flow and enforces two
load-bearing boundaries: **stored facts vs. derived values**, and **reader vs. render pipeline**. The latter is
compiler-enforced: the format-neutral `rpt-model` crate holds the semantic model as pure data, so every pipeline crate
depends on it and links **no decoder** (no CFB, no inflate). Only the `rpt-render` facade, whose job is to open a file,
depends on `rpt-reader`. The rendering layer has its own guided walkthrough —
[Rendering](../rendering/README.md) — covering how the pieces compose and the public API for driving a render.

**Directory layout.** The 18 library crates live under `crates/`; the three binaries live under `apps/` — `rpt-cli`
(the `rpt` inspector/exporter), `rpt-render-cli` (the `rpt-render` renderer), and the dev-only `meridian-seed` fixture
generator. Nothing in `crates/` builds a `[[bin]]`, so the publishable libraries stay cleanly separated from the
deployable tools. The tables below group members by *layer*, not by directory; the three under `apps/` are marked.

## Reader crates

| Crate         | Kind                    | Responsibility                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
|---------------|-------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `rpt-reader`  | library                 | Read the `.rpt` format, and write it back at byte level: container → decryption → records → `rpt-model`. Decodes only what is _stored_ in the bytes. Re-exports the model as `rpt_reader::model`; which record a model field is decoded from is stated at the decoder that reads it and in the field table it reads through.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `rpt-model`   | library                 | The format-neutral, pure-data **semantic model** (the L1 IR): every model DTO and enum, no I/O, WASM-safe, `serde`-optional. Produced by `rpt-reader` today (and by future non-binary readers); consumed directly by the render/data pipeline, so those crates never link the decoder.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `rpt-formula` | library                 | The Crystal/Basic **formula language** as a standalone crate: lexer, recursive-descent parser, AST, type system, and bytecode evaluator. Depends only on `rpt-format-value` (no `rpt-reader` dependency), so it is reusable outside the binary reader.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `rpt-kdl`     | library                 | Exports the semantic model to a [KDL](https://kdl.dev) document (`to_document` / `to_kdl_string`) — a hand-written, sparse, human-readable node mapping (kinds→nodes, name→first arg, scalars→properties, nesting→children, twips/`#rrggbb`/kebab-case enums, multi-line formula bodies). A **lossless** view of the model — every stored fact is emitted — kept so **by construction**: each model struct on the report/definition/layout path is destructured without `..` and each enum matched without a `_` wildcard, so a new `rpt-model` field or variant fails to compile until the exporter handles it. Binary payloads stay out of the KDL: pictures emit a `source="…"` reference and `assets` returns the bytes for sidecar files. Depends only on `rpt-model` + `kdl` — no decoder, no I/O, WASM-safe. |
| `rpt-json`    | library                 | The exhaustive JSON export surface (`export_json`) — the decode **regression** contract, callable in-process. Emits the full serde serialization of the model under `model`: every field, including defaults, and the whole subreport tree. **Stored facts only** — nothing inferred, recomputed, or reconstructed, so a change in the output always means a change in the decode. Deterministic by construction: sorted-key maps, two-space indent, trailing newline. Depends on `rpt-reader` alone; reader-side, so **not** WASM-safe by design.                                                                                                                                                                                                                                                                  |
| `rpt-cli`     | binary (`rpt`), `apps/` | The inspection/export CLI over the `rpt-reader` library: read-only inspectors, the **`rpt json-dump`** subcommand (a thin caller of `rpt-json`), the **`rpt kdl`** subcommand (KDL export via `rpt-kdl`, with picture sidecar files), and the byte-level write-path commands `anonymize` / `reencode` / `patch`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |

## Rendering & data pipeline

Built on the reader, this layer turns a decoded report + a data source into paginated, rendered output. Every crate here
is pure, WASM-safe Rust (the exceptions, `rpt-db-postgres` and `rpt-db-sqlite`, are native-only and isolated behind the
`RowSource` trait).

| Crate              | Responsibility                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
|--------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `rpt-format-value` | Value → string formatting (number / currency / date / time), a dependency-free leaf crate.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `rpt-data`         | The record pipeline: a `RowSource` → record selection → sort → grouping → summaries, plus the formula evaluation context (`Global`/`Shared` variable persistence, per-record value cache).                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `rpt-query`        | SQL generation for the live-DB path: builds a joined `SELECT` from the table/link graph, projecting only the fields the report references (`build_query_for_report` prunes unused tables/columns via `used_database_fields`) and pushing the translatable selection-formula subset into `WHERE`.                                                                                                                                                                                                                                                                                                             |
| `rpt-layout`       | The layout & pagination engine: walks the dataset, places each object, paginates band-by-band, emits the Page IR. Also the bridge between the data pipeline's diagnostics and the Page IR's, which is what keeps `rpt-data` free of an `rpt-pages` dependency.                                                                                                                                                                                                                                                                                                                                               |
| `rpt-pages`        | The backend-agnostic Page IR (`serde` draw-ops) that every renderer consumes.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `rpt-text`         | The text stack: under the default `cosmic` feature, a cosmic-text `TextLayout` (real font metrics + Unicode/CJK line-breaking); with `cosmic` off, just the fontdb-based `FontDb` face resolver the PDF backend needs. (The dependency-free `ApproxLayout` fallback lives in `rpt-pages`, beside the `TextLayout` trait, and is re-exported by `rpt-layout`.)                                                                                                                                                                                                                                                |
| `rpt-render`       | Orchestration facade (a `ReportDocument` + free functions) tying decode → data → layout → backend together. A library crate — the `rpt-render` binary lives in `rpt-render-cli`.                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `rpt-render-cli`   | The `rpt-render` binary (`[[bin]] name = "rpt-render"`, under `apps/`): resolves the five render inputs (report, datasource, locale, parameters, output) and drives the facade.                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `metafile`         | A standalone **Windows metafile parser** (EMF today; WMF/EMF+ planned): decodes a metafile's command stream into device-independent vector primitives, interpreting its own coordinate machinery (world transform, window/viewport mapping, object table, `SAVEDC`/`RESTOREDC` state stack) and handing the consumer resolved shapes through the `MetafileSink` visitor trait — never pixels. Zero dependencies and no GDI, so it is WASM-safe; `rpt-layout` is its only consumer here (EMF pictures), and it is generic enough to publish on its own (dual MIT/Apache-2.0, unlike the workspace's MPL-2.0). |
| `rpt-render-util`  | Backend-serialization helpers used by the PDF backend: the twip→point constant, text-placement math (alignment anchor, justification slack, baseline fallback), image content hashing and BMP decoding — kept out of the frozen Page IR. WASM-safe (depends only on `rpt-pages`).                                                                                                                                                                                                                                                                                                                            |
| `rpt-render-pdf`   | The only output backend, consuming the Page IR: PDF via `krilla` with real font-subset embedding. A new output target attaches to the Page IR through `PageBackend`, not as a variation on this crate.                                                                                                                                                                                                                                                                                                                                                                                                       |
| `rpt-db-postgres`  | A live PostgreSQL `RowSource` (native-only) that executes the `rpt-query` SQL.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `rpt-db-sqlite`    | A live SQLite `RowSource` (native-only); runs in-process, so it needs no server.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |

Two dev-only members round out the workspace to 21. **`rpt-test-support`** is a crate of shared test helpers
(fixture-path resolution, hand-built saved-data batches), pulled in under `[dev-dependencies]` only. **`meridian-seed`**
(under `apps/`) is a standalone binary that deterministically generates the synthetic "Meridian Global Logistics"
render-test seed database — emitting portable PostgreSQL/SQLite SQL (DDL + data) from a single fixed PRNG seed, so the
render-parity fixtures are reproducible without shipping a database dump.

### The stored-vs-derived boundary

The rule: **if a value is in the bytes, it is decoded in `rpt-reader`; if it is computed or inferred, it is computed by
the consumer that needs it, on demand.** A derived value is never stored as a field on an `rpt-model` struct, and never
enters the export. This keeps the I/O layer a faithful representation of the file, and keeps inference — which can be
wrong, or version-specific — out of the surface that is supposed to describe the file.

The export side has no derive layer at all. `json-dump` is a pure projection of the decoded model, which is what lets a
baseline diff mean "the decoder's reading of this file changed" and nothing else; a dump carrying derived values would
also move when a *derivation* changed, with the bytes untouched.

Derivations that a consumer genuinely needs live with that consumer: the render pipeline resolves display formats in
`rpt-layout`, and a formula's runtime result width is computed in `rpt-formula` where a live datasource is available. A
derivation that **more than one** consumer needs, and that requires nothing but the model, goes in
`rpt-model`'s own `analysis` module: pure functions over a decoded `Report` (today, a placed field object's effective
value type, and a cross-tab's measures — the latter moved off `CrossTabObject` as a stored field precisely to honour
this rule), with no formula engine and no decoder behind them. They are still *functions*, never stored fields — the
boundary is about where a derived value may be **written**, not about which crate may compute one.

## Inside `rpt-reader`

The library is a stack of layers, each in its own module, mirroring the decode pipeline. Today the write path is a
**byte-faithful re-encoder**: it round-trips and patches the raw records but has **no model→records lowering** — you
cannot mutate the typed model and serialize it back (see the invertibility note below).

| Module                | Responsibility                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
|-----------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `container`           | Open the CFB/OLE compound file; classify and read streams.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `codec`               | The stream header, the cipher, decompression, splitting a stream into flat records, the recursive record tree (the masking lives here), and `RecordSearch` — the format's own way of finding a record, by type within a container closed by an end marker.                                                                                                                                                                                                                                                                                                                                                                                                      |
| `records`             | One decoded stream as records: the record stream, raw records, the `RecordTag` record-type catalog the decoders match against, and the on-demand typed record tree (`Node`/`Unknown`/`Part`/`Value`/`RecordTypeCount`, re-exported as `rpt_reader::raw`). An `Unknown`'s content is its `parts` — the record's own field-byte runs and the records nested between them, in wire order. A type number means nothing on its own: it is resolved in the **`Dialect`** the stream is written in (`Contents`, `QeSession`, `Catalog`, `ReportParameters` — enumerated by `Dialect::ALL`), which is what `RecordTag::name` takes and `RecordTag::from_name` searches. |
| `bytes`               | The crate's binary-decoding vocabulary: checked scalar reads, the sequential `Cursor`, and the length-prefixed-string scanner the record decoders are built from — the last re-exported as `raw::lp_strings` / `LpString` / `LpScan`, this reader's own answer to what in a record's bytes is text.                                                                                                                                                                                                                                                                                                                                                             |
| `digest`              | Dependency-free MD5 + Base64, used to fingerprint an OLE embedding.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `field_table`         | The declarative field tables the record decoders read a record through: a table states the record's content as the *sequence* of typed fields it is — each named, with its wire type, and gated on the schema word where a version added it — so a field's position follows from what precedes it rather than from a hand-derived offset. Also carries the header a writer stamps on each type, so the write direction reproduces the version it claims. The tables themselves live in `field_table/tables/`, one module per domain, enrolled in a single registry; `parity/` sweeps every one of them over the whole corpus.                                   |
| `fields`              | **Public.** A field table's own reading of one record — each field's name, value and byte range, and whether the table consumed the record exactly. What `rpt dump` prints, and what the named-field write path addresses.                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `build_model`         | Build the typed model from the record tree (split by domain); the inverse today is byte-level (record-tree re-serialize / field-byte patch), not a model→records lowering.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `annotate`            | One record's decoded values as a concise, human-readable summary (what `rpt tree` prints). A consumer of the record decoders rather than a builder: it reads a record through the same decoders `build_model` does, and fills in no part of the model.                                                                                                                                                                                                                                                                                                                                                                                                          |
| `model` (`rpt-model`) | The typed report model (the object graph callers use) — the format-neutral `rpt-model` crate, re-exported as `rpt_reader::model`, not a module inside `rpt-reader`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `io`                  | Orchestration: ties the layers together into `Rpt::open` and exposes the report and its streams. Its `diagnose` submodule turns a failed open into an answer (what the file *is*, not "wrong magic number"); `cleared` holds the write path's cleared-for-editing allow-list.                                                                                                                                                                                                                                                                                                                                                                                   |
| `coverage`            | How completely a report decoded (`Rpt::decode_coverage`): unrecognized records and their types, bytes covered by no record, per-stream decode errors. Read off the already-decoded records, not a second pass.                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `error`               | The error type, and the shared cause-chain printer (`error_chain`) its consumers report through.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |

The **lossless record layer** is the foundation: `container`, `codec` and `records` round-trip every record
byte-identically, including records that are not yet understood. The typed model is built on top and can grow without
ever risking the round-trip.

Note what `build_model` does *not* do: **building the model is infallible by design.** It returns defaults for anything
it cannot interpret rather than failing, so a report the Crystal engine opens is never refused over a record this reader
does not model — the right trade for a reader of an undocumented format. The cost is that an incomplete decode raises no
error and would otherwise be invisible, so it is reported as a *diagnostic* instead: that is what the `coverage` module
is for, and why `rpt_reader::Error` has no "could not interpret this record" variant.

The record layer is also **invertible**. `RecordStream::serialize_tree` rebuilds a stream's logical (inflated) bytes
from its record tree, and the `codec` layer runs the write pipeline (re-serialize → deflate → AES-CFB encrypt with the
stored header IV → CFB rewrite, other streams verbatim). Four entry points sit on top:

- `Rpt::reencode` produces a valid `.rpt` that re-opens to byte-identical logical bytes (deflate is non-canonical, so
  only the inflated level round-trips) — the no-op writer proof.
- `Rpt::patch_record_field` stores a value in a **named field** of one record, and is the form to reach for: the record
  type's field table says where the field's bytes are and how wide they are, so the caller states a name and a value
  rather than an offset that moves with the string before it. A value of a different width is written, and the record's
  own length prefix and every enclosing record's absorb the delta. It refuses (writing nothing) unless the table
  reproduces the record byte for byte and the written record reads back with that field changed and no other.
- `Rpt::patch_record_bytes` overwrites an **equal-length** region of a record's field bytes — its runs joined, children
  spliced out — for a record type that has no field table.
- `Rpt::patch_record_bytes_resize` replaces such a region with bytes of **any** length, recomputing the same length
  prefixes. Because the `Contents` tree holds no absolute byte offsets, nothing else needs fixing; it errors (writing
  nothing) if the region straddles a nested child or a recomputed prefix would overflow its on-disk field width.

There is still no model→bytes path; edits are made against one decoded record at a time.

### `rpt-model` submodules

The semantic model lives in the standalone `rpt-model` crate (re-exported as `rpt_reader::model`), organized internally
by domain: `document` (the top-level `Report` and summary info), `database` (connections, tables, fields, links),
`data_def`
(parameters, formulas, groups, sorts, summaries), `report_def` (areas, sections, objects), `objects` (the report object
kinds), `format` (object and section formatting), `enums` (the SDK-style enumerations), `primitives` (shared value types
like `Twips`, `Color`, `Rect`, `Conditioned`), `saved` (the decoded saved-data model — cached columns and rows), `fit`
(the integer-code → model-enum conversions the readers use to decode low-level codes), `analysis` (the dependency-free
derived helpers shared by the export and render layers, above) and the `hex_bytes` serde helper. These modules are
**private**: every type is re-exported flat at the crate root, so callers write `rpt_reader::model::Report`, never
`rpt_reader::model::document::Report`. The generic record-tree view (`Node`/`Value`/`Unknown`) and the `RecordTag`
registry are **not** part of the neutral model — they live in the reader (`rpt_reader::raw`) and are built on demand via
`Rpt::typed_record_tree()`/`Rpt::inventory()`.

### `build_model` submodules

The model-building code is organized to match the model: `document`, `database`, `data_def` (with `fields` and
`groups`), `report_def` (with `walk` — the ordered record walk that builds the layout tree — plus `sections`,
`objects`, `conditions`, `data_source`, `formats`, `chart`, `crosstab`, `bindings`, and `summary`), `parameters`,
`print_options`, `picture`, `embeds`, `saved` and `saved_catalog` (the saved-data catalog and its rowset),
`subreport` (decoding each `Subdocument N` stream), `typed_tree` (builds the on-demand typed record tree and the
record-type inventory), and the shared helpers: `tree_search` (finding records and their strings in the tree)
and `record_values` (reading colors, coordinates and stored tokens out of a record's field bytes). No module here
addresses bytes itself: every record is read through its own field table, which says where each named field sits — the
layouts documented in the [block catalog](../format/06-block-catalog.md) are those tables.

## The `rpt-formula` crate

The Crystal/Basic formula language — lexer, parser, AST, type system, and bytecode evaluator — is a **standalone
crate**, independent of the report reader. The rationale:

- **It is genuinely independent of the `.rpt` binary container.** A formula body is a text language; parsing and
  evaluating it has nothing to do with the CFB/OLE2 file layout, so it belongs behind its own crate boundary.
- **It has no dependency on the `rpt-reader` decoder** — only on `rpt-format-value` (a dependency-free leaf, needed
  because a
  `Value` carries `Date`/`Time`). So it can be reused without pulling in the whole binary decoder: a formula language
  server, a WASM formula sandbox, and a standalone validator/playground can all depend on just `rpt-formula`.
- **Cross-boundary type mappings stay with their consumers.** `rpt-formula` exposes its own `ResultKind`; any code that
  needs to relate a formula's result kind to the `rpt-model` `FieldValueType` does so in the consumer that knows both
  types, never by coupling the formula crate to the model.

Every consumer that needs the formula engine (`rpt-data`, `rpt-layout`, `rpt-query`, `rpt-db-sqlite`, `rpt-render`,
`rpt-cli`, `rpt-render-cli`) depends on `rpt-formula` directly.

## The binaries

- **`rpt-cli`** (`rpt`) is an inspector with thirteen subcommands (`inspect` / `inputs` / `tree` / `streams` / `dump` /
  `saved` / `sql` / `formulas` / `json-dump` / `kdl` / `anonymize` / `reencode` / `patch`) and a `--json` flag. Ten are
  read-only: `dump` is the byte-layout workbench for a record's raw bytes — annotated hex of the record's own demasked
  field bytes, followed by either the record type's field-table reading (every field's name, value and byte range, and
  whether the table consumed the record exactly) or, for a type with no table, the scalar-probe grid, plus the
  minimal-pair byte diff and a `--glob` corpus sweep;
  `saved` decodes the saved-data rows (schema + cached rowset); `sql` lists every SQL the report can issue (the
  generated join query + stored SQL Commands + SQL Expression fields, recursively through subreports, with
  connection/table provenance — via `rpt-query`, no DB connection made); **`formulas`** parses and semantically
  validates every formula the report defines (formula fields, record/group selection, conditional-format formulas,
  recursively through subreports) and exits nonzero on any error, so a broken formula is *rejected* somewhere rather
  than silently evaluating from a recovery AST; **`json-dump`**
  emits the exhaustive, deterministic JSON dump of the decoded model — the project's regression surface; **`kdl`**
  exports the model to a KDL document (via `rpt-kdl`), writing each embedded picture as a sidecar file when an output
  path is given. The three write-path commands rest on the byte-faithful re-encoder: `reencode`
  round-trips the `Contents` stream to a fresh `.rpt` (a no-op writer proof), `patch` changes one **named field** of one
  record and writes a new `.rpt` (there is no model→records lowering — see the invertibility note above): the record
  type's field table says where the field's bytes are and how wide they are, and a value of a different width is written
  with every enclosing length prefix recomputed. It **refuses**, writing nothing, unless the table reproduces that
  record byte for byte *and* the written record reads back with that field changed and no other — evidence the record
  itself supplies, per record. A raw `@<offset>` region edit has neither property available, so it is same-size only and
  falls back to the cleared-for-editing allow-list in `crates/rpt-reader/src/io/cleared.rs`. `--force` skips every
  check. **`anonymize`** strips authoring metadata — the author and last-saver in the
  `SummaryInformation` property set, and a re-imported subreport's source path (reduced to its file name so
  `IsImported` survives). Its edits are all same-length, so no record or property offset moves and the decoded model is
  unchanged apart from those fields. See [Usage](../reader/03-cli.md). The binary is a thin driver: the CLI surface is
  declared in
  `args` and dispatched from `main`, each subcommand has its own module under `apps/rpt-cli/src/` (bar `patch`, which
  shares the re-encoder's), and the JSON and KDL exports are delegated to the `rpt-json` and `rpt-kdl` libraries rather
  than implemented in the app.
  `rpt <command> --help` prints scoped help.
- **`rpt-render`** renders a report end-to-end to PDF, from saved data or a live database (`--db`), with `--param`,
  `--locale`, and file-or-stdout output. See [Rendering](../rendering/README.md) for the pipeline and the live-DB path,
  and [Render examples](../rendering/10-examples.md) for driving the facade from code.

## Conventions

- The whole workspace forbids `unsafe` code.
- The minimum supported Rust version is 1.92. The floor moves when a dependency we want requires it, and for no other
  reason — 1.92 is what `krilla` 0.8.2 needs. State the reason whenever it changes, so the number stays justified rather
  than merely inherited.
- Dependencies are deliberately minimal: the CFB container, a zlib inflater, an error derive, and serde for the JSON
  export and the CLI's `--json` output. The cipher is implemented in-crate with no cryptography dependency.
- Every shared dependency is declared once in the root `[workspace.dependencies]` and inherited by members with
  `{ workspace = true }` (features and `optional` stay at the leaf). `scripts/check-workspace-deps.sh` enforces this in
  CI: a bare per-crate version can silently drift into a second compiled copy of the same crate.
- Workspace lints are strict: `unsafe_code = "forbid"`, `missing_docs = "deny"`, and
  `rustdoc::broken_intra_doc_links = "deny"`, so every public item is documented and every doc link resolves. On top of
  those, `clippy::missing_errors_doc` and `clippy::missing_panics_doc` require a public fallible function to say what
  can fail and a public panicking one to say when. The lints deliberately *not* enabled (`unwrap_used`, `expect_used`,
  `let_underscore_must_use`, `let_underscore_untyped`) are listed with their measured hit counts and the reasoning in
  the `[workspace.lints.clippy]` block — the decision is recorded where someone hits the question.
- `scripts/check-error-handling.sh` (also CI) guards two error-handling invariants clippy cannot express: formula parse
  diagnostics discarded in production code, and an error variant that both interpolates `{0}` and marks it
  `#[from]`/`#[source]` (which makes every chain-printing reporter emit the cause twice).

## Feature flags

Most crates have none. The build knobs that exist:

| Crate                                         | Feature                                   | Effect                                                                                                                             |
|-----------------------------------------------|-------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------|
| `rpt-text`                                    | `cosmic` (default)                        | The cosmic-text shaping stack (`CosmicLayout` + `FontProvider`); off leaves only the `FontDb` resolver the physical backends need. |
| `rpt-render-cli`                              | `db-postgres`, `db-sqlite` (both default) | The live-DB backends behind `--db` live **here**, not in the `rpt-render` library — the facade never links a driver.               |
| `rpt-reader`, `rpt-model`, `rpt-format-value` | `serde`                                   | Derive `Serialize`/`Deserialize` on the model and its value types.                                                                 |
| `rpt-formula`                                 | `differential`                            | Exposes the tree-walking `Evaluator` (the VM's differential-test reference) to external test crates.                               |

---

[Index](README.md) · **Next:** [Building](building.md) →
