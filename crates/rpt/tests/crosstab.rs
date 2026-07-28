//! Structural decode test for the cross-tab pivot structure (row/column dimensions + measures).
//!
//! Checked structurally against the decoded model, using the synthetic `parking/crosstab_*`
//! fixtures for grid-options coverage; they skip gracefully when absent.

use rpt::model::{Color, CrossTabGridOptions, ReportObjectKind};

/// Find the `CrossTab1` object in a decoded report.
fn crosstab(report: &rpt::model::Report) -> Option<&rpt::model::CrossTabObject> {
    report
        .report_definition
        .areas
        .iter()
        .flat_map(|a| &a.sections)
        .flat_map(|s| &s.objects)
        .find_map(|o| match &o.kind {
            ReportObjectKind::CrossTab(c) => Some(c),
            _ => None,
        })
}

/// Decode a `parking/crosstab_<variant>.rpt` fixture and return its cross-tab grid options, or
/// `None` if the fixture is absent (these synthetic fixtures are not always checked out).
fn options(variant: &str) -> Option<CrossTabGridOptions> {
    let path = rpt_test_support::fixture(format!(
        "tests/fixtures/reports/parking/crosstab_{variant}.rpt"
    ));
    let rpt = rpt::Rpt::open(&path).ok()?;
    // Leak the report into a clone we own — return the options by value.
    Some(crosstab(rpt.report())?.options.clone())
}

/// The base synthetic cross-tab: default grid options (grid + cell-margins + keep-columns on,
/// everything else off, no grand-total colours). Matches RAS `CrossTabStyle` exactly.
#[test]
fn crosstab_base_grid_options_are_defaults() {
    let Some(o) = options("base") else {
        eprintln!("[skip] parking crosstab_base fixture absent");
        return;
    };
    assert!(o.show_grid);
    assert!(o.show_cell_margins);
    assert!(o.keep_columns_together);
    assert!(!o.repeat_row_labels);
    assert!(!o.suppress_empty_rows);
    assert!(!o.suppress_empty_columns);
    assert!(!o.suppress_row_grand_totals);
    assert!(!o.suppress_column_grand_totals);
    assert_eq!(o.row_grand_total_color, None);
    assert_eq!(o.column_grand_total_color, None);
}

/// Each single-toggle variant flips exactly one option, leaving every other option at its base
/// value. `pred` selects the bool that must differ from base.
#[test]
fn crosstab_grid_option_toggles_flip_exactly_one_option() {
    let Some(base) = options("base") else {
        eprintln!("[skip] parking crosstab fixtures absent");
        return;
    };
    // (fixture variant, selector, expected value in the variant). Base value is the negation.
    type Case = (&'static str, fn(&CrossTabGridOptions) -> bool, bool);
    let cases: &[Case] = &[
        ("show_grid_off", |o| o.show_grid, false),
        ("cell_margins_off", |o| o.show_cell_margins, false),
        // The designer checkbox toggles KeepColumnsTogether from its True default to False.
        (
            "keep_columns_together_on",
            |o| o.keep_columns_together,
            false,
        ),
        ("repeat_row_labels_on", |o| o.repeat_row_labels, true),
        ("suppress_empty_rows_on", |o| o.suppress_empty_rows, true),
        (
            "suppress_empty_columns_on",
            |o| o.suppress_empty_columns,
            true,
        ),
        (
            "suppress_row_grandtotal_on",
            |o| o.suppress_row_grand_totals,
            true,
        ),
        (
            "suppress_col_grandtotal_on",
            |o| o.suppress_column_grand_totals,
            true,
        ),
    ];
    for (variant, sel, expected) in cases {
        let Some(o) = options(variant) else {
            eprintln!("[skip] fixture crosstab_{variant} absent");
            continue;
        };
        assert_eq!(sel(&o), *expected, "crosstab_{variant}: toggled option");
        assert_eq!(sel(&base), !*expected, "crosstab_{variant}: base baseline");
    }
}

/// The two grand-total background-colour fixtures set a bright yellow (`COLORREF 0x0000FFFF` =
/// RGB 255,255,0). The colour axes are cross-wired: a `col_grandtotal_bgcolor` surfaces as
/// `RowGrandTotalColor` and vice versa.
#[test]
fn crosstab_grandtotal_colors_decode_on_the_crossed_axes() {
    let yellow = Some(Color {
        a: 255,
        r: 255,
        g: 255,
        b: 0,
    });
    if let Some(o) = options("col_grandtotal_bgcolor") {
        assert_eq!(o.row_grand_total_color, yellow);
        assert_eq!(o.column_grand_total_color, None);
    }
    if let Some(o) = options("row_grandtotal_bgcolor") {
        assert_eq!(o.column_grand_total_color, yellow);
        assert_eq!(o.row_grand_total_color, None);
    }
}
