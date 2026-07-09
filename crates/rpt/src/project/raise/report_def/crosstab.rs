//! Cross-tab grid formatting decode — the `0x0143 CrossTabGridFormat` word and the `0x0145
//! CrossTabGridCellFormat` per-region records, plus the per-axis option words on the `0x00ce` /
//! `0x00d2` level records. STRUCTURAL only (RptToXml never exports cross-tabs); verified against the
//! record tree, not an XML oracle.

use super::*;
use crate::model::{Color, CrossTabCellFormat, CrossTabGridFormat, CrossTabGridOptions};

/// One cross-tab object's decoded grid formatting: the grid-level format word + per-region cell
/// formats, the two per-axis option words, and the grid display options (`0xb8`/`0xb9` leaves).
#[derive(Default)]
pub(super) struct CrossTabGrid {
    pub(super) grid_format: CrossTabGridFormat,
    pub(super) column_axis_options: u16,
    pub(super) row_axis_options: u16,
    pub(super) options: CrossTabGridOptions,
}

/// Collect each cross-tab object's grid formatting, keyed by object name. Walks the `0xb9`-wrapper
/// scope (same skeleton as [`collect_crosstab_dimensions`]): within a cross-tab's block the
/// `0x0143` word opens a run of `0x0145` cell-format records, and the `0x00ce` / `0x00d2` level
/// records each carry a 2-byte per-axis option word (shared by every level of that axis; the first
/// seen is kept).
pub(super) fn collect_crosstab_grid(
    tree: &[RecordNode],
    logical: &[u8],
) -> std::collections::HashMap<String, CrossTabGrid> {
    let mut out: std::collections::HashMap<String, CrossTabGrid> = std::collections::HashMap::new();
    for (current, node) in binding_scopes(tree, logical, &[CROSSTAB_WRAPPER]) {
        let Some(name) = &current else { continue };
        match node.rtype {
            // The `0xb9` wrapper leaf carries the two grand-total suppress flags: own-leaf byte 1 =
            // SuppressColumnGrandTotals, byte 3 = SuppressRowGrandTotals (each a `00`/`01` bool).
            CROSSTAB_WRAPPER => {
                let leaf = node.leaf_bytes(logical);
                let o = &mut out.entry(name.clone()).or_default().options;
                o.suppress_column_grand_totals = leaf.get(1).copied().unwrap_or(0) != 0;
                o.suppress_row_grand_totals = leaf.get(3).copied().unwrap_or(0) != 0;
            }
            // The `0xb8` opener leaf carries the grid display booleans (own-leaf offsets, each a
            // `00`/`01` bool in a binary header that precedes the object's `0x9e` name child).
            CROSSTAB_OBJECT => {
                let leaf = node.leaf_bytes(logical);
                let o = &mut out.entry(name.clone()).or_default().options;
                o.show_grid = leaf.get(1).copied().unwrap_or(0) != 0;
                o.show_cell_margins = leaf.get(10).copied().unwrap_or(0) != 0;
                o.keep_columns_together = leaf.get(22).copied().unwrap_or(0) != 0;
                o.repeat_row_labels = leaf.get(24).copied().unwrap_or(0) != 0;
                o.suppress_empty_columns = leaf.get(26).copied().unwrap_or(0) != 0;
                o.suppress_empty_rows = leaf.get(28).copied().unwrap_or(0) != 0;
            }
            CROSSTAB_GRID_FORMAT => {
                let leaf = node.leaf_bytes(logical);
                out.entry(name.clone()).or_default().grid_format.raw =
                    u16_be(&leaf, 0).unwrap_or(0);
            }
            CROSSTAB_GRID_CELL_FORMAT => {
                let cell = decode_cell_format(&node.leaf_bytes(logical));
                out.entry(name.clone())
                    .or_default()
                    .grid_format
                    .cells
                    .push(cell);
            }
            CROSSTAB_COLUMN_AXIS => {
                let opt = u16_be(&node.leaf_bytes(logical), 0).unwrap_or(0);
                let g = out.entry(name.clone()).or_default();
                if g.column_axis_options == 0 {
                    g.column_axis_options = opt;
                }
            }
            CROSSTAB_ROW_AXIS => {
                let opt = u16_be(&node.leaf_bytes(logical), 0).unwrap_or(0);
                let g = out.entry(name.clone()).or_default();
                if g.row_axis_options == 0 {
                    g.row_axis_options = opt;
                }
            }
            _ => {}
        }
    }
    out
}

/// Decode a grand-total dimension level's background colour — the first `0x00cb` level's leaf
/// `[0..4]`, a big-endian `COLORREF` (`0x00BBGGRR`). `0xFFFFFFFF` is the "auto" sentinel → `None`.
/// (Big-endian, unlike the `0x0145` region colour which is a trailing-zero little-endian COLORREF.)
pub(super) fn decode_grandtotal_color(leaf: &[u8]) -> Option<Color> {
    let c = u32_be(leaf, 0)?;
    (c != 0xFFFF_FFFF).then_some(Color {
        a: 255,
        r: (c & 0xff) as u8,
        g: ((c >> 8) & 0xff) as u8,
        b: ((c >> 16) & 0xff) as u8,
    })
}

/// Decode a `0x0145 CrossTabGridCellFormat` leaf (11 bytes):
/// `[0..4]` format-override flags (big-endian; `0x28` on regions carrying explicit formatting),
/// `[4]`/`[5]` fixed, `[6..10]` the region background colour as a little-endian `COLORREF`
/// (`[R, G, B, 0]` — the trailing zero marks it little-endian, unlike the pad-first `0x0100`
/// FontColor), `[10]` the region enabled flag. The colour is not exposed by RAS/the HTML render
/// (the cross-tab grid-region template is engine-internal), so the `[R,G,B]` order is inferred from
/// the trailing-zero COLORREF layout, not oracle-verified.
fn decode_cell_format(leaf: &[u8]) -> CrossTabCellFormat {
    let flags = u32_be(leaf, 0).unwrap_or(0);
    // Background colour: a little-endian `COLORREF` at `[6..10]` = `[R, G, B, 0]`. A region with no
    // explicit background reads all-zero.
    let background_color = match (leaf.get(6), leaf.get(7), leaf.get(8)) {
        (Some(&r), Some(&g), Some(&b)) if (r, g, b) != (0, 0, 0) => Some(Color { a: 255, r, g, b }),
        _ => None,
    };
    CrossTabCellFormat {
        flags,
        background_color,
        enabled: leaf.get(10).copied().unwrap_or(0) != 0,
    }
}

/// The trailing length-prefixed string of a record leaf — the string whose framing ends exactly at
/// the leaf's end. Used to read a `0x00cb` cross-tab dimension's bound field reference, which follows
/// a fixed binary geometry/id header. Returns empty when there is no trailing string (a grand-total
/// dimension level).
fn trailing_lp_string(leaf: &[u8]) -> String {
    (0..leaf.len())
        .find_map(|off| match read_be_lp_string_lossy(leaf, off) {
            Some((s, used)) if off + used == leaf.len() => Some(s),
            _ => None,
        })
        .unwrap_or_default()
}

/// Collect each cross-tab object's **dimension structure** — the `0x00cb` `CrossTabDimensionField`
/// records between a cross-tab's `0xb9` wrapper and the next layout marker, keyed by the cross-tab's
/// object name, split by axis. A level nested under a `0x00ce CrossTabDimension` is a **column**
/// (its generated field objects are named `Column #N`); one nested under a `0x00d2 CrossTabRecord`
/// is a **row** (`Row #N`). Levels are emitted in the stream as all columns then all rows; the first
/// level of each axis is the grand-total level (empty field reference). Distinct from
/// [`super::grid::collect_grid_bindings`], which reads the `0xe5` grid groups for `Field.UseCount`;
/// this preserves the row/column split and grand-total levels for the model.
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
                    let leaf = node.leaf_bytes(logical);
                    let field_ref = trailing_lp_string(&leaf);
                    let dim = crate::model::CrossTabDimension { field_ref };
                    let s = out.entry(name.clone()).or_default();
                    s.dimensions.push(dim.clone());
                    if is_column {
                        // The first column level is the grand-total level; its `[0..4]` colour is
                        // what RAS exposes as `RowGrandTotalColor` (axes cross-wired, see model).
                        if s.columns.is_empty() {
                            s.column_gt_color = decode_grandtotal_color(&leaf);
                        }
                        s.columns.push(dim);
                    } else {
                        if s.rows.is_empty() {
                            s.row_gt_color = decode_grandtotal_color(&leaf);
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
    /// Background colour of the first column-axis level (the column grand-total pseudo-field) —
    /// the SDK's `RowGrandTotalColor` (the colour axes are cross-wired; see [`CrossTabGridOptions`]).
    pub(super) column_gt_color: Option<crate::model::Color>,
    /// Background colour of the first row-axis level — the SDK's `ColumnGrandTotalColor`.
    pub(super) row_gt_color: Option<crate::model::Color>,
}

/// Collect the report's cross-tab **measures** — the aggregation + summarized field for each
/// data-cell summary. These are the report's pre-layout `0x7e SummaryFieldDefinition` records (the
/// operation byte at leaf offset 0, the summarized field the first `Table.field`/`@formula` string
/// in the leaf), excluding running totals (a `0x7e` immediately preceded by its `0x80` reset). In
/// the corpus (single-cross-tab budget reports) every report summary is a cross-tab measure and the
/// count matches the measure count stored in the `0x00db CrossTabFieldGrid` record; a report with a
/// cross-tab *and* an unrelated group summary can't be disambiguated from the corpus, so all report
/// summaries are attributed to the cross-tab.
///
/// Boundary note: this all-summaries attribution is a derived **inference**, not a stored fact.
/// It is admissible here only because cross-tab measures have no XML/render surface today (STRUCTURAL
/// — no XML/oracle surface). If cross-tabs gain one, the attribution must move to the derive layer (the
/// stored-vs-derived boundary), leaving `rpt` to decode only what the bytes state.
pub(super) fn collect_crosstab_measures(
    tree: &[RecordNode],
    logical: &[u8],
) -> Vec<crate::model::CrossTabMeasure> {
    let nodes = flatten(tree);
    let mut out = Vec::new();
    for i in 0..nodes.len() {
        let node = nodes[i];
        // The layout region begins at the first area marker; summary definitions all precede it.
        if node.rtype == AREA_MARKER {
            break;
        }
        if node.rtype != SUMMARY_DEF {
            continue;
        }
        // A running total is a `0x7e` immediately preceded by its `0x80` reset record — not a measure.
        if i > 0 && nodes[i - 1].rtype == RT_RESET {
            continue;
        }
        let leaf = node.leaf_bytes(logical);
        let operation = crate::model::SummaryOperation::from_code(i32::from(
            leaf.first().copied().unwrap_or(0),
        ));
        // The summarized field is the first field-shaped length-prefixed string in the leaf (the
        // operation byte and a fixed header precede it; `Table.field` or `@formula`).
        let field = leaf
            .get(4..)
            .and_then(read_lp_string)
            .map(|(s, _)| s)
            .filter(|s| is_field_ref(s))
            .unwrap_or_default();
        out.push(crate::model::CrossTabMeasure { operation, field });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::decode_cell_format;
    use crate::model::Color;

    #[test]
    fn default_region_has_no_colour() {
        // A grid-region format with no explicit formatting: flags 0, no colour, disabled.
        let leaf = [
            0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let c = decode_cell_format(&leaf);
        assert_eq!(c.flags, 0);
        assert_eq!(c.background_color, None);
        assert!(!c.enabled);
    }

    #[test]
    fn enabled_flag_from_byte_10() {
        let leaf = [
            0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
        ];
        assert!(decode_cell_format(&leaf).enabled);
    }

    #[test]
    fn styled_region_decodes_flags_and_colour() {
        // An explicitly-formatted region: flags 0x28, a little-endian COLORREF [R,G,B,0], enabled.
        let leaf = [
            0x00, 0x00, 0x00, 0x28, 0x01, 0x00, 0xff, 0x98, 0x44, 0x00, 0x01,
        ];
        let c = decode_cell_format(&leaf);
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
}

#[cfg(test)]
mod crosstab_dim_tests {
    use super::trailing_lp_string;

    fn lp(s: &str) -> Vec<u8> {
        let mut v = ((s.len() + 1) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v.push(0);
        v
    }

    #[test]
    fn reads_trailing_field_ref_past_binary_header() {
        // A 0x00cb leaf: binary geometry/id header, then the trailing dimension field reference.
        let mut leaf = vec![0xff, 0xff, 0xff, 0xff, 0x04, 0x7e, 0x00, 0x00, 0x05, 0xa0];
        leaf.extend(lp("Data.Date1"));
        assert_eq!(trailing_lp_string(&leaf), "Data.Date1");
    }

    #[test]
    fn grand_total_level_has_no_trailing_string() {
        // No valid trailing text string → empty (a grand-total dimension level).
        let leaf = vec![0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x05, 0xa0, 0x12, 0x34];
        assert_eq!(trailing_lp_string(&leaf), "");
    }
}
