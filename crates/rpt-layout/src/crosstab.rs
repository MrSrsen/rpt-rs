//! Render a cross-tab object as a native Page-IR grid.
//!
//! A cross-tab pivots the data by one or more **row** dimensions (down the left) × **column**
//! dimensions (across the top), with a **measure** (an aggregate — e.g. `Sum of {amount}`) in each
//! cell. The decode exposes `CrossTabObject::{rows, columns, measures}`; the layout
//! engine computes the pivot from the dataset and this module draws it as ordinary [`DrawOp`]s
//! (cell rects + grid lines + text), so it renders identically through every backend with no new
//! dependency (the same approach as the chart renderer).
//!
//! Supports one row dimension × one column dimension × the first measure. Nested multi-level axes
//! are not implemented.

use rpt_model::{Color, Rect, Twips};
use rpt_pages::{
    DrawOp, FontSpec, LineOp, LineStyle, ObjectKind, ObjectRef, Point, RectOp, Stroke, TextAlign,
    TextRun,
};

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

/// The computed pivot to draw: the corner label, the column headers (across the top), the row
/// headers (down the left), and `cells[r][c]` = the formatted measure for (row r, column c).
pub(crate) struct Grid {
    pub corner: String,
    pub col_headers: Vec<String>,
    pub row_headers: Vec<String>,
    pub cells: Vec<Vec<String>>,
}

/// Draw the cross-tab `grid` inside `rect` (twips) as cell rects, grid lines, and text. The header
/// row/column are shaded; data cells are right-aligned (measures are numeric).
pub(crate) fn grid_ops(rect: Rect, grid: &Grid, section_name: &str, obj_name: &str) -> Vec<DrawOp> {
    let ncols = grid.col_headers.len() + 1; // +1 for the row-header column
    let nrows = grid.row_headers.len() + 1; // +1 for the column-header row
    if ncols < 2 || nrows < 2 {
        return Vec::new();
    }
    let src = || Some(ObjectRef::new(section_name, ObjectKind::CrossTab).named(obj_name));
    let mut ops: Vec<DrawOp> = Vec::new();

    let (rl, rt, rw, rh) = (rect.left.0, rect.top.0, rect.width.0, rect.height.0);
    let col_w = (rw / ncols as i32).max(1);
    let row_h = (rh / nrows as i32).max(1);
    let cell_x = |c: usize| rl + c as i32 * col_w;
    let cell_y = |r: usize| rt + r as i32 * row_h;

    // Header shading: the top row and the left column.
    let shade = |ops: &mut Vec<DrawOp>, x: i32, y: i32, w: i32, h: i32| {
        ops.push(DrawOp::Rect(RectOp {
            bounds: Rect {
                left: Twips(x),
                top: Twips(y),
                width: Twips(w),
                height: Twips(h),
            },
            fill: Some(HEADER_FILL.into()),
            stroke: None,
            corner_radius: Twips(0),
            source: src(),
        }));
    };
    shade(&mut ops, rl, rt, col_w * ncols as i32, row_h); // top row
    shade(&mut ops, rl, rt, col_w, row_h * nrows as i32); // left column

    // Grid lines (horizontal + vertical), enclosing the whole grid.
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
            stroke: Stroke {
                color: GRID,
                width: Twips(10),
                style: LineStyle::Single,
            },
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

    // Text: a cell's content, aligned, clipped to its box (a small inset).
    let pad = 40;
    let text =
        |ops: &mut Vec<DrawOp>, c: usize, r: usize, s: &str, align: TextAlign, bold: bool| {
            if s.is_empty() {
                return;
            }
            ops.push(DrawOp::Text(TextRun {
                bounds: Rect {
                    left: Twips(cell_x(c) + pad),
                    top: Twips(cell_y(r) + pad),
                    width: Twips(col_w - 2 * pad),
                    height: Twips(row_h - 2 * pad),
                },
                text: truncate(s, 24),
                font: FontSpec {
                    family: "Arial".into(),
                    size_pt: 8.0,
                    bold,
                    ..Default::default()
                },
                color: TEXT,
                align,
                rotation: 0.0,
                metrics: None,
                source: src(),
            }));
        };

    // Corner + column headers (top row).
    text(&mut ops, 0, 0, &grid.corner, TextAlign::Left, true);
    for (c, h) in grid.col_headers.iter().enumerate() {
        text(&mut ops, c + 1, 0, h, TextAlign::Center, true);
    }
    // Row headers (left column) + data cells.
    for (r, rh) in grid.row_headers.iter().enumerate() {
        text(&mut ops, 0, r + 1, rh, TextAlign::Left, true);
        for c in 0..grid.col_headers.len() {
            let v = grid
                .cells
                .get(r)
                .and_then(|row| row.get(c))
                .map(String::as_str)
                .unwrap_or("");
            text(&mut ops, c + 1, r + 1, v, TextAlign::Right, false);
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
    /// first row dimension × first column dimension, aggregating its first measure into each cell.
    /// Falls back to a placeholder + diagnostic when the pivot structure or data is missing.
    pub(crate) fn emit_crosstab(
        &mut self,
        ct: &rpt_model::CrossTabObject,
        rect: Rect,
        section_name: &str,
        obj: &rpt_model::ReportObject,
    ) {
        let row_field = ct.rows.iter().find(|d| !d.field_ref.is_empty());
        let col_field = ct.columns.iter().find(|d| !d.field_ref.is_empty());
        let (Some(row_field), Some(col_field), Some(measure)) =
            (row_field, col_field, ct.measures.first())
        else {
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
        let grid = crate::aggregate::crosstab_pivot(
            self.dataset,
            self.formulas,
            &self.locale,
            &row_field.field_ref,
            &col_field.field_ref,
            measure,
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
        for op in grid_ops(rect, &grid, section_name, &obj.name) {
            self.cur.push(op);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_grid_yields_no_ops() {
        let g = Grid {
            corner: "x".into(),
            col_headers: vec![],
            row_headers: vec![],
            cells: vec![],
        };
        assert!(grid_ops(
            Rect {
                left: Twips(0),
                top: Twips(0),
                width: Twips(3000),
                height: Twips(2000)
            },
            &g,
            "S",
            "CT"
        )
        .is_empty());
    }

    #[test]
    fn draws_headers_and_cells() {
        let g = Grid {
            corner: "Region".into(),
            col_headers: vec!["Q1".into(), "Q2".into()],
            row_headers: vec!["East".into(), "West".into()],
            cells: vec![
                vec!["10".into(), "20".into()],
                vec!["30".into(), "40".into()],
            ],
        };
        let ops = grid_ops(
            Rect {
                left: Twips(0),
                top: Twips(0),
                width: Twips(6000),
                height: Twips(4000),
            },
            &g,
            "RH",
            "CT1",
        );
        let texts = ops.iter().filter(|o| matches!(o, DrawOp::Text(_))).count();
        // corner + 2 col headers + 2 row headers + 4 data cells = 9
        assert_eq!(texts, 9, "all headers + cells drawn");
        // A 3×3 grid → 4 horizontal + 4 vertical lines.
        let lines = ops.iter().filter(|o| matches!(o, DrawOp::Line(_))).count();
        assert_eq!(lines, 8);
    }
}
