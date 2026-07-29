//! The formatter's chart-object emit path: dispatch a decoded [`rpt_model::ChartObject`] to the
//! right renderer, push the resulting draw-ops onto the current page, and record any fidelity
//! diagnostic. The series/point data each renderer plots is built by [`crate::aggregate`]; this
//! module owns only the dispatch + emit (so the formatter calls it, not hosts it).

use crate::aggregate;
use crate::chart;
use crate::{push_diag, Formatter};
use rpt_formula::token::{brace_groups, last_segment, strip_braces};
use rpt_model::Rect;
use rpt_model::ReportObject;
use rpt_model::Twips;
use rpt_pages::{Diagnostic, DiagnosticKind, DrawOp, ObjectKind, ObjectRef};

impl Formatter<'_> {
    /// Render a chart object as native draw-ops from the group summaries: one bar per
    /// group, height = the group's summary of the charted field. Falls back to the placeholder box +
    /// an unsupported diagnostic when there is no group series to plot (a detail/cross-tab chart, or
    /// no matching summary).
    pub(crate) fn emit_chart(
        &mut self,
        chart: &rpt_model::ChartObject,
        rect: Rect,
        section_name: &str,
        obj: &ReportObject,
    ) {
        // Chart text scales with the chart's own height, so take it before a band reduces `rect`.
        let style = chart::ChartStyle {
            def: &chart.definition,
            height: rect.height,
        };
        // The inherently 3-D families (Riser3D/Surface3D) take the perspective riser path, which draws
        // multiple data series as z-rows over its own frame.
        if chart.definition.is_3d() {
            self.emit_chart_3d(chart, rect, section_name, obj);
            return;
        }
        // A 2-D area chart with the depth-effect bit set (`graph_subtype & 0x02`) is drawn as an
        // extruded ribbon receding into the scene, not the flat 2-D area.
        if chart.definition.graph_type == rpt_model::ChartGraphType::Area
            && chart.definition.has_depth_effect()
        {
            self.emit_chart_area3d(chart, rect, section_name, obj);
            return;
        }
        // A 2-D bar chart bound to more than one data series takes the separate multi-series path
        // (clustered/stacked/percent). Every other chart — including all single-series ones — keeps
        // the single-series path below, byte-for-byte unchanged.
        if matches!(chart.definition.graph_type, rpt_model::ChartGraphType::Bar)
            && chart.data_refs.len() > 1
        {
            self.emit_chart_multi(chart, rect, section_name, obj);
            return;
        }
        // Scatter/bubble/stock/histogram bind their data differently from the (category → value)
        // series the other 2-D families share (XY point pairs / XY + a size value / per-category
        // hi-lo ranges / a binned value distribution), so each has its own builder + renderer rather
        // than the shared path below.
        {
            use rpt_model::ChartGraphType as Gt;
            match chart.definition.graph_type {
                Gt::Scatter => {
                    self.emit_chart_scatter(chart, rect, section_name, obj);
                    return;
                }
                Gt::Bubble => {
                    self.emit_chart_bubble(chart, rect, section_name, obj);
                    return;
                }
                Gt::Stock => {
                    self.emit_chart_stock(chart, rect, section_name, obj);
                    return;
                }
                Gt::Histogram => {
                    self.emit_chart_histogram(chart, rect, section_name, obj);
                    return;
                }
                Gt::Gantt => {
                    self.emit_chart_gantt(chart, rect, section_name, obj);
                    return;
                }
                _ => {}
            }
        }
        let series = aggregate::chart_series(self.dataset, &self.locale, chart);
        if series.is_empty() {
            self.chart_empty(
                rect,
                section_name,
                obj,
                "chart has no group series to plot; rendered as an empty placeholder",
            );
            return;
        }
        let title = if !chart.definition.title.is_empty() {
            chart.definition.title.clone()
        } else {
            chart.definition.group_axis_title.clone()
        };
        // Axis-chart families draw the two decoded axis titles around the plot (Y rotated, X below),
        // so their top title is the chart title alone — the group-axis title moves to the X position
        // rather than doubling as the top title (the non-axis families keep the `title` fallback).
        let axis_titles = chart::AxisTitles {
            value: &chart.definition.data_axis_title,
            category: &chart.definition.group_axis_title,
        };
        // Dispatch on the decoded visual type. Bar and Line have renderers; other types fall back to
        // a bar chart with a type-specific diagnostic rather than silently drawing the wrong shape.
        use rpt_model::ChartGraphType as Gt;
        // An area/line/stock/radar chart draws its whole series in one color, so the engine legends
        // the series itself — one boxed entry naming it — rather than listing categories. The
        // per-category-colored families (bar/pie/doughnut/funnel/gauge/…) keep their category list.
        let kind = legend_kind(chart);
        // Pie/doughnut legends match their per-slice fills; the axis families cycle the base palette.
        let per_slice = matches!(chart.definition.graph_type, Gt::Pie | Gt::Doughnut);
        // The axis families draw the chart title alone on top (their axis titles go around the plot);
        // the proportional families draw the `title` fallback to the group-axis title.
        let axis_top = chart.definition.title.as_str();
        let is_axis_family = matches!(
            chart.definition.graph_type,
            Gt::Bar | Gt::Line | Gt::Area | Gt::NumericAxis
        );
        // Reserve the subtitle band (under the title) and footnote band (at the bottom) for every
        // family, drawing those decoded text elements centrally. With neither set (the common case),
        // this is a no-op and the per-type renderer draws its own title unchanged. When present, the
        // renderer is handed an empty title (drawn centrally above the subtitle) and the reduced rect.
        let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(&obj.name));
        let top_title = if is_axis_family {
            axis_top
        } else {
            title.as_str()
        };
        let (caption_ops, chart_area, render_title, render_axis_top): (
            Vec<DrawOp>,
            Rect,
            &str,
            &str,
        ) = match chart_captions(
            style,
            rect,
            top_title,
            &chart.definition.subtitle,
            &chart.definition.footnote,
            &src,
        ) {
            Some((ops, body)) => (ops, body, "", ""),
            None => (Vec::new(), rect, title.as_str(), axis_top),
        };
        // Reserve a legend band and draw the chart body into the reduced rect, honouring the decoded
        // legend visibility + position (`0x0121` `+0x410`). A hidden or suppressed
        // legend gives the whole rect to the chart body.
        let (legend_ops, body) = resolve_legend(
            style,
            chart_area,
            kind,
            &series,
            per_slice,
            section_name,
            &obj.name,
        );
        // Per-point data-value labels are drawn only when the report's decoded "show value" flag is
        // set (`0x0121` `+0x4a8` bit1); category labels and axes always draw.
        let show_labels = chart.definition.data_labels_show_value;
        // The shared renderer context. The proportional families draw `render_title` on top; the axis
        // families draw `render_axis_top` (their axis titles go around the plot), so those take a
        // context with the axis-top title substituted.
        let cx = chart::ChartCtx {
            style,
            rect: body,
            title: render_title,
            axis_titles,
            section_name,
            obj_name: &obj.name,
            show_labels,
        };
        let axis_cx = chart::ChartCtx {
            title: render_axis_top,
            ..cx
        };
        // Every 2-D type dispatches on its shape (bar/line/area/pie); unknown 2-D types fall back to
        // bars.
        let mut ops = match chart.definition.graph_type {
            Gt::Bar => chart::bar_chart(&axis_cx, &series),
            Gt::Line => chart::line_chart(&axis_cx, &series),
            Gt::Area => chart::area_chart(&axis_cx, &series),
            Gt::Pie => chart::pie_chart(&cx, &series, chart.definition.has_depth_effect()),
            Gt::Doughnut => chart::doughnut_chart(&cx, &series),
            Gt::Radar => chart::radar_chart(&cx, &series),
            Gt::Funnel => chart::funnel_chart(&cx, &series),
            Gt::Gauge => chart::gauge_chart(&cx, &series),
            Gt::NumericAxis => chart::numeric_axis_chart(&axis_cx, &series),
            // Any type without a dedicated renderer falls back to a bar chart; this is the single
            // source of truth for "unsupported", so the diagnostic is emitted right here.
            _ => {
                push_diag(
                    &self.diagnostics,
                    Diagnostic::warn(
                        DiagnosticKind::UnsupportedObject,
                        format!(
                            "chart type {:?} is not yet supported; rendered as a bar chart",
                            chart.definition.graph_type
                        ),
                    )
                    .with_source(&obj.name),
                );
                chart::bar_chart(&axis_cx, &series)
            }
        };
        ops.extend(legend_ops);
        self.cur.extend(caption_ops);
        self.cur.extend(ops);
    }

    /// Render a 3-D riser chart: categories on X, each data binding a z-row receding into the scene,
    /// projected with the native perspective transform. Draws no legend, whatever the report stores.
    /// Records the view-angle-approximation diagnostic for a non-default preset — the preset is
    /// decoded, but most presets' concrete angles are approximate.
    fn emit_chart_3d(
        &mut self,
        chart: &rpt_model::ChartObject,
        rect: Rect,
        section_name: &str,
        obj: &ReportObject,
    ) {
        // Chart text scales with the chart's own height, so take it before a band reduces `rect`.
        let style = chart::ChartStyle {
            def: &chart.definition,
            height: rect.height,
        };
        let (categories, series) = aggregate::chart_series_multi(self.dataset, &self.locale, chart);
        if categories.is_empty() || series.is_empty() {
            self.chart_empty(
                rect,
                section_name,
                obj,
                "chart has no group series to plot; rendered as an empty placeholder",
            );
            return;
        }
        let title = if !chart.definition.title.is_empty() {
            chart.definition.title.clone()
        } else {
            chart.definition.group_axis_title.clone()
        };
        let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(&obj.name));
        let (caption_ops, rect, title) = reserve_captions(style, rect, &title, &src);
        // A 3-D family draws no legend and gives the freed width to the room, whatever the report's
        // legend flag says: its series are the depth axis, already labelled on the floor beside the
        // series-axis title, so there is nothing left for a legend to name. The rule is the engine's
        // behaviour, not a stored fact — the flat families still honour the same flag.
        let (legend_ops, body) = resolve_legend(
            style,
            rect,
            LegendKind::None,
            &[],
            false,
            section_name,
            &obj.name,
        );
        let show_labels = chart.definition.data_labels_show_value;
        // Riser3D draws shaded boxes; Surface3D draws a flat-shaded top-ribbon mesh over the same
        // scenery and perspective. Both recede their data series along Z.
        let view_angle = chart.definition.view_angle;
        let cx = chart::ChartCtx {
            style,
            rect: body,
            title,
            axis_titles: chart::AxisTitles::default(),
            section_name,
            obj_name: &obj.name,
            show_labels,
        };
        let mut ops = if chart.definition.graph_type == rpt_model::ChartGraphType::Surface3D {
            chart::chart3d::surface_3d(&cx, &categories, &series, view_angle)
        } else {
            chart::chart3d::riser_3d(&cx, &categories, &series, view_angle)
        };
        if view_angle != rpt_model::ChartViewAngle::Standard {
            push_diag(
                &self.diagnostics,
                Diagnostic::warn(
                    DiagnosticKind::UnsupportedObject,
                    "3-D chart uses a non-default view-angle preset; rendered at an approximated angle",
                )
                .with_source(&obj.name),
            );
        }
        ops.extend(legend_ops);
        self.cur.extend(caption_ops);
        self.cur.extend(ops);
    }

    /// Render a depth-effect area chart: the flat 2-D area frame with each series' ribbon given a
    /// shallow cast solid, routed here from the flat area path when the Area family's depth-effect bit
    /// is set. It is not a 3-D scene — the engine draws no room and no perspective for it — so it
    /// keeps the area family's axis titles and legend and records no view-angle diagnostic.
    fn emit_chart_area3d(
        &mut self,
        chart: &rpt_model::ChartObject,
        rect: Rect,
        section_name: &str,
        obj: &ReportObject,
    ) {
        // Chart text scales with the chart's own height, so take it before a band reduces `rect`.
        let style = chart::ChartStyle {
            def: &chart.definition,
            height: rect.height,
        };
        let (categories, series) = aggregate::chart_series_multi(self.dataset, &self.locale, chart);
        if categories.is_empty() || series.is_empty() {
            self.chart_empty(
                rect,
                section_name,
                obj,
                "chart has no group series to plot; rendered as an empty placeholder",
            );
            return;
        }
        // An axis family: the chart title alone goes on top, the two decoded axis titles around the
        // plot.
        let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(&obj.name));
        let (caption_ops, rect, title) =
            reserve_captions(style, rect, &chart.definition.title, &src);
        let legend_series = multi_legend_series(&categories, &series);
        // The cast area is still the area family: a single series takes the engine's one-entry
        // series legend, and only a genuinely multi-series chart lists its series.
        let kind = if series.len() == 1 {
            legend_kind(chart)
        } else {
            LegendKind::Categories
        };
        let (legend_ops, body) = resolve_legend(
            style,
            rect,
            kind,
            &legend_series,
            false,
            section_name,
            &obj.name,
        );
        let show_labels = chart.definition.data_labels_show_value;
        let cx = chart::ChartCtx {
            style,
            rect: body,
            title,
            axis_titles: chart::AxisTitles {
                value: &chart.definition.data_axis_title,
                category: &chart.definition.group_axis_title,
            },
            section_name,
            obj_name: &obj.name,
            show_labels,
        };
        let mut ops = chart::chart3d::area_3d(&cx, &categories, &series);
        ops.extend(legend_ops);
        self.cur.extend(caption_ops);
        self.cur.extend(ops);
    }

    /// Render a multi-series bar chart: one riser series per data binding, arranged clustered/stacked/
    /// percent per [`rpt_model::ChartDefinition::arrangement`]. The legend lists the series names (not
    /// the categories), and the chart body draws into the reduced rect the legend leaves.
    fn emit_chart_multi(
        &mut self,
        chart: &rpt_model::ChartObject,
        rect: Rect,
        section_name: &str,
        obj: &ReportObject,
    ) {
        // Chart text scales with the chart's own height, so take it before a band reduces `rect`.
        let style = chart::ChartStyle {
            def: &chart.definition,
            height: rect.height,
        };
        let (categories, series) = aggregate::chart_series_multi(self.dataset, &self.locale, chart);
        if categories.is_empty() || series.is_empty() {
            self.chart_empty(
                rect,
                section_name,
                obj,
                "chart has no group series to plot; rendered as an empty placeholder",
            );
            return;
        }
        // The chart title alone tops the plot; the group-axis title moves to the X-axis position.
        let axis_titles = chart::AxisTitles {
            value: &chart.definition.data_axis_title,
            category: &chart.definition.group_axis_title,
        };
        let title = chart.definition.title.clone();
        let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(&obj.name));
        let (caption_ops, rect, title) = reserve_captions(style, rect, &title, &src);
        let series_names: Vec<String> = series.iter().map(|(n, _)| n.clone()).collect();
        // The legend entries are the series names (each a distinct palette color), so compose it from
        // a synthetic series list carrying those labels.
        let legend_series: Vec<(String, f64)> =
            series_names.iter().map(|n| (n.clone(), 0.0)).collect();
        let (legend_ops, body) = resolve_legend(
            style,
            rect,
            LegendKind::Categories,
            &legend_series,
            false,
            section_name,
            &obj.name,
        );
        let show_labels = chart.definition.data_labels_show_value;
        // Transpose the series-major values into the category-major layout the renderer places from.
        let values: Vec<Vec<f64>> = (0..categories.len())
            .map(|ci| series.iter().map(|(_, vals)| vals[ci]).collect())
            .collect();
        let cx = chart::ChartCtx {
            style,
            rect: body,
            title,
            axis_titles,
            section_name,
            obj_name: &obj.name,
            show_labels,
        };
        let mut ops = chart::bar_chart_multi(
            &cx,
            &categories,
            &series_names,
            &values,
            chart.definition.arrangement(),
        );
        ops.extend(legend_ops);
        self.cur.extend(caption_ops);
        self.cur.extend(ops);
    }

    /// Render an XY scatter chart: a marker at each detail row's `(x, y)` over two numeric axes,
    /// where `x` is the first data binding and `y` the second. Falls back to the placeholder + a
    /// diagnostic when the chart lacks two value bindings or has no plottable points.
    fn emit_chart_scatter(
        &mut self,
        chart: &rpt_model::ChartObject,
        rect: Rect,
        section_name: &str,
        obj: &ReportObject,
    ) {
        // Chart text scales with the chart's own height, so take it before a band reduces `rect`.
        let style = chart::ChartStyle {
            def: &chart.definition,
            height: rect.height,
        };
        let (Some(x_ref), Some(y_ref)) = (chart.data_refs.first(), chart.data_refs.get(1)) else {
            self.chart_empty(
                rect,
                section_name,
                obj,
                "scatter chart needs two value bindings",
            );
            return;
        };
        // The data bindings are group-scoped summaries (e.g. `Sum({weight}, {@Group})`): plot one
        // (x, y) point per category group, x/y = each binding's per-group value. Falls back to one
        // point per detail row (formula-aware) for an ungrouped point scatter.
        let (xy, _sizes) = self.chart_xy_points(chart, &[x_ref, y_ref]);
        let points: Vec<(f64, f64)> = xy.iter().map(|p| (p[0], p[1])).collect();
        if points.is_empty() {
            self.chart_empty(
                rect,
                section_name,
                obj,
                "scatter chart has no plottable points",
            );
            return;
        }
        // The Y axis is the "show value" data axis; the X axis is the group-axis binding.
        let axis_titles = chart::AxisTitles {
            value: &chart.definition.data_axis_title,
            category: &chart.definition.group_axis_title,
        };
        let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(&obj.name));
        let (caption_ops, rect, title) =
            reserve_captions(style, rect, &chart.definition.title, &src);
        let cx = chart::ChartCtx {
            style,
            rect,
            title,
            axis_titles,
            section_name,
            obj_name: &obj.name,
            show_labels: chart.definition.data_labels_show_value,
        };
        let ops = chart::scatter_chart(&cx, &points, None);
        self.cur.extend(caption_ops);
        self.cur.extend(ops);
    }

    /// Render a bubble chart: an XY scatter whose third value binding sizes each marker (a filled
    /// circle, area ∝ value). Needs three value bindings (x, y, size); with only two it falls back to
    /// a plain scatter, and with fewer the scatter path's own "needs two value bindings" diagnostic
    /// fires — the same diagnostic style as the scatter path.
    fn emit_chart_bubble(
        &mut self,
        chart: &rpt_model::ChartObject,
        rect: Rect,
        section_name: &str,
        obj: &ReportObject,
    ) {
        // Chart text scales with the chart's own height, so take it before a band reduces `rect`.
        let style = chart::ChartStyle {
            def: &chart.definition,
            height: rect.height,
        };
        let (Some(x_ref), Some(y_ref), Some(size_ref)) = (
            chart.data_refs.first(),
            chart.data_refs.get(1),
            chart.data_refs.get(2),
        ) else {
            // Fewer than three bindings: degrade to a plain scatter (which itself handles the
            // two-binding case and the empty diagnostic).
            self.emit_chart_scatter(chart, rect, section_name, obj);
            return;
        };
        // Three group-scoped bindings (x, y, size): one bubble per category group, area ∝ the size
        // binding's per-group value. Falls back to one bubble per detail row (formula-aware).
        let (xyz, sizes) = self.chart_xy_points(chart, &[x_ref, y_ref, size_ref]);
        let points: Vec<(f64, f64)> = xyz.iter().map(|p| (p[0], p[1])).collect();
        if points.is_empty() {
            self.chart_empty(
                rect,
                section_name,
                obj,
                "bubble chart has no plottable points",
            );
            return;
        }
        let axis_titles = chart::AxisTitles {
            value: &chart.definition.data_axis_title,
            category: &chart.definition.group_axis_title,
        };
        let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(&obj.name));
        let (caption_ops, rect, title) =
            reserve_captions(style, rect, &chart.definition.title, &src);
        let cx = chart::ChartCtx {
            style,
            rect,
            title,
            axis_titles,
            section_name,
            obj_name: &obj.name,
            show_labels: chart.definition.data_labels_show_value,
        };
        let ops = chart::scatter_chart(&cx, &points, Some(&sizes));
        self.cur.extend(caption_ops);
        self.cur.extend(ops);
    }

    /// Build the XY(Z) points for a scatter/bubble chart from `refs` (two bindings for scatter, three
    /// for bubble). Scatter/bubble bindings are group-scoped summaries, so the primary source is one
    /// point per category group — each binding's per-group value (a formula binding resolves through
    /// its precomputed group summary). An ungrouped chart falls back to one point per detail row,
    /// evaluating each binding (formula-aware) in the row context. Returns the points as `[x, y, z]`
    /// (z = 0 with two bindings) plus the z (size) values aligned with the points for the bubble path.
    fn chart_xy_points(
        &self,
        chart: &rpt_model::ChartObject,
        refs: &[&str],
    ) -> (Vec<[f64; 3]>, Vec<f64>) {
        let want_size = refs.len() >= 3;
        // Per-group: each binding's value in every category group.
        let (_cats, series) = aggregate::chart_series_multi(self.dataset, &self.locale, chart);
        if series.len() >= refs.len() {
            let n = series.iter().map(|(_, v)| v.len()).min().unwrap_or(0);
            if n > 0 {
                let mut pts = Vec::with_capacity(n);
                let mut sizes = Vec::with_capacity(n);
                for i in 0..n {
                    let z = if want_size { series[2].1[i] } else { 0.0 };
                    pts.push([series[0].1[i], series[1].1[i], z]);
                    sizes.push(z);
                }
                return (pts, sizes);
            }
        }
        // Fallback: one point per detail row, each binding evaluated (formula-aware) in the row.
        let mut pts = Vec::new();
        let mut sizes = Vec::new();
        for row in self.dataset.iter_detail_rows() {
            let ctx = rpt_data::DataContext::new(row, self.formulas);
            let (Some(x), Some(y)) = (
                crate::resolve::eval_field_ref(refs[0], &ctx).as_number(),
                crate::resolve::eval_field_ref(refs[1], &ctx).as_number(),
            ) else {
                continue;
            };
            let z = if want_size {
                crate::resolve::eval_field_ref(refs[2], &ctx)
                    .as_number()
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            pts.push([x, y, z]);
            sizes.push(z);
        }
        (pts, sizes)
    }

    /// Render a stock chart: a vertical hi-lo bar per category (its low/high the category's minimum
    /// and maximum of the bound value fields), with open/close ticks for the OHLC subtype. Falls
    /// back to the placeholder + a diagnostic when there is no category series.
    fn emit_chart_stock(
        &mut self,
        chart: &rpt_model::ChartObject,
        rect: Rect,
        section_name: &str,
        obj: &ReportObject,
    ) {
        // Chart text scales with the chart's own height, so take it before a band reduces `rect`.
        let style = chart::ChartStyle {
            def: &chart.definition,
            height: rect.height,
        };
        let points = aggregate::chart_stock_series(self.dataset, &self.locale, chart);
        if points.is_empty() {
            self.chart_empty(
                rect,
                section_name,
                obj,
                "stock chart has no category series to plot",
            );
            return;
        }
        let axis_titles = chart::AxisTitles {
            value: &chart.definition.data_axis_title,
            category: &chart.definition.group_axis_title,
        };
        let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(&obj.name));
        let (caption_ops, rect, title) =
            reserve_captions(style, rect, &chart.definition.title, &src);
        // A stock chart draws its hi-lo bars in one color, so it takes the single-entry series
        // legend rather than a category list.
        let (legend_ops, body) = resolve_legend(
            style,
            rect,
            legend_kind(chart),
            &[],
            false,
            section_name,
            &obj.name,
        );
        let cx = chart::ChartCtx {
            style,
            rect: body,
            title,
            axis_titles,
            section_name,
            obj_name: &obj.name,
            show_labels: chart.definition.data_labels_show_value,
        };
        let ops = chart::stock_chart(&cx, &points);
        self.cur.extend(caption_ops);
        self.cur.extend(legend_ops);
        self.cur.extend(ops);
    }

    /// Render a histogram: the frequency distribution of the first value binding, binned into
    /// equal-width ranges. Falls back to the placeholder + a diagnostic when the chart has no value
    /// binding or no values to bin.
    fn emit_chart_histogram(
        &mut self,
        chart: &rpt_model::ChartObject,
        rect: Rect,
        section_name: &str,
        obj: &ReportObject,
    ) {
        // Chart text scales with the chart's own height, so take it before a band reduces `rect`.
        let style = chart::ChartStyle {
            def: &chart.definition,
            height: rect.height,
        };
        let Some(field) = chart.data_refs.first().map(|r| aggregate::inner_field(r)) else {
            self.chart_empty(
                rect,
                section_name,
                obj,
                "histogram chart has no value binding",
            );
            return;
        };
        let values: Vec<f64> = self
            .dataset
            .iter_detail_rows()
            .iter()
            .filter_map(|r| r.get(&field).and_then(|v| v.as_number()))
            .collect();
        if values.is_empty() {
            self.chart_empty(
                rect,
                section_name,
                obj,
                "histogram chart has no values to bin",
            );
            return;
        }
        // The category axis is the value distribution; the value axis is the bin frequency.
        let axis_titles = chart::AxisTitles {
            value: "",
            category: &chart.definition.group_axis_title,
        };
        // Seven bins matches the native engine's default binning for this distribution.
        const BINS: usize = 7;
        let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(&obj.name));
        let (caption_ops, rect, title) =
            reserve_captions(style, rect, &chart.definition.title, &src);
        let cx = chart::ChartCtx {
            style,
            rect,
            title,
            axis_titles,
            section_name,
            obj_name: &obj.name,
            show_labels: chart.definition.data_labels_show_value,
        };
        let ops = chart::histogram_chart(&cx, &values, BINS);
        self.cur.extend(caption_ops);
        self.cur.extend(ops);
    }

    /// Render a Gantt chart: one horizontal time bar per detail record, spanning its start→end date
    /// on a shared date X axis, records stacked top-to-bottom. Binds two date fields
    /// (start, end) — this is a per-record chart, not a group summary — so it bypasses the group-series
    /// path. Falls back to the placeholder + a diagnostic when there is no start/end binding or no
    /// datable rows.
    fn emit_chart_gantt(
        &mut self,
        chart: &rpt_model::ChartObject,
        rect: Rect,
        section_name: &str,
        obj: &ReportObject,
    ) {
        // Chart text scales with the chart's own height, so take it before a band reduces `rect`.
        let style = chart::ChartStyle {
            def: &chart.definition,
            height: rect.height,
        };
        let (Some(start_ref), Some(end_ref)) = (chart.data_refs.first(), chart.data_refs.get(1))
        else {
            self.chart_empty(
                rect,
                section_name,
                obj,
                "gantt chart needs a start-date and an end-date binding",
            );
            return;
        };
        let mut bars = aggregate::chart_gantt_series(
            self.dataset,
            &self.locale,
            chart,
            &aggregate::inner_field(start_ref),
            &aggregate::inner_field(end_ref),
        );
        if bars.is_empty() {
            self.chart_empty(
                rect,
                section_name,
                obj,
                "gantt chart has no datable records to plot",
            );
            return;
        }
        // The engine plots every datable record, so we do too (row-label thinning keeps dense charts
        // legible). A high defensive guard only trims a pathological detail set, whose sub-pixel bars
        // would otherwise explode the op count; it is far above any realistic gantt. Note it once.
        const MAX_ROWS: usize = 2000;
        if bars.len() > MAX_ROWS {
            let total = bars.len();
            bars.truncate(MAX_ROWS);
            push_diag(
                &self.diagnostics,
                Diagnostic::warn(
                    DiagnosticKind::UnsupportedObject,
                    format!("gantt chart capped at {MAX_ROWS} of {total} records"),
                )
                .with_source(&obj.name),
            );
        }
        let axis_titles = chart::AxisTitles {
            value: "",
            category: &chart.definition.group_axis_title,
        };
        let src = || Some(ObjectRef::new(section_name, ObjectKind::Chart).named(&obj.name));
        let (caption_ops, rect, title) =
            reserve_captions(style, rect, &chart.definition.title, &src);
        let cx = chart::ChartCtx {
            style,
            rect,
            title,
            axis_titles,
            section_name,
            obj_name: &obj.name,
            show_labels: chart.definition.data_labels_show_value,
        };
        let ops = chart::gantt_chart(&cx, &bars);
        self.cur.extend(caption_ops);
        self.cur.extend(ops);
    }

    /// Emit the placeholder box plus an `UnsupportedObject` diagnostic carrying `msg` — the shared
    /// "this chart had nothing plottable" path for the per-type chart renderers.
    fn chart_empty(&mut self, rect: Rect, section_name: &str, obj: &ReportObject, msg: &str) {
        push_diag(
            &self.diagnostics,
            Diagnostic::warn(DiagnosticKind::UnsupportedObject, msg).with_source(&obj.name),
        );
        self.placeholder_box(rect, section_name, obj, ObjectKind::Chart);
    }
}

/// Reserve a legend band and return `(legend_ops, body_rect)`, honouring the decoded legend
/// visibility + position (`0x0121` `+0x410`). When `visible` is false the whole
/// `rect` is given to the chart body and no legend ops are emitted. `per_slice` picks the pie/
/// doughnut per-slice swatch colors over the cycled base palette.
fn resolve_legend(
    style: chart::ChartStyle,
    rect: Rect,
    kind: LegendKind,
    series: &[(String, f64)],
    per_slice: bool,
    section_name: &str,
    obj_name: &str,
) -> (Vec<DrawOp>, Rect) {
    if !style.def.legend_visible || kind == LegendKind::None {
        return (Vec::new(), rect);
    }
    use rpt_model::ChartLegendPosition as Lp;
    let pos = match style.def.legend_position {
        Lp::Right => chart::LegendPosition::Right,
        Lp::Left => chart::LegendPosition::Left,
        // A manually-positioned legend: place it at the top as a render approximation (the exact
        // stored geometry is not decoded).
        Lp::Custom => chart::LegendPosition::Top,
        Lp::BottomCenter => chart::LegendPosition::Bottom,
    };
    match kind {
        LegendKind::Series(name) => {
            chart::series_legend(style, rect, pos, &name, section_name, obj_name)
        }
        _ => chart::legend(style, rect, pos, series, per_slice, section_name, obj_name),
    }
}

/// What a chart's legend lists.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LegendKind {
    /// One boxed entry naming the plotted series — the engine's legend for the single-color
    /// families (area/line/stock/radar).
    Series(String),
    /// One entry per category or per series, colored to match the marks.
    Categories,
    /// No legend at all: a single-color family whose series name cannot be derived. A category list
    /// would be actively wrong here (every mark is one color), so nothing is drawn.
    None,
}

/// Which legend `chart` draws. The single-color families legend their series; every other family
/// legends its categories.
fn legend_kind(chart: &rpt_model::ChartObject) -> LegendKind {
    use rpt_model::ChartGraphType as Gt;
    if !matches!(
        chart.definition.graph_type,
        Gt::Area | Gt::Line | Gt::Stock | Gt::Radar
    ) {
        return LegendKind::Categories;
    }
    // The stored data-axis title is the same auto-generated string when the author never edited it,
    // so it stands in for a binding this does not parse.
    match series_name(chart) {
        Some(name) => LegendKind::Series(name),
        None if !chart.definition.data_axis_title.is_empty() => {
            LegendKind::Series(chart.definition.data_axis_title.clone())
        }
        None => LegendKind::None,
    }
}

/// The name the engine gives a single-series chart's legend entry: the summary operation applied to
/// the charted field, `Sum of id` / `Min of total`, built from the chart's first data binding
/// (`Sum ({orders.id}, {orders.created_at}, "daily")`). It is derived rather than read from
/// [`ChartDefinition::data_axis_title`](rpt_model::ChartDefinition::data_axis_title), which a stock
/// chart leaves empty while the engine still legends its series.
fn series_name(chart: &rpt_model::ChartObject) -> Option<String> {
    let binding = chart.definition.data_refs.first()?.trim();
    let op = binding
        .split(['(', ' '])
        .find(|t| !t.is_empty())
        .filter(|t| !t.starts_with('{'))?;
    let field = brace_groups(binding)
        .next()
        .map(|g| last_segment(strip_braces(g)))?;
    if field.is_empty() {
        return None;
    }
    Some(format!("{} of {field}", summary_label(op)?))
}

/// The engine's display name for a summary operation in a series label — abbreviated for the
/// operations Crystal shortens (`Minimum` legends as `Min`), the operation's own name otherwise.
fn summary_label(op: &str) -> Option<&'static str> {
    Some(match op.to_ascii_lowercase().as_str() {
        "sum" => "Sum",
        "count" => "Count",
        "distinctcount" | "distinct count" => "DCount",
        "average" | "avg" => "Avg",
        "maximum" | "max" => "Max",
        "minimum" | "min" => "Min",
        "median" => "Median",
        _ => return None,
    })
}

/// Reserve the subtitle band (under the title) and footnote band (at the chart bottom) and draw both
/// decoded text elements, returning `(caption_ops, body_rect)` — the reduced rect the chart body
/// (title/plot/legend) draws into. Returns `None` when both are empty (the common case), so the
/// per-type renderer keeps drawing its own title into the full rect and existing output is unchanged.
/// When present, the top title is drawn centrally here (subtitle sits just under it) and the renderer
/// is handed an empty title. Fonts come from the per-element default table
/// ([`chart::ChartText`]): subtitle Arial 10, footnote Arial 8 bold-italic, title Arial 14 bold.
fn chart_captions(
    style: chart::ChartStyle,
    rect: Rect,
    top_title: &str,
    subtitle: &str,
    footnote: &str,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> Option<(Vec<DrawOp>, Rect)> {
    if subtitle.is_empty() && footnote.is_empty() {
        return None;
    }
    let (rl, rt, rw, rh) = (rect.left.0, rect.top.0, rect.width.0, rect.height.0);
    let pad = 60;
    let title_h = if top_title.is_empty() {
        0
    } else {
        (rh / 8).clamp(180, 360)
    };
    let subtitle_h = if subtitle.is_empty() {
        0
    } else {
        (rh / 12).clamp(150, 260)
    };
    let footnote_h = if footnote.is_empty() {
        0
    } else {
        (rh / 14).clamp(130, 220)
    };
    let mut ops = Vec::new();
    let band = |top: i32, height: i32| Rect {
        left: Twips(rl),
        top: Twips(top),
        width: Twips(rw),
        height: Twips(height),
    };
    if !top_title.is_empty() {
        ops.push(chart::chart_text_op(
            style,
            band(rt + pad / 2, title_h),
            top_title,
            chart::ChartText::Title,
            src,
        ));
    }
    if !subtitle.is_empty() {
        ops.push(chart::chart_text_op(
            style,
            band(rt + title_h, subtitle_h),
            subtitle,
            chart::ChartText::Subtitle,
            src,
        ));
    }
    if !footnote.is_empty() {
        ops.push(chart::chart_text_op(
            style,
            band(rt + rh - footnote_h, footnote_h),
            footnote,
            chart::ChartText::Footnote,
            src,
        ));
    }
    let body = Rect {
        left: Twips(rl),
        top: Twips(rt + title_h + subtitle_h),
        width: Twips(rw),
        height: Twips((rh - title_h - subtitle_h - footnote_h).max(1)),
    };
    Some((ops, body))
}

/// Reserve the caption bands around `rect` for a chart that builds its own title and legend instead
/// of going through the shared 2-D dispatch (multi-series bar, 3-D riser/area, scatter/bubble, stock,
/// histogram, gantt). Returns the caption draw-ops, the reduced rect the chart body draws into, and
/// the title the per-type renderer must still draw itself — empty once [`chart_captions`] has drawn
/// it above the subtitle. With neither caption set (the common case) this reserves nothing and the
/// renderer's own output is unchanged.
fn reserve_captions<'a>(
    style: chart::ChartStyle,
    rect: Rect,
    title: &'a str,
    src: &dyn Fn() -> Option<ObjectRef>,
) -> (Vec<DrawOp>, Rect, &'a str) {
    match chart_captions(
        style,
        rect,
        title,
        &style.def.subtitle,
        &style.def.footnote,
        src,
    ) {
        Some((ops, body)) => (ops, body, ""),
        None => (Vec::new(), rect, title),
    }
}

/// The legend entries for a depth-effect area chart: a single-series ribbon colors its area per
/// category (legend lists the categories with their values), a multi-series one colors per series
/// (legend lists the series names). Callers guard `series` non-empty, so `series[0]` is safe.
fn multi_legend_series(categories: &[String], series: &[(String, Vec<f64>)]) -> Vec<(String, f64)> {
    if series.len() > 1 {
        series.iter().map(|(n, _)| (n.clone(), 0.0)).collect()
    } else {
        categories
            .iter()
            .zip(&series[0].1)
            .map(|(c, v)| (c.clone(), *v))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::chart_captions;
    use crate::chart::ChartText;
    use rpt_model::{Rect, Twips};
    use rpt_pages::DrawOp;

    fn rect() -> Rect {
        Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(6000),
            height: Twips(6000),
        }
    }

    fn style(def: &rpt_model::ChartDefinition) -> crate::chart::ChartStyle<'_> {
        crate::chart::ChartStyle {
            def,
            height: rect().height,
        }
    }

    /// With neither a subtitle nor a footnote (the common case), no bands are reserved and the
    /// per-type renderer keeps drawing its own title into the full rect — so existing output is
    /// byte-identical.
    #[test]
    fn no_captions_leaves_the_rect_untouched() {
        let src = || None;
        let def = rpt_model::ChartDefinition::default();
        assert!(chart_captions(style(&def), rect(), "Title", "", "", &src).is_none());
    }

    /// A chart with a subtitle and footnote draws both as text ops (the title centrally above the
    /// subtitle), each in its per-element default font, and reserves top+bottom bands so the returned
    /// body rect is shorter than the full rect and pushed down from the top.
    #[test]
    fn subtitle_and_footnote_are_drawn_and_reserve_bands() {
        let src = || None;
        let def = rpt_model::ChartDefinition::default();
        let (ops, body) =
            chart_captions(style(&def), rect(), "Title", "Sub here", "Foot here", &src)
                .expect("captions present");

        let texts: Vec<(&str, &rpt_pages::FontSpec)> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some((t.text.as_str(), &t.font)),
                _ => None,
            })
            .collect();
        let by = |s: &str| texts.iter().find(|(t, _)| *t == s).map(|(_, f)| *f);

        // The title, subtitle, and footnote are all emitted.
        assert!(by("Title").is_some(), "title drawn: {texts:?}");
        let sub = by("Sub here").expect("subtitle drawn");
        let foot = by("Foot here").expect("footnote drawn");
        // Each uses its per-element default font, scaled to the chart's height.
        let scaled = |pt: f32| style(&def).scaled_pt(pt);
        assert_eq!(sub.size_pt, scaled(10.0), "subtitle Arial 10");
        assert!(!sub.bold && !sub.italic, "subtitle normal");
        assert_eq!(foot.size_pt, scaled(8.0), "footnote Arial 8");
        assert!(foot.bold && foot.italic, "footnote bold-italic");
        let _ = ChartText::Footnote; // the caption fonts come from this table.

        // The body rect is reserved away from the top (subtitle band) and bottom (footnote band).
        assert!(
            body.top.0 > rect().top.0,
            "body pushed below the subtitle band"
        );
        assert!(
            body.height.0 < rect().height.0,
            "body shorter than the full rect"
        );
    }

    /// The specialized chart paths (multi-series bar, 3-D riser/area, scatter/bubble, stock,
    /// histogram, gantt) reserve their caption bands through this wrapper: with captions it draws
    /// them, shrinks the body rect and takes the title off the renderer's hands; without, it reserves
    /// nothing and hands the title straight back.
    #[test]
    fn reserve_captions_hands_the_title_over_only_when_it_draws_one() {
        use super::reserve_captions;
        let src = || None;
        let mut def = rpt_model::ChartDefinition::default();

        let (ops, body, title) = reserve_captions(style(&def), rect(), "Sales", &src);
        assert!(ops.is_empty(), "no captions reserves nothing");
        assert_eq!(body, rect());
        assert_eq!(title, "Sales", "the renderer still draws its own title");

        def.subtitle = "By region".to_string();
        def.footnote = "Source: ledger".to_string();
        let (ops, body, title) = reserve_captions(style(&def), rect(), "Sales", &src);
        let texts: Vec<&str> = ops
            .iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["Sales", "By region", "Source: ledger"]);
        assert!(title.is_empty(), "the caption band drew the title");
        assert!(body.height.0 < rect().height.0, "body shrunk for the bands");
    }

    /// The single-series legend entry names the summary applied to the charted field, built from the
    /// chart's first data binding — the engine's `Sum of id` / `Min of total`. It is derived, not read
    /// from `data_axis_title`, which a stock chart leaves empty while still legending its series.
    #[test]
    fn series_name_is_the_summary_of_the_charted_field() {
        use super::series_name;
        let named = |binding: &str| {
            let mut chart = rpt_model::ChartObject::default();
            chart.definition.data_refs = vec![binding.to_string()];
            series_name(&chart)
        };
        assert_eq!(
            named("Sum ({orders.id}, {orders.created_at}, \"daily\")").as_deref(),
            Some("Sum of id")
        );
        assert_eq!(
            named("Minimum ({orders.total}, {orders.created_at}, \"daily\")").as_deref(),
            Some("Min of total"),
            "Minimum abbreviates to Min"
        );
        assert_eq!(
            named("Average of {orders.total}").as_deref(),
            Some("Avg of total")
        );
        // A binding whose leading token is not a summary operation, or that names no field, has no
        // series name — the chart then falls back to the per-category legend rather than inventing one.
        assert_eq!(named("{orders.total}"), None);
        assert_eq!(named("Wibble ({orders.total})"), None);
        assert_eq!(named("Sum ()"), None);
    }

    /// The single-color families — area, line, stock and radar — legend the plotted series with one
    /// boxed entry; every other family lists its categories. Radar belongs with them: its polygon is
    /// drawn in one color, so a per-category swatch list would name colors the chart never uses.
    #[test]
    fn the_single_color_families_legend_their_series() {
        use super::{legend_kind, LegendKind};
        use rpt_model::ChartGraphType as Gt;
        let kind = |ty: Gt| {
            let mut chart = rpt_model::ChartObject::default();
            chart.definition.graph_type = ty;
            chart.definition.data_refs =
                vec!["Sum ({orders.total}, {orders.created_at}, \"daily\")".to_string()];
            legend_kind(&chart)
        };
        for ty in [Gt::Area, Gt::Line, Gt::Stock, Gt::Radar] {
            assert_eq!(
                kind(ty),
                LegendKind::Series("Sum of total".to_string()),
                "{ty:?} legends its series"
            );
        }
        for ty in [Gt::Bar, Gt::Pie, Gt::Doughnut, Gt::Funnel, Gt::Gauge] {
            assert_eq!(
                kind(ty),
                LegendKind::Categories,
                "{ty:?} legends categories"
            );
        }
    }
}
