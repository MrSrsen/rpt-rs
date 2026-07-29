<div align="center">
  <img src="docs/assets/rpt-rs.png" alt="rpt-rs logo" width="180" />
  <p>
    <a href="LICENSE"><img alt="License: MPL-2.0" src="https://img.shields.io/badge/license-MPL--2.0-blue.svg"></a>
    <img alt="Rust 1.92+" src="https://img.shields.io/badge/rust-1.92%2B-orange.svg">
  </p>
</div>

# rpt-rs

> ⚠️ This project is still experimental and its API is unstable. Expect major refactorings and breaking changes. If you
> need stability, pin a commit or fork the repository.

**rpt-rs** reads and renders **Crystal Reports `.rpt`** files in pure Rust — no Crystal Reports runtime, no Windows, no
.NET. Point it at a `.rpt` file and it can:

- **Inspect** the report: its data sources, parameters, formulas, groups, sections, and objects.
- **Export** the full report definition as JSON or KDL — handy for search, review, and diffing reports in version
  control.
- **Render** the report to paginated **PDF**, using the data saved inside the file or rows fetched
  live from a database.
- **Evaluate and check Crystal formulas** — both formula dialects, usable as a standalone library.
- **Run anywhere**: Linux, macOS, Windows, and (for the render core) WebAssembly.

`.rpt` is the native, proprietary file format of SAP Crystal Reports, a trademark of SAP SE. rpt-rs is an independent
implementation, reverse-engineered from the file format; it is not associated with, endorsed by, or sponsored by SAP SE,
and requires no SAP software to run.

## Example

A report carrying its own saved data renders to a print-ready PDF in one command, with no database anywhere:

```sh
rpt-render tests/fixtures/reports/worrall/SportsTeams.rpt -o out.pdf
```

Below is the first page of the repository's own [Meridian](tests/meridian/) product catalog — a report stored *without*
usable saved data, so it takes `--db` and reads its rows from the committed synthetic seed database instead:

<div align="center">
  <img src="docs/assets/example-render.png" alt="A report page rendered by rpt-rs" width="560" />
</div>

Everything on the page comes out of the `.rpt`: the embedded logo, the group hierarchy, the banded rows, per-row image
fields, the multi-currency prices, the conditional hazard flag, and the `Page 1 of 37` footer that only resolves once
pagination is done.

## Installation

The crates are not on crates.io yet — build from source with a Rust toolchain:

```sh
cargo build --release
```

The two binaries you want land in `target/release/`: `rpt` (inspect/export) and `rpt-render` (render). Or skip Rust
entirely and build the [Docker image](docs/project/building.md#docker), which contains nothing but those two binaries,
statically linked.

## Usage

A taste of what the two binaries do; `--help` on either lists the rest.

```sh
rpt inspect report.rpt              # version, summary info, streams
rpt json-dump report.rpt out.json   # the whole decoded definition as JSON (or `rpt kdl`)
rpt sql report.rpt                  # every SQL it can run — generated and stored — with provenance
rpt formulas report.rpt             # check every formula; exit 1 if any is broken

rpt-render report.rpt -o out.pdf                    # render the saved data
rpt-render report.rpt -p Region=West > out.pdf      # pass a parameter, write to stdout
rpt-render report.rpt --pdfa 2b -o archive.pdf      # archival PDF/A — the claim is checked
```

To read rows from a live database instead, pass `--db`; the URL comes from the environment rather than a flag, and
`--list-sources` tells you which variable to set:

```sh
RPT_DB_URL='postgres://user:pass@host:5432/db' rpt-render report.rpt --db -o out.pdf
```

## Library

The same two capabilities as crates — read the typed model, or render one:

```rust
let rpt = rpt_reader::Rpt::open("report.rpt")?;
println!("{}", rpt.report().summary_info.title);

let doc = rpt_render::ReportDocument::load("report.rpt")?;
doc.export_pdf_to_disk("out.pdf")?;    // …or `doc.to_pdf()` for bytes; the render itself never fails
```

More recipes — a live database, your own rows, WASM — are in the [render examples](docs/rendering/10-examples.md).

## Project state

**Reading is the mature half.** The decode pipeline — container, decryption, decompression, record tree, subreports — is
lossless on every file in the test corpus, and on top of it sit the report model and its JSON/KDL exports, the formula
engine, and the render pipeline: layout and pagination, charts, cross-tabs, live PostgreSQL/SQLite sources, and tagged
PDF with checked PDF/A and PDF/UA conformance.

**Writing is byte-level only.** Records round-trip and a decoded record's field bytes can be patched, but there is no
model→records lowering: you cannot yet mutate the semantic model and serialize it back. A few record families — maps,
OLAP grids, alerts, Flash widgets, XML/XSLT export definitions — are recognized but not decoded, for want of a report
that uses one.

The [support matrix](docs/reader/02-support-matrix.md) is the feature-by-feature account.

## Documentation

Everything lives in [`docs/`](docs/) — start at the [documentation index](docs/README.md). Five sets, one per domain,
each with its own index and reading order:

- **[What is in the file](docs/format/README.md)** — the `.rpt` binary format: container, cipher, record tree, saved
  data, block catalog. Language-agnostic.
- **[Inspecting and exporting reports](docs/reader/README.md)** — the typed report model, what the decoder supports, the
  `rpt` CLI, and the library API.
- **[Rendering reports](docs/rendering/README.md)** — the pipeline from data to Page IR to PDF, the render API,
  pagination and typography, the `rpt-render` CLI, and how the renderer is tested.
- **[Writing and checking formulas](docs/formula-engine/README.md)** — the Crystal/Basic formula language, its VM, its
  builtins, and its validator.
- **[Working on rpt-rs](docs/project/README.md)** — the crate map and its boundaries, and how to build, test and release
  the workspace.

## Acknowledgments

This project would not have been possible without **[RptToXml](https://github.com/ajryan/RptToXml)** by ajryan, an
early reference for reading the format.

This project was developed with the assistance of AI (Claude Opus 4.8/5.0).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, test, and structure changes.

## License

MPL-2.0, with two exceptions inside the workspace:

- [`metafile`](crates/metafile) is **MIT OR Apache-2.0**, so it is reusable outside this project.
- The fonts bundled in `rpt-text` keep their own licences — the Liberation family under the SIL Open Font License 1.1,
  DejaVu under the Bitstream Vera licence. Both texts ship in [`crates/rpt-text/fonts/`](crates/rpt-text/fonts/).

The binaries are statically linked, so a release archive carries its whole dependency tree and both font families with
it. [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md) reproduces their licences and ships in every archive and in the
Docker image.

SAP and SAP Crystal Reports, as well as their respective logos, are trademarks or registered trademarks of SAP SE (or an
SAP affiliate company) in Germany and other countries. All other product and service names mentioned are the trademarks
of their respective companies.
