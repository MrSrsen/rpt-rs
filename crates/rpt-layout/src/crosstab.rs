//! Render a cross-tab object as a native Page-IR grid.
//!
//! A cross-tab pivots the data by one or more **row** dimensions (down the left) × **column**
//! dimensions (across the top), with a **measure** (an aggregate — e.g. `Sum of {amount}`) in each
//! cell. The decode exposes `CrossTabObject::{rows, columns, measures}`; the layout
//! engine computes the pivot from the dataset and this module draws it as ordinary [`DrawOp`]s
//! (cell rects + grid lines + text), so it renders identically through every backend with no new
//! dependency (the same approach as the chart renderer).
//!
//! Supports one row dimension × one column dimension × every measure (measures are drawn stacked in
//! each cell). Nested multi-level axes are not implemented.

use crate::place::{fill_rect, hairline_stroke};
use rpt_model::{Color, CrossTabGridOptions, Rect, Twips};
use rpt_pages::{DrawOp, FontSpec, LineOp, ObjectKind, ObjectRef, Point, TextAlign, TextRun};

const HEADER_FILL: Color = Color {
    a: 255,
    r: 0xe8,
    g: 0xe8,
    b: 0xe8,
};
const GRID: Color = Color {
    a: 255,
    r: 0x99,
    g: 0x99,
    b: 0x99,
};
const TEXT: Color = Color {
    a: 255,
    r: 0x22,
    g: 0x22,
    b: 0x22,
};
/// Inset from a cell's edge to its text, in twips.
const CELL_PAD: i32 = 40;
/// Cross-tab cell font size, in points.
const CELL_FONT_PT: f32 = 8.0;
/// Maximum characters kept in a cell label before eliding.
const LABEL_MAX: usize = 24;

/// The computed pivot to draw: the corner label, the column headers (across the top), the row
/// headers (down the left), and `cells[r][c]` = the formatted measure for (row r, column c). The
/// grand totals are the measure re-aggregated across a whole axis: [`row_totals`](Self::row_totals)
/// one per row (drawn as the grand-total column), [`col_totals`](Self::col_totals) one per column
/// (drawn as the grand-total row), and [`grand_total`](Self::grand_total) over everything.
pub(crate) struct Grid {
    pub corner: String,
    pub col_headers: Vec<String>,
    pub row_headers: Vec<String>,
    pub cells: Vec<Vec<String>>,
    /// Per-row totals (the measure across every column), one per row header — the grand-total column.
    pub row_totals: Vec<String>,
    /// Per-column totals (the measure across every row), one per column header — the grand-total row.
    pub col_totals: Vec<String>,
    /// The grand grand-total, over every row and column.
    pub grand_total: String,
}

/// One logical grid cell: its text, alignment, weight, and background fill.
struct Cell {
    text: String,
    align: TextAlign,
    bold: bool,
    fill: Option<Color>,
}

/// Draw the cross-tab `grid` inside `rect` (twips) as cell rects, grid lines, and text. The header
/// row/column are shaded; data cells are right-aligned (measures are numeric). The decoded grid
/// [`options`](CrossTabGridOptions) drive the display: grid lines are emitted only when
/// [`show_grid`](CrossTabGridOptions::show_grid) is set.
///
/// The grand totals sit first — the grand-total column right after the row labels, the grand-total
/// row right below the column labels — matching the engine's layout. Their colours are supplied on
/// the axis RAS names them by, which is the opposite axis to where they draw: `RowGrandTotalColor`
/// ([`row_grand_total_color`](CrossTabGridOptions::row_grand_total_color)) fills the grand-total
/// *column* (the per-row totals), `ColumnGrandTotalColor` the grand-total *row*. Each grand total is
/// dropped when its suppress flag is set — `EnableSuppressRowGrandTotals` drops the column,
/// `EnableSuppressColumnGrandTotals` the row (same axis convention as the colours).
pub(crate) fn grid_ops(
    rect: Rect,
    grid: &Grid,
    opts: &CrossTabGridOptions,
    section_name: &str,
    obj_name: &str,
    base_instance: u32,
) -> Vec<DrawOp> {
    if grid.col_headers.is_empty() || grid.row_headers.is_empty() {
        return Vec::new();
    }
    // Which grand-total bands are drawn, and the fixed indices they occupy (right after the labels).
    let has_gt_col = !grid.row_totals.is_empty() && !opts.suppress_row_grand_totals;
    let has_gt_row = !grid.col_totals.is_empty() && !opts.suppress_column_grand_totals;
    let gt_col = 1usize;
    let gt_row = 1usize;
    let data_c0 = 1 + has_gt_col as usize;
    let data_r0 = 1 + has_gt_row as usize;
    let ncols = data_c0 + grid.col_headers.len();
    let nrows = data_r0 + grid.row_headers.len();

    // Build the logical grid: text/alignment/weight per cell, then the background fills.
    let blank = || Cell {
        text: String::new(),
        align: TextAlign::Left,
        bold: false,
        fill: None,
    };
    let mut m: Vec<Vec<Cell>> = (0..nrows)
        .map(|_| (0..ncols).map(|_| blank()).collect())
        .collect();
    let put =
        |m: &mut Vec<Vec<Cell>>, r: usize, c: usize, s: &str, align: TextAlign, bold: bool| {
            m[r][c].text = s.to_string();
            m[r][c].align = align;
            m[r][c].bold = bold;
        };

    // Header labels: corner, the grand-total column header, and the column headers.
    put(&mut m, 0, 0, &grid.corner, TextAlign::Left, true);
    if has_gt_col {
        put(&mut m, 0, gt_col, "Total", TextAlign::Center, true);
    }
    for (c, h) in grid.col_headers.iter().enumerate() {
        put(&mut m, 0, data_c0 + c, h, TextAlign::Center, true);
    }
    // Grand-total row (per-column totals) and its label + the corner grand total.
    if has_gt_row {
        put(&mut m, gt_row, 0, "Total", TextAlign::Left, true);
        if has_gt_col {
            put(
                &mut m,
                gt_row,
                gt_col,
                &grid.grand_total,
                TextAlign::Right,
                false,
            );
        }
        for (c, v) in grid.col_totals.iter().enumerate() {
            put(&mut m, gt_row, data_c0 + c, v, TextAlign::Right, false);
        }
    }
    // Row labels, the grand-total column (per-row totals), and the data cells.
    for (r, rh) in grid.row_headers.iter().enumerate() {
        let rr = data_r0 + r;
        put(&mut m, rr, 0, rh, TextAlign::Left, true);
        if has_gt_col {
            let v = grid.row_totals.get(r).map(String::as_str).unwrap_or("");
            put(&mut m, rr, gt_col, v, TextAlign::Right, false);
        }
        for c in 0..grid.col_headers.len() {
            let v = grid
                .cells
                .get(r)
                .and_then(|row| row.get(c))
                .map(String::as_str)
                .unwrap_or("");
            put(&mut m, rr, data_c0 + c, v, TextAlign::Right, false);
        }
    }

    // Fills: header shading, then the grand-total colours (which override the header grey on their
    // own band). The grand-total colours are cross-wired to the drawn axis — see the fn doc.
    for (r, row) in m.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            let mut fill = (r == 0 || c == 0).then_some(HEADER_FILL);
            if has_gt_col && c == gt_col {
                fill = opts.row_grand_total_color.or(fill);
            }
            if has_gt_row && r == gt_row {
                fill = opts.column_grand_total_color.or(fill);
            }
            cell.fill = fill;
        }
    }

    let src = || Some(ObjectRef::new(section_name, ObjectKind::CrossTab).named(obj_name));
    let (rl, rt, rw, rh) = (rect.left.0, rect.top.0, rect.width.0, rect.height.0);
    let col_w = (rw / ncols as i32).max(1);
    let row_h = (rh / nrows as i32).max(1);
    let cell_x = |c: usize| rl + c as i32 * col_w;
    let cell_y = |r: usize| rt + r as i32 * row_h;
    let mut ops: Vec<DrawOp> = Vec::new();

    // Background fills.
    for (r, row) in m.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if let Some(color) = cell.fill {
                ops.push(fill_rect(
                    Rect {
                        left: Twips(cell_x(c)),
                        top: Twips(cell_y(r)),
                        width: Twips(col_w),
                        height: Twips(row_h),
                    },
                    color,
                    src(),
                ));
            }
        }
    }

    // Grid lines (horizontal + vertical), enclosing the whole grid. Emitted only when the decoded
    // `EnableShowGrid` option is set; a grid-off cross-tab draws its shaded headers and cell text
    // with no interior or bounding rules.
    if opts.show_grid {
        let line = |ops: &mut Vec<DrawOp>, x1: i32, y1: i32, x2: i32, y2: i32| {
            ops.push(DrawOp::Line(LineOp {
                from: Point {
                    x: Twips(x1),
                    y: Twips(y1),
                },
                to: Point {
                    x: Twips(x2),
                    y: Twips(y2),
                },
                stroke: hairline_stroke(GRID),
                source: src(),
            }));
        };
        let right = cell_x(ncols);
        let bottom = cell_y(nrows);
        for r in 0..=nrows {
            line(&mut ops, rl, cell_y(r), right, cell_y(r));
        }
        for c in 0..=ncols {
            line(&mut ops, cell_x(c), rt, cell_x(c), bottom);
        }
    }

    // Text: each cell's content, aligned, clipped to its box (a small inset). A cell may stack
    // several measures (`\n`-joined), each drawn on its own line, splitting the cell height evenly.
    // Each drawn line gets its own per-placement instance id so the HTML backend keeps it a separate
    // positioned element (like the engine) rather than merging a cell's stacked measures into one
    // multi-line paragraph.
    let mut inst = base_instance;
    for (r, row) in m.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if cell.text.is_empty() {
                continue;
            }
            let lines: Vec<&str> = cell.text.split('\n').collect();
            let inner_h = row_h - 2 * CELL_PAD;
            let line_h = (inner_h / lines.len() as i32).max(1);
            for (li, line) in lines.iter().enumerate() {
                if line.is_empty() {
                    continue;
                }
                let source = Some(
                    ObjectRef::new(section_name, ObjectKind::CrossTab)
                        .named(obj_name)
                        .with_instance(inst),
                );
                inst += 1;
                ops.push(DrawOp::Text(TextRun {
                    bounds: Rect {
                        left: Twips(cell_x(c) + CELL_PAD),
                        top: Twips(cell_y(r) + CELL_PAD + li as i32 * line_h),
                        width: Twips(col_w - 2 * CELL_PAD),
                        height: Twips(line_h),
                    },
                    text: truncate(line, LABEL_MAX),
                    font: FontSpec {
                        family: "Arial".into(),
                        size_pt: CELL_FONT_PT,
                        bold: cell.bold,
                        ..Default::default()
                    },
                    color: TEXT,
                    align: cell.align,
                    rotation: 0.0,
                    metrics: None,
                    source,
                }));
            }
        }
    }

    ops
}

/// Truncate a label to `max` chars with an ellipsis (char-safe).
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

impl crate::Formatter<'_> {
    /// Render a cross-tab object as a native grid: pivot the dataset by the cross-tab's
    /// first row dimension × first column dimension, aggregating every measure (stacked) into each
    /// cell. Falls back to a placeholder + diagnostic when the pivot structure or data is missing.
    pub(crate) fn emit_crosstab(
        &mut self,
        ct: &rpt_model::CrossTabObject,
        rect: Rect,
        section_name: &str,
        obj: &rpt_model::ReportObject,
    ) {
        let row_field = ct.rows.iter().find(|d| !d.field_ref.is_empty());
        let col_field = ct.columns.iter().find(|d| !d.field_ref.is_empty());
        let (Some(row_field), Some(col_field)) = (row_field, col_field) else {
            crate::push_diag(
                &self.diagnostics,
                rpt_pages::Diagnostic::warn(
                    rpt_pages::DiagnosticKind::UnsupportedObject,
                    "cross-tab is missing a row/column dimension or measure; rendered as a placeholder",
                )
                .with_source(&obj.name),
            );
            self.placeholder_box(rect, section_name, obj, ObjectKind::CrossTab);
            return;
        };
        if ct.measures.is_empty() {
            crate::push_diag(
                &self.diagnostics,
                rpt_pages::Diagnostic::warn(
                    rpt_pages::DiagnosticKind::UnsupportedObject,
                    "cross-tab is missing a row/column dimension or measure; rendered as a placeholder",
                )
                .with_source(&obj.name),
            );
            self.placeholder_box(rect, section_name, obj, ObjectKind::CrossTab);
            return;
        }
        // Every measure is drawn stacked in each data cell (the engine renders all of them).
        let grid = crate::aggregate::crosstab_pivot(
            self.dataset,
            self.formulas,
            &self.locale,
            row_field,
            col_field,
            &ct.measures,
            Some((&self.diagnostics, &obj.name)),
        );
        if grid.col_headers.is_empty() || grid.row_headers.is_empty() {
            crate::push_diag(
                &self.diagnostics,
                rpt_pages::Diagnostic::warn(
                    rpt_pages::DiagnosticKind::UnsupportedObject,
                    "cross-tab has no data to pivot; rendered as a placeholder",
                )
                .with_source(&obj.name),
            );
            self.placeholder_box(rect, section_name, obj, ObjectKind::CrossTab);
            return;
        }
        // Reserve a block of per-placement instance ids for the grid's text lines so each is a
        // separate positioned element downstream (unique across the report, no cross-object merge).
        let base_instance = self.next_instance_id;
        let ops = grid_ops(
            rect,
            &grid,
            &ct.options,
            section_name,
            &obj.name,
            base_instance,
        );
        self.next_instance_id += ops.iter().filter(|o| matches!(o, DrawOp::Text(_))).count() as u32;
        for op in ops {
            self.cur.push(op);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Grid options with the grid lines enabled (the visible default the render tests exercise).
    fn opts_show_grid() -> CrossTabGridOptions {
        CrossTabGridOptions {
            show_grid: true,
            ..Default::default()
        }
    }

    fn sample_grid() -> Grid {
        Grid {
            corner: "Region".into(),
            col_headers: vec!["Q1".into(), "Q2".into()],
            row_headers: vec!["East".into(), "West".into()],
            cells: vec![
                vec!["10".into(), "20".into()],
                vec!["30".into(), "40".into()],
            ],
            // Unique markers so a total is never confused with a body cell in the text assertions.
            row_totals: vec!["RTa".into(), "RTb".into()],
            col_totals: vec!["CTa".into(), "CTb".into()],
            grand_total: "GG".into(),
        }
    }

    fn rect() -> Rect {
        Rect {
            left: Twips(0),
            top: Twips(0),
            width: Twips(8000),
            height: Twips(8000),
        }
    }

    const RED: Color = Color {
        a: 255,
        r: 255,
        g: 0,
        b: 0,
    };
    const BLUE: Color = Color {
        a: 255,
        r: 0,
        g: 0,
        b: 255,
    };

    /// The text strings drawn, in op order.
    fn texts(ops: &[DrawOp]) -> Vec<String> {
        ops.iter()
            .filter_map(|o| match o {
                DrawOp::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect()
    }

    /// The solid-fill rects drawn, as `(color, left, top)`.
    fn fills(ops: &[DrawOp]) -> Vec<(Color, i32, i32)> {
        ops.iter()
            .filter_map(|o| match o {
                DrawOp::Rect(r) => match r.fill {
                    Some(rpt_pages::Fill::Solid(c)) => Some((c, r.bounds.left.0, r.bounds.top.0)),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn empty_grid_yields_no_ops() {
        let g = Grid {
            corner: "x".into(),
            col_headers: vec![],
            row_headers: vec![],
            cells: vec![],
            row_totals: vec![],
            col_totals: vec![],
            grand_total: String::new(),
        };
        assert!(grid_ops(rect(), &g, &opts_show_grid(), "S", "CT", 0).is_empty());
    }

    #[test]
    fn draws_headers_cells_and_grand_totals() {
        let ops = grid_ops(rect(), &sample_grid(), &opts_show_grid(), "RH", "CT1", 0);
        let t = texts(&ops);
        // 4×4 grid: corner + "Total" col header + 2 col headers; the grand-total row (label + grand
        // + 2 col totals); two data rows (label + row total + 2 cells) = 16 text cells.
        assert_eq!(t.len(), 16);
        // The grand totals are present.
        assert!(t.contains(&"GG".to_string()), "grand grand-total drawn");
        assert!(t.contains(&"CTa".to_string()), "a column total drawn");
        assert!(t.contains(&"RTa".to_string()), "a row total drawn");
        assert_eq!(
            t.iter().filter(|s| *s == "Total").count(),
            2,
            "two Total labels"
        );
        // A 4×4 grid → 5 horizontal + 5 vertical lines.
        let lines = ops.iter().filter(|o| matches!(o, DrawOp::Line(_))).count();
        assert_eq!(lines, 10);
    }

    #[test]
    fn row_grand_total_color_fills_the_grand_total_column() {
        // RAS `RowGrandTotalColor` is cross-wired: it colours the grand-total COLUMN (per-row
        // totals), which sits at grid column index 1 (right after the row labels).
        let opts = CrossTabGridOptions {
            show_grid: true,
            row_grand_total_color: Some(RED),
            ..Default::default()
        };
        let ops = grid_ops(rect(), &sample_grid(), &opts, "RH", "CT1", 0);
        let col_w = 8000 / 4; // ncols == 4
        let red: Vec<_> = fills(&ops)
            .into_iter()
            .filter(|(c, ..)| *c == RED)
            .collect();
        // Every cell of the grand-total column is red — all four rows, none elsewhere.
        assert_eq!(red.len(), 4, "grand-total column fully coloured");
        assert!(
            red.iter().all(|(_, left, _)| *left == col_w),
            "red fills land only on the grand-total column (index 1)"
        );
    }

    #[test]
    fn column_grand_total_color_fills_the_grand_total_row() {
        // RAS `ColumnGrandTotalColor` colours the grand-total ROW (per-column totals) at row index 1.
        let opts = CrossTabGridOptions {
            show_grid: true,
            column_grand_total_color: Some(BLUE),
            ..Default::default()
        };
        let ops = grid_ops(rect(), &sample_grid(), &opts, "RH", "CT1", 0);
        let row_h = 8000 / 4; // nrows == 4
        let blue: Vec<_> = fills(&ops)
            .into_iter()
            .filter(|(c, ..)| *c == BLUE)
            .collect();
        assert_eq!(blue.len(), 4, "grand-total row fully coloured");
        assert!(
            blue.iter().all(|(_, _, top)| *top == row_h),
            "blue fills land only on the grand-total row (index 1)"
        );
    }

    #[test]
    fn suppress_row_grand_totals_drops_the_column() {
        // `EnableSuppressRowGrandTotals` drops the grand-total COLUMN (the per-row totals).
        let opts = CrossTabGridOptions {
            show_grid: true,
            suppress_row_grand_totals: true,
            ..Default::default()
        };
        let ops = grid_ops(rect(), &sample_grid(), &opts, "RH", "CT1", 0);
        let t = texts(&ops);
        // The per-row totals and the corner grand total are gone; the per-column totals remain.
        assert!(!t.contains(&"RTa".to_string()) && !t.contains(&"RTb".to_string()));
        assert!(!t.contains(&"GG".to_string()), "corner grand total dropped");
        assert!(t.contains(&"CTa".to_string()), "column totals kept");
    }

    #[test]
    fn suppress_column_grand_totals_drops_the_row() {
        // `EnableSuppressColumnGrandTotals` drops the grand-total ROW (the per-column totals).
        let opts = CrossTabGridOptions {
            show_grid: true,
            suppress_column_grand_totals: true,
            ..Default::default()
        };
        let ops = grid_ops(rect(), &sample_grid(), &opts, "RH", "CT1", 0);
        let t = texts(&ops);
        assert!(!t.contains(&"CTa".to_string()) && !t.contains(&"CTb".to_string()));
        assert!(!t.contains(&"GG".to_string()), "corner grand total dropped");
        assert!(t.contains(&"RTa".to_string()), "row totals kept");
    }

    #[test]
    fn show_grid_off_suppresses_grid_lines() {
        let opts = CrossTabGridOptions::default();
        let ops = grid_ops(rect(), &sample_grid(), &opts, "RH", "CT1", 0);
        assert_eq!(
            ops.iter().filter(|o| matches!(o, DrawOp::Line(_))).count(),
            0,
            "grid lines suppressed"
        );
        // The headers, cells, and grand totals are still drawn (grid-off hides only the rules).
        assert_eq!(texts(&ops).len(), 16, "cells still drawn");
    }

    #[test]
    fn show_grid_toggle_only_affects_lines() {
        let on = grid_ops(rect(), &sample_grid(), &opts_show_grid(), "RH", "CT1", 0);
        let off = grid_ops(
            rect(),
            &sample_grid(),
            &CrossTabGridOptions::default(),
            "RH",
            "CT1",
            0,
        );
        let non_line =
            |ops: &[DrawOp]| ops.iter().filter(|o| !matches!(o, DrawOp::Line(_))).count();
        // Toggling the grid changes only the line ops; every other op (shading + text) is identical.
        assert_eq!(non_line(&on), non_line(&off));
    }
}
