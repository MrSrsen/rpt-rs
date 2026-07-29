# Charts and cross-tabs

Both charts and cross-tabs render as **ordinary Page-IR draw-ops** — rects, lines, polygons, ellipses, and text — with
**no rasterization** and no new dependency. The decision is deliberate: emitting native primitives means a chart or grid
renders through any backend without per-backend image embedding, and stays visible in the Page IR.

- **Charts.** The corpus charts are *group charts*: one data point per group, its value the group's summary of the
  charted field — data the layout engine already computes. Dispatch lives in `rpt-layout`'s `chart/` module, one
  renderer per shape keyed off the decoded `ChartGraphType`. Sixteen chart types are named and drawn — bar, line, area,
  pie, doughnut, 3-D riser, 3-D surface, scatter, radar, bubble, stock, numeric-axis, gauge, Gantt, funnel, and
  histogram — plus a verbatim `Other` fallback; the inherently three-dimensional families (3-D riser / surface, and the
  depth-effect area ribbon) take a perspective-riser path. A type without a dedicated renderer falls back to bars, and a
  chart with no plottable group series falls back to a placeholder box, each with a diagnostic. An axis family's value
  axis auto-scales the way the engine does: the tick step is the smallest 1/2/4/5×10ⁿ value that keeps the axis to **at
  most 9 divisions**, and the top tick is the first step multiple at or above the data max (a count of 24 ticks
  `0/4/8/…/24`; a sum of 1,677,019.90 ticks `0/200000/…/1800000`). Tick and data labels are plain decimals — no
  thousands separator and no magnitude abbreviation, since the engine emits neither. A **category** label is drawn
  upright while it fits its own slot; a wider one is drawn **rotated 45°**, and only a rotated axis is then thinned — to
  every *n*-th label, the smallest *n* that keeps adjacent rotated baselines a full line apart — so the band under the
  plot grows to hold the rotated labels' projection and the plot area shortens. No label anywhere is shortened to fit:
  the engine emits no ellipsis and draws over-long text in full, letting the object's box clip it, which is what the
  Page IR already specifies for a text run. Which families carry a **legend** is the family, not the series count: an
  area, line, stock or radar chart draws every mark in one colour, so it legends the *series* it plots as a single boxed
  entry rather than listing categories, and a 3-D family draws no legend at all (its series are its depth axis, already
  labelled beside the series-axis title).
- **Cross-tabs.** A cross-tab pivots the dataset by row × column dimensions with an aggregate measure per cell, drawn as
  a native grid (cell rects + grid lines + text) by `rpt-layout`'s `crosstab` module. The current cut handles one row
  dimension × one column dimension, with every measure drawn stacked in each cell; nested multi-level axes are a
  follow-up.

The chart *definition* decodes further than the renderer draws it: not every decoded style is drawn yet, so a heavily
customized chart can still render with a default for one of them. What decodes is in
[the support matrix](../reader/02-support-matrix.md).

The per-shape geometry (axis frames, label fitting, riser projection and the 3-D room's camera, pivot computation)
lives in the crate rustdoc for
`rpt-layout`'s `chart`/`crosstab` modules — see `cargo doc -p rpt-layout`.

---

← [Paragraph typography](06-typography.md) · [Index](README.md) · **Next:** [The `rpt-render` CLI](08-cli.md) →
