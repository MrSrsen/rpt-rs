# rpt-rs

[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](LICENSE)
![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)

A pure-Rust library and command-line tools for reading **SAP Crystal Reports (`.rpt`)** files without the SAP runtime, a
database connection, or any Windows component.

`.rpt` is an undocumented, encrypted binary format. `rpt-rs` opens the file, decrypts and decompresses its streams,
decodes the internal record structure, and projects it into a typed report model you can inspect or export.

## What it does

- **Reads `.rpt` directly** — opens the OLE/CFB compound file, decrypts the report streams, inflates them, and decodes
  the internal records. No SAP DLLs, no database, cross-platform.
- **Builds a typed model** — projects the records into a structured report model: data sources and tables, parameters,
  formulas, groups, sections, and the laid-out report objects.
- **Exports to XML** — serializes the model to a structured XML document for inspection or for diffing report
  definitions in version control.
- **Inspects from the CLI** — summarizes a report, lists its parameters, and dumps streams and recovered strings, as
  text or JSON.
- **Round-trips losslessly** — every record is preserved exactly, including record types not yet modelled, so nothing is
  lost when reading.

## Workspace

| Crate        | What it is                                                                           |
| ------------ | ------------------------------------------------------------------------------------ |
| `rpt`        | The core library: container → decryption → records → typed model.                    |
| `rpt-engine` | Derived analytics computed from the model (e.g. field use counts, formula analysis). |
| `rpt-to-xml` | XML exporter (binary).                                                               |
| `rpt-cli`    | Inspection CLI (binary: `rpt`).                                                      |

## Build from source

The crate is not published to crates.io yet, so build it from source (or use Docker below). With a Rust toolchain
installed:

```sh
cargo build --release
```

This writes two self-contained binaries to `target/release/`:

- `rpt` — the inspection CLI
- `rpt-to-xml` — the XML exporter

## Usage

Run the built binaries directly — no cargo needed at runtime:

```sh
# Inspect a report
./target/release/rpt inspect report.rpt

# List the report's parameters as JSON
./target/release/rpt inputs report.rpt --json

# Export to XML
./target/release/rpt-to-xml report.rpt out.xml
```

As a library:

```rust
let rpt = rpt::Rpt::open("report.rpt")?;
let report = rpt.report();
println!("{}", report.summary_info.title.as_deref().unwrap_or(""));
```

## Docker

Don't have Rust? A multistage build produces a tiny image (~14 MB) containing only the two statically linked binaries —
no toolchain, shell, or OS packages.

```sh
docker build -t rpt-rs .

# Inspect a report (mount the directory holding it as /data)
docker run --rm -v "$PWD:/data" rpt-rs rpt inspect report.rpt

# Export to XML
docker run --rm -v "$PWD:/data" rpt-rs rpt-to-xml report.rpt out.xml
```

Both `rpt` and `rpt-to-xml` are on the image's `PATH`; override the command to run either.

Prebuilt images are published to the GitHub Container Registry on each release:

```sh
docker pull ghcr.io/mrsrsen/rpt-rs:latest
```

## Documentation

Full technical documentation lives in [`docs/`](docs/):

- [Documentation index](docs/README.md)
- [Format overview](docs/01-format-overview.md) — what `.rpt` is and how decoding works, end to end
- [The container](docs/02-container.md) — the CFB compound file and its streams
- [Stream decoding](docs/03-stream-decoding.md) — encryption, compression, and record framing
- [The record tree](docs/04-record-tree.md) — record nesting, masking, and the lossless substrate
- [The semantic model](docs/05-semantic-model.md) — projecting records into the report model
- [Block catalog](docs/06-block-catalog.md) — every decoded record type, what it means, and its layout
- [The codebase](docs/07-codebase.md) — crates and modules
- [Usage](docs/08-usage.md) — CLI tools and the library API
- [Support matrix](docs/09-support-matrix.md) — what is and isn't supported

## Acknowledgments

This project would not have been possible without **[RptToXml](https://github.com/ajryan/RptToXml)** by ajryan. RptToXml
exports a Crystal Reports `.rpt` file to XML using the SAP Crystal Reports runtime/SDK. The `rpt-to-xml` binary here is,
in effect, a reimplementation of RptToXml: it produces the same kind of XML, but by decoding the `.rpt` bytes directly —
without the SAP runtime, a database, or Windows OS.

This project was developed with the assistance of AI (Claude Opus 4.8).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, test, and structure changes.

## License

MPL-2.0.
