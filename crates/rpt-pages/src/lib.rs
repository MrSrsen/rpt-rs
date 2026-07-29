//! # Page IR — the formatted-report representation
//!
//! A [`Page`] is a list of absolutely-positioned drawing primitives ([`DrawOp`]) in twips, the
//! output of the layout engine and the input to every output backend. It is the contract every
//! backend and the WASM split diff on, and it mirrors the native engine's positioned page
//! representation, the same shape its own export filters consume.
//!
//! Two design commitments baked in from the start:
//! - **Object identity travels with every op** ([`ObjectRef`]) — a draw-op knows which report
//!   object produced it, so hit-testing / drill-down and attribute-level parity diffing are a
//!   rectangle+identity lookup rather than an inference from geometry.
//! - **A page is a checkpoint, not an artifact** ([`PageCheckpoint`]) — a page is defined by where
//!   it begins plus a snapshot of print-time state, so any page is independently re-formattable
//!   (random access, drill-down, re-export).
//!
//! Geometry reuses [`rpt_model`]'s [`Twips`]/[`Rect`]/[`Color`] — one source of truth with the
//! decoded model. Everything is `serde`-serializable; [`Page::to_normalized_json`] is the exact
//! surface the render parity tooling consumes.
//!
//! ## Stability policy — additive-stable
//!
//! This is a `serde` wire format frozen against golden fixtures, so changes are governed:
//! - **Additive changes are allowed**: a new field carrying `#[serde(default)]` (so older
//!   serialized pages still deserialize) and a new [`DrawOp`] variant are non-breaking.
//! - **Renames and removals are breaking** and must be deliberate — they invalidate every stored
//!   golden and every consumer, so they are gated (bump the format, re-bless the fixtures) rather
//!   than made in passing.
//!
//! [`PrintState`] and [`PageCheckpoint`] are **excluded** from this guarantee: their field shapes
//! are provisional stubs (see [`PrintState::variables`]) and may change incompatibly.

use rpt_model::{AreaSectionKind, Color, Rect, Twips};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

mod backend;
pub use backend::PageBackend;

mod text;
pub use text::{greedy_wrap, ApproxLayout, TextLayout, TWIPS_PER_PT};

/// `skip_serializing_if` predicate for a twip field whose neutral value is zero, so a field that is
/// neutral on almost every op costs nothing in a serialized page.
fn twips_is_zero(t: &Twips) -> bool {
    t.0 == 0
}

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

/// A stroked edge/border: color, thickness in twips, style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Stroke {
    /// The line color.
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
/// gradient/hatch fills in addition to a solid color. [`Fill::Solid`] is rendered identically by
/// every backend, while gradient/hatch are best-effort per backend (a backend that can't express
/// one falls back to a representative solid color).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Fill {
    /// A single flat color.
    Solid(Color),
    /// A linear gradient over color `stops` (`(offset 0.0..=1.0, color)`, in paint order) along a
    /// direction of `angle_deg` degrees.
    LinearGradient {
        /// Color stops as `(offset 0.0..=1.0, color)` in paint order.
        stops: Vec<(f32, Color)>,
        /// Gradient direction in degrees.
        angle_deg: f32,
    },
    /// A two-color hatch: `fg` lines over a `bg` field in the given `pattern`.
    Hatch {
        /// The hatch line (foreground) color.
        fg: Color,
        /// The field (background) color behind the hatch lines.
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
    /// A representative solid color for a backend that can't render this fill: the solid color
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
    /// alignment (a backend that measures the text itself may ignore it).
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
    /// The text color.
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
    /// Extra advance inserted after **every Unicode scalar** of `text`, in twips (SDK
    /// `ParagraphTextElement.CharacterSpacing`; GDI `SetTextCharacterExtra`), including the trailing
    /// one. `0` — the overwhelming default — means natural advances only.
    ///
    /// It is a parameter of the producer's advance model, not a style hint: `metrics.advance` already
    /// includes it and the same adjusted width decided the wrap. A backend that re-shapes the run must
    /// add `character_spacing × (Unicode scalars in the cluster)` after each shaped cluster — per
    /// *scalar*, never per glyph, or a ligature makes the drawn width disagree with the measured one.
    #[serde(default, skip_serializing_if = "twips_is_zero")]
    pub character_spacing: Twips,
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
    /// The line's stroke (color, width, style).
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

/// How a raster is fitted into an [`ImageOp`]'s `bounds`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ImageFit {
    /// Scale the raster to fill the box on both axes, distorting aspect ratio if they differ.
    /// The right default for a raster already sized to its box (chart islands, placeholders).
    #[default]
    Fill,
    /// Scale the raster uniformly to the largest size that fits within the box, preserving its
    /// source pixel aspect ratio, and center it — the surrounding space is left empty (letterbox).
    /// Crystal renders picture and blob-field images this way.
    Contain,
}

/// A placed image (picture object, chart raster, OLE object). `image_id` references bytes held
/// out-of-band (the IR stays cheap to diff and serialize); the backend resolves it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageOp {
    /// The placement box in twips (printable-relative, top-left origin); the raster is fitted into
    /// it per [`Self::fit`].
    pub bounds: Rect,
    /// Key into the document's out-of-band [`assets`](PagedDocument::assets) map for the image bytes.
    pub image_id: String,
    /// How the raster is scaled into [`Self::bounds`]. Defaults to [`ImageFit::Fill`] so existing
    /// Page-IR dumps deserialize unchanged.
    #[serde(default)]
    pub fit: ImageFit,
    /// The report object this image was formatted from, if known.
    pub source: Option<ObjectRef>,
}

/// The resolved bytes an [`ImageOp`] references by `image_id`, held out-of-band from the page IR
/// (which stays cheap to diff/serialize). A backend that can embed images looks the asset up by the
/// op's `image_id`; when there is no asset for an id the
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

/// Dispatch one uniform expression over every [`DrawOp`] variant, binding each variant's payload to
/// the caller-named `$inner`. Centralizes the variant walk that the payload-uniform accessors
/// ([`DrawOp::source`]/[`DrawOp::source_mut`]) share, so adding a variant is one edit here.
macro_rules! for_each_op {
    ($op:expr, $inner:ident => $body:expr) => {
        match $op {
            DrawOp::Text($inner) => $body,
            DrawOp::Rect($inner) => $body,
            DrawOp::Ellipse($inner) => $body,
            DrawOp::Line($inner) => $body,
            DrawOp::Polygon($inner) => $body,
            DrawOp::Image($inner) => $body,
        }
    };
}

/// Dispatch a geometry expression over [`DrawOp`], splitting the box-carrying variants (Text/Rect/
/// Ellipse/Image, each bound to `$bounded`) from the two variants with their own point geometry.
/// The bounded arm's body is shared across all four; Line and Polygon get their own bodies.
macro_rules! for_geometry {
    ($op:expr, $bounded:ident => $bounded_body:expr, $line:ident => $line_body:expr, $poly:ident => $poly_body:expr $(,)?) => {
        match $op {
            DrawOp::Text($bounded) => $bounded_body,
            DrawOp::Rect($bounded) => $bounded_body,
            DrawOp::Ellipse($bounded) => $bounded_body,
            DrawOp::Image($bounded) => $bounded_body,
            DrawOp::Line($line) => $line_body,
            DrawOp::Polygon($poly) => $poly_body,
        }
    };
}

impl DrawOp {
    /// The op's bounding box (a line's box is its endpoints' extent). Used by hit-testing and the
    /// parity matcher's geometry key.
    pub fn bounds(&self) -> Rect {
        for_geometry!(self,
            b => b.bounds,
            l => {
                let (x0, x1) = (l.from.x.0.min(l.to.x.0), l.from.x.0.max(l.to.x.0));
                let (y0, y1) = (l.from.y.0.min(l.to.y.0), l.from.y.0.max(l.to.y.0));
                Rect {
                    left: Twips(x0),
                    top: Twips(y0),
                    width: Twips(x1 - x0),
                    height: Twips(y1 - y0),
                }
            },
            p => {
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
            },
        )
    }

    /// A copy of this op with every coordinate shifted by `(dx, dy)` twips — used to place subreport
    /// content into its box on the containing page. Geometry only; paint attributes are unchanged.
    pub fn translate(&self, dx: i32, dy: i32) -> DrawOp {
        let mut op = self.clone();
        for_geometry!(&mut op,
            b => b.bounds = b.bounds.translate(dx, dy),
            l => {
                l.from.x.0 += dx;
                l.from.y.0 += dy;
                l.to.x.0 += dx;
                l.to.y.0 += dy;
            },
            p => {
                for pt in &mut p.points {
                    pt.x.0 += dx;
                    pt.y.0 += dy;
                }
            },
        );
        op
    }

    /// Mutable access to the op's originating [`ObjectRef`], if any — used to remap the instance id
    /// when merging a subreport's ops into the containing page.
    pub fn source_mut(&mut self) -> Option<&mut ObjectRef> {
        for_each_op!(self, inner => inner.source.as_mut())
    }

    /// The report object this op came from, if known.
    pub fn source(&self) -> Option<&ObjectRef> {
        for_each_op!(self, inner => inner.source.as_ref())
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
    /// backend (PDF) adds this origin to place content on the paper, while a host that carries
    /// the margin itself (as the RAS web host does) draws content 0-based inside it.
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

    /// Append many draw-ops to the top of the paint order, in iteration order.
    pub fn extend(&mut self, ops: impl IntoIterator<Item = DrawOp>) {
        self.ops.extend(ops);
    }

    /// The topmost draw-op whose bounds contain `p` (last in paint order wins) — the IR-level
    /// analogue of the native engine's object hit-test.
    pub fn hit_test(&self, p: Point) -> Option<&DrawOp> {
        self.ops
            .iter()
            .rev()
            .find(|op| op.bounds().contains(p.x, p.y))
    }

    /// A stable, pretty-printed serialization of this page for diffing two renders. Serialization
    /// order is paint order, and enum tags are explicit (`"op"`), so a diff is a structural
    /// node-level comparison, never a byte comparison.
    ///
    /// # Panics
    ///
    /// Never in practice: the Page IR is a closed data model with no non-string map keys and no custom
    /// `Serialize` impls, so `serde_json` has nothing it can fail on.
    pub fn to_normalized_json(&self) -> String {
        // serde_json cannot fail on this closed, non-Map-keyed data model.
        serde_json::to_string_pretty(self).expect("Page is always serializable")
    }
}

/// A snapshot of print-time state captured at a page boundary that makes a page independently
/// re-formattable. The concrete state (running totals, `WhilePrintingRecords` variables,
/// page-number counters) is a stub map; the type exists so the checkpoint is designed in, not
/// retrofitted.
///
/// Excluded from the crate's additive-stable wire guarantee: the stub string encoding below is
/// provisional.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PrintState {
    /// Serialized snapshot of Global/Shared formula variables and running-total accumulators,
    /// keyed by name. Placeholder representation: values are stored as strings, so the field's
    /// wire shape is not stable.
    pub variables: BTreeMap<String, String>,
}

/// The checkpoint that begins a page: the record position at the top of the page plus the
/// print-time state snapshot taken there. Restoring this and formatting forward reproduces the
/// page exactly (random page access without replaying pages 1..N-1).
///
/// Excluded from the crate's additive-stable wire guarantee via its [`PrintState`] field.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum Severity {
    /// The page was produced but a fidelity gap was hit (object rendered blank, format unwired, …).
    Warning,
    /// A hard failure that produced no/partial output for that element.
    Error,
}

/// What kind of fidelity gap a [`Diagnostic`] reports (so a caller can group/count by cause).
///
/// This is the *single* diagnostic vocabulary for the whole pipeline. The data pipeline reports its
/// own fail-open failures through [`rpt_data::DiagnosticKind`](https://docs.rs/rpt-data) and
/// `rpt-layout` converts them into these kinds on the way out — so a record-selection failure and a
/// font substitution reach the caller in one list, comparable and countable, rather than in two
/// unbridged vocabularies of which only one ever arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// An object kind with no real renderer was drawn as a placeholder box (chart, cross-tab, …).
    UnsupportedObject,
    /// A formula used a builtin/feature the evaluator does not implement (`EvalError::Unsupported`).
    UnsupportedFormula,
    /// A formula errored at runtime (type mismatch, divide-by-zero, unknown name, bad arg).
    FormulaError,
    /// A formula did not **parse**: it was compiled from a partial recovery AST and evaluated anyway,
    /// so its value is meaningless. Distinct from [`FormulaError`](DiagnosticKind::FormulaError)
    /// because the user can fix it by editing the report, and because it is reported once per formula
    /// rather than once per row.
    FormulaParse,
    /// A record-selection formula errored or returned a non-boolean, so the row was **dropped**. The
    /// most consequential fail-open case: enough of these and the report renders empty.
    RecordSelection,
    /// A group-selection formula errored or returned a non-boolean, so the group was **kept**.
    GroupSelection,
    /// A group's grouping condition is an ordinal the pipeline cannot bucket, so rows were grouped by
    /// the field's raw value instead.
    UnsupportedGroupCondition,
    /// A cell would not parse as its column's declared type, so a different type (or null) was
    /// substituted — which silently changes sorting, grouping, and summaries.
    TypeCoercion,
    /// A requested font was not available and a substitute was used.
    FontSubstituted,
    /// Anything else worth surfacing.
    Other,
}

/// Where a [`Diagnostic`] happened, structurally.
///
/// A name alone (`Diagnostic::source`) does not let a user find the problem: the same formula runs on
/// every row, and the same object appears on every page. Every field is optional and — following the
/// convention `rpt_reader::StreamLoc` established for decode errors — **never fabricated**: a site fills in
/// only what it genuinely has in scope.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticLocation {
    /// 1-based page number, when the diagnostic arose while formatting a page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    /// The report area being formatted (e.g. `PageHeader`, `Details`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    /// The section within that area.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// 0-based index of the record that was current, for a per-row failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_index: Option<u64>,
    /// Byte range within the formula text that the failure points at, when the evaluator or parser
    /// reported a span.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<std::ops::Range<usize>>,
}

impl DiagnosticLocation {
    /// Whether nothing at all is known about where this happened.
    pub fn is_empty(&self) -> bool {
        self.page.is_none()
            && self.area.is_none()
            && self.section.is_none()
            && self.record_index.is_none()
            && self.span.is_none()
    }
}

impl fmt::Display for DiagnosticLocation {
    /// Renders only the fields that are known, as `page 2, Details, record 41, bytes 14..17`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut sep = "";
        if let Some(page) = self.page {
            write!(f, "{sep}page {page}")?;
            sep = ", ";
        }
        if let Some(area) = &self.area {
            write!(f, "{sep}{area}")?;
            sep = ", ";
        }
        if let Some(section) = &self.section {
            write!(f, "{sep}{section}")?;
            sep = ", ";
        }
        if let Some(idx) = self.record_index {
            write!(f, "{sep}record {idx}")?;
            sep = ", ";
        }
        if let Some(span) = &self.span {
            write!(f, "{sep}bytes {}..{}", span.start, span.end)?;
        }
        Ok(())
    }
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
    /// Where this happened — page, area/section, record, formula span — as far as the reporting site
    /// knew.
    #[serde(default)]
    pub location: DiagnosticLocation,
}

impl Diagnostic {
    /// A warning-level diagnostic.
    pub fn warn(kind: DiagnosticKind, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            kind,
            message: message.into(),
            source: None,
            location: DiagnosticLocation::default(),
        }
    }

    /// An error-level diagnostic: the element produced no or partial output.
    pub fn error(kind: DiagnosticKind, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            ..Diagnostic::warn(kind, message)
        }
    }

    /// Attach the object/section/formula name this diagnostic is about.
    pub fn with_source(mut self, source: impl Into<String>) -> Diagnostic {
        self.source = Some(source.into());
        self
    }

    /// Attach the structural location this happened at.
    pub fn at(mut self, location: DiagnosticLocation) -> Diagnostic {
        self.location = location;
        self
    }

    /// Note the record that was current, for a per-row failure.
    pub fn at_record(mut self, record_index: u64) -> Diagnostic {
        self.location.record_index = Some(record_index);
        self
    }

    /// Note the byte range within the formula text the failure points at.
    pub fn at_span(mut self, span: std::ops::Range<usize>) -> Diagnostic {
        self.location.span = Some(span);
        self
    }

    /// Note the section being formatted.
    pub fn in_section(mut self, section: impl Into<String>) -> Diagnostic {
        self.location.section = Some(section.into());
        self
    }

    /// Note the report area being formatted. Separate from [`in_section`](Diagnostic::in_section) so a
    /// site that knows only one of the two does not have to invent the other.
    pub fn in_area(mut self, area: impl Into<String>) -> Diagnostic {
        self.location.area = Some(area.into());
        self
    }

    /// Note the 1-based page being formatted.
    pub fn on_page(mut self, page: u32) -> Diagnostic {
        self.location.page = Some(page);
        self
    }

    /// The full one-line rendering: `message (source) [location]`, omitting whatever is unknown. What
    /// a CLI should print.
    pub fn describe(&self) -> String {
        let mut s = self.message.clone();
        if let Some(source) = &self.source {
            s.push_str(&format!(" ({source})"));
        }
        if !self.location.is_empty() {
            s.push_str(&format!(" [{}]", self.location));
        }
        s
    }
}

/// What a section is and where it sits, for a consumer that needs the report's structure and not
/// just its marks (a PDF structure tree deciding artifact-vs-content, an outline, group navigation).
///
/// Held document-level in [`PagedDocument::sections`] and keyed by [`ObjectRef::section`] — the
/// stored section name is a poor classifier (most reports carry at least one `Section1`-style name)
/// but a fine key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionInfo {
    /// Which band this section belongs to.
    pub band: AreaSectionKind,
    /// For a group header/footer, its 0-based nesting level (outermost = 0). `None` for every other
    /// band.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_level: Option<usize>,
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
    /// by its `image_id`. Collected during layout so a backend can embed images without the caller
    /// having to gather them separately — an [`ImageOp`] whose id is absent here draws a placeholder.
    #[serde(default)]
    pub assets: std::collections::BTreeMap<String, ImageAsset>,
    /// What each section named by an [`ObjectRef::section`] on these pages actually is. Empty when
    /// the producer did not classify; a consumer that finds no entry for a section **must** fall back
    /// to treating its content as document content, never as furniture — missing information must
    /// never delete content from the reading order.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sections: BTreeMap<String, SectionInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diagnostic_with_no_location_describes_only_what_it_knows() {
        let d = Diagnostic::warn(DiagnosticKind::Other, "something happened");
        assert!(d.location.is_empty());
        assert_eq!(d.describe(), "something happened");
        assert_eq!(
            Diagnostic::warn(DiagnosticKind::Other, "boom")
                .with_source("Field3")
                .describe(),
            "boom (Field3)"
        );
    }

    #[test]
    fn a_location_renders_every_field_it_has_and_nothing_it_does_not() {
        let d = Diagnostic::error(DiagnosticKind::FormulaError, "type mismatch")
            .with_source("Order Total")
            .on_page(2)
            .in_area("Details")
            .in_section("DetailsA")
            .at_record(41)
            .at_span(14..17);
        assert_eq!(
            d.describe(),
            "type mismatch (Order Total) [page 2, Details, DetailsA, record 41, bytes 14..17]"
        );
        // Partial knowledge renders partially rather than with placeholders.
        let partial = Diagnostic::warn(DiagnosticKind::FormulaError, "boom").at_record(7);
        assert_eq!(partial.describe(), "boom [record 7]");
        assert_eq!(d.severity, Severity::Error);
    }

    /// The location is additive on the wire: a page serialized before it existed must still load.
    #[test]
    fn a_diagnostic_without_a_serialized_location_deserializes() {
        let json = r#"{"severity":"Warning","kind":"Other","message":"m","source":null}"#;
        let d: Diagnostic = serde_json::from_str(json).expect("older payloads must still load");
        assert!(d.location.is_empty());
    }

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
            character_spacing: Twips(0),
            source: Some(ObjectRef::new("Details", ObjectKind::Field).named("name")),
        }));
        page
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
            sections: BTreeMap::new(),
        };
        let json = serde_json::to_string_pretty(&doc).unwrap();
        rpt_test_support::assert_golden(env!("CARGO_MANIFEST_DIR"), "page.json", &json);
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
            character_spacing: Twips(0),
            source: None,
        };
        let back: TextRun = serde_json::from_str(&serde_json::to_string(&with).unwrap()).unwrap();
        assert_eq!(back, with);
    }

    #[test]
    fn textrun_character_spacing_is_absent_at_zero_and_round_trips_otherwise() {
        // Older serialized runs (no `character_spacing` key) still deserialize, defaulting to the
        // natural advances — the additive IR contract holds.
        let json = r#"{"bounds":{"left":0,"top":0,"width":10,"height":10},"text":"x",
            "font":{"family":"Arial","size_pt":10.0,"bold":false,"italic":false,
            "underline":false,"strikethrough":false},"color":{"a":255,"r":0,"g":0,"b":0},
            "align":"Left","rotation":0.0,"metrics":null,"source":null}"#;
        let mut run: TextRun = serde_json::from_str(json).unwrap();
        assert_eq!(run.character_spacing, Twips(0));
        // The neutral value costs zero bytes on the wire.
        assert!(!serde_json::to_string(&run)
            .unwrap()
            .contains("character_spacing"));

        run.character_spacing = Twips(30);
        let back: TextRun = serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        assert_eq!(back, run);
    }

    #[test]
    fn document_sections_are_absent_when_empty_and_round_trip_otherwise() {
        // A document serialized before the dictionary existed still loads, with no classification —
        // which a consumer must read as "treat everything as content".
        let json = r#"{"pages":[],"checkpoints":[]}"#;
        let mut doc: PagedDocument = serde_json::from_str(json).unwrap();
        assert!(doc.sections.is_empty());
        assert!(!serde_json::to_string(&doc).unwrap().contains("sections"));

        doc.sections.insert(
            "PageHeaderA".to_string(),
            SectionInfo {
                band: AreaSectionKind::PageHeader,
                group_level: None,
            },
        );
        doc.sections.insert(
            "GroupHeaderB".to_string(),
            SectionInfo {
                band: AreaSectionKind::GroupHeader,
                group_level: Some(1),
            },
        );
        let text = serde_json::to_string(&doc).unwrap();
        // A band with no nesting level emits no key for it.
        assert!(
            text.contains(r#""PageHeaderA":{"band":"PageHeader"}"#),
            "{text}"
        );
        let back: PagedDocument = serde_json::from_str(&text).unwrap();
        assert_eq!(back, doc);
    }
}
