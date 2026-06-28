# The codebase

`rpt-rs` is a Cargo workspace of four crates. The split mirrors the decode pipeline and enforces one load-bearing
boundary: **stored facts vs. derived analytics**.

## Crates

| Crate        | Kind           | Responsibility                                                                                                                                         |
| ------------ | -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `rpt`        | library        | Read (and eventually write) the `.rpt` format: container → decryption → records → typed model. Decodes only what is _stored_ in the bytes.             |
| `rpt-engine` | library        | Analytics _derived_ from the model — values the Crystal engine computes rather than stores (e.g. field use counts), plus formula parsing and analysis. |
| `rpt-to-xml` | binary         | Serializes the model to a structured XML document.                                                                                                     |
| `rpt-cli`    | binary (`rpt`) | A read-only inspection CLI over the `rpt` library.                                                                                                     |

### The `rpt` / `rpt-engine` boundary

The rule: **if a value is in the bytes, it is decoded in `rpt`; if it is computed or inferred, it lives in
`rpt-engine`.** A derived value is never stored as a field on a core `rpt` model struct. This keeps the I/O layer a
faithful representation of the file and isolates inference (which can be wrong, or version-specific) in a separate crate
that consumes the model. The canonical example is a field's use count: it is not stored in the file, so `rpt-engine`
computes it by walking the model.

## Inside `rpt`

The library is a stack of layers, each in its own module, mirroring the decode pipeline. Reading and writing are
intended to be the same code run in opposite directions.

| Module                 | Layer   | Responsibility                                                                                                      |
| ---------------------- | ------- | ------------------------------------------------------------------------------------------------------------------- |
| `container`            | L0      | Open the CFB/OLE compound file; classify and read streams.                                                          |
| `codec`                | L0.5–L1 | The stream header, the cipher, decompression, record tiling and the recursive record tree (the masking lives here). |
| `records`              | L1      | The record model: the typed record stream, raw records, and the record-type registry (`RecordTag`).                 |
| `project`              | L2      | Raise the record tree into the typed model (`project::raise`, split by domain) and the inverse path.                |
| `model`                | L3      | The typed report model (the object graph callers use).                                                              |
| `io`                   | —       | Orchestration: ties the layers together into `Rpt::open` and exposes the report and its streams.                    |
| `error`, `diagnostics` | —       | The error type, and the crash/backtrace hook used by the binaries.                                                  |

The **lossless substrate** is the foundation: layers L0–L1 round-trip every record byte-identically, including records
that are not yet understood. The typed model (L3) is a projection on top that can grow without ever risking the
round-trip.

### `model` submodules

The model is split by domain: `document` (the top-level `Report` and summary info), `database` (connections, tables,
fields, links), `data_def` (parameters, formulas, groups, sorts, summaries), `report_def` (areas, sections, objects),
`objects` (the report object kinds), `format` (object and section formatting), `enums` (the SDK-style enumerations),
`primitives` (shared value types like `Twips`, `Color`, `Rect`, `Conditioned`), and `dom` (the generic record-tree view:
`Node`, `Value`, `Unknown`).

### `project::raise` submodules

The projection code is organized to match the model: `database`, `data_def`, `report_def` (with `conditions`,
`data_source`, `formats`), `parameters`, `print_options`, and shared helpers in `common`. This is where record bytes are
interpreted into typed elements — the layouts documented in the [block catalog](06-block-catalog.md) are implemented
here.

## Inside `rpt-engine`

`rpt-engine` consumes a `Report` and computes derived information. Its main piece is the `formula` module: a lexer,
parser, AST, and reference/type analysis for the Crystal formula language. These power use-count computation (which
fields and formulas are referenced, and how often) and other analytics that depend on understanding formula bodies.

## The binaries

- **`rpt-to-xml`** walks the model (plus `rpt-engine` analytics) and emits XML. It has a default mode (the modelled
  report) and a `--full` mode that also dumps the complete record tree.
- **`rpt-cli`** (`rpt`) is a read-only inspector with four subcommands and a `--json` flag. See [Usage](08-usage.md).

## Conventions

- The `rpt` crate forbids `unsafe` code.
- The minimum supported Rust version is 1.85.
- Dependencies are deliberately minimal: the CFB container, a zlib inflater, an XML writer, an error derive, and serde
  for the CLI's JSON output. The cipher is implemented in-crate with no cryptography dependency.
