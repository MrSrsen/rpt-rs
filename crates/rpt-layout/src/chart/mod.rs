//! Render a chart object as native Page-IR draw-ops.
//!
//! Crystal charts are modeled as **Group charts**: one data point per group, its height the
//! group's summary of the charted field. The layout engine already computes those group summaries
//! ([`rpt_data::GroupInstance`]), so a chart's series is `(group key → summary value)`. Rather than
//! rasterize via an external chart crate (which would then need per-backend image embedding), we emit
//! the chart as ordinary [`rpt_pages::DrawOp`]s (bars = rects, axes = lines, labels = text) — so it
//! renders identically through every backend (HTML/SVG/PDF/raster) with no new dependency.
//!
//! The visual *shape* comes from the decoded [`rpt_model::ChartGraphType`], one
//! renderer per submodule: [`bar`], [`line`], [`area`], [`pie`], [`doughnut`], [`scatter`], [`stock`],
//! [`histogram`], and more. The axis families ([`bar`]/[`line`]/[`area`]) share the Num+Ord frame
//! ([`common::chart_frame`]) and label helpers in [`common`] (including the shared category-label
//! thinning), differing only in the series builder (risers vs. a polyline + markers vs. a filled
//! region); [`scatter`] plots markers over two numeric axes; [`stock`] draws hi-lo/OHLC bars;
//! [`histogram`] bins a value field; [`pie`]/[`doughnut`] need no axes. A multi-series bar chart
//! (more than one data binding) uses [`bar::bar_chart_multi`], which arranges the series
//! clustered/stacked/percent. Types without a renderer fall back to bars, and the caller records a
//! diagnostic noting the approximation.

mod area;
mod bar;
pub(crate) mod chart3d;
mod common;
mod doughnut;
mod funnel;
mod gantt;
mod gauge;
mod histogram;
mod line;
mod numeric_axis;
mod pie;
mod radar;
mod render;
mod scatter;
mod stock;

pub(crate) use area::area_chart;
pub(crate) use bar::{bar_chart, bar_chart_multi};
pub(crate) use common::{legend, AxisTitles, LegendPosition};
pub(crate) use doughnut::doughnut_chart;
pub(crate) use funnel::funnel_chart;
pub(crate) use gantt::{gantt_chart, GanttBar};
pub(crate) use gauge::gauge_chart;
pub(crate) use histogram::histogram_chart;
pub(crate) use line::line_chart;
pub(crate) use numeric_axis::numeric_axis_chart;
pub(crate) use pie::pie_chart;
pub(crate) use radar::radar_chart;
pub(crate) use scatter::scatter_chart;
pub(crate) use stock::{stock_chart, StockPoint};
