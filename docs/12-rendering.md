# Rendering

The rendering layer turns a decoded [report model](05-semantic-model.md) plus a data source into paginated, rendered
output (HTML / SVG / PDF / PNG). It is built on the reader and is pure, WASM-safe Rust — the one native exception, the
live-database `RowSource`, is isolated behind a trait so the core never depends on it.

## The pipeline

```mermaid
flowchart TD
    RPT[".rpt file"] -->|" rpt (decode) "| MODEL["Report model<br/>(rpt-model)"]
    SRC["data source<br/>(saved data or live DB)"] --> DATA
    MODEL --> DATA["rpt-data<br/>RowSource → selection → sort → grouping → summaries"]
    DATA --> DS["Dataset"]
    DS --> LAYOUT["rpt-layout<br/>place objects · paginate · resolve values + formats"]
    LAYOUT --> IR["PagedDocument<br/>the backend-agnostic Page IR (rpt-pages)"]
    IR --> HTML["rpt-render-html → HTML"]
    IR --> SVG["rpt-render-svg → SVG"]
    IR --> PDF["rpt-render-pdf → PDF"]
    IR --> RASTER["rpt-render-raster → PNG"]
```

Each stage is a crate with one job (see [the codebase](10-codebase.md) for the full table):

| Stage        | Crate                                     | Role                                                                                                                                                                                                                                                                              |
|--------------|-------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| **Data**     | `rpt-data`                                | A `RowSource` feeds rows through record selection → sort → grouping → summaries into a `Dataset`. Carries the formula-evaluation context (`Global`/`Shared` variables, per-record cache).                                                                                         |
|              | `rpt-query` / `rpt-db-postgres`           | The live-DB path: `rpt-query` builds the joined `SELECT` over only the tables/columns the report references (unused tables are pruned rather than cross-joined) and pushes the translatable record-selection subset into `WHERE`; `rpt-db-postgres` executes it as a `RowSource`. |
| **Layout**   | `rpt-layout`                              | Walks the report's areas/sections over the `Dataset`, resolves each object's value + display format, places it at its twip position, and paginates band-by-band. Text metrics come from an injected `TextLayout`.                                                                 |
|              | `rpt-format-value`                        | Value → string (number / currency / date / time / bool), driven by a `Locale` merged with the field's stored format.                                                                                                                                                              |
|              | `rpt-text`                                | The real text stack (cosmic-text): font metrics + Unicode/CJK line-breaking behind the `TextLayout` trait.                                                                                                                                                                        |
|              | `metafile`                                | The standalone Windows-metafile (EMF) parser `rpt-layout` replays vector pictures through: it resolves the metafile's coordinate machinery and emits device-independent shapes via the `MetafileSink` trait. Dependency-free and WASM-safe.                                        |
| **IR**       | `rpt-pages`                               | The `PagedDocument` / `Page` / `DrawOp` intermediate representation every backend consumes.                                                                                                                                                                                       |
| **Backends** | `rpt-render-html` `-svg` `-pdf` `-raster` | Serialize the Page IR to HTML / SVG / PDF / PNG.                                                                                                                                                                                                                                  |
|              | `rpt-render-util`                         | Backend-serialization helpers shared by the four backends and the layout engine: twip↔unit constants, XML/HTML text escaping, stroke dash-pattern math — kept out of the frozen Page IR (WASM-safe, depends only on `rpt-pages`).                                                 |
| **Facade**   | `rpt-render`                              | Ties it together (`ReportDocument`, free functions). A library crate.                                                                                                                                                                                                             |
| **CLI**      | `rpt-render-cli` (`apps/`)                | The `rpt-render` binary: resolves the five render inputs (report, datasource, locale, parameters, output) and drives the facade.                                                                                                                                                  |

### WASM targets

The pipeline up to the Page IR is WASM-safe, but only two of the four backends are: **`rpt-render-html`
and `rpt-render-svg`** build for `wasm32-unknown-unknown`. **`rpt-render-raster`** (fontdb / fontdue / tiny-skia) and **
`rpt-render-pdf`**'s default krilla backend (fontdb) are native-only. Build the facade with `--no-default-features` for
wasm — `cosmic` is the library's *only* feature, and dropping it drops the system-font scan (inject a font-loaded
`CosmicLayout` via `render_dataset_with` instead). The DB drivers are never a concern here: they are features of the
**`rpt-render-cli`** binary, not of the `rpt-render` library, so the facade links no driver in any configuration. The
`wasm` CI job compiles the WASM-safe crates for `wasm32-unknown-unknown` on every push, so an accidental native-dep leak
fails CI.

## Driving a render (library API)

The SDK-shaped facade mirrors `ReportDocument`. Only `load` (decode) and the `export_*_to_disk` methods (file I/O) are
fallible; **rendering itself never fails** — a `PagedDocument` always comes back, with any fidelity problem reported as a
[diagnostic](#diagnostics) rather than an error:

```rust
use rpt_render::ReportDocument;

let doc = ReportDocument::load("report.rpt")?;   // decode
doc.export_html_to_disk("out.html")?;            // render saved data → HTML
let pdf: Vec<u8> = doc.to_pdf();                 // …or bytes
```

Under the facade are free functions for finer control. The pipeline default is the report's **saved data** (the offline
path); with no saved data it runs over zero rows (headers/footers still format):

```rust
use rpt_render::{render, render_pdf, render_html, render_svg_pages};

let pages = render(report);                 // Report → PagedDocument (saved data)
let pdf   = render_pdf(report);             // → PDF bytes
let html  = render_html(report);            // → one self-contained HTML document
let svgs  = render_svg_pages(report);       // → Vec<String>, one SVG per page
```

To render from a **pre-built `Dataset`** (e.g. a live datasource), with an explicit locale and per-subreport scope rows,
use the options-driven entry point with `RenderSource::Dataset`:

```rust
use rpt_render::{render_with, Locale, RenderOptions, RenderSource};

let doc = render_with(
    report,
    RenderOptions {
        datasource: RenderSource::Dataset(&dataset),
        locale: Locale::from_tag("de-DE"),
        scope: Some(&scope_data),
        ..RenderOptions::default()
    },
);
for diag in &doc.diagnostics { /* fidelity warnings, see below */ }
```

For a full end-to-end render cookbook — a custom `RowSource`, the live-DB library path, WASM, and error handling —
see [Render examples](13-render-examples.md).

- **`Locale`** (from `rpt-format-value`, re-exported): the render locale — separators, month/day names, AM/PM.
  `Locale::from_tag("en-US" | "de-DE" | …)`; unknown tags fall back to en-US.
  See [format resolution](#format-resolution).
- **`ScopeData`**: supplies each subreport scope's rows so a whole tree renders from a live datasource, without
  `rpt-layout` depending on any DB crate. `None` renders subreports from their saved data. An **inline** subreport runs
  once per placement against a **per-instance** dataset filtered by the enclosing row's link values — a
  parameter-routed link (`SubreportLink.linked_parameter`) binds the parent field into the subreport's parameters and a
  direct field link applies a structural equality filter (`rpt_data::build_dataset_with` + `FieldFilter`). `Shared`
  variables accumulated inside a subreport are visible to the main report (shared eval scope). An **on-demand**
  subreport (`SubreportObject.on_demand`) is not executed — it emits only its caption placeholder, matching the
  engine's click-to-expand behaviour in a static export. A subreport taller than its placeholder box **grows the
  enclosing band**: it is formatted once ahead of pagination (so its `Shared`/`Global` writes fire exactly once), the
  band grows to its full height (reusing the can-grow machinery), and the checkpoint pagination flows the enlarged band
  across pages. A subreport taller than a **whole page** is split across parent pages at row boundaries (distinct
  op-bottom Y values): still formatted once, the split is pure geometry over the cached ops, so a subreport with an
  internal forced page break puts each of its pages on its own parent page. A subreport that fits on a page is placed
  atomically (moved whole to the next page when the space left is too small).
- **`TextLayout`**: inject `rpt-text::CosmicLayout` (the default when the `cosmic` feature is on) for font-accurate
  metrics, or the dependency-free `ApproxLayout`, via `render_dataset_with`.
- **`as_of`** (`DateTimeSpecials`): the render's "current" instant, resolving the date/time formula specials
  `CurrentDate` (and its alias `Today`), `CurrentDateTime`, and `CurrentTime`. Captured once so the whole render is
  deterministic — the record pipeline (selection/grouping formulas) and every layout context (including crosstab pivots
  and subreports) share one fixed value. `None` (the default) captures the system clock at render start via
  `default_as_of`; set it explicitly for a reproducible render (frozen baselines, oracle diffs). The render core reads
  no clock — the instant is captured at the facade/CLI entry, and a WASM build falls back to the Unix epoch unless the
  host supplies `as_of`.

## The Page IR (`rpt-pages`)

A `PagedDocument` is `{ pages, checkpoints, diagnostics, assets }` — `assets` holds the out-of-band image bytes an
`Image` op references by id (see below). A `Page` is `{ number, size, origin, ops }` where each
`DrawOp` is a `Rect`, `Ellipse`, `Line`, `Text`, `Polygon`, or `Image` primitive in **twips** (1/1440 inch). The IR is
`serde`-serializable so it can be frozen for tests and diffed independently of any backend.

An `Ellipse` is an axis-aligned ellipse inscribed in its `bounds` (exact round pie centres / bubble markers, which a
`Polygon` can only approximate). A `TextRun` carries a `rotation` (degrees CCW about the run's top-left; `0.0` =
upright, a no-op every backend renders byte-identically to unrotated). A `Rect`/`Ellipse`/`Polygon` `fill` is a `Fill`:
`Solid(Color)`, `LinearGradient { stops, angle_deg }`, or `Hatch { fg, bg, pattern }`. Every backend renders `Solid`
exactly; the SVG backend emits real `<linearGradient>`/`<pattern>` defs, while PDF/raster/HTML fall back to a
representative solid colour for gradient/hatch (a gradient's midpoint stop, a hatch's foreground).

A raster picture object becomes an `Image` op referencing a `PagedDocument` asset (a browser-renderable
BMP/PNG/JPEG/GIF)
that a backend inlines, each keyed by a content hash so identical bytes — a repeated logo, duplicate thumbnails — cost a
single embed shared across every placement: the **HTML** backend embeds each distinct image once as a `background-image`
CSS class referenced by class; the **PDF** backend embeds each once as an image XObject (PNG/JPEG/GIF via krilla, BMP
decoded in-crate to RGBA); the **SVG** backend inlines each once as an `<image>` in `<defs>` (a `data:` URI) referenced
by `<use>` at each placement (BMP/PNG/JPEG/GIF all embed as-is — browsers decode them); and the **raster** backend
decodes each distinct image once to a bitmap (PNG via tiny-skia, BMP in-crate, JPEG via `zune-jpeg`, GIF's first frame
via the `gif` crate) and composites it into each placement box. A backend that can't inline/decode the format, or an op with no matching asset, draws a placeholder. A
**database blob field** ({image} column) resolves its per-row bytes the same way — each row gets a distinct asset.

An `Image` op carries a `fit: ImageFit` (`Fill` or `Contain`) governing how the raster maps to its box. Pictures and
blob fields use `Contain`: the raster is scaled uniformly to the largest size that fits, preserving its source pixel
aspect ratio, and centered — the surrounding space is left empty (letterbox), matching the native engine rather than
distorting. `Fill` (the default, used for pre-sized chart-island rasters and placeholders) stretches to the box on
both axes. HTML expresses `Contain` with `background-size:contain`, SVG with `preserveAspectRatio="xMidYMid meet"`, and
the PDF/raster backends compute the centered fitted sub-rect from the decoded image's pixel dimensions. A
binary column is fetched raw (no `::text` cast) and carried through the pipeline as `Value::Bytes`; the Postgres `\x`
hex-escape `bytea` text form is still accepted and decoded back to the original bytes (saved-data path). An **EMF**
(Enhanced Metafile) picture is a vector command stream, not raster bytes, so it is parsed by the standalone `metafile`
crate and replayed instead: `rpt-layout` implements that crate's `MetafileSink`, turning each resolved shape into a
native draw-op (line / polygon / ellipse / rect / text) scaled into the object's box. A bad or truncated stream falls
back to the placeholder with a diagnostic. WMF and OLE-embedded presentations are still placeholders.

### Coordinate model

Draw-op coordinates are **printable-relative** (0-based: `0,0` is the top-left of the printable area, the margin
removed). Each page carries `origin` — the report's top-left margin. A backend re-applies it **once**, in the way that
backend needs, instead of the margin being baked into every coordinate:

- **HTML** draws content 0-based inside a container that carries the margin as CSS (matching the engine's RAS host).
- **SVG / PDF / raster** are physical pages, so they add `origin` to every coordinate (an SVG `translate` group, a PDF
  `cm` transform, a raster pixel offset).

This keeps the whole coordinate model in one place; there is no `±margin` scattered across position sites.

### Diagnostics

Rendering collects into `PagedDocument.diagnostics` everything it worked around — the deep issues that would otherwise
never reach the caller. Each `Diagnostic` carries a severity, a kind, a message, the object/formula it is about, and a
`DiagnosticLocation`.

Two families arrive there, in one vocabulary:

- **Layout/render fidelity.** An object that falls back to a placeholder box (a chart with no plottable group series, a
  WMF / OLE-embedded picture), an unimplemented formula builtin, a runtime formula error, a substituted font.
- **Data-pipeline fail-open.** The record pipeline is deliberately fail-open — a record-selection formula that errors
  **drops the row**, a `{@formula}` that errors resolves to `Null`, a group-selection failure **keeps** the group, an
  unsupported group condition falls back to raw-value grouping, and a cell that will not parse as its declared type is
  coerced. That behaviour is right (one broken formula must not abort a render) but it is silent, and enough dropped
  rows renders an empty report that reports success. `rpt-data` reports each occurrence to a `DiagnosticSink`;
  `render_with` attaches one, and `rpt-layout` does the same for every subreport dataset.

The two sides are bridged in `rpt-layout`'s `diagnostics` module — the only crate depending on both — so `rpt-data`
keeps no `rpt-pages` dependency and stays WASM-safe. The conversion loses nothing, and **adds** the severity by an
explicit rule: a fail-open that *discards data* is an `Error`, one that keeps the data but formats or groups it
differently is a `Warning`.

`DiagnosticLocation { page, area, section, record_index, span }` is all-optional and **never fabricated** — a site fills
in only what it genuinely has, the same convention `rpt::StreamLoc` uses for decode errors. `span` is the byte range
within the formula text (the evaluator's `eval_spanned` supplies it); `record_index` is what distinguishes one bad row
from a formula that fails on every row. `Diagnostic::describe()` renders the one-line form a CLI prints.

The CLI surfaces all of it into its warning summary, printing errors through a channel `-q` cannot suppress and
collapsing identical repeats (`… [record 0] — and 606 more like it`) so a per-row failure cannot bury the summary that
explains it.

## Charts and cross-tabs

Both charts and cross-tabs render as **ordinary Page-IR draw-ops** — rects, lines, polygons, ellipses, and text — with
**no rasterization** and no new dependency. The decision is deliberate: emitting native primitives means a chart or grid
renders identically through every backend (HTML / SVG / PDF / raster) and needs no per-backend image embedding.

- **Charts.** The corpus charts are *group charts*: one data point per group, its value the group's summary of the
  charted field — data the layout engine already computes. Dispatch lives in `rpt-layout`'s `chart/` module, one
  renderer per shape keyed off the decoded `ChartGraphType`. Sixteen chart types are named and drawn — bar, line, area,
  pie, doughnut, 3-D riser, 3-D surface, scatter, radar, bubble, stock, numeric-axis, gauge, Gantt, funnel, and
  histogram — plus a verbatim `Other` fallback; the inherently three-dimensional families (3-D riser / surface, and the
  depth-effect area ribbon) take a perspective-riser path. A type without a dedicated renderer falls back to bars, and a
  chart with no plottable group series falls back to a placeholder box, each with a diagnostic.
- **Cross-tabs.** A cross-tab pivots the dataset by row × column dimensions with an aggregate measure per cell, drawn as
  a native grid (cell rects + grid lines + text) by `rpt-layout`'s `crosstab` module. The current cut handles one row
  dimension × one column dimension × the first measure (the shape of every corpus cross-tab); nested multi-level axes
  are a follow-up.

The per-shape geometry (axis frames, label thinning, riser projection, pivot computation) lives in the crate rustdoc for
`rpt-layout`'s `chart`/`crosstab` modules — see `cargo doc -p rpt-layout`.

## Section-break & pagination controls

`rpt-layout` paginates band-by-band (a band never splits mid-section: one that would overflow the body moves whole to a
new page). On top of that it honours the section/group format flags decoded onto `SectionAreaFormatBase` /
`GroupAreaFormat`:

- **New Page Before / After** (`new_page_before` / `new_page_after`) — start a fresh page before a band, or after it
  (deferred to the next flow band so a trailing break leaves no blank page). Applied on both the single-column band path
  and the multi-column detail path; a break at the top of a fresh page is skipped so no leading blank page appears.
- **Keep Group Together** (`GroupAreaFormat.keep_group_together`) — before emitting a group header, the group's subtree
  (header + details + footer, nested subgroups recursively) is pre-measured from static design heights; if it would
  split across the current page boundary but fits on a page by itself, the whole group moves to a fresh page. A group
  taller than a full page is left to paginate naturally. The pre-measure deliberately ignores can-grow growth —
  resolving it would re-fire `WhilePrintingRecords` variable writes.
- **Print at Bottom of Page** (`print_at_bottom_of_page`) — pin a group/report footer against the body bottom (above the
  page footer), then treat the page as full so the next band starts fresh.
- **Reset Page Number After** (`reset_page_number_after`) — restart the page-number counter at the next page top, giving
  per-group page numbering. `PageNumber` / `Page N of M` follow the reset; `TotalPageCount` stays the whole-document
  count (a per-section total would need a second pass).
- **`TotalPageCount` / `PageNofM`** are a forward reference the single layout pass cannot know up front. Each placed run
  is recorded and rewritten with the true final page count once pagination completes (its stored advance recomputed so a
  right/centre-aligned footer re-anchors). The displayed page number is preserved (it already honoured any reset).
- **Underlay Following Sections** (`SectionAreaFormatBase.underlay_section`, SDK `EnableUnderlaySection`) — an underlay
  band is a background for the sections that follow it: after it emits, the flow cursor stays at the band's top so the
  next band overlays it in the same vertical space rather than being pushed below.
- **Suppress If Blank Section** (`SectionFormat.suppress_if_blank`, SDK `EnableSuppressIfBlank`) — a section whose objects
  all resolve to no visible output is dropped and reserves no vertical space, so it neither renders nor pushes following
  bands (and cannot force an extra page). A section is "blank" when every object is suppressed or is an empty
  text/field/heading with no drawn border and no visible (opaque, non-white) fill; any shape, picture, chart, cross-tab,
  blob, subreport, or non-empty text keeps it. Its formulas still evaluate (needed to decide blankness), so their
  record-time side effects fire.
- **Group-footer level order.** Group footers are stored innermost-first in the report (the canonical
  `GH1..GHN, Detail, GFN..GF1` area order) while group headers are outermost-first; the footer list is reversed at
  collect time so both index by group level.
- **Band record context.** A non-detail band still resolves its field/formula objects against a "current record", the
  way Crystal does — otherwise a header/footer `{table.field}` (or a formula reading one) would evaluate to `Null` and
  render blank. Each band picks its record: report header → the report's first record, report footer → its last, group
  header → the group's first record, group footer → its last, page header/footer → the record straddling the page
  boundary (tracked as detail rows print). Summary/`GroupName`/special objects don't need a row and resolve from the
  print state regardless. This context depends on the area being classified correctly — the area kind comes from the
  band-marker record (`0x8d`–`0x99`), not the area name, so a group area a report tool named after its group field (e.g.
  `nameHeader`) still lays out as a group band.

Within a band, box objects are emitted before the band's text/field/image ops so a shading box underlays the row content
even when stored after it, and a section-spanning box (its design bottom reaching most of the section height) grows with
a can-grow band so its fill/frame tracks the actual rendered row height.

Formula-driven conditional variants of these flags are not yet applied (they wait on section condition-formula
plumbing).

## Format resolution

A field's displayed value is resolved from **two layers** (see `rpt-layout`'s format module):

1. the **locale** — the "system default" layer: separators, month/day names, AM/PM, default date order, default
   decimals, currency symbol; and
2. the field's **stored `FieldFormat`** leaf — the explicit authoring choices (decimals, negative style, currency symbol
   placement, date component forms, boolean word pair).

The field's own `use_system_defaults` flag arbitrates: when set, the locale supplies the effective format; otherwise the
stored leaf wins for the attributes it sets, with names/separators still coming from the locale (Crystal never stores
"January", only `MonthFormat::LongMonth`). This mirrors the native engine's runtime format resolution. The stored
numeric leaf's `thousands_separator` (grouping on/off), `suppress_if_zero` (a zero value renders blank),
`currency_position` (leading/trailing symbol placement), and the datetime leaf's `separator` (the string joining the
date and time parts) are all honoured for an explicit field; a system-default field takes grouping and symbol placement
from the locale.

Not every decoded model field is a render input. These stored facts are exported/inspected but intentionally do **not**
feed the render pipeline (no wiring needed): `SummaryInfo.revision_number` / `last_saved_by` / `saved_printer_name`,
`ReportOptions.enable_verify_on_every_print` / `convert_null_field_to_default`, and
`Table.qualified_name`. Conversely, a field's `FieldValueType` (drives the format-spec branch above) and a chart's
`data_refs` / `category_refs` are verified render inputs.

## Paragraph typography

A text object is a tree of paragraphs, each a run of styled text. The layout engine (`rpt-layout`'s `paginate`/`place`)
honors the per-paragraph formatting instead of flattening the object to one font at single spacing:

- **Per-paragraph font.** Each paragraph is placed in its own run font (a run's stored font override, else the object
  font), so a paragraph's point size drives its own wrap width, line pitch, and ascent — a multi-paragraph object
  mixing sizes draws each paragraph at its size.
- **Line spacing.** `IndentAndSpacingFormat.line_spacing` (decoded from the paragraph leaf) gives each line its pitch: a
  `Multiple` value scales the font's natural line height (1.0 single, 1.5, 2.0 double), an `Exact` value pins it to a
  twip pitch. Line pitches are summed for the can-grow band height (unequal lines, not count × one).
- **Justified alignment.** The layout marks every wrapped line of a justified paragraph `Justified` except the last
  (which stays `Left`, as typography never stretches a final line). Every backend flushes both edges: SVG via
  `textLength`/`lengthAdjust`, HTML via `text-align-last:justify`, the basic PDF writer via the `Tw` word-spacing
  operator (the krilla writer draws the line word-by-word), and the raster backend by spreading the slack across the
  inter-word gaps.
- **Text rotation.** A quarter-turn `TextRotationAngle` (90°/270°) flows the text along the box's tall axis (wrapping
  against the height) and stacks the wrapped lines as columns across the width; each run carries the angle and all four
  backends rotate it about the run's top-left (SVG/PDF/raster as a transform, HTML as a CSS `rotate` transform),
  reading up for 90° and down for 270°.

## The `rpt-render` CLI

One entrypoint for the five inputs a render needs — report, datasource, locale, parameters, output:

```
rpt-render <report.rpt> [OPTIONS]
  --saved | --db            saved data (default) or a live database (URL from the environment)
  --list-sources            print the report's data sources + the env var to set for each, then exit
  -p, --param Name=Value    repeatable; repeat a name for a multi-value parameter
  --locale <tag>            e.g. en-US, de-DE (default: the host locale, else en-US)
  -f, --format html|pdf|svg|png     default: inferred from -o's extension, else html
  -o, --output <path>       output file; '-' or omitted writes to stdout
  --force                   overwrite existing multi-file (SVG/PNG) pages
```

HTML and PDF are single self-contained files (safe to pipe to stdout). SVG and PNG are one file per page
(`<base>-N.svg` / `<base>-N.png`), so they need a real `-o` path. Live-DB connection URLs — including any password — are
read **only** from the environment (`RPT_DB_URL` / `DATABASE_URL`, or `RPT_DB_URL_<SERVER>` per source), never from a
flag, so they do not appear in `ps` output or shell history.

## Testing the renderer

The saved-data corpus is sparse, so the renderer is exercised against committed, first-class **render-parity test
corpora**: one synthetic database read *identically* by (a) our renderer and (b) the native Crystal engine, so the only
variable under test is rendering, not data.

**Single DB technology: PostgreSQL only.** SQLite's typelessness reintroduces the boolean/decimal/order variables the
corpora exist to remove between our renderer and the Crystal engine, so it is left out of the *parity* corpora (the
`rpt-db-sqlite` adapter + its own unit tests are kept, just not part of them). Our render is backend-independent
anyway — every column is re-typed against the report's declared field types, not the DB's — so a row set produces the
same output regardless of which engine served it.

### Meridian — the corpus going forward

`tests/meridian/` is a self-contained synthetic universe: **Meridian Global Logistics**, an invented third-party
logistics group whose data is deliberately shaped (a Pareto revenue split, a Q4 peak, a mid-year fuel spike, one
chronically late carrier, a multi-year build-out program) so that Top-N reports, trend charts, OHLC charts, scorecard
gauges, and Gantt charts all have something real to show. Everything about it is fictional and safe to publish.

| Path                              | What it is                                                                                              |
|-----------------------------------|---------------------------------------------------------------------------------------------------------|
| `tests/meridian/README.md`        | The company story — what the divisions are and what the numbers mean.                                    |
| `tests/meridian/SCHEMA.md`        | Every table and field, and how each feeds the reports/charts.                                             |
| `tests/meridian/sql/meridian.sql` | The **one** seed database, read by every report in the corpus. Generated deterministically by `apps/meridian-seed` from a fixed PRNG seed, so it is reproducible without shipping a database dump. |
| `tests/meridian/reports/**`       | The `.rpt` files, organized by division (`executive`, `freight`, `projects`, `sales`, plus `probes` for single-feature isolations). |
| `tests/meridian/baselines/html/`  | One HTML baseline per report, mirroring its path under `reports/` with `.rpt`→`.html`.                    |

### Legacy fixtures (`tests/fixtures/`)

The older group-shared corpus, **deprecated in favour of Meridian** but still driven by the same harness for as long as
it exists. Its structure is `sql/<group>/<name>.sql` for a per-report seed, or `sql/<group>/<group>.sql` for a
**group-shared** one read by every report under `reports/<group>/`; baselines mirror the `<group>/<name>` path. The
group-shared `parking` seed (`sql/parking/parking.sql`, 36 reports) is a synthetic car-park booking domain whose columns
are typed to exercise every formatting path — `DATE`/`TIME`/`TIMESTAMP`, `DECIMAL` with negative values, `BOOLEAN`
word-pairs, short+long nullable notes for can-grow/wrap, grouping and cross-tab columns, and a `total` for
summaries/running-totals/charts.

### CI regression layer (our side, zero external process)

`crates/rpt-render/tests/postgres_fixtures.rs` drives **both** corpora: it seeds each committed `.sql` into a PostgreSQL
server, renders every matching report through the whole pipeline with the deterministic `ApproxLayout` (no system
fonts → host-independent bytes), and diffs the HTML against the committed baseline. It **skips** when no `RPT_DB_URL` is
set, so a DB-less `cargo test` stays green; CI provides a `postgres` service. `docker-compose.yml` brings up the local
server on the project-wide port 55432. Re-bless after an intentional render change:

```sh
RPT_DB_URL=postgres://rpt:rpt@localhost:55432/rptfixtures \
  RPT_BLESS=1 cargo test -p rpt-render --test postgres_fixtures
```

### Typography baselines (no database)

`crates/rpt-render/tests/typography_baselines.rs` covers the five **data-free** fixtures under
`tests/fixtures/reports/typography/` — blank reports whose Report Header holds static text objects, each isolating one
font/text axis (face, size, style flags, colour + alignment, paragraph indent/spacing). They bind no datasource, so this
harness needs no database and runs on every checkout. Two baselines per fixture: the HTML backend's output
(`baselines/html/typography/`) and the **normalized Page IR** (`baselines/page-ir/typography/`). The Page IR is the
structural contract — op kinds, twip positions, text, resolved font — so it is the baseline that actually pins layout
behaviour; the HTML is the serialization check on top. Re-bless with
`RPT_BLESS=1 cargo test -p rpt-render --test typography_baselines`.

These freeze *our* behaviour, not engine parity: a baseline changing when a known divergence gets implemented is the
expected signal. Like the other committed render baselines, they are blessed against the project's font environment, so
a machine with a different set of installed faces will legitimately differ.

### Decode baselines

Independently of rendering, `apps/rpt-cli/tests/json_baseline.rs` runs `rpt json-dump` over every fixture under
`tests/fixtures/reports/` and diffs it against `tests/fixtures/baselines/json/<group>/<name>.json`, so authoring a
report also pins the *decoder* — exhaustively, since the dump serializes every decoded field. It needs no sandbox: the
dump depends on nothing outside the `.rpt`, so it is byte-identical on any machine, and it skips only when the fixtures
are absent. Re-bless with `RPT_BLESS=1 cargo test -p rpt-cli --test json_baseline`.

### Cross-engine oracle (out-of-band, maintainer machine)

The blessed baselines are validated out-of-band on a maintainer machine by rendering **both** stacks over the *same*
live Postgres — ours (`rpt-render --db`) and the reference implementation — and scoring positioned
parity:

- **Whole-report HTML** — an out-of-band maintainer sweep renders every corpus report through both stacks and scores
  positioned text. Text *positions* are the signal; absolute text *widths* differ under `ApproxLayout`-vs-Uniscribe
  metrics, so widths are not scored.
- **Chart vector geometry** — the engine's page-1 chart draw-ops (extracted from its EMF metafile) are compared against
  our Page IR, scored on mark family / count / position rather than text density.
- **Whole-page vector** — a whole-page comparison scores our page draw-ops against the engine's page EMF across the
  *whole* page (text-run anchors + boxes + lines, not just the chart), recovering the constant page-origin offset from
  unique text anchors first. The engine embeds a page EMF only for chart-containing pages; text-only pages fall to the
  HTML oracle.

Because the native engine only ever queries the **used** tables/columns, the live-DB path prunes the same way
(`rpt-query::build_query_for_report`); this is what keeps a report with unused/unlinked tables from cross-joining into a
cartesian (see the pipeline table above).

### Feature coverage

Both corpora are authored **incrementally** — from an empty report (no bound table → static bands only) upward, one
feature at a time — and each report is committed with its render baseline. Between them:

| Feature                                | Where                                                                                                                                |
|----------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------|
| Charts                                 | the legacy `orders_*` family (bar/3-D/area/pie/doughnut/funnel/gauge/surface + legend variants) and the 6 advanced `chart_*` reports (scatter/stock/histogram/radar/gantt); in Meridian, the volume trend, mode split, fuel OHLC, carrier scorecard, and capital-projects Gantt |
| Cross-tabs                             | 11 legacy `crosstab_*` reports (grid + grand-total / suppress / margin / repeat-label variants); Meridian's revenue and invoice-aging cross-tabs |
| Grouping / summaries / running totals  | 18 legacy `orders_*` reports (group on `state`, date-group granularity); Meridian's sales-by-region, Top-10, and statement reports    |
| **Subreports**                         | 6 Meridian reports (customer statement, sales by region, shipment tracking, fuel price/revenue, executive dashboard, shared-variable probe) — the gap the legacy corpus never covered |
| Formula-engine surface                 | Meridian's `probes/` set — Basic syntax, string functions, `Shared` variables, summary operations, SQL Expression fields, page-number reset |
| Typography (data-free)                 | the 5 `typography` fixtures — see above                                                                                              |

---

← [Usage](11-usage.md) · [Index](README.md) · **Next:** [Render examples](13-render-examples.md) →
