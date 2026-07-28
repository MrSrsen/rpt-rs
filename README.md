<div align="center">
  <img src="docs/rpt-rs.png" alt="rpt-rs logo" width="180" />
  <p>
    <a href="LICENSE"><img alt="License: MPL-2.0" src="https://img.shields.io/badge/license-MPL--2.0-blue.svg"></a>
    <img alt="Rust 1.89+" src="https://img.shields.io/badge/rust-1.89%2B-orange.svg">
  </p>
</div>

# rpt-rs

**rpt-rs** reads and renders **Crystal Reports `.rpt`** files in pure Rust — no Crystal Reports runtime, no Windows, no
.NET. Point it at a `.rpt` file and it can:

- **Inspect** the report: its data sources, parameters, formulas, groups, sections, and objects.
- **Export** the full report definition as JSON or KDL — handy for search, review, and diffing reports in version
  control.
- **Render** the report to paginated **HTML, SVG, PDF, or PNG**, using the data saved inside the file or rows fetched
  live from a database.
- **Evaluate and check Crystal formulas** — both formula dialects, usable as a standalone library; `rpt formulas`
  validates every formula in a report without rendering it.
- **Run anywhere**: Linux, macOS, Windows, and (for the render core) WebAssembly.

> ⚠️ This project is experimental and its API is unstable. Expect major refactorings and breaking changes. If you need
> stability, pin a commit or fork the repository.

## Example

The report below is from the repository's own [Meridian](tests/meridian/) corpus — a product catalog rendered over the
committed synthetic seed database:

```sh
rpt-render tests/meridian/reports/sales/02_product_catalog.rpt --db -f png -o catalog.png
```

PNG and SVG are one file per page, so that writes `catalog-1.png`, `catalog-2.png`, … — this is `catalog-1.png`:

<div align="center">
  <img src="docs/example-render.png" alt="A report page rendered to PNG by rpt-rs" width="560" />
</div>

Everything on the page comes out of the `.rpt`: the embedded logo, the group hierarchy, the banded rows, per-row image
fields, the multi-currency prices, the conditional hazard flag, and the `Page 1 of 37` footer that only resolves once
pagination is done. The same command renders the whole report to a single self-contained HTML file (`-f html`) or a
print-ready PDF (`-f pdf`); a report that carries usable saved data needs no `--db` at all.

## Installation

The crates are not on crates.io yet — build from source with a Rust toolchain:

```sh
cargo build --release
```

This produces two binaries in `target/release/`: `rpt` (inspect/export) and `rpt-render` (render).

Or skip Rust entirely with Docker — a multistage build produces a `scratch` image containing nothing but the two
statically linked binaries:

```sh
docker build -t rpt-rs .
```

The image has no system fonts, so a render inside it falls back to the metric-compatible Liberation faces bundled in
`rpt-text`; text geometry will differ slightly from a host with the report's real fonts installed.

## Usage

```sh
# Inspect a report: version, summary info, streams
rpt inspect report.rpt

# List its parameters as JSON
rpt inputs report.rpt --json

# Export the whole decoded definition as JSON
rpt json-dump report.rpt out.json

# Show every SQL the report can run (generated queries + stored commands), with provenance
rpt sql report.rpt

# List and check every formula the report defines (exit 1 if any is broken; --source shows the bodies)
rpt formulas report.rpt

# Render the report's saved data to a PDF
rpt-render report.rpt -o out.pdf

# Render to HTML on stdout, passing a parameter
rpt-render report.rpt -p Region=West -f html > out.html

# Render from a live database (URL comes from the environment, never a flag)
rpt-render report.rpt --list-sources          # which env var to set
RPT_DB_URL='postgres://user:pass@host:5432/db' rpt-render report.rpt --db -o out.pdf
```

With Docker, mount the directory holding the report as `/data` and run the same commands:

```sh
docker run --rm -v "$PWD:/data" rpt-rs rpt inspect report.rpt
docker run --rm -v "$PWD:/data" rpt-rs rpt-render report.rpt -o out.pdf
```

## Library

Read a report:

```rust
use rpt::Rpt;

fn main() -> rpt::Result<()> {
    let rpt = Rpt::open("report.rpt")?;
    let report = rpt.report();

    println!("Title: {}", report.summary_info.title);
    for table in &report.database.tables {
        println!("Table: {}", table.name);
    }
    for (field, _param) in report.data_definition.parameter_fields() {
        println!("Parameter: {}", field.name);
    }
    Ok(())
}
```

Render one — loading and writing files can fail, but the render itself never does:

```rust
use rpt_render::ReportDocument;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = ReportDocument::load("report.rpt")?;
    doc.export_html_to_disk("out.html")?; // saved data → HTML
    let pdf: Vec<u8> = doc.to_pdf();      // …or bytes (infallible)
    std::fs::write("out.pdf", pdf)?;
    Ok(())
}
```

More recipes — rendering from a live database, feeding a report from your own data, choosing an output backend, WASM —
are in the [render examples](docs/13-render-examples.md).

## What works — and what doesn't

**Solid today**

- The whole decode pipeline: container, decryption, decompression, record tree, subreports — lossless on every file in
  the test corpus, with round-trip re-encoding.
- The report model and its exports: data sources, parameters, formulas, groups, sorting, summaries, running totals,
  sections, objects, formatting (incl. conditional format formulas), subreport links, page setup.
- The formula engine: both dialects, a large builtin library, static typing, and runtime evaluation.
- Rendering: field/text layout with real font metrics (cosmic-text) and Unicode/CJK line breaking, pagination controls
  (new-page before/after, keep-group-together, print-at-bottom, underlay, page-number reset), locale-driven value
  formatting, charts (16 chart types drawn as native vector ops), cross-tabs, pictures (incl. EMF vector replay via the
  standalone `metafile` crate), hyperlinks.
- Live databases: PostgreSQL and SQLite behind a `RowSource` trait, with report-driven SQL generation (only the tables
  and columns the report uses, selection formulas pushed into `WHERE`).
- Diagnostics: a failure says what went wrong, where, and what to do about it. Reading is deliberately forgiving — an
  unrecognized record becomes a default rather than an error, and a broken formula does not abort a render — so anything
  worked around is *reported* instead: an incomplete decode, a selection that dropped every row, a formula that would not
  parse, a cell coerced away from its declared type. Errors name the file, the data source, or the failing statement.

**Partial or missing**

- **Writing**: byte-level only — re-encoding round-trips, and a decoded record's leaf can be patched in place or resized
  (enclosing record lengths are recomputed). Only record types proven safe to edit are writable by default; anything else
  is refused unless you pass `--force`, because a record carrying its own offset table can be overwritten into a file
  that re-decodes cleanly but is semantically corrupt. `anonymize` is the one semantic edit built on that path; you
  cannot yet mutate the semantic model generally and serialize it back.
- **Saved data**: two storage classes decode (the external memo-heap class and the packed, memo-less one); a batch whose
  metadata yields no valid decryption IV is reported but not decoded.
- **Charts**: chart types without a dedicated renderer fall back to a bar rendering with a diagnostic; the opaque chart
  styling blob is not decoded, so heavily customized charts render with default styling.
- **Cross-tabs**: one row dimension × one column dimension × the first measure; nested multi-level axes are pending.
- Maps, OLAP grids, alerts, Flash widgets, and XML/XSLT export definitions are recognized but not decoded (absent from
  the available corpus).
- MySQL / MariaDB / MSSQL connection URLs are recognized but the drivers are not implemented.
- WASM builds cover the pipeline plus the HTML and SVG backends; the PDF and PNG backends are native-only.

The [support matrix](docs/08-support-matrix.md) has the full feature-by-feature table.

## How it works

```mermaid
flowchart LR
    file[".rpt file"] --> dec["rpt<br/>decrypt · inflate · decode"]
    dec --> model["typed report model<br/>(rpt-model)"]
    model --> dump["JSON / KDL export<br/>(rpt json-dump · rpt kdl)"]
    model --> data["rpt-data<br/>select · sort · group · summarize"]
    saved["saved data"] --> data
    db[("live DB<br/>(RowSource)")] --> data
    data --> layout["rpt-layout<br/>place · paginate"]
    layout --> ir["Page IR<br/>(rpt-pages)"]
    ir --> out["HTML · SVG · PDF · PNG"]
```

- **Reads `.rpt` directly** — opens the OLE/CFB compound file, decrypts the report streams (AES-128-CFB, the format's
  fixed key), inflates them, and decodes the internal record tree. Reading is **lossless**: records the decoder does not
  yet understand are preserved byte-exactly.
- **Builds a typed model** — data sources, tables and joins, parameters, formulas, groups, sorts, summaries, running
  totals, sections, and every laid-out report object, as plain Rust structs you can walk.
- **Exports the decoded model** — `json-dump` writes the exhaustive, deterministic JSON document: every decoded field,
  and only decoded fields, which is what lets it double as the project's regression baseline; `kdl` writes a sparse,
  human-readable authoring projection of the same model.
- **Renders reports** — evaluates formulas, runs the data pipeline, lays out and paginates the report, and emits HTML,
  SVG, PDF (with real font subsetting), or PNG. Rows come from the report's saved data, or from a live database when it
  has none — connection URLs are read only from the environment, never from flags.
- **Speaks the formula language** — the standalone [`crystal-formula`](crates/crystal-formula) crate implements both
  formula dialects (Crystal and Basic syntax): lexer, parser, type checker, bytecode VM, and a validation pass with
  LSP-shaped diagnostics. Usable entirely without a `.rpt` file.
- **Inspects, checks, and byte-patches from the CLI** — one-screen summaries, parameter listings, the decoded record
  tree, per-stream decode coverage, saved-data rows, the SQL the report can run (`sql` — generated queries + stored
  commands, with provenance), every formula it defines (`formulas` — parsed and validated, with the bodies under
  `--source`), plus a byte-faithful re-encoder (`reencode` / `patch`) that writes valid `.rpt` files back out and
  refuses an edit to a record type not cleared as safe.
- **Scrubs authoring metadata** — `anonymize` removes the author, the last person to save, and the machine paths a
  re-imported subreport carries, writing a clean `.rpt`. Every edit is same-length, so the report decodes to the same
  model and the real Crystal engine still opens it.

## Workspace

24 members in two layers: 21 library crates under [`crates/`](crates/) and three binaries under [`apps/`](apps/). The
**reader** decodes the stored facts from the bytes: `rpt` (container → decryption → records → model, `unsafe`-free), the
pure-data `rpt-model` semantic model, the standalone `crystal-formula` engine, the `rpt-json` / `rpt-kdl` export
surfaces, and the `rpt-cli` inspection/export binary. The **render & data pipeline** is built purely on the decoded
model — it depends on `rpt-model`, not the decoder, so it stays cross-platform and WASM-safe: `rpt-data` → `rpt-layout`
→ the `rpt-pages` Page IR → the four `rpt-render-*` backends, orchestrated by the `rpt-render` facade and the
`rpt-render` CLI. Database drivers (`rpt-db-postgres`, `rpt-db-sqlite`) are isolated behind the `RowSource` trait so the
portable core never links one. A few crates are useful on their own — `crystal-formula` (the formula language) and
[`metafile`](crates/metafile) (a Windows-metafile/EMF vector parser).

The [codebase guide](docs/10-codebase.md) has the full crate-by-crate map.

## Documentation

Everything lives in [`docs/`](docs/) — start at the [documentation index](docs/README.md), which chains the pages in
reading order:

- **Format**: [overview](docs/01-format-overview.md) · [container](docs/02-container.md) ·
  [stream decoding](docs/03-stream-decoding.md) · [record tree](docs/04-record-tree.md) ·
  [semantic model](docs/05-semantic-model.md) · [saved data](docs/06-saved-data.md)
- **Reference**: [block catalog](docs/07-block-catalog.md) · [support matrix](docs/08-support-matrix.md) ·
  [endianness](docs/09-endianness.md)
- **Using the library**: [codebase map](docs/10-codebase.md) · [usage](docs/11-usage.md) ·
  [rendering](docs/12-rendering.md) · [render examples](docs/13-render-examples.md)
- **Formula engine**: [`docs/formula-engine/`](docs/formula-engine/) — architecture & VM, language reference, builtins,
  validation

The synthetic render-test corpus documents itself under [`tests/meridian/`](tests/meridian/) — the invented company its
reports are written against, and its schema.

## Acknowledgments

This project would not have been possible without **[RptToXml](https://github.com/ajryan/RptToXml)** by ajryan. Its output was the oracle that drove this
decoder at the beginning.

This project was developed with the assistance of AI (Claude Opus 4.8/5.0).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, test, and structure changes.

## License

MPL-2.0.

Crystal Reports is a product of its respective owner; this project is an independent, clean-room implementation
and is not affiliated with or endorsed by it.
