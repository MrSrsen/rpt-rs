//! Chart definition decode — each chart object's [`ChartDefinition`] (type/subtype, titles, legend
//! placement, per-axis gridlines, data labels) parsed from the flat binding region.

use super::*;

/// Collect each chart's decoded [`ChartDefinition`] (type + titles + data label) from the flat
/// binding region, keyed by chart object name.
///
/// A chart is written as a contiguous group in section order — `0xb4 ChartObject` → its `0x9e`
/// name → `0x011c` analytic header → the analytic data section → `0x0121 ChartDefinition2` — so the
/// chart named by the most recent `0xb4` owns the `0x011f`/`0x0121` records until the next `0xb4`
/// (or an area/section marker) begins another. The `0x0121` leaf leads with two 1-byte enums
/// (`graph_type` `+0x4c`, `graph_subtype` `+0x50`) then a run of length-prefixed (`u32` big-endian,
/// NUL-terminated) strings: title, subtitle, footnote, two format-mask strings, group-axis title,
/// data-axis title. The `0x011f` leaf carries the data-value label after a 6-byte header. See
/// [`ChartDefinition`] for the (parity-inert, no-oracle) status of this decode.
pub(super) fn collect_chart_styles(
    tree: &[RecordNode],
    logical: &[u8],
) -> std::collections::HashMap<String, ChartDefinition> {
    let mut out: std::collections::HashMap<String, ChartDefinition> =
        std::collections::HashMap::new();
    for (current, node) in binding_scopes(tree, logical, &[CHART_BINDING]) {
        match node.rtype {
            CHART_DATA_VALUE => {
                if let Some(name) = &current {
                    let label = read_be_lp_string_lossy(&node.leaf_bytes(logical), 6)
                        .map(|(s, _)| s)
                        .unwrap_or_default();
                    out.entry(name.clone()).or_default().data_label = label;
                }
            }
            CHART_DEFINITION2 => {
                if let Some(name) = &current {
                    let mut def = parse_chart_definition2(&node.leaf_bytes(logical));
                    // The `0x011f` record (which precedes this one) is the only source of the data
                    // label; `0x0121` never carries it, so preserve whatever was captured there.
                    let slot = out.entry(name.clone()).or_default();
                    def.data_label = std::mem::take(&mut slot.data_label);
                    *slot = def;
                }
            }
            _ => {}
        }
    }
    out
}

/// The count of length-prefixed strings the `0x0121` writer emits before the fixed-width styling
/// struct: 7 title/axis strings (title, subtitle, footnote, two format-masks, group-axis, data-axis),
/// then 1 empty separator, then 8 per-text-element font names. Verified constant across every corpus
/// chart; the writer serializes a fixed text-element schema.
const CHART_STRING_COUNT: usize = 16;

/// Parse a `0x0121 ChartDefinition2` leaf into the byte-legible chart fields (type/subtype +
/// title/axis strings + legend placement). Layout: `[graph_type u8][graph_subtype u8]` then
/// [`CHART_STRING_COUNT`] length-prefixed strings (`u32` big-endian byte count incl. trailing NUL) —
/// title, subtitle, footnote, two format-mask strings (skipped), group-axis title, data-axis title,
/// one empty separator, then 8 font names — then a 1-byte separator and the fixed styling struct.
///
/// The styling struct opens with the legend `short` (leaf `+0x410`): its **low byte** is a
/// flags byte whose bit0 = legend visible, and its **high byte** is the [`ChartLegendPosition`] enum.
/// Because the styling struct follows the variable-length string run, the legend is located at a
/// **variable offset** — one byte past the end of the string block — not a fixed leaf offset. The
/// rest of the styling struct (axis/marker/colour state) is opaque and ignored.
fn parse_chart_definition2(leaf: &[u8]) -> ChartDefinition {
    let mut def = ChartDefinition::default();
    let Some(&e0) = leaf.first() else {
        return def;
    };
    def.graph_type = ChartGraphType::from_code(i32::from(e0));
    def.graph_subtype = leaf.get(1).map_or(0, |&b| i32::from(b));
    // Read the fixed string run starting after the two enum bytes, tracking `off` so the styling
    // struct that follows the variable-length strings can be located.
    let mut off = 2usize;
    let mut strs: Vec<String> = Vec::new();
    while strs.len() < CHART_STRING_COUNT {
        let Some((s, used)) = read_be_lp_string_lossy(leaf, off) else {
            break;
        };
        strs.push(s);
        off += used;
    }
    let take = |i: usize| strs.get(i).cloned().unwrap_or_default();
    def.title = take(0);
    def.subtitle = take(1);
    def.footnote = take(2);
    // strs[3], strs[4] are the two format-mask strings (empty across the corpus) — skipped.
    def.group_axis_title = take(5);
    def.data_axis_title = take(6);
    // The legend `short` opens the styling struct one byte past the end of the full string block
    // (a 1-byte separator sits between). Default to visible/Right if the leaf is short/truncated or
    // the string run didn't complete (guarding against reading mid-field).
    def.legend_visible = true;
    if strs.len() == CHART_STRING_COUNT {
        let flags_off = off + 1;
        if let (Some(&flags), Some(&pos)) = (leaf.get(flags_off), leaf.get(flags_off + 1)) {
            def.legend_visible = flags & 0x01 != 0;
            def.legend_position = ChartLegendPosition::from_code(pos);
        }
        // Per-axis gridline mode (Axes tab). The group (category, X) axis mode sits at `flags_off + 7`,
        // the value (Y) axis mode at `flags_off + 9`, each a `CrGridTypeEnum` (bit0 minor, bit1 major).
        // This fixed offset is byte-confirmed against RAS ground truth only for the standard cartesian
        // families — every such corpus chart reads group=None / value=Major, and the three `*_legend_*`
        // fixtures read Both / Both, matching RAS exactly. The specialty/axis-less families
        // (Pie 3, Doughnut 4, Gauge 12, Gantt 13, Funnel 14, Histogram 15) lay their styling tail out
        // differently, so this offset is not their gridline byte — leave them at the `None` default
        // rather than decode a wrong value (their real gridline offset is a separate RE target).
        if !matches!(e0, 3 | 4 | 12 | 13 | 14 | 15) {
            if let Some(&g) = leaf.get(flags_off + 7) {
                def.group_axis_gridlines = ChartGridType::from_code(g);
            }
            if let Some(&v) = leaf.get(flags_off + 9) {
                def.value_axis_gridlines = ChartGridType::from_code(v);
            }
        }
        // Data-labels enum byte (leaf `+0x4a8`, bit1 = show value): a fixed-width store walk
        // from the legend `short` lands it 81 bytes on for an axis chart. Pie/doughnut charts
        // (type 3/4) carry two extra detach/rotate enum bytes mid-struct, shifting the tail +2.
        let pie_family = matches!(e0, 3 | 4);
        let data_label_off = flags_off + 81 + usize::from(pie_family) * 2;
        if let Some(&dl) = leaf.get(data_label_off) {
            def.data_labels_show_value = dl & 0x02 != 0;
        }
    }
    def
}

#[cfg(test)]
mod chart_def2_tests {
    use super::{parse_chart_definition2, CHART_STRING_COUNT};
    use crate::model::{ChartGraphType, ChartLegendPosition};

    /// Build a minimal synthetic `0x0121` leaf: two enum bytes (`graph_type`, subtype `0`), then
    /// [`CHART_STRING_COUNT`] empty length-prefixed strings (len `1` = a lone NUL), a 1-byte
    /// separator, and the styling struct opened by the legend `short` (`legend_flags`,
    /// `legend_pos`). Padded so the data-labels byte (81 bytes past the legend short for an axis
    /// chart) is present and set from `data_label`.
    fn synth_leaf(graph_type: u8, legend_flags: u8, legend_pos: u8, data_label: u8) -> Vec<u8> {
        let mut v = vec![graph_type, 0];
        for _ in 0..CHART_STRING_COUNT {
            v.extend_from_slice(&[0, 0, 0, 1, 0]); // len=1, single NUL → empty string
        }
        v.push(0); // separator; legend short opens at off+1
        let flags_off = v.len();
        v.push(legend_flags);
        v.push(legend_pos);
        // Data-labels enum byte is a fixed 81-byte walk past the legend short (pie/doughnut add 2,
        // exercised separately). Pad the intervening styling bytes with zeros, then write it.
        let data_label_off = flags_off + 81;
        v.resize(data_label_off, 0);
        v.push(data_label);
        v
    }

    /// The decoder maps all four legend-position codes 0..=3 to Right/Left/Bottom/Top. Right(0),
    /// Left(1), Bottom(2) are byte-confirmed by fixtures; Top(3) has no fixture, so this proves the
    /// DECODER handles code 3 — whether Crystal actually *emits* 3 for a top legend is an open
    /// empirical question (no synthetic legend-top report captured yet), not a decoder gap.
    #[test]
    fn legend_position_decodes_all_four_codes() {
        for (code, want) in [
            (0u8, ChartLegendPosition::Right),
            (1, ChartLegendPosition::Left),
            (2, ChartLegendPosition::Bottom),
            (3, ChartLegendPosition::Top),
        ] {
            let def = parse_chart_definition2(&synth_leaf(0, 0x01, code, 0));
            assert_eq!(def.legend_position, want, "legend position code {code}");
            assert!(def.legend_visible, "bit0 set → visible (code {code})");
        }
    }

    /// The per-axis gridline bytes decode from `flags_off + 7` (group/X axis) and `flags_off + 9`
    /// (value/Y axis) for the axis families, reproducing the two RAS-confirmed corpus configurations:
    /// the default `None`/`Major` and the `*_legend_*` fixtures' `Both`/`Both`. Pie-family charts (no
    /// axes) leave both at the `None` default regardless of the bytes in that region.
    #[test]
    fn axis_gridlines_decode_group_x_and_value_y() {
        use crate::model::ChartGridType;
        // flags_off matches synth_leaf: 2 enum bytes + the string block + a 1-byte separator.
        let flags_off = 2 + CHART_STRING_COUNT * 5 + 1;
        let with_grid = |graph_type: u8, gx: u8, vy: u8| {
            let mut leaf = synth_leaf(graph_type, 0x01, 0, 0);
            leaf[flags_off + 7] = gx;
            leaf[flags_off + 9] = vy;
            parse_chart_definition2(&leaf)
        };
        // Default axis chart: group axis None, value axis Major.
        let def = with_grid(0, 0, 2);
        assert_eq!(def.group_axis_gridlines, ChartGridType::None);
        assert_eq!(def.value_axis_gridlines, ChartGridType::Major);
        // The legend-fixture configuration: both axes Both.
        let def = with_grid(0, 3, 3);
        assert_eq!(def.group_axis_gridlines, ChartGridType::Both);
        assert_eq!(def.value_axis_gridlines, ChartGridType::Both);
        // Pie family (type 3): the axis-gridline read is gated off, so both stay at the None default
        // even though this region carries the pie's shifted detach/rotate bytes.
        let def = with_grid(3, 3, 3);
        assert_eq!(def.group_axis_gridlines, ChartGridType::None);
        assert_eq!(def.value_axis_gridlines, ChartGridType::None);
    }

    /// `ChartGridType::from_code` maps the `CrGridTypeEnum` bitmask (bit0 minor, bit1 major).
    #[test]
    fn chart_grid_type_from_code() {
        use crate::model::ChartGridType;
        assert_eq!(ChartGridType::from_code(0), ChartGridType::None);
        assert_eq!(ChartGridType::from_code(1), ChartGridType::Minor);
        assert_eq!(ChartGridType::from_code(2), ChartGridType::Major);
        assert_eq!(ChartGridType::from_code(3), ChartGridType::Both);
    }

    /// `from_code` maps the raw legend-position enum byte independently of the leaf walk.
    #[test]
    fn legend_position_from_code() {
        assert_eq!(
            ChartLegendPosition::from_code(0),
            ChartLegendPosition::Right
        );
        assert_eq!(ChartLegendPosition::from_code(1), ChartLegendPosition::Left);
        assert_eq!(
            ChartLegendPosition::from_code(2),
            ChartLegendPosition::Bottom
        );
        assert_eq!(ChartLegendPosition::from_code(3), ChartLegendPosition::Top);
        // Any unsampled code falls back to the engine default Right.
        assert_eq!(
            ChartLegendPosition::from_code(4),
            ChartLegendPosition::Right
        );
    }

    /// The legend-visible flag is bit0 of the legend `short`'s low byte.
    #[test]
    fn legend_visible_is_bit0() {
        assert!(parse_chart_definition2(&synth_leaf(0, 0x01, 0, 0)).legend_visible);
        assert!(!parse_chart_definition2(&synth_leaf(0, 0x00, 0, 0)).legend_visible);
    }

    /// The data-labels "show value" flag is bit1 of the data-labels enum byte, 81 bytes past the
    /// legend short for an axis (bar/line/area) chart.
    #[test]
    fn data_labels_show_value_bit1_axis() {
        assert!(!parse_chart_definition2(&synth_leaf(0, 0x01, 0, 0x00)).data_labels_show_value);
        assert!(parse_chart_definition2(&synth_leaf(0, 0x01, 0, 0x02)).data_labels_show_value);
        // bit0 alone (a different label mode) is not "show value".
        assert!(!parse_chart_definition2(&synth_leaf(0, 0x01, 0, 0x01)).data_labels_show_value);
    }

    /// Pie/doughnut charts (type 3/4) carry two extra mid-struct enum bytes, so the data-labels byte
    /// sits at +83 rather than +81. Build a pie leaf with the show-value bit at the shifted offset.
    #[test]
    fn data_labels_show_value_pie_family_shift() {
        // Manually build a pie leaf: same prefix as synth_leaf but data-labels at flags_off+83.
        let build = |gt: u8, dl_extra: usize| {
            let mut v = vec![gt, 0];
            for _ in 0..CHART_STRING_COUNT {
                v.extend_from_slice(&[0, 0, 0, 1, 0]);
            }
            v.push(0);
            let flags_off = v.len();
            v.push(0x01); // visible
            v.push(0x00); // right
            let dl_off = flags_off + 81 + dl_extra;
            v.resize(dl_off, 0);
            v.push(0x02); // show value
            v
        };
        // Pie (3): decoder must read at +83; the +81 slot is zero, so only the shift decodes true.
        let pie = parse_chart_definition2(&build(3, 2));
        assert_eq!(pie.graph_type, ChartGraphType::Pie);
        assert!(pie.data_labels_show_value, "pie reads data-labels at +83");
        // Doughnut (code 4) is a distinct pie-family type that shares the +2 shift.
        let doughnut = parse_chart_definition2(&build(4, 2));
        assert_eq!(doughnut.graph_type, ChartGraphType::Doughnut);
        assert!(
            doughnut.data_labels_show_value,
            "doughnut reads data-labels at +83"
        );
    }

    /// A short/truncated leaf must not panic and defaults sensibly (visible, no data labels).
    #[test]
    fn short_leaf_defaults() {
        let def = parse_chart_definition2(&[0x00, 0x00]);
        assert!(def.legend_visible);
        assert!(!def.data_labels_show_value);
    }
}
