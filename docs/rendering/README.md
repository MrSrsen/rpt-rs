# Rendering

The render pipeline: a decoded report model plus a data source in, paginated PDF out. Read this if you are rendering
reports, supplying rows from your own datasource, or working on the layout engine or the PDF backend. It is built on
[the reader](../reader/README.md), and is pure, WASM-safe Rust.

Read front to back:

1. [The pipeline](01-pipeline.md) — the stages and the crate behind each one, and what a WASM target does and does not
   get.
2. [Driving a render (library API)](02-api.md) — the `ReportDocument` facade, the free functions, and every render
   option.
3. [The Page IR (`rpt-pages`)](03-page-ir.md) — the backend-agnostic page/draw-op contract, the coordinate model, and
   diagnostics.
4. [Section-break & pagination controls](04-pagination.md) — the section and group format flags the paginator honours.
5. [Format resolution](05-format-resolution.md) — the locale layer, the field's stored format, and how the effective
   display format is resolved.
6. [Paragraph typography](06-typography.md) — per-paragraph fonts and spacing, wrapping, and the resolved alignment
   default.
7. [Charts and cross-tabs](07-charts-crosstabs.md) — how both render as ordinary draw-ops, with no rasterization.
8. [The `rpt-render` CLI](08-cli.md) — the flag-and-contract reference, including the PDF/A and tagged-PDF levels.
9. [Testing the renderer](09-testing-parity.md) — the render-test corpora and the baseline layers.
10. [Render examples](10-examples.md) — copy-paste recipes: saved data, a live DB, a custom `RowSource`, and WASM.

---

← [Documentation index](../README.md) · **Start here:** [The pipeline](01-pipeline.md) →
