# The `rpt-reader` library

Open a file and work with the typed model.

```rust
use rpt_reader::Rpt;

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
# Ok::<(), rpt_reader::Error>(())
```

`Rpt::open` returns a handle that owns the decoded report and its streams:

- `rpt.report()` — the typed [`Report`](01-semantic-model.md) model.
- `rpt.streams()` — the decoded streams, for record-level inspection.
- `rpt.typed_record_tree()` / `rpt.inventory()` — the generic [record tree](../format/04-record-tree.md) and its
  per-type census, built on demand from the decoded records (the types live in `rpt_reader::raw`).

`raw` also holds what is needed to read those records: `Dialect` — the vocabulary a stream's type numbers are resolved
in, which `RecordTag::name` takes and `RecordTag::from_name` searches — and `lp_strings` / `LpString` / `LpScan`, this
reader's own answer to what in a record's bytes is text.

The model types live in `rpt_reader::model` (the standalone `rpt-model` crate, re-exported whole). The exact field names
of the model are documented by the crate's API docs (`cargo doc -p rpt-reader --open`);
the [semantic model](01-semantic-model.md) and [block catalog](../format/06-block-catalog.md) explain what each part
means. `Report::objects()` walks every placed object in layout order, across all areas.

## What is at the crate root

The crate root is the `Rpt::open` path: a name is re-exported flat when a caller meets it while driving `Rpt` — the
handle, what its methods hand back, and what is needed to build an argument or read a failure. A module that contributes
at all contributes its whole public vocabulary, so `error`, `coverage`, the facade and the container are flat in full.
What sits a level *below* that path is reached through its own module, entered deliberately: `raw` (the records the
model was built from), `fields` and `annotate` (a field table's own reading of one — which is also how a model field
maps back onto the record it came from). `model` is the one module contributing a subset — flat from it are only the
types walking a `Report` cannot reach. There is no prelude: a glob reaching past the root would hand over in one line
exactly what entering those modules deliberately is for. The crate docs state the rule in full.

## Derived analytics

Values the Crystal engine computes rather than stores — a field's use count, a formula's runtime result length, the
locale-resolved display format — are not on the `rpt-model` types, and are not in the JSON export either. They are not
properties of the file, so the library does not pretend to report them. Where one is genuinely needed it is computed by
the consumer that needs it: `rpt-layout` resolves the effective display format for rendering, and
`rpt_formula::string_max_bytes` recomputes a formula's result width where a live datasource is available. A derivation
more than one consumer needs, and that requires only the model, goes in `rpt-model` as a pure function (its
`analysis` module, re-exported flat at the crate root). See [The codebase](../project/codebase.md) for the boundary.

## Error handling

`Rpt::open` returns `rpt_reader::Result<_>`. Errors are layer-tagged (`Container`, `Codec`, `Crypto`, `NotAReport`,
`Edit`,
`Io`), so callers can tell "this file is broken" from "this file uses something not yet supported" — and an input that
is not a report at all is diagnosed rather than reported as a CFB library complaint:

```console
$ rpt inspect notes.txt
error: `notes.txt` is not a Crystal Reports report: it has no OLE2/CFB signature, which every `.rpt` starts
with; it looks like plain text
```

Two conventions apply throughout:

- **A variant that carries a `source` never interpolates it.** Printing `{e}` gives that layer's message; printing the
  `source()` chain gives each cause exactly once. Use [`rpt_reader::error_chain`] to render the whole chain — both
  binaries do, so they report to one standard. A bare `{e}` on a wrapping variant is the layer's message *without* the
  cause.
- **An I/O error names the path.** `rpt_reader::IoError` carries the operation and the file, so the commonest failure
  answers itself: ``cannot read `/nope/missing.rpt`: No such file or directory (os error 2)``.

Building the model (records → model) is **infallible by design**: it returns defaults for anything it cannot interpret,
so a report the Crystal engine opens is never refused over a record this reader does not model. The cost is that an
incomplete decode raises no error, so it is reported as a *diagnostic* instead — see `Rpt::decode_coverage` and the
[`--strict` flag](03-cli.md#export-json-dump--kdl). `DecodeCoverage` reports two axes: per stream, how much of it was
understood — the unrecognized record types over the stream's whole **record tree**, and the bytes belonging to no record
of its **linear walk**, which is the only walk whose spans lie side by side and can account for them; and, for the
saved-data path, what became of the stored rows (`Rpt::saved_data_status`) — which is what separates a report saved
without data from one whose batches would not decrypt, since both leave
`report.saved_data` empty.

Laying a report out is likewise infallible once it is loaded — a `PagedDocument` always comes back — and for the same
reason it reports what it swallowed: it carries `diagnostics`, each with a severity, a kind, and an optional structural
location (page, area/section, record index, formula span). The data pipeline's fail-open sites — a selection formula
that errors and drops the row, a `{@formula}` that resolves to `Null`, a cell that will not parse as its declared type —
all arrive there. Serializing those pages *can* fail — a font that will not embed, an image that will not encode, a
PDF/A or PDF/UA claim the document does not meet — so the PDF backend's `try_*` entry points return a `PdfError`; see
[Render examples › Error handling](../rendering/10-examples.md#error-handling). So can fetching rows from a live
database, and that happens in the caller — or, for the CLI, in `rpt-render-cli`'s own `RenderError`.

[`rpt_reader::error_chain`]: https://docs.rs/rpt-reader/latest/rpt_reader/fn.error_chain.html

---

← [The `rpt` CLI](03-cli.md) · **Back to the** [reader index](README.md)
