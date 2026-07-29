# The Page IR (`rpt-pages`)

A `PagedDocument` is `{ pages, checkpoints, diagnostics, assets, sections }` — `assets` holds the out-of-band image
bytes an `Image` op references by id (see below), and `sections` maps each `ObjectRef::section` name to a
`SectionInfo { band, group_level }`, the report structure a consumer needs to tell page furniture from document content.
The band is `rpt_model::AreaSectionKind`, resolved from the *area* (its kind and `group_level` are authoritative; the
section name is user-renameable). Subreport section names are not namespaced when their ops are merged, so a name the
parent and a subreport disagree about is **dropped** rather than guessed — and a consumer that finds no entry must treat
the content as document content, never as furniture. A `Page` is `{ number, size, origin, ops }` where each
`DrawOp` is a `Rect`, `Ellipse`, `Line`, `Text`, `Polygon`, or `Image` primitive in **twips** (1/1440 inch). The IR is
`serde`-serializable so it can be frozen for tests and diffed independently of any backend.

An `Ellipse` is an axis-aligned ellipse inscribed in its `bounds` (exact round pie centres / bubble markers, which a
`Polygon` can only approximate). A `TextRun` carries a `rotation` (degrees CCW about the run's top-left; `0.0` =
upright, a no-op every backend renders byte-identically to unrotated) and a `character_spacing` — extra advance after
**every Unicode scalar**, the trailing one included. Spacing is a parameter of the producer's advance model, not a style
hint: `metrics.advance` already includes it and the same adjusted width decided the wrap, so a backend that re-shapes
the run must add `character_spacing × the scalars in each cluster`. Charging it per *glyph* instead diverges the moment
a ligature appears, and the visible symptom is a wrong wrap on the page before the spaced one. A
`Rect`/`Ellipse`/`Polygon` `fill` is a `Fill`:
`Solid(Color)`, `LinearGradient { stops, angle_deg }`, or `Hatch { fg, bg, pattern }`. The PDF backend renders all
three:
a gradient as an axial (type 2) shading whose axis spans the op's bounds, a hatch as a tiling pattern of `fg` lines over
a `bg` field. `angle_deg` follows the same convention as a `TextRun`'s `rotation` — degrees counter-clockwise as seen on
the page — so `0°` runs left→right and `90°` bottom→top. A representative solid color survives only for a gradient with
no stops or no area, which has no axis to shade along.

Nothing in the pipeline constructs either variant yet: the layout engine builds only `Solid`, `metafile` flattens a
hatched brush to its foreground color while parsing, and `rpt-model` models neither, so no report reaches the gradient
or hatch path today. The hatch cell is 6 pt square with 0.75 pt lines — the classic GDI `HS_*` brush geometry (an 8×8
device-pixel cell with single-pixel lines) at a 96 dpi logical inch. That mapping is the backend's choice, not a
constant measured from the format's own behaviour, and wants confirming before it counts as settled.

A raster picture object becomes an `Image` op referencing a `PagedDocument` asset (a browser-renderable
BMP/PNG/JPEG/GIF)
that a backend inlines, each keyed by a content hash so identical bytes — a repeated logo, duplicate thumbnails — cost a
single embed shared across every placement: the **PDF** backend embeds each once as an image XObject (PNG/JPEG/GIF via
krilla, BMP decoded in-crate to RGBA). A backend that can't inline/decode the format, or an op with no matching asset,
draws a placeholder. A **database blob field** ({image} column) resolves its per-row bytes the same way — each row gets
a distinct asset.

An `Image` op carries a `fit: ImageFit` (`Fill` or `Contain`) governing how the raster maps to its box. Pictures and
blob fields use `Contain`: the raster is scaled uniformly to the largest size that fits, preserving its source pixel
aspect ratio, and centered — the surrounding space is left empty (letterbox), matching the native engine rather than
distorting. `Fill` (the default, used for pre-sized chart-island rasters and placeholders) stretches to the box on both
axes. The PDF backend implements `Contain` by computing the centered fitted sub-rect from the decoded image's pixel
dimensions. A binary column is fetched raw (no `::text` cast) and carried through the pipeline as `Value::Bytes`; the
Postgres `\x`
hex-escape `bytea` text form is still accepted and decoded back to the original bytes (saved-data path). An **EMF**
(Enhanced Metafile) picture is a vector command stream, not raster bytes, so it is parsed by the standalone `metafile`
crate and replayed instead: `rpt-layout` implements that crate's `MetafileSink`, turning each resolved shape into a
native draw-op (line / polygon / ellipse / rect / text) scaled into the object's box. A bad or truncated stream falls
back to the placeholder with a diagnostic. WMF and OLE-embedded presentations are still placeholders.

## Coordinate model

Draw-op coordinates are **printable-relative** (0-based: `0,0` is the top-left of the printable area, the margin
removed). Each page carries `origin` — the report's top-left margin. A backend re-applies it **once**, in the way that
backend needs, instead of the margin being baked into every coordinate:

- **PDF** is a physical page, so it adds `origin` to every coordinate (a `cm` transform).
- A host that carries the margin itself (as the engine's RAS web host does) instead draws the content 0-based inside a
  container of its own.

This keeps the whole coordinate model in one place; there is no `±margin` scattered across position sites.

## Diagnostics

Rendering collects into `PagedDocument.diagnostics` everything it worked around — the deep issues that would otherwise
never reach the caller. Each `Diagnostic` carries a severity, a kind, a message, the object/formula it is about, and a
`DiagnosticLocation`.

Two families arrive there, in one vocabulary:

- **Layout/render fidelity.** An object that falls back to a placeholder box (a chart with no plottable group series, a
  WMF / OLE-embedded picture), an unimplemented formula builtin, a runtime formula error, a substituted font.
- **Data-pipeline fail-open.** The record pipeline is deliberately fail-open — a record-selection formula that errors
  **drops the row**, a `{@formula}` that errors resolves to `Null`, a group-selection failure **keeps** the group, an
  unsupported group condition falls back to raw-value grouping, and a cell that will not parse as its declared type is
  coerced. That behaviour is right (one broken formula must not abort a render) but it is silent, and enough dropped
  rows renders an empty report that reports success. `rpt-data` reports each occurrence to a `DiagnosticSink`;
  `render_with` attaches one, and `rpt-layout` does the same for every subreport dataset.

The two sides are bridged in `rpt-layout`'s `diagnostics` module — the only crate depending on both — so `rpt-data`
keeps no `rpt-pages` dependency and stays WASM-safe. The conversion loses nothing, and **adds** the severity by an
explicit rule: a fail-open that *discards data* is an `Error`, one that keeps the data but formats or groups it
differently is a `Warning`.

`DiagnosticLocation { page, area, section, record_index, span }` is all-optional and **never fabricated** — a site fills
in only what it genuinely has, the same convention `rpt_reader::StreamLoc` uses for decode errors. `span` is the byte
range within the formula text (the evaluator's `eval_spanned` supplies it); `record_index` is what distinguishes one bad
row from a formula that fails on every row. `Diagnostic::describe()` renders the one-line form a CLI prints.

The CLI surfaces all of it into its warning summary, printing errors through a channel `-q` cannot suppress and
collapsing identical repeats (`… [record 0] — and 606 more like it`) so a per-row failure cannot bury the summary that
explains it.

---

← [Driving a render (library API)](02-api.md) · [Index](README.md) ·
**Next:** [Section-break & pagination controls](04-pagination.md) →
