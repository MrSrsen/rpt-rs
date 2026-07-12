# Changelog

All notable changes to rpt-rs will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),

## [Unreleased]

TBD.

## [0.2.0]

This release turns `rpt-rs` from a reader/exporter into a full reporting engine: a complete render pipeline
(data → layout → Page IR → HTML/SVG/PDF/PNG), a formula evaluator, live-database support, saved-data decoding,
a byte-faithful writer, and the test corpus to validate it all against the native Crystal engine.

### Added

- **End-to-end report rendering.** A new render & data pipeline built purely on the decoded model:
  - **`rpt-data`** — the record pipeline: `RowSource` → record selection → sort → grouping → summaries → running
    totals, with the formula evaluation context (`Global`/`Shared` variables, per-record cache, evaluation-time
    scheduling). Two-field summaries (`WeightedAverage`, `Correlation`, `Covariance`) resolve to an
    empty/unavailable value rather than a plausible-but-wrong one until the second field is decoded.
  - **`rpt-layout`** — the layout & pagination engine: places every object at its twip position, paginates
    band-by-band, and honours the section-break controls (New Page Before/After, Keep Group Together, Print at
    Bottom of Page, Reset Page Number After, Underlay Following Sections). Resolves each field's display format
    from the locale + its stored format spec.
  - **`rpt-pages`** — the backend-agnostic Page IR: `Rect` / `Ellipse` / `Line` / `Text` / `Polygon` / `Image`
    draw-ops in twips, solid/gradient/hatch fills, rotated text runs, image assets, checkpoints, and fidelity
    diagnostics. `serde`-serializable — the frozen contract between layout and backends.
  - **Four output backends** — `rpt-render-html` (self-contained XHTML, images inlined), `rpt-render-svg` (one file
    per page), `rpt-render-pdf` (krilla with real font-subset embedding, plus a dependency-free fallback writer),
    and `rpt-render-raster` (tiny-skia → PNG per page).
  - **`rpt-text`** — the real text stack (cosmic-text): font metrics, Unicode/CJK line breaking, bidi, and font
    fallback behind a swappable `TextLayout` trait, with bundled Liberation fonts and a dependency-free
    `ApproxLayout` for deterministic output.
  - **`rpt-render`** — the orchestration facade: `ReportDocument` (load → inspect → `to_pdf`/`to_html`/…) and an
    options-driven `render_with(report, RenderOptions)` threading the datasource (`Saved`/`Rows`/`Dataset`),
    parameters, locale, and subreport scope, with typed `RenderError`s.
  - **The `rpt-render` CLI** — renders a report to HTML / PDF / SVG / PNG from its saved data or a live database
    (`--db`), with `--param`, `--locale`, format inference from the output extension, and stdout piping.
  - The pipeline up to the Page IR plus the HTML and SVG backends compile to **WebAssembly**; a CI job guards the
    boundary.

- **Chart rendering — 16 chart types as native vector draw-ops** (no rasterization): bar (clustered / stacked /
  percent, multi-series), line, area, pie, doughnut, 3-D riser, 3-D surface, 3-D area, scatter, bubble, stock
  (hi-lo / OHLC), histogram, radar, gauge, funnel, and numeric-axis, plus a bar fallback (with a diagnostic) for
  types without a dedicated renderer. Matches the native engine's defaults: the full 20-colour palette, axis
  titles, tick density, compact temporal category labels with period bucketing (weekly/monthly/quarterly/…), label
  thinning on dense axes, family-dependent legend rules, and a perspective 3-D scene (corner room, floor grid,
  painter-sorted risers). Per-axis gridline modes are decoded from the chart styling record.

- **Cross-tab rendering.** A cross-tab pivots the dataset by row × column dimensions with an aggregate measure per
  cell and draws a native grid (cell rects, grid lines, headers, grand totals). Current cut: one row dimension ×
  one column dimension × the first measure.

- **Live database support.** `rpt-query` generates the joined `SELECT` from the report's table/link graph —
  projecting only the tables and columns the report actually uses (matching the native engine's SQL and avoiding
  accidental cartesian joins) and pushing the translatable record-selection subset into `WHERE`. `rpt-db-postgres`
  and `rpt-db-sqlite` implement `RowSource` (native-only, isolated behind the trait so the portable core links no
  driver). Connection URLs are read only from the environment (`RPT_DB_URL`, `DATABASE_URL`,
  `RPT_DB_URL_<SERVER>`), never from flags.

- **Formula evaluation.** The `crystal-formula` engine gained a full evaluator: a bytecode VM (with a tree-walking
  reference evaluator), the builtin library across every family — string, math, conversion, date/time (incl.
  `DatePart` week-numbering modes), financial (`Pmt`/`FV`/`PV`/`NPV`/`IRR`/`Rate`/`DDB`/`SLN`/`SYD`), statistical
  (sample + population), and numeral (`ToWords`, `Roman`) — plus loop `Exit`, textual `#Month d, yyyy#` date
  literals, and a **semantic validation pass** (`validate`/`validate_str`): unknown/misspelled builtins with
  suggestions, arity and operator-type errors, and unknown field/parameter/formula references, as spanned,
  severity-tagged diagnostics.

- **Saved-data decoding.** A report saved with data now yields its schema (column names + types) and cached rows
  (`Report::saved_data`, the `rpt saved` subcommand) — both the external-memo and inline packed batch layouts
  decode, including the per-batch encryption and the memo heap.

- **`.rpt` writer (byte-faithful).** The decode pipeline is invertible at the record-substrate level:
  `Rpt::reencode` re-serializes, deflates, encrypts, and rewrites a valid `.rpt` that re-opens byte-identically at
  the logical level, and `Rpt::patch_record_leaf` overwrites a same-size region of a decoded record's leaf. Exposed
  as the `rpt reencode` / `rpt patch` subcommands. There is no model→records lowering yet.

- **New decoders in the reader:** charts (bindings, analytic layout, data-value labels, gridline modes),
  cross-tabs (dimensions, grid formats), object hyperlinks, hierarchical grouping, formula variables
  (name/type/scope), typed field sub-formats (number / currency / date / time / boolean / string masks), subreport
  re-import metadata, save metadata, designer state (rulers, guidelines), and the field-manager census. Every
  record type observed in the corpus is now named in the registry. `rpt` decode errors carry structured context
  (stream, byte offset, record type) via dedicated error types instead of message strings.

- **CLI additions.** `rpt inspect` shows each chart's data binding ("show value(s)" / "on change of");
  `rpt tree` prints the colorized decoded record DOM; `rpt streams` reports per-stream decode coverage;
  `rpt dump` is a byte-layout workbench (annotated hex, string scan, scalar probe, minimal-pair diff);
  `rpt saved` prints the decoded saved rows; `rpt sql` lists every SQL a report can run against its database
  (the generated join query + stored SQL Commands + SQL Expression fields, recursively through subreports, with
  connection/table provenance and a `--dialect` selector); `Report::objects()` / `objects_mut()` iterate all
  report objects.

- **Render-parity test corpus and infrastructure.** A committed corpus of 36 reports over one synthetic
  "parking" database, each with an XML decode baseline and an HTML render baseline, validated out-of-band against
  the native Crystal engine; golden-file tests for the Page IR and every backend; `docker-compose.yml` and a
  `Makefile` for the fixture database; DB-gated CI regression tests.

- **Documentation.** New guides: rendering (`docs/11-rendering.md`), a compile-verified render cookbook
  (`docs/12-render-examples.md`), and the five-part formula-engine set (architecture/VM, language reference,
  builtins, validation). GitHub-native Mermaid diagrams throughout, a rewritten README (status section, quick
  start, example render), and a docs↔code audit bringing the block catalog, support matrix, and saved-data docs
  in line with the code.

### Changed

- **Workspace restructured into a 20-crate, two-layer workspace** with compiler-enforced boundaries:
  - The semantic model moved out of `rpt` into the standalone, pure-data **`rpt-model`** crate (no I/O, WASM-safe,
    optional `serde`); `rpt` re-exports it as `rpt::model`. The whole render/data pipeline depends on `rpt-model`,
    not the decoder, so the render stack links no CFB/inflate. Byte-level provenance notes live in the
    documentation-only `rpt::provenance` module.
  - The formula language moved into the standalone **`crystal-formula`** crate (depends only on
    `rpt-format-value`), reusable without the binary reader (LSP, WASM sandbox, validator).
  - The `rpt-engine` crate was dissolved (its derived analytics now live in `rpt-cli`'s private
    `export::analysis`), and the `rpt-to-xml` binary was folded into `rpt-cli` as the **`rpt xml-dump`**
    subcommand — one `rpt` binary for all inspection and export. XML output is byte-identical.
- **Naming:** standalone "SAP" was dropped in favor of "Crystal Reports" across the README, docs, CLI help, and
  crate metadata.

### Removed

- The standalone **`rpt-to-xml`** binary (now `rpt xml-dump`) and the **`rpt-engine`** crate (dissolved into
  `rpt-cli` / `crystal-formula`), per the restructuring above.

## [0.1.0]

### Added

- **Saved data (stored rows).** Decodes the cached rows a report carries when saved with data (`SavedRecordsStream` +
  `MemoValuesStream`) and exports them as a `<SavedData>` element. See [`docs/10-saved-data.md`](docs/10-saved-data.md).
- **Formula syntax.** Reports each formula field's authoring dialect (`Syntax` — `crFormulaSyntaxCrystal` or
  `crFormulaSyntaxBasic`).
- **SQL-expression fields.** Decodes `{%name}` SQL-expression field references.
- **Dynamic parameters.** Recognises dynamic (list-of-values) parameters and reports their editing flags accordingly.
- **Top N / Bottom N group sorts.** Decodes group summary sorts and renders their summary sort expression and direction.
- **Percentage summaries.** Decodes percentage summaries (`PercentOfSum (…)`, etc.).
- **Running-total conditions.** Decodes running-total reset and evaluation conditions (`OnChangeOfField` / `OnFormula`).
- **Cross-section boxes.** Resolves a box that spans into a later section, reporting its end section and bottom edge.
- **Dynamic image locations.** Decodes a picture object's dynamic graphic-location formula, and its `EnableCanGrow`
  flag.
- **Subreport on-demand flag.** Decodes a subreport's `EnableOnDemand` flag.

### Fixed

- **Subreport parameter report name.** A parameter defined inside a subreport now reports that subreport's name as its
  `ReportName` (previously always empty); main-report parameters remain empty, as the engine emits.
- **Basic-syntax formulas.** A formula authored in Basic syntax now reports `Syntax="crFormulaSyntaxBasic"` (read from
  the formula record's stored dialect flag) instead of defaulting every formula to Crystal syntax.
- **Table aliases with spaces.** Aliases whose table name contains spaces (which Crystal substitutes with underscores)
  now match correctly, fixing the alias and the field long-names and formula forms derived from it.
- **Range parameter current values.** A range (non-discrete) current value now sets `HasCurrentValue`.
- **Summary result types.** `Maximum` / `Minimum` summaries report the summarized field's own type; a Currency running
  total reports a Number result, matching the engine.
- **Negative line heights.** A line drawn bottom-to-top reports its height as a magnitude.
- **Cross-tab keep-together.** A cross-tab no longer inherits the object-level keep-together flag.
- **Field use counts.** Corrects use-count totals for summary-sorted groups.

## [0.0.0]

The initial release: a pure-Rust reader for SAP Crystal Reports `.rpt` files, with no dependency on the SAP runtime, a
database connection, or any Windows component.

### Added

- **Direct `.rpt` decoding.** Opens the CFB/OLE2 compound file, decrypts the report streams (AES-128 in CFB mode, fixed
  key, per-stream IV) with a self-contained pure-Rust cipher, inflates the zlib payload, and tiles it into the record
  stream.
- **Recursive record tree.** Resolves the per-record content mask to build the full nested record tree, and recurses
  into subreports (`Subdocument N` storages).
- **Lossless record substrate.** Every record is preserved verbatim, including types not yet modelled, so reading never
  loses data.
- **Typed report model.** Projects records into a structured model: summary info; report and print options (paper size,
  orientation, margins, page rectangle); database (connections, tables, command/SQL tables, fields, joins); data
  definition (parameters with types and default/current values, formulas, groups, sort fields, summaries, running
  totals, record/group selection formulas); and report definition (areas, sections, and report objects with placement,
  fonts, borders, colors, and conditional formatting).
- **Subreport links.** Decodes how values pass between a report and its subreports.
- **Derived analytics (`rpt-engine`).** Computes values the engine derives rather than stores — including field use
  counts — backed by a Crystal formula lexer, parser, and reference/type analysis.
- **`rpt-to-xml` exporter.** Serializes a report to a structured XML document, with a `--full` mode that also dumps the
  complete decoded record tree.
- **`rpt` command-line inspector.** A read-only CLI with `inspect`, `inputs`, `streams`, and `strings` subcommands and a
  `--json` flag for machine-readable output.
- **Docker image.** A multistage build producing a minimal (~14 MB) image containing only the statically linked
  binaries.
- **Release workflow.** On a version tag, publishes cross-platform binaries (Linux, macOS, Windows) to a GitHub Release
  and pushes the Docker image to the GitHub Container Registry.
- **Documentation.** A guide to the `.rpt` format and the library under [`docs/`](docs/).
