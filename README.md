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
- **Export** the full report definition as XML — handy for search, review, and diffing reports in version control.
- **Render** the report to paginated **HTML, SVG, PDF, or PNG**, using the data saved inside the file or rows fetched
  live from a database.
- **Evaluate Crystal formulas** — both formula dialects, usable as a standalone library.
- **Run anywhere**: Linux, macOS, Windows, and (for the render core) WebAssembly.

> ⚠️ This project is experimental and its API is unstable. Expect major refactorings and breaking changes. If you need
> stability, pin a commit or fork the repository.

## Example

Every report below comes from the repository's own test fixtures. This page was produced by rendering a public sample
report from its embedded saved data — no database, no Crystal Reports installation:

```sh
rpt-render tests/fixtures/reports/worrall/MajorCitiesInCanadaUSAandMexico.rpt -f png -o city.png
```

<div align="center">
  <img src="docs/example-render.png" alt="A report page rendered to PNG by rpt-rs" width="560" />
</div>

The same command renders the same page to HTML (`-f html`), a print-ready PDF (`-f pdf`), or SVG (`-f svg`).

## Installation

The crates are not on crates.io yet — build from source with a Rust toolchain:

```sh
cargo build --release
```

This produces two binaries in `target/release/`: `rpt` (inspect/export) and `rpt-render` (render).

Or skip Rust entirely with Docker — a multistage build produces a small image (~14 MB) containing just the two
statically linked binaries:

```sh
docker build -t rpt-rs .
```

## Usage

```sh
# Inspect a report: version, summary info, streams
rpt inspect report.rpt

# List its parameters as JSON
rpt inputs report.rpt --json

# Export the definition to XML
rpt xml-dump report.rpt out.xml

# Show every SQL the report can run (generated queries + stored commands), with provenance
rpt sql report.rpt

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

Render one:

```rust
use rpt_render::ReportDocument;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = ReportDocument::load("report.rpt")?;
    doc.export_html_to_disk("out.html")?; // saved data → HTML
    let pdf: Vec<u8> = doc.to_pdf();      // …or bytes
    std::fs::write("out.pdf", pdf)?;
    Ok(())
}
```

More recipes — rendering from a live database, feeding a report from your own data, choosing an output backend, WASM —
are in the [render examples](docs/12-render-examples.md).

## What works — and what doesn't

**Solid today**

- The whole decode pipeline: container, decryption, decompression, record tree, subreports — lossless on every file in
  the test corpus, with round-trip re-encoding.
- The report model and XML export: data sources, parameters, formulas, groups, sorting, summaries, running totals,
  sections, objects, formatting (incl. conditional format formulas), subreport links, page setup.
- The formula engine: both dialects, a large builtin library, static typing, and runtime evaluation.
- Rendering: field/text layout with real font metrics (cosmic-text) and Unicode/CJK line breaking, pagination controls
  (new-page before/after, keep-group-together, print-at-bottom, underlay, page-number reset), locale-driven value
  formatting, charts (16 chart types drawn as native vector ops), cross-tabs, pictures (incl. EMF vector replay),
  hyperlinks.
- Live databases: PostgreSQL and SQLite behind a `RowSource` trait, with report-driven SQL generation (only the tables
  and columns the report uses, selection formulas pushed into `WHERE`).

**Partial or missing**

- **Writing**: byte-level only — re-encode and same-size patching work; you cannot yet mutate the semantic model and
  serialize it back.
- **Saved data**: the common storage variant decodes; rarer batch classes are recognized but not decoded.
- **Charts**: chart types without a dedicated renderer fall back to a bar rendering with a diagnostic; the opaque chart
  styling blob is not decoded, so heavily customized charts render with default styling.
- **Cross-tabs**: one row dimension × one column dimension × the first measure; nested multi-level axes are pending.
- Maps, OLAP grids, alerts, Flash widgets, and XML/XSLT export definitions are recognized but not decoded (absent from
  the available corpus).
- MySQL / MariaDB / MSSQL connection URLs are recognized but the drivers are not implemented.
- WASM builds cover the pipeline plus the HTML and SVG backends; the PDF and PNG backends are native-only.

The [support matrix](docs/09-support-matrix.md) has the full feature-by-feature table.

## How it works

```mermaid
flowchart LR
    file[".rpt file"] --> dec["rpt<br/>decrypt · inflate · decode"]
    dec --> model["typed report model<br/>(rpt-model)"]
    model --> xml["XML export<br/>(rpt xml-dump)"]
    model --> data["rpt-data<br/>select · sort · group · summarize"]
    saved["saved data"] --> data
    db[("live DB<br/>(RowSource)")] --> data
    data --> layout["rpt-layout<br/>place · paginate"]
    layout --> ir["Page IR<br/>(rpt-pages)"]
    ir --> out["HTML · SVG · PDF · PNG"]
```

- **Reads `.rpt` directly** — opens the OLE/CFB compound file, decrypts the report streams (AES-128-CFB, the format's
  fixed key), inflates them, and decodes the internal record tree. Reading is **lossless**: records the decoder does
  not yet understand are preserved byte-exactly.
- **Builds a typed model** — data sources, tables and joins, parameters, formulas, groups, sorts, summaries, running
  totals, sections, and every laid-out report object, as plain Rust structs you can walk.
- **Exports XML** — a structured, RptToXml-compatible document of the whole report definition, validated
  attribute-by-attribute against the native Crystal engine's output.
- **Renders reports** — evaluates formulas, runs the data pipeline, lays out and paginates the report, and emits HTML,
  SVG, PDF (with real font subsetting), or PNG. Rows come from the report's saved data, or from a live database when it
  has none — connection URLs are read only from the environment, never from flags.
- **Speaks the formula language** — the standalone [`crystal-formula`](crates/crystal-formula) crate implements both
  formula dialects (Crystal and Basic syntax): lexer, parser, type checker, bytecode VM, and a validation pass with
  LSP-shaped diagnostics. Usable entirely without a `.rpt` file.
- **Inspects and byte-patches from the CLI** — one-screen summaries, parameter listings, the decoded record tree,
  per-stream decode coverage, saved-data rows, the SQL the report can run (`sql` — generated queries + stored commands,
  with provenance), plus a byte-faithful re-encoder (`reencode` / `patch`) that writes valid `.rpt` files back out.

## Workspace

20 crates in two layers. The **reader** decodes the stored facts from the bytes: `rpt` (container → decryption →
records → model, `unsafe`-free), the pure-data `rpt-model` semantic model, the standalone `crystal-formula` engine, and
the `rpt-cli` inspection/export binary. The **render & data pipeline** is built purely on the decoded model — it
depends on `rpt-model`, not the decoder, so it stays cross-platform and WASM-safe: `rpt-data` → `rpt-layout` → the
`rpt-pages` Page IR → the four `rpt-render-*` backends, orchestrated by the `rpt-render` facade and the `rpt-render`
CLI. Database drivers (`rpt-db-postgres`, `rpt-db-sqlite`) are isolated behind the `RowSource` trait so the portable
core never links one.

The [codebase guide](docs/07-codebase.md) has the full crate-by-crate map.

## Documentation

Everything lives in [`docs/`](docs/) — start at the [documentation index](docs/README.md):

- **Format**: [overview](docs/01-format-overview.md) · [container](docs/02-container.md) ·
  [stream decoding](docs/03-stream-decoding.md) · [record tree](docs/04-record-tree.md) ·
  [semantic model](docs/05-semantic-model.md) · [saved data](docs/10-saved-data.md)
- **Reference**: [block catalog](docs/06-block-catalog.md) · [support matrix](docs/09-support-matrix.md) ·
  [endianness](docs/appendix-endianness.md)
- **Using the library**: [codebase map](docs/07-codebase.md) · [usage](docs/08-usage.md) ·
  [rendering](docs/11-rendering.md) · [render examples](docs/12-render-examples.md)
- **Formula engine**: [`docs/formula-engine/`](docs/formula-engine/) — architecture & VM, language reference,
  builtins, validation

## Acknowledgments

This project would not have been possible without **[RptToXml](https://github.com/ajryan/RptToXml)** by ajryan, which
exports `.rpt` files to XML through the official Crystal Reports .NET runtime. The `rpt xml-dump` exporter here is, in
effect, a reimplementation of RptToXml: it produces the same kind of XML, but by decoding the `.rpt` bytes directly —
no Crystal Reports runtime, no database, no Windows.

The example report above is from the public
[worrallbrian/crystal_reports](https://github.com/worrallbrian/crystal_reports) sample set, included in the test
fixtures.

This project was developed with the assistance of AI (Claude Opus 4.8).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, test, and structure changes.

## License

MPL-2.0. Crystal Reports is a product of its respective owner; this project is an independent, clean-room
implementation and is not affiliated with or endorsed by it.
