//! Structural decode test for the chart legend (visible flag + placement), read from the `0x0121`
//! `ChartDefinition2` styling struct. Each case pins one styling property byte-exactly against a
//! fixture that varies only that property.

use rpt_reader::model::{ChartLegendPosition, ReportObjectKind};

/// The first chart's decoded [`rpt_reader::model::ChartDefinition`] in a report.
fn chart_def(report: &rpt_reader::model::Report) -> Option<&rpt_reader::model::ChartDefinition> {
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

/// The first chart definition's `(legend_visible, legend_position)` in a committed fixture.
fn legend(rel: &str) -> (bool, ChartLegendPosition) {
    let rpt = open(rel);
    let def = chart_def(rpt.report()).expect("fixture has a chart");
    (def.legend_visible, def.legend_position)
}

/// The baseline bar chart: legend shown, default position Right (`01 00` in the styling struct).
#[test]
fn baseline_legend_visible_right() {
    let (visible, pos) = legend("synthetic/chart_baseline.rpt");
    assert!(visible, "baseline legend is shown");
    assert_eq!(pos, ChartLegendPosition::Right);
}

/// The legend flags decode byte-exactly:
/// right → (visible, Right), left → (visible, Left), off → (hidden, ...).
#[test]
fn parking_legend_minimal_pairs() {
    let (visible, pos) = legend("parking/orders_legend_right.rpt");
    assert!(visible, "right variant legend is shown");
    assert_eq!(pos, ChartLegendPosition::Right);
    // Left (code 1) — the position high byte reads 01.
    let (visible, pos) = legend("parking/orders_legend_left.rpt");
    assert!(visible, "left variant legend is shown");
    assert_eq!(pos, ChartLegendPosition::Left);
    // Off — the visible bit clears (low byte 01→00). The stored position is not meaningful when
    // hidden (the engine resets it to Bottom on save), so only the visible flag is asserted.
    let (visible, _pos) = legend("parking/orders_legend_off.rpt");
    assert!(!visible, "off variant legend is hidden");
}

/// The first chart's `(graph_type, data_labels_show_value, n_series_colors)` in a committed fixture.
fn data_labels(rel: &str) -> (rpt_reader::model::ChartGraphType, bool, usize) {
    let rpt = open(rel);
    let def = chart_def(rpt.report()).expect("fixture has a chart");
    (
        def.graph_type,
        def.data_labels_show_value,
        def.series_colors.len(),
    )
}

/// The data-labels "show value" flag is bit 1 of the `0x0121` data-labels enum byte, at an offset
/// that shifts +2 for the pie family relative to the bar family. `series_colors` is always empty:
/// the design-time palette is not byte-recoverable.
#[test]
fn data_labels_and_series_colors_across_fixtures() {
    // Axis charts: bar/area walk data-labels at legend+81.
    for rel in [
        "synthetic/chart_baseline.rpt",
        "parking/orders.rpt",
        "parking/orders_area.rpt",
        "synthetic/chart_datalabel_bar_control.rpt",
    ] {
        let (_gt, show, ncolors) = data_labels(rel);
        assert!(!show, "{rel}: no data labels enabled");
        assert_eq!(ncolors, 0, "{rel}: series colors are not byte-recoverable");
    }
    // Pie chart: the pie-family +2 shift must land on a plausible (off) value, not garbage.
    for rel in ["parking/orders_pie.rpt", "synthetic/chart_pie_control.rpt"] {
        let (gt, show, ncolors) = data_labels(rel);
        assert_eq!(gt, rpt_reader::model::ChartGraphType::Pie, "{rel}");
        assert!(!show, "{rel}: no data labels enabled");
        assert_eq!(ncolors, 0, "{rel}");
    }
    // Positive case: the "show value" bit set (bar family). Proves the offset isn't landing on a
    // second, coincidental zero — a wrong-by-one decoder would fail this, not just the control above.
    let (gt, show, ncolors) = data_labels("synthetic/chart_datalabel_bar_showvalue.rpt");
    assert_eq!(gt, rpt_reader::model::ChartGraphType::Bar);
    assert!(show, "bar showvalue fixture has data labels enabled");
    assert_eq!(ncolors, 0);
    // Positive case, pie family: confirms the +2 pie-family shift lands on the same bit when set,
    // not just when clear.
    let (gt, show, ncolors) = data_labels("synthetic/chart_datalabel_pie_showvalue.rpt");
    assert_eq!(gt, rpt_reader::model::ChartGraphType::Pie);
    assert!(show, "pie showvalue fixture has data labels enabled");
    assert_eq!(ncolors, 0);
}

/// `SliceDetachment` is the second of the two enums only a pie/doughnut chart writes, so it is a
/// field of the pie families' own sequence rather than a slot every chart has.
///
/// A bar chart writes neither enum, so it reports the same default value the control pie does.
#[test]
fn pie_slice_detachment_is_stored_only_by_the_pie_families() {
    use rpt_reader::model::{ChartGraphType, ChartPieSize, ChartSliceDetachment};

    let detached = open("synthetic/chart_pie_slicedetachment.rpt");
    let def = chart_def(detached.report()).expect("fixture has a chart");
    assert_eq!(def.graph_type, ChartGraphType::Pie);
    assert_eq!(def.slice_detachment, ChartSliceDetachment::LargestSlice);
    // The circle size is the sizing run's word, not the superseded copy stored beside this enum.
    assert_eq!(def.pie_size, ChartPieSize::Large);

    let control = open("synthetic/chart_pie_control.rpt");
    let def = chart_def(control.report()).expect("fixture has a chart");
    assert_eq!(def.slice_detachment, ChartSliceDetachment::NoDetachment);

    // A family that writes neither enum: everything after them still lands, so the graph type and
    // the circle size read normally while the detachment stays at its default.
    let bar = open("synthetic/chart_baseline.rpt");
    let def = chart_def(bar.report()).expect("fixture has a chart");
    assert_eq!(def.graph_type, ChartGraphType::Bar);
    assert_eq!(def.slice_detachment, ChartSliceDetachment::NoDetachment);
}

/// `ChartGraphType::from_code` names the documented shape families so the render
/// side can dispatch by shape; unnamed codes round-trip through `Other`.
#[test]
fn graph_type_from_code_maps_documented_families() {
    use rpt_reader::model::ChartGraphType as G;
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
    // Codes past the gallery (0..=15) are preserved verbatim, not silently collapsed.
    assert_eq!(G::from_code(16), G::Other(16));
    assert_eq!(G::from_code(99), G::Other(99));
}

/// `is_3d()` is true only for the inherently three-dimensional disk families (Riser3D=5, Surface3D=6).
/// The 2-D-type "depth effect" is not byte-isolatable, so a depth-on area chart still reports false
/// (documented limitation on `ChartDefinition::is_3d`).
#[test]
fn is_3d_only_for_3d_families() {
    use rpt_reader::model::{ChartDefinition, ChartGraphType as G};
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
    let rpt = open("parking/orders_area.rpt");
    let def = chart_def(rpt.report()).expect("area fixture has a chart");
    assert_eq!(def.graph_type, G::Area);
    assert!(
        !def.is_3d(),
        "2-D area with depth-effect is not reported as 3-D"
    );
}

/// Bar-family `arrangement()` decoded from the `graph_subtype` variant slot: subtype `0`→Clustered,
/// `1`→Stacked, `2`→Percent.
#[test]
fn arrangement_decodes_from_the_bar_subtype_slot() {
    use rpt_reader::model::{ChartArrangement, ChartDefinition};
    // Default and a truncated record both fall back to Clustered.
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
        let rpt = open(rel);
        let def = chart_def(rpt.report()).expect("fixture has a chart");
        assert_eq!(
            def.graph_type,
            rpt_reader::model::ChartGraphType::Bar,
            "{rel}"
        );
        assert_eq!(def.arrangement(), want, "{rel}");
    }
}

/// The 2-D depth-effect bit (`0x02` of `graph_subtype`) on the Area family (subtype `0x14`→`0x16`).
/// Depth is a shallow-3-D look, not a 3-D chart type, so `is_3d()` stays `false`.
#[test]
fn area_depth_effect_reads_the_subtype_bit() {
    let cases = [
        ("parking/orders_area.rpt", false),
        ("parking/orders_area_depth.rpt", true),
    ];
    for (rel, want) in cases {
        let rpt = open(rel);
        let def = chart_def(rpt.report()).expect("fixture has a chart");
        assert_eq!(
            def.graph_type,
            rpt_reader::model::ChartGraphType::Area,
            "{rel}"
        );
        assert_eq!(def.has_depth_effect(), want, "{rel} depth");
        assert!(!def.is_3d(), "{rel}: depth effect is not a 3-D chart type");
    }
}

/// Open a committed chart fixture.
///
/// # Panics
///
/// If the fixture is missing. Every report named in this file lives under `tests/fixtures/reports`,
/// so an absent one is the corpus having moved, not a check being unavailable — and a skip there
/// would leave each case matrix below asserting nothing.
fn open(rel: &str) -> rpt_reader::Rpt {
    let path = rpt_test_support::fixture("tests/fixtures/reports").join(rel);
    rpt_reader::Rpt::open(&path).unwrap_or_else(|e| panic!("open committed fixture {rel}: {e}"))
}

/// The chart's "on change of `<date>`" category period, decoded from the category grid `0xe5`
/// group's SDK-ordinal byte (`used + 3`) — the same encoding a report group's date condition uses.
///
/// The period is read independently of the stock/stacked flag (`0x0126` first-u32).
///
/// A chart whose category has no report group can render its axis with biweekly-spaced labels even
/// when the stored ordinal reads `Weekly` — with no report group to bucket the detail rows itself, the
/// engine auto-thins a crowded date axis at render time; that is a render-time cosmetic, not a stored
/// interval. A genuinely biweekly chart stores ordinal `2` directly and decodes as `Biweekly`.
#[test]
fn category_period_from_grid_group() {
    use rpt_reader::model::ChartCategoryPeriod::*;
    let cases = [
        ("parking/orders.rpt", Some(Monthly)),
        ("parking/orders_weekly.rpt", Some(Weekly)),
        ("parking/orders_bar_stacked.rpt", Some(Monthly)),
        // Discrete/XY category (scatter) — no periodic grouping.
        ("parking/chart_scatter.rpt", None),
        // Stock charts genuinely group weekly (ordinal 1); the engine's biweekly-spaced axis labels
        // are render-time auto-thinning, not a stored interval (see the doc above). Decodes Weekly.
        ("parking/chart_stock.rpt", Some(Weekly)),
        ("parking/chart_stock_open.rpt", Some(Weekly)),
    ];
    for (rel, want) in cases {
        let rpt = open(rel);
        let def = chart_def(rpt.report()).expect("fixture has a chart");
        assert_eq!(def.category_period, want, "{rel} category period");
    }
}
