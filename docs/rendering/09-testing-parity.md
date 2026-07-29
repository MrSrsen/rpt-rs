# Testing the renderer

The saved-data corpus is sparse, so the renderer is exercised against committed, first-class **render-test corpora**,
each built on a committed synthetic seed database — one for the whole Meridian corpus, per-report or group-shared for
the legacy fixtures — so the only variable under test is rendering, not data.

**Single DB technology: PostgreSQL only.** SQLite's typelessness reintroduces the boolean/decimal/order variables the
corpora exist to remove, so it is left out of them (the
`rpt-db-sqlite` adapter + its own unit tests are kept, just not part of them). Our render is backend-independent
anyway — every column is re-typed against the report's declared field types, not the DB's — so a row set produces the
same output regardless of which engine served it.

The **layer map** — which harness owns L1…L4b, and how to run and bless each — lives with the build instructions in
[Tests, and the four layers](../project/building.md#tests-and-the-four-layers). This page is the other half: what each
corpus *is* and what each fixture buys. Every section below names the layer it sits at.

## Meridian — the corpus going forward

`tests/meridian/` is a self-contained synthetic universe: **Meridian Global Logistics**, an invented third-party
logistics group whose data is deliberately shaped (a Pareto revenue split, a Q4 peak, a mid-year fuel spike, one
chronically late carrier, a multi-year build-out program) so that Top-N reports, trend charts, OHLC charts, scorecard
gauges, and Gantt charts all have something real to show. Everything about it is fictional and safe to publish.

| Path                                               | What it is                                                                                                                                                                                 |
|----------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `tests/meridian/README.md`                         | The company story — what the divisions are and what the numbers mean.                                                                                                                      |
| `tests/meridian/SCHEMA.md`                         | Every table and field, and how each feeds the reports/charts.                                                                                                                              |
| `tests/meridian/sql/meridian.sql`                  | The seed database, read by every report in the corpus. Generated deterministically by `apps/meridian-seed` from a fixed PRNG seed, so it is reproducible without shipping a database dump. |
| `tests/meridian/sql/pg-init/25-meridian-views.sql` | Helper views applied after the seed: per-row aggregates and correlated lookups a table-linking join engine cannot express, which reports bind as ordinary tables.                          |
| `tests/meridian/reports/**`                        | The `.rpt` files, organized by division (`executive`, `freight`, `projects`, `sales`, plus `probes` for single-feature isolations).                                                        |
| `tests/meridian/baselines/page-ir/`                | One Page IR baseline per report, mirroring its path under `reports/` with `.rpt`→`.json`.                                                                                                  |
| `tests/meridian/baselines/json/`                   | One L1 decode baseline per report, mirroring its path below `tests/meridian/` — so `reports/<division>/<name>.json`. The corpus, and the authoring template, are in the decode net too.    |

## Legacy fixtures (`tests/fixtures/`)

The older group-shared corpus, **deprecated in favour of Meridian** but still driven by the same harness for as long as
it exists. Its structure is `sql/<group>/<name>.sql` for a per-report seed, or `sql/<group>/<group>.sql` for a
**group-shared** one read by every report under `reports/<group>/`; baselines mirror the `<group>/<name>` path. The
group-shared `parking` seed (`sql/parking/parking.sql`, 36 reports) is a synthetic car-park booking domain whose columns
are typed to exercise every formatting path — `DATE`/`TIME`/`TIMESTAMP`, `DECIMAL` with negative values, `BOOLEAN`
word-pairs, short+long nullable notes for can-grow/wrap, grouping and cross-tab columns, and a `total` for
summaries/running-totals/charts.

## Dataset snapshot baselines (no database)

**L2.** `crates/rpt-render/tests/dataset_baselines.rs` is the layer *above* the Page IR: it builds each fixture's
`Dataset` and diffs a text snapshot of it against `tests/fixtures/baselines/dataset/<group>/<name>.txt`. This is where
`rpt-data` is tested at its own boundary — record selection, sort, grouping, summaries, running totals and formula
evaluation. Without it a grouping bug and a pagination bug are the same symptom, a moved Page IR, and an investigation
has to bisect which stage moved.

The snapshot records the column list with its value types, the surviving `row_count`, the parameter values in play, each
formula field with the evaluation time `rpt-data` classifies it as, the pipeline's own fail-open diagnostics, the grand
total, and then the group tree — indented per level, each level carrying its key and summaries, with the detail rows at
the deepest one. A detail row is one line: its read-order index, every column's value in `Dataset::columns` order, then
each evaluable formula's value. Values are emitted **typed** (`Cur 1234.5`, not `$1,234.50`) — display formatting is
`rpt-layout`'s job against a `Locale`, so formatting here would put the host locale in the baseline and duplicate L3.
Floats print with six fixed decimals, trailing zeros trimmed, so a summary cannot go red on the last bit of an `f64`
fold.

The formatter is hand-written in the harness rather than derived: `rpt-data` and `rpt-formula` both deliberately carry
no serde dependency (`rpt-formula` stays reusable from an LSP or WASM sandbox on minimal deps), and deriving
`Serialize` for a test surface would change two shipping crates' dependency shape. Driving each row's cells off
`Dataset::columns` also sidesteps the fact that a `Row` stores every column twice — under both `table.field` and its
bare short name.

Nine fixtures, each buying a stage the others do not reach: the smallest end-to-end pipeline (`Big Cells - Mexico`), a
saved batch that does *not* satisfy its own record selection and must still render all 52 rows (`PinkPaletteSampler`), a
three-key record sort (`Country_Region_CustName_sort`), group summaries plus a per-record `Select … Case` formula
(`China Orders, Grouped with dsct`), a running total surfacing as a `#name` summary
(`China Orders, with running totals`), formula evaluation with no grouping (`Formulas`), `Weekly` date-condition group
bucketing (`parking/orders_weekly`), three nested group levels with a Sum at each (`synthetic/nested_group3`), and
`{?Param}` resolution over a second unfiltered saved batch (`Orders10k`). Every one reads its **own saved data**, so
this harness needs no database and runs on every checkout. Nested grouping was the gap until `nested_group3`: the
corpus's saved batches all top out at one group level, and the Meridian reports that nest produce datasets far too large
to snapshot at this layer, so the coverage came from a report authored small enough to afford — 34 rows, three levels,
branching at every one. Re-bless with `RPT_BLESS=1 cargo test -p rpt-render --test dataset_baselines`.

## CI regression layer (our render only — no engine comparison, but a live PostgreSQL)

**L3.** `crates/rpt-render/tests/postgres_fixtures.rs` drives **both** corpora: it seeds each committed `.sql` into a
PostgreSQL server, renders every matching report through the whole pipeline with the deterministic `ApproxLayout` (no
system fonts → host-independent bytes), and diffs the normalized Page IR against the committed baseline. It **skips**
when no `RPT_DB_URL` is set, so a DB-less `cargo test` stays green; CI provides a `postgres` service.
`docker-compose.yml` brings up the local server on the project-wide port 55432. Re-bless after an intentional render
change:

```sh
RPT_DB_URL=postgres://rpt:rpt@localhost:55432/rptfixtures \
  RPT_BLESS=1 cargo test -p rpt-render --test postgres_fixtures
```

These baselines are laid out with the deterministic `ApproxLayout`, not the cosmic-text stack the `rpt-render` CLI uses,
so their page counts are deliberately **not** the CLI's. A baseline page count is not a number to check a render
against — see [`ApproxLayout` and its pagination divergence](10-examples.md#approxlayout-and-its-pagination-divergence).

Every scope is fed: each **subreport** gets its own rows from the same server, and a report that declares **parameters**
is rendered with values from the harness's own per-fixture table (`fixture_params`). Both exist because the alternative
is a fixture that looks healthy and covers nothing — a subreport with no rows formats as nothing, and a record selection
filtering on an unbound `{?Param}` fails on every record and is dropped fail-open, leaving the report as its static
headers. The values are also chosen to keep the fixture small: a parameterized report run wide open is not a better
test, it is a multi-megabyte baseline nobody reads.

## Typography baselines (no database)

**L3.** `crates/rpt-render/tests/typography_baselines.rs` covers the five **data-free** fixtures under
`tests/fixtures/reports/typography/` — blank reports whose Report Header holds static text objects, each isolating one
font/text axis (face, size, style flags, color + alignment, paragraph indent/spacing). They bind no datasource, so this
harness needs no database and runs on every checkout. One baseline per fixture: the **normalized Page IR**
(`baselines/page-ir/typography/`), the structural contract — op kinds, twip positions, text, resolved font — so it pins
layout behaviour directly. Re-bless with `RPT_BLESS=1 cargo test -p rpt-render --test typography_baselines`.

These freeze *our* behaviour, not engine parity: a baseline changing when a known divergence gets implemented is the
expected signal. They render through `FontProvider::bundled()` — the compiled-in Liberation/DejaVu set, never the host
font registry — so they mean the same thing on every machine, and a bare CI runner cannot read as a layout regression.

## PDF content-stream baselines (no database)

**L4a.** `crates/rpt-render/tests/pdf_baselines.rs` is the layer below the Page IR: it renders nine fixtures to PDF and
diffs the **operator listing** — the writer's own output as text — against
`tests/fixtures/baselines/pdf/<group>/<name>.txt`. With the Page IR baselines green, a diff here isolates the *PDF
backend's serialization*: the font resource a run selects, the text and transform matrices, path construction for shapes
and chart interiors, image XObject placement.

The listing, not the PDF bytes. Our PDF output is byte-deterministic, so a byte golden is possible — but it is
unreadable before blessing (a one-twip move reads as "binary files differ") and pins incidental structure (object
numbering, xref offsets, the per-subset font tag). `rpt_test_support::pdf::operator_listing` projects the document
instead: one block per page carrying its media box, the faces and images its resource dictionary binds, and its content
stream one operator per line, with glyph strings decoded back to text through the font's own `/ToUnicode` map (the bytes
on the wire are subset glyph indices assigned in order of first use, so they renumber wholesale on any change), subset
tags stripped from face names, and numbers rounded to 3 decimals — 1/1000 pt, finer than the twip the layout works in.

Nine fixtures, not the corpus: the five data-free typography reports (font resolution/fallback, `Tf` sizes, four
embedded subsets of one family, rotated + colored + aligned runs, two pages), plus
`worrall/USStatesWithAbbreviations` (image XObject, stroked rules, saved rows),
`benbrahim777/Top5USA_piechart` (a chart interior — ~200 polygon paths with fill-and-stroke),
`benbrahim777/USA Orders, Percentages` (four pages of grouped data) and
`synthetic/currency_symbol_per_page` (the post-pagination `OneCurrencySymbolPerPage` pass, over three pages). Every
fixture renders from its **own saved data**, so this harness needs no database and runs on every checkout, and both
halves of the font stack are pinned to the bundled faces (`CosmicLayout` over `FontProvider::bundled()` for metrics,
`PdfOptions { fonts: FontSource::Bundled }` for embedding). Re-bless with
`RPT_BLESS=1 cargo test -p rpt-render --test pdf_baselines`.

## PDF artifact checks (no database)

**L4b.** `crates/rpt-render/tests/pdf_artifacts.rs` asserts *relationships* rather than diffing a baseline, so it has
nothing to bless. With L4a pinning the writer's operators, what a golden still cannot answer is whether the bytes around
them form a document: page count against the Page IR's, one font object per resolved face carrying a width table and an
embedded program, one image XObject per placed asset — and, the highest-value check, that the pen advance a reader
computes from the declared widths (less the writer's `TJ` adjustments) equals the advance the layout engine measured and
placed the run by, since the symptom of a disagreement is a page of displaced glyphs. A third asserts a JPEG asset is
passed through as
`/DCTDecode` rather than re-encoded on the way in, and a fourth opens every artifact with `qpdf`, kept separate so a
machine without the tool cannot mask the three in-Rust classes. PDF/A and PDF/UA conformance is deliberately **not**
asserted here — these fixtures render with no validator set, so a conformance assertion over them would test nothing.

## Decode baselines

**L1.** Independently of rendering, `apps/rpt-cli/tests/json_baseline.rs` runs `rpt json-dump` over **both** report
trees — the fixtures under `tests/fixtures/reports/` and the Meridian corpus under `tests/meridian/` — and diffs each
against its baseline (`tests/fixtures/baselines/json/<group>/<name>.json` and
`tests/meridian/baselines/json/reports/<division>/<name>.json` — the Meridian walk is rooted at `tests/meridian/`, so a
baseline mirrors the report's path below *that*), so authoring a report also pins the *decoder* — exhaustively, since
the dump serializes every decoded field. Each tree is **walked** rather than listed, and the walk is exact in both
directions: every `.rpt` found must have a baseline, and every baseline must have a report behind it. It needs no
sandbox — the dump depends on nothing outside the `.rpt`, so it is byte-identical on any machine — and a walk that finds
too few reports **fails** rather than skipping, so a misresolved path cannot report `ok` having compared nothing.
Re-bless with `RPT_BLESS=1 cargo test -p rpt-cli --test json_baseline`.

## Engine parity

Everything above freezes *our* behaviour: a Page IR diff between two of our own runs is **regression**, not parity. What
tells us whether the render is *right* is a comparison against the native engine's own output, and there are two
instruments, because neither sees the whole page.

- **The engine's PDF.** Positioned glyphs — what text was drawn, in what face and size, at what point on which page.
  This is the primary parity instrument, and the one that catches a displaced or mis-formatted value.
- **The engine's page metafile.** The PDF cannot see inside a chart: the engine embeds a chart as a raster image, so
  every glyph and every stroke in it is invisible to a text comparison. Its page EMF carries the chart as draw-ops
  instead, which our own [`metafile`](03-page-ir.md) crate parses — so chart geometry, fonts and axis labels can be
  compared shape by shape. It is the only instrument that reaches chart interiors.

The engine's **HTML export is not usable** as a matcher: it fails outright on a page carrying a chart, so a corpus-wide
HTML comparison silently measures only the reports without one.

One artifact of the engine's own export path is worth knowing before reading a diff: the ODBC driver the engine reads Postgres through
drops the scale of a `NUMERIC` column, so a value the engine renders with fewer decimals than we do is the driver, not a
formatting divergence.

## Feature coverage

Both corpora are authored **incrementally** — from an empty report (no bound table → static bands only) upward, one
feature at a time — and each report is committed with its render baseline. Between them:

| Feature                               | Where                                                                                                                                                                                                                                                                                                                                                                                                                                            |
|---------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Charts                                | the legacy `orders_*` family (bar/3-D/area/pie/doughnut/funnel/gauge/surface + legend variants), the 6 advanced `chart_*` reports (scatter/stock/histogram/radar/gantt), and the `synthetic/chart_*` minimal pairs that toggle one chart property each (data labels, group-title slant, pie slice detachment, view angle) against a control; in Meridian, the volume trend, mode split, fuel OHLC, carrier scorecard, and capital-projects Gantt |
| Cross-tabs                            | 11 legacy `crosstab_*` reports (grid + grand-total / suppress / margin / repeat-label variants); Meridian's revenue and invoice-aging cross-tabs                                                                                                                                                                                                                                                                                                 |
| Grouping / summaries / running totals | the 18 legacy `orders*` reports (group on `state`, date-group granularity); Meridian's sales-by-region, Top-10, and statement reports                                                                                                                                                                                                                                                                                                            |
| **Subreports**                        | 6 Meridian reports (customer statement, sales by region, shipment tracking, fuel price/revenue, executive dashboard, shared-variable probe) — the gap the legacy corpus never covered                                                                                                                                                                                                                                                            |
| Hierarchical grouping                 | Meridian's employee directory — a self-referential `manager_id → employee_id` org tree, 200 instances, 10 levels deep                                                                                                                                                                                                                                                                                                                            |
| Formula-engine surface                | Meridian's `probes/` set — Basic syntax, string functions, `Shared` variables, summary operations, SQL Expression fields, page-number reset                                                                                                                                                                                                                                                                                                      |
| Typography (data-free)                | the 5 [`typography` fixtures](#typography-baselines-no-database)                                                                                                                                                                                                                                                                                                                                                                                 |

---

← [The `rpt-render` CLI](08-cli.md) · [Index](README.md) · **Next:** [Render examples](10-examples.md) →
