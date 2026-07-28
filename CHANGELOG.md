# Changelog

All notable changes to rpt-rs will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),

## [Unreleased]

This release replaces the RptToXml-compatible XML export with a plain, exhaustive **JSON** dump of the decoded model as
the decode regression surface, finishes the stored-vs-derived separation, adds a lossless KDL projection and a `.rpt`
anonymizer, pushes the render pipeline through typography, subreport flow, charts, and live-database work, and closes a
workspace-wide audit of **error surfacing** so that every failure says what went wrong, where, and what to do next —
including the ones the pipeline used to swallow silently.

### Breaking changes

The upgrade checklist; each item is expanded in the sections below.

- `From<std::io::Error> for rpt::Error` removed — a bare `?` on an I/O call no longer compiles. Use
  `rpt::IoError::at(op, path, source)`, or `::new` where there genuinely is no path.
- `rpt::Error::Record`, `rpt::RecordError`, and `ProjectErrorKind::UnknownRecord` deleted (zero construction sites) —
  use `Rpt::decode_coverage()` to detect an incomplete decode.
- `rpt_query::build_query{,_in,_full,_for_report}` return `Result<_, QueryError>`, not `Option`.
- `rpt_json::export_json` returns `DecodeCoverage`, not `()`; its `Error::Io` split into `Write { path }` /
  `Serialize { input }`.
- `ReportDocument::export_{pdf,html}_to_disk` return `rpt::Result<()>`, not `std::io::Result<()>`.
- `rpt_render::render_with` is infallible (`-> PagedDocument`); `RenderError` moved to the CLI; the `db-postgres` /
  `db-sqlite` features are gone.
- `rpt_pages::DiagnosticKind` is now `#[non_exhaustive]` — an exhaustive `match` needs a wildcard arm.
- `rpt_data::DbError::{Connect,Query}` are struct variants carrying the data source and statement; `no_table()` →
  `no_query(reason)`.
- `metafile::Error` variants carry `offset` / `record`.
- `rpt_model::TableLink.join_type` is replaced by `join_kind` + `operator`.
- `rpt_model::Report` no longer carries `records` / `record_inventory`, and `record_count`, `distinct_record_types`,
  `count_of` are gone — use `Rpt::record_dom()` / `Rpt::inventory()`.
- `Sqlite/PostgresSource::fetch` take a unified argument list.
- `rpt patch` refuses a record type that is not cleared for editing — pass `--force` for the previous behaviour.
- `rpt xml-dump`, the `rpt-xml` crate, the XML baselines, and the export `--locale` flag are gone — use `rpt json-dump`.
- `docs/` was renumbered; existing links to the old numbering do not resolve.

### Added

- **`rpt json-dump <file.rpt> [out.json]` — the decode regression surface.** Emits an exhaustive, deterministic
  `{ "model": <Report> }` document: the full serde serialization of the decoded model, every field including defaults,
  the whole subreport tree, sorted-key maps, two-space indent, byte-identical across runs. It carries **stored facts
  only** — nothing inferred, recomputed, or reconstructed — so a diff against a committed baseline always means the
  decoder's reading of the file changed. Backed by the new reader-side library crate **`rpt-json`** (`export_json`),
  callable in-process instead of by spawning the CLI. The committed baselines (94 fixtures) live under
  `tests/fixtures/baselines/json/` and are checked by `cargo test -p rpt-cli --test json_baseline`
  (`RPT_BLESS=1` re-blesses).
- **`rpt-kdl` crate + `rpt kdl <file.rpt> [out.kdl]`.** Exports the semantic model to a [KDL](https://kdl.dev) document
  (`rpt_kdl::to_document` / `to_kdl_string`): construct kinds as node names, the identifying name as the first argument,
  scalars as `key=value`, nested structs/`Vec`s as child nodes — *sparse* (non-default values only), geometry in twips,
  colours as `#rrggbb`, enums as kebab-case tokens, formula/text bodies as multi-line strings. Binary payloads never
  enter the KDL: pictures emit a `source="…"` reference with bytes returned out-of-band by `rpt_kdl::assets`. The
  mapping is **lossless and compiler-enforced** — every struct is destructured without `..` and every enum matched
  without a `_` wildcard, so a new `rpt-model` field fails to compile until the exporter handles it. It emits the whole
  stored surface: the record-level sort, report options, saved printer, table columns, field-pool metadata, the full
  parameter surface (edit mask, report name, current/initial values with range bounds, display/sort/discrete-or-range
  kinds, prompt group, dynamic LOV binding), persisted formula variables, running-total condition formulas, hierarchical
  group values, the full chart definition, and cross-tab axis options. Depends only on `rpt-model` + `kdl`. The CLI
  subcommand writes sidecar picture files, or prints to stdout.
- **`rpt anonymize <file.rpt> [out.rpt]` — strip authoring metadata.** Removes the identity a report carries: the OLE
  `SummaryInformation` author and last-saver (blanked), and a re-imported subreport's stored source path (reduced to its
  bare file name, *not* blanked, because it is the only evidence the subreport was imported at all — emptying it would
  silently turn `SubreportObject.IsImported` false). The database connection's stored path is deliberately left alone:
  it is a live datasource locator, not authoring metadata. **Every edit is same-length** — a value's length prefix is
  untouched and only its characters are overwritten, then NUL-padded to the original width — so no record length, no
  enclosing record length, no property offset and no section size moves, and the decoded model is unchanged apart from
  those fields. A report with nothing to remove round-trips byte-identically. `--dry-run` reports what would go without
  writing; `--json` machine-reads the removals. Exposed as `Rpt::anonymize` / `rpt::io::AnonymizeReport`; this is what
  keeps the committed fixture corpus publishable.
- **`rpt dump --stream DataSourceManager`.** The `DataSourceManager` saved-data catalog stream's logical payload
  (decrypted + inflated) is now exposed through the reader's `streams()` / `logical_bytes()` surface, so `rpt dump`
  can read its QE-dialect record tree and dump its records (structure `0x2d`, field header `0x41`, batch entry `0x6d`).
  `rpt streams` reports the stream's decoded logical byte count instead of labelling it opaque.
- **`rpt tree` shows decoded field-value summaries.** For recognized records — the field-format leaves
  (numeric/string/date/time/date-time/boolean/common), group-area options (`0x88`), and summary/running-total
  definitions (`0x7e`) — each node shows a concise decoded summary (`DecimalPlaces=2 Negative=Bracketed …`) in place of
  the raw byte preview; `--json` carries the same as a `decoded` object. Backed by the new public
  `rpt::annotate::summarize` API.
- **SQL tracking comments on every generated query.** A query the render path issues is prefixed with a sanitized
  single-line block comment identifying the report and scope (`/* rpt-rs report="orders.rpt" scope=main */ SELECT …`),
  so a statement surfacing in the database's own logs is traceable back to the report that issued it. Built by
  `rpt_query::SqlQuery::with_comment`, which flattens control characters and breaks up `*/` / `/*` so the comment can
  neither terminate early nor nest; an empty comment is a no-op. Both drivers accept it.
- **`metafile` crate.** A Windows-metafile vector parser (EMF today; WMF/EMF+ planned) that resolves a metafile's own
  coordinate machinery into backend-agnostic primitives through a `MetafileSink` visitor, so the pipeline can draw
  OLE-embedded EMF pictures as vectors. Zero dependencies, no GDI, WASM-safe, and free of `.rpt` concepts — publishable
  on its own (MIT OR Apache-2.0).
- **`meridian-seed` + the synthetic render-test corpus.** A deterministic generator for the "Meridian Global Logistics"
  render-test database, emitting portable Postgres/SQLite SQL from one fixed-seed PRNG (~39 tables, ~40k rows in the
  committed `small` tier; `--tier`, `--dialect`, `--out`, `--ddl-only`). Per-field distributions are realistic and date
  envelopes internally consistent; the committed seed lives at `tests/meridian/sql/meridian.sql`, the human-readable
  schema at `apps/meridian-seed/schema.sql`. Alongside it: a corpus of own reports over that database with HTML render
  baselines, and a typography fixture set with Page-IR/HTML baselines.
- **Formula constants: `cr*` enum constants and relative date ranges are now evaluable.** The 117 `cr*` enum constants
  (alignment / font style / line style / negative & currency format / calendar, e.g. `crBold`, `crLeftAligned`,
  `crDashedLine`) and the 27 relative-date-range constants (`YearToDate`, `MonthToDate`, `Last7Days`, `LastFullMonth`,
  `AllDatesToToday`, `Aged0To30Days`, the calendar quarters/halves, …) now evaluate instead of failing with an
  unsupported-builtin error. The `cr*` constants resolve to a bare number; the date ranges resolve to an inclusive date
  `To` range computed against the context's `CurrentDate`/`Today`. Conditional-format formulas returning a `cr*`
  constant and record-selection formulas using a date range no longer surface as unsupported-formula diagnostics.
- **Date/time formula specials in the render pipeline.** Formulas reading `CurrentDate`, `Today`, `CurrentDateTime`, or
  `CurrentTime` now resolve during a render instead of evaluating to null — so a date-relative formula (e.g. an aging
  bucket that buckets rows by days from `CurrentDate`) produces its real value. A single "as-of" instant is captured
  once per render (`RenderOptions::as_of`, defaulting to the system clock at render start via `default_as_of`, or set
  explicitly for a reproducible render), threaded through both the record pipeline and the layout pass so every
  evaluation context — including crosstab pivots and subreports — shares the same fixed value. `Today` is recognized as
  the engine's alias for `CurrentDate`. The render core stays clock-free and WASM-safe: the clock is read only at the
  entry point (the `rpt-render` facade / CLI), and a WASM build falls back to the Unix epoch unless the host supplies an
  `as_of`.
- **Paragraph typography in text objects.** A text object's per-paragraph formatting is now honored end to end instead
  of being flattened to one object-level font at single spacing:
    - **Per-paragraph font.** Each paragraph renders in its own run font — a multi-paragraph object mixing point sizes
      (e.g. a 12pt paragraph followed by a 20pt one) now draws each paragraph at its own size and wraps it with the
      right metrics, rather than rendering every paragraph at the object's base size.
    - **Line spacing.** The paragraph line spacing (`LineSpacing` / `LineSpacingType`) is now decoded (the `0x00c0`
      paragraph leaf: type byte 17, a 16.16 multiplier — or exact twip pitch — at bytes 18-21) into a new
      `IndentAndSpacingFormat.line_spacing` model field, and applied by the layout engine: single / one-and-a-half /
      double spacing grows the inter-line gap, and exact spacing pins the line pitch to its twip value. Can-grow band
      heights account for the taller lines.
    - **Justified alignment.** Justified text now stretches to both edges by spreading the inter-word slack across every
      wrapped line except a paragraph's last (which stays flush-left), instead of rendering identically to left-aligned.
      The layout marks which lines justify; all four backends flush both edges (SVG `textLength`, HTML
      `text-align-last:justify`, PDF `Tw`/word-by-word, raster per-gap spread).
    - **Text rotation.** The stored text rotation angle (`TextRotationAngle`, `0x00fc` bytes 20-21, degrees) is now
      decoded and applied: a 90°/270° text object lays its wrapped lines out as vertical columns (reading up for 90°,
      down for 270°) and all four backends (SVG, HTML, PDF, raster) rotate the runs, matching the native engine's
      vertical text.
- **Chart text: subtitles, footnotes, and per-element fonts.** A chart's decoded subtitle (drawn under the title) and
  footnote (drawn at the chart bottom) are now rendered for every chart family; previously both were decoded but never
  drawn. Chart text also renders in the font stored per text element instead of one hardcoded Arial: the layout engine
  resolves each title/subtitle/footnote against the decoded `ChartDefinition.element_fonts` run, preferring the stored
  face name (when it is a real override, not the default Arial) and stored point size, and otherwise falls back to the
  engine's per-element default table (title Arial 14 bold, subtitle Arial 10, footnote Arial 8 bold-italic,
  axis/data/series titles Arial 8 bold, legend/data labels Arial 7). Charts that store defaults render byte-identically.
- **3-D ("Faked3DRegular") pie rendering.** A pie chart whose stored subtype requests the depth effect now draws a
  tilted-ellipse face with an extruded, shaded crust along its front rim instead of the flat 2-D disc, matching the
  engine's faked-3D pie. Flat pies are unchanged.
- **Biweekly chart category axis.** A chart whose category axis is a date field grouped biweekly now buckets on
  fortnight-aligned two-week boundaries instead of falling back to weekly grouping.
- **Aspect-fit picture rendering.** Picture objects and DB blob-field images now letterbox — the raster is scaled
  uniformly to the largest size that fits its object box, preserving the source pixel aspect ratio, and centered, with
  the surrounding space left empty — matching the native engine instead of non-uniformly stretching to fill the box. The
  Page IR's `ImageOp` carries a new `fit: ImageFit { Fill, Contain }` (defaulting to `Fill`, so existing Page-IR dumps
  deserialize unchanged); layout emits `Contain` for pictures and blob fields, and all four backends honor it (HTML
  `background-size:contain`, SVG `preserveAspectRatio="xMidYMid meet"`, PDF/raster compute the centered fitted sub-rect
  from the decoded pixel dimensions).
- **Raster backend JPEG/GIF pictures.** The `rpt-render-raster` backend now decodes JPEG (via the pure-Rust
  `zune-jpeg`) and GIF first frames (via the pure-Rust `gif` crate) in addition to PNG and BMP, so those picture formats
  composite as pixels instead of falling back to the placeholder outline. Both decoders are WASM-safe (no C bindings),
  matching the render core's portability rule.
- **Cross-page inline-subreport flow.** An inline subreport taller than a whole page is now split across parent pages at
  row boundaries instead of overflowing off the page bottom. The child is still formatted exactly once (its
  `Shared`/`Global` writes fire once); the split is pure geometry over the cached box-local ops, so a subreport with an
  internal forced page break places each of its pages on its own parent page. A subreport that fits on a page keeps the
  byte-identical atomic placement (moving whole to the next page when the space left is too small).
- **A large set of previously-unmapped stored fields is now decoded**, and is carried by the JSON and KDL exports:
    - *Fields & formats:* value types for formula / special / group-name / running-total / SQL-expression references;
      the numeric & currency leaf (negative format, currency-symbol format, symbol text and placement, thousands
      separator, suppress-if-zero); the string leaf (word-wrap, maximum line count, text interpretation); the date /
      time / date-time display members (date order, hour/minute/second, date-time order) and separator; object vertical
      alignment.
    - *Objects:* the object hyperlink **type**, read from its stored `CrHyperlinkTypeEnum` byte (see *Changed*); a box's
      rounded-corner ellipse (width/height, the stored basis for rounded boxes and ovals); picture geometry (original
      size, X/Y scaling, cropping); box/line end-section spanning; section codes; paragraph indentation and
      field-heading reading order; subreport links, the imported flag and the re-import flag; the group-name special
      field in SDK form (`GroupName ({field})`).
    - *Data definition:* **custom-function declarations** (`DataDefinition.custom_functions`), split out of the formula
      pool by body shape — a body opening with the `Function` keyword can only be a custom function, never a report
      formula; SQL-Expression field definitions; parameter range kind and range current value; command-table /
      stored-procedure parameters; formula null-treatment; the group long-period date-grouping condition (weekly …
      annually) and the six boolean group conditions; the summary operation parameter, percentage-summary flag, and a
      two-field summary's secondary field; Top N / Bottom N `DiscardOthers`.
    - *Report & database:* report options (convert-database-null-to-default and its sibling, verify-on-every-print);
      database table qualified names; a table link's join kind **and** comparison operator as independent values (see
      *Changed*); cross-tab grid style flags, grand-total colors, and per-level suppress-subtotal / suppress-label;
      chart data/category field references (in summary form), 3-D view angle, layout type, and the full legend-position
      set (`Left` / `Right` / `BottomCenter` / `Custom`, all four now corpus-confirmed); summary-info provenance
      (revision number, last-saved-by, saved printer name).
- **`rpt formulas <file.rpt>` — list and check every formula, without rendering.** Prints one line per formula marked
  `ok` / `warn` / `ERROR` with findings indented beneath, so you can see what was covered rather than only how many.
  Covers formula fields, record- and group-selection formulas, and conditional-format formulas wherever they hang — on
  a section, an object's format, its border, or a field/text object's font colour — through every subreport. Reads the
  `.rpt` alone; exits 1 on any error, so it works as a CI gate. `--source` quotes each formula's body under its line,
  and a formula field the report declares but left blank is listed as `empty` rather than silently omitted. `--json`
  always carries the source, kind, syntax, and size.
- **Incomplete decodes are reported instead of passing silently.** `rpt json-dump`, `rpt kdl`, `rpt-render`, and
  `rpt streams` now say when a report did not fully decode — previously an export missing content looked identical to a
  faithful one, because projection returns defaults for records it does not recognize rather than failing. New
  `Rpt::decode_coverage()` exposes the figures (unrecognized records and their types, bytes covered by no record,
  per-stream decode errors); `rpt streams` gains them in `--json`. `--strict` on the two export commands turns the
  warning into exit 1 for CI — the document is still written, so only the exit status changes.
- **The write path refuses uncleared record edits; `rpt patch --force` overrides.** Patching a record type that is not
  on the cleared-for-editing allow-list now fails with nothing written, because the writer's mechanical checks cannot
  catch a leaf carrying its own offset table or checksum being overwritten into a file that re-decodes cleanly but is
  semantically corrupt. The list holds one entry today (`0x0142 SubreportReimportInfo`), so most existing `rpt patch`
  invocations need `--force` (`EditPolicy::Forced`) — the right flag when writing an invalid record is the point, the
  wrong one for a report you intend to keep.
- **Diagnostics carry kind, severity, and location end to end.** `rpt_pages::Diagnostic` gained a
  `DiagnosticLocation { page, area, section, record_index, span }` (all optional, never fabricated) and `describe()`;
  `DiagnosticKind` gained the data-pipeline kinds and is now `#[non_exhaustive]`. Formula diagnostics carry the failing
  sub-expression's byte range and per-row failures the record index. `rpt_data::DatasetOptions` / `build_dataset_opts`
  is the general pipeline entry point and the only way to attach a `DiagnosticSink` alongside parameters, link filters,
  and an as-of instant. See [Rendering › Diagnostics](docs/12-rendering.md#diagnostics).
- **Errors say which file, and print their cause once.** `rpt::IoError` carries the operation and path, so the commonest
  failure names itself (``cannot read `/nope/missing.rpt`: No such file or directory``); `rpt::error_chain` renders a
  full `source()` chain and is shared by both binaries. `Error::NotAReport` diagnoses an input that is not a report at
  all — no OLE2 signature (with a sniff of what it actually is), a compound file with no `Contents` stream (listing what
  it does carry), a truncated container, or a `Contents` that will not decrypt — in place of `Invalid CFB file (wrong
  magic number)`.
- **Database failures name the source, the statement, and the next step.** `DbError::hint()` returns the failing SQL and,
  for a missing table or column, a pointer at `rpt sql <file>`; the error names the data source, which is what tells a
  multi-`RPT_DB_URL_<SERVER>` report which connection is wrong. `rpt_query::QueryError` replaces a reasonless `None`, and
  `SqlQuery::not_pushed` reports the selection conjuncts that could not be pushed into `WHERE` and so run locally after
  the fetch — the query reads more rows than the report shows, now warned at normal verbosity rather than only under `-v`.
- **Silent type coercions are reported.** A cell that will not parse as its column's declared type still falls back to
  text or null, but `RowSource::coercions()` now reports it once per column with the affected row count and an example —
  values kept as text sort, group, and summarize as text, which is otherwise invisible.
- **Test and CI infrastructure.** A deterministic fuzz target over `crystal-formula` (parse → compile → run → validate
  over generated input, both syntaxes, plus deep nesting), a CI guard for error-handling anti-patterns clippy cannot
  express, and `clippy::missing_errors_doc` / `missing_panics_doc` at workspace level — every public fallible API now
  documents the error variants a caller can match on.

### Changed

- **The export surface is JSON + KDL, and it is stored-facts-only.** The XML exporter and the derived analytics are
  gone (see *Removed*); the exhaustive `rpt-json` dump and the sparse `rpt-kdl` projection replace them.
- **Workspace restructured into `crates/` + `apps/` (24 members).** The three binaries moved out of the library tree
  into `apps/` — `apps/rpt-cli` (the `rpt` binary), `apps/rpt-render-cli` (`rpt-render`), and the dev-only
  `apps/meridian-seed`. No `[[bin]]` remains under `crates/` and no library logic remains under `apps/`: an app parses
  arguments, resolves inputs, and calls a library.
- **A table link's join kind and comparison operator are decoded independently** (breaking model change). The file
  stores the link's outer-ness and its comparison separately, so `TableLink.join_type: TableJoinType` is replaced by
  `join_kind: TableJoinKind` (inner / left / right / full outer) + `operator: TableLinkOperator` (`=`, `<>`, `<`, `<=`,
  `>`, `>=`); both are one-hot bit codes, not ordinals. The SDK's single `TableJoinType` cannot express "left outer
  **and** `>`", so it is now a lossy fold a consumer performs, not the decoded shape. `rpt-query` generates the ON
  condition from the operator and the join keyword from the kind, so a left-outer inequality join renders correctly; a
  full outer join has no counterpart and still degrades to an inner join.
- **A field renders its own stored currency symbol.** The currency symbol is stored *per field*, so two fields in one
  report can carry two different currencies: an explicit (non-system-default) field now uses its stored symbol text
  (`€`, `Kč`, `kr `, including any spacing baked into it) and its NoSymbol/Fixed/Floating choice, instead of always
  taking the locale's symbol. `NoSymbol` drops to a plain number; a system-default field still resolves the symbol from
  the render locale, which is what the engine does. Symbol *placement* still follows the locale pending the stored
  position byte.
- **An object's hyperlink type is read from the file, not guessed from the target.** The stored `CrHyperlinkTypeEnum`
  ordinal in the `0x00fc` ObjectFormat leaf is now located by walking past `ToolTipText` and `CssClass`, so a non-empty
  tooltip cannot shift it, and it decides whether a hyperlink exists (code `6`, `Undefined`, is the engine's "no
  hyperlink" sentinel). Previously the kind was inferred from a `mailto:` prefix and presence was decided by a non-empty
  target — which dropped every real hyperlink kind that legitimately carries an empty target (a field-value website, a
  report-part drill-down).
- **Subreport fetches are memoized per render.** The layout engine asks for a subreport's rows once per *instance*, and
  because a parent link is applied in memory after the fetch (never in the `WHERE`), every instance of one subreport
  issued the identical query — an O (instances × rows) blow-up on a report with many groups. The `rpt-render` CLI's live
  scope now memoizes each distinct fetch, so those repeats collapse to a single round-trip and a shared, refcounted
  rowset.
- **The raw record substrate left the format-neutral model.** `rpt_model::Report` no longer carries `records` (a full
  second copy of the record tree, every leaf byte cloned into the model on each open) or `record_inventory`, and the raw
  `Node`/`Unknown`/`Value`/`RecordTag`/`RecordTypeCount` types moved out of `rpt-model` into the `rpt` reader
  (`rpt::raw`). The reader now projects them **on demand** from the substrate it already owns: `Rpt::record_dom()`,
  `Rpt::inventory()`, and `Rpt::subreport_record_doms()`. This makes the neutral model actually neutral — the whole
  render/data/db pipeline (which links `rpt-model`, not `rpt`) no longer drags in the `.rpt` record-type registry, and
  no report pays the per-open cost of duplicating its record tree. `rpt tree` output is unchanged.
  (`Report::record_count`/`distinct_record_types`/`count_of` are gone; derive them from `Rpt::inventory()`.)
- **`rpt-render` facade error model honesty.** The library's `render_with` is now infallible (`-> PagedDocument`
  instead of `Result<_, RenderError>`) — its built-in datasources (saved data, a materialized `RowSource`, a pre-built
  `Dataset`) cannot fail, and a live fetch fails before the render, in the caller. The stringly-typed `RenderError`
  (which the library never actually constructed) moved to the `rpt-render` CLI where every failure is raised; its `Db`
  and `Io` variants now carry the underlying `DbError`/`std::io::Error` as a real `source()` instead of flattening it
  into a message, so CLI errors print the full cause chain (e.g. `cannot write "…": No such file or directory (os error
  2)`). As a result the `rpt-render` library no longer links the native DB-driver crates at all (its `db-postgres`/
  `db-sqlite` features are gone), keeping the render core fully DB-free. The bring-your-own-layout
  `render_dataset_with` now takes the same `locale`/`scope`/`as_of` reproducibility knobs `RenderOptions` carries,
  instead of silently defaulting them.
- **Unified live-DB driver skeleton.** `rpt-db-postgres` and `rpt-db-sqlite` now share one error type and one
  constructor shape instead of duplicating them. The shared, driver-agnostic `DbError` lives in `rpt-data` (generic over
  each driver's own error type, so the crate stays WASM-safe with no DB-driver dependency), and both drivers alias it.
  Both `SqliteSource::fetch` and `PostgresSource::fetch` now take the same arguments (`url`/`conn_str`, `database`,
  `sql_exprs`, `selection`, `params`, `comment`): SQLite gains the `selection`/`params`
  push-down parameters and a `fetch_for_report` pruning constructor, and Postgres gains query-log `comment` support.
  Existing default (no-selection) query output is unchanged. The `rpt-render` CLI's SQLite path now builds the SQL once
  (via a new `SqliteConn`) and runs that exact query, so the logged SQL is by construction the SQL executed.
- **`rpt` CLI flags are scoped to their subcommand.** The argument parser is now table-driven: each subcommand declares
  exactly the flags it accepts, so a flag meant for another command (e.g. `rpt inspect f.rpt --probe u32`) is now a
  clean usage error (`unknown option '--probe' for 'inspect'`, exit 2) instead of being silently ignored. All valid
  invocations, flags, and output are unchanged.
- **`rpt` CLI usage errors are attributed correctly and exit with code 2.** A malformed flag or argument value (e.g.
  `rpt sql report.rpt --dialect bogus`, an invalid `dump` option value, a bad `patch` argument) is now reported as a
  plain usage message and exits with code `2`, instead of being laundered through an I/O error that printed an `io:`
  prefix and exited `1`. Reader and output errors are unchanged (still exit `1`).
- **A stream encrypted under a foreign key is diagnosed as a key problem.** Crystal never emits one, but an application
  embedding CSLib300 can round-trip reports under its own key, and such a payload simply will not decrypt with the
  built-in key. That case is now reported as an encryption-key failure rather than as a bogus zlib error. The
  `Contents` header's `useFixed` flag is confirmed inert — clearing it changes nothing about how the stream decodes, and
  the reader deliberately does not branch on it, matching the engine.
- **Linked subreports execute per instance.** An inline subreport is rendered against a per-instance dataset filtered by
  its parent-row link values: a parameter-routed link binds the parent field into the subreport's parameter (so its
  record-selection formula, and the live-DB `WHERE` push-down, keep only the linked subset) and a direct field link
  applies a structural equality filter. Previously every subreport instance rendered the same unfiltered dataset. This
  also propagates `Shared` variables accumulated inside a subreport back to the main report (a main-report grand total
  reading a subreport-accumulated `Shared` variable now resolves). New `rpt_data::build_dataset_with` /
  `rpt_data::FieldFilter`.
- **On-demand subreports render their placeholder caption instead of executing.** A subreport flagged `EnableOnDemand`
  is a click-to-expand link the native engine never runs in a static export; the layout now emits just its caption (the
  subreport name) and skips the query/dataset entirely, instead of executing it inline and drawing its clipped bands.
- **A subreport taller than its placeholder box grows the enclosing band instead of clipping.** An inline subreport is
  now formatted once ahead of pagination (its `Shared`/`Global` writes fire exactly once); the containing band grows to
  fit its full height and the existing checkpoint pagination flows the enlarged band across pages, so a subreport's
  detail rows are no longer truncated to the first page of its box.
- **Stored display formats are honored at render.** Numeric/date/time fields apply their stored thousands-separator,
  suppress-if-zero, per-field currency symbol and placement, and date/time separator instead of the locale's.
- **Formula null-treatment is honored during evaluation.** `crTreatNullAsDefaultValue` replaces a null field with its
  type's default and continues; the default `crTreatNullAsException` propagates the null.
- **Top N / Bottom N group sorts apply their limit and DiscardOthers** — groups are ranked by their summary and cut to
  N, the rest discarded or collapsed into a single "Others" group.
- **Date groups bucket by their stored granularity** (daily … annually, plus the order-sensitive boolean conditions)
  rather than by raw value. The boolean conditions break on a transition, on the row before it, or on the row after it,
  depending on the condition family; an unrecognized condition ordinal falls back to raw-value grouping and raises a
  diagnostic.
- **Chart/cross-tab temporal category labels are driven per period by an intentional style** rather than a
  monthly-vs-everything catch-all: each `ChartCategoryPeriod` maps to an explicit label style — annual reads `YYYY`,
  monthly `M/YYYY`, quarterly/semi-annually roll to their start month `M/YYYY`, and the day-granular periods
  (daily/weekly/semi-monthly) `M/d/YYYY`. Monthly (`M/YYYY`) and weekly (`M/d/YYYY`) are unchanged (oracle-confirmed);
  the previously catch-all annual/quarterly/semi-annual labels now read in their own style. A raw (un-grouped) date
  cross-tab column now shows the field's default date format instead of the compact style.
- **Two-field summaries compute** — WeightedAverage / Correlation / Covariance now resolve from the decoded secondary
  field instead of an empty value.
- **Layout controls are applied when placing objects** — center/bottom vertical alignment, paragraph indentation and
  right-to-left reading order, pictures at their authored scale (with cropping), and boxes/lines spanning to their end
  section.
- **Cross-tab grid style and grand totals render** — grid lines follow the stored show-grid option; grand totals are
  computed, filled with the stored colors, and suppressed when set.
- **Rounded box corners render** (border radius) in the HTML and PDF backends, driven by the box's newly decoded corner
  ellipse.
- **Field headings render** as text runs with their stored label, font, and colour, instead of being dropped.
- **Pictures render across all four backends, embedded once and referenced by content hash.** HTML, PDF, SVG, and raster
  all draw report pictures (BMP/PNG/JPEG/GIF), each distinct image embedded a single time. Binary columns are fetched as
  real bytes (a new `Value::Bytes`, no `::text` cast), so a blob-bound picture — including a per-row database blob —
  receives its true image bytes end-to-end.
- **Per-glyph Unicode font fallback** with a bundled open symbol/emoji font replaces `.notdef` boxes for glyphs the
  primary font lacks.
- **Report-footer grand totals render** — row-independent field kinds (summary / special / group-name)
  now resolve from the print state even in a row-less band.
- **`rpt-pages` Page IR** documents one additive-stable policy (new `#[serde(default)]` fields and
  `DrawOp` variants are non-breaking; renames/removals gated) and moves `serde_json` behind a default-on
  `json` feature so a consumer of the IR types can drop it.
- **The committed test corpus was reshaped.** The third-party `ajryan` reports were dropped (their licensing and content
  made them unpublishable as a regression corpus) in favour of reports authored for this project, the synthetic Meridian
  corpus, and a typography fixture set. Every committed fixture is `rpt anonymize`-clean, enforced by a test.
- **The `docs/` set was renumbered and rewritten** so it reads front to back in one order (format `01`–`04`, semantic
  model `05`, saved data `06`, block catalog `07`, support matrix `08`, endianness `09`, codebase map `10`, usage `11`,
  rendering `12`, render examples `13`). Existing links to the old numbering no longer resolve.
- **The render CLI reports the data pipeline's diagnostics.** Previously only layout/render diagnostics reached the
  warning channel. Data-pipeline failures now arrive too, errors print through a channel `-q` cannot suppress, and
  identical repeats collapse (`… [record 0] — and 606 more like it`) so a per-row failure cannot bury the summary.
- **Supplying a parameter the report does not declare is an error, not a warning.** The value cannot reach the render, so
  the output is for different criteria than asked for. It survives `-q` (which previously hid it from scripted runs) and
  suggests the nearest declared name: `parameter "order_amt_rang" is not declared by the report … did you mean
  "Order_Amt_Range"?`. The render still proceeds on the report's own defaults.

### Fixed

- **Formula / parameter / running-total / SQL-expression / special fields honor their stored format when rendered.**
  A placed field object carries a bound `value_type` only for database and summary fields; for every other kind the type
  lives on the field *definition*, so the render pipeline saw `Unknown` and formatted the value as a bare
  string/number — a formula field with a stored 2-decimal, currency, or date format leaf silently lost that format. The
  layout engine now resolves each object's **effective** value type (via the shared
  `rpt_model::field_object_value_type`) before picking the display format, so such a field renders with its own stored
  numeric/currency/date format (e.g. `$100.50`) instead of `100.50`.
- **Charts stay robust under degenerate and extreme data.** Non-finite (NaN/±Inf) plotted values are folded to zero at
  aggregation and clamped through a shared value→axis fraction in every axis/3-D renderer, so a divide-by-zero formula
  or an extreme magnitude can no longer produce runaway (i32-saturating) bar/riser geometry or an integer-overflow panic
  in the 3-D label placement; the value scale ignores non-finite values, and empty / all-zero / negative-only /
  high-cardinality (1000+ mark) series render without panicking.
- **A chart title no longer decodes to a garbage point size.** The stored title-size slot is shared with adjacent layout
  state and is not written by every authoring tool, so it could decode as a 2–4 pt or 200 pt+ title. A decoded size
  outside the range a real title occupies is now treated as "not an override" and falls back to the engine default.
- **A group-by formula fetches the database fields it references.** When a report grouped by a formula
  (`{@Key} = ToText({t.id},0) & " - " & {t.name}`), the live-DB query collected fields only from placed objects,
  selection formulas, running totals, and subreport links — never from a group's condition/sort formula. A field
  referenced *only* inside the group formula was left out of the `SELECT`, so the group key couldn't distinguish rows
  and every value collapsed into one group (e.g. a Top-N subreport showing one item instead of five, at a fraction of
  its true height). `rpt-query` now also walks each group's condition and sort formulas for their referenced fields.
- **Box and border background fills are no longer forced to white.** The border/format record's background colour was
  discarded by a bogus "white" sentinel on the fill's (inert) alpha byte, so every filled box decoded as white — a
  navy/rust column-heading bar rendered white-on-white (invisible). The fill colour is now always read from its RGB
  bytes, matching the engine (a genuinely auto/white box is unaffected).
- **Cross-tab dimensions bucket by their grouping period, and all measures render.** A cross-tab column bound to a date
  grouped monthly previously produced one column per raw date (an exploded, mislaid grid with wrong totals) instead of
  one per month; the dimension's stored grouping period (decoded from its grid-group leaf) is now applied. A cross-tab
  with several measures drew only the first; every declared measure now renders, stacked per cell.
- **Group header/footer bands sort by their decoded nesting level.** A 3-level nested group's areas (and their group
  associations) could come out in the wrong order when the group areas were stored, or named (`nameHeader`/
  `customeridHeader`, no trailing digit), out of nesting order; the level now comes from the per-area section-code
  record rather than binary-appearance order or an area-name digit.
- **The live-Postgres fetch streams instead of buffering the whole result set.** `rpt-db-postgres` now reads rows
  through a server-side portal (`query_raw`) rather than `query`, which materialized the entire joined result into a
  driver-side `Vec` before the pipeline saw a single row. On a large joined result this held two full copies of the
  result at once (the raw driver `Vec` plus the typed rows built from it)
  and could exhaust memory during the fetch phase. Driver-side memory is now O (1) in the row count — measured at ~60 MB
  flat for 5M wide rows versus ~1.46 GB (and growing linearly) for the buffered path — so a report whose live result
  runs to tens of millions of rows no longer doubles its peak footprint just to fetch. (The pipeline still materializes
  the selected rows into the in-memory `Dataset` for sort/group/summary, so a genuinely enormous result set can still be
  memory-bound there; that is a separate, larger streaming-layout effort.)
- **A cross-tab's decomposed cell objects are no longer drawn on top of its grid.** The decoder surfaces a cross-tab's
  internal cell/label/summary objects flat in the section, geometrically inside the cross-tab box; the layout now
  suppresses any object whose top-left sits within a cross-tab's bounds (the native grid already draws the whole pivot),
  so a cross-tab's grand-total labels and cells render once instead of doubled/tripled.
- **A tall subreport in the Report Header now flows across pages instead of clipping.** Cross-page inline-subreport flow
  previously engaged only for detail/group bands; a subreport in the report header (emitted at page-top) rendered
  clipped to page 1. The report header is now emitted through the same flow path, so a report-header subreport taller
  than a page splits across continuation pages (each repeating the page header) before the main body begins. The common
  case — a report header shorter than a page, with no overflowing subreport — is unchanged.
- **Scatter and bubble charts plot their points for group-scoped summary bindings.** An XY-scatter or bubble chart whose
  data bindings are group summaries (`Sum({field}, {group})`) — including a formula binding — now plots one point per
  category group (x/y/size = each binding's per-group value) instead of logging "no plottable points". An ungrouped
  point scatter falls back to one point per detail row, evaluating each binding formula-aware in the row context.
- **Section-level conditional formatting is now evaluated at render time.** A section's own `BackgroundColor`
  condition formula (e.g. `if {@IsUnderperformer} then RGB(255,220,220) else crWhite`) is resolved per record and tints
  the whole band, overriding the static section fill; a `Section_Visibility` condition suppresses the band per record
  like the static suppress flag. Previously only object-level conditional formats were honoured.
- **`ToText(<date/time>, "picture")` now formats the value instead of erroring.** The two-argument date/time overload of
  `ToText` (e.g. `ToText(CDate({?StatementDate}), "dd-MMM-yyyy")`, `ToText({date}, "yyyy-MM-dd")`) previously returned
  `EvalError::Unsupported`, which blanked the whole enclosing string concatenation. It now renders the Crystal picture
  tokens `d`/`dd`/`ddd`/`dddd`, `M`/`MM`/`MMM`/`MMMM`, `yy`/`yyyy`, `H`/`HH`/`h`/`hh`, `m`/`mm`, `s`/`ss`, `t`/`tt`,
  with separators and quoted runs passed through literally (month/weekday names + AM/PM from en-US). A datetime picture
  may mix date and time tokens in one string. Backed by a new `rpt_format_value::format_picture` helper.
- **The layout engine honours "Suppress If Blank Section", so a blank section reserves no vertical space.** A section
  flagged `EnableSuppressIfBlank` whose objects all resolve to no visible output (empty text/fields, no drawn border or
  visible fill) is now dropped during pagination instead of occupying its designed height. A report with a conditional
  group-footer note band (empty for most groups) previously accumulated that wasted height into one extra page; it now
  paginates to the same page count as the native engine.
- **The join builder follows link direction, so a table with redundant links is not reached backwards.** A stored
  `TableLink` is directional (`source.field = target.field` joins the target as a lookup onto the source). When a table
  was reachable by more than one link (a link-graph cycle), `rpt-query` could attach it by reversing a
  dimension-to-dimension link instead of via its own fact link — joining one dimension row to many fact rows, a
  cartesian blow-up. `join_order` now prefers forward links, then reversed links to a pure-source (fact) table, and only
  uses a reversed link into a dimension to break a deadlock — reaching each lookup from its fact.
- **Summaries over a formula field aggregate the evaluated value.** A declared summary whose summarized field is a
  `{@formula}` (e.g. `Sum of {@LineTotal}`) folded straight off the raw fetched row — which never holds a formula
  value — so every group and grand total was `Null`/blank. The fold now evaluates the formula per record before
  aggregating, matching a summary over a raw database field.
- **A placed summary field resolves by operation, not just field.** When one field carried several summaries (e.g. `Sum`
  and `Average` of it), the object showed the first summary of that field regardless of its operation (an `Avg` object
  rendered the `Sum`). Summary resolution now keys on `(operation, field, group scope)`.
- **A 1-argument (grand-total) summary resolves to the report total from any band.** `Sum({field})` with no group
  operand is the report grand total, but in a group/detail band it resolved to the innermost group's subtotal. It now
  always resolves against the grand total; the 2-argument form (`Sum({field}, {group})`) still resolves to the named
  group.
- **Summary functions inside a formula body resolve to the report's summaries.** A summary function over a reference —
  `Count({field}, {group})`, `Sum({field})` — evaluated to `Null` because only the array-literal aggregate form was
  supported. The formula engine now resolves such a call to the corresponding computed group/grand-total summary (the
  way the engine treats it — a reference to an existing subtotal), and a formula using one is classified
  `WhilePrintingRecords` so it runs after the summaries are computed.
- **Group header/footer areas are classified by their band marker, not their name.** A group band is named after its
  group field (RAS authoring writes e.g. `nameHeader`/`customeridHeader`, not `GroupHeaderArea1`), so the name-prefix
  heuristic silently classified those areas as `ReportHeader`. The area kind now comes from the authoritative
  band-marker record (`0x8d`–`0x99`) that parents each section, so a RAS-authored group lays out as `GroupHeader`/
  `GroupFooter` and its header/footer bands (and every field/summary in them) render. Reports whose area names are
  standard are unaffected (the marker yields the same kind).
- **Non-detail bands resolve their fields against the correct record.** A field/formula object in a report header,
  report footer, group footer, or page header/footer was evaluated with no current record, so database-field references
  (and formulas reading them) collapsed to `Null` and rendered blank. Each band now evaluates against Crystal's record
  context: the report's first record (report header), its last (report footer), the group's first/last record (group
  header/footer), and the page-boundary record (page header/footer). Detail bands and summary values are unchanged.
- **Record selection now resolves report parameters** (`build_dataset_with_params`): the selection and grouping formulas
  evaluate `{?Param}` against the supplied values, so a parameter-filtered report keeps the rows the parameters select
  instead of dropping every row. Previously the selection ran before parameters were attached, so a
  `{?Param}` reference failed to resolve and every record was dropped fail-open (an empty report). Unset declared
  parameters resolve to `Null`, so `HasValue({?Param})` reports false — matching the engine's treatment of an optional
  parameter left unset.
- **A declared parameter left unset at render binds to its stored default/current value.** When `rpt-render` ran a
  parameterized report with no `--param` override, the selection formula saw the parameter as unbound and skipped its
  filter, so every row rendered (e.g. an `IN`-list default selecting nothing). The CLI now binds each unsupplied
  parameter to its stored current value (else its default value (s)), so defaults feed the record selection exactly as
  when the engine runs a report accepting defaults; a parameter with no stored value still binds `Null`.
- **System-default integer fields group thousands** (`1,002`, not `1002`), matching the engine, which applies the same
  grouped number format to an integer field as to a decimal one; only the decimal places drop to zero.
- **The en-US system-default short date renders unpadded** (`M/d/yyyy`, e.g. `5/2/2023`), matching the engine, instead
  of zero-padding month and day (`05/02/2023`). A new `Locale::short_date_leading_zero` flag drives the system-default
  short-date pattern: false for en-US, true for the locales whose Windows short date pads (`dd/MM/yyyy` — en-GB, de-DE,
  fr-FR, es-ES, it-IT). Explicit stored date formats are unaffected.
- **Bar charts dispatch to the bar renderer explicitly** instead of the unsupported-type fallback, so a `Bar` chart no
  longer logs a spurious "chart type Bar is not yet supported" warning (the visual output was already a bar chart).
- **The gantt chart plots every datable record**, matching the engine. A hardcoded 60-row cap silently truncated a
  longer detail set (e.g. 109 project rows drawn as 60); the cap is now a high defensive guard (2000) that only trims a
  pathological set, with row-label thinning keeping dense charts legible.
- **"Hide (Drill-Down OK)" areas render no bands.** An area flagged `hide_for_drill_down` (the SDK's
  Hide-for-drill-down) was laid out normally, so a report that hides its detail to show a per-group summary dumped every
  detail row. The layout now contributes no bands for a hidden area (its structural bookkeeping — group level, footer
  pairing key, group format — is preserved), matching the engine's normal (non-drill-down) view.
- **An empty dataset renders the group-band skeleton instead of a blank page.** A report that defines groups but
  produced none (no records passed selection) dropped its whole body — the static content authored into the group
  header/footer (letterhead, column labels, totals captions) never rendered, so the page came out blank. The layout now
  emits the group headers (outermost→innermost) and footers (innermost→outermost) once for an empty grouped dataset,
  resolving field references against a synthetic empty row; no detail band is emitted.
- **Group selection applies a group-scoped summary at the level its group argument names.** A `GroupSelectionFormula`
  built from an inline summary — `DistinctCount({field}, {group}) > 1` — was fail-open (every group kept) because the
  safety gate rejected the summarized field and the filter only ran at the innermost leaf. The filter now allows
  references consumed by a group-scoped aggregation, resolves the summary from the group's computed subtotals, and
  prunes at the group level the summary's group argument names (matched on the full field name), dropping whole groups
  the way the engine does.
- **PDF text no longer inherits a leaked stroke** (doubled/haloed glyphs): a run drawn right after a line left krilla's
  stroke active, filling *and* stroking the glyphs; text now clears the stroke first.
- **`TotalPageCount` / `PageNofM` resolve the true final page count**, patched into every placed run once pagination
  completes (re-anchoring right/centre-aligned footers) instead of showing pages-so-far; the field also includes its
  `Page ` / ` of ` literals ("Page 1 of 37").
- **Conditional object visibility is evaluated** under its stored reserved name (was looked up under the wrong key), so
  per-row features like zebra backgrounds render.
- **Group-footer bands map to the correct group level by pairing each footer to its header by name** (the area name with
  its `Header`/`Footer` band token removed — `nameHeader`↔`nameFooter`), robust to whatever order the file presents
  footers in; a blind innermost-first reversal misattributed footers (and their subtotals) to the wrong nesting level,
  so an outer-group footer fired at every inner-group break. Falls back to the reversal when the names don't pair
  cleanly. **Group-scoped summaries disambiguate shared short names** by matching the full field name.
- **Lines and boxes read their stroke from the border record** (only thickness lives on the shape), so border-defined
  styles — including vertical column dividers — render instead of being dropped.
- **Row-background boxes underlay content and grow with the band**, covering a can-grow row's full height.
- **The PDF backend draws underline and strikethrough**, matching the other backends (HTML also gained strikethrough;
  existing output unchanged).
- **Pictures draw at their op-box size** (aspect/fit) and **detail text uses its stored font size** — fixing images
  drawn too large and mis-sized table text.
- **Conditional-format colours resolve to the correct hue**, and **page-header bands no longer render as black bars**.
- **A Postgres boolean bound to a String field reads as `1`/`0`** (psqlODBC `BoolsAsChar`), fixing a
  `{field} <> "1"` comparison broken by the live-DB path text-casting it to `true`/`false`.
- **Thin lines (~10 twips) render** (e.g. a footer divider), and **a table alias no longer decodes as its base name**.
- **The formula parser rejected valid Crystal, and the affected field rendered a wrong value.** A parenthesised group is
  a statement **sequence** with an optional trailing separator, not a single expression — how loop bodies and
  multi-statement branches are written (`Then (c := c + 1;) Else (c := c;)`). The parser failed at the first `;`, then
  compiled and evaluated the recovery AST, so a corpus report silently produced a wrong value.
- **A deeply-nested formula overflowed the stack and aborted the process.** `SIGABRT` cannot be caught, unlike a panic,
  so a pathological formula took down any host embedding the engine. Expression nesting is now capped at 128 with a
  diagnostic — far above anything a real formula reaches.
- **`codec::decode_contents` panicked on a length read from the stream.** The payload offset comes from the header
  record's own declared length, which on a damaged or non-report stream can point past the end. Now a `CodecError`.
- **A render that drops every row now says why.** `rpt-data`'s `DiagnosticSink` was attached by nothing outside its own
  tests, so every fail-open site produced silently wrong output — three corpus reports rendered zero rows from non-empty
  saved data and exited 0. A sink is now attached on every render path: the facade, `rpt-layout`'s subreport datasets,
  and the CLI. A selection that *failed* on every row is an error; one that cleanly *excluded* every row is a warning
  pointing at the report's parameters.
- **Failures that were previously swallowed are now reported.** Formula parse errors (once per formula at compile time,
  not per row, naming the formula and span — every one of the four compile sites bound them to `_`, so a syntax error
  evaluated from a partial parse with nothing said); EMF parse failures with their reason, distinguishing damaged data
  from a gap in this parser; a cross-tab cell's evaluation failure, which could blank a whole column silently; and the
  real reason no query could be built, in place of a synthesized "report has no database table".
- **Cause chains printed the same cause twice.** `connection failed: error connecting to server: error connecting to
  server: Connection refused`, and the same for a missing table. A variant that carries a `source` no longer
  interpolates it — applied to `rpt::Error::Io`, `DbError::{Connect,Query}`, and `rpt_json::Error`, and documented as a
  convention in `rpt::error`.
- **Panic sites in the formula engine and HTML backend hardened.** Formula text comes from an arbitrary `.rpt` and the
  engine is meant to be embeddable in an LSP, validator, or WASM sandbox, where a panic crashes the host. Seven internal
  invariants became `debug_assert!` plus a graceful release path — the new `EvalError::Internal` where a `Result` exists,
  a skipped op or object where none does.

### Removed

- **The dead error surface.** `rpt::Error::Record`, `rpt::RecordError`, and `ProjectErrorKind::UnknownRecord` are gone —
  publicly exported and documented, but with zero construction sites, because the `raise` layer is infallible by design
  and returns defaults for anything it cannot interpret. `Rpt::decode_coverage()` is how a decode gap is reported now.
  `Error::Project` survives behind the write-path clearance gate, with `UnclearedRecordEdit` as its remaining kind.
- **The blanket `From<std::io::Error> for rpt::Error`.** A contextless `?` on an I/O call no longer compiles, which is
  the point: it made losing the path the path of least resistance. Use `rpt::IoError::at(op, path, source)`.
- **The XML export surface.** `rpt xml-dump` (and its `--full` record-tree mode), the `rpt-xml` crate, the
  RptToXml-compatible serializers, and the committed XML baselines are gone; `rpt json-dump` and the JSON baselines
  replace them as the decode regression surface, and `rpt tree` remains for the record DOM. The XML shape existed to be
  diffed against the RptToXml oracle — an oracle that could not reach large parts of the model (charts, cross-tabs,
  maps, OLAP) and whose element/attribute vocabulary forced the exporter to reshape, rename, and re-serialize decoded
  values. A plain serde dump of the model is a strictly better regression surface and costs nothing to keep exhaustive.
  With it go the XML-only behaviours: the RptToXml-shaped `RecordSelectionFormula` re-serializer, the
  `SavedData/@RecordCount` element (the count is still available as `Rpt::saved_record_count`, and `rpt saved` prints
  it), and the `--locale` flag that selected a host locale for the export's runtime-resolved display formats.
- **The derived analytics, and with them the last derived values in any export.** `Field.UseCount`, parameter usage,
  `<SummaryFields>`, the locale-resolved effective field format, and the derived preferred view no longer exist in the
  export path. They were reconstructions of engine bookkeeping that is not in the file: `UseCount` is a live reference
  count in the running engine and was never fully correct, and the effective display format needs a locale and formula
  return types, i.e. a consumer's context. The dump is now a pure projection of the decode, which is the only reason a
  baseline diff can be read as "the decoder's reading of this file changed". Consumers that need a derivation compute it
  themselves; `rpt_model::analysis` holds the dependency-free ones more than one consumer needs (e.g.
  `field_object_value_type`).
- **Saved-data cipher brute-forcing.** The saved-data batch key is known, so the IV/key-search path was removed from the
  reader.

## [0.2.0]

This release turns `rpt-rs` from a reader/exporter into a full reporting engine: a complete render pipeline (data →
layout → Page IR → HTML/SVG/PDF/PNG), a formula evaluator, live-database support, saved-data decoding, a byte-faithful
writer, and the test corpus to validate it all against the native Crystal engine.

### Added

- **End-to-end report rendering.** A new render & data pipeline built purely on the decoded model:
    - **`rpt-data`** — the record pipeline: `RowSource` → record selection → sort → grouping → summaries → running
      totals, with the formula evaluation context (`Global`/`Shared` variables, per-record cache, evaluation-time
      scheduling). Two-field summaries (`WeightedAverage`, `Correlation`, `Covariance`) resolve to an empty/unavailable
      value rather than a plausible-but-wrong one until the second field is decoded.
    - **`rpt-layout`** — the layout & pagination engine: places every object at its twip position, paginates
      band-by-band, and honours the section-break controls (New Page Before/After, Keep Group Together, Print at Bottom
      of Page, Reset Page Number After, Underlay Following Sections). Resolves each field's display format from the
      locale + its stored format spec.
    - **`rpt-pages`** — the backend-agnostic Page IR: `Rect` / `Ellipse` / `Line` / `Text` / `Polygon` / `Image`
      draw-ops in twips, solid/gradient/hatch fills, rotated text runs, image assets, checkpoints, and fidelity
      diagnostics. `serde`-serializable — the frozen contract between layout and backends.
    - **Four output backends** — `rpt-render-html` (self-contained XHTML, images inlined), `rpt-render-svg` (one file
      per page), `rpt-render-pdf` (krilla with real font-subset embedding, plus a dependency-free fallback writer), and
      `rpt-render-raster` (tiny-skia → PNG per page).
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

- **Chart rendering — 16 chart types as native vector draw-ops** (no rasterization): bar (clustered / stacked / percent,
  multi-series), line, area, pie, doughnut, 3-D riser, 3-D surface, 3-D area, scatter, bubble, stock (hi-lo / OHLC),
  histogram, radar, gauge, funnel, and numeric-axis, plus a bar fallback (with a diagnostic) for types without a
  dedicated renderer. Matches the native engine's defaults: the full 20-colour palette, axis titles, tick density,
  compact temporal category labels with period bucketing (weekly/monthly/quarterly/…), label thinning on dense axes,
  family-dependent legend rules, and a perspective 3-D scene (corner room, floor grid, painter-sorted risers). Per-axis
  gridline modes are decoded from the chart styling record.

- **Cross-tab rendering.** A cross-tab pivots the dataset by row × column dimensions with an aggregate measure per cell
  and draws a native grid (cell rects, grid lines, headers, grand totals). Each dimension buckets by its stored grouping
  period — a date column grouped monthly produces one column per month (`M/YYYY`), not one per raw date — and every
  declared measure is rendered, stacked per cell (e.g. a Sum over a DistinctCount), each formatted for its operation.
  Current cut: one row dimension × one column dimension.

- **Live database support.** `rpt-query` generates the joined `SELECT` from the report's table/link graph — projecting
  only the tables and columns the report actually uses (matching the native engine's SQL and avoiding accidental
  cartesian joins) and pushing the translatable record-selection subset into `WHERE`. `rpt-db-postgres`
  and `rpt-db-sqlite` implement `RowSource` (native-only, isolated behind the trait so the portable core links no
  driver). Connection URLs are read only from the environment (`RPT_DB_URL`, `DATABASE_URL`,
  `RPT_DB_URL_<SERVER>`), never from flags.

- **Formula evaluation.** The `crystal-formula` engine gained a full evaluator: a bytecode VM (with a tree-walking
  reference evaluator), the builtin library across every family — string, math, conversion, date/time (incl.
  `DatePart` week-numbering modes), financial (`Pmt`/`FV`/`PV`/`NPV`/`IRR`/`Rate`/`DDB`/`SLN`/`SYD`), statistical
  (sample + population), and numeral (`ToWords`, `Roman`) — plus loop `Exit`, textual `#Month d, yyyy#` date literals,
  and a **semantic validation pass** (`validate`/`validate_str`): unknown/misspelled builtins with suggestions, arity
  and operator-type errors, and unknown field/parameter/formula references, as spanned, severity-tagged diagnostics.

- **Saved-data decoding.** A report saved with data now yields its schema (column names + types) and cached rows
  (`Report::saved_data`, the `rpt saved` subcommand) — both the external-memo and inline packed batch layouts decode,
  including the per-batch encryption and the memo heap.

- **`.rpt` writer (byte-faithful).** The decode pipeline is invertible at the record-substrate level:
  `Rpt::reencode` re-serializes, deflates, encrypts, and rewrites a valid `.rpt` that re-opens byte-identically at the
  logical level, and `Rpt::patch_record_leaf` overwrites a same-size region of a decoded record's leaf. Exposed as the
  `rpt reencode` / `rpt patch` subcommands. There is no model→records lowering yet.

- **New decoders in the reader:** charts (bindings, analytic layout, data-value labels, gridline modes), cross-tabs
  (dimensions, grid formats), object hyperlinks, hierarchical grouping, formula variables (name/type/scope), typed field
  sub-formats (number / currency / date / time / boolean / string masks), subreport re-import metadata, save metadata,
  designer state (rulers, guidelines), and the field-manager census. Every record type observed in the corpus is now
  named in the registry. `rpt` decode errors carry structured context (stream, byte offset, record type) via dedicated
  error types instead of message strings.

- **CLI additions.** `rpt inspect` shows each chart's data binding ("show value (s)" / "on change of");
  `rpt tree` prints the colorized decoded record DOM; `rpt streams` reports per-stream decode coverage;
  `rpt dump` is a byte-layout workbench (annotated hex, string scan, scalar probe, minimal-pair diff);
  `rpt saved` prints the decoded saved rows; `rpt sql` lists every SQL a report can run against its database (the
  generated join query + stored SQL Commands + SQL Expression fields, recursively through subreports, with
  connection/table provenance and a `--dialect` selector); `Report::objects()` / `objects_mut()` iterate all report
  objects.

- **Render-parity test corpus and infrastructure.** A committed corpus of 36 reports over one synthetic
  "parking" database, each with an XML decode baseline and an HTML render baseline, validated out-of-band against the
  native Crystal engine; golden-file tests for the Page IR and every backend; `docker-compose.yml` and a
  `Makefile` for the fixture database; DB-gated CI regression tests.

- **Documentation.** New guides: rendering (`docs/12-rendering.md`), a compile-verified render cookbook
  (`docs/13-render-examples.md`), and the five-part formula-engine set (architecture/VM, language reference, builtins,
  validation). GitHub-native Mermaid diagrams throughout, a rewritten README (status section, quick start, example
  render), and a docs↔code audit bringing the block catalog, support matrix, and saved-data docs in line with the code.

### Changed

- **Workspace restructured into a 20-crate, two-layer workspace** with compiler-enforced boundaries:
    - The semantic model moved out of `rpt` into the standalone, pure-data **`rpt-model`** crate (no I/O, WASM-safe,
      optional `serde`); `rpt` re-exports it as `rpt::model`. The whole render/data pipeline depends on `rpt-model`, not
      the decoder, so the render stack links no CFB/inflate. Byte-level provenance notes live in the documentation-only
      `rpt::provenance` module.
    - The formula language moved into the standalone **`crystal-formula`** crate (depends only on
      `rpt-format-value`), reusable without the binary reader (LSP, WASM sandbox, validator).
    - The `rpt-engine` crate was dissolved (its derived analytics now live in `rpt-cli`'s private
      `export::analysis`), and the `rpt-to-xml` binary was folded into `rpt-cli` as the **`rpt xml-dump`**
      subcommand — one `rpt` binary for all inspection and export. XML output is byte-identical.
- **Naming:** standalone "SAP" was dropped in favor of "Crystal Reports" across the README, docs, CLI help, and crate
  metadata.

### Removed

- The standalone **`rpt-to-xml`** binary (now `rpt xml-dump`) and the **`rpt-engine`** crate (dissolved into
  `rpt-cli` / `crystal-formula`), per the restructuring above.

## [0.1.0]

### Added

- **Saved data (stored rows).** Decodes the cached rows a report carries when saved with data (`SavedRecordsStream` +
  `MemoValuesStream`) and exports them as a `<SavedData>` element. See [`docs/06-saved-data.md`](docs/06-saved-data.md).
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
