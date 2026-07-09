# Render examples

A cookbook for driving the render pipeline from Rust. Each block is self-contained and pasteable; together they cover
loading a report, choosing where its rows come from, picking an output backend, feeding a report from your own data,
building for WebAssembly, and handling errors. For the pipeline's design — the Page IR, the coordinate model, format
resolution — see [Rendering](11-rendering.md); for the CLI, see [Usage](08-usage.md).

The facade and free functions live in the `rpt-render` crate. Examples that build a custom data source also use
`rpt-data` (the record pipeline), `rpt-model` (the semantic model types), and `crystal-formula` (the `Value` type). Add
whichever you use to your `Cargo.toml`.

## Load and render, zero-config

[`ReportDocument`](https://docs.rs/rpt-render) is the SDK-shaped facade: one object that loads a report, holds its
decoded model, and exports it. The zero-config exporters (`to_pdf` / `to_html` / `export_svg_pages`) render from the
report's **saved data** (or, with none, just the static header/footer bands) and **never fail**.

```rust
use rpt_render::ReportDocument;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = ReportDocument::load("report.rpt")?;

    // Inspect the decoded model.
    let report = doc.report();
    println!("{} areas", report.report_definition.areas.len());

    // Zero-config export — these paths are infallible.
    let pdf: Vec<u8> = doc.to_pdf();
    let html: String = doc.to_html();
    let svg_pages: Vec<String> = doc.export_svg_pages();

    std::fs::write("report.pdf", pdf)?;
    std::fs::write("report.html", html)?;
    let _ = svg_pages;

    // Or write straight to disk (SDK-style `ExportToDisk`).
    doc.export_pdf_to_disk("out.pdf")?;
    doc.export_html_to_disk("out.html")?;
    Ok(())
}
```

To decode without the facade, `rpt::Rpt::open("report.rpt")?.report()` hands back the same typed
[`Report`](05-semantic-model.md); the facade is sugar over it.

## Choosing where rows come from: `RenderOptions`

Where the rows come from is [`RenderOptions::datasource`](https://docs.rs/rpt-render), a
[`RenderSource`](https://docs.rs/rpt-render):

- `RenderSource::Saved` (the default) — the report's own saved data if present, else zero rows.
- `RenderSource::Rows(&dyn RowSource)` — a live or custom row feed (see [below](#feeding-a-report-from-your-own-data)).
- `RenderSource::Dataset(&Dataset)` — a pipeline result you built yourself.

`render_with` is the fallible, options-driven path — it also carries report parameters, the render locale, and an
optional subreport scope. Because `RenderOptions` derives `Default`, set only the fields you need and spread the rest;
`RenderOptions::default()` is exactly the zero-config render.

```rust
use rpt_render::{Locale, RenderOptions, RenderSource, ReportDocument};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = ReportDocument::load("report.rpt")?;

    // Default: the report's saved data.
    let from_saved = doc.render_with(RenderOptions::default())?;

    // A custom RowSource, with a non-default locale. `EmptySource` stands in for a real feed.
    let source = rpt_data::EmptySource;
    let from_rows = doc.render_with(RenderOptions {
        datasource: RenderSource::Rows(&source),
        locale: Locale::from_tag("de-DE"),
        ..RenderOptions::default()
    })?;

    println!("{} / {} pages", from_saved.pages.len(), from_rows.pages.len());
    Ok(())
}
```

### Parameters and locale

`RenderOptions::params` supplies report parameter current-values so formulas referencing `{?Name}` resolve; it is a
[`Parameters`](https://docs.rs/rpt-data) map (ignored when the datasource is a pre-built `Dataset`, which carries its
own). `RenderOptions::locale` is the render [`Locale`](https://docs.rs/rpt-format-value) — separators, month/day names,
AM/PM, default decimals — merged with each field's stored format leaf. `Locale::from_tag("en-US" | "de-DE" | …)` falls
back to en-US for an unknown tag.

`Parameters` is a `HashMap<String, Value>` keyed by the *normalized* parameter name — `normalize_param_name` drops any
surrounding `{}` and a leading `?` and lowercases — so a formula's `{?Region}` resolves to the value you set under
`"region"`:

```rust
use crystal_formula::eval::Value;
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
    })?;
    println!("{} pages", pages.pages.len());
    Ok(())
}
```

## Picking a backend via the Page IR

Every exporter funnels through one seam: [`render`](https://docs.rs/rpt-render) produces a
[`PagedDocument`](https://docs.rs/rpt-pages) — the backend-agnostic Page IR of positioned draw-ops in twips — and each
backend consumes it. [`render_backend`](https://docs.rs/rpt-render) lets a caller pick a backend as a **value** (e.g.
from a CLI flag) rather than calling a format-specific function:

```rust
use rpt_render::{
    render, render_backend, HtmlBackend, HtmlOptions, PdfBackend, PdfOptions, RasterBackend,
    RasterOptions, SvgBackend,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rpt = rpt::Rpt::open("report.rpt")?;
    let pages = render(rpt.report()); // PagedDocument

    let pdf: Vec<u8> = render_backend(&pages, &PdfBackend, &PdfOptions::default());
    let html: String = render_backend(&pages, &HtmlBackend, &HtmlOptions);
    let svgs: Vec<String> = render_backend(&pages, &SvgBackend, &());
    let pngs: Vec<Vec<u8>> = render_backend(&pages, &RasterBackend, &RasterOptions::default());

    std::fs::write("report.pdf", pdf)?;
    let _ = (html, svgs, pngs);
    Ok(())
}
```

The four backends re-exported from `rpt-render` and their outputs:

| Backend | Options | Output |
| ------- | ------- | ------ |
| `HtmlBackend` | `HtmlOptions` | `String` (one self-contained document) |
| `SvgBackend` | `()` | `Vec<String>` (one SVG per page) |
| `PdfBackend` | `PdfOptions` | `Vec<u8>` (one multi-page PDF) |
| `RasterBackend` | `RasterOptions` | `Vec<Vec<u8>>` (one PNG per page) |

## Feeding a report from your own data

A report doesn't have to render from its saved data. [`rpt_data::RowSource`](https://docs.rs/rpt-data) is the extension
point — a schema and the rows:

```rust
pub trait RowSource {
    fn columns(&self) -> &[Column];
    fn rows(&self) -> Vec<Row>;
}
```

A `Column` is a `name` plus a `FieldValueType`; a `Row` holds field values keyed by column name. Names resolve
case-insensitively and by both their full `table.field` and bare `field` forms, so a formula referencing either finds
the value. Build a row with `Row::insert`, which stores a value under both its full and short names.

A complete in-memory source, fed into a render:

```rust
use crystal_formula::eval::Value;
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
    })?;
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
    let rpt = rpt::Rpt::open("report.rpt")?;
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
supply a [`ScopeData`](https://docs.rs/rpt-data) provider — one method returning a boxed `RowSource` for a given
(sub)report scope, or `None` to fall back to that scope's saved data:

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
the report's table/link graph. They are **native-only** and isolated behind the trait, so the portable render core
never links them (a WASM build simply omits them — see [below](#building-for-webassembly)).

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

See [Usage](08-usage.md#database-configuration---db) for the full CLI contract.

### Library: your own `RowSource` over a live connection

From Rust, you own the fetch: query your database however you like, wrap the result in a `RowSource`, and pass it via
`RenderSource::Rows`. The [in-memory source above](#feeding-a-report-from-your-own-data) is the whole pattern — swap its
fixed rows for rows you fetched. This keeps the database dependency in *your* code, not the render pipeline, and is
exactly how a WASM host supplies rows fetched in JavaScript.

To use a built-in driver directly, construct its `RowSource` (each `rpt-db-*` crate exposes one that runs the
`rpt-query` SQL) and hand it to `RenderSource::Rows` the same way.

## Building for WebAssembly

The whole decode → data → layout → Page IR → backend chain is portable and compiles to `wasm32-unknown-unknown`. What
is *not* WASM-safe lives behind a seam:

- The native database drivers (`rpt-db-postgres` / `rpt-db-sqlite`) — a WASM build omits them and supplies its own
  `RowSource` (fetch rows in JS, wrap them; see [above](#library-your-own-rowsource-over-a-live-connection)).
- `rpt-text`'s system-font scan — cosmic-text can shape on WASM, but scanning OS fonts uses `std::fs`. Inject fonts
  explicitly instead (below).
- Of the four backends, **`rpt-render-html` and `rpt-render-svg`** build for wasm32; **`rpt-render-raster`** and
  **`rpt-render-pdf`**'s default backend are native-only.

`rpt-render`'s default features (`cosmic`, `db-postgres`, `db-sqlite`) pull native-only code, so disable them for a
WASM target:

```text
# ApproxLayout default — dependency-free, no fonts, no DB crates:
cargo build -p rpt-render --target wasm32-unknown-unknown --no-default-features

# Font-accurate layout on WASM — cosmic-text without the system-font scan:
cargo build -p rpt-render --target wasm32-unknown-unknown --no-default-features --features cosmic
```

### `ApproxLayout` and its pagination divergence

With `--no-default-features`, the default text layout is `ApproxLayout`: dependency-free, but only *approximate* — a
fixed average advance per em and greedy space-based wrapping. It triggers wrapping and stacks lines, but it is **not
metric-accurate and not script-aware** (it cannot wrap CJK, which has no spaces). Because wrap points and can-grow
heights feed pagination, **page counts from an approximate layout are not byte-identical with a real font stack** — the
paginator emits a one-shot diagnostic when an approximate layout is in use. Fine for a quick preview; use a real font
stack when pagination must match.

### Injecting a font-loaded layout

For metric-accurate layout on WASM, build a `CosmicLayout` that never touches the filesystem — a `FontProvider` with
`use_system_fonts: false` and no local dirs loads only the bundled metric-compatible fallback faces. Add any
host-supplied fonts with `load_font_bytes`, then hand the layout to `render_dataset_with` — the bring-your-own-layout
entry point:

```rust
use rpt_data::{build_dataset, EmptySource};
use rpt_render::{render_dataset_with, CosmicLayout, FontProvider};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rpt = rpt::Rpt::open("report.rpt")?;
    let report = rpt.report();

    // In a real WASM app, rows come from a JS-side fetch wrapped in a RowSource.
    let dataset = build_dataset(&EmptySource, &report.data_definition);

    // No filesystem: bundled fallback faces only.
    let provider = FontProvider { use_system_fonts: false, local_dirs: Vec::new() };
    let layout = CosmicLayout::new(provider);

    // Register fonts the host handed us (e.g. bytes fetched in JS).
    let font_bytes: Vec<u8> = Vec::new();
    layout.load_font_bytes(font_bytes);

    let pages = render_dataset_with(report, &dataset, Box::new(layout));
    println!("{} pages", pages.pages.len());
    Ok(())
}
```

The same `render_dataset_with` is how a native caller reuses one `CosmicLayout` across renders (avoiding a per-render
font scan); on WASM it is how host-supplied fonts get in.

## Error handling

The zero-config paths (`render` and the `to_*` / `export_*` facade methods) are **infallible**. `render_with` returns
[`RenderError`](https://docs.rs/rpt-render), a typed cause a caller can match on — a decode error, a datasource
problem, a parameter-coercion failure, a database-driver error, or an output error — rather than a message string:

```rust
use rpt_render::{RenderError, RenderOptions, ReportDocument};

fn run() -> Result<(), RenderError> {
    // `load` returns rpt::Result; `?` converts via `RenderError: From<rpt::Error>`.
    let doc = ReportDocument::load("report.rpt")?;

    // Infallible.
    let _pdf: Vec<u8> = doc.to_pdf();

    // Fallible — a typed cause.
    match doc.render_with(RenderOptions::default()) {
        Ok(pages) => println!("{} pages", pages.pages.len()),
        Err(RenderError::Datasource(msg)) => eprintln!("datasource: {msg}"),
        Err(RenderError::Params(msg)) => eprintln!("parameter: {msg}"),
        Err(RenderError::Db(msg)) => eprintln!("database: {msg}"),
        Err(RenderError::Io(msg)) => eprintln!("output: {msg}"),
        Err(e) => eprintln!("{e}"),
    }
    Ok(())
}
```

`RenderError` is `#[non_exhaustive]`, so match with a trailing `_` arm as above.
