//! Page model: reduce a [`Page`] of draw-ops into emit-ready [`Elem`]s (positioned in px), grouping
//! stacked text runs into object instances and interning styles into the [`Tables`].

use crate::tables::{AdornKey, FontKey, Tables};
use crate::{px, PAGE_MARGIN_PX};
use rpt_model::{Rect, Twips};
use rpt_pages::{DrawOp, ImageAsset, ImageFit, LineOp, ObjectKind, Page, TextAlign, TextRun};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Leading multiple for the twip gap between stacked wrapped lines (~1.2 line pitch).
const LINE_LEADING: f64 = 1.2;
/// Line-height heuristic (~1.17 em) when a run carries no measured metrics.
const LINE_HEIGHT_EM: f64 = 1.17;
/// CSS px per inch at 96 dpi (points/inch is [`rpt_render_util::POINTS_PER_INCH`]).
const PX_PER_INCH: f64 = 96.0;

/// A page-relative position/size in px.
#[derive(Clone, Copy)]
pub(crate) struct Pos {
    pub(crate) left: i64,
    pub(crate) top: i64,
    pub(crate) width: i64,
    pub(crate) height: i64,
}

impl Pos {
    pub(crate) fn from_rect(b: &rpt_model::Rect) -> Pos {
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
pub(crate) enum Elem {
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
        /// One entry per visual line: (font class index, text, justify target width in px). The width
        /// is `Some` only for a justified wrapped (non-last) line, which stretches to fill it.
        lines: Vec<(usize, String, Option<i64>)>,
        line_height: i64,
        /// Text rotation in degrees CCW (`0.0` = upright); applied as a CSS transform about the top-left.
        rotation: f32,
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
        /// Text rotation in degrees CCW (`0.0` = upright); applied as a CSS transform about the top-left.
        rotation: f32,
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
        /// Interned image-class index when the op has embeddable bytes; `None` draws a placeholder.
        class: Option<usize>,
        /// How the raster fills its box: `Contain` letterboxes (preserve aspect, center), else fill.
        fit: ImageFit,
    },
    /// An inline `<svg>` island for a chart (or any non-axis-aligned geometry the div model can't
    /// express): the whole chart's ops rendered as vector SVG, positioned at its bounding box.
    SvgIsland { pos: Pos, svg: String },
}

/// A page reduced to emit-ready elements, plus its container dimensions.
pub(crate) struct PageModel {
    pub(crate) width: i64,
    pub(crate) height: i64,
    pub(crate) elems: Vec<Elem>,
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
    /// The object's text rotation in degrees CCW (`0.0` = upright). A rotated object's runs are placed
    /// as separate columns by the layout, so each is its own single-run group (never merged/stacked).
    rotation: f32,
    first_op: usize,
    runs: Vec<usize>,
    left_px: i64,
    last_top_px: i64,
}

/// Estimate the twips between the top of one wrapped line and the next for a given font — used to
/// tell stacked lines of one cell (gap ≈ line height) from distinct detail rows (gap ≈ cell height).
fn line_gap_twips(font: &rpt_pages::FontSpec) -> i64 {
    // 1pt = 20 twips; ~1.2 leading.
    (font.size_pt as f64 * 20.0 * LINE_LEADING).round() as i64
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
    let font_px = font.size_pt as f64 * PX_PER_INCH / rpt_render_util::POINTS_PER_INCH;
    (font_px * LINE_HEIGHT_EM).round() as i64
}

/// Reduce a page to emit-ready elements, interning fonts/adornments/images into `tables`.
pub(crate) fn build_page(
    page: &Page,
    tables: &mut Tables,
    assets: &BTreeMap<String, ImageAsset>,
) -> PageModel {
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
        // A rotated run is placed as its own column by the layout, so it never merges into a stacked
        // group — each is emitted as a standalone rotated element (upright grouping assumes lines stack
        // vertically, which a rotation breaks).
        let can_merge = t.rotation == 0.0
            && groups.last().is_some_and(|g| match (&g.key, &key) {
                (ObjKey::Instance(a), ObjKey::Instance(b)) => a == b && g.rotation == 0.0,
                (ObjKey::Named(..), ObjKey::Named(..)) => {
                    g.rotation == 0.0
                        && g.key == key
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
                rotation: t.rotation,
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
                // Intern the bytes by content hash so identical images share one `<style>` entry;
                // an op with no asset (chart / undecoded picture) draws a placeholder box.
                let class = assets
                    .get(&im.image_id)
                    .map(|a| tables.image(&a.media_type, &a.bytes));
                elems.push(Elem::Image {
                    top: p.top,
                    left: p.left,
                    width: p.width,
                    height: p.height,
                    class,
                    fit: im.fit,
                });
            }
            DrawOp::Text(_) => {
                let Some(&gi) = group_first.get(&i) else {
                    continue;
                };
                let g = &groups[gi];
                let Some(e) = build_text_object(g, ops, &adorn_of, tables) else {
                    continue;
                };
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
///
/// `None` when the group holds no text runs. The grouping pass only ever puts text-op indices in a
/// [`TextGroup`], so both ways that can happen are internal invariant violations rather than anything
/// a report can cause — but this is a rendering backend, and skipping one object degrades the page
/// where a panic would lose the whole render (and, in a hosted viewer, the host).
fn build_text_object(
    g: &TextGroup,
    ops: &[DrawOp],
    adorn_of: &HashMap<ObjKey, AdornKey>,
    tables: &mut Tables,
) -> Option<Elem> {
    let runs: Vec<&TextRun> = g
        .runs
        .iter()
        .filter_map(|&i| match ops.get(i) {
            Some(DrawOp::Text(t)) => Some(t),
            other => {
                debug_assert!(false, "group index {i} points at {other:?}, not a text op");
                None
            }
        })
        .collect();
    if runs.is_empty() {
        debug_assert!(false, "text group with no text runs");
        return None;
    }

    // Union bounds of all lines in container coordinates (positions un-bake the page margin).
    // `runs` is non-empty, so every min/max below yields a value.
    let left = runs.iter().map(|r| px(r.bounds.left.0)).min().unwrap_or(0);
    let top = runs.iter().map(|r| px(r.bounds.top.0)).min().unwrap_or(0);
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
        // A justified line stretches to its usable width (the layout marks a paragraph's last line
        // `Left`, so only interior wrapped lines carry the justify width).
        let lines = runs
            .iter()
            .map(|r| {
                let justify = (matches!(r.align, TextAlign::Justified)
                    && rpt_render_util::word_gap_count(&r.text) > 0)
                    .then(|| px(r.bounds.width.0));
                (
                    tables.font(FontKey::new(&r.font, r.color)),
                    r.text.clone(),
                    justify,
                )
            })
            .collect();
        Some(Elem::Para {
            id: g.name.clone(),
            section: g.section.clone(),
            kind: g.kind,
            pos,
            adorn,
            align: g.align,
            lines,
            line_height: line_height_px_of(runs[0]),
            rotation: g.rotation,
        })
    } else {
        let r = runs[0];
        Some(Elem::Cell {
            id: g.name.clone(),
            section: g.section.clone(),
            kind: g.kind,
            pos,
            adorn,
            align: g.align,
            font: tables.font(FontKey::new(&r.font, r.color)),
            text: r.text.clone(),
            rotation: g.rotation,
        })
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
