# Rendering

The rendering layer turns a decoded [report model](05-semantic-model.md) plus a data source into paginated,
rendered output (HTML / SVG / PDF / PNG). It is built on the reader and is pure, WASM-safe Rust — the one native
exception, the live-database `RowSource`, is isolated behind a trait so the core never depends on it.

## The pipeline

```mermaid
flowchart TD
    RPT[".rpt file"] -->|"rpt (decode)"| MODEL["Report model<br/>(rpt-model)"]
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

Each stage is a crate with one job (see [the codebase](07-codebase.md) for the full table):

| Stage | Crate | Role |
| ----- | ----- | ---- |
| **Data** | `rpt-data` | A `RowSource` feeds rows through record selection → sort → grouping → summaries into a `Dataset`. Carries the formula-evaluation context (`Global`/`Shared` variables, per-record cache). |
| | `rpt-query` / `rpt-db-postgres` | The live-DB path: `rpt-query` builds the joined `SELECT` over only the tables/columns the report references (unused tables are pruned rather than cross-joined) and pushes the translatable record-selection subset into `WHERE`; `rpt-db-postgres` executes it as a `RowSource`. |
| **Layout** | `rpt-layout` | Walks the report's areas/sections over the `Dataset`, resolves each object's value + display format, places it at its twip position, and paginates band-by-band. Text metrics come from an injected `TextLayout`. |
| | `rpt-format-value` | Value → string (number / currency / date / time / bool), driven by a `Locale` merged with the field's stored format. |
| | `rpt-text` | The real text stack (cosmic-text): font metrics + Unicode/CJK line-breaking behind the `TextLayout` trait. |
| **IR** | `rpt-pages` | The `PagedDocument` / `Page` / `DrawOp` intermediate representation every backend consumes. |
| **Backends** | `rpt-render-html` `-svg` `-pdf` `-raster` | Serialize the Page IR to HTML / SVG / PDF / PNG. |
| | `rpt-render-util` | Backend-serialization helpers shared by the four backends and the layout engine: twip↔unit constants, XML/HTML text escaping, stroke dash-pattern math — kept out of the frozen Page IR (WASM-safe, depends only on `rpt-pages`). |
| **Facade** | `rpt-render` | Ties it together (`ReportDocument`, free functions). A library crate. |
| **CLI** | `rpt-render-cli` | The `rpt-render` binary: resolves the five render inputs (report, datasource, locale, parameters, output) and drives the facade. |

### WASM targets

The pipeline up to the Page IR is WASM-safe, but only two of the four backends are: **`rpt-render-html`
and `rpt-render-svg`** build for `wasm32-unknown-unknown`. **`rpt-render-raster`** (fontdb / fontdue /
tiny-skia) and **`rpt-render-pdf`**'s default krilla backend (fontdb) are native-only. Build the facade with
`--no-default-features` for wasm — this drops the `cosmic` system-font scan (inject a font-loaded
`CosmicLayout` via `render_dataset_with` instead) and the native DB drivers. The `wasm` CI job compiles the
WASM-safe crates for `wasm32-unknown-unknown` on every push, so an accidental native-dep leak fails CI.

## Driving a render (library API)

The SDK-shaped facade mirrors `ReportDocument`:

```rust
use rpt_render::ReportDocument;

let doc = ReportDocument::load("report.rpt")?;   // decode
doc.export_html_to_disk("out.html")?;            // render saved data → HTML
let pdf: Vec<u8> = doc.to_pdf();                 // …or bytes
```

Under the facade are free functions for finer control. The pipeline default is the report's **saved data** (the
offline path); with no saved data it runs over zero rows (headers/footers still format):

```rust
use rpt_render::{render, render_pdf, render_html, render_svg_pages};

let pages = render(report);                 // Report → PagedDocument (saved data)
let pdf   = render_pdf(report);             // → PDF bytes
let html  = render_html(report);            // → one self-contained HTML document
let svgs  = render_svg_pages(report);       // → Vec<String>, one SVG per page
```

To render from a **pre-built `Dataset`** (e.g. a live datasource), with an explicit locale and per-subreport
scope rows, use the options-driven entry point with `RenderSource::Dataset`:

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
)?;
for diag in &doc.diagnostics { /* fidelity warnings, see below */ }
```

For a full end-to-end render cookbook — a custom `RowSource`, the live-DB library path, WASM, and error handling
— see [Render examples](12-render-examples.md).

- **`Locale`** (from `rpt-format-value`, re-exported): the render locale — separators, month/day names, AM/PM.
  `Locale::from_tag("en-US" | "de-DE" | …)`; unknown tags fall back to en-US. See [format resolution](#format-resolution).
- **`ScopeData`**: supplies each subreport scope's rows so a whole tree renders from a live datasource, without
  `rpt-layout` depending on any DB crate. `None` renders subreports from their saved data.
- **`TextLayout`**: inject `rpt-text::CosmicLayout` (the default when the `cosmic` feature is on) for
  font-accurate metrics, or the dependency-free `ApproxLayout`, via `render_dataset_with`.

## The Page IR (`rpt-pages`)

A `PagedDocument` is `{ pages, checkpoints, diagnostics, assets }` — `assets` holds the out-of-band image bytes an
`Image` op references by id (see below). A `Page` is `{ number, size, origin, ops }` where each
`DrawOp` is a `Rect`, `Ellipse`, `Line`, `Text`, `Polygon`, or `Image` primitive in **twips** (1/1440 inch). The IR is
`serde`-serializable so it can be frozen for tests and diffed independently of any backend.

An `Ellipse` is an axis-aligned ellipse inscribed in its `bounds` (exact round pie centres / bubble markers, which a
`Polygon` can only approximate). A `TextRun` carries a `rotation` (degrees CCW about the run's top-left; `0.0` = upright,
a no-op every backend renders byte-identically to unrotated). A `Rect`/`Ellipse`/`Polygon` `fill` is a `Fill`:
`Solid(Color)`, `LinearGradient { stops, angle_deg }`, or `Hatch { fg, bg, pattern }`. Every backend renders `Solid`
exactly; the SVG backend emits real `<linearGradient>`/`<pattern>` defs, while PDF/raster/HTML fall back to a
representative solid colour for gradient/hatch (a gradient's midpoint stop, a hatch's foreground).

A raster picture object becomes an `Image` op referencing a `PagedDocument` asset (a browser-renderable BMP/PNG/JPEG/GIF)
that a backend inlines, or a placeholder when the format isn't inlinable. An **EMF** (Enhanced Metafile) picture is a
vector command stream, not raster bytes, so `rpt-layout`'s EMF interpreter replays its records into native draw-ops
(line / polygon / ellipse / rect / text) scaled into the object's box instead; a bad or truncated stream falls back to
the placeholder with a diagnostic. WMF and OLE-embedded presentations are still placeholders.

### Coordinate model

Draw-op coordinates are **printable-relative** (0-based: `0,0` is the top-left of the printable area, the margin
removed). Each page carries `origin` — the report's top-left margin. A backend re-applies it **once**, in the way
that backend needs, instead of the margin being baked into every coordinate:

- **HTML** draws content 0-based inside a container that carries the margin as CSS (matching the engine's RAS host).
- **SVG / PDF / raster** are physical pages, so they add `origin` to every coordinate (an SVG `translate` group, a
  PDF `cm` transform, a raster pixel offset).

This keeps the whole coordinate model in one place; there is no `±margin` scattered across position sites.

### Diagnostics

Rendering collects **fidelity warnings** into `PagedDocument.diagnostics` — deep issues that would otherwise never
reach the caller: an object that falls back to a placeholder box (a chart with no plottable group series, a WMF /
OLE-embedded picture), an unimplemented formula builtin, or a runtime formula error. The CLI surfaces these into its
warning summary. Each `Diagnostic` carries a severity, a kind, a message, and the object/formula it is about.

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
  taller than a full page is left to paginate naturally. The pre-measure deliberately ignores can-grow growth — resolving
  it would re-fire `WhilePrintingRecords` variable writes.
- **Print at Bottom of Page** (`print_at_bottom_of_page`) — pin a group/report footer against the body bottom (above the
  page footer), then treat the page as full so the next band starts fresh.
- **Reset Page Number After** (`reset_page_number_after`) — restart the page-number counter at the next page top, giving
  per-group page numbering. `PageNumber` / `Page N of M` follow the reset; `TotalPageCount` stays the whole-document
  count (a per-section total would need a second pass).
- **Underlay Following Sections** (`SectionAreaFormatBase.underlay_section`, SDK `EnableUnderlaySection`) — an underlay
  band is a background for the sections that follow it: after it emits, the flow cursor stays at the band's top so the
  next band overlays it in the same vertical space rather than being pushed below.

Formula-driven conditional variants of these flags are not yet applied (they wait on section condition-formula plumbing).

## Format resolution

A field's displayed value is resolved from **two layers** (see `rpt-layout`'s format module):

1. the **locale** — the "system default" layer: separators, month/day names, AM/PM, default date order, default
   decimals, currency symbol; and
2. the field's **stored `FieldFormat`** leaf — the explicit authoring choices (decimals, negative style, currency
   symbol placement, date component forms, boolean word pair).

The field's own `use_system_defaults` flag arbitrates: when set, the locale supplies the effective format; otherwise
the stored leaf wins for the attributes it sets, with names/separators still coming from the locale (Crystal never
stores "January", only `MonthFormat::LongMonth`). This mirrors the native engine's runtime format resolution.

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
(`<base>-N.svg` / `<base>-N.png`), so they need a real `-o` path. Live-DB connection URLs — including any password —
are read **only** from the environment (`RPT_DB_URL` / `DATABASE_URL`, or `RPT_DB_URL_<SERVER>` per source), never
from a flag, so they do not appear in `ps` output or shell history.

## Testing the renderer

The saved-data corpus is sparse, so the renderer is exercised against a committed, first-class **render-parity test
corpus**: one synthetic database read *identically* by (a) our renderer and (b) the native Crystal engine, so the only
variable under test is rendering, not data.

### The shared "parking" database

`tests/fixtures/sql/parking/parking.sql` is the single, portable-ANSI-SQL seed for the whole corpus — a synthetic
car-park booking domain whose columns are deliberately typed to exercise every formatting path:

| Table | Role | Exercises |
| ----- | ---- | --------- |
| `orders` (fact) | bookings | `DATE`/`TIME`/`TIMESTAMP`, `DECIMAL` (decimal-places, negative `discount`), `BOOLEAN` word-pairs, short+long nullable `note` (can-grow/wrap), `state`/`source` (grouping / cross-tab columns), `total` (summaries/running-totals/charts) |
| `partner`, `parking_lot`, `service` (dims) | linked lookups | equijoins; `BOOLEAN`, `DECIMAL`, long nullable `notes` |
| `order_service` (bridge) | order↔service | multi-hop linking |

It is one **group-shared** seed: `sql/parking/parking.sql` is read by *every* report under `reports/parking/`, so a
single database drives the whole corpus (the render-parity model — one DB, many reports).

**Single DB technology: PostgreSQL only.** SQLite's typelessness reintroduces the boolean/decimal/order
variables the corpus exists to remove between our renderer and the Crystal engine, so it is left out of the *parity
corpus* (the `rpt-db-sqlite` adapter + its own unit tests are kept, just not part of this corpus). Our render is
backend-independent anyway — every column is re-typed against the report's declared field types, not the DB's — so a
row set produces the same output regardless of which engine served it.

### CI regression layer (our side, zero external process)

`crates/rpt-render/tests/postgres_fixtures.rs` renders each report over the seeded DB with the deterministic
`ApproxLayout` (no system fonts → host-independent bytes) and diffs the HTML against a committed baseline under
`tests/fixtures/baselines/html/parking/<name>.html`. It **skips** when no `RPT_DB_URL` is set, so a DB-less
`cargo test` stays green; CI provides a `postgres` service. Re-bless after an intentional render change:

```sh
RPT_DB_URL=postgres://rpt:rpt@localhost:5432/rptdemo \
  RPT_BLESS=1 cargo test -p rpt-render --test postgres_fixtures
```

### Cross-engine oracle (out-of-band, maintainer machine)

The blessed baselines are validated against the real engine by rendering **both** stacks over the *same* live
Postgres — ours (`rpt-render --db`) and the native Crystal engine (`crpe32` via ODBC, hosted under Wine) — and
scoring positioned parity:

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
(`rpt-query::build_query_for_report`); this is what keeps a report with unused/unlinked tables from cross-joining into
a cartesian (see the pipeline table above).

### Feature coverage (36 parking reports)

| Feature | Reports | Notes |
| ------- | ------- | ----- |
| Charts | 24 | `orders_*` (bar/3-D/area/pie/doughnut/funnel/gauge/surface + legend variants) and the advanced `chart_*` (scatter/stock/histogram/radar/gantt) |
| Cross-tabs | 11 | `crosstab_*` — grid + grand-total / suppress / margin / repeat-label option variants |
| Grouping | 18 | `orders_*` group on `state`; date-group granularity on `arrival_date` |
| Detail field / summary / running-total | `orders*` | `total` summarized per group and report |
| **Subreport** | **0** | **gap** — no corpus report exercises a subreport yet (needs authoring in the Crystal designer) |

The corpus is authored **incrementally** — from an empty `base.rpt` (no bound table → static bands only) upward, one
feature at a time — and each report is committed with both an XML decode baseline
(`tests/fixtures/baselines/xml/parking/<name>.xml`, so authoring also validates the decoder) and the HTML render
baseline above.
