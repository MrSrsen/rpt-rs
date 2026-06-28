# Changelog

All notable changes to rpt-rs will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),

## [Unreleased]

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
- **Release workflow.** On a version tag, publishes cross-platform binaries (Linux, macOS, Windows) to a GitHub Release and pushes the Docker image to the GitHub Container Registry.
- **Documentation.** A guide to the `.rpt` format and the library under [`docs/`](docs/).
