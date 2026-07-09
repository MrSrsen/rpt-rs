//! # Page IR — the formatted-report representation
//!
//! A [`Page`] is a list of absolutely-positioned drawing primitives ([`DrawOp`]) in twips, the
//! output of the layout engine and the input to every backend (SVG/PDF/raster/HTML). It is the
//! project's frozen contract: backends, the WASM split, and the render parity harness all diff on
//! it. It mirrors the native engine's positioned page representation, the same shape its own
//! export filters consume.
//!
//! Two design commitments baked in from the start:
//! - **Object identity travels with every op** ([`ObjectRef`]) — a draw-op knows which report
//!   object produced it, so hit-testing / drill-down and attribute-level parity diffing are a
//!   rectangle+identity lookup, not reverse-engineering.
//! - **A page is a checkpoint, not an artifact** ([`PageCheckpoint`]) — a page is defined by where
//!   it begins plus a snapshot of print-time state, so any page is independently re-formattable
//!   (random access, drill-down, re-export).
//!
//! Geometry reuses [`rpt_model`]'s [`Twips`]/[`Rect`]/[`Color`] — one source of truth with the
//! decoded model. Everything is `serde`-serializable; [`Page::to_normalized_json`] is the exact
//! surface the render parity tooling consumes.
//!
//! > **Status: unfrozen scaffold.** The shape is deliberately provisional and may still gain or
//! > rename fields as the native page-export format is understood further.

use rpt_model::{Color, Rect, Twips};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod backend;
pub use backend::PageBackend;

/// A point in twips (page-absolute).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
pub struct Point {
    /// Horizontal coordinate in twips (page-absolute, increasing rightward).
    pub x: Twips,
    /// Vertical coordinate in twips (page-absolute, increasing downward).
    pub y: Twips,
}

impl Point {
    /// A point at `(x, y)` twips.
    pub fn new(x: i32, y: i32) -> Point {
        Point {
            x: Twips(x),
            y: Twips(y),
        }
    }
}

/// The report-object kind that produced a draw-op (mirrors the SDK object taxonomy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObjectKind {
    /// A static text object (literal caption / label).
    Text,
    /// A database, formula, parameter, or summary field object.
    Field,
    /// A line object.
    Line,
    /// A box / rectangle object.
    Box,
    /// A picture / OLE image object.
    Image,
    /// A chart object.
    Chart,
    /// A cross-tab object.
    CrossTab,
    /// A subreport object.
    Subreport,
    /// Section background / decoration not owned by a named object.
    Section,
    /// Any object kind not distinguished above.
    Other,
}

/// Back-reference from a draw-op to the report object (and section) it was formatted from — the
/// key for hit-testing, drill-down, and attribute-level parity diffing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectRef {
    /// The section the object was placed in (e.g. `Details`, `PageHeaderA`).
    pub section: String,
    /// The report object's name, when it has one (`Text2`, `some_field`, …).
    pub object_name: Option<String>,
    /// The report-object taxonomy this op came from.
    pub kind: ObjectKind,
    /// A per-placement id: every draw-op the layout engine emits for one placed object instance
    /// (its wrapped text lines *and* its border/fill box) shares this id, so a consumer groups a
    /// wrapped value and links its adornment by id rather than by geometry heuristics. Monotonic
    /// within a report (subreport ids are remapped on merge). `None` for producers that don't assign
    /// one (charts, EMF, synthetic ops) and for older serialized pages — an additive contract change.
    #[serde(default)]
    pub instance: Option<u32>,
}

impl ObjectRef {
    /// A ref to an unnamed object of `kind` in `section` (name and instance unset).
    pub fn new(section: impl Into<String>, kind: ObjectKind) -> ObjectRef {
        ObjectRef {
            section: section.into(),
            object_name: None,
            kind,
            instance: None,
        }
    }
    /// Set this ref's [`object_name`](ObjectRef::object_name).
    pub fn named(mut self, name: impl Into<String>) -> ObjectRef {
        self.object_name = Some(name.into());
        self
    }
    /// Stamp this ref's per-placement [`instance`](ObjectRef::instance) id.
    pub fn with_instance(mut self, instance: u32) -> ObjectRef {
        self.instance = Some(instance);
        self
    }
}

/// Horizontal alignment of a text run within its bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
pub enum TextAlign {
    /// Text is flush against the left edge of its bounds (the default).
    #[default]
    Left,
    /// Text is centred within its bounds.
    Center,
    /// Text is flush against the right edge of its bounds.
    Right,
    /// Text is stretched to fill the full width of its bounds.
    Justified,
}

/// A realized font for a text run. `size_pt` is the point size (the layout engine has already
/// resolved conditional formatting); the family is the resolved face name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontSpec {
    /// The resolved font face/family name (e.g. `Arial`).
    pub family: String,
    /// The point size (already resolved from any conditional formatting).
    pub size_pt: f32,
    /// Whether the run is bold.
    pub bold: bool,
    /// Whether the run is italic.
    pub italic: bool,
    /// Whether the run is underlined.
    pub underline: bool,
    /// Whether the run is struck through.
    pub strikethrough: bool,
}

impl Default for FontSpec {
    fn default() -> FontSpec {
        FontSpec {
            family: "Arial".to_string(),
            size_pt: 10.0,
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
        }
    }
}

/// Line/border style (mirrors the SDK `LineStyle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
pub enum LineStyle {
    /// A single solid line (the default).
    #[default]
    Single,
    /// Two parallel solid lines.
    Double,
    /// A dashed line.
    Dashed,
    /// A dotted line.
    Dotted,
}

/// A stroked edge/border: colour, thickness in twips, style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Stroke {
    /// The line colour.
    pub color: Color,
    /// The line thickness in twips.
    pub width: Twips,
    /// The line/dash style.
    pub style: LineStyle,
}

/// A hatch/cross-hatch line pattern for a [`Fill::Hatch`] (mirrors the GDI+ `HatchStyle` subset the
/// native engine uses for pattern-filled boxes and chart series).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HatchPattern {
    /// Horizontal lines.
    Horizontal,
    /// Vertical lines.
    Vertical,
    /// Diagonal lines running bottom-left to top-right.
    ForwardDiagonal,
    /// Diagonal lines running top-left to bottom-right.
    BackwardDiagonal,
    /// Crossed horizontal and vertical lines.
    Cross,
    /// Crossed diagonal lines.
    DiagonalCross,
}

/// How a region is filled: box objects, section backgrounds, and chart geometry can carry
/// gradient/hatch fills in addition to a solid colour. [`Fill::Solid`] is rendered identically by
/// every backend, while gradient/hatch are best-effort per backend (a backend that can't express
/// one falls back to a representative solid colour).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Fill {
    /// A single flat colour (the pre-widening behaviour).
    Solid(Color),
    /// A linear gradient over colour `stops` (`(offset 0.0..=1.0, colour)`, in paint order) along a
    /// direction of `angle_deg` degrees.
    LinearGradient {
        /// Colour stops as `(offset 0.0..=1.0, colour)` in paint order.
        stops: Vec<(f32, Color)>,
        /// Gradient direction in degrees.
        angle_deg: f32,
    },
    /// A two-colour hatch: `fg` lines over a `bg` field in the given `pattern`.
    Hatch {
        /// The hatch line (foreground) colour.
        fg: Color,
        /// The field (background) colour behind the hatch lines.
        bg: Color,
        /// The hatch line pattern.
        pattern: HatchPattern,
    },
}

impl From<Color> for Fill {
    fn from(color: Color) -> Fill {
        Fill::Solid(color)
    }
}

impl Fill {
    /// A representative solid colour for a backend that can't render this fill: the solid colour
    /// itself, a gradient's midpoint stop (its first stop if it has no stops), or a hatch's
    /// foreground. Backends use this for their gradient/hatch fallback.
    pub fn representative_color(&self) -> Color {
        match self {
            Fill::Solid(c) => *c,
            Fill::LinearGradient { stops, .. } => {
                if stops.is_empty() {
                    Color::default()
                } else {
                    stops[stops.len() / 2].1
                }
            }
            Fill::Hatch { fg, .. } => *fg,
        }
    }
}

/// Resolved text metrics for a [`TextRun`], measured by the layout engine's injected `TextLayout` so
/// backends place text from stored values instead of re-estimating them. All in twips. The baseline
/// sits `ascent` below the run's top edge, consecutive lines advance by `line_height`, and `advance`
/// is the shaped run width used as the alignment anchor for centre/right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextMetrics {
    /// Shaped advance width of the run's text — the horizontal extent used to anchor centre/right
    /// alignment (a backend that measures exactly, like SVG's `text-anchor`, may ignore it).
    pub advance: Twips,
    /// Baseline offset below the run's top edge (the max ascent over the run's font).
    pub ascent: Twips,
    /// Line pitch: the vertical advance from one wrapped line's top to the next.
    pub line_height: Twips,
}

/// A shaped, positioned run of text (already laid out on one line by the layout engine — wrapping
/// produces multiple runs). This is the leaf the native engine draws via `ExtTextOutW`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextRun {
    /// The run's layout box in twips (printable-relative); text is aligned and clipped within it.
    pub bounds: Rect,
    /// The shaped run's text (one line).
    pub text: String,
    /// The resolved font for the run.
    pub font: FontSpec,
    /// The text colour.
    pub color: Color,
    /// Horizontal alignment of the text within `bounds`.
    pub align: TextAlign,
    /// Rotation in degrees counter-clockwise about the run's origin (the top-left of `bounds`).
    /// `0.0` (the default) draws upright; backends apply a rotation transform for a non-zero angle
    /// (rotated axis titles/labels and stored `TextRotationAngle` fields). The layout metrics are
    /// unaffected — the producer positions the run's box; only the paint step rotates.
    #[serde(default)]
    pub rotation: f32,
    /// Resolved advance/ascent/line-height in twips, when the producer measured them (the layout
    /// engine does; chart/EMF/synthetic producers pass `None`). `None` means the backend falls back
    /// to its own point-size heuristic — older serialized pages (no `metrics` key) deserialize to
    /// `None`, so this is an additive contract change.
    #[serde(default)]
    pub metrics: Option<TextMetrics>,
    /// The report object this run was formatted from, if known.
    pub source: Option<ObjectRef>,
}

/// A filled and/or stroked rectangle: a box object, a section background, or a field shading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RectOp {
    /// The rectangle in twips (printable-relative, top-left origin).
    pub bounds: Rect,
    /// The interior fill, or `None` for an unfilled (outline-only) rectangle.
    pub fill: Option<Fill>,
    /// The border stroke, or `None` for no border.
    pub stroke: Option<Stroke>,
    /// Corner radius in twips (rounded box); 0 = square corners.
    pub corner_radius: Twips,
    /// The report object this rectangle was formatted from, if known.
    pub source: Option<ObjectRef>,
}

/// An axis-aligned ellipse inscribed in `bounds` (a circle when `bounds` is square). Exact round
/// geometry — pie centres, bubble/scatter circles, round markers — that [`PolygonOp`] can only
/// approximate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EllipseOp {
    /// The bounding box the ellipse is inscribed in, in twips (printable-relative, top-left origin).
    pub bounds: Rect,
    /// The interior fill, or `None` for an unfilled ellipse.
    pub fill: Option<Fill>,
    /// The outline stroke, or `None` for no outline.
    pub stroke: Option<Stroke>,
    /// The report object this ellipse was formatted from, if known.
    pub source: Option<ObjectRef>,
}

/// A straight line (line object, or a box edge the layout engine chose to emit separately).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineOp {
    /// The line's start point in twips (printable-relative).
    pub from: Point,
    /// The line's end point in twips (printable-relative).
    pub to: Point,
    /// The line's stroke (colour, width, style).
    pub stroke: Stroke,
    /// The report object this line was formatted from, if known.
    pub source: Option<ObjectRef>,
}

/// A filled and/or stroked polygon from a twips point list, in draw order (implicitly closed). Its
/// edges may be non-axis-aligned, unlike [`RectOp`]/[`LineOp`] — used for chart geometry that boxes
/// can't express (pie/doughnut slices, filled area series, radar polygons); arcs are tessellated to
/// points by the producer so every backend only needs straight segments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolygonOp {
    /// The vertices in twips (printable-relative), in draw order.
    pub points: Vec<Point>,
    /// `true` = a closed region (the last point joins the first) — a filled pie/area/radar shape;
    /// `false` = an open polyline (e.g. a line-chart series drawn as one joined path).
    pub closed: bool,
    /// The interior fill (for a closed polygon), or `None` for an unfilled shape.
    pub fill: Option<Fill>,
    /// The edge stroke, or `None` for no outline.
    pub stroke: Option<Stroke>,
    /// The report object this polygon was formatted from, if known.
    pub source: Option<ObjectRef>,
}

/// A placed image (picture object, chart raster, OLE object). `image_id` references bytes held
/// out-of-band (the IR stays cheap to diff and serialize); the backend resolves it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageOp {
    /// The placement box in twips (printable-relative, top-left origin); the image is drawn to fill it.
    pub bounds: Rect,
    /// Key into the document's out-of-band [`assets`](PagedDocument::assets) map for the image bytes.
    pub image_id: String,
    /// The report object this image was formatted from, if known.
    pub source: Option<ObjectRef>,
}

/// The resolved bytes an [`ImageOp`] references by `image_id`, held out-of-band from the page IR
/// (which stays cheap to diff/serialize). A backend that can embed images (e.g. the HTML backend as
/// a `data:` URI) looks the asset up by the op's `image_id`; when there is no asset for an id the
/// backend draws a placeholder instead. `media_type` is the image MIME (e.g. `image/png`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageAsset {
    /// The image MIME type (e.g. `image/png`).
    pub media_type: String,
    /// The raw encoded image bytes.
    pub bytes: Vec<u8>,
}

/// One positioned drawing primitive on a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum DrawOp {
    /// A shaped, positioned run of text.
    Text(TextRun),
    /// A filled and/or stroked rectangle.
    Rect(RectOp),
    /// A filled and/or stroked ellipse.
    Ellipse(EllipseOp),
    /// A straight stroked line.
    Line(LineOp),
    /// A filled and/or stroked polygon / polyline.
    Polygon(PolygonOp),
    /// A placed image.
    Image(ImageOp),
}

impl DrawOp {
    /// The op's bounding box (a line's box is its endpoints' extent). Used by hit-testing and the
    /// parity matcher's geometry key.
    pub fn bounds(&self) -> Rect {
        match self {
            DrawOp::Text(t) => t.bounds,
            DrawOp::Rect(r) => r.bounds,
            DrawOp::Ellipse(e) => e.bounds,
            DrawOp::Image(i) => i.bounds,
            DrawOp::Line(l) => {
                let (x0, x1) = (l.from.x.0.min(l.to.x.0), l.from.x.0.max(l.to.x.0));
                let (y0, y1) = (l.from.y.0.min(l.to.y.0), l.from.y.0.max(l.to.y.0));
                Rect {
                    left: Twips(x0),
                    top: Twips(y0),
                    width: Twips(x1 - x0),
                    height: Twips(y1 - y0),
                }
            }
            DrawOp::Polygon(p) => {
                let xs = p.points.iter().map(|pt| pt.x.0);
                let ys = p.points.iter().map(|pt| pt.y.0);
                let x0 = xs.clone().min().unwrap_or(0);
                let x1 = xs.max().unwrap_or(0);
                let y0 = ys.clone().min().unwrap_or(0);
                let y1 = ys.max().unwrap_or(0);
                Rect {
                    left: Twips(x0),
                    top: Twips(y0),
                    width: Twips(x1 - x0),
                    height: Twips(y1 - y0),
                }
            }
        }
    }

    /// A copy of this op with every coordinate shifted by `(dx, dy)` twips — used to place subreport
    /// content into its box on the containing page. Geometry only; paint attributes are unchanged.
    pub fn translate(&self, dx: i32, dy: i32) -> DrawOp {
        let mut op = self.clone();
        match &mut op {
            DrawOp::Text(t) => t.bounds = t.bounds.translate(dx, dy),
            DrawOp::Rect(r) => r.bounds = r.bounds.translate(dx, dy),
            DrawOp::Ellipse(e) => e.bounds = e.bounds.translate(dx, dy),
            DrawOp::Image(i) => i.bounds = i.bounds.translate(dx, dy),
            DrawOp::Line(l) => {
                l.from.x.0 += dx;
                l.from.y.0 += dy;
                l.to.x.0 += dx;
                l.to.y.0 += dy;
            }
            DrawOp::Polygon(p) => {
                for pt in &mut p.points {
                    pt.x.0 += dx;
                    pt.y.0 += dy;
                }
            }
        }
        op
    }

    /// Mutable access to the op's originating [`ObjectRef`], if any — used to remap the instance id
    /// when merging a subreport's ops into the containing page.
    pub fn source_mut(&mut self) -> Option<&mut ObjectRef> {
        match self {
            DrawOp::Text(t) => t.source.as_mut(),
            DrawOp::Rect(r) => r.source.as_mut(),
            DrawOp::Ellipse(e) => e.source.as_mut(),
            DrawOp::Line(l) => l.source.as_mut(),
            DrawOp::Polygon(p) => p.source.as_mut(),
            DrawOp::Image(i) => i.source.as_mut(),
        }
    }

    /// The report object this op came from, if known.
    pub fn source(&self) -> Option<&ObjectRef> {
        match self {
            DrawOp::Text(t) => t.source.as_ref(),
            DrawOp::Rect(r) => r.source.as_ref(),
            DrawOp::Ellipse(e) => e.source.as_ref(),
            DrawOp::Line(l) => l.source.as_ref(),
            DrawOp::Polygon(p) => p.source.as_ref(),
            DrawOp::Image(i) => i.source.as_ref(),
        }
    }
}

/// Page dimensions in twips (the paper box the layout engine filled).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
pub struct PageSize {
    /// The paper width in twips.
    pub width: Twips,
    /// The paper height in twips.
    pub height: Twips,
}

/// A formatted page: its number, size, and the draw-ops in paint order.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Page {
    /// The 1-based page number in the document.
    pub number: u32,
    /// The paper dimensions in twips.
    pub size: PageSize,
    /// The printable-area origin (the report's top-left margin) in twips. Draw-op coordinates are
    /// **printable-relative** (0,0 = top-left of the printable area, margin removed); a physical
    /// backend (SVG/PDF/raster) adds this origin to place content on the paper, while the RAS/HTML
    /// host draws content 0-based inside a container that carries the margin as CSS.
    #[serde(default)]
    pub origin: Point,
    /// The page's draw-ops in paint order (earlier ops are painted first, under later ones).
    pub ops: Vec<DrawOp>,
}

impl Page {
    /// An empty page of the given number and size, with a zero [`origin`](Page::origin).
    pub fn new(number: u32, size: PageSize) -> Page {
        Page {
            number,
            size,
            origin: Point::default(),
            ops: Vec::new(),
        }
    }

    /// Append a draw-op to the top of the paint order.
    pub fn push(&mut self, op: DrawOp) {
        self.ops.push(op);
    }

    /// The topmost draw-op whose bounds contain `p` (last in paint order wins) — the IR-level
    /// analogue of the native `PEFindObjectOnPage` hit-test.
    pub fn hit_test(&self, p: Point) -> Option<&DrawOp> {
        self.ops
            .iter()
            .rev()
            .find(|op| op.bounds().contains(p.x, p.y))
    }

    /// The normalized draw-op JSON the `rendermatch.py` parity tool consumes: a stable,
    /// pretty-printed serialization of this page. Serialization order is paint order (matching how
    /// the EMF/oracle stream is captured), and enum tags are explicit (`"op"`), so a diff is a
    /// structural node-level comparison, never a byte comparison.
    pub fn to_normalized_json(&self) -> String {
        // serde_json cannot fail on this closed, non-Map-keyed data model.
        serde_json::to_string_pretty(self).expect("Page is always serializable")
    }
}

/// A snapshot of print-time state captured at a page boundary that makes a page independently
/// re-formattable. The concrete state (running totals, `WhilePrintingRecords` variables,
/// page-number counters) is currently a stub map; the type exists so the checkpoint is designed
/// in, not retrofitted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PrintState {
    /// Serialized snapshot of Global/Shared formula variables and running-total accumulators,
    /// keyed by name. Placeholder representation: values are stored as strings.
    pub variables: BTreeMap<String, String>,
}

/// The checkpoint that begins a page: the record position at the top of the page plus the
/// print-time state snapshot taken there. Restoring this and formatting forward reproduces the
/// page exactly (random page access without replaying pages 1..N-1).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PageCheckpoint {
    /// The 1-based number of the page this checkpoint begins.
    pub page_number: u32,
    /// Index into the (grouped) record/instance stream at the top of this page.
    pub record_position: u64,
    /// The print-time state snapshot taken at the top of the page.
    pub state: PrintState,
}

/// How serious a render [`Diagnostic`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    /// The page was produced but a fidelity gap was hit (object rendered blank, format unwired, …).
    Warning,
    /// A hard failure that produced no/partial output for that element.
    Error,
}

/// What kind of fidelity gap a [`Diagnostic`] reports (so a caller can group/count by cause).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticKind {
    /// An object kind with no real renderer yet was drawn as a placeholder box (chart, cross-tab, …).
    UnsupportedObject,
    /// A formula used a builtin/feature the evaluator does not implement (`EvalError::Unsupported`).
    UnsupportedFormula,
    /// A formula errored at runtime (type mismatch, divide-by-zero, unknown name, bad arg).
    FormulaError,
    /// A requested font was not available and a substitute was used.
    FontSubstituted,
    /// Anything else worth surfacing.
    Other,
}

/// A pipeline fidelity warning collected during data/layout/render and returned alongside the
/// [`PagedDocument`], so the caller (the `rpt-render` CLI) can surface *why* the output may differ
/// from the engine — the deep warnings that don't reach the caller otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// How serious the gap is.
    pub severity: Severity,
    /// What kind of fidelity gap this reports.
    pub kind: DiagnosticKind,
    /// A human-readable one-liner.
    pub message: String,
    /// The object/section/formula name this is about, if any.
    pub source: Option<String>,
}

impl Diagnostic {
    /// A warning-level diagnostic.
    pub fn warn(kind: DiagnosticKind, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            kind,
            message: message.into(),
            source: None,
        }
    }

    /// Attach the object/section/formula name this diagnostic is about.
    pub fn with_source(mut self, source: impl Into<String>) -> Diagnostic {
        self.source = Some(source.into());
        self
    }
}

/// A whole formatted document: its pages, the checkpoint that begins each one, and any pipeline
/// fidelity [`Diagnostic`]s collected while producing it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PagedDocument {
    /// The formatted pages in order.
    pub pages: Vec<Page>,
    /// The checkpoint that begins each page (parallel to `pages`), enabling random re-formatting.
    pub checkpoints: Vec<PageCheckpoint>,
    /// Pipeline fidelity warnings collected while producing the document.
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    /// The resolved bytes for every embedded image referenced by an [`ImageOp`] on these pages, keyed
    /// by its `image_id`. Collected during layout so a backend can inline images (e.g. HTML `data:`
    /// URIs) without the caller having to gather them separately — an [`ImageOp`] whose id is absent
    /// here draws a placeholder.
    #[serde(default)]
    pub assets: std::collections::BTreeMap<String, ImageAsset>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_page() -> Page {
        let mut page = Page::new(
            1,
            PageSize {
                width: Twips(12240),  // 8.5in
                height: Twips(15840), // 11in
            },
        );
        page.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(100),
                top: Twips(100),
                width: Twips(2000),
                height: Twips(400),
            },
            fill: Some(Color::WHITE.into()),
            stroke: Some(Stroke {
                color: Color::default(),
                width: Twips(15),
                style: LineStyle::Single,
            }),
            corner_radius: Twips(0),
            source: Some(ObjectRef::new("Details", ObjectKind::Box).named("Box1")),
        }));
        page.push(DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(150),
                top: Twips(150),
                width: Twips(1900),
                height: Twips(300),
            },
            text: "Afghanistan".to_string(),
            font: FontSpec::default(),
            color: Color::default(),
            align: TextAlign::Left,
            rotation: 0.0,
            metrics: None,
            source: Some(ObjectRef::new("Details", ObjectKind::Field).named("name")),
        }));
        page
    }

    /// Compare `actual` against the committed golden at `tests/golden/<name>`, catching any change to
    /// the serialized Page-IR contract. Regenerate: `RPT_BLESS=1 cargo test -p rpt-pages`.
    fn assert_golden(name: &str, actual: &str) {
        let dir = format!("{}/tests/golden", env!("CARGO_MANIFEST_DIR"));
        let path = format!("{dir}/{name}");
        if std::env::var_os("RPT_BLESS").is_some() {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, actual).unwrap();
            return;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!("missing golden {path}; regenerate with RPT_BLESS=1 cargo test -p rpt-pages")
        });
        assert_eq!(
            actual, expected,
            "Page-IR JSON changed for {name}; if the IR change is intentional, regenerate with RPT_BLESS=1"
        );
    }

    #[test]
    fn golden_page_ir_json() {
        // The Page IR is serde-serializable precisely so it can be frozen as a contract; this pins its
        // exact JSON shape so a field rename/reorder or a default change is caught.
        let doc = PagedDocument {
            pages: vec![sample_page()],
            checkpoints: vec![PageCheckpoint {
                page_number: 1,
                record_position: 0,
                state: Default::default(),
            }],
            diagnostics: Vec::new(),
            assets: std::collections::BTreeMap::new(),
        };
        let json = serde_json::to_string_pretty(&doc).unwrap();
        assert_golden("page.json", &json);
    }

    #[test]
    fn draw_op_bounds_and_source() {
        let page = sample_page();
        assert_eq!(page.ops.len(), 2);
        let text = &page.ops[1];
        assert_eq!(text.bounds().left, Twips(150));
        assert_eq!(text.source().unwrap().kind, ObjectKind::Field);
    }

    #[test]
    fn line_bounds_are_the_endpoint_extent() {
        let line = DrawOp::Line(LineOp {
            from: Point::new(100, 500),
            to: Point::new(900, 500),
            stroke: Stroke {
                color: Color::default(),
                width: Twips(15),
                style: LineStyle::Single,
            },
            source: None,
        });
        let b = line.bounds();
        assert_eq!(
            (b.left, b.width, b.height),
            (Twips(100), Twips(800), Twips(0))
        );
    }

    #[test]
    fn hit_test_returns_topmost() {
        let page = sample_page();
        // A point inside both the box and the text → the text (painted last) wins.
        let hit = page.hit_test(Point::new(200, 200)).unwrap();
        assert!(matches!(hit, DrawOp::Text(_)));
        // A point in the box but outside the text.
        let hit = page.hit_test(Point::new(1500, 120)).unwrap();
        assert!(matches!(hit, DrawOp::Rect(_)));
        // Off the page.
        assert!(page.hit_test(Point::new(99999, 99999)).is_none());
    }

    #[test]
    fn normalized_json_roundtrips() {
        let page = sample_page();
        let json = page.to_normalized_json();
        assert!(json.contains("\"op\": \"Text\""));
        assert!(json.contains("Afghanistan"));
        let back: Page = serde_json::from_str(&json).unwrap();
        assert_eq!(back, page);
    }

    #[test]
    fn ellipse_op_bounds_source_and_roundtrip() {
        let e = DrawOp::Ellipse(EllipseOp {
            bounds: Rect {
                left: Twips(100),
                top: Twips(200),
                width: Twips(600),
                height: Twips(400),
            },
            fill: Some(Color::default().into()),
            stroke: None,
            source: Some(ObjectRef::new("Details", ObjectKind::Chart).named("Chart1")),
        });
        assert_eq!(e.bounds().left, Twips(100));
        assert_eq!(e.source().unwrap().kind, ObjectKind::Chart);
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"op\":\"Ellipse\""));
        let back: DrawOp = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn fill_from_color_and_variants_roundtrip() {
        assert_eq!(Fill::from(Color::WHITE), Fill::Solid(Color::WHITE));
        let grad = Fill::LinearGradient {
            stops: vec![(0.0, Color::default()), (1.0, Color::WHITE)],
            angle_deg: 90.0,
        };
        assert_eq!(grad.representative_color(), Color::WHITE);
        let hatch = Fill::Hatch {
            fg: Color::default(),
            bg: Color::WHITE,
            pattern: HatchPattern::ForwardDiagonal,
        };
        assert_eq!(hatch.representative_color(), Color::default());
        for f in [Fill::Solid(Color::WHITE), grad, hatch] {
            let json = serde_json::to_string(&f).unwrap();
            let back: Fill = serde_json::from_str(&json).unwrap();
            assert_eq!(back, f);
        }
    }

    #[test]
    fn textrun_rotation_defaults_when_absent() {
        // Older serialized runs (no `rotation` key) still deserialize, defaulting to upright.
        let json = r#"{"bounds":{"left":0,"top":0,"width":10,"height":10},"text":"x",
            "font":{"family":"Arial","size_pt":10.0,"bold":false,"italic":false,
            "underline":false,"strikethrough":false},"color":{"a":255,"r":0,"g":0,"b":0},
            "align":"Left","source":null}"#;
        let run: TextRun = serde_json::from_str(json).unwrap();
        assert_eq!(run.rotation, 0.0);
    }

    #[test]
    fn textrun_metrics_defaults_when_absent() {
        // Older serialized runs (no `metrics` key) still deserialize, defaulting to `None` so the
        // backend keeps its point-size heuristic — the additive IR contract holds.
        let json = r#"{"bounds":{"left":0,"top":0,"width":10,"height":10},"text":"x",
            "font":{"family":"Arial","size_pt":10.0,"bold":false,"italic":false,
            "underline":false,"strikethrough":false},"color":{"a":255,"r":0,"g":0,"b":0},
            "align":"Left","rotation":0.0,"source":null}"#;
        let run: TextRun = serde_json::from_str(json).unwrap();
        assert_eq!(run.metrics, None);

        // A run that carries metrics round-trips them.
        let with = TextRun {
            bounds: Rect {
                left: Twips(0),
                top: Twips(0),
                width: Twips(10),
                height: Twips(10),
            },
            text: "x".into(),
            font: FontSpec::default(),
            color: Color::default(),
            align: TextAlign::Left,
            rotation: 0.0,
            metrics: Some(TextMetrics {
                advance: Twips(120),
                ascent: Twips(160),
                line_height: Twips(234),
            }),
            source: None,
        };
        let back: TextRun = serde_json::from_str(&serde_json::to_string(&with).unwrap()).unwrap();
        assert_eq!(back, with);
    }
}
