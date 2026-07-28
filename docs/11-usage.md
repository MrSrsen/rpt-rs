# Usage

The tools: the `rpt` binary (inspection, JSON/KDL export, and a byte-level write path), the `rpt-render` binary
(rendering to HTML / SVG / PDF / PNG), and the `rpt` library. A database connection is needed only to render a report
that has no usable saved data; every other operation reads the file alone.

Build the binaries first (`cargo build --release`); they land in `target/release/` as `rpt` and `rpt-render`. The
examples below assume they are on your `PATH` — otherwise call them by path (e.g. `./target/release/rpt`) or via Docker
(see the README).

## CLI: `rpt` (inspection)

The `rpt-cli` app (`apps/rpt-cli`) builds the `rpt` binary. Most of its commands are read-only inspectors — they open the compound
file, decrypt and decode its streams, and report on them; three write-path commands (`anonymize` / `reencode` /
`patch`, below) run the
byte-faithful re-encoder to a new file. Every command takes a file and an optional `--json` flag.

```
rpt <COMMAND> <file.rpt> [--json] [--depth N] [--color | --no-color]
```

| Command          | What it prints                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
|------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `inspect <file>` | A one-screen summary: report version, summary info (title / author / timestamps / application), each chart's data binding (value + category fields), and a per-stream overview.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `inputs <file>`  | The report's external inputs — every parameter it defines, with its type — in declaration order.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `tree <file>`    | A structural tree of the decoded record DOM, grouped by source stream. Each node is tagged by kind — `CfbStream(<name>)` (first tier: the main report's `Contents`, then each subreport's `Subdocument N/Contents`), `Branch(<type>)` (a node with nested children), or `Leaf(<type>)` (a node with none) — where `<type>` is the registry name or raw `0xNNNN` word. For record types the decoder understands (the field-format leaves, group-area options, and summary definitions) the node shows a concise decoded summary of the record's stored values (e.g. `DecimalPlaces=2 Negative=Bracketed`); every other record shows a truncated preview of its raw content. Node kinds use plain CFB/tree vocabulary, not project-specific terms. With `--json`, a recognized node carries the decoded values as a `decoded` object alongside `preview`. |
| `streams <file>` | Raw substrate coverage per stream: record count, how many are still Unknown (undecoded), logical vs on-disk byte sizes, and the top record types. The meter for record-type decode coverage.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `dump <file…>`   | The byte-layout workbench for a record's raw bytes. Selects records by `--type` (hex `0x76` or a registry name like `Formula`) and dumps each one's **demasked leaf bytes** — the exact bytes a `raise` decoder reads via `RecordNode::leaf_bytes` — as annotated hex, plus the length-prefixed strings it contains (mirrors the reader's `read_lp_string`) and a scalar-probe grid (u16/u32, big- and little-endian, at every offset) mapping 1:1 onto the `crate::bytes` reader vocabulary. With no `--type` it prints the stream's record-type index; given two or more files it also prints a byte-aligned minimal-pair diff of the first match in each.                                                                                                                                                                                    |
| `saved <file>`   | The report's **decoded saved-data rows**: the column schema (names + value types) and the cached rowset a report carries when saved with data (stored record order; not the engine's result rowset). `--schema` prints only the columns + record count; `--limit N` caps the rows (default 20; `all` for every row). The decoded-rows counterpart to `dump`'s raw bytes — reports the record count even when the batch class doesn't decode, so you know to `dump` the raw bytes.                                                                                                                                                                                                                                                                                                                                                                       |
| `sql <file>`     | **Every SQL statement the report can run against its database**, each tagged with where it came from: the engine's generated join `SELECT` (built from the table/link graph, pruned to the referenced tables, with the record-selection formula pushed into `WHERE`), each **stored SQL Command** (`Table.CommandText`) emitted verbatim, and each **SQL Expression field** — recursively through subreports. Also summarises the **connections** (server / database / driver / user) and the table list. `--dialect postgres` (default) `/ sqlite / mysql` picks the generated query's dialect. Static analysis — **no database connection is made**; it's the SQL the report *would* issue.                                                                                                                                                           |
| `formulas <file>` | **Every formula the report defines, listed and checked** — one line per formula marked `ok`/`warn`/`ERROR`/`empty`, with findings indented beneath, so you can see what was covered and not just how many; `--source` quotes each body. Covers formula fields, the record- and group-selection formulas, and conditional-format formulas wherever they hang (section, object format, border, font colour), through every subreport. Reports syntax errors (the parser) and semantic ones (unknown functions, wrong arity, operator type errors) with the offending byte span. Exits 1 if any formula has an error, so it works as a CI gate. Reads the file alone — no database, no render. `--quiet` reports only through the exit status. This is the only thing that rejects a broken formula: everywhere else the parser recovers, the evaluator runs the partial parse, and the field renders blank. |

`--json` emits the command's output as JSON instead of text, for scripting. `--depth N` (for `tree`) caps the tree at N
record levels (the stream tier is always shown); deeper nodes are collapsed to a `… N more` marker.
`--dialect D` (for `sql`) selects the generated query's SQL dialect (`postgres` (default) / `sqlite` / `mysql`).
`rpt <command> --help` prints scoped, per-command help.

`dump` options: `--type T`, `--nth N` (0-based match index), `--stream S` (`contents` (default) / `qe` / `all` / a
stream-id substring), `--probe N` (scalar-grid cap; `0` off, `all` whole leaf), `--whole` (dump the whole masked on-disk
span instead of the demasked leaf), `--offset O --len L` (raw span escape hatch), and `--saved` (inspect the saved-data
batch substrate: the decoded schema, the batch directory, and each batch's derived decrypt IV + whether it inflates —
with an IV search on the batches whose derived IV fails, so an unmodelled batch class is still reachable). See the
**rpt-dump** skill for the byte-layout workflow.

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

# Byte layout: annotated leaf bytes of the first formula record
rpt dump report.rpt --type Formula --nth 0

# Minimal-pair diff: which bytes moved between two near-identical reports?
rpt dump base.rpt variant.rpt --type 0x0121

# The decoded saved-data rows (schema + cached rowset)
rpt saved report.rpt --limit all

# Every SQL the report can issue (generated queries + stored commands), with provenance
rpt sql report.rpt

# The same as JSON (e.g. to extract just the stored SQL Commands), targeting SQLite
rpt sql report.rpt --dialect sqlite --json

# Check every formula for syntax and semantic errors, without rendering (exit 1 if any error)
rpt formulas report.rpt
```

`rpt formulas` exists because nothing else rejects a broken formula: the parser recovers from a syntax error, the
evaluator runs the partial parse, and the field renders blank — indistinguishable from a null value or an unimplemented
feature. It **lists every formula it checked**, marked `ok` / `warn` / `ERROR`, with any findings indented beneath —
a count alone would not tell you whether the formula you care about was among them:

```console
$ rpt formulas report.rpt
report.rpt
  ok     the record-selection formula                    crystal, 1 line
  ERROR  section Details's Section_Visibility formula    crystal, 1 line
           error: expected `)` at byte 14 (near `;`)
  empty  formula "Unused"                                crystal, 0 lines

1 formula checked, 1 declared but empty — 1 error, 0 warnings
```

`empty` is a formula field the report declares but left blank — listed so the accounting is complete, and excluded from
the checked count since there is nothing to verify. `--source` quotes each formula's body under its line:

```console
$ rpt formulas report.rpt --source
report.rpt
  ok     formula "Xcount"  crystal, 12 lines
         │ Local NumberVar Index;
         │ Local NumberVar Xcount := 0;
         │ For Index := 1 to Length({Product.Size}) Step 1 Do (
         │     If ({Product.Size}[Index] = "x") Then (Xcount := Xcount + 1;)
         │ );
         │ Xcount
```

Coverage is formula fields, the record- and group-selection formulas, and conditional-format formulas wherever they
hang — on a **section**, an object's format, its **border**, or a field/text object's **font colour** — through every
subreport. (Sections are where most of them live in practice.) `--json` always includes each formula's source, kind,
syntax, and size, with no `--source` needed; `--quiet` reports only through the exit status.

## CLI: `rpt` write path (`anonymize` / `reencode` / `patch`)

Three commands run the **byte-faithful re-encoder** — the write path of the `rpt` library — to a fresh `.rpt`. All take
an explicit output path and only ever write that one file; none edits in place. This is a substrate-level writer: it round-trips and
byte-patches the raw record bytes. There is **no model→records lowering** — you cannot mutate the decoded semantic model
and serialize it back; edits are byte-patches against a decoded record's leaf.
(See [the support matrix](08-support-matrix.md)
and [the codebase](10-codebase.md) for the boundary.)

| Command                                                    | What it does                                                                                                                                                                                                                                   |
|------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `reencode <in.rpt> <out.rpt>`                              | Re-encodes `<in.rpt>`'s `Contents` stream from its own logical bytes (a no-op writer round-trip) and writes `<out.rpt>`. The result re-opens to byte-identical record bytes; only the compressed file bytes differ (deflate is non-canonical). |
| `patch <in.rpt> <tag> <nth> <offset> <hexbytes> <out.rpt>` | Locates the `<nth>` (0-based, pre-order) record of type `<tag>` in the `Contents` record tree, overwrites `len(<hexbytes>)` bytes of its demasked leaf starting at `<offset>`, then re-encodes to `<out.rpt>`. **Same-size only.**             |
| `anonymize <in.rpt> <out.rpt>`                             | Removes personally identifying authoring metadata (below) and writes `<out.rpt>`. `--dry-run` reports without writing; `--json` for machine-readable output.                                                                                    |

`patch` arguments: `<tag>` is the record type as hex (e.g. `0x64`) or decimal; `<nth>` is the 0-based occurrence of that
type in pre-order; `<offset>` is the byte offset into the demasked leaf; `<hexbytes>` is the replacement bytes as hex
(e.g. `01ff2a`), whose length sets the region size.

The CLI exposes only the same-size form. From Rust, `Rpt::patch_record_leaf_resize` also replaces a leaf region with
bytes of a *different* length, recomputing the record's and every enclosing record's length prefix — see
[the codebase](10-codebase.md).

**The clearance gate.** An edit to a record type that is not *cleared for safe editing* is refused, and nothing is
written. The reason is that the mechanical checks (record exists, region inside the leaf, length prefixes fit) cannot
catch the failure that matters: a record whose leaf carries an internal offset table, element count, or checksum can be
overwritten into a `.rpt` that re-encodes, re-opens, and re-decodes perfectly while being semantically corrupt — the
damage surfaces later in the Crystal designer or as a wrong render, with nothing pointing back at the edit. So the
default is refuse, and a record type is added to the allow-list (`crates/rpt/src/io/cleared.rs`) only with evidence that
editing it cannot desynchronize anything.

`--force` (`EditPolicy::Forced` from Rust) writes the edit anyway. That is the right flag when writing a deliberately
invalid record is the point — probing what a field means — and the wrong one for editing a report you intend to keep.

```sh
# Prove the writer round-trips: re-encode Contents to a new file
rpt reencode report.rpt out.rpt

# Overwrite 2 bytes at offset 12 of the first ReportRoot (0x64) record's leaf.
# 0x64 is not cleared for editing, so this is refused unless --force is passed.
rpt patch --force report.rpt 0x64 0 12 01ff out.rpt

# See what authoring metadata a report carries, without writing anything
rpt anonymize report.rpt --dry-run
```

### Anonymizing a report

A `.rpt` records who made it and where. The OLE `SummaryInformation` property set holds the **author** and the **last
person to save**; a re-imported subreport holds the full path of the `.rpt` it came from
(`\\HOST\user\Documents\…`). None of it affects rendering, and all of it follows the file into any corpus it is
committed to. `rpt anonymize` removes it:

- **`author`, `last_saved_by`** — blanked. They are identity and nothing else.
- **`reimport.source_path`** — reduced to its **file name**, not blanked. A non-empty source path is the only evidence in
  the file that a subreport was imported at all, and `SubreportObject.IsImported` is resolved from it, so emptying it
  would silently turn a true fact false. The directory prefix is the identifying part; the bare file name is the
  subreport's own name, which the `Subdocument` storage already records.

The database connection's stored path is deliberately **left alone**: it is a live datasource locator, not authoring
metadata, and blanking it would break the report against its own data.

Every edit is **same-length** — a value's length prefix is untouched and only its characters are rewritten, then padded
with NULs, which readers of both formats stop at. No record length, property offset or section size moves, so the result
is a structurally identical file whose decoded model is unchanged apart from those fields, and which the real Crystal
engine still opens. A report with nothing to remove is returned byte-identical, so the command is idempotent and safe to
re-run across a corpus.

From Rust the same pass is [`Rpt::anonymize`], which returns the new bytes alongside a report of every value removed and
what it was replaced with.

[`Rpt::anonymize`]: https://docs.rs/rpt/latest/rpt/struct.Rpt.html#method.anonymize

## CLI: `rpt` export (`json-dump` / `kdl`)

Two subcommands export the whole decoded report, differing in how much they say. Both take an optional output path and
write to stdout when it is omitted; neither needs a database. They are thin callers of the `rpt-json` and `rpt-kdl`
libraries — the same surfaces you can call in-process (see [the codebase](10-codebase.md)).

```
rpt json-dump <file.rpt> [out.json] [--strict]
rpt kdl       <file.rpt> [out.kdl]  [--strict]
```

- **`json-dump`** writes the **exhaustive** JSON document: the full serde serialization of the decoded model under
  `model` — every field, including defaults, and the whole subreport tree. **Stored facts only**: nothing inferred or
  recomputed, so a change in the output always means a change in the decode. Output is deterministic (sorted-key maps,
  two-space indent, trailing newline) and depends on nothing outside the `.rpt`, which is what makes it usable as a
  regression baseline and as one side of an engine comparison.
- **`kdl`** writes the model as a [KDL](https://kdl.dev) document: a sparse, human-readable, lossless view. Given an
  output path it also writes each embedded picture beside it as a sidecar file (the KDL carries a `source=` reference
  instead of the bytes); on stdout the pictures are referenced but not written.

```sh
# Everything the decoder knows, as JSON
rpt json-dump report.rpt out.json

# The same model, human-readable
rpt kdl report.rpt out.kdl
```

Use `json-dump` for scripting, structural diffs, and regression baselines; `kdl` for reading and reviewing a report
definition in version control. To see the raw record tree instead of the model, use `rpt tree`.

**Incomplete decodes.** Projection is infallible by design — a record the reader does not recognize becomes a default
rather than an error — so an export missing content otherwise looks exactly like a faithful one. Both commands warn on
stderr (never stdout, so a piped export is unaffected) when the report did not decode completely, naming the
unrecognized record types and pointing at `rpt streams` for the breakdown. `--strict` makes that a failure instead
(exit 1) for CI, where a silent partial export is worse than a stopped build; the document is still written, so it
remains available for working out what is missing.

## CLI: `rpt-render` (rendering)

The `rpt-render-cli` app (`apps/rpt-render-cli`) builds the `rpt-render` binary: it opens a report, runs the data pipeline + layout engine,
and writes the paginated result through the chosen backend. It resolves the five inputs a render needs — the report, a
datasource, a locale, parameters, and an output format/destination. The [rendering guide](12-rendering.md) covers the
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

| Scheme                                                     | Status                          |
|------------------------------------------------------------|---------------------------------|
| `postgres://` (or `postgresql://`)                         | implemented                     |
| `sqlite:///path/to/file.db` (or `sqlite::memory:`)         | implemented                     |
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
- `rpt.record_dom()` / `rpt.inventory()` — the generic [record tree](04-record-tree.md) and its per-type census,
  projected on demand from the substrate (the types live in `rpt::raw`).

The model types are re-exported **flat** from `rpt::model` (the standalone `rpt-model` crate), and a convenience
`rpt::prelude` re-exports the most common ones. The exact field names of the model are documented by the crate's API docs
(`cargo doc -p rpt --open`); the [semantic model](05-semantic-model.md) and [block catalog](07-block-catalog.md) explain
what each part means. `Report::objects()` walks every placed object in layout order, across all areas. For a binary
front-end, `rpt::install_panic_hook()` installs a crash/backtrace hook (the one the `rpt` and `rpt-render` binaries use).

### Derived analytics

Values the Crystal engine computes rather than stores — a field's use count, a formula's runtime result length, the
locale-resolved display format — are not on the `rpt` model, and are not in the JSON export either. They are not
properties of the file, so the library does not pretend to report them. Where one is genuinely needed it is computed by
the consumer that needs it: `rpt-layout` resolves the effective display format for rendering, and
`crystal_formula::string_max_bytes` recomputes a formula's result width where a live datasource is available. A
derivation more than one consumer needs, and that requires only the model, goes in `rpt_model::analysis` as a pure
function. See [The codebase](10-codebase.md) for the boundary.

## Error handling

`Rpt::open` returns `rpt::Result<_>`. Errors are layer-tagged (`Container`, `Codec`, `Crypto`, `NotAReport`, `Project`,
`Io`), so callers can tell "this file is broken" from "this file uses something not yet supported" — and an input that
is not a report at all is diagnosed rather than reported as a CFB library complaint:

```console
$ rpt inspect notes.txt
error: `notes.txt` is not a Crystal Reports report: it has no OLE2/CFB signature, which every `.rpt` starts
with; it looks like plain text
```

Two conventions apply throughout:

- **A variant that carries a `source` never interpolates it.** Printing `{e}` gives that layer's message; printing the
  `source()` chain gives each cause exactly once. Use [`rpt::error_chain`] to render the whole chain — both binaries do,
  so they report to one standard. A bare `{e}` on a wrapping variant is the layer's message *without* the cause.
- **An I/O error names the path.** `rpt::IoError` carries the operation and the file, so the commonest failure answers
  itself: ``cannot read `/nope/missing.rpt`: No such file or directory (os error 2)``.

Projection (records → model) is **infallible by design**: it returns defaults for anything it cannot interpret, so a
report the Crystal engine opens is never refused over a record this reader does not model. The cost is that an
incomplete decode raises no error, so it is reported as a *diagnostic* instead — see `Rpt::decode_coverage` and the
`--strict` flag above.

The render side is likewise infallible once a report is loaded (see
[Render examples › Error handling](13-render-examples.md#error-handling)), and for the same reason reports what it
swallowed: a `PagedDocument` carries `diagnostics`, each with a severity, a kind, and an optional structural location
(page, area/section, record index, formula span). The data pipeline's fail-open sites — a selection formula that errors
and drops the row, a `{@formula}` that resolves to `Null`, a cell that will not parse as its declared type — all arrive
there. Fetching rows from a live database *can* fail, and that happens in the caller — or, for the CLI, in
`rpt-render-cli`'s own `RenderError`.

[`rpt::error_chain`]: https://docs.rs/rpt/latest/rpt/fn.error_chain.html

---

← [The codebase](10-codebase.md) · [Index](README.md) · **Next:** [Rendering](12-rendering.md) →
