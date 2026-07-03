# Usage

Two binaries and one library. All of them open a `.rpt` directly — no SAP runtime, no database.

Build the binaries first (`cargo build --release`); they land in `target/release/` as `rpt` and `rpt-to-xml`. The
examples below assume those binaries are on your `PATH` — otherwise call them by path (e.g. `./target/release/rpt`) or
via Docker (see the README).

## CLI: `rpt` (inspection)

The `rpt-cli` crate builds the `rpt` binary, a read-only inspector. It opens the compound file, decrypts and decodes its
streams, and reports on them. Every command takes a file and an optional `--json` flag.

```
rpt <COMMAND> <file.rpt> [--json]
```

| Command          | What it prints                                                                                                             |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `inspect <file>` | A one-screen summary: report version, summary info (title / author / timestamps / application), and a per-stream overview. |
| `inputs <file>`  | The report's external inputs — every parameter it defines, with its type — in declaration order.                           |
| `streams <file>` | Raw substrate coverage per stream: record count and how many records are still undecoded.                                  |
| `strings <file>` | Every readable string (4+ characters) recovered from the decoded `Contents` tree.                                          |

`--json` emits the command's output as JSON instead of text, for scripting.

```sh
# Quick look
rpt inspect report.rpt

# Parameters, machine-readable
rpt inputs report.rpt --json

# How much of each stream is decoded
rpt streams report.rpt
```

## CLI: `rpt-to-xml` (export)

The `rpt-to-xml` crate exports a report to a structured XML document.

```
rpt-to-xml [OPTIONS] <file.rpt> [out.xml]
```

- With no output path, the XML is written to stdout.
- `--full` exports everything: the modelled report plus a `<Records>` tree of every decoded record (typed where
  recognized, raw otherwise) with its decoded leaf values.
- `-h` / `--help` shows usage.

```sh
# Default: the modelled report
rpt-to-xml report.rpt out.xml

# Everything, including the raw record tree, to stdout
rpt-to-xml --full report.rpt
```

The XML is useful for inspection and for diffing report definitions in version control.

## Library: `rpt`

Open a file and work with the typed model.

```rust
use rpt::Rpt;

let rpt = Rpt::open("report.rpt")?;
let report = rpt.report();

// Summary info
if let Some(title) = &report.summary_info.title {
    println!("Title: {title}");
}

// Data sources
for table in &report.database.tables {
    println!("Table: {}", table.name);
}

// Parameters
for param in report.data_definition.parameters() {
    println!("Parameter: {}", param.name);
}

// Saved data (cached rows), when the report was saved with data.
// `columns` and each `rows` entry line up positionally, so it reads as a simple matrix.
if let Some(saved) = &report.saved_data {
    // Header: column names.
    let header: Vec<&str> = saved.columns.iter().map(|c| c.name.as_str()).collect();
    println!("{}", header.join("\t"));

    // Rows: one cell per column (`None` is a null cell).
    for row in &saved.rows {
        let cells: Vec<&str> = row.iter().map(|c| c.as_deref().unwrap_or("")).collect();
        println!("{}", cells.join("\t"));
    }
}
# Ok::<(), rpt::Error>(())
```

`Rpt::open` returns a handle that owns the decoded report and its streams:

- `rpt.report()` — the typed [`Report`](05-semantic-model.md) model.
- `rpt.streams()` — the decoded streams, for substrate-level inspection.

The model types are re-exported from `rpt::model`, and a convenience `rpt::prelude` re-exports the most common ones. The
exact field names of the model are documented by the crate's API docs (`cargo doc -p rpt --open`); the
[semantic model](05-semantic-model.md) and [block catalog](06-block-catalog.md) explain what each part means.

### Derived analytics

Values the Crystal engine computes rather than stores (such as field use counts) are not on the `rpt` model — they are
produced by the `rpt-engine` crate, which takes a `Report` and walks it. Use `rpt-engine` when you need that derived
information; use `rpt` alone when you only need the stored facts. See [The codebase](07-codebase.md) for the boundary.

## Error handling

`Rpt::open` returns `rpt::Result<_>`. Errors distinguish a malformed or unreadable file from internal decode limits, so
callers can tell "this file is broken" from "this file uses something not yet supported".
