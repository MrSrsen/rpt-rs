//! HTML output backend for the [`rpt_pages`] Page IR, shaped to match the native Crystal engine's
//! **reportrenderer (RAS-direct)** HTML output structurally — not just positionally.
//!
//! Coordinate model: `px = round(twips / 15)` at 96 dpi — see [`rpt_render_util`] for the
//! cross-backend coordinate reference.
//!
//! The document mirrors the reportrenderer emission spec: an
//! XHTML 1.0 Transitional frame, one `<div class="crystalstyle" …overflow:hidden>` container per
//! page, and one positioned `<div>` per report object. Two per-report, deduplicated style tables
//! (`fc<uid>-N` typography classes and `ad<uid>-N` adornment/border classes) live in the `<style>`
//! block; objects reference them by class and carry only position/z-index inline.
//!
//! Object templates:
//! - **Template A** (`<p>`/stacked `<span display:block>`): every TextObject and any field whose
//!   value wrapped to ≥2 visual lines.
//! - **Template B** (nested `<table>`): a single-line FieldObject.
//! - Section background, Box, Line, Image: empty/near-empty positioned divs.
//!
//! Geometry converts twips → CSS px at 96 dpi with `px = round(twips / 15)` (round half away from
//! zero). Positions are the page-relative coordinates carried in the Page IR. The additive
//! `data-object`/`data-section`/`data-kind` attributes are kept on object divs for the parity
//! tooling; they are not part of the native output.

use rpt_model::{Color, Rect, Twips};
use rpt_pages::{
    DrawOp, ImageAsset, LineOp, LineStyle, ObjectKind, Page, RectOp, TextAlign, TextRun,
};
use rpt_render_util::TWIPS_PER_PX;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

/// Page margin the RAS host applies to each page container, in px (= 360 twips = 0.25").
const PAGE_MARGIN_PX: i64 = 24;

/// `round(twips / 15)`, rounding half away from zero (96 dpi), matching the native engine.
fn px(twips: i32) -> i64 {
    let v = twips as f64 / TWIPS_PER_PX;
    if v >= 0.0 {
        (v + 0.5).floor() as i64
    } else {
        -((-v + 0.5).floor() as i64)
    }
}

/// A deduplicated typography class (`fc<uid>-N`).
#[derive(Clone, PartialEq, Eq, Hash)]
struct FontKey {
    /// Point size × 1000 (so `f32` sizes compare/hash exactly).
    size_milli: i32,
    rgb: (u8, u8, u8),
    family: String,
    bold: bool,
    italic: bool,
    underline: bool,
}

impl FontKey {
    fn new(font: &rpt_pages::FontSpec, color: Color) -> FontKey {
        FontKey {
            size_milli: (font.size_pt * 1000.0).round() as i32,
            rgb: (color.r, color.g, color.b),
            family: font.family.clone(),
            bold: font.bold,
            italic: font.italic,
            underline: font.underline,
        }
    }
}

/// The four border sides of an adornment, in the order the engine emits them.
const SIDES: [&str; 4] = ["left", "right", "top", "bottom"];

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum BorderStyle {
    Solid,
    Double,
    Dashed,
    Dotted,
}

impl BorderStyle {
    fn from(style: LineStyle) -> BorderStyle {
        match style {
            LineStyle::Single => BorderStyle::Solid,
            LineStyle::Double => BorderStyle::Double,
            LineStyle::Dashed => BorderStyle::Dashed,
            LineStyle::Dotted => BorderStyle::Dotted,
        }
    }
    fn css(self) -> &'static str {
        match self {
            BorderStyle::Solid => "solid",
            BorderStyle::Double => "double",
            BorderStyle::Dashed => "dashed",
            BorderStyle::Dotted => "dotted",
        }
    }
}

/// A deduplicated adornment class (`ad<uid>-N`): optional fill plus a per-side border.
#[derive(Clone, PartialEq, Eq, Hash)]
struct AdornKey {
    bg: Option<(u8, u8, u8)>,
    border_rgb: (u8, u8, u8),
    /// (style, width-px) for left, right, top, bottom. Width 0 = no visible border on that side.
    sides: [(BorderStyle, u32); 4],
}

impl AdornKey {
    /// A borderless, unfilled object's default adornment (black border color, zero widths).
    fn plain() -> AdornKey {
        AdornKey {
            bg: None,
            border_rgb: (0, 0, 0),
            sides: [(BorderStyle::Solid, 0); 4],
        }
    }

    /// Derive the adornment from a Box rect (an object's border/fill descriptor).
    fn from_rect(r: &RectOp) -> AdornKey {
        // A pure-white fill is the "no fill" default in the native output — omit it. A gradient/hatch
        // box fill collapses to its representative solid colour here (the div model has no pattern).
        let bg = r
            .fill
            .as_ref()
            .map(|f| f.representative_color())
            .and_then(|c| {
                if (c.r, c.g, c.b) == (255, 255, 255) {
                    None
                } else {
                    Some((c.r, c.g, c.b))
                }
            });
        let (border_rgb, side) = match &r.stroke {
            Some(s) => (
                (s.color.r, s.color.g, s.color.b),
                (BorderStyle::from(s.style), px(s.width.0).max(1) as u32),
            ),
            None => ((0, 0, 0), (BorderStyle::Solid, 0)),
        };
        AdornKey {
            bg,
            border_rgb,
            sides: [side; 4],
        }
    }

    fn has_border(&self) -> bool {
        self.sides.iter().any(|(_, w)| *w > 0)
    }
}

/// The two per-report, first-appearance-ordered dedup tables emitted into `<style>`.
#[derive(Default)]
struct Tables {
    fonts: Vec<FontKey>,
    fmap: HashMap<FontKey, usize>,
    adorns: Vec<AdornKey>,
    amap: HashMap<AdornKey, usize>,
}

impl Tables {
    fn font(&mut self, k: FontKey) -> usize {
        if let Some(&i) = self.fmap.get(&k) {
            return i;
        }
        let i = self.fonts.len();
        self.fmap.insert(k.clone(), i);
        self.fonts.push(k);
        i
    }

    fn adorn(&mut self, k: AdornKey) -> usize {
        if let Some(&i) = self.amap.get(&k) {
            return i;
        }
        let i = self.adorns.len();
        self.amap.insert(k.clone(), i);
        self.adorns.push(k);
        i
    }

    /// A deterministic per-report id for the class families — derived from the (order-independent)
    /// contents of both tables, so it is stable across runs but distinct per report. The native
    /// engine uses a random GUID here; the parity tooling normalizes it, and determinism keeps our
    /// tests stable.
    fn uid(&self) -> String {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |bytes: &[u8]| {
            for &b in bytes {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        for f in &self.fonts {
            mix(&f.size_milli.to_le_bytes());
            mix(&[f.rgb.0, f.rgb.1, f.rgb.2]);
            mix(f.family.as_bytes());
            mix(&[f.bold as u8, f.italic as u8, f.underline as u8]);
        }
        for a in &self.adorns {
            mix(&[a.border_rgb.0, a.border_rgb.1, a.border_rgb.2]);
            for (s, w) in &a.sides {
                mix(&[*s as u8]);
                mix(&w.to_le_bytes());
            }
        }
        format!("{h:016x}")
    }
}

/// A page-relative position/size in px.
#[derive(Clone, Copy)]
struct Pos {
    left: i64,
    top: i64,
    width: i64,
    height: i64,
}

impl Pos {
    fn from_rect(b: &rpt_model::Rect) -> Pos {
        // Page IR coordinates are printable-relative (0-based; the margin is carried as the page
        // origin, not baked in). The RAS/HTML host positions content 0-based inside a
        // container that carries the margin as CSS, so the coordinates map straight through here; the
        // container (`PAGE_MARGIN_PX`) supplies the margin, matching the engine's HTML.
        Pos {
            left: px(b.left.0),
            top: px(b.top.0),
            width: px(b.width.0),
            height: px(b.height.0),
        }
    }
}

/// One emitted element on a page (resolved against the style tables in pass 1, serialized in pass 2).
enum Elem {
    /// A section-background div; fill is inline (native: not a class).
    Section {
        id: String,
        top: i64,
        height: i64,
        bg: Option<(u8, u8, u8)>,
    },
    /// Template A: a TextObject or wrapped multi-line field.
    Para {
        id: Option<String>,
        section: String,
        kind: ObjectKind,
        pos: Pos,
        adorn: usize,
        align: TextAlign,
        /// One entry per visual line: (font class index, text).
        lines: Vec<(usize, String)>,
        line_height: i64,
    },
    /// Template B: a single-line field, via a nested table.
    Cell {
        id: Option<String>,
        section: String,
        kind: ObjectKind,
        pos: Pos,
        adorn: usize,
        align: TextAlign,
        font: usize,
        text: String,
    },
    /// A standalone Box object.
    BoxDiv {
        id: Option<String>,
        pos: Pos,
        adorn: usize,
    },
    Line {
        id: Option<String>,
        pos: Pos,
        horizontal: bool,
        thick: i64,
        rgb: (u8, u8, u8),
    },
    Image {
        top: i64,
        left: i64,
        width: i64,
        height: i64,
        image_id: String,
    },
    /// An inline `<svg>` island for a chart (or any non-axis-aligned geometry the div model can't
    /// express): the whole chart's ops rendered as vector SVG, positioned at its bounding box.
    SvgIsland { pos: Pos, svg: String },
}

/// A page reduced to emit-ready elements, plus its container dimensions.
struct PageModel {
    width: i64,
    height: i64,
    elems: Vec<Elem>,
}

/// Identity key for a placed object: its layout-assigned instance id when present (exact — every run
/// and the border box of one placement share it), else a `(section, name)` fallback for producers
/// that assign no instance (charts/EMF) and older pages, which still lean on the geometry heuristic.
#[derive(Clone, PartialEq, Eq, Hash)]
enum ObjKey {
    Instance(u32),
    Named(String, Option<String>),
}

/// The [`ObjKey`] for a draw-op's source: its instance id if it has one, else `(section, name)`.
fn obj_key(src: Option<&rpt_pages::ObjectRef>) -> ObjKey {
    match src.and_then(|s| s.instance) {
        Some(id) => ObjKey::Instance(id),
        None => {
            let (section, name, _) = src_of(src);
            ObjKey::Named(section, name)
        }
    }
}

/// A run of one or more stacked [`TextRun`]s that form one object instance (≥2 = a wrapped value).
struct TextGroup {
    key: ObjKey,
    section: String,
    name: Option<String>,
    kind: ObjectKind,
    align: TextAlign,
    first_op: usize,
    runs: Vec<usize>,
    left_px: i64,
    last_top_px: i64,
}

/// Estimate the twips between the top of one wrapped line and the next for a given font — used to
/// tell stacked lines of one cell (gap ≈ line height) from distinct detail rows (gap ≈ cell height).
fn line_gap_twips(font: &rpt_pages::FontSpec) -> i64 {
    // 1pt = 20 twips; ~1.2 leading.
    (font.size_pt as f64 * 20.0 * 1.2).round() as i64
}

/// The `line-height:Npx` the native engine sets on each visual line's wrapper span: the run's
/// resolved line pitch when the layout engine measured it, else the ~1.17-em point-size heuristic.
fn line_height_px_of(run: &TextRun) -> i64 {
    match &run.metrics {
        Some(m) => px(m.line_height.0),
        None => line_height_px(&run.font),
    }
}

/// The point-size line-height heuristic (~1.17 em) — the fallback when a run carries no resolved
/// metrics.
fn line_height_px(font: &rpt_pages::FontSpec) -> i64 {
    let font_px = font.size_pt as f64 * 96.0 / 72.0;
    (font_px * 1.17).round() as i64
}

/// Render a slice of pages to one self-contained reportrenderer-shaped HTML document. Image ops are
/// drawn as placeholders (no bytes are available); use [`render_pages_with_assets`] to embed images.
pub fn render_pages(pages: &[Page]) -> String {
    render_pages_with_assets(pages, &BTreeMap::new())
}

/// Like [`render_pages`], but embeds each image op whose `image_id` has an entry in `assets` as an
/// inline `data:` URI, so the output stays a single self-contained file (safe to write to a pipe).
/// An image op with no matching asset is drawn as a visible placeholder box.
pub fn render_pages_with_assets(pages: &[Page], assets: &BTreeMap<String, ImageAsset>) -> String {
    let mut tables = Tables::default();
    let models: Vec<PageModel> = pages.iter().map(|p| build_page(p, &mut tables)).collect();
    let uid = tables.uid();

    let mut h = String::new();
    emit_head(&mut h, &tables, &uid);

    // Page containers, concatenated. Container `top` accumulates page-absolutely; page 1 carries a
    // top margin, the last page a bottom margin (matching the native RAS host).
    let mut cum_top: i64 = 0;
    let last = models.len().saturating_sub(1);
    for (i, m) in models.iter().enumerate() {
        let margin = if i == 0 {
            format!("margin-top:{PAGE_MARGIN_PX}px;")
        } else if i == last {
            format!("margin-bottom:{PAGE_MARGIN_PX}px;")
        } else {
            String::new()
        };
        let _ = writeln!(
            h,
            "<div class=\"crystalstyle\" style=\"{margin}margin-left:{m}px;margin-right:{m}px;\
             top:{top}px;left:0px;width:{w}px;height:{ht}px;overflow:hidden;\">",
            m = PAGE_MARGIN_PX,
            top = cum_top,
            w = m.width,
            ht = m.height,
        );
        for e in &m.elems {
            emit_elem(&mut h, e, &uid, m.width, assets);
        }
        h.push_str("</div>\n");
        cum_top += m.height + if i == 0 { PAGE_MARGIN_PX } else { 0 };
    }

    h.push_str("</Div>\n</BODY>\n</HTML>\n");
    h
}

/// Render a single page to a self-contained HTML document.
pub fn render_page(page: &Page) -> String {
    render_pages(std::slice::from_ref(page))
}

/// The HTML backend as a [`PageBackend`](rpt_pages::PageBackend): one self-contained document embedding the document's
/// [`assets`](rpt_pages::PagedDocument::assets), so a caller never threads images separately.
#[derive(Debug, Default, Clone, Copy)]
pub struct HtmlBackend;

/// Knobs for [`HtmlBackend`]. None today (images come from the document's assets); the struct exists
/// so future HTML options are an additive field, not a signature change.
#[derive(Debug, Default, Clone, Copy)]
pub struct HtmlOptions;

impl rpt_pages::PageBackend for HtmlBackend {
    type Output = String;
    type Options = HtmlOptions;

    fn render(&self, doc: &rpt_pages::PagedDocument, _opts: &HtmlOptions) -> String {
        render_pages_with_assets(&doc.pages, &doc.assets)
    }
}

/// Reduce a page to emit-ready elements, interning fonts/adornments into `tables`.
fn build_page(page: &Page, tables: &mut Tables) -> PageModel {
    let ops = &page.ops;

    // Group text runs into object instances (stacked lines = one wrapped instance).
    let mut groups: Vec<TextGroup> = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        let DrawOp::Text(t) = op else { continue };
        if is_chart_op(op) {
            continue; // chart labels live inside the chart's SVG island, not as page text
        }
        let (section, name, kind) = src_of(t.source.as_ref());
        let key = obj_key(t.source.as_ref());
        // Un-bake the page margin (see `Pos::from_rect`): the container's CSS margin re-adds it.
        let left = px(t.bounds.left.0);
        let top = px(t.bounds.top.0);
        // With an instance id, same id = same placed object = merge (exact). Without one, fall back to
        // the geometry heuristic: same name, same left, and within ~2 line-heights below the last line.
        let can_merge = groups.last().is_some_and(|g| match (&g.key, &key) {
            (ObjKey::Instance(a), ObjKey::Instance(b)) => a == b,
            (ObjKey::Named(..), ObjKey::Named(..)) => {
                g.key == key
                    && g.left_px == left
                    && (top - g.last_top_px) <= 2 * px(line_gap_twips(&t.font) as i32)
                    && (top - g.last_top_px) >= 0
            }
            _ => false,
        });
        if can_merge {
            let g = groups.last_mut().unwrap();
            g.runs.push(i);
            g.last_top_px = top;
        } else {
            groups.push(TextGroup {
                key,
                section,
                name,
                kind,
                align: t.align,
                first_op: i,
                runs: vec![i],
                left_px: left,
                last_top_px: top,
            });
        }
    }

    // Every object-key that names a text object — a Box rect sharing this key (same instance, or the
    // same section+name in the fallback) is that object's border/fill adornment, not a standalone box.
    let text_keys: HashSet<ObjKey> = groups.iter().map(|g| g.key.clone()).collect();

    // Adornment box per object key, and the set of op indices those consume.
    let mut adorn_of: HashMap<ObjKey, AdornKey> = HashMap::new();
    let mut consumed_box: HashSet<usize> = HashSet::new();
    for (i, op) in ops.iter().enumerate() {
        let DrawOp::Rect(r) = op else { continue };
        if !matches!(kind_of(r.source.as_ref()), ObjectKind::Box) {
            continue;
        }
        let key = obj_key(r.source.as_ref());
        if text_keys.contains(&key) {
            adorn_of
                .entry(key)
                .or_insert_with(|| AdornKey::from_rect(r));
            consumed_box.insert(i);
        }
    }

    // Map an op index to the group that starts there (so we emit each object once, in first-line
    // order, and skip runs already merged into it).
    let group_first: HashMap<usize, usize> = groups
        .iter()
        .enumerate()
        .map(|(gi, g)| (g.first_op, gi))
        .collect();

    let container_w = container_width(page);
    let mut elems: Vec<Elem> = Vec::new();
    let mut max_bottom: i64 = 0;
    // Chart geometry (and any stray non-axis-aligned op) is collected per chart object, then emitted
    // as one inline <svg> island below.
    let mut chart_ops: BTreeMap<String, Vec<DrawOp>> = BTreeMap::new();

    for (i, op) in ops.iter().enumerate() {
        if is_chart_op(op) {
            let key = name_of(op.source()).unwrap_or_default();
            chart_ops.entry(key).or_default().push(op.clone());
            continue;
        }
        match op {
            DrawOp::Rect(r) => {
                let kind = kind_of(r.source.as_ref());
                let (section, _, _) = src_of(r.source.as_ref());
                let p = Pos::from_rect(&r.bounds);
                max_bottom = max_bottom.max(p.top + p.height);
                match kind {
                    ObjectKind::Section => {
                        let bg = r.fill.as_ref().map(|f| {
                            let c = f.representative_color();
                            (c.r, c.g, c.b)
                        });
                        elems.push(Elem::Section {
                            id: section,
                            top: p.top,
                            height: p.height,
                            bg,
                        });
                    }
                    ObjectKind::Box if !consumed_box.contains(&i) => {
                        let adorn = tables.adorn(AdornKey::from_rect(r));
                        elems.push(Elem::BoxDiv {
                            id: name_of(r.source.as_ref()),
                            pos: p,
                            adorn,
                        });
                    }
                    _ => {}
                }
            }
            DrawOp::Line(l) => {
                let e = build_line(l);
                if let Elem::Line { pos, .. } = &e {
                    max_bottom = max_bottom.max(pos.top + pos.height);
                }
                elems.push(e);
            }
            DrawOp::Image(im) => {
                let p = Pos::from_rect(&im.bounds);
                max_bottom = max_bottom.max(p.top + p.height);
                elems.push(Elem::Image {
                    top: p.top,
                    left: p.left,
                    width: p.width,
                    height: p.height,
                    image_id: im.image_id.clone(),
                });
            }
            DrawOp::Text(_) => {
                let Some(&gi) = group_first.get(&i) else {
                    continue;
                };
                let g = &groups[gi];
                let e = build_text_object(g, ops, &adorn_of, tables);
                if let Some(p) = elem_pos(&e) {
                    max_bottom = max_bottom.max(p.top + p.height);
                }
                elems.push(e);
            }
            // Polygons and ellipses are chart geometry, routed to an SVG island above.
            DrawOp::Polygon(_) | DrawOp::Ellipse(_) => {}
        }
    }

    // Emit each chart as one inline <svg> island at its bounding box, its interior drawn by the SVG
    // backend (the single owner of DrawOp→SVG) in the ops' own absolute-twip coordinates.
    for cops in chart_ops.into_values() {
        let Some(bbox) = ops_bbox(&cops) else {
            continue;
        };
        let pos = Pos::from_rect(&bbox);
        max_bottom = max_bottom.max(pos.top + pos.height);
        elems.push(Elem::SvgIsland {
            pos,
            svg: rpt_render_svg::render_fragment(&cops, bbox),
        });
    }

    let height = if max_bottom > 0 {
        max_bottom
    } else {
        px(page.size.height.0)
    };
    PageModel {
        width: container_w,
        height,
        elems,
    }
}

/// Build the Template-A / Template-B element for one grouped text object.
fn build_text_object(
    g: &TextGroup,
    ops: &[DrawOp],
    adorn_of: &HashMap<ObjKey, AdornKey>,
    tables: &mut Tables,
) -> Elem {
    let runs: Vec<&TextRun> = g
        .runs
        .iter()
        .map(|&i| match &ops[i] {
            DrawOp::Text(t) => t,
            _ => unreachable!("group indices point at text ops"),
        })
        .collect();

    // Union bounds of all lines in container coordinates (positions un-bake the page margin).
    let left = runs.iter().map(|r| px(r.bounds.left.0)).min().unwrap();
    let top = runs.iter().map(|r| px(r.bounds.top.0)).min().unwrap();
    let right = runs
        .iter()
        .map(|r| px(r.bounds.left.0) + px(r.bounds.width.0))
        .max()
        .unwrap();
    let bottom = runs
        .iter()
        .map(|r| px(r.bounds.top.0) + px(r.bounds.height.0))
        .max()
        .unwrap();
    let pos = Pos {
        left,
        top,
        width: right - left,
        height: bottom - top,
    };

    let adorn_key = adorn_of
        .get(&g.key)
        .cloned()
        .unwrap_or_else(AdornKey::plain);
    let adorn = tables.adorn(adorn_key);

    let multiline = runs.len() > 1;
    // TextObjects always use Template A; single-line fields use Template B; wrapped fields → A.
    if matches!(g.kind, ObjectKind::Text) || multiline {
        let lines = runs
            .iter()
            .map(|r| (tables.font(FontKey::new(&r.font, r.color)), r.text.clone()))
            .collect();
        Elem::Para {
            id: g.name.clone(),
            section: g.section.clone(),
            kind: g.kind,
            pos,
            adorn,
            align: g.align,
            lines,
            line_height: line_height_px_of(runs[0]),
        }
    } else {
        let r = runs[0];
        Elem::Cell {
            id: g.name.clone(),
            section: g.section.clone(),
            kind: g.kind,
            pos,
            adorn,
            align: g.align,
            font: tables.font(FontKey::new(&r.font, r.color)),
            text: r.text.clone(),
        }
    }
}

fn build_line(l: &LineOp) -> Elem {
    let (x0, x1) = (l.from.x.0.min(l.to.x.0), l.from.x.0.max(l.to.x.0));
    let (y0, y1) = (l.from.y.0.min(l.to.y.0), l.from.y.0.max(l.to.y.0));
    let horizontal = (x1 - x0) >= (y1 - y0);
    let pos = Pos {
        left: px(x0),
        top: px(y0),
        width: px(x1 - x0).max(1),
        height: px(y1 - y0).max(1),
    };
    Elem::Line {
        id: l.source.as_ref().and_then(|s| s.object_name.clone()),
        pos,
        horizontal,
        thick: px(l.stroke.width.0).max(1),
        rgb: (l.stroke.color.r, l.stroke.color.g, l.stroke.color.b),
    }
}

fn elem_pos(e: &Elem) -> Option<Pos> {
    match e {
        Elem::Para { pos, .. } | Elem::Cell { pos, .. } | Elem::BoxDiv { pos, .. } => Some(*pos),
        Elem::Line { pos, .. } | Elem::SvgIsland { pos, .. } => Some(*pos),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------------------------

fn emit_head(h: &mut String, tables: &Tables, uid: &str) {
    h.push_str(
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//EN\" \
         \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd\"><HTML><style>\n\
         \x20\x20\x20\x20div.crystalstyle div {position:absolute; z-index:25}\n\
         \x20\x20\x20\x20div.crystalstyle a {text-decoration:none}\n\
         \x20\x20\x20\x20div.crystalstyle a img {border-style:none; border-width:0}\n\
         \x20\x20\x20\x20div.crystalstyle div.tbg {background: url(\"js/dhtmllib/images/transp.gif\") repeat-x repeat-y;}\n",
    );
    for (i, f) in tables.fonts.iter().enumerate() {
        let _ = write!(
            h,
            "\t.fc{uid}-{i} {{font-size:{size}pt;color:{color};font-family:\"{fam}\";font-weight:{weight};",
            size = fmt_pt(f.size_milli),
            color = css_rgb(f.rgb),
            fam = f.family,
            weight = if f.bold { "bold" } else { "normal" },
        );
        if f.italic {
            h.push_str("font-style:italic;");
        }
        if f.underline {
            h.push_str("text-decoration:underline !important;");
        }
        h.push_str("}\n");
    }
    for (i, a) in tables.adorns.iter().enumerate() {
        let _ = write!(h, "\t.ad{uid}-{i} {{");
        if let Some(bg) = a.bg {
            let _ = write!(
                h,
                "background-color:{c};layer-background-color:{c};",
                c = css_rgb(bg)
            );
        }
        let _ = write!(h, "border-color:{};", css_rgb(a.border_rgb));
        if a.has_border() {
            h.push_str("border-style:solid;border-width:0px;");
            for (side, (style, w)) in SIDES.iter().zip(a.sides.iter()) {
                let _ = write!(
                    h,
                    "border-{side}-style:{s};border-{side}-width:{w}px;",
                    s = style.css()
                );
            }
        } else {
            for side in SIDES {
                let _ = write!(h, "border-{side}-width:0px;");
            }
        }
        h.push_str("}\n");
    }
    h.push_str(
        "</style>\n<TITLE>Crystal Report Viewer</TITLE>\n\
         <BODY BGCOLOR=\"FFFFFF\" LEFTMARGIN=31 TOPMARGIN=31>\n\
         <Div class=\"crystalstyle\" style=\"position:absolute; top:31px; left:31px; \">\n",
    );
}

fn emit_elem(
    h: &mut String,
    e: &Elem,
    uid: &str,
    container_w: i64,
    assets: &BTreeMap<String, ImageAsset>,
) {
    match e {
        Elem::Section {
            id,
            top,
            height,
            bg,
        } => {
            let _ = write!(
                h,
                "    <div id=\"{id}\" style=\"z-index:3;top:{top}px;left:0px;width:{w}px;height:{height}px;",
                id = escape_attr(id),
                w = container_w,
            );
            if let Some(bg) = bg {
                let _ = write!(
                    h,
                    "background-color:{c};layer-background-color:{c};",
                    c = css_rgb(*bg)
                );
            }
            h.push_str("\">\n\n    </div>\n");
        }
        Elem::Para {
            id,
            section,
            kind,
            pos,
            adorn,
            align,
            lines,
            line_height,
        } => {
            let mut style = pos_style(pos);
            if matches!(align, TextAlign::Right) {
                style.push_str("text-align:right;");
            }
            let _ = writeln!(
                h,
                "    <div{id} class=\"ad{uid}-{adorn}\"{data} style=\"{style}\">",
                id = id_attr(id),
                data = data_attrs(section, id.as_deref(), *kind),
            );
            let _ = write!(
                h,
                "        <p{align} style=\"position:relative;padding-left:1px;margin:0px;white-space:nowrap;\">",
                align = p_align(*align),
            );
            for (font, text) in lines {
                let _ = write!(
                    h,
                    "<span style=\"position:relative;display:block;line-height:{line_height}px;\">\
                     <span class=\"fc{uid}-{font}\">{t}</span></span>",
                    t = escape_html(text),
                );
            }
            h.push_str("</p>\n    </div>\n");
        }
        Elem::Cell {
            id,
            section,
            kind,
            pos,
            adorn,
            align,
            font,
            text,
        } => {
            let mut style = pos_style(pos);
            if matches!(align, TextAlign::Right) {
                style.push_str("text-align:right;");
            }
            let _ = write!(
                h,
                "    <div{id} class=\"ad{uid}-{adorn}\"{data} style=\"{style}\">\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20<table width=\"100%\" border=\"0\" cellpadding=\"0\" cellspacing=\"0\">\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<tr>\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<td><table width=\"100%\" border=\"0\" cellpadding=\"0\" cellspacing=\"0\">\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<tr>\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<td{td_align} nowrap=\"true\"><span class=\"fc{uid}-{font}\">{t}</span></td>\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20</tr>\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20</table></td>\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20</tr>\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20</table>\n    </div>\n",
                id = id_attr(id),
                data = data_attrs(section, id.as_deref(), *kind),
                td_align = td_align(*align),
                t = escape_html(text),
            );
        }
        Elem::BoxDiv { id, pos, adorn } => {
            let _ = writeln!(
                h,
                "    <div{id} class=\"ad{uid}-{adorn}\" style=\"{style}\"></div>",
                id = id_attr(id),
                style = pos_style(pos),
            );
        }
        Elem::Line {
            id,
            pos,
            horizontal,
            thick,
            rgb,
        } => {
            let side = if *horizontal { "top" } else { "left" };
            let _ = writeln!(
                h,
                "    <div{id} style=\"z-index:15;top:{t}px;left:{l}px;\
                 border-color:{c};border-style:solid;border-width:0px;\
                 border-{side}-width:{thick}px;width:{w}px;height:{ht}px;\"></div>",
                id = id_attr(id),
                t = pos.top,
                l = pos.left,
                c = css_rgb(*rgb),
                w = pos.width,
                ht = pos.height,
            );
        }
        Elem::Image {
            top,
            left,
            width,
            height,
            image_id,
        } => {
            match assets.get(image_id) {
                // Bytes available: inline as a data: URI so the document stays self-contained.
                Some(asset) => {
                    let _ = write!(
                        h,
                        "    <div style=\"z-index:10;top:{top}px;left:{left}px;\">\n\
                         \x20\x20\x20\x20\x20\x20\x20\x20<img alt=\"Image\" border=\"0\" src=\"data:{media};base64,{data}\" style=\"width:{width}px;height:{height}px;\" />\n\
                         \x20\x20\x20\x20</div>\n",
                        media = escape_attr(&asset.media_type),
                        data = base64_encode(&asset.bytes),
                    );
                }
                // No bytes for this id (blob field, chart, or picture bytes not decoded): draw a
                // visible placeholder box rather than a broken reference to an unwritten file.
                None => {
                    let _ = write!(
                        h,
                        "    <div style=\"z-index:10;top:{top}px;left:{left}px;\">\n\
                         \x20\x20\x20\x20\x20\x20\x20\x20<div class=\"rpt-image-missing\" title=\"image not embedded\" style=\"width:{width}px;height:{height}px;border:1px dashed #b0b0b0;box-sizing:border-box;\"></div>\n\
                         \x20\x20\x20\x20</div>\n",
                    );
                }
            }
        }
        Elem::SvgIsland { pos, svg } => {
            // The chart as one positioned box; the inline <svg> (width/height 100%) fills it.
            let _ = writeln!(
                h,
                "    <div style=\"z-index:15;{}overflow:hidden;\">{svg}</div>",
                pos_style(pos),
            );
        }
    }
}

fn pos_style(p: &Pos) -> String {
    format!(
        "top:{}px;left:{}px;width:{}px;height:{}px;",
        p.top, p.left, p.width, p.height
    )
}

// ---------------------------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------------------------

/// The page container's content width in px = full page width minus the two page margins.
fn container_width(page: &Page) -> i64 {
    (px(page.size.width.0) - 2 * PAGE_MARGIN_PX).max(0)
}

fn src_of(src: Option<&rpt_pages::ObjectRef>) -> (String, Option<String>, ObjectKind) {
    match src {
        Some(o) => (o.section.clone(), o.object_name.clone(), o.kind),
        None => (String::new(), None, ObjectKind::Other),
    }
}

fn kind_of(src: Option<&rpt_pages::ObjectRef>) -> ObjectKind {
    src.map(|o| o.kind).unwrap_or(ObjectKind::Other)
}

fn name_of(src: Option<&rpt_pages::ObjectRef>) -> Option<String> {
    src.and_then(|o| o.object_name.clone())
}

/// Whether an op belongs to a chart — its geometry is routed into an inline `<svg>` island because
/// the positioned-div model can't express bars/axes/slices/diagonal lines coherently.
fn is_chart_op(op: &DrawOp) -> bool {
    matches!(kind_of(op.source()), ObjectKind::Chart)
        || matches!(op, DrawOp::Polygon(_) | DrawOp::Ellipse(_))
}

/// The bounding box (twips) enclosing every op in `ops`, or `None` if empty.
fn ops_bbox(ops: &[DrawOp]) -> Option<Rect> {
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for op in ops {
        let b = op.bounds();
        x0 = x0.min(b.left.0);
        y0 = y0.min(b.top.0);
        x1 = x1.max(b.left.0 + b.width.0);
        y1 = y1.max(b.top.0 + b.height.0);
    }
    (x1 > x0 && y1 > y0).then_some(Rect {
        left: Twips(x0),
        top: Twips(y0),
        width: Twips(x1 - x0),
        height: Twips(y1 - y0),
    })
}

fn id_attr(id: &Option<String>) -> String {
    match id {
        Some(n) if !n.is_empty() => format!(" id=\"{}\"", escape_attr(n)),
        _ => String::new(),
    }
}

fn data_attrs(section: &str, object: Option<&str>, kind: ObjectKind) -> String {
    format!(
        " data-section=\"{}\" data-object=\"{}\" data-kind=\"{:?}\"",
        escape_attr(section),
        escape_attr(object.unwrap_or("")),
        kind
    )
}

fn p_align(a: TextAlign) -> &'static str {
    match a {
        TextAlign::Center => " align=\"center\"",
        TextAlign::Right => " align=\"right\"",
        _ => "",
    }
}

fn td_align(a: TextAlign) -> &'static str {
    match a {
        TextAlign::Center => " align=\"center\"",
        TextAlign::Right => " align=\"right\"",
        _ => "",
    }
}

fn fmt_pt(size_milli: i32) -> String {
    if size_milli % 1000 == 0 {
        (size_milli / 1000).to_string()
    } else {
        let s = format!("{:.3}", size_milli as f64 / 1000.0);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn css_rgb(c: (u8, u8, u8)) -> String {
    Color {
        a: 255,
        r: c.0,
        g: c.1,
        b: c.2,
    }
    .to_hex()
}

/// Escape text content and turn spaces into `&nbsp;` (the engine pre-measures geometry, so runs
/// never reflow — `white-space:nowrap` plus non-breaking spaces). The `&`/`<`/`>` escaping is the
/// shared XML text escape; the `&nbsp;` step is HTML-backend-specific.
fn escape_html(s: &str) -> String {
    rpt_render_util::escape_xml_text(s).replace(' ', "&nbsp;")
}

use rpt_render_util::escape_xml_attr as escape_attr;

/// Standard-alphabet base64 (RFC 4648) with `=` padding, for embedding image bytes in a `data:`
/// URI. Kept dependency-free — the alphabet and padding are the whole spec.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(n >> 18) as usize & 0x3f] as char);
        out.push(ALPHABET[(n >> 12) as usize & 0x3f] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 0x3f] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 0x3f] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_model::{Rect, Twips};
    use rpt_pages::{FontSpec, ObjectKind, ObjectRef, PageSize, Point, Stroke};

    fn text_run(
        left: i32,
        top: i32,
        w: i32,
        h: i32,
        text: &str,
        kind: ObjectKind,
        name: &str,
    ) -> DrawOp {
        DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(left),
                top: Twips(top),
                width: Twips(w),
                height: Twips(h),
            },
            text: text.into(),
            font: FontSpec::default(),
            color: Color {
                a: 255,
                r: 0,
                g: 0,
                b: 0,
            },
            align: TextAlign::Left,
            rotation: 0.0,
            metrics: None,
            source: Some(ObjectRef::new("DetailSection1", kind).named(name)),
        })
    }

    /// A text run stamped with a placement instance id (as the layout engine emits).
    fn text_run_inst(top: i32, text: &str, name: &str, instance: u32) -> DrawOp {
        let DrawOp::Text(mut t) = text_run(420, top, 660, 240, text, ObjectKind::Field, name)
        else {
            unreachable!()
        };
        t.source = t.source.map(|s| s.with_instance(instance));
        DrawOp::Text(t)
    }

    fn page_with(ops: Vec<DrawOp>) -> Page {
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(12240),
                height: Twips(15840),
            },
        );
        for op in ops {
            p.push(op);
        }
        p
    }

    /// Compare `actual` against the committed golden at `tests/golden/<name>`, so a formatting change
    /// (not just a missing probe substring) fails the test. Regenerate after an intentional change:
    /// `RPT_BLESS=1 cargo test -p rpt-render-html`.
    fn assert_golden(name: &str, actual: &str) {
        let path = format!("{}/tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
        if std::env::var_os("RPT_BLESS").is_some() {
            std::fs::create_dir_all(format!("{}/tests/golden", env!("CARGO_MANIFEST_DIR")))
                .unwrap();
            std::fs::write(&path, actual).unwrap();
            return;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!(
                "missing golden {path}; regenerate with RPT_BLESS=1 cargo test -p rpt-render-html"
            )
        });
        assert_eq!(
            actual, expected,
            "golden mismatch for {name}; if intentional, regenerate with RPT_BLESS=1"
        );
    }

    /// A deterministic page exercising the op kinds and attributes the `contains` probes don't pin:
    /// a right-aligned field, a multi-word text object (escaping + spacing), a filled+stroked box,
    /// and a line.
    fn snapshot_page() -> Page {
        let field = DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(420),
                top: Twips(300),
                width: Twips(1200),
                height: Twips(240),
            },
            text: "42".into(),
            font: FontSpec::default(),
            color: Color {
                a: 255,
                r: 0,
                g: 0,
                b: 0,
            },
            align: TextAlign::Right,
            rotation: 0.0,
            metrics: None,
            source: Some(ObjectRef::new("Details", ObjectKind::Field).named("qty")),
        });
        let label = DrawOp::Text(TextRun {
            bounds: Rect {
                left: Twips(150),
                top: Twips(300),
                width: Twips(3000),
                height: Twips(240),
            },
            text: "A & B < C".into(),
            font: FontSpec::default(),
            color: Color {
                a: 255,
                r: 20,
                g: 40,
                b: 60,
            },
            align: TextAlign::Left,
            rotation: 0.0,
            metrics: None,
            source: Some(ObjectRef::new("Details", ObjectKind::Text).named("label")),
        });
        let box_op = DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(120),
                top: Twips(240),
                width: Twips(4000),
                height: Twips(360),
            },
            fill: Some(
                Color {
                    a: 255,
                    r: 240,
                    g: 240,
                    b: 240,
                }
                .into(),
            ),
            stroke: Some(Stroke {
                color: Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                },
                width: Twips(15),
                style: LineStyle::Single,
            }),
            corner_radius: Twips(0),
            source: Some(ObjectRef::new("Details", ObjectKind::Box).named("frame")),
        });
        let line = DrawOp::Line(LineOp {
            from: Point::new(120, 620),
            to: Point::new(4120, 620),
            stroke: Stroke {
                color: Color {
                    a: 255,
                    r: 0,
                    g: 0,
                    b: 0,
                },
                width: Twips(10),
                style: LineStyle::Single,
            },
            source: Some(ObjectRef::new("Details", ObjectKind::Line).named("rule")),
        });
        page_with(vec![box_op, line, label, field])
    }

    #[test]
    fn golden_html_page_snapshot() {
        assert_golden("page.html", &render_page(&snapshot_page()));
    }

    #[test]
    fn frame_is_reportrenderer_shaped() {
        let html = render_page(&page_with(vec![text_run(
            150,
            150,
            3000,
            300,
            "A & B",
            ObjectKind::Field,
            "name1",
        )]));
        assert!(html.contains("XHTML 1.0 Transitional"));
        assert!(html.contains("<TITLE>Crystal Report Viewer</TITLE>"));
        assert!(html.contains("BGCOLOR=\"FFFFFF\" LEFTMARGIN=31 TOPMARGIN=31"));
        assert!(html.contains("div.crystalstyle div {position:absolute; z-index:25}"));
        // One page container, content width = 816 - 48 = 768px.
        assert!(html.contains("width:768px;height:"));
        assert!(html.contains("overflow:hidden;"));
        // Ampersand escaped, spaces → &nbsp;.
        assert!(html.contains("A&nbsp;&amp;&nbsp;B"));
    }

    #[test]
    fn single_line_field_uses_table_template() {
        let html = render_page(&page_with(vec![text_run(
            420,
            1939,
            660,
            240,
            "1",
            ObjectKind::Field,
            "id1",
        )]));
        assert!(html.contains("id=\"id1\""));
        assert!(html
            .contains("<table width=\"100%\" border=\"0\" cellpadding=\"0\" cellspacing=\"0\">"));
        assert!(html.contains("nowrap=\"true\""));
        assert!(html.contains("data-object=\"id1\""));
    }

    #[test]
    fn text_object_uses_paragraph_template() {
        let html = render_page(&page_with(vec![text_run(
            420,
            1519,
            660,
            240,
            "ID",
            ObjectKind::Text,
            "Text9",
        )]));
        assert!(html.contains("id=\"Text9\""));
        assert!(html.contains(
            "<p style=\"position:relative;padding-left:1px;margin:0px;white-space:nowrap;\">"
        ));
        assert!(html.contains("display:block;line-height:"));
        assert!(!html.contains("<table"));
    }

    #[test]
    fn stacked_lines_merge_into_one_paragraph() {
        // Two runs of the same object, one line-height apart → one Template-A div, two spans.
        let html = render_page(&page_with(vec![
            text_run(9711, 1332, 1095, 240, "Numeric", ObjectKind::Text, "Text13"),
            text_run(9711, 1572, 1095, 240, "Code", ObjectKind::Text, "Text13"),
        ]));
        // Exactly one object div for Text13.
        assert_eq!(html.matches("id=\"Text13\"").count(), 1);
        // Both lines present as stacked spans, top = min run top (1332/15 = 89).
        assert!(html.contains(">Numeric</span>"));
        assert!(html.contains(">Code</span>"));
        assert!(html.contains("top:89px;"));
    }

    #[test]
    fn instance_id_groups_exactly_regardless_of_gap() {
        // Two runs sharing an instance id are one placed object: they merge into one div even when the
        // vertical gap far exceeds the line-height heuristic (which would have split them).
        let html = render_page(&page_with(vec![
            text_run_inst(1000, "line one", "wrapped", 7),
            text_run_inst(9000, "line two", "wrapped", 7),
        ]));
        assert_eq!(
            html.matches("id=\"wrapped\"").count(),
            1,
            "same instance id → one object div"
        );
        assert!(html.contains(">line&nbsp;one</span>"));
        assert!(html.contains(">line&nbsp;two</span>"));

        // Same name but distinct instance ids are two placements → two divs, even one line apart.
        let html = render_page(&page_with(vec![
            text_run_inst(1000, "a", "dup", 1),
            text_run_inst(1240, "b", "dup", 2),
        ]));
        assert_eq!(
            html.matches("id=\"dup\"").count(),
            2,
            "distinct instance ids → separate divs"
        );
    }

    #[test]
    fn distinct_rows_do_not_merge() {
        // Same field name on two detail rows a cell-height apart → two separate divs.
        let html = render_page(&page_with(vec![
            text_run(420, 1939, 660, 840, "1", ObjectKind::Field, "id1"),
            text_run(420, 2779, 660, 840, "2", ObjectKind::Field, "id1"),
        ]));
        assert_eq!(html.matches("id=\"id1\"").count(), 2);
    }

    #[test]
    fn font_and_adornment_classes_are_deduped() {
        let html = render_page(&page_with(vec![
            text_run(0, 0, 660, 240, "a", ObjectKind::Field, "f1"),
            text_run(0, 500, 660, 240, "b", ObjectKind::Field, "f2"),
        ]));
        // Two identical default fonts dedupe to a single fc class definition.
        let uid_defs: Vec<_> = html.match_indices(".fc").collect();
        assert_eq!(
            uid_defs.len(),
            1,
            "duplicate fonts should dedupe to one class"
        );
    }

    #[test]
    fn geometry_rounds_half_away_from_zero() {
        // 1474 twips / 15 = 98.27 → 98; 221/15 = 14.73 → 15.
        assert_eq!(px(1474), 98);
        assert_eq!(px(221), 15);
        assert_eq!(px(11474), 765);
        assert_eq!(px(0), 0);
    }

    #[test]
    fn section_background_is_inline_and_empty() {
        let mut p = page_with(vec![]);
        p.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(360),
                top: Twips(1939),
                width: Twips(12240),
                height: Twips(840),
            },
            fill: Some(
                Color {
                    a: 255,
                    r: 255,
                    g: 255,
                    b: 0,
                }
                .into(),
            ),
            stroke: None,
            corner_radius: Twips(0),
            source: Some(ObjectRef::new("DetailSection1", ObjectKind::Section)),
        }));
        let html = render_page(&p);
        assert!(html.contains("id=\"DetailSection1\""));
        assert!(html.contains("z-index:3;"));
        assert!(html.contains("background-color:#ffff00;layer-background-color:#ffff00;"));
    }

    #[test]
    fn bordered_text_object_merges_box_rect_as_adornment() {
        let mut p = page_with(vec![text_run(
            440,
            440,
            11340,
            600,
            "Title",
            ObjectKind::Text,
            "Text7",
        )]);
        p.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(440),
                top: Twips(440),
                width: Twips(11340),
                height: Twips(600),
            },
            fill: Some(
                Color {
                    a: 255,
                    r: 255,
                    g: 255,
                    b: 255,
                }
                .into(),
            ),
            stroke: Some(Stroke {
                color: Color {
                    a: 255,
                    r: 0,
                    g: 255,
                    b: 255,
                },
                width: Twips(60),
                style: LineStyle::Double,
            }),
            corner_radius: Twips(0),
            source: Some(ObjectRef::new("DetailSection1", ObjectKind::Box).named("Text7")),
        }));
        let html = render_page(&p);
        // The box rect is not a standalone box; it becomes Text7's adornment class.
        assert_eq!(html.matches("id=\"Text7\"").count(), 1);
        assert!(html.contains("border-color:#00ffff;"));
        assert!(html.contains("border-top-style:double;"));
    }

    fn image_page(id: &str) -> Page {
        let mut p = Page::new(
            1,
            PageSize {
                width: Twips(4000),
                height: Twips(4000),
            },
        );
        p.push(DrawOp::Image(rpt_pages::ImageOp {
            bounds: Rect {
                left: Twips(100),
                top: Twips(100),
                width: Twips(720),
                height: Twips(720),
            },
            image_id: id.to_string(),
            source: Some(ObjectRef::new("DetailSection1", ObjectKind::Image).named(id)),
        }));
        p
    }

    #[test]
    fn image_without_asset_renders_placeholder_not_dangling_ref() {
        let html = render_page(&image_page("Picture1"));
        // No broken reference to an unwritten sidecar file, and a visible placeholder instead.
        assert!(!html.contains("images/Picture1.png"));
        assert!(!html.contains("<img"));
        assert!(html.contains("rpt-image-missing"));
    }

    #[test]
    fn image_with_asset_inlines_data_uri() {
        // A 1x1 PNG (real magic so sniff_media_type accepts it).
        let png: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d,
        ];
        let mut assets = BTreeMap::new();
        assets.insert(
            "Picture1".to_string(),
            ImageAsset {
                media_type: "image/png".to_string(),
                bytes: png.to_vec(),
            },
        );
        let html = render_pages_with_assets(&[image_page("Picture1")], &assets);
        assert!(html.contains("src=\"data:image/png;base64,"));
        assert!(!html.contains("images/Picture1.png"));
        // The encoded PNG header round-trips through our base64.
        assert!(html.contains(&base64_encode(png)));
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }
}
