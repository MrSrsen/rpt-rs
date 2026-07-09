//! Object placement: turn one resolved [`crate::TextPlan`]/report object into Page-IR draw-ops at its
//! twip position on the current page. This is the per-variant emit (`emit_object`), the section
//! background + object loop (`emit_band_plans`), the box/line/border/shadow primitives, the printable
//! rect mapping (`page_rect`), and the recursive subreport placement (`emit_subreport`). The
//! formatter's pagination path (see [`crate::paginate`]) calls these once a band's position is fixed.

use crate::{browser_renderable, emf, push_diag, translate_op, EmptyRows, Formatter, TextPlan};
use rpt_data::{build_dataset, compile_formulas, DataContext, RunningTotals, SavedDataSource};
use rpt_model::{
    Color, ImageFormat, LineStyle as RptLineStyle, Rect, ReportObject, ReportObjectKind, Section,
    SubreportObject, Twips,
};
use rpt_pages::{
    Diagnostic, DiagnosticKind, DrawOp, ImageOp, LineOp, LineStyle, ObjectKind, ObjectRef, RectOp,
    Stroke, TextRun,
};

impl Formatter<'_> {
    /// Emit a band's section background + objects at `origin_y` (respecting `self.col_offset` for
    /// multi-column). Does not paginate or advance the cursor.
    pub(crate) fn emit_band_plans(
        &mut self,
        section: &Section,
        plans: &[Option<TextPlan>],
        origin_y: i32,
        height: i32,
        ctx: Option<&DataContext>,
    ) {
        if let Some(bg) = section.format.background_color {
            self.cur.push(DrawOp::Rect(RectOp {
                bounds: Rect {
                    left: Twips(self.content_left + self.col_offset),
                    top: Twips(origin_y),
                    width: self.page_size.width,
                    height: Twips(height),
                },
                fill: Some(bg.into()),
                stroke: None,
                corner_radius: Twips(0),
                source: Some(ObjectRef::new(&section.name, ObjectKind::Section)),
            }));
        }
        for (obj, plan) in section.objects.iter().zip(plans) {
            self.emit_object(obj, &section.name, origin_y, plan.as_ref(), ctx);
        }
    }

    fn emit_object(
        &mut self,
        obj: &ReportObject,
        section_name: &str,
        origin_y: i32,
        plan: Option<&TextPlan>,
        ctx: Option<&DataContext>,
    ) {
        // A conditional `EnableSuppress` formula, when present, overrides the static suppress flag.
        let suppressed =
            crate::resolve::cond_bool(&obj.format.condition_formulas, "EnableSuppress", ctx)
                .unwrap_or(obj.format.suppress.value);
        if suppressed {
            return;
        }
        let rect = self.page_rect(&obj.bounds, origin_y);
        // One instance id per placed object: its text runs and its border/fill box share it, so the
        // HTML backend groups a wrapped value and links its adornment by id, not by geometry.
        let instance = self.next_instance_id;
        self.next_instance_id += 1;
        let src = |kind| {
            Some(
                ObjectRef::new(section_name, kind)
                    .named(&obj.name)
                    .with_instance(instance),
            )
        };

        match &obj.kind {
            ReportObjectKind::Field(_) | ReportObjectKind::Text(_) => {
                if let Some(plan) = plan {
                    // Resolved per-font metrics: baseline offset = ascent, line pitch =
                    // line height. Backends place from these instead of a per-backend point-size guess.
                    let ascent = Twips(self.text_layout.ascent_twips(&plan.font) as i32);
                    let line_height = Twips(self.text_layout.line_height_twips(&plan.font) as i32);
                    let lh = line_height.0;
                    // One text run per wrapped line, stacked from the object's top.
                    for (i, line) in plan.lines.iter().enumerate() {
                        let mut line_rect = rect;
                        line_rect.top = Twips(rect.top.0 + i as i32 * lh);
                        if plan.lines.len() > 1 {
                            line_rect.height = Twips(lh);
                        }
                        let advance = Twips(self.text_layout.width_twips(line, &plan.font) as i32);
                        self.cur.push(DrawOp::Text(TextRun {
                            bounds: line_rect,
                            text: line.clone(),
                            font: plan.font.clone(),
                            color: plan.color,
                            align: plan.align,
                            rotation: 0.0,
                            metrics: Some(rpt_pages::TextMetrics {
                                advance,
                                ascent,
                                line_height,
                            }),
                            source: src(plan.kind),
                        }));
                    }
                }
                self.push_border(obj, rect, section_name, instance);
            }
            ReportObjectKind::Box(b) => {
                // Fill/border resolve the per-row conditional-format formulas first (e.g. a color
                // swatch's `BackgroundColor = Color({r},{g},{b})`), then the static colors.
                let conds = &obj.border.condition_formulas;
                let fill = crate::resolve::cond_color(conds, "BackgroundColor", ctx)
                    .or(obj.border.background_color)
                    .or(b.fill_color);
                let border_color = crate::resolve::cond_color(conds, "BorderColor", ctx)
                    .unwrap_or(b.shape.line_color);
                self.cur.push(DrawOp::Rect(RectOp {
                    bounds: rect,
                    fill: fill.map(Into::into),
                    stroke: stroke_of(b.shape.line_style, border_color, b.shape.line_thickness),
                    corner_radius: Twips(b.corner_ellipse_width.0.max(b.corner_ellipse_height.0)),
                    source: src(ObjectKind::Box),
                }));
                // Drop shadow: two filled bars offset to the bottom-right in the border color (the
                // engine draws the shadow as edge rectangles, not a CSS shadow).
                if obj.border.has_drop_shadow {
                    self.push_drop_shadow(rect, border_color, section_name, &obj.name, instance);
                }
            }
            ReportObjectKind::Line(l) => {
                // A horizontal line uses the box top; a vertical line its left. Use the object rect.
                let (from, to) = line_endpoints(rect);
                if let Some(stroke) = stroke_of(
                    l.shape.line_style,
                    l.shape.line_color,
                    l.shape.line_thickness,
                ) {
                    self.cur.push(DrawOp::Line(LineOp {
                        from,
                        to,
                        stroke,
                        source: src(ObjectKind::Line),
                    }));
                }
            }
            ReportObjectKind::Picture(p) => {
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
                    self.cur.push(DrawOp::Image(ImageOp {
                        bounds: rect,
                        image_id: obj.name.clone(),
                        source: src(ObjectKind::Image),
                    }));
                } else if fmt == ImageFormat::Emf {
                    // An EMF is a vector command stream: interpret its records into draw-ops mapped
                    // into the box. A bad/truncated stream falls back to the placeholder image op.
                    match emf::interpret_emf(&p.data, rect, src(ObjectKind::Image)) {
                        Some(ops) => {
                            for op in ops {
                                self.cur.push(op);
                            }
                        }
                        None => {
                            push_diag(
                                &self.diagnostics,
                                Diagnostic::warn(
                                    DiagnosticKind::UnsupportedObject,
                                    "EMF picture could not be interpreted; rendered as a placeholder",
                                )
                                .with_source(&obj.name),
                            );
                            self.cur.push(DrawOp::Image(ImageOp {
                                bounds: rect,
                                image_id: obj.name.clone(),
                                source: src(ObjectKind::Image),
                            }));
                        }
                    }
                } else {
                    // WMF / OLE-embedded / other metafile presentations are not yet interpreted
                    // (separate follow-ups); draw the placeholder image op.
                    self.cur.push(DrawOp::Image(ImageOp {
                        bounds: rect,
                        image_id: obj.name.clone(),
                        source: src(ObjectKind::Image),
                    }));
                }
            }
            ReportObjectKind::BlobField(_) => {
                self.cur.push(DrawOp::Image(ImageOp {
                    bounds: rect,
                    image_id: obj.name.clone(),
                    source: src(ObjectKind::Image),
                }));
            }
            // Charts render as native draw-ops from the group summaries; cross-tabs
            // and unlinked subreports still fall back to a placeholder box carrying identity.
            ReportObjectKind::Chart(c) => self.emit_chart(c, rect, section_name, obj),
            ReportObjectKind::CrossTab(ct) => self.emit_crosstab(ct, rect, section_name, obj),
            ReportObjectKind::Subreport(sr) => self.emit_subreport(sr, rect),
            _ => {}
        }
    }

    /// Render a subreport (a full nested [`rpt_model::Report`]) into the placeholder object's box: lay
    /// it out recursively (sharing our text layout), then translate its first page's draw-ops so the
    /// subreport's printable top-left lands at the box's top-left, clipping content past the box.
    ///
    /// First cut: renders the subreport's own saved data (empty when it has none — static content
    /// still shows) and takes page 1 only, clipping any vertical overflow. Linked (on-demand,
    /// per-row) subreports and multi-page growth are not yet supported.
    fn emit_subreport(&mut self, sr_obj: &SubreportObject, rect: Rect) {
        let Some(sub) = self
            .report
            .subreports
            .iter()
            .find(|s| s.name == sr_obj.subreport_name)
        else {
            return;
        };
        let sub_report = &sub.report;
        // Prefer live rows from the scope-data provider; fall back to the subreport's
        // saved data, then to empty. `live` is held so the boxed source outlives `build_dataset`.
        let live = self.scope_data.and_then(|p| p.rows_for(sub_report));
        let dataset = match (&live, &sub_report.saved_data) {
            (Some(src), _) => build_dataset(src.as_ref(), &sub_report.data_definition),
            (None, Some(saved)) => build_dataset(
                &SavedDataSource::from_report(saved, sub_report),
                &sub_report.data_definition,
            ),
            (None, None) => build_dataset(&EmptyRows, &sub_report.data_definition),
        };
        let formulas = compile_formulas(&sub_report.data_definition);
        // A subreport gets its own Global variable store (its running totals / global counters reset
        // per subreport, matching the engine) but **shares the parent's `Shared` scope** — a `Shared`
        // variable set in the main report is visible in the subreport and vice-versa.
        let sub_state = self.state_vars.child();
        let sub_running = RunningTotals::from_data_def(&sub_report.data_definition);
        let sub_scheduled = crate::run_schedule(sub_report, &dataset, &formulas, &sub_state);
        let sub_doc = Formatter::new(
            sub_report,
            &dataset,
            &formulas,
            self.text_layout,
            &sub_state,
            &sub_running,
            &sub_scheduled,
            self.scope_data,
            self.locale,
        )
        .run();
        // Lift the subreport's own diagnostics into the parent document, tagged with its name.
        for mut d in sub_doc.diagnostics {
            d.source = Some(match d.source {
                Some(s) => format!("{}/{s}", sr_obj.subreport_name),
                None => sr_obj.subreport_name.clone(),
            });
            push_diag(&self.diagnostics, d);
        }
        // Lift the subreport's image assets into the parent document (its pictures render in the box).
        self.assets.borrow_mut().extend(sub_doc.assets);

        // Map the subreport's printable origin (its margins) to the placeholder box's top-left, and
        // lift its 0-based instance ids into the parent's id space so they don't collide.
        let dx = rect.left.0 - sub_report.print_options.margins.left.0;
        let dy = rect.top.0 - sub_report.print_options.margins.top.0;
        let box_bottom = rect.bottom().0;
        let id_offset = self.next_instance_id;
        let mut max_instance: Option<u32> = None;
        if let Some(page) = sub_doc.pages.first() {
            for op in &page.ops {
                let moved = translate_op(op, dx, dy, id_offset);
                if let Some(inst) = moved.source().and_then(|s| s.instance) {
                    max_instance = Some(max_instance.map_or(inst, |m| m.max(inst)));
                }
                // Clip content that overflows the placeholder box vertically.
                if moved.bounds().top.0 < box_bottom {
                    self.cur.push(moved);
                }
            }
        }
        // Advance past the merged subreport ids so the parent's next placement id is unique.
        if let Some(m) = max_instance {
            self.next_instance_id = m + 1;
        }
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
                color: Color {
                    a: 255,
                    r: 136,
                    g: 136,
                    b: 136,
                },
                width: Twips(10),
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
            self.cur.push(DrawOp::Rect(RectOp {
                bounds: Rect {
                    left: Twips(left),
                    top: Twips(top),
                    width: Twips(width),
                    height: Twips(height),
                },
                fill: Some(color.into()),
                stroke: None,
                corner_radius: Twips(0),
                source: Some(
                    ObjectRef::new(section_name, ObjectKind::Box)
                        .named(obj_name)
                        .with_instance(instance),
                ),
            }));
        };
        // Bottom bar, then right bar.
        bar(rect.left.0 + T, rect.bottom().0 + O, rect.width.0, T);
        bar(rect.right().0 + O, rect.top.0 + T, T, rect.height.0);
    }

    fn push_border(&mut self, obj: &ReportObject, rect: Rect, section_name: &str, instance: u32) {
        let b = &obj.border;
        let visible = [b.top, b.bottom, b.left, b.right]
            .iter()
            .any(|s| !matches!(s, RptLineStyle::NoLine));
        if !visible {
            return;
        }
        let color = b.border_color.unwrap_or(Color {
            a: 255,
            r: 0,
            g: 0,
            b: 0,
        });
        self.cur.push(DrawOp::Rect(RectOp {
            bounds: rect,
            fill: b.background_color.map(Into::into),
            stroke: Some(Stroke {
                color,
                width: Twips(10),
                style: LineStyle::Single,
            }),
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
            left: Twips(self.content_left + self.col_offset + b.left.0),
            top: Twips(origin_y + b.top.0),
            width: b.width,
            height: b.height,
        }
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
        width: Twips(thickness.0.max(10)),
        style: s,
    })
}
