# Paragraph typography

A text object is a tree of paragraphs, each a run of styled text. The layout engine (`rpt-layout`'s `paginate`/`place`)
honors the per-paragraph formatting instead of flattening the object to one font at single spacing:

- **Per-paragraph font.** Each paragraph is placed in its own run font (a run's stored font override, else the object
  font), so a paragraph's point size drives its own wrap width, line pitch, and ascent — a multi-paragraph object mixing
  sizes draws each paragraph at its size.
- **Line spacing.** `IndentAndSpacingFormat.line_spacing` (decoded from the paragraph format record) gives each line its
  pitch: a
  `Multiple` value scales the font's natural line height (1.0 single, 1.5, 2.0 double), an `Exact` value pins it to a
  twip pitch. Line pitches are summed for the can-grow band height (unequal lines, not count × one).
- **Justified alignment.** The layout marks every wrapped line of a justified paragraph `Justified` except the last
  (which stays `Left`, as typography never stretches a final line). The PDF backend flushes both edges by drawing the
  line word-by-word, each word at its own pen position.
- **Character spacing.** `ParagraphTextElement.CharacterSpacing` widens the run rigidly, so it decides the wrap as well
  as the draw. The PDF backend re-shapes each run to obtain glyphs, and charges the spacing after the last glyph of each
  shaped cluster, weighted by the **Unicode scalars** that cluster covers — so the drawn advance reproduces the one the
  wrap was measured against even when a ligature makes glyph count ≠ scalar count. PDF's `Tc` operator is not usable for
  it: `Tc` spaces every glyph *shown*, which is the per-glyph rule, so the extra rides in the glyph advances instead
  (visible as the run's `TJ` adjustments).
- **Text rotation.** A quarter-turn `TextRotationAngle` (90°/270°) flows the text along the box's tall axis (wrapping
  against the height) and stacks the wrapped lines as columns across the width; each run carries the angle and the
  backend rotates it about the run's top-left (in PDF, as a transform), reading up for 90° and down for 270°.
- **Horizontal tabs.** A tab is an advance, not a glyph — a shaper maps `U+0009` to `.notdef` and paints a box — so
  `rpt-layout` resolves it before the Page IR: a tabbed line is split at its tabs and each segment emitted as its own
  run, positioned at the stop its tab advanced to and aligned `Left`. Stops sit every **0.25 inch (360 twips)** from the
  line's left edge, independent of the font; the alignment anchor is still the line's full tabbed advance, so a
  right-aligned line ending in a tab shifts left by the advance the way the engine's does. No `DrawOp::Text` reaching a
  backend carries a control character, so no backend needs tab logic and none can disagree with layout about the width.

## Resolving `DefaultAlign`

`Alignment::DefaultAlign` is the stored fact for the large majority of objects, and it does not mean "flush left" — the
engine resolves it at paint time from what the object holds. `rpt-layout` reproduces that:

| Object                                                                                    | Default resolves to                                                                                                                                                                  |
|-------------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Field object, numeric value type (`Int8s`/`Int16s`/`Int32s`/`Int32u`/`Number`/`Currency`) | **Right** — the column lines up on its decimal point                                                                                                                                 |
| Field heading                                                                             | whatever the field it heads resolves to: that field's own explicit alignment if it has one, else the same numeric rule — so the heading sits over its column the way the column sits |
| Any object in a right-to-left paragraph                                                   | Right — its base direction                                                                                                                                                           |
| Everything else                                                                           | Left                                                                                                                                                                                 |

The rows are tried in that order, so the reading-order rule is the fallthrough for *every* object kind: a non-numeric
field object in a right-to-left paragraph resolves right, not left.

The value type is the **declared** one (`rpt_model::field_object_value_type`) — the object's bound type for a database
or summary field, else the referenced definition's. A Crystal formula has exactly one return type, fixed when it
compiles, so there is no run-time type to consult and no row is needed to decide alignment. An explicitly stored
alignment always wins; only the default is resolved.

The engine exposes no property for the resolved value — like the effective display format, it exists only inside its
paint code — so this is a render-side derivation and the decoder keeps reporting the stored `DefaultAlign`.

---

← [Format resolution](05-format-resolution.md) · [Index](README.md) ·
**Next:** [Charts and cross-tabs](07-charts-crosstabs.md) →
