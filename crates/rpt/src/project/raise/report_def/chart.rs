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
/// [`ChartDefinition`] for the status of this decode.
pub(super) fn collect_chart_styles(
    tree: &[RecordNode],
    logical: &[u8],
) -> std::collections::HashMap<String, ChartDefinition> {
    let mut out: std::collections::HashMap<String, ChartDefinition> =
        std::collections::HashMap::new();
    for (current, node) in binding_scopes(tree, logical, &[CHART_BINDING]) {
        match node.rtype {
            CHART_ANALYTIC => {
                // The chart's data-layout axis (Group / Detail / CrossTab) is leaf byte 2 of the
                // `0x011c` analytic header. It precedes the `0x0121` definition record, whose arm
                // below replaces the slot wholesale — so, like `data_label`, it is stashed on the
                // slot here and restored across that replacement.
                if let Some(name) = &current {
                    if let Some(&code) = node.leaf_bytes(logical).get(2) {
                        out.entry(name.clone()).or_default().layout_type =
                            crate::model::ChartLayoutType::from_code(code);
                    }
                }
            }
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
                    // label, and the `0x011c` header the only source of the layout type; `0x0121`
                    // carries neither, so preserve whatever was captured on the slot.
                    let slot = out.entry(name.clone()).or_default();
                    def.data_label = std::mem::take(&mut slot.data_label);
                    def.layout_type = slot.layout_type;
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
/// then 1 empty separator, then 8 per-text-element font names. Invariant — the writer serializes a
/// fixed text-element schema.
const CHART_STRING_COUNT: usize = 16;

/// The point-size range a genuine chart-title override occupies. The `off + 87` title-size slot is a
/// shared/overloaded byte (see [`parse_chart_definition2`]); a decoded size outside this range is not a
/// real override and is dropped to the engine default. The bounds bracket every real title seen across
/// a real title (14–35 pt) while excluding both failure modes: a programmatically created chart leaves a
/// small counter in the slot (≤ 6 pt — the floor sits above it, since a floor of 6 let that artifact
/// through as a bogus 6 pt title), and other layout state can bleed a large byte in (200 pt+).
const TITLE_PT_PLAUSIBLE: std::ops::RangeInclusive<u16> = 10..=72;

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
    // The subtype is a single byte at offset 1, EXCEPT when it is ≥ 128 (the Gantt/Funnel/Histogram
    // families, whose base subtype is `graph_type × 10` = 130/140/150). The writer then escapes it:
    // offset 1 carries a `0x80` sentinel and the real subtype byte follows at offset 2, pushing the
    // string run one byte later. `off` tracks where that string run begins.
    let mut off = 2usize;
    def.graph_subtype = match leaf.get(1) {
        Some(&0x80) => {
            off = 3;
            leaf.get(2).map_or(0, |&b| i32::from(b))
        }
        Some(&b) => i32::from(b),
        None => 0,
    };
    // Read the fixed string run starting after the enum prefix, tracking `off` so the styling
    // struct that follows the variable-length strings can be located.
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
    // strs[3], strs[4] are the two format-mask strings (normally empty) — skipped.
    def.group_axis_title = take(5);
    def.data_axis_title = take(6);
    // strs[7] is a 1-byte empty separator; strs[8..16] are the eight contiguous per-text-element
    // font-name strings (the Chart Expert Text tab), in stored order with index 0 = the Title element
    // at strs[8]. Two further label-font
    // strings live past the fixed styling struct and are not captured. Verbatim faces (usually the
    // default "Arial") are the stored fact and kept as-is.
    def.element_fonts = strs
        .get(8..CHART_STRING_COUNT)
        .unwrap_or_default()
        .iter()
        .map(|name| crate::model::ChartElementFont {
            name: name.clone(),
            size_pt: None,
        })
        .collect();
    // The legend `short` opens the styling struct one byte past the end of the full string block
    // (a 1-byte separator sits between). Default to visible/Right if the leaf is short/truncated or
    // the string run didn't complete (guarding against reading mid-field).
    def.legend_visible = true;
    if strs.len() == CHART_STRING_COUNT {
        // The Title element's explicit point size sits at a fixed offset past the end of the string
        // run (`off + 87`), relative to the styling-struct start — it rides along when a longer face
        // name lengthens the string block rather than being an absolute leaf offset. Encoding
        // `pt = round(byte × 7 / 6)` (`0x0c` ⇒ 14 pt, `0x11` ⇒ 20 pt, `0x1a` ⇒ 30 pt); a stored `0`
        // means the engine default.
        //
        // This is a SHARED slot, not a dedicated title-size field: it is only read for the **Bar
        // family** (`e0 == 0`; the pie/area/3-D styling structs are laid out differently, so it lands
        // on unrelated bytes there), and even within Bar it is perturbed by adjacent legend/arrangement
        // state. Only a chart created in the designer stores its title size here at all: a
        // programmatically created chart puts the explicit size in the chart's CVOM sidecar stream
        // instead and leaves this record untouched, and carries no explicit-vs-default override flag
        // here either. What sits here for such a chart is unrelated state. So the read is guarded by
        // a plausibility window: a
        // decoded size outside the range a real chart title occupies is dropped to the engine default
        // (`None`) rather than emitted as a garbage small or 200 pt+ title.
        if e0 == 0 {
            if let Some(&sz) = leaf.get(off + 87) {
                let pt = (u16::from(sz) * 7 + 3) / 6;
                if TITLE_PT_PLAUSIBLE.contains(&pt) {
                    if let Some(title) = def.element_fonts.first_mut() {
                        title.size_pt = Some(pt);
                    }
                }
            }
        }
        let flags_off = off + 1;
        if let (Some(&flags), Some(&pos)) = (leaf.get(flags_off), leaf.get(flags_off + 1)) {
            def.legend_visible = flags & 0x01 != 0;
            def.legend_position = ChartLegendPosition::from_code(pos);
        }
        // Per-axis gridline mode (Axes tab). The group (category, X) axis mode sits at `flags_off + 7`
        // (leaf `+0x430`), the value (Y) axis mode at `flags_off + 9` (leaf `+0x438`), each a
        // `CrGridTypeEnum` (bit0 minor, bit1 major). The offset holds for the cartesian families. The
        // axis-less families (Pie 3, Doughnut 4, Gauge 12, Gantt 13, Funnel 14, Histogram 15) have no
        // group/value axes at all — every gridline is `None` for them and the pie branch shifts this
        // region — so they are gated off and left at the `None` default (there is no gridline byte to
        // decode for them, not merely a relocated one).
        if !matches!(e0, 3 | 4 | 12 | 13 | 14 | 15) {
            if let Some(&g) = leaf.get(flags_off + 7) {
                def.group_axis_gridlines = ChartGridType::from_code(g);
            }
            if let Some(&v) = leaf.get(flags_off + 9) {
                def.value_axis_gridlines = ChartGridType::from_code(v);
            }
        }
        // Data-labels enum byte (leaf `+0x4a8`, bit1 = show value), a fixed 81 bytes past the legend
        // `short` (leaf `+0x410`) for an axis chart. Pie/doughnut
        // charts (type 3/4) insert two extra detach/rotate enum bytes (leaf `+0x420`/`+0x424`) before
        // this point, shifting the tail +2 → offset 83.
        let pie_family = matches!(e0, 3 | 4);
        let data_label_off = flags_off + 81 + usize::from(pie_family) * 2;
        if let Some(&dl) = leaf.get(data_label_off) {
            def.data_labels_show_value = dl & 0x02 != 0;
        }
        // The 3-D camera preset (`ViewingAngle`, `CrViewingAngleEnum`) is the `+0x4cc` enum, two
        // bytes past the data-labels byte (`+0x4a8`) — the order in the styling struct is
        // `+0x4a8` (data labels), `+0x4c8`, `+0x4cc`, then the 3-D-only `+0x4d0`. The stored value
        // is the 1-based `CrViewingAngleEnum` ordinal; only interpret it for the two 3-D families
        // (graph_type 5/6), where Standard = 1 and DistortedView = 4. `+0x4cc` is a single byte (all 16 ordinals fit in one byte). A 2-D
        // chart carries the byte too, but the angle is meaningless there.
        if matches!(e0, 5 | 6) {
            if let Some(&va) = leaf.get(data_label_off + 2) {
                def.view_angle = crate::model::ChartViewAngle::from_stored(va);
            }
        }
    }
    def
}

/// The RAS `FormulaForm` operator name for a chart data summary — the full engine spelling used in a
/// summary expression (`Maximum`/`Minimum` in full, not the abbreviated store form). Falls back to
/// `Sum` for an unmapped operation (the engine's default aggregation).
pub(super) fn chart_summary_op_name(op: crate::model::SummaryOperation) -> &'static str {
    use crate::model::SummaryOperation::*;
    match op {
        Sum => "Sum",
        Average => "Average",
        Count => "Count",
        DistinctCount => "DistinctCount",
        Maximum => "Maximum",
        Minimum => "Minimum",
        SampleVariance => "SampleVariance",
        SampleStandardDeviation => "SampleStandardDeviation",
        PopVariance => "PopVariance",
        PopStandardDeviation => "PopStandardDeviation",
        Correlation => "Correlation",
        Covariance => "Covariance",
        WeightedAvg => "WeightedAvg",
        Median => "Median",
        Percentile => "Percentile",
        NthLargest => "NthLargest",
        NthSmallest => "NthSmallest",
        Mode => "Mode",
        NthMostFrequent => "NthMostFrequent",
        Other(_) => "Sum",
    }
}

/// Compose a chart's RAS `DataFields` / `ConditionFields` `FormulaForm` lists (see
/// [`ChartDefinition::data_refs`]/[`ChartDefinition::category_refs`]) from the decoded data summaries
/// (`(operation, field)` pairs), the ordered category ("on change of") field references, and the
/// scoping (innermost) category's grouping-period token.
///
/// - `ConditionFields` = each category brace-wrapped (`{field}`), in axis order.
/// - `DataFields` = for each data summary, `Op ({field}, {scoping_category}[, "period"])`, or
///   `Op ({field})` when the chart has no category. The scoping category is the innermost (last)
///   axis; `scoping_period` is its explicit non-daily grouping period token, or `None` (a discrete
///   category, or a date category on its implicit daily default — see [`ChartDefinition::data_refs`]).
pub(super) fn compose_chart_refs(
    data: &[(crate::model::SummaryOperation, String)],
    categories: &[String],
    scoping_period: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    let condition = categories.iter().map(|c| format!("{{{c}}}")).collect();
    let scoping = categories.last();
    let data_forms = data
        .iter()
        .map(|(op, field)| {
            let name = chart_summary_op_name(*op);
            match scoping {
                Some(cat) => match scoping_period {
                    Some(p) => format!("{name} ({{{field}}}, {{{cat}}}, \"{p}\")"),
                    None => format!("{name} ({{{field}}}, {{{cat}}})"),
                },
                None => format!("{name} ({{{field}}})"),
            }
        })
        .collect();
    (data_forms, condition)
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

    /// The decoder maps all four legend-position codes 0..=3 to Right/Left/BottomCenter/Custom, each
    /// (code 3 = a manually positioned legend, reported by the engine as `crLegendPositionCustom`).
    #[test]
    fn legend_position_decodes_all_four_codes() {
        for (code, want) in [
            (0u8, ChartLegendPosition::Right),
            (1, ChartLegendPosition::Left),
            (2, ChartLegendPosition::BottomCenter),
            (3, ChartLegendPosition::Custom),
        ] {
            let def = parse_chart_definition2(&synth_leaf(0, 0x01, code, 0));
            assert_eq!(def.legend_position, want, "legend position code {code}");
            assert!(def.legend_visible, "bit0 set → visible (code {code})");
        }
    }

    /// The per-axis gridline bytes decode from `flags_off + 7` (group/X axis) and `flags_off + 9`
    /// (value/Y axis) for the axis families, covering both stored configurations: the default
    /// `None`/`Major` and `Both`/`Both`. Pie-family charts (no axes) leave both at the `None` default
    /// regardless of the bytes in that region.
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
            ChartLegendPosition::BottomCenter
        );
        assert_eq!(
            ChartLegendPosition::from_code(3),
            ChartLegendPosition::Custom
        );
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

    /// The 3-D camera preset decodes from the `+0x4cc` enum byte, two bytes past the data-labels
    /// byte (`+0x4a8`). Only the 3-D families (graph_type 5/6) interpret it; the stored value is the
    /// 1-based `CrViewingAngleEnum` ordinal (Standard = 1, DistortedView = 4).
    #[test]
    fn view_angle_decodes_for_3d_families() {
        use crate::model::ChartViewAngle;
        // Build a leaf whose view-angle byte (flags_off + 83 = data_label_off + 2) carries `code`.
        let build = |gt: u8, code: u8| {
            let mut v = vec![gt, 0];
            for _ in 0..CHART_STRING_COUNT {
                v.extend_from_slice(&[0, 0, 0, 1, 0]);
            }
            v.push(0);
            let flags_off = v.len();
            v.push(0x01); // legend visible
            v.push(0x00); // right
            let va_off = flags_off + 83; // = data_label_off (flags_off+81, non-pie) + 2 = +0x4cc
            v.resize(va_off, 0);
            v.push(code);
            v
        };
        // 3-D Riser (5) and 3-D Surface (6) both read the byte; the value is the 1-based ordinal.
        assert_eq!(
            parse_chart_definition2(&build(5, 1)).view_angle,
            ChartViewAngle::Standard
        );
        assert_eq!(
            parse_chart_definition2(&build(5, 4)).view_angle,
            ChartViewAngle::DistortedView
        );
        assert_eq!(
            parse_chart_definition2(&build(6, 15)).view_angle,
            ChartViewAngle::BirdsEyeView
        );
        // 0 (custom/unset) falls back to Standard.
        assert_eq!(
            parse_chart_definition2(&build(5, 0)).view_angle,
            ChartViewAngle::Standard
        );
        // A 2-D family (bar) never carries a meaningful view angle — the byte in that position is
        // not interpreted, so it stays at the default `Standard`.
        assert_eq!(
            parse_chart_definition2(&build(0, 4)).view_angle,
            ChartViewAngle::Standard
        );
    }

    /// The per-element font run (strs[8..16]) decodes into `element_fonts` in stored order with
    /// index 0 = the Title element, keeping verbatim face names, and the Title size byte at `off + 87`
    /// decodes to points via `round(byte × 7 / 6)` (`0x0c` ⇒ 14, `0x11` ⇒ 20).
    #[test]
    fn element_fonts_and_title_size_decode() {
        // A length-prefixed string: 4-byte BE prefix = content length incl. trailing NUL.
        fn lp(s: &str) -> Vec<u8> {
            let mut v = ((s.len() + 1) as u32).to_be_bytes().to_vec();
            v.extend_from_slice(s.as_bytes());
            v.push(0);
            v
        }
        let build = |title_face: &str, title_size_byte: u8| {
            let mut v = vec![0u8, 0]; // graph_type, subtype
                                      // 8 leading strings (title/subtitle/footnote/2 masks/group/data axis + 1 separator),
                                      // all empty; the font run's element order is what this test exercises.
            for _ in 0..8 {
                v.extend_from_slice(&[0, 0, 0, 1, 0]);
            }
            // 8 contiguous per-element font names: index 0 = Title, the rest default Arial.
            v.extend_from_slice(&lp(title_face));
            for _ in 0..7 {
                v.extend_from_slice(&lp("Arial"));
            }
            let off = v.len(); // start of the styling struct
            v.resize(off + 87, 0);
            v.push(title_size_byte); // Title size slot at off + 87
            v
        };
        // Title face set (Times New Roman) + size byte 0x11 ⇒ 20 pt.
        let def = parse_chart_definition2(&build("Times New Roman", 0x11));
        assert_eq!(def.element_fonts.len(), 8, "eight contiguous element fonts");
        assert_eq!(
            def.element_fonts[0].name, "Times New Roman",
            "index 0 = Title"
        );
        assert_eq!(def.element_fonts[0].size_pt, Some(20), "0x11 ⇒ 20 pt");
        for f in &def.element_fonts[1..] {
            assert_eq!(f.name, "Arial", "non-Title elements keep the default face");
            assert_eq!(f.size_pt, None, "only the Title size slot is byte-located");
        }
        // Baseline Title (Arial) + size byte 0x0c ⇒ 14 pt.
        let def = parse_chart_definition2(&build("Arial", 0x0c));
        assert_eq!(def.element_fonts[0].name, "Arial");
        assert_eq!(def.element_fonts[0].size_pt, Some(14), "0x0c ⇒ 14 pt");
        // A stored size byte of 0 means the engine default ⇒ None.
        let def = parse_chart_definition2(&build("Arial", 0x00));
        assert_eq!(def.element_fonts[0].size_pt, None, "0 ⇒ engine default");
    }

    /// A short/truncated leaf must not panic and defaults sensibly (visible, no data labels).
    #[test]
    fn short_leaf_defaults() {
        let def = parse_chart_definition2(&[0x00, 0x00]);
        assert!(def.legend_visible);
        assert!(!def.data_labels_show_value);
    }
}
