//! Structural decode test for the chart legend (visible flag + placement), read from the `0x0121`
//! `ChartDefinition2` styling struct. RptToXml never emits a `ChartObject`, so there is no oracle —
//! these are verified byte-exactly against single-property synthetic/minimal-pair fixtures.

use rpt::model::{ChartLegendPosition, ReportObjectKind};
use std::path::Path;

/// The first chart's decoded [`rpt::model::ChartDefinition`] in a report.
fn chart_def(report: &rpt::model::Report) -> Option<&rpt::model::ChartDefinition> {
    report
        .report_definition
        .areas
        .iter()
        .flat_map(|a| &a.sections)
        .flat_map(|s| &s.objects)
        .find_map(|o| match &o.kind {
            ReportObjectKind::Chart(c) => Some(&c.definition),
            _ => None,
        })
}

/// Open a fixture and return its first chart definition's `(legend_visible, legend_position)`,
/// or `None` if the fixture is absent (so the suite stays green without the private corpus).
fn legend(rel: &str) -> Option<(bool, ChartLegendPosition)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/reports")
        .join(rel);
    let rpt = rpt::Rpt::open(&path).ok()?;
    let def = chart_def(rpt.report()).expect("fixture has a chart");
    Some((def.legend_visible, def.legend_position))
}

/// The baseline bar chart: legend shown, default position Right (`01 00` in the styling struct).
#[test]
fn baseline_legend_visible_right() {
    let Some((visible, pos)) = legend("synthetic/chart_baseline.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    assert!(visible, "baseline legend is shown");
    assert_eq!(pos, ChartLegendPosition::Right);
}

/// Minimal-pair fixtures over the parking bar chart confirm the legend flags byte-exactly:
/// right → (visible, Right), left → (visible, Left), off → (hidden, ...).
#[test]
fn parking_legend_minimal_pairs() {
    if let Some((visible, pos)) = legend("parking/orders_legend_right.rpt") {
        assert!(visible, "right variant legend is shown");
        assert_eq!(pos, ChartLegendPosition::Right);
    } else {
        eprintln!("[skip] parking legend fixtures absent");
        return;
    }
    // Left (code 1) — confirmed by the right→left byte diff at the position high byte (00→01).
    let (visible, pos) = legend("parking/orders_legend_left.rpt").expect("left fixture");
    assert!(visible, "left variant legend is shown");
    assert_eq!(pos, ChartLegendPosition::Left);
    // Off — the visible bit clears (low byte 01→00). The stored position is not meaningful when
    // hidden (the engine resets it to Bottom on save), so only the visible flag is asserted.
    let (visible, _pos) = legend("parking/orders_legend_off.rpt").expect("off fixture");
    assert!(!visible, "off variant legend is hidden");
}

/// Open a fixture and return its first chart's `(graph_type, data_labels_show_value, n_series_colors)`,
/// or `None` if the fixture is absent.
fn data_labels(rel: &str) -> Option<(rpt::model::ChartGraphType, bool, usize)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/reports")
        .join(rel);
    let rpt = rpt::Rpt::open(&path).ok()?;
    let def = chart_def(rpt.report()).expect("fixture has a chart");
    Some((
        def.graph_type,
        def.data_labels_show_value,
        def.series_colors.len(),
    ))
}

/// The data-labels "show value" flag (bit1 of the `0x0121` data-labels enum byte) decodes off across
/// every corpus chart — none of the sampled fixtures enable point labels, so the walk lands on `00`
/// for axis charts (+81 from the legend short) and on `00` for the pie fixture (+83, pie-family
/// shift). The positive (`02`) case is proven by the differential byte-map, not a fixture.
/// `series_colors` is always empty: the design-time palette is not byte-recoverable.
#[test]
fn data_labels_and_series_colors_across_fixtures() {
    // Axis charts: bar/area walk data-labels at legend+81.
    for rel in [
        "synthetic/chart_baseline.rpt",
        "parking/orders.rpt",
        "parking/orders_area.rpt",
    ] {
        let Some((_gt, show, ncolors)) = data_labels(rel) else {
            eprintln!("[skip] {rel} absent");
            continue;
        };
        assert!(!show, "{rel}: no data labels enabled");
        assert_eq!(ncolors, 0, "{rel}: series colours are not byte-recoverable");
    }
    // Pie chart: the pie-family +2 shift must land on a plausible (off) value, not garbage.
    if let Some((gt, show, ncolors)) = data_labels("parking/orders_pie.rpt") {
        assert_eq!(gt, rpt::model::ChartGraphType::Pie);
        assert!(!show, "pie fixture has no data labels");
        assert_eq!(ncolors, 0);
    }
}

/// `ChartGraphType::from_code` names the documented shape families so the render
/// side can dispatch by shape; unnamed codes round-trip through `Other`.
#[test]
fn graph_type_from_code_maps_documented_families() {
    use rpt::model::ChartGraphType as G;
    let cases = [
        (0, G::Bar),
        (1, G::Line),
        (2, G::Area),
        (3, G::Pie),
        (4, G::Doughnut),
        (5, G::Riser3D),
        (6, G::Surface3D),
        (7, G::Scatter),
        (8, G::Radar),
        (9, G::Bubble),
        (10, G::Stock),
        (11, G::NumericAxis),
        (12, G::Gauge),
        (13, G::Gantt),
        (14, G::Funnel),
        (15, G::Histogram),
    ];
    for (code, want) in cases {
        assert_eq!(G::from_code(code), want, "code {code}");
    }
    // Codes past the confirmed gallery (0..=15) are preserved verbatim, not silently collapsed.
    assert_eq!(G::from_code(16), G::Other(16));
    assert_eq!(G::from_code(99), G::Other(99));
}

/// `is_3d()` is true only for the inherently three-dimensional disk families (Riser3D=5, Surface3D=6).
/// The 2-D-type "depth effect" is not byte-isolatable, so a depth-on area chart still reports false
/// (documented limitation on `ChartDefinition::is_3d`).
#[test]
fn is_3d_only_for_3d_families() {
    use rpt::model::{ChartDefinition, ChartGraphType as G};
    let with = |gt: G| ChartDefinition {
        graph_type: gt,
        ..Default::default()
    };
    assert!(with(G::Riser3D).is_3d());
    assert!(with(G::Surface3D).is_3d());
    for gt in [
        G::Bar,
        G::Line,
        G::Area,
        G::Pie,
        G::Doughnut,
        G::Scatter,
        G::Radar,
        G::Bubble,
        G::Funnel,
        G::Other(99),
    ] {
        assert!(!with(gt).is_3d(), "{gt:?} is not a 3-D family");
    }
    // The depth-on area fixture (graph_subtype 22) is stored as 2-D type Area — is_3d stays false.
    if let Some(rpt) = open("parking/orders_area.rpt") {
        let def = chart_def(rpt.report()).expect("area fixture has a chart");
        assert_eq!(def.graph_type, G::Area);
        assert!(
            !def.is_3d(),
            "2-D area with depth-effect is not reported as 3-D"
        );
    }
}

/// Bar-family `arrangement()` decoded from the `graph_subtype` variant slot, confirmed by the parking
/// bar minimal pairs: subtype `0`→Clustered, `1`→Stacked, `2`→Percent.
#[test]
fn arrangement_bar_minimal_pairs() {
    use rpt::model::{ChartArrangement, ChartDefinition};
    // Default and a truncated leaf both fall back to Clustered.
    assert_eq!(
        ChartDefinition::default().arrangement(),
        ChartArrangement::Clustered
    );
    let cases = [
        ("parking/orders.rpt", ChartArrangement::Clustered),
        ("parking/orders_bar_stacked.rpt", ChartArrangement::Stacked),
        (
            "parking/orders_bar_percentage.rpt",
            ChartArrangement::Percent,
        ),
    ];
    for (rel, want) in cases {
        let Some(rpt) = open(rel) else {
            eprintln!("[skip] {rel} absent");
            continue;
        };
        let def = chart_def(rpt.report()).expect("fixture has a chart");
        assert_eq!(def.graph_type, rpt::model::ChartGraphType::Bar, "{rel}");
        assert_eq!(def.arrangement(), want, "{rel}");
    }
}

/// The 2-D depth-effect bit (`0x02` of `graph_subtype`) on the Area family, confirmed by the area
/// minimal pair (subtype `0x14`→`0x16`). Depth is a shallow-3-D look, not a 3-D chart type, so
/// `is_3d()` stays `false`.
#[test]
fn area_depth_effect_minimal_pair() {
    let cases = [
        ("parking/orders_area.rpt", false),
        ("parking/orders_area_depth.rpt", true),
    ];
    for (rel, want) in cases {
        let Some(rpt) = open(rel) else {
            eprintln!("[skip] {rel} absent");
            continue;
        };
        let def = chart_def(rpt.report()).expect("fixture has a chart");
        assert_eq!(def.graph_type, rpt::model::ChartGraphType::Area, "{rel}");
        assert_eq!(def.has_depth_effect(), want, "{rel} depth");
        assert!(!def.is_3d(), "{rel}: depth effect is not a 3-D chart type");
    }
}

/// Open a fixture, returning `None` if absent so the suite stays green without the private corpus.
fn open(rel: &str) -> Option<rpt::Rpt> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/reports")
        .join(rel);
    rpt::Rpt::open(&path).ok()
}

/// The chart's "on change of `<date>`" category period, decoded from the category grid `0xe5`
/// group's SDK-ordinal byte (`used + 3`) — the same encoding a report group's date condition uses.
///
/// `orders` (monthly) and `orders_weekly` (weekly) are byte-confirmed AND cross-checked against the
/// engine's rendered category-axis labels (6 monthly vs 25 weekly buckets). `orders_bar_stacked`
/// (monthly) proves the period is read independently of the stock/stacked flag (`0x0126` first-u32).
///
/// KNOWN RESIDUAL: the engine renders the `chart_stock` stock chart with **biweekly** (13) buckets,
/// but its category grid stores ordinal `1` (weekly) — byte-identical to genuinely-weekly
/// `orders_weekly` at every non-confounded position — so it decodes as `Weekly`. The biweekly signal
/// is not byte-isolable from the current fixtures (it is confounded with the stock chart type and
/// the absence of a report group).
#[test]
fn category_period_from_grid_group() {
    use rpt::model::ChartCategoryPeriod::*;
    let cases = [
        ("parking/orders.rpt", Some(Monthly)),
        ("parking/orders_weekly.rpt", Some(Weekly)),
        ("parking/orders_bar_stacked.rpt", Some(Monthly)),
        // Discrete/XY category (scatter) — no periodic grouping.
        ("parking/chart_scatter.rpt", None),
        // Stock charts: engine renders biweekly, but the stored SDK ordinal is 1 (weekly). Decodes
        // as Weekly — the stored value; the biweekly render is a documented, un-isolable residual.
        ("parking/chart_stock.rpt", Some(Weekly)),
        ("parking/chart_stock_open.rpt", Some(Weekly)),
    ];
    let mut ran = false;
    for (rel, want) in cases {
        let Some(rpt) = open(rel) else {
            eprintln!("[skip] {rel} absent");
            continue;
        };
        ran = true;
        let def = chart_def(rpt.report()).expect("fixture has a chart");
        assert_eq!(def.category_period, want, "{rel} category period");
    }
    if !ran {
        eprintln!("[skip] parking chart fixtures absent");
    }
}
