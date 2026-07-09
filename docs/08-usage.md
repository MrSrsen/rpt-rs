# Usage

The tools: the `rpt` binary (inspection, XML export, and a byte-level write path), the `rpt-render` binary (rendering
to HTML / SVG / PDF / PNG), and the `rpt` library. All open a `.rpt` directly — no Crystal Reports runtime, and no
database unless you render a report with no saved data against a live one.

Build the binaries first (`cargo build --release`); they land in `target/release/` as `rpt` and `rpt-render`. The
examples below assume they are on your `PATH` — otherwise call them by path (e.g. `./target/release/rpt`) or
via Docker (see the README).

## CLI: `rpt` (inspection)

The `rpt-cli` crate builds the `rpt` binary. Most of its commands are read-only inspectors — they open the compound
file, decrypt and decode its streams, and report on them; two write-path commands (`reencode` / `patch`, below) run the
byte-faithful re-encoder to a new file. Every command takes a file and an optional `--json` flag.

```
rpt <COMMAND> <file.rpt> [--json] [--depth N] [--color | --no-color]
```

| Command          | What it prints                                                                                                                                                 |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `inspect <file>` | A one-screen summary: report version, summary info (title / author / timestamps / application), each chart's data binding (value + category fields), and a per-stream overview. |
| `inputs <file>`  | The report's external inputs — every parameter it defines, with its type — in declaration order.                                                              |
| `tree <file>`    | A structural tree of the decoded record DOM, grouped by source stream. Each node is tagged by kind — `CfbStream(<name>)` (first tier: the main report's `Contents`, then each subreport's `Subdocument N/Contents`), `Branch(<type>)` (a node with nested children), or `Leaf(<type>)` (a node with none) — where `<type>` is the registry name or raw `0xNNNN` word, followed by a truncated preview of the content. Node kinds use plain CFB/tree vocabulary, not project-specific terms. |
| `streams <file>` | Raw substrate coverage per stream: record count, how many are still Unknown (undecoded), logical vs on-disk byte sizes, and the top record types. The meter for record-type decode coverage. |
| `dump <file…>`   | The byte-layout workbench for reverse-engineering a record. Selects records by `--type` (hex `0x76` or a registry name like `Formula`) and dumps each one's **demasked leaf bytes** — the exact bytes a `raise` decoder reads via `RecordNode::leaf_bytes` — as annotated hex, plus the length-prefixed strings it contains (mirrors the reader's `read_lp_string`) and a scalar-probe grid (u16/u32, big- and little-endian, at every offset) mapping 1:1 onto the `crate::bytes` reader vocabulary. With no `--type` it prints the stream's record-type index; given two or more files it also prints a byte-aligned minimal-pair diff of the first match in each. |
| `saved <file>`   | The report's **decoded saved-data rows**: the column schema (names + value types) and the cached rowset a report carries when saved with data (stored record order; not the engine's result rowset). `--schema` prints only the columns + record count; `--limit N` caps the rows (default 20; `all` for every row). The decoded-rows counterpart to `dump`'s raw bytes — reports the record count even when the batch class doesn't decode, so you know to `dump` the raw bytes. |
| `sql <file>`     | **Every SQL statement the report can run against its database**, each tagged with where it came from: the engine's generated join `SELECT` (built from the table/link graph, pruned to the referenced tables, with the record-selection formula pushed into `WHERE`), each **stored SQL Command** (`Table.CommandText`) emitted verbatim, and each **SQL Expression field** — recursively through subreports. Also summarises the **connections** (server / database / driver / user) and the table list. `--dialect postgres` (default) `/ sqlite / mysql` picks the generated query's dialect. Static analysis — **no database connection is made**; it's the SQL the report *would* issue. |

`--json` emits the command's output as JSON instead of text, for scripting. `--depth N` (for `tree`) caps
the tree at N record levels (the stream tier is always shown); deeper nodes are collapsed to a `… N more` marker.
`--dialect D` (for `sql`) selects the generated query's SQL dialect (`postgres` (default) / `sqlite` / `mysql`).
`rpt <command> --help` prints scoped, per-command help.

`dump` options: `--type T`, `--nth N` (0-based match index), `--stream S` (`contents` (default) / `qe` / `all` /
a stream-id substring), `--probe N` (scalar-grid cap; `0` off, `all` whole leaf), `--whole` (dump the whole masked
on-disk span instead of the demasked leaf), `--offset O --len L` (raw span escape hatch), and `--saved` (inspect the
saved-data batch substrate: the decoded schema, the batch directory, and each batch's derived decrypt IV + whether it
inflates — with an IV search on the batches whose derived IV fails, for cracking a new saved-data batch class). See
the **rpt-dump** skill for the reverse-engineering workflow.

`tree` and `sql` colorize by **prominence** — for `tree`, recognized record types and field/text content are
highlighted, large embedded data blobs (images / saved data, shown as `[N B blob]`) are flagged in magenta, while
scaffolding (unknown types, small byte runs, tree connectors) is dimmed; for `sql`, section headers, table names, and
each statement's source are highlighted while scaffolding (indices, separators, field labels) is dimmed and the SQL
bodies stay plain for readability. Color is on by default when writing to a terminal and off when piped/redirected.
`--color` forces it on (e.g. to keep colors through a pager), `--no-color` forces it off; the `NO_COLOR` and
`CLICOLOR_FORCE` environment variables are also honored. The codes are standard ANSI SGR sequences — no extra
dependency.

```sh
# Quick look
rpt inspect report.rpt

# Parameters, machine-readable
rpt inputs report.rpt --json

# What's inside, as a structural tree (first 3 levels)
rpt tree report.rpt --depth 3

# Keep the colors when paging
rpt tree report.rpt --color | less -R

# How much of each stream is decoded (undecoded-record coverage)
rpt streams report.rpt

# Reverse-engineering: annotated leaf bytes of the first formula record
rpt dump report.rpt --type Formula --nth 0

# Minimal-pair diff: which bytes moved between two near-identical reports?
rpt dump base.rpt variant.rpt --type 0x0121

# The decoded saved-data rows (schema + cached rowset)
rpt saved report.rpt --limit all

# Every SQL the report can issue (generated queries + stored commands), with provenance
rpt sql report.rpt

# The same as JSON (e.g. to extract just the stored SQL Commands), targeting SQLite
rpt sql report.rpt --dialect sqlite --json
```

## CLI: `rpt` write path (`reencode` / `patch`)

Two commands run the **byte-faithful re-encoder** — the write path of the `rpt` library — to a fresh `.rpt`. Both take
an explicit output path and only ever write that one file. This is a substrate-level writer: it round-trips and
byte-patches the raw record bytes. There is **no model→records lowering** — you cannot mutate the decoded semantic model
and serialize it back; edits are byte-patches against a decoded record's leaf. (See [the support matrix](09-support-matrix.md)
and [the codebase](07-codebase.md) for the boundary.)

| Command | What it does |
| ------- | ------------ |
| `reencode <in.rpt> <out.rpt>` | Re-encodes `<in.rpt>`'s `Contents` stream from its own logical bytes (a no-op writer round-trip) and writes `<out.rpt>`. The result re-opens to byte-identical record bytes; only the compressed file bytes differ (deflate is non-canonical). |
| `patch <in.rpt> <tag> <nth> <offset> <hexbytes> <out.rpt>` | Locates the `<nth>` (0-based, pre-order) record of type `<tag>` in the `Contents` record tree, overwrites `len(<hexbytes>)` bytes of its demasked leaf starting at `<offset>`, then re-encodes to `<out.rpt>`. **Same-size only.** |

`patch` arguments: `<tag>` is the record type as hex (e.g. `0x64`) or decimal; `<nth>` is the 0-based occurrence of
that type in pre-order; `<offset>` is the byte offset into the demasked leaf; `<hexbytes>` is the replacement bytes as
hex (e.g. `01ff2a`), whose length sets the region size.

```sh
# Prove the writer round-trips: re-encode Contents to a new file
rpt reencode report.rpt out.rpt

# Overwrite 2 bytes at offset 12 of the first ReportRoot (0x64) record's leaf
rpt patch report.rpt 0x64 0 12 01ff out.rpt
```

## CLI: `rpt xml-dump` (export)

The `xml-dump` subcommand exports a report to a structured XML document.

```
rpt xml-dump <file.rpt> [out.xml] [--full]
```

- With no output path, the XML is written to stdout.
- `--full` exports everything: the modelled report plus a `<Records>` tree of every decoded record (typed where
  recognized, raw otherwise) with its decoded leaf values.
- `-h` / `--help` shows usage.

```sh
# Default: the modelled report
rpt xml-dump report.rpt out.xml

# Everything, including the raw record tree, to stdout
rpt xml-dump --full report.rpt
```

The XML is useful for inspection and for diffing report definitions in version control.

## CLI: `rpt-render` (rendering)

The `rpt-render-cli` crate builds the `rpt-render` binary: it opens a report, runs the data pipeline + layout engine,
and writes the paginated result through the chosen backend. It resolves the five inputs a render needs — the report, a
datasource, a locale, parameters, and an output format/destination. The [rendering guide](11-rendering.md) covers the
pipeline design; this is the flag-and-contract reference.

```
rpt-render <file.rpt> [OPTIONS]

DATASOURCE (default: the report's saved data if present, else empty)
    --saved            use the report's embedded saved data
    --db               fetch rows live from the database URL(s) in the environment
    --list-sources     print the report's live sources + the env var to set for each, then exit

PARAMETERS
    -p, --param Name=Value   repeatable; repeat a name for a multi-value parameter

LOCALE
    --locale <tag>     e.g. en-US, de-DE (default: the host locale, else en-US)

OUTPUT
    -f, --format html|pdf|svg|png   default: inferred from -o's extension, else html
    -o, --output <path>  output file; '-' or omitted writes to stdout
    --force              overwrite existing multi-file (SVG/PNG) pages

LOGGING
    -v, --verbose      also log the SQL sent, timings, and push-down decisions
    -q, --quiet        errors only
```

HTML and PDF are single self-contained files (safe to pipe to stdout). SVG and PNG are one file per page
(`<base>-N.svg` / `<base>-N.png`), so they need a real `-o` path (a single-page report may still pipe one page).

### Parameters

`-p Name=Value` supplies a report parameter (list them with `rpt inputs <file>`). Each value is coerced to the
parameter's declared type. Repeat the same name to build a multi-value parameter:

```sh
rpt-render report.rpt -p AsOfDate=2026-01-31 -p Region=West -p Region=East -o out.html
```

### Locale

`--locale <tag>` selects the locale used for date/number formatting. Resolution precedence: an explicit `--locale`
overrides the host OS locale (`LC_ALL` / `LC_NUMERIC` / `LANG`), which overrides the `en-US` fallback. Built-in tags are
`en-US`, `en-GB`, `de-DE`, `fr-FR`, `es-ES`, and `it-IT`; an unrecognized tag formats with the `en-US` fallback (the CLI
warns). This mirrors the native engine, which reads the host locale once at process start to resolve "System Default"
formats — there is no stored per-report locale.

### Database configuration (`--db`)

When a report has no saved data (or you pass `--db`), rows come from a live database. The connection is a single URL
taken **only from the environment**, never a command-line flag, so the password never appears in `ps` output or shell
history. The URL **scheme** selects the backend:

| Scheme | Status |
| ------ | ------ |
| `postgres://` (or `postgresql://`) | implemented |
| `sqlite:///path/to/file.db` (or `sqlite::memory:`) | implemented |
| `mysql://` · `mariadb://` · `mssql://` (or `sqlserver://`) | recognized, not yet implemented |

For a single-server report, set `RPT_DB_URL` (or the 12-factor `DATABASE_URL` fallback; `RPT_DB_URL` takes precedence).
A report plus its subreports can read from more than one server; each distinct server gets its own
`RPT_DB_URL_<SERVER>` variable, where `<SERVER>` is the server name upper-cased with non-alphanumerics turned to `_`.
Run `--list-sources` to print the exact variable name for each source:

```sh
# Discover what --db needs for this report
rpt-render report.rpt --list-sources

# Render from a live database (URL from the environment), verbose
RPT_DB_URL='postgres://user:pass@host:5432/dbname' rpt-render report.rpt --db -o out.pdf -v
```

## Library: `rpt`

Open a file and work with the typed model.

```rust
use rpt::Rpt;

let rpt = Rpt::open("report.rpt")?;
let report = rpt.report();

// Summary info (title is a plain string; empty when the report sets none)
if !report.summary_info.title.is_empty() {
    println!("Title: {}", report.summary_info.title);
}

// Data sources
for table in &report.database.tables {
    println!("Table: {}", table.name);
}

// Parameters (each is a field definition paired with its parameter metadata)
for (field, _param) in report.data_definition.parameter_fields() {
    println!("Parameter: {}", field.name);
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

The model types are re-exported from `rpt::model` (the standalone `rpt-model` crate), and a convenience `rpt::prelude`
re-exports the most common ones. The exact field names of the model are documented by the crate's API docs
(`cargo doc -p rpt --open`); the [semantic model](05-semantic-model.md) and [block catalog](06-block-catalog.md) explain
what each part means. For a binary front-end, `rpt::install_panic_hook()` installs a crash/backtrace hook (the one the
`rpt` and `rpt-render` binaries use).

### Derived analytics

Values the Crystal engine computes rather than stores (such as field use counts) are not on the `rpt` model — they are
produced by the derive layer in the XML exporter (`rpt-cli`'s `export::analysis`, driven by `rpt xml-dump`), which takes
a `Report` and walks it. Use `rpt xml-dump` when you need that derived information; use the `rpt` library (or the `rpt`
inspection commands) alone when you only need the stored facts. See [The codebase](07-codebase.md) for the boundary.

## Error handling

`Rpt::open` returns `rpt::Result<_>`. Errors distinguish a malformed or unreadable file from internal decode limits, so
callers can tell "this file is broken" from "this file uses something not yet supported".
