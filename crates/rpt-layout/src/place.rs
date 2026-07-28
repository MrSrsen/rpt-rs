//! Object placement: turn one resolved [`crate::TextPlan`]/report object into Page-IR draw-ops at its
//! twip position on the current page. This is the per-variant emit (`emit_object`), the section
//! background + object loop (`emit_band_plans`), the box/line/border/shadow primitives, the printable
//! rect mapping (`page_rect`), and the recursive subreport placement (`emit_subreport`). The
//! formatter's pagination path (see [`crate::paginate`]) calls these once a band's position is fixed.

use crate::resolve::cond;
use crate::{
    browser_renderable, emf, push_diag, translate_op, EmptyRows, Formatter, SubreportChunk,
    SubreportRender, TextPlan,
};
use crystal_formula::eval::Value;
use crystal_formula::token::strip_braces;
use rpt_data::{
    compile_formulas, compile_formulas_at, normalize_param_name, DataContext, FieldFilter,
    Parameters, RunningTotals, SavedDataSource,
};
use rpt_model::{
    BlobFieldObject, BoxShape, Color, FieldObject, FieldRefKind, ImageFormat, LineShape,
    LineStyle as RptLineStyle, PictureObject, Rect, ReportObject, ReportObjectKind, Section,
    SubreportObject, Twips, VerticalAlignment,
};
use rpt_pages::{
    Diagnostic, DiagnosticKind, DrawOp, ImageFit, ImageOp, LineOp, LineStyle, ObjectKind,
    ObjectRef, RectOp, Stroke, TextRun,
};

/// The engine's default thin rule width (a hairline, ~0.7px at 96dpi) — used for object borders,
/// grid lines, and as the minimum stroke width.
pub(crate) const HAIRLINE_W: Twips = Twips(10);

/// The default border colour when a box/field border sets no explicit colour: opaque black.
const DEFAULT_BORDER: Color = Color {
    a: 255,
    r: 0,
    g: 0,
    b: 0,
};

/// The dashed placeholder-box outline colour (mid-grey) drawn for objects that cannot be rendered.
const PLACEHOLDER_STROKE: Color = Color {
    a: 255,
    r: 136,
    g: 136,
    b: 136,
};

/// A solid-filled rectangle with square corners and no border — the shared shape for section
/// backgrounds, drop-shadow bars, and cross-tab header shading.
pub(crate) fn fill_rect(bounds: Rect, color: Color, source: Option<ObjectRef>) -> DrawOp {
    DrawOp::Rect(RectOp {
        bounds,
        fill: Some(color.into()),
        stroke: None,
        corner_radius: Twips(0),
        source,
    })
}

/// A single hairline stroke in `color` — the engine's default thin rule for borders and grid lines.
pub(crate) fn hairline_stroke(color: Color) -> Stroke {
    Stroke {
        color,
        width: HAIRLINE_W,
        style: LineStyle::Single,
    }
}

/// Whether `inner`'s top-left corner lies within `outer` (inclusive). Used to detect a cross-tab's
/// decomposed cell objects, which the decoder places flat in the section but geometrically inside the
/// cross-tab box; a degenerate (zero-width/height) cell is matched by its corner alone.
fn rect_contains_top_left(outer: &rpt_model::Rect, inner: &rpt_model::Rect) -> bool {
    inner.left.0 >= outer.left.0
        && inner.left.0 <= outer.left.0 + outer.width.0
        && inner.top.0 >= outer.top.0
        && inner.top.0 <= outer.top.0 + outer.height.0
}

/// The name of an object kind that the placement path does not yet draw, or `None` for a kind that
/// is handled (or an intentional no-op). Drives the aggregated "not yet rendered" diagnostic.
fn unsupported_kind_name(kind: &ReportObjectKind) -> Option<&'static str> {
    use ReportObjectKind as K;
    match kind {
        K::FieldHeading(_) => Some("FieldHeading"),
        K::OlapGrid => Some("OlapGrid"),
        K::Map => Some("Map"),
        K::Flash => Some("Flash"),
        K::Deferred(_) => Some("Deferred"),
        K::Unknown => Some("Unknown"),
        _ => None,
    }
}

/// The positional context an object is placed into: the band's section name, its page-space
/// `origin_y` and grown `band_height`, the section's design height (gating a section-spanning box's
/// growth), and the per-object `instance` id shared by an object's runs and adornment.
#[derive(Debug, Clone, Copy)]
struct Placement<'a> {
    section_name: &'a str,
    origin_y: i32,
    band_height: i32,
    section_design_height: i32,
    instance: u32,
}

impl Placement<'_> {
    /// The [`ObjectRef`] naming `obj` within this placement's section, tagged with the shared
    /// instance id — the source stamped on every draw-op an object emits.
    fn src(&self, obj: &ReportObject, kind: ObjectKind) -> Option<ObjectRef> {
        Some(
            ObjectRef::new(self.section_name, kind)
                .named(&obj.name)
                .with_instance(self.instance),
        )
    }
}

impl Formatter<'_> {
    /// Emit a band's section background + objects at `origin_y` (respecting `self.col_offset` for
    /// multi-column). Does not paginate or advance the cursor. `skip`, when set, omits the object at
    /// that index — used by the cross-page-flow path, which places its inline subreport separately
    /// (flowing it across pages) after this emits the band's other objects once.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_band_plans(
        &mut self,
        section: &Section,
        plans: &[Option<TextPlan>],
        origin_y: i32,
        height: i32,
        ctx: Option<&DataContext>,
        subs: &[Option<SubreportRender>],
        skip: Option<usize>,
    ) {
        // A section-level BackgroundColor condition formula (e.g. tinting an under-performer's group
        // header) overrides the static fill, resolved per record; the static color is the fallback.
        let bg = crate::resolve::cond_color_any(
            &section.condition_formulas,
            crate::resolve::cond::SECTION_BACK_COLORS,
            ctx,
        )
        .or(section.format.background_color);
        if let Some(bg) = bg {
            self.cur.push(fill_rect(
                Rect {
                    left: Twips(self.col_offset),
                    top: Twips(origin_y),
                    width: self.page_size.width,
                    height: Twips(height),
                },
                bg,
                Some(ObjectRef::new(&section.name, ObjectKind::Section)),
            ));
        }
        // A cross-tab decomposes into its own cell/label/summary objects, which the decoder surfaces
        // flat in the section alongside the cross-tab. The native grid ([`Self::emit_crosstab`]) draws
        // the whole pivot, so those decomposed objects — everything (bar the cross-tab) whose top-left
        // sits inside a cross-tab's box — must not be drawn again on top of it.
        let crosstab_rects: Vec<rpt_model::Rect> = section
            .objects
            .iter()
            .filter(|o| matches!(o.kind, ReportObjectKind::CrossTab(_)))
            .map(|o| o.bounds)
            .collect();
        // Box fills underlay the row content: emit box objects first, then everything else, so a
        // row-shading (zebra) box sits behind the text/fields/images even though it is stored after
        // them in object order. `height` is the band's grown height; the section's design height gates
        // a section-spanning box's growth (see `emit_object`).
        let design_height = section.height.0;
        for boxes_first in [true, false] {
            for (i, (obj, plan)) in section.objects.iter().zip(plans).enumerate() {
                if Some(i) == skip {
                    continue;
                }
                if matches!(obj.kind, ReportObjectKind::Box(_)) != boxes_first {
                    continue;
                }
                if !matches!(obj.kind, ReportObjectKind::CrossTab(_))
                    && crosstab_rects
                        .iter()
                        .any(|ct| rect_contains_top_left(ct, &obj.bounds))
                {
                    continue;
                }
                self.emit_object(
                    obj,
                    &section.name,
                    origin_y,
                    plan.as_ref(),
                    ctx,
                    height,
                    design_height,
                    subs.get(i).and_then(Option::as_ref),
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_object(
        &mut self,
        obj: &ReportObject,
        section_name: &str,
        origin_y: i32,
        plan: Option<&TextPlan>,
        ctx: Option<&DataContext>,
        band_height: i32,
        section_design_height: i32,
        sub: Option<&SubreportRender>,
    ) {
        // A conditional visibility formula, when present, overrides the static suppress flag. It is
        // stored under its reserved Crystal name (`Object_Visibility`), evaluated per row: a `True`
        // result hides the object (drives the zebra row shading and per-row flags).
        let suppressed =
            crate::resolve::cond_bool(&obj.format.condition_formulas, cond::OBJECT_VISIBILITY, ctx)
                .unwrap_or(obj.format.suppress.value);
        if suppressed {
            return;
        }
        // One instance id per placed object: its text runs and its border/fill box share it, so the
        // HTML backend groups a wrapped value and links its adornment by id, not by geometry.
        let instance = self.next_instance_id;
        self.next_instance_id += 1;
        let at = Placement {
            section_name,
            origin_y,
            band_height,
            section_design_height,
            instance,
        };

        match &obj.kind {
            ReportObjectKind::Field(_)
            | ReportObjectKind::Text(_)
            | ReportObjectKind::FieldHeading(_) => self.emit_text_object(obj, plan, &at),
            ReportObjectKind::Box(b) => self.emit_box(obj, b, ctx, &at),
            ReportObjectKind::Line(l) => self.emit_line(obj, l, &at),
            ReportObjectKind::Picture(p) => self.emit_picture(obj, p, &at),
            ReportObjectKind::BlobField(bf) => self.emit_blob_field(obj, bf, ctx, &at),
            // Charts render as native draw-ops from the group summaries; cross-tabs
            // and unlinked subreports still fall back to a placeholder box carrying identity.
            ReportObjectKind::Chart(c) => {
                let rect = self.page_rect(&obj.bounds, origin_y);
                self.emit_chart(c, rect, section_name, obj)
            }
            ReportObjectKind::CrossTab(ct) => {
                let rect = self.page_rect(&obj.bounds, origin_y);
                self.emit_crosstab(ct, rect, section_name, obj)
            }
            ReportObjectKind::Subreport(sr) => {
                let rect = self.page_rect(&obj.bounds, origin_y);
                self.emit_subreport(sr, obj, rect, section_name, instance, sub)
            }
            // A kind the placement path does not yet draw: emit one aggregated (deduped) diagnostic
            // naming it rather than dropping it silently, matching the EMF/WMF/cross-tab paths.
            other => {
                if let Some(name) = unsupported_kind_name(other) {
                    push_diag(
                        &self.diagnostics,
                        Diagnostic::warn(
                            DiagnosticKind::UnsupportedObject,
                            format!("object kind {name} is not yet rendered; omitted"),
                        )
                        .in_section(section_name),
                    );
                }
            }
        }
    }

    /// Place a text/field/field-heading object: its background fill (behind the glyphs), one text run
    /// per wrapped line (with vertical alignment and per-line rotation), the page-total fixup for a
    /// `TotalPageCount`/`PageNofM` field, and the border stroke (on top).
    fn emit_text_object(&mut self, obj: &ReportObject, plan: Option<&TextPlan>, at: &Placement) {
        let rect = self.page_rect(&obj.bounds, at.origin_y);
        // Background fill goes *behind* the text; the border stroke on top. Drawing them in one
        // rect after the text would paint the fill over the glyphs (a black header bar with
        // white text would render as a solid black bar).
        self.push_object_fill(obj, rect, at.section_name, at.instance);
        if let Some(plan) = plan {
            // Text rotation (0 / 90 / 270°, CCW) is applied by the backends as a transform about
            // the run's top-left; the layout metrics and box are unaffected.
            let rotation = obj.format.text_rotation.degrees() as f32;
            // Each line carries its own pitch (mixed paragraph fonts / 1.5 / double / exact
            // spacing make lines unequal); the block's total height is their sum.
            let total_h: i32 = plan.lines.iter().map(|l| l.line_height.0).sum();
            // Vertical alignment offsets the whole text block within the box's free space.
            // With can-grow the box grows to fit the text, so there is no slack (free == 0 →
            // top); a fixed box taller than its text centres or bottom-aligns the block.
            let free = (rect.height.0 - total_h).max(0);
            let v_offset = match obj.format.vertical_alignment {
                VerticalAlignment::VerticalCenter => free / 2,
                VerticalAlignment::Bottom => free,
                _ => 0,
            };
            // One text run per wrapped line. Upright text stacks from the object's top by each
            // line's own pitch; a quarter-turn rotation stacks the lines as columns across the
            // box (see `rotated_line_rect`), so the backend's per-run rotation about the run's
            // top-left composes them into vertical, wrapped, engine-matching columns. Each
            // line's `x_offset`/`width` carry its paragraph indentation and its
            // `font`/`ascent`/`line_height` its paragraph's resolved font metrics (all equal to
            // the object font in the common single-font case, so the bounds are unchanged).
            let deg = obj.format.text_rotation.degrees();
            let multiline = plan.lines.len() > 1;
            let mut y = rect.top.0 + v_offset;
            for (i, line) in plan.lines.iter().enumerate() {
                let line_rect = if deg % 180 == 90 {
                    rotated_line_rect(rect, deg, i as i32, line.line_height.0, line.width)
                } else {
                    let mut lr = rect;
                    lr.top = Twips(y);
                    if multiline {
                        lr.height = line.line_height;
                    }
                    lr.left = Twips(rect.left.0 + line.x_offset.0);
                    lr.width = line.width;
                    y += line.line_height.0;
                    lr
                };
                let advance = Twips(self.text_layout.width_twips(&line.text, &line.font) as i32);
                self.cur.push(DrawOp::Text(TextRun {
                    bounds: line_rect,
                    text: line.text.clone(),
                    font: line.font.clone(),
                    color: plan.color,
                    align: line.align,
                    rotation,
                    metrics: Some(rpt_pages::TextMetrics {
                        advance,
                        ascent: line.ascent,
                        line_height: line.line_height,
                    }),
                    source: at.src(obj, plan.kind),
                }));
            }
            // A `TotalPageCount`/`PageNofM` field is a forward reference: its total is not
            // known until the last page is laid out. Record the just-emitted single-line run
            // so the final count can be patched in (see `Formatter::resolve_page_totals`).
            if plan.lines.len() == 1 {
                if let ReportObjectKind::Field(f) = &obj.kind {
                    if total_page_dependent(f) {
                        self.page_count_fixups.push(crate::PageCountFixup {
                            page_index: self.pages.len(),
                            op_index: self.cur.ops.len() - 1,
                            field: (**f).clone(),
                            page_number: self.page_number,
                        });
                    }
                }
            }
        }
        self.push_border(obj, rect, at.section_name, at.instance);
    }

    /// Draw a box shape: its fill/border rect (resolving the per-row conditional-format formulas
    /// first, then the static colours), extended to its end section or grown with the band as needed,
    /// plus its drop shadow.
    fn emit_box(
        &mut self,
        obj: &ReportObject,
        b: &BoxShape,
        ctx: Option<&DataContext>,
        at: &Placement,
    ) {
        let rect = self.page_rect(&obj.bounds, at.origin_y);
        // A box that names a later end section spans down to it: its height is the run from
        // its top to that section's bottom (summed from the design section layout). Otherwise
        // it is an in-section box: a section-spanning background box (its design bottom reaches
        // most of the way down the section) grows with the band, so when can-grow content makes
        // the row taller the shading/frame tracks the actual rendered height instead of covering
        // only the top slice, preserving its distance from the band bottom.
        let rect = if spans_sections(&b.end_section_name, at.section_name) {
            self.span_rect(rect, at.section_name, &b.end_section_name, obj.bounds.top.0)
        } else {
            extend_section_box(rect, &obj.bounds, at.band_height, at.section_design_height)
        };
        // Fill/border resolve the per-row conditional-format formulas first (e.g. a color
        // swatch's `BackgroundColor = Color({r},{g},{b})`), then the static colors.
        let conds = &obj.border.condition_formulas;
        let fill = crate::resolve::cond_color(conds, cond::BACK_COLOR, ctx)
            .or(obj.border.background_color)
            .or(b.fill_color);
        let border_color =
            crate::resolve::cond_color(conds, cond::FORE_COLOR, ctx).unwrap_or(b.shape.line_color);
        self.cur.push(DrawOp::Rect(RectOp {
            bounds: rect,
            fill: fill.map(Into::into),
            stroke: stroke_of(b.shape.line_style, border_color, b.shape.line_thickness),
            corner_radius: Twips(b.corner_ellipse_width.0.max(b.corner_ellipse_height.0)),
            source: at.src(obj, ObjectKind::Box),
        }));
        // Drop shadow: two filled bars offset to the bottom-right in the border color (the
        // engine draws the shadow as edge rectangles, not a CSS shadow).
        if obj.border.has_drop_shadow {
            self.push_drop_shadow(rect, border_color, at.section_name, &obj.name, at.instance);
        }
    }

    /// Draw a line shape: its endpoints from the object rect (extended to its end section for a
    /// spanning line), stroked with the style/colour from the border record (falling back to the
    /// shape's own fields).
    fn emit_line(&mut self, obj: &ReportObject, l: &LineShape, at: &Placement) {
        let rect = self.page_rect(&obj.bounds, at.origin_y);
        // A horizontal line uses the box top; a vertical line its left. Use the object rect.
        let (mut from, mut to) = line_endpoints(rect);
        // A line that names a later end section spans down to it (a spanning line is vertical):
        // extend its lower endpoint to that section's bottom in the design section layout.
        if spans_sections(&l.end_section_name, at.section_name) {
            if let Some(h) =
                self.design_span_height(at.section_name, &l.end_section_name, obj.bounds.top.0)
            {
                let bottom = rect.top.0 + h;
                if to.y.0 >= from.y.0 {
                    to.y = Twips(to.y.0.max(bottom));
                } else {
                    from.y = Twips(from.y.0.max(bottom));
                }
            }
        }
        // A line's stroke style and colour live in its border record (the `0xec` leaf), while
        // the shape carries only the thickness. Prefer the border's style/colour; fall back to
        // the shape's own fields for a line that does carry them there.
        let style = border_line_style(&obj.border).unwrap_or(l.shape.line_style);
        let color = obj.border.border_color.unwrap_or(l.shape.line_color);
        if let Some(stroke) = stroke_of(style, color, l.shape.line_thickness) {
            self.cur.push(DrawOp::Line(LineOp {
                from,
                to,
                stroke,
                source: at.src(obj, ObjectKind::Line),
            }));
        }
    }

    /// Place a picture object: register its browser-renderable raster as a page asset and draw it at
    /// its scaled size; interpret an EMF vector stream into draw-ops; otherwise draw a placeholder.
    fn emit_picture(&mut self, obj: &ReportObject, p: &PictureObject, at: &Placement) {
        let rect = self.page_rect(&obj.bounds, at.origin_y);
        // Collect the picture's decoded bytes as a page-document asset (keyed by the same
        // name the ImageOp references), so any backend can inline it without the caller
        // gathering images separately — only a browser-renderable raster is kept; anything
        // else has no asset and the backend draws a placeholder.
        let fmt = p.image_format();
        if browser_renderable(fmt) {
            if let Some(bytes) = p.to_bmp() {
                self.assets.borrow_mut().insert(
                    obj.name.clone(),
                    rpt_pages::ImageAsset {
                        media_type: fmt.mime_type().to_string(),
                        bytes: bytes.into_owned(),
                    },
                );
            }
            // The raster is drawn at its authored scaled size (`original × scaling`), then
            // fitted into that box preserving its pixel aspect ratio (Crystal letterboxes,
            // it does not distort). Edge cropping (`crop_*`) is latent
            // (all-zero) and has no Page-IR representation, so it is not yet applied.
            self.cur.push(DrawOp::Image(ImageOp {
                bounds: scaled_picture_rect(p, rect),
                image_id: obj.name.clone(),
                fit: ImageFit::Contain,
                source: at.src(obj, ObjectKind::Image),
            }));
        } else if fmt == ImageFormat::Emf {
            // An EMF is a vector command stream: interpret its records into draw-ops mapped
            // into the box. A bad/truncated stream falls back to the placeholder image op.
            match emf::interpret_emf(&p.data, rect, at.src(obj, ObjectKind::Image), &obj.name) {
                Ok(interp) => {
                    for (id, asset) in interp.assets {
                        self.assets.borrow_mut().insert(id, asset);
                    }
                    for op in interp.ops {
                        self.cur.push(op);
                    }
                    if interp.has_emf_plus {
                        push_diag(
                            &self.diagnostics,
                            Diagnostic::warn(
                                DiagnosticKind::UnsupportedObject,
                                "EMF picture embeds EMF+ content that was not rendered",
                            )
                            .with_source(&obj.name),
                        );
                    }
                }
                Err(e) => {
                    // The reason distinguishes cases the user can act on: a truncated picture is
                    // damaged data, a `NotAMetafile` is probably a different image format, and an
                    // `Unsupported` is a gap in this parser — a bug report, not a broken file.
                    push_diag(
                        &self.diagnostics,
                        Diagnostic::warn(
                            DiagnosticKind::UnsupportedObject,
                            match e {
                                // The picture was already classified as EMF upstream
                                // (`ImageFormat::Emf`), so the parser disagreeing means the header
                                // looks like an EMF but its signature does not hold up — a
                                // mis-detected or corrupt picture, not simply "another format".
                                metafile::Error::NotAMetafile => format!(
                                    "picture was detected as EMF but carries no valid EMF \
                                     signature ({} bytes); rendered as a placeholder",
                                    p.data.len()
                                ),
                                metafile::Error::Unsupported { .. } => format!(
                                    "EMF picture uses something this parser does not handle: {e}; \
                                     rendered as a placeholder"
                                ),
                                e => format!(
                                    "EMF picture could not be interpreted: {e}; rendered as a \
                                     placeholder"
                                ),
                            },
                        )
                        .with_source(&obj.name),
                    );
                    self.cur.push(DrawOp::Image(ImageOp {
                        bounds: rect,
                        image_id: obj.name.clone(),
                        fit: ImageFit::Fill,
                        source: at.src(obj, ObjectKind::Image),
                    }));
                }
            }
        } else {
            // WMF / OLE-embedded / other metafile presentations are not yet interpreted
            // (separate follow-ups); draw the placeholder image op.
            self.cur.push(DrawOp::Image(ImageOp {
                bounds: rect,
                image_id: obj.name.clone(),
                fit: ImageFit::Fill,
                source: at.src(obj, ObjectKind::Image),
            }));
        }
    }

    /// Place a blob (database image) field: resolve the bound field from the current row, decode it to
    /// image bytes, register a per-instance asset (`Name#instance`) and draw it; a null or non-raster
    /// value falls back to a placeholder image op keyed by the object name.
    fn emit_blob_field(
        &mut self,
        obj: &ReportObject,
        bf: &BlobFieldObject,
        ctx: Option<&DataContext>,
        at: &Placement,
    ) {
        let rect = self.page_rect(&obj.bounds, at.origin_y);
        // The blob's bytes come from the current row: resolve the bound field, decode it to
        // image bytes, and register a per-instance asset so every row's picture is distinct
        // (`Name#instance`). A null value, or bytes that aren't a browser-renderable raster,
        // fall back to the placeholder image op keyed by the object name (no asset).
        let image_id = format!("{}#{}", obj.name, at.instance);
        let asset = ctx
            .and_then(|c| crate::resolve::blob_value(&bf.data_source, c))
            .and_then(decode_blob_image);
        match asset {
            Some((fmt, bytes)) => {
                self.assets.borrow_mut().insert(
                    image_id.clone(),
                    rpt_pages::ImageAsset {
                        media_type: fmt.mime_type().to_string(),
                        bytes,
                    },
                );
                self.cur.push(DrawOp::Image(ImageOp {
                    bounds: rect,
                    image_id,
                    fit: ImageFit::Contain,
                    source: at.src(obj, ObjectKind::Image),
                }));
            }
            None => self.cur.push(DrawOp::Image(ImageOp {
                bounds: rect,
                image_id: obj.name.clone(),
                fit: ImageFit::Fill,
                source: at.src(obj, ObjectKind::Image),
            })),
        }
    }

    /// Format every non-suppressed, non-on-demand subreport in `section` exactly once, returning a
    /// per-object cache (parallel to `section.objects`) of its box-local draw-ops and used height.
    ///
    /// This runs in the band-planning phase, **before** pagination, so a subreport taller than its
    /// placeholder box can grow the enclosing band (see [`Formatter::grow_for_subreports`]) and the
    /// existing checkpoint pagination then flows the enlarged band across pages. It must run once and
    /// be cached because formatting a subreport fires its `Shared`/`Global` variable writes — running
    /// it a second time at emit would double-count them (e.g. a Shared grand total the main report
    /// reads back). Suppressed and on-demand subreports are skipped (never formatted).
    pub(crate) fn run_band_subreports(
        &self,
        section: &Section,
        ctx: Option<&DataContext>,
    ) -> Vec<Option<SubreportRender>> {
        section
            .objects
            .iter()
            .map(|obj| {
                let ReportObjectKind::Subreport(sr) = &obj.kind else {
                    return None;
                };
                if sr.on_demand {
                    return None;
                }
                let suppressed = crate::resolve::cond_bool(
                    &obj.format.condition_formulas,
                    cond::OBJECT_VISIBILITY,
                    ctx,
                )
                .unwrap_or(obj.format.suppress.value);
                if suppressed {
                    return None;
                }
                self.format_subreport(sr, obj.bounds.height.0, true)
            })
            .collect()
    }

    /// Grow `height` so every cached subreport (from [`Self::run_band_subreports`]) fits below its
    /// placeholder box top. Mirrors the can-grow text growth in
    /// [`Formatter::band_plans_and_height`], keeping subreport growth on the same band-planning path.
    pub(crate) fn grow_for_subreports(
        &self,
        section: &Section,
        subs: &[Option<SubreportRender>],
        height: i32,
    ) -> i32 {
        section
            .objects
            .iter()
            .zip(subs)
            .filter_map(|(obj, sub)| sub.as_ref().map(|s| obj.bounds.top.0 + s.used_height()))
            .fold(height, i32::max)
    }

    /// Render a subreport (a full nested [`rpt_model::Report`]) into the placeholder object's box.
    ///
    /// An **on-demand** subreport ([`SubreportObject::on_demand`]) is not executed: the engine draws a
    /// click-to-expand placeholder that only runs the subreport on expansion (which a static
    /// HTML/PDF export never triggers), so we emit just its caption (the subreport name) and return.
    ///
    /// An **inline** subreport is normally formatted ahead of pagination and passed in via `cached`
    /// (see [`Self::run_band_subreports`]); its box-local ops are translated so the subreport's
    /// printable top-left lands at the box's top-left and emitted in full — the band was already grown
    /// to fit, so nothing is clipped. When no cache is supplied (the fixed page-header/footer and
    /// multi-column paths, which do not grow) it falls back to formatting the subreport once here and
    /// clipping its first page to the placeholder box.
    fn emit_subreport(
        &mut self,
        sr_obj: &SubreportObject,
        obj: &ReportObject,
        rect: Rect,
        section_name: &str,
        instance: u32,
        cached: Option<&SubreportRender>,
    ) {
        // On-demand: emit the caption placeholder only; never execute the subreport.
        if sr_obj.on_demand {
            self.emit_subreport_caption(sr_obj, obj, rect, section_name, instance);
            return;
        }
        // Cache hit: the subreport was formatted ahead of pagination and the band grown to fit it, so
        // translate its box-local ops to the box top-left and emit them all (no clipping). This is the
        // atomic (fits-on-the-page) path — the cross-page-flow path places the subreport itself and
        // never routes it through here (see [`Formatter::emit_band`]), so a single chunk is expected.
        if let Some(render) = cached {
            if let Some(first) = render.chunks.first() {
                self.place_subreport_ops(&first.ops, rect, None);
            }
            return;
        }
        // Cache miss (fixed / multi-column bands, not grown): format once here and clip to the box.
        if let Some(render) = self.format_subreport(sr_obj, obj.bounds.height.0, false) {
            if let Some(first) = render.chunks.first() {
                self.place_subreport_ops(&first.ops, rect, Some(rect.height.0));
            }
        }
    }

    /// Place a subreport's box-local ops (printable top-left at `0,0`) into its box at `rect`: shift
    /// each by the box top-left and lift its 0-based instance ids into the parent's id space so they
    /// don't collide. `clip_below`, when set, drops ops whose top lies at/below that box-local height
    /// (the fallback clipped path); `None` emits every op (the grown, cached path).
    fn place_subreport_ops(&mut self, ops: &[DrawOp], rect: Rect, clip_below: Option<i32>) {
        let (dx, dy) = (rect.left.0, rect.top.0);
        let id_offset = self.next_instance_id;
        let mut max_instance: Option<u32> = None;
        for op in ops {
            if let Some(limit) = clip_below {
                if op.bounds().top.0 >= limit {
                    continue;
                }
            }
            let moved = translate_op(op, dx, dy, id_offset);
            if let Some(inst) = moved.source().and_then(|s| s.instance) {
                max_instance = Some(max_instance.map_or(inst, |m| m.max(inst)));
            }
            self.cur.push(moved);
        }
        // Advance past the merged subreport ids so the parent's next placement id is unique.
        if let Some(m) = max_instance {
            self.next_instance_id = m + 1;
        }
    }

    /// Flow a tall subreport's cached, box-local chunks across parent pages, splitting each chunk at
    /// row boundaries so it never overflows the body. Pure geometry over the already-run child (the
    /// child is never re-formatted, so its `Shared`/`Global` writes fire exactly once): each chunk is a
    /// forced page break (a child page boundary); within a chunk, filling the body soft-breaks to the
    /// next page. `rect` is the subreport box on the current page (its top the first slice's anchor).
    /// Leaves `cursor_y` at the bottom of the last placed slice so following bands flow beneath it.
    pub(crate) fn place_subreport_flowing(&mut self, chunks: &[SubreportChunk], rect: Rect) {
        let dx = rect.left.0;
        let id_offset = self.next_instance_id;
        let mut max_instance: Option<u32> = None;
        for (ci, chunk) in chunks.iter().enumerate() {
            if ci > 0 {
                // A child page break: each subsequent chunk starts a fresh parent page.
                self.finish_page();
                self.begin_page();
                self.cursor_y = 0;
            }
            // Row boundaries = the distinct op-bottom Y values in the chunk (detail rows are stacked
            // bands, so op-bottoms cluster at row edges); a slice breaks at one of these.
            let mut boundaries: Vec<i32> =
                chunk.ops.iter().map(|op| op.bounds().bottom().0).collect();
            boundaries.sort_unstable();
            boundaries.dedup();
            let mut y_lo = 0;
            let mut first_slice = true;
            loop {
                // The first slice of a chunk anchors at the box top; a slice after a soft break starts
                // at the top of the fresh page.
                let parent_top = if first_slice { rect.top.0 } else { 0 };
                first_slice = false;
                // Available body height for this slice (`.max(1)` guarantees forward progress even if
                // the box top sits at/below the body bottom, so the loop can never spin).
                let avail = (self.body_bottom - parent_top).max(1);
                let limit = y_lo + avail;
                // Break at the largest row boundary that fits; if a single row is taller than the page,
                // fall back to the available height to force progress.
                let y_break = boundaries
                    .iter()
                    .copied()
                    .filter(|&b| b > y_lo && b <= limit)
                    .max()
                    .unwrap_or(limit);
                let dy = parent_top - y_lo;
                for op in &chunk.ops {
                    let top = op.bounds().top.0;
                    if top < y_lo || top >= y_break {
                        continue;
                    }
                    let moved = translate_op(op, dx, dy, id_offset);
                    if let Some(inst) = moved.source().and_then(|s| s.instance) {
                        max_instance = Some(max_instance.map_or(inst, |m| m.max(inst)));
                    }
                    self.cur.push(moved);
                }
                let slice_start = y_lo;
                y_lo = y_break;
                if y_lo >= chunk.height {
                    // The last slice's parent-space bottom becomes the flow cursor.
                    self.cursor_y = parent_top + (chunk.height - slice_start);
                    break;
                }
                // Soft break: the chunk overflows this page, so continue it on the next.
                self.finish_page();
                self.begin_page();
                self.cursor_y = 0;
            }
        }
        // Advance past the merged subreport ids so the parent's next placement id is unique.
        if let Some(m) = max_instance {
            self.next_instance_id = m + 1;
        }
    }

    /// Format a subreport once against its per-instance link-filtered dataset and return its box-local
    /// draw-ops (the subreport's printable top-left mapped to `0,0`) with the content's used height.
    /// Its diagnostics and image assets are lifted into the parent document here (once per format).
    ///
    /// The enclosing row's link values (from [`SubreportObject::links`]) are bound into the
    /// subreport's parameters (parameter-routed links) and applied as structural filters (direct field
    /// links), so each instance renders only the parent-linked subset. `flow` runs the child formatter
    /// without pagination (a single tall page) so the whole subreport lands in one op list for inline
    /// growth; the non-flow path paginates normally and returns only its first page's ops.
    fn format_subreport(
        &self,
        sr_obj: &SubreportObject,
        box_height: i32,
        flow: bool,
    ) -> Option<SubreportRender> {
        let sub = self
            .report
            .subreports
            .iter()
            .find(|s| s.name == sr_obj.subreport_name)?;
        let sub_report = &sub.report;
        // Bind the parent row's link values: parameter-routed links (`linked_parameter`) merge into
        // the subreport's parameters (consumed by its record-selection formula and the WHERE
        // push-down); direct field links become structural equality filters on the fetched rows.
        let (params, link_filters) = self.subreport_link_bindings(&sr_obj.links);
        // Prefer live rows from the scope-data provider; fall back to the subreport's
        // saved data, then to empty. `live` is held so the boxed source outlives `build_dataset`.
        let live = self.scope_data.and_then(|p| p.rows_for(sub_report));
        let saved_holder;
        let source: &dyn rpt_data::RowSource = match (&live, &sub_report.saved_data) {
            (Some(src), _) => src.as_ref(),
            (None, Some(saved)) => {
                saved_holder = SavedDataSource::from_report(saved, sub_report);
                &saved_holder
            }
            (None, None) => &EmptyRows,
        };
        // A subreport inherits the parent render's as-of instant, so its date-relative
        // formulas (`CurrentDate`/…) resolve against the same fixed time as the main report.
        let datetime = self.formulas.datetime();
        // A subreport's pipeline fails open exactly like the main report's, so it needs the same sink —
        // a subreport that silently drops every row is if anything harder to notice, being one box on
        // an otherwise plausible page.
        let sub_sink = rpt_data::CollectingSink::new();
        let dataset = rpt_data::build_dataset_opts(
            source,
            &sub_report.data_definition,
            rpt_data::DatasetOptions {
                params: Some(&params),
                extra: &link_filters,
                sink: Some(&sub_sink),
                datetime,
            },
        );
        let formulas = match datetime {
            Some(dt) => compile_formulas_at(&sub_report.data_definition, dt),
            None => compile_formulas(&sub_report.data_definition),
        };
        // A subreport gets its own Global variable store (its running totals / global counters reset
        // per subreport, matching the engine) but **shares the parent's `Shared` scope** — a `Shared`
        // variable set in the main report is visible in the subreport and vice-versa.
        let sub_state = self.state_vars.child();
        let sub_running = RunningTotals::from_data_def(&sub_report.data_definition);
        let sub_scheduled = crate::run_schedule(sub_report, &dataset, &formulas, &sub_state);
        let mut sub_fmt = Formatter::new(
            sub_report,
            &dataset,
            &formulas,
            self.text_layout,
            &sub_state,
            &sub_running,
            &sub_scheduled,
            self.scope_data,
            self.locale,
        );
        if flow {
            sub_fmt.set_flow_mode();
        }
        let mut sub_doc = sub_fmt.run();
        // Fold the subreport's data-pipeline diagnostics in ahead of its layout ones, so both get the
        // subreport-name tagging below and a selection failure is read before its consequences.
        let mut sub_diags = crate::diagnostics::from_evals(&sub_sink.into_diagnostics());
        sub_diags.append(&mut sub_doc.diagnostics);
        // Lift the subreport's own diagnostics into the parent document, tagged with its name.
        for mut d in sub_diags {
            d.source = Some(match d.source {
                Some(s) => format!("{}/{s}", sr_obj.subreport_name),
                None => sr_obj.subreport_name.clone(),
            });
            push_diag(&self.diagnostics, d);
        }
        // Lift the subreport's image assets into the parent document (its pictures render in the box).
        self.assets.borrow_mut().extend(sub_doc.assets);

        // Map the subreport's printable origin (its margins) to box-local `0,0`. Flow mode paginates
        // only on the subreport's own forced `NewPage` breaks (the body is unbounded), so it yields
        // one chunk per such break — the interior boundaries become forced parent page breaks. The
        // non-flow path (fixed / multi-column bands) takes only the first page.
        let mx = sub_report.print_options.margins.left.0;
        let my = sub_report.print_options.margins.top.0;
        if sub_doc.pages.is_empty() {
            return None;
        }
        let take = if flow { sub_doc.pages.len() } else { 1 };
        let chunks: Vec<SubreportChunk> = sub_doc
            .pages
            .iter()
            .take(take)
            .map(|page| {
                let ops: Vec<DrawOp> = page.ops.iter().map(|op| op.translate(-mx, -my)).collect();
                // Height = the deepest op bottom below the box top; a chunk shorter than the box keeps
                // the box height so the band never shrinks.
                let height = ops
                    .iter()
                    .map(|op| op.bounds().bottom().0)
                    .max()
                    .unwrap_or(0)
                    .max(box_height);
                SubreportChunk { ops, height }
            })
            .collect();
        Some(SubreportRender { chunks })
    }

    /// Resolve the enclosing row's subreport link values into a `(parameters, filters)` pair for the
    /// per-instance subreport dataset. A parameter-routed link (`linked_parameter`) merges the parent
    /// field's value into the subreport's parameters (consumed by its record-selection formula and the
    /// live-DB `WHERE` push-down); a direct field link becomes a structural equality filter on the
    /// subreport field. A link whose parent field is absent from the current row is skipped.
    fn subreport_link_bindings(
        &self,
        links: &[rpt_model::SubreportLink],
    ) -> (Parameters, Vec<FieldFilter>) {
        let mut params = self.dataset.params.clone();
        let mut filters = Vec::new();
        for link in links {
            let Some(value) = self
                .current_row
                .and_then(|r| r.get(strip_braces(&link.main_report_field)))
            else {
                continue;
            };
            match &link.linked_parameter {
                Some(param) => {
                    params.insert(normalize_param_name(param), value.clone());
                }
                None => filters.push(FieldFilter {
                    field: strip_braces(&link.subreport_field).to_string(),
                    value: value.clone(),
                }),
            }
        }
        (params, filters)
    }

    /// Emit an on-demand subreport's placeholder caption (its name) in the placeholder box. The engine
    /// renders an on-demand subreport as a click-to-expand link that a static HTML/PDF export shows as
    /// its caption, so we emit just that text — the subreport is never executed (no query, no dataset,
    /// no Shared-variable side effects), matching the engine.
    fn emit_subreport_caption(
        &mut self,
        sr_obj: &SubreportObject,
        obj: &ReportObject,
        rect: Rect,
        section_name: &str,
        instance: u32,
    ) {
        let font = rpt_pages::FontSpec::default();
        let ascent = Twips(self.text_layout.ascent_twips(&font) as i32);
        let line_height = Twips(self.text_layout.line_height_twips(&font) as i32);
        let advance = Twips(self.text_layout.width_twips(&sr_obj.subreport_name, &font) as i32);
        self.cur.push(DrawOp::Text(TextRun {
            bounds: rect,
            text: sr_obj.subreport_name.clone(),
            font,
            color: DEFAULT_BORDER,
            align: crate::align_of(obj.format.horizontal_alignment),
            rotation: 0.0,
            metrics: Some(rpt_pages::TextMetrics {
                advance,
                ascent,
                line_height,
            }),
            source: Some(
                ObjectRef::new(section_name, ObjectKind::Subreport)
                    .named(&obj.name)
                    .with_instance(instance),
            ),
        }));
    }

    /// Emit ONE diagnostic naming every table bound to a raw SQL **Command** / stored proc — their
    /// SQL is author-written for a specific database and passed through **verbatim** (never
    /// translated), so the report can only be rendered with live data against that database. These
    /// often number in the dozens, so they are aggregated into a single line (a per-table warning
    /// each would bury the log). Names the authored driver(s) from the connection's `Database_DLL`
    /// / `QE_DatabaseType` attribute when present.
    pub(crate) fn note_command_tables(&self) {
        let mut tables: Vec<&str> = Vec::new();
        let mut drivers: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for t in &self.report.database.tables {
            if t.command_text
                .as_deref()
                .is_none_or(|c| c.trim().is_empty())
            {
                continue;
            }
            tables.push(t.alias.as_str());
            let driver = t
                .connection
                .attributes
                .iter()
                .find(|(k, _)| {
                    k.eq_ignore_ascii_case("Database_DLL")
                        || k.eq_ignore_ascii_case("QE_DatabaseType")
                })
                .map(|(_, v)| v.as_str())
                .filter(|v| !v.is_empty())
                .unwrap_or("a specific database");
            drivers.insert(driver);
        }
        if tables.is_empty() {
            return;
        }
        let driver_list = drivers.into_iter().collect::<Vec<_>>().join(", ");
        let msg = format!(
            "{n} table(s) use an untranslatable raw SQL command (authored for {driver_list}); \
             live rendering works only against that database: {list}",
            n = tables.len(),
            list = tables.join(", "),
        );
        push_diag(
            &self.diagnostics,
            Diagnostic::warn(DiagnosticKind::Other, msg),
        );
    }

    /// Draw just the dashed placeholder box (no diagnostic) — used when the caller emits its own,
    /// more specific diagnostic (e.g. a chart that has no group series to plot).
    pub(crate) fn placeholder_box(
        &mut self,
        rect: Rect,
        section_name: &str,
        obj: &ReportObject,
        kind: ObjectKind,
    ) {
        self.cur.push(DrawOp::Rect(RectOp {
            bounds: rect,
            fill: None,
            stroke: Some(Stroke {
                color: PLACEHOLDER_STROKE,
                width: HAIRLINE_W,
                style: LineStyle::Dashed,
            }),
            corner_radius: Twips(0),
            source: Some(ObjectRef::new(section_name, kind).named(&obj.name)),
        }));
    }

    /// Emit a box's drop shadow: two filled bars offset to the bottom-right (in the shadow/border
    /// color), matching how the engine draws it as edge rectangles rather than a CSS shadow.
    fn push_drop_shadow(
        &mut self,
        rect: Rect,
        color: Color,
        section_name: &str,
        obj_name: &str,
        instance: u32,
    ) {
        const T: i32 = 60; // bar thickness (~4px)
        const O: i32 = 30; // offset (~2px)
        let mut bar = |left: i32, top: i32, width: i32, height: i32| {
            self.cur.push(fill_rect(
                Rect {
                    left: Twips(left),
                    top: Twips(top),
                    width: Twips(width),
                    height: Twips(height),
                },
                color,
                Some(
                    ObjectRef::new(section_name, ObjectKind::Box)
                        .named(obj_name)
                        .with_instance(instance),
                ),
            ));
        };
        // Bottom bar, then right bar.
        bar(rect.left.0 + T, rect.bottom().0 + O, rect.width.0, T);
        bar(rect.right().0 + O, rect.top.0 + T, T, rect.height.0);
    }

    /// Draw a text/field object's background fill, emitted *before* its text so it sits behind the
    /// glyphs. No-op when the object has no background colour (the transparent default).
    fn push_object_fill(
        &mut self,
        obj: &ReportObject,
        rect: Rect,
        section_name: &str,
        instance: u32,
    ) {
        let Some(fill) = obj.border.background_color else {
            return;
        };
        self.cur.push(DrawOp::Rect(RectOp {
            bounds: rect,
            fill: Some(fill.into()),
            stroke: None,
            corner_radius: Twips(0),
            source: Some(
                ObjectRef::new(section_name, ObjectKind::Box)
                    .named(&obj.name)
                    .with_instance(instance),
            ),
        }));
    }

    /// Draw a text/field object's border stroke, emitted *after* its text so the frame sits on top.
    /// No-op when every edge is `NoLine`. The fill is drawn separately by [`Self::push_object_fill`].
    fn push_border(&mut self, obj: &ReportObject, rect: Rect, section_name: &str, instance: u32) {
        let b = &obj.border;
        let visible = [b.top, b.bottom, b.left, b.right]
            .iter()
            .any(|s| !matches!(s, RptLineStyle::NoLine));
        if !visible {
            return;
        }
        let color = b.border_color.unwrap_or(DEFAULT_BORDER);
        self.cur.push(DrawOp::Rect(RectOp {
            bounds: rect,
            fill: None,
            stroke: Some(hairline_stroke(color)),
            corner_radius: Twips(0),
            source: Some(
                ObjectRef::new(section_name, ObjectKind::Box)
                    .named(&obj.name)
                    .with_instance(instance),
            ),
        }));
    }

    pub(crate) fn page_rect(&self, b: &Rect, origin_y: i32) -> Rect {
        Rect {
            left: Twips(self.col_offset + b.left.0),
            top: Twips(origin_y + b.top.0),
            width: b.width,
            height: b.height,
        }
    }

    /// The height of a shape that spans from `start` section (its top `box_top` twips below the
    /// section top) down to the bottom of `end` section, summed from the design section layout (the
    /// same canonical section order the decoder resolves `end_section_name` against). `None` when
    /// either section is not found or the end lies above the start. Growth of the intervening bands is
    /// not tracked (the span uses design heights); the static layout is reproduced faithfully.
    fn design_span_height(&self, start: &str, end: &str, box_top: i32) -> Option<i32> {
        let flat: Vec<(&str, i32)> = self
            .report
            .report_definition
            .areas
            .iter()
            .flat_map(|a| &a.sections)
            .map(|s| (s.name.as_str(), s.height.0))
            .collect();
        let s = flat.iter().position(|(n, _)| *n == start)?;
        let e = flat.iter().position(|(n, _)| *n == end)?;
        if e < s {
            return None;
        }
        // From the box top to the bottom of the start section, then every full section down to and
        // including the end section.
        let mut h = flat[s].1 - box_top;
        for (_, height) in &flat[s + 1..=e] {
            h += *height;
        }
        Some(h.max(0))
    }

    /// Extend a spanning box's rect to reach its end section's bottom (design layout). The box never
    /// shrinks below its own decoded height; a span that cannot be resolved leaves the rect unchanged.
    fn span_rect(&self, rect: Rect, start: &str, end: &str, box_top: i32) -> Rect {
        match self.design_span_height(start, end, box_top) {
            Some(h) if h > rect.height.0 => Rect {
                height: Twips(h),
                ..rect
            },
            _ => rect,
        }
    }
}

/// Whether a box/line's decoded `end_section_name` names a *different, later* section than the one
/// it is placed in — the signal that the shape spans across sections. A non-spanning shape carries
/// its own section name (or an empty name), which this rejects.
fn spans_sections(end_section_name: &str, section_name: &str) -> bool {
    !end_section_name.is_empty() && end_section_name != section_name
}

/// The rectangle a picture's raster is drawn into: its authored scaled size (`original × scaling`)
/// placed at the object box's top-left. Falls back to the box when the natural size is unknown
/// (`original_* == 0`, e.g. a picture with no OLE embedding) or the scale is not finite/positive, in
/// which case the raster fills the box. Normally the box already equals `original × scaling`.
fn scaled_picture_rect(p: &PictureObject, box_rect: Rect) -> Rect {
    let (ow, oh) = (p.original_width.0, p.original_height.0);
    if ow <= 0
        || oh <= 0
        || !p.x_scaling.is_finite()
        || !p.y_scaling.is_finite()
        || p.x_scaling <= 0.0
        || p.y_scaling <= 0.0
    {
        return box_rect;
    }
    Rect {
        left: box_rect.left,
        top: box_rect.top,
        width: Twips((ow as f64 * p.x_scaling).round() as i32),
        height: Twips((oh as f64 * p.y_scaling).round() as i32),
    }
}

/// The page rect for wrapped line `index` of a quarter-turn-rotated text object. The text flows along
/// the box's tall axis (each line's `width` is that flow length), and the lines stack as columns of
/// pitch `line_height` across the box. The rect's top-left is the point the backend rotates the run
/// about (`deg` CCW): for `90°` the columns run left→right from the box bottom (text reads up); for
/// `270°` they run right→left from the box top (text reads down) — matching the native engine.
fn rotated_line_rect(rect: Rect, deg: i32, index: i32, line_height: i32, width: Twips) -> Rect {
    let offset = index * line_height;
    let (left, top) = if deg == 90 {
        (rect.left.0 + offset, rect.bottom().0)
    } else {
        (rect.right().0 - offset, rect.top.0)
    };
    Rect {
        left: Twips(left),
        top: Twips(top),
        width,
        height: Twips(line_height),
    }
}

/// A line object's endpoints from its bounding rect: horizontal if wider than tall, else vertical.
fn line_endpoints(rect: Rect) -> (rpt_pages::Point, rpt_pages::Point) {
    use rpt_pages::Point;
    if rect.width.0 >= rect.height.0 {
        let y = rect.top.0 + rect.height.0 / 2;
        (Point::new(rect.left.0, y), Point::new(rect.right().0, y))
    } else {
        let x = rect.left.0 + rect.width.0 / 2;
        (Point::new(x, rect.top.0), Point::new(x, rect.bottom().0))
    }
}

/// Whether a field's value depends on the report's final page count — the `TotalPageCount` and
/// `PageNofM` special fields, whose total is a forward reference resolved after pagination.
fn total_page_dependent(f: &FieldObject) -> bool {
    if f.ref_kind != FieldRefKind::Special {
        return false;
    }
    let key = f.data_source.to_lowercase().replace(['{', '}', ' '], "");
    matches!(key.as_str(), "pagenofm" | "totalpagecount")
}

/// Extend a section-spanning box to the band's grown height. A box whose design bottom reaches at
/// least 80% of the section's design height is treated as a full-section background/frame: when the
/// band grows past its design height (can-grow content), the box's bottom follows, preserving the gap
/// between the box bottom and the band bottom. A partial box, or a band that did not grow, is
/// returned unchanged.
fn extend_section_box(
    rect: Rect,
    design: &Rect,
    band_height: i32,
    section_design_height: i32,
) -> Rect {
    if section_design_height <= 0 || band_height <= section_design_height {
        return rect;
    }
    let design_bottom = design.top.0 + design.height.0;
    if design_bottom * 5 < section_design_height * 4 {
        return rect; // a partial box, not a full-section background
    }
    let bottom_gap = (section_design_height - design_bottom).max(0);
    let mut grown = rect;
    grown.height = Twips((band_height - bottom_gap - design.top.0).max(design.height.0));
    grown
}

/// The line style a line/box border defines, as the first non-`NoLine` edge (top, then bottom, left,
/// right). `None` when every edge is `NoLine` — the caller then falls back to the shape's own style.
fn border_line_style(b: &rpt_model::Border) -> Option<RptLineStyle> {
    [b.top, b.bottom, b.left, b.right]
        .into_iter()
        .find(|s| !matches!(s, RptLineStyle::NoLine))
}

fn stroke_of(style: RptLineStyle, color: Color, thickness: Twips) -> Option<Stroke> {
    let s = match style {
        RptLineStyle::NoLine => return None,
        RptLineStyle::SingleLine => LineStyle::Single,
        RptLineStyle::DoubleLine => LineStyle::Double,
        RptLineStyle::DashLine => LineStyle::Dashed,
        RptLineStyle::DotLine => LineStyle::Dotted,
        _ => LineStyle::Single,
    };
    Some(Stroke {
        color,
        width: Twips(thickness.0.max(HAIRLINE_W.0)),
        style: s,
    })
}

/// Decode a blob field's resolved value into image bytes + sniffed format, when it is a
/// browser-renderable raster. A bytes-capable backend delivers [`Value::Bytes`] directly; a text
/// delivery ([`Value::Str`]) may hold raw bytes in a lossy string or a Postgres `\x` hex-escape,
/// both recovered here. `None` when the bytes aren't a recognised inline image (the caller then
/// draws a placeholder).
fn decode_blob_image(value: Value) -> Option<(ImageFormat, Vec<u8>)> {
    let bytes = match value {
        Value::Bytes(b) => b,
        Value::Str(s) => decode_hex_bytea(&s).unwrap_or_else(|| s.into_bytes()),
        _ => return None,
    };
    let fmt = ImageFormat::sniff(&bytes);
    browser_renderable(fmt).then_some((fmt, bytes))
}

/// Decode a Postgres `bytea` text representation (`\x` followed by hex-digit pairs) to raw bytes;
/// `None` when the string is not in that form.
fn decode_hex_bytea(s: &str) -> Option<Vec<u8>> {
    let hex = s.strip_prefix("\\x")?;
    if hex.is_empty() || hex.len() % 2 != 0 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod blob_tests {
    use super::{decode_blob_image, decode_hex_bytea, Value};
    use rpt_model::ImageFormat;

    // The 8-byte PNG signature is enough for the format sniffer.
    const PNG_SIG: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

    #[test]
    fn decodes_raw_bytes_value() {
        // A bytes-capable backend delivers the blob as `Value::Bytes` directly, byte-for-byte.
        let (fmt, bytes) =
            decode_blob_image(Value::Bytes(PNG_SIG.to_vec())).expect("raw PNG bytes decode");
        assert_eq!(fmt, ImageFormat::Png);
        assert_eq!(bytes, PNG_SIG);
    }

    #[test]
    fn decodes_postgres_hex_bytea() {
        // A text delivery in `\x` + hex form (Postgres bytea::text) recovers the exact bytes.
        let hex: String = PNG_SIG.iter().map(|b| format!("{b:02x}")).collect();
        let s = format!("\\x{hex}");
        assert_eq!(decode_hex_bytea(&s).as_deref(), Some(PNG_SIG));
        let (fmt, bytes) = decode_blob_image(Value::Str(s)).expect("hex bytea decodes to an image");
        assert_eq!(fmt, ImageFormat::Png);
        assert_eq!(bytes, PNG_SIG);
    }

    #[test]
    fn decodes_raw_byte_string() {
        // A text value that isn't in `\x` hex form is taken verbatim, then sniffed. GIF's signature
        // is ASCII, so it survives as a plain string (unlike PNG/JPEG, whose non-ASCII signature
        // bytes can only be delivered losslessly via the hex-bytea form or `Value::Bytes`).
        let (fmt, _) = decode_blob_image(Value::Str("GIF89a raw bytes".into()))
            .expect("raw bytes sniff as GIF");
        assert_eq!(fmt, ImageFormat::Gif);
    }

    #[test]
    fn rejects_non_image_and_malformed_hex() {
        assert!(decode_blob_image(Value::Str("just some text".into())).is_none());
        assert!(decode_blob_image(Value::Null).is_none());
        assert!(decode_hex_bytea("\\xZZ").is_none()); // non-hex digits
        assert!(decode_hex_bytea("\\x123").is_none()); // odd length
    }
}
