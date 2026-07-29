# Render examples

A cookbook for driving the render pipeline from Rust. Each block is self-contained and pasteable; together they cover
loading a report, choosing where its rows come from, picking an output backend, feeding a report from your own data,
building for WebAssembly, and handling errors. For the pipeline's design — the Page IR, the coordinate model, format
resolution — see [Rendering](README.md); for the CLI, see [the `rpt-render` CLI](08-cli.md).

The facade and free functions live in the `rpt-render` crate. Examples that open a `.rpt` themselves also use
`rpt-reader`; those that build a custom data source use `rpt-data` (the record pipeline), `rpt-model` (the semantic
model types) and `rpt-formula` (the `Value` type); the error-handling example uses `rpt-pages` (`Severity`). Add
whichever you use to your `Cargo.toml`.

## Load and render, zero-config

[`ReportDocument`](https://docs.rs/rpt-render) is the SDK-shaped facade: one object that loads a report, holds its
decoded model, and exports it. The zero-config exporters (`to_pdf` / `export_pdf_to_disk`) render from the report's
**saved data** (or, with none, just the static header/footer bands); `to_pdf` cannot fail, and
`export_pdf_to_disk` fails only on the write.

```rust
use rpt_render::ReportDocument;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = ReportDocument::load("report.rpt")?;

    // Inspect the decoded model.
    let report = doc.report();
    println!("{} areas", report.report_definition.areas.len());

    // Zero-config export — rendering itself is infallible.
    let pdf: Vec<u8> = doc.to_pdf();
    std::fs::write("report.pdf", pdf)?;

    // Or write straight to disk (SDK-style `ExportToDisk`).
    doc.export_pdf_to_disk("out.pdf")?;
    Ok(())
}
```

To decode without the facade, `rpt_reader::Rpt::open("report.rpt")?.report()` hands back the same typed
[`Report`](../reader/01-semantic-model.md); the facade is sugar over it.

## Choosing where rows come from: `RenderOptions`

Where the rows come from is [`RenderOptions::datasource`](https://docs.rs/rpt-render), a
[`RenderSource`](https://docs.rs/rpt-render):

- `RenderSource::Saved` (the default) — the report's own saved data if present, else zero rows.
- `RenderSource::Rows(&dyn RowSource)` — a live or custom row feed (see [below](#feeding-a-report-from-your-own-data)).
- `RenderSource::Dataset(&Dataset)` — a pipeline result you built yourself.

`render_with` is the options-driven path — it also carries report parameters, the render locale, an optional subreport
scope, and the render's as-of instant. It is **infallible**, like every other render entry point: each built-in
`RenderSource` (saved data, a materialized `RowSource`, a pre-built `Dataset`) always succeeds, because fetching the
rows happened before the call. Because `RenderOptions` derives `Default`, set only the fields you need and spread the
rest;
`RenderOptions::default()` is exactly the zero-config render.

```rust
use rpt_render::{Locale, RenderOptions, RenderSource, ReportDocument};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = ReportDocument::load("report.rpt")?;

    // Default: the report's saved data.
    let from_saved = doc.render_with(RenderOptions::default());

    // A custom RowSource, with a non-default locale. `EmptySource` stands in for a real feed.
    let source = rpt_data::EmptySource;
    let from_rows = doc.render_with(RenderOptions {
        datasource: RenderSource::Rows(&source),
        locale: Locale::from_tag("de-DE"),
        ..RenderOptions::default()
    });

    println!("{} / {} pages", from_saved.pages.len(), from_rows.pages.len());
    Ok(())
}
```

### Parameters and locale

`RenderOptions::params` supplies report parameter current-values so formulas referencing `{?Name}` resolve; it is a
[`Parameters`](https://docs.rs/rpt-data) map (ignored when the datasource is a pre-built `Dataset`, which carries its
own). `RenderOptions::locale` is the render [`Locale`](https://docs.rs/rpt-format-value) — separators, month/day names,
AM/PM, default decimals — merged with each field's stored format record. `Locale::from_tag("en-US" | "de-DE" | …)` falls
back to en-US for an unknown tag.

`Parameters` is a `HashMap<String, Value>` keyed by the *normalized* parameter name — `normalize_param_name` drops any
surrounding `{}` and a leading `?` and lowercases — so a formula's `{?Region}` resolves to the value you set under
`"region"`:

```rust
use rpt_formula::eval::Value;
use rpt_data::{normalize_param_name, Parameters};
use rpt_render::{RenderOptions, RenderSource, ReportDocument};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = ReportDocument::load("report.rpt")?;

    let mut params = Parameters::new();
    params.insert(normalize_param_name("Region"), Value::Str("North".into()));

    let source = rpt_data::EmptySource;
    let pages = doc.render_with(RenderOptions {
        datasource: RenderSource::Rows(&source),
        params,
        ..RenderOptions::default()
    });
    println!("{} pages", pages.pages.len());
    Ok(())
}
```

### A reproducible render: `as_of`

`RenderOptions::as_of` fixes the instant the date/time specials (`CurrentDate`/`Today`, `CurrentDateTime`,
`CurrentTime`)
resolve to. Left `None`, it captures the system clock once at render start, so the whole render is internally consistent
but differs run to run — set it explicitly for frozen baselines. On `wasm32` there is no wall clock, so the default
falls back to the Unix epoch and a host that needs a real date must supply this.

## Picking a backend via the Page IR

Every exporter funnels through one seam: [`render`](https://docs.rs/rpt-render) produces a
[`PagedDocument`](https://docs.rs/rpt-pages) — the backend-agnostic Page IR of positioned draw-ops in twips — and a
backend consumes it. PDF is the only backend in-tree; [`render_backend`](https://docs.rs/rpt-render) is the seam it
attaches through, so a caller can hold a backend as a **value**, and an out-of-tree target (a viewer, another file
format) plugs in the same way without this crate knowing about it:

```rust
use rpt_render::{render, render_backend, PdfBackend, PdfOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rpt = rpt_reader::Rpt::open("report.rpt")?;
    let pages = render(rpt.report()); // PagedDocument

    let pdf: Vec<u8> = render_backend(&pages, &PdfBackend, &PdfOptions::default());
    std::fs::write("report.pdf", pdf)?;
    Ok(())
}
```

The backend re-exported from `rpt-render` and its output:

| Backend      | Options      | Output                         |
|--------------|--------------|--------------------------------|
| `PdfBackend` | `PdfOptions` | `Vec<u8>` (one multi-page PDF) |

`render_ir_json(report)` is a further output: the normalized Page-IR JSON, one string per page — not a backend but the
surface a render regression is diffed at, and the handiest thing to snapshot in a test.

## Feeding a report from your own data

A report doesn't have to render from its saved data. [`rpt_data::RowSource`](https://docs.rs/rpt-data) is the extension
point — a schema and the rows:

```rust
pub trait RowSource {
    fn columns(&self) -> &[Column];
    fn rows(&self) -> Vec<Row>;

    // Both have defaults, so a minimal source can ignore them.
    fn coercions(&self) -> Vec<ColumnCoercion> { Vec::new() }
    fn already_selected(&self) -> bool { false }
}
```

A `Column` is a `name` plus a `FieldValueType`; a `Row` holds field values keyed by column name. Names resolve
case-insensitively and by both their full `table.field` and bare `field` forms, so a formula referencing either finds
the value. Build a row with `Row::insert`, which stores a value under both its full and short names.

A complete in-memory source, fed into a render:

```rust
use rpt_formula::eval::Value;
use rpt_data::{Column, Row, RowSource};
use rpt_model::FieldValueType;
use rpt_render::{RenderOptions, RenderSource, ReportDocument};

struct InMemorySource {
    columns: Vec<Column>,
    rows: Vec<Row>,
}

impl RowSource for InMemorySource {
    fn columns(&self) -> &[Column] {
        &self.columns
    }
    fn rows(&self) -> Vec<Row> {
        self.rows.clone()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let columns = vec![
        Column { name: "customers.name".into(), value_type: FieldValueType::String },
        Column { name: "customers.balance".into(), value_type: FieldValueType::Currency },
    ];

    let mut row = Row::default();
    row.insert("customers.name", Value::Str("Acme".into()));
    row.insert("customers.balance", Value::Currency(1250.0));

    let source = InMemorySource { columns, rows: vec![row] };

    // The short name resolves too — formulas may reference either form.
    assert_eq!(source.rows()[0].get("name"), Some(&Value::Str("Acme".into())));

    let doc = ReportDocument::load("report.rpt")?;
    let pages = doc.render_with(RenderOptions {
        datasource: RenderSource::Rows(&source),
        ..RenderOptions::default()
    });
    println!("{} pages", pages.pages.len());
    Ok(())
}
```

### The saved-data date-typing footgun

[`SavedDataSource`](https://docs.rs/rpt-data) is the built-in source over a report's stored rows, and its two
constructors differ for dates:

- `SavedDataSource::new(saved)` types each column from the saved batch's own schema. But a saved batch stores
  Date/DateTime fields as **integer** day serials typed as integers, so a date column surfaces as a bare number and
  never groups, sorts, or formats as a date.
- `SavedDataSource::from_report(saved, report)` reconciles the batch's physical types against the report's *declared*
  field types, re-typing those serial columns back to Date/DateTime. **Prefer this for offline renders** — it makes the
  saved-data path type dates exactly like the live-DB path.

The zero-config path already uses `from_report` internally; reach for the constructors directly only when building a
[`Dataset`](https://docs.rs/rpt-data) by hand:

```rust
use rpt_data::{build_dataset, SavedDataSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rpt = rpt_reader::Rpt::open("report.rpt")?;
    let report = rpt.report();

    if let Some(saved) = &report.saved_data {
        // Date columns are re-typed from the report's declared field types.
        let source = SavedDataSource::from_report(saved, report);
        let dataset = build_dataset(&source, &report.data_definition);
        let _ = dataset;
    }
    Ok(())
}
```

### Subreports: `ScopeData`

Subreports are nested reports with their own tables. To render them from live data instead of their own saved data,
supply a [`ScopeData`](https://docs.rs/rpt-data) provider — one method returning a boxed `RowSource` for a given (sub)
report scope, or `None` to fall back to that scope's saved data:

```rust
use rpt_data::{RowSource, ScopeData};
use rpt_model::Report;
use rpt_render::RenderOptions;

struct MyScopes;

impl ScopeData for MyScopes {
    fn rows_for(&self, report: &Report) -> Option<Box<dyn RowSource>> {
        // Inspect `report` (its tables/connection) and return a live source, or None to keep
        // that scope's saved data.
        let _ = report;
        None
    }
}

fn main() {
    let scopes = MyScopes;
    let opts = RenderOptions { scope: Some(&scopes), ..Default::default() };
    let _ = opts;
}
```

## The live-database path

When a report has no saved data, its rows come from a live database. The built-in drivers `rpt-db-postgres` and
`rpt-db-sqlite` implement `RowSource` over a real database, executing the joined `SELECT` that `rpt-query` builds from
the report's table/link graph. They are **native-only** and isolated behind the trait, so the portable render core never
links them (a WASM build simply omits them — see [below](#building-for-webassembly)).

### CLI: `rpt-render --db`

The CLI wires the drivers up from the **environment** rather than a flag, so a connection URL (and any embedded
password) never lands in `argv`. Connections are keyed by **server**: set `RPT_DB_URL_<SERVER>` per distinct server,
or — for a single-server report — the generic `RPT_DB_URL` / `DATABASE_URL`. The URL's scheme selects the backend
(`postgres://…`, `sqlite://…`).

```sh
# Discover exactly which variables this report needs.
rpt-render report.rpt --list-sources

# Render from a live database (URL from the environment).
RPT_DB_URL='postgres://user:pass@host:5432/dbname' rpt-render report.rpt --db -o out.pdf -v
```

See [Usage](08-cli.md#database-configuration---db) for the full CLI contract.

### Library: your own `RowSource` over a live connection

From Rust, you own the fetch: query your database however you like, wrap the result in a `RowSource`, and pass it via
`RenderSource::Rows`. The [in-memory source above](#feeding-a-report-from-your-own-data) is the whole pattern — swap its
fixed rows for rows you fetched. This keeps the database dependency in *your* code, not the render pipeline, and is
exactly how a WASM host supplies rows fetched in JavaScript.

To use a built-in driver directly, construct its `RowSource` (each `rpt-db-*` crate exposes one that runs the
`rpt-query` SQL) and hand it to `RenderSource::Rows` the same way.

## Building for WebAssembly

The whole decode → data → layout → Page IR → backend chain is portable and compiles to `wasm32-unknown-unknown`. What is
*not* WASM-safe lives behind a seam:

- The native database drivers (`rpt-db-postgres` / `rpt-db-sqlite`) — a WASM build omits them and supplies its own
  `RowSource` (fetch rows in JS, wrap them; see [above](#library-your-own-rowsource-over-a-live-connection)).
- `rpt-text`'s system-font scan — cosmic-text can shape on WASM, but scanning OS fonts uses `std::fs`. Nothing scans by
  default (the bundled faces are the default source on both halves of the stack), so this only bites a caller that asks
  for `FontSource::System`; inject host fonts explicitly instead (below).
- Not the output backend: **`rpt-render-pdf`** (krilla) compiles for `wasm32-unknown-unknown` unmodified. A WASM render
  draws with the bundled faces unless the host injects its own or stops at the Page IR and draws it itself.

`rpt-render` has no features. The font-accurate cosmic-text layout is always compiled, including for WASM — the bundled
faces are `include_bytes!`d, so shaping needs no filesystem. (The `db-postgres` /
`db-sqlite` features belong to the **`rpt-render-cli`** binary, not the library — the facade never links a driver, so
there is nothing to turn off here.)

```text
cargo build -p rpt-render --target wasm32-unknown-unknown
```

### `ApproxLayout` and its pagination divergence

`ApproxLayout` is no longer a default anything — it is reached only by passing it to
`render_dataset_with`, which the data-driven baselines do deliberately. It is dependency-free, but only *approximate* —
a fixed average advance per em and greedy space-based wrapping. It triggers wrapping and stacks lines, but it is **not
metric-accurate and not script-aware** (it cannot wrap CJK, which has no spaces). Because wrap points and can-grow
heights feed pagination, **page counts from an approximate layout are not byte-identical with a real font stack** — the
paginator emits a one-shot diagnostic when an approximate layout is in use. Fine for a quick preview; use a real font
stack when pagination must match.

### Injecting a font-loaded layout

The default layout already never touches the filesystem — `RenderOptions::fonts` defaults to `FontSource::Bundled`, i.e.
a `FontProvider` with `use_system_fonts: false` and no local dirs, which loads only the bundled metric-compatible
fallback faces. Build the `CosmicLayout` yourself when you have **host-supplied** fonts to add: register them with
`load_font_bytes`, then hand the layout to `render_dataset_with` — the bring-your-own-layout entry point:

```rust
use rpt_data::{build_dataset, EmptySource};
use rpt_render::{render_dataset_with, CosmicLayout, FontProvider, Locale};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rpt = rpt_reader::Rpt::open("report.rpt")?;
    let report = rpt.report();

    // In a real WASM app, rows come from a JS-side fetch wrapped in a RowSource.
    let dataset = build_dataset(&EmptySource, &report.data_definition);

    // No filesystem: bundled fallback faces only.
    let provider = FontProvider { use_system_fonts: false, local_dirs: Vec::new() };
    let layout = CosmicLayout::new(provider);

    // Register fonts the host handed us (e.g. bytes fetched in JS).
    let font_bytes: Vec<u8> = Vec::new();
    layout.load_font_bytes(font_bytes);

    // The remaining three arguments are the render locale, per-subreport scope rows, and the
    // "current" instant (`None` captures the clock; on WASM, the Unix epoch).
    let pages = render_dataset_with(report, &dataset, Box::new(layout), Locale::from_tag("en-US"), None, None);
    println!("{} pages", pages.pages.len());
    Ok(())
}
```

The same `render_dataset_with` is how a native caller reuses one `CosmicLayout` across renders (avoiding a per-render
font scan); on WASM it is how host-supplied fonts get in.

That covers the *metrics* half. The PDF backend resolves its own faces, and its default is the bundled set too, so
`PdfOptions::default()` needs nothing here; state it explicitly when the surrounding code should be obvious about it:

```rust
use rpt_render::{render_backend, FontSource, PdfBackend, PdfOptions};

// `pages` is the PagedDocument from the render above.
let opts = PdfOptions { fonts: FontSource::Bundled, ..PdfOptions::default() };   // the default; spelled out
let pdf: Vec<u8> = render_backend(&pages, &PdfBackend, &opts);
```

The two halves are independent: mismatching them (bundled metrics, host faces) lays text out to one face's advances and
draws it in another's, so set both the same way — `FontSource::System` on both to read the host's library, which is what
the CLI's `--system-fonts` does. Bundled on both sides is what makes a render reproducible off the machine that produced
it, the property a committed baseline rests on, and it is why bundled is the default.

## Error handling

**Rendering does not fail.** Every entry point in `rpt-render` — `render`, `render_with`, `render_dataset_with`,
`render_backend`, and the facade's `to_pdf` — returns its output directly, not a `Result`. What is fallible is loading
and writing:

- `ReportDocument::load` returns `rpt_reader::Result` (a decode error).
- `export_pdf_to_disk` returns `Result<(), ExportError>` (writing the file). It has its own error rather than
  `std::io::Result` so the failure **names the path** — an embedding caller gets the same "which file?" answer the CLI
  does, instead of a bare `No such file or directory` — and rather than the reader's error type, which would advertise
  container, codec and crypto failures an export cannot reach.

The exception is the `try_*` family — `try_render_document`, `try_render_pages_with_options`,
`try_render_pages_with_assets`, re-exported from `rpt-render-pdf` — which returns `Result<_, PdfError>`: a font that
would not embed, an image that would not encode, a serialization failure, or a **checked** conformance claim (`--pdfa` /
`--pdfua`) the document does not meet. Their infallible counterparts absorb the first three into a PDF that names the
failure; take the `try_*` form when you need to handle it, and take it always when you asked for a conformance level,
since a file claiming a standard it does not meet is worse than no file.

Infallible does not mean nothing went wrong. Everything the pipeline discovers and works around is collected on the
document as a **diagnostic**, so the caller decides what to do about it. Two families arrive there:

- **Layout/render fidelity** — a chart with no plottable series, a WMF picture it can only draw as a placeholder, an
  unimplemented formula builtin, a runtime formula error, a substituted font.
- **Data-pipeline fail-open** — a record-selection formula that errored and **dropped the row**, a `{@formula}` that
  resolved to `Null`, a group-selection failure, an unsupported group condition, or a cell that would not parse as its
  declared type. These are the consequential ones: enough dropped rows and the report renders empty while reporting
  success. `render_with` attaches the collecting sink for you.

```rust
use rpt_pages::Severity;
use rpt_render::{RenderOptions, ReportDocument};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let doc = ReportDocument::load("report.rpt")?;   // fallible: decode

    let pages = doc.render_with(RenderOptions::default());   // infallible
    for diag in &pages.diagnostics {
        // `describe()` renders message, source, and location: e.g.
        //   "type mismatch in comparison: Currency vs Range (Order Total) [page 2, Details, record 41]"
        match diag.severity {
            Severity::Error => eprintln!("error: {}", diag.describe()),
            Severity::Warning => eprintln!("warning: {}", diag.describe()),
        }
    }

    doc.export_pdf_to_disk("out.pdf")?;              // fallible: I/O, and the error names the path
    Ok(())
}
```

The severity rule is worth knowing when deciding what to escalate: a fail-open that **discarded data** is an `Error`
(the output does not represent the input), one that kept the data but formatted or grouped it differently is a
`Warning`. If you build the `Dataset` yourself rather than letting `render_with` do it, the pipeline's diagnostics are
yours to collect — pass a `rpt_data::CollectingSink` via `rpt_data::DatasetOptions` and convert them with
`rpt_render::data_diagnostics::from_evals`, or they are silently discarded.

The `rpt-render` **CLI** does have a typed error — `rpt_render_cli::RenderError`, with `Rpt` / `Datasource` / `Params` /
`Db` / `Io` / `Output` / `Conformance` variants — because resolving a connection URL, coercing `-p` values, checking a
`--pdfa`/`--pdfua` claim, and writing multi-page output are all things the binary owns. It is a property of the CLI, not
of the library. `RenderError::hint()` carries the follow-up where there is one: for a `Db` failure the failing
statement, and for a missing table or column a pointer at `rpt sql <file>`; for a `Conformance` failure, one line per
unmet requirement.

---

← [Testing the renderer](09-testing-parity.md) · **Back to the** [rendering index](README.md)
