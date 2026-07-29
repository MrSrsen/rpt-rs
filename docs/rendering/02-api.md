# Driving a render (library API)

The SDK-shaped facade mirrors `ReportDocument`. **Laying a report out never fails** — a `PagedDocument` always comes
back, with any fidelity problem reported as a [diagnostic](03-page-ir.md#diagnostics) rather than an error. What is
fallible is `load` (decode), the `export_*_to_disk` methods (file I/O), and *serializing* the pages, which the PDF
backend's `try_*` entry points report rather than absorb (see
[Render examples › Error handling](10-examples.md#error-handling)):

```rust
use rpt_render::ReportDocument;

let doc = ReportDocument::load("report.rpt")?;   // decode
doc.export_pdf_to_disk("out.pdf")?;              // render saved data → PDF
let pdf: Vec<u8> = doc.to_pdf();                 // …or bytes
```

Under the facade are free functions for finer control. The pipeline default is the report's **saved data** (the offline
path); with no saved data it runs over zero rows (headers/footers still format):

```rust
use rpt_render::{render, render_pdf};

let pages = render(report);                 // Report → PagedDocument (saved data)
let pdf   = render_pdf(report);             // → PDF bytes
```

To render from a **pre-built `Dataset`** (e.g. a live datasource), with an explicit locale and per-subreport scope rows,
use the options-driven entry point with `RenderSource::Dataset`:

```rust
use rpt_render::{render_with, Locale, RenderOptions, RenderSource};

let doc = render_with(
    report,
    RenderOptions {
        datasource: RenderSource::Dataset(&dataset),
        locale: Locale::from_tag("de-DE"),
        scope: Some(&scope_data),
        ..RenderOptions::default()
    },
);
for diag in &doc.diagnostics { /* fidelity warnings */ }
```

For a full end-to-end render cookbook — a custom `RowSource`, the live-DB library path, WASM, and error handling —
see [Render examples](10-examples.md).

- **`Locale`** (from `rpt-format-value`, re-exported): the render locale — separators, month/day names, AM/PM.
  `Locale::from_tag("en-US" | "de-DE" | …)`; unknown tags fall back to en-US.
  See [format resolution](05-format-resolution.md).
- **`ScopeData`**: supplies each subreport scope's rows so a whole tree renders from a live datasource, without
  `rpt-layout` depending on any DB crate. `None` renders subreports from their saved data. An **inline** subreport runs
  once per placement against a **per-instance** dataset filtered by the enclosing row's link values — a parameter-routed
  link (`SubreportLink.linked_parameter`) binds the parent field into the subreport's parameters and a direct field link
  applies a structural equality filter (`rpt_data::build_dataset_with` + `FieldFilter`). `Shared`
  variables accumulated inside a subreport are visible to the main report (shared eval scope). An **on-demand**
  subreport (`SubreportObject.on_demand`) is not executed — it emits only its caption placeholder, matching the engine's
  click-to-expand behaviour in a static export. A subreport taller than its placeholder box **grows the enclosing
  band**: it is formatted once ahead of pagination (so its `Shared`/`Global` writes fire exactly once), the band grows
  to its full height (reusing the can-grow machinery), and the checkpoint pagination flows the enlarged band across
  pages. A subreport taller than a **whole page** is split across parent pages at row boundaries (distinct op-bottom Y
  values): still formatted once, the split is pure geometry over the cached ops, so a subreport with an internal forced
  page break puts each of its pages on its own parent page. A subreport that fits on a page is placed atomically (moved
  whole to the next page when the space left is too small).
- **`TextLayout`**: `rpt-text`'s `CosmicLayout` is the default; inject a pre-built one (to avoid re-scanning fonts per
  render, or to supply host fonts on WASM) or the dependency-free `ApproxLayout` via `render_dataset_with`.
- **Fonts** (a `FontSource`, on both halves of the stack). Which faces exist decides two separate things, so it is set
  in two places: **`RenderOptions::fonts`** picks the library the default `CosmicLayout` takes its *metrics* from (and
  therefore the wrap points, can-grow heights and page count), and **`PdfOptions::fonts`** picks the library the backend
  *resolves, shapes and subsets* from. **`Bundled` is the default on both**: the compiled-in Liberation/DejaVu set
  alone, so the same report and rows yield the same geometry and the same bytes on every machine — the source a
  committed baseline is blessed against, and what a fontless host (WASM, a minimal container) wants. `System` reads the
  host's installed library instead, so the output becomes a property of the machine: a host with a real Arial lays out
  to and embeds Arial, one without falls back to Liberation Sans. That is worth asking for when the report's real faces
  are installed and fidelity beats reproducibility — the bundled Liberation set is metric-compatible with Arial, Times
  New Roman and Courier New and **nothing else**, so for any other family the two modes differ. Set both the same way,
  or text is laid out to one face's advances and drawn in another's; the CLI's `--system-fonts` flips both at once.
  `PdfOptions::fonts` reaches the writer through `PdfBackend` (`render_backend(&doc, &PdfBackend, &opts)`) or the
  `try_render_document` / `try_render_pages_with_options` free functions; the entry points that take no options
  (`render_pdf`, `to_pdf`) use the default. Prefer the whole-document ones: only they carry the document's assets and
  section dictionary, which the images and the structure tree need.
- **`as_of`** (`DateTimeSpecials`): the render's "current" instant, resolving the date/time formula specials
  `CurrentDate` (and its alias `Today`), `CurrentDateTime`, and `CurrentTime` — and, through the same context, the
  date/time **special fields** `PrintDate` / `PrintTime` / `DataDate` / `DataTime`, so a placed `PrintDate` and a
  `CurrentDate` formula beside it can never disagree. Captured once so the whole render is deterministic — the record
  pipeline (selection/grouping formulas) and every layout context (including crosstab pivots and subreports) share one
  fixed value. `None` (the default) captures the system clock at render start via
  `default_as_of`; set it explicitly for a reproducible render (frozen baselines). The render core reads no clock — the
  instant is captured at the facade/CLI entry, and a WASM build falls back to the Unix epoch unless the host supplies
  `as_of`.

The report file's own timestamps follow the same timezone policy. `ModificationDate` / `ModificationTime` read the
summary set's last-save `FILETIME` and `FileCreationDate` its creation `FILETIME`, each split into a calendar date and
time of day **in UTC** — the zone `as_of` already uses, so one report cannot print `PrintDate` in one zone and
`ModificationDate` in another, and a rendered date never becomes a property of the machine that rendered it (a local
conversion would need a timezone database the WASM-safe render core cannot link). The engine reports these in the host's
local time, so ours can differ from it by the host's UTC offset. A zero `FILETIME` means "never set" and renders blank.
`FilePath` and `GroupNumber` render blank: the first is a property of where the file sits rather than of its bytes, the
second is layout state rather than a stored fact.

---

← [The pipeline](01-pipeline.md) · [Index](README.md) · **Next:** [The Page IR (`rpt-pages`)](03-page-ir.md) →
