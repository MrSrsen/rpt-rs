# metafile

A dependency-free, cross-platform Rust parser for **Windows metafiles** — **EMF** (Enhanced
Metafile) today, with **WMF** and **EMF+** planned — that decodes them into device-independent
vector primitives.

No Windows, GDI, or native dependency of any kind: `metafile` interprets the metafile's own
coordinate machinery (world transform, window/viewport mapping, object selection, the
`SAVEDC`/`RESTOREDC` graphics-state stack) in pure Rust and hands a consumer resolved shapes, never
pixels. It is WASM-safe and has zero dependencies.

## Backend-agnostic output

Drawing is delivered through the `MetafileSink` visitor trait — implement the callbacks you care
about and map each into your own scene (an SVG/PDF backend, a document renderer, a viewer):

```rust
use metafile::{parse_emf, GraphicsState, MetafileSink, Point};

struct MySink;
impl MetafileSink for MySink {
    fn rectangle(&mut self, bounds: metafile::Rect, state: &GraphicsState<'_>) {
        // draw `bounds` with state.pen / state.brush into your scene
    }
    fn polyline(&mut self, points: &[Point], state: &GraphicsState<'_>) { /* ... */ }
}

let bytes = std::fs::read("picture.emf")?;
let header = parse_emf(&bytes, &mut MySink)?;
// map `header.bounds` onto your target box to place the metafile
# Ok::<(), Box<dyn std::error::Error>>(())
```

Every callback receives geometry already transformed into the metafile's device space plus a
resolved `GraphicsState` (pen, brush, font, text color). The `MetafileHeader` reported first carries
the device-space `bounds` you map onto your target box, so the crate stays device- and
backend-independent.

For consumers that prefer data over callbacks, `Recording` is a ready-made sink that collects every
primitive into a flat `Primitive` list:

```rust
use metafile::{collect_emf, Primitive};

let recording = collect_emf(&std::fs::read("picture.emf")?)?;
for prim in &recording.primitives {
    if let Primitive::Text { text, .. } = prim {
        println!("{text}");
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Robustness

Every field read is bounds-checked: a truncated or garbage stream returns an `Error`, never a panic.
Unknown records are skipped by their length, so an unsupported record never aborts a parse.

## Coverage

EMF today covers: lines and polylines/polygons (32- and 16-bit), `PolyPolygon`/`PolyPolyline`, cubic
Béziers (flattened), rectangles/rounded-rectangles/ellipses, `ExtTextOut` text, pens
(`CreatePen`/`ExtCreatePen`, dash styles, stock pens), brushes (`CreateBrushIndirect`, stock
brushes), fonts (`ExtCreateFontIndirectW`), the world transform
(`SetWorldTransform`/`ModifyWorldTransform`), window/viewport mapping, `SaveDC`/`RestoreDC`, and
raster bitmaps (`StretchDIBits`/`SetDIBitsToDevice`/`BitBlt`/`StretchBlt`/`AlphaBlend`/
`TransparentBlt`, delivered as a self-contained BMP/PNG/JPEG via `MetafileSink::image`).

Embedded **EMF+** (GDI+) content is detected — reported through `MetafileSink::unsupported` — but not
yet interpreted. Clipping, GDI path brackets, native arcs, and gradient fills are skipped by length;
**WMF** and EMF+ rendering are planned.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your
option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this crate shall be dual licensed as above, without any additional terms or conditions.
