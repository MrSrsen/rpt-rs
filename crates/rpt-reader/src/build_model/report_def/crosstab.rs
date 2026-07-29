//! Cross-tab decode — the dimension levels, the `0x0143 CrossTabGridFormat` count and
//! the `0x0145 CrossTabGridCellFormat` per-region records it opens, the per-axis level counts on
//! the `0x00ce` / `0x00d2` level records, and the assembly of a decoded cross-tab onto its object.
//!
//! The data-cell **measures** are not here: the file records no link from a summary definition to
//! the cross-tab that aggregates it, so attributing them is a consumer's inference
//! (`rpt_model::crosstab_measures`), not a reading of these records.

use super::bindings::{binding_scopes, GridBindings};
use crate::build_model::record_values::colorref;
use crate::build_model::row_of;
use crate::codec::RecordNode;
use crate::field_table::table::Row;
use crate::field_table::tables as ft;
use crate::model::{
    Color, CrossTabCellFormat, CrossTabDimension, CrossTabGridFormat, CrossTabGridOptions,
    GroupCondition,
};
use crate::records::rtype::*;

/// The report-wide inputs every cross-tab's assembly draws on: the decoded dimension structures and
/// grid formatting, keyed by object name.
pub(super) struct CrossTabAttach {
    dimensions: std::collections::HashMap<String, CrossTabStructure>,
    grids: std::collections::HashMap<String, CrossTabGrid>,
}

impl CrossTabAttach {
    pub(super) fn new(tree: &[RecordNode], logical: &[u8]) -> Self {
        CrossTabAttach {
            dimensions: collect_crosstab_dimensions(tree, logical),
            grids: collect_crosstab_grid(tree, logical),
        }
    }

    /// Assemble one cross-tab object from its decoded dimensions, grid formatting and bindings.
    ///
    /// Every cross-tab grid binding is a row/column dimension — there is no data role here.
    pub(super) fn attach(
        &mut self,
        name: &str,
        ct: &mut crate::model::CrossTabObject,
        bindings: Option<&GridBindings>,
    ) {
        if let Some(refs) = bindings {
            ct.field_refs = refs.category.clone();
        }
        if let Some(s) = self.dimensions.remove(name) {
            ct.dimensions = s.dimensions;
            ct.columns = s.columns;
            ct.rows = s.rows;
            if let Some(b) = bindings {
                fill_axis(
                    &mut ct.columns,
                    &b.crosstab_columns,
                    &b.crosstab_column_periods,
                    &b.crosstab_column_suppress,
                );
                fill_axis(
                    &mut ct.rows,
                    &b.crosstab_rows,
                    &b.crosstab_row_periods,
                    &b.crosstab_row_suppress,
                );
            }
            // Axes are cross-wired as the SDK exposes them: the column-axis grand-total level's
            // color is RAS `RowGrandTotalColor`, and vice versa.
            ct.options.row_grand_total_color = s.column_gt_color;
            ct.options.column_grand_total_color = s.row_gt_color;
        }
        if let Some(g) = self.grids.remove(name) {
            ct.grid_format = g.grid_format;
            ct.column_level_count = g.column_level_count;
            ct.row_level_count = g.row_level_count;
            ct.options.show_grid = g.options.show_grid;
            ct.options.show_cell_margins = g.options.show_cell_margins;
            ct.options.keep_columns_together = g.options.keep_columns_together;
            ct.options.repeat_row_labels = g.options.repeat_row_labels;
            ct.options.suppress_empty_rows = g.options.suppress_empty_rows;
            ct.options.suppress_empty_columns = g.options.suppress_empty_columns;
            ct.options.suppress_row_grand_totals = g.options.suppress_row_grand_totals;
            ct.options.suppress_column_grand_totals = g.options.suppress_column_grand_totals;
        }
        // The RAS `CrossTabFormat.CrossTabStyle` view mirrors `options`, but reflects the
        // grand-total colors as concrete engine COLORREF colors: the "auto" default (stored
        // `0xFFFFFFFF`, decoded to `None` on `options`) surfaces as white.
        ct.grid_format.style = CrossTabGridOptions {
            row_grand_total_color: Some(ct.options.row_grand_total_color.unwrap_or(Color::WHITE)),
            column_grand_total_color: Some(
                ct.options.column_grand_total_color.unwrap_or(Color::WHITE),
            ),
            ..ct.options.clone()
        };
    }
}

/// Fill one axis's real (non-grand-total) levels from its axis-tagged grid groups.
///
/// The field reference is taken only when the `0x00cb` level record omits it (designer-authored
/// cross-tabs store the field only in the `0x00e5` grid group, not the level). The grouping period
/// and the two suppress flags are stored *only* on the grid group, so they are always taken. The
/// first level of each axis is the grand-total level (legitimately empty); the remaining levels are
/// the real dimensions, in the same order as the grid groups.
fn fill_axis(
    levels: &mut [CrossTabDimension],
    fields: &[String],
    periods: &[Option<GroupCondition>],
    suppress: &[(bool, bool)],
) {
    let groups = fields.iter().zip(periods.iter()).zip(suppress.iter());
    for (level, ((field, period), suppress)) in levels.iter_mut().skip(1).zip(groups) {
        if level.field_ref.is_empty() {
            level.field_ref = field.clone();
        }
        level.period = *period;
        (level.suppress_subtotal, level.suppress_label) = *suppress;
    }
}

/// One cross-tab object's decoded grid formatting: the cell-format run and its stated length, the
/// two per-axis level counts, and the grid display options (`0xb8`/`0xb9` records).
#[derive(Default)]
pub(super) struct CrossTabGrid {
    pub(super) grid_format: CrossTabGridFormat,
    pub(super) column_level_count: u16,
    pub(super) row_level_count: u16,
    pub(super) options: CrossTabGridOptions,
    /// Whether a `0x00d6` grid cell has already supplied the cell-margin setting.
    saw_grid_cell: bool,
}

/// Collect each cross-tab object's grid formatting, keyed by object name. Walks the `0xb9`-wrapper
/// scope (same skeleton as [`collect_crosstab_dimensions`]): within a cross-tab's block the
/// `0x0143` word opens a run of `0x0145` cell-format records, and the `0x00ce` / `0x00d2` level
/// records each carry a 2-byte per-axis level count (shared by every level of that axis; the first
/// seen is kept).
pub(super) fn collect_crosstab_grid(
    tree: &[RecordNode],
    logical: &[u8],
) -> std::collections::HashMap<String, CrossTabGrid> {
    let mut out: std::collections::HashMap<String, CrossTabGrid> = std::collections::HashMap::new();
    for (current, node) in binding_scopes(tree, logical, &[CROSSTAB_WRAPPER]) {
        let Some(name) = &current else { continue };
        match node.rtype {
            // The `0xb9` wrapper carries the two grand-total suppress flags.
            CROSSTAB_WRAPPER => {
                let row = row_of(node, logical, &ft::CROSSTAB_WRAPPER);
                let o = &mut out.entry(name.clone()).or_default().options;
                o.suppress_column_grand_totals = row.u("suppress_column_grand_totals") != 0;
                o.suppress_row_grand_totals = row.u("suppress_row_grand_totals") != 0;
            }
            // The `0xb8` opener carries the grid display booleans.
            CROSSTAB_OBJECT => {
                let row = row_of(node, logical, &ft::CROSSTAB_OBJECT);
                let o = &mut out.entry(name.clone()).or_default().options;
                o.show_grid = row.i("show_grid") != 0;
                o.keep_columns_together = row.i("keep_columns_together") != 0;
                o.repeat_row_labels = row.i("repeat_row_labels") != 0;
                o.suppress_empty_columns = row.i("suppress_empty_columns") != 0;
                o.suppress_empty_rows = row.i("suppress_empty_rows") != 0;
            }
            // Cell margins are stored as a twip margin on each grid cell rather than as a flag on
            // the opener: every cell of a grid carries the same pair, so the first one decides.
            CROSSTAB_GRID_CELL => {
                let row = row_of(node, logical, &ft::CROSSTAB_GRID_CELL);
                let g = out.entry(name.clone()).or_default();
                if !g.saw_grid_cell {
                    g.saw_grid_cell = true;
                    g.options.show_cell_margins = row.i("margin_h") != 0 || row.i("margin_v") != 0;
                }
            }
            CROSSTAB_GRID_FORMAT => {
                let row = row_of(node, logical, &ft::CROSSTAB_GRID_FORMAT);
                out.entry(name.clone()).or_default().grid_format.cell_count =
                    row.u("cell_count") as u16;
            }
            CROSSTAB_GRID_CELL_FORMAT => {
                let cell =
                    decode_cell_format(&row_of(node, logical, &ft::CROSSTAB_GRID_CELL_FORMAT));
                out.entry(name.clone())
                    .or_default()
                    .grid_format
                    .cells
                    .push(cell);
            }
            // Each axis record's level count follows its nested level record; it is shared by every
            // level of that axis, so the first seen is kept.
            CROSSTAB_COLUMN_AXIS => {
                let n = row_of(node, logical, &ft::CROSSTAB_COLUMN_AXIS).u("level_count") as u16;
                let g = out.entry(name.clone()).or_default();
                if g.column_level_count == 0 {
                    g.column_level_count = n;
                }
            }
            CROSSTAB_ROW_AXIS => {
                let n = row_of(node, logical, &ft::CROSSTAB_ROW_AXIS).u("level_count") as u16;
                let g = out.entry(name.clone()).or_default();
                if g.row_level_count == 0 {
                    g.row_level_count = n;
                }
            }
            _ => {}
        }
    }
    out
}

/// A dimension level's background color, from the `0x00cb` record's leading `COLORREF`
/// (`0x00BBGGRR`, big-endian). `0xFFFFFFFF` is the "auto" sentinel → `None`. Only the first level of
/// each axis (the grand-total level) has one the model keeps.
fn dimension_color(row: &Row) -> Option<Color> {
    let c = row.u("background_color");
    (c != 0xFFFF_FFFF).then(|| colorref(c))
}

/// Build a `0x0145 CrossTabGridCellFormat` from its decoded row: the region's leading word, its
/// background color and its enabled flag.
///
/// The color is read as the whole `COLORREF` the engine loads; a region with no explicit background
/// reads zero and yields `None`.
fn decode_cell_format(row: &Row) -> CrossTabCellFormat {
    let color = row.u("background_color");
    CrossTabCellFormat {
        flags: row.i("flags") as u32,
        background_color: (color != 0).then(|| colorref(color)),
        enabled: row.i("enabled") != 0,
    }
}

/// Collect each cross-tab object's **dimension structure** — the `0x00cb` `CrossTabDimensionField`
/// records between a cross-tab's `0xb9` wrapper and the next layout marker, keyed by the cross-tab's
/// object name, split by axis. A level nested under a `0x00ce CrossTabDimension` is a **column**
/// (its generated field objects are named `Column #N`); one nested under a `0x00d2 CrossTabRecord`
/// is a **row** (`Row #N`). Levels are emitted in the stream as all columns then all rows; the first
/// level of each axis is the grand-total level (empty field reference). Distinct from
/// [`super::bindings::collect_grid_bindings`], which reads the `0xe5` grid groups into one flat binding
/// list; this preserves the row/column split and grand-total levels for the model.
pub(super) fn collect_crosstab_dimensions(
    tree: &[RecordNode],
    logical: &[u8],
) -> std::collections::HashMap<String, CrossTabStructure> {
    let mut out: std::collections::HashMap<String, CrossTabStructure> =
        std::collections::HashMap::new();
    // The axis whose `0x00cb` levels are currently being read: `true` = column (opened by a
    // `0x00ce CrossTabDimension`), `false` = row (opened by a `0x00d2 CrossTabRecord`). Written in
    // the stream as all column levels then all row levels.
    let mut is_column = true;
    for (current, node) in binding_scopes(tree, logical, &[CROSSTAB_WRAPPER]) {
        match node.rtype {
            CROSSTAB_WRAPPER => is_column = true,
            CROSSTAB_COLUMN_AXIS => is_column = true,
            CROSSTAB_ROW_AXIS => is_column = false,
            CROSSTAB_DIM_FIELD => {
                if let Some(name) = &current {
                    let row = row_of(node, logical, &ft::CROSSTAB_DIM_FIELD);
                    // Neither the grouping period nor the two suppress flags are on the `0x00cb`
                    // level record; both are filled from the dimension's `0x00e5` grid group by
                    // [`fill_axis`].
                    let dim = crate::model::CrossTabDimension {
                        field_ref: row.text("field_ref").to_owned(),
                        period: None,
                        suppress_subtotal: false,
                        suppress_label: false,
                    };
                    let s = out.entry(name.clone()).or_default();
                    s.dimensions.push(dim.clone());
                    if is_column {
                        // The first column level is the grand-total level; its color is what the
                        // SDK exposes as `RowGrandTotalColor` (axes cross-wired, see model).
                        if s.columns.is_empty() {
                            s.column_gt_color = dimension_color(&row);
                        }
                        s.columns.push(dim);
                    } else {
                        if s.rows.is_empty() {
                            s.row_gt_color = dimension_color(&row);
                        }
                        s.rows.push(dim);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// One cross-tab object's decoded dimension structure, split by axis. `dimensions` is every
/// `0x00cb` level in stream order (columns then rows); `columns`/`rows` are the same levels split by
/// their parent axis record (`0x00ce` vs `0x00d2`).
#[derive(Default)]
pub(super) struct CrossTabStructure {
    pub(super) dimensions: Vec<crate::model::CrossTabDimension>,
    pub(super) columns: Vec<crate::model::CrossTabDimension>,
    pub(super) rows: Vec<crate::model::CrossTabDimension>,
    /// Background color of the first column-axis level (the column grand-total pseudo-field) —
    /// the SDK's `RowGrandTotalColor` (the color axes are cross-wired; see [`CrossTabGridOptions`]).
    pub(super) column_gt_color: Option<crate::model::Color>,
    /// Background color of the first row-axis level — the SDK's `ColumnGrandTotalColor`.
    pub(super) row_gt_color: Option<crate::model::Color>,
}

#[cfg(test)]
mod tests {
    use super::{decode_cell_format, dimension_color, ft};
    use crate::field_table::cursor::{Piece, RecordContent, StringFormat};
    use crate::field_table::table::{read_strings, Row, Table};
    use crate::model::Color;

    /// Read a synthetic run through a record's own field table.
    ///
    /// A record built here carries no header to declare a string form, so the reading names the
    /// enhanced form — the one the record-tree reader admits — rather than leaving it assumed.
    fn row_of_run(table: &Table, run: &[u8]) -> Row {
        read_strings(
            table,
            &RecordContent {
                rtype: table.rtype,
                schema: 0x0700,
                pieces: vec![Piece::Run(run.to_vec())],
            },
            StringFormat::Enhanced,
        )
        .row
    }

    #[test]
    fn default_region_has_no_color() {
        // A grid-region format with no explicit formatting: flags 0, no color, disabled.
        let run = [
            0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let c = decode_cell_format(&row_of_run(&ft::CROSSTAB_GRID_CELL_FORMAT, &run));
        assert_eq!(c.flags, 0);
        assert_eq!(c.background_color, None);
        assert!(!c.enabled);
    }

    #[test]
    fn the_enabled_flag_closes_the_region() {
        let run = [
            0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
        ];
        assert!(decode_cell_format(&row_of_run(&ft::CROSSTAB_GRID_CELL_FORMAT, &run)).enabled);
    }

    /// The region's color is one whole `COLORREF`, decoded like every other one in the format —
    /// not three channel bytes with padding either side.
    #[test]
    fn a_styled_region_reads_its_color_as_one_colorref() {
        let run = [
            0x00, 0x00, 0x00, 0x28, 0x01, 0x00, 0x44, 0x98, 0xff, 0x00, 0x01,
        ];
        let c = decode_cell_format(&row_of_run(&ft::CROSSTAB_GRID_CELL_FORMAT, &run));
        assert_eq!(c.flags, 0x28);
        assert_eq!(
            c.background_color,
            Some(Color {
                a: 255,
                r: 0xff,
                g: 0x98,
                b: 0x44,
            })
        );
        assert!(c.enabled);
    }

    fn lp(s: &str) -> Vec<u8> {
        let mut v = ((s.len() + 1) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v.push(0);
        v
    }

    /// A dimension level's field reference sits at a stated position past the header, and its
    /// leading `COLORREF` is the level's background color.
    #[test]
    fn dimension_level_reads_its_field_ref_and_color() {
        let mut run = vec![0x00, 0x44, 0x98, 0xff];
        run.resize(27, 0);
        run.extend(lp("Data.Date1"));
        let row = row_of_run(&ft::CROSSTAB_DIM_FIELD, &run);
        assert_eq!(row.text("field_ref"), "Data.Date1");
        assert_eq!(
            dimension_color(&row),
            Some(Color {
                a: 255,
                r: 0xff,
                g: 0x98,
                b: 0x44,
            })
        );
    }

    /// A grand-total level stores an empty field reference — a length-prefixed lone NUL, not an
    /// absent field — and the "auto" color sentinel.
    #[test]
    fn grand_total_level_stores_an_empty_field_ref() {
        let mut run = vec![0xff, 0xff, 0xff, 0xff];
        run.resize(27, 0);
        run.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x00]);
        let row = row_of_run(&ft::CROSSTAB_DIM_FIELD, &run);
        assert_eq!(row.text("field_ref"), "");
        assert_eq!(dimension_color(&row), None);
    }
}
