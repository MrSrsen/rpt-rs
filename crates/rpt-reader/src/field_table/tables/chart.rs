//! The chart object and the `0x0121 ChartDefinition2` that states how it is drawn.

use super::*;

/// Pie and Doughnut, the two families whose content carries an extra pair of enums mid-sequence.
fn pie_family(c: &Ctx<'_>) -> bool {
    matches!(c.row.u("graph_type"), 3 | 4)
}

/// 3-D Riser and 3-D Surface, the two families that carry the viewing angle.
fn three_d_family(c: &Ctx<'_>) -> bool {
    matches!(c.row.u("graph_type"), 5 | 6)
}

/// One face name in the eight-entry run that precedes the styling block, and in the two-entry run
/// that follows it.
const FACE: &[Field] = &[Field::new("face", Kind::Str)];

/// One text element's default-font flag word.
const ELEMENT_FLAG: &[Field] = &[Field::new("flag", Kind::U16Be)];

/// One text element's weight and slant. The entries run five bytes apart, except that the fourth
/// (GroupTitle) is a byte wider — remove that byte and every following element's weight moves.
const ELEMENT_STYLE: &[Field] = &[
    Field::new("weight", Kind::U16Be),
    // The group-axis title's entry is one byte longer than every other, and the extra byte sits
    // between the weight and the slant: read the other way round the entry's two trailing pad bytes
    // stop being the `00 00` every other entry ends with, and a slanted group-axis title reads
    // upright. The irregularity is the record's, not a mis-derived offset — a uniform reading is
    // simpler and wrong, so do not fold this entry into the other eight.
    Field::when("_u1", Kind::Skip(1), |c| c.index == 3),
    Field::new("italic", Kind::U8),
    Field::new("_u0", Kind::Skip(2)),
];

/// `0x0121 ChartDefinition2` — the chart's type, its text, and its styling block.
///
/// Two families change the sequence rather than the meaning of a fixed offset: Pie/Doughnut carry
/// an extra pair of enums mid-sequence, and the 3-D families carry one extra field, the viewing
/// angle. Both are conditional entries here, so every field after them lands by construction
/// instead of by a corrected offset.
///
/// The styling block is a straight run of per-axis properties: the four gridline modes in the axis
/// order group / series / value / value-2, then everything else in the order value, value-2, series
/// — three `(min, max)` double pairs, the number-format / auto-range / division-method triples, and
/// the three division counts. The two auto-scale flags' triple lives past the per-element style
/// block, next to the legend layout.
///
/// The nine style entries cover every text element but DataLabel, which has none.
///
/// Every enum here is a narrowing read — one byte below `0x80`, two from `0x80` up — so the whole
/// block's width follows its values rather than being fixed. `legend_flags` and `is_vertical_bar`
/// are whole signed shorts whose low byte is the flag.
pub(crate) const CHART_DEFINITION2: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0121,
    name: "ChartDefinition2",
    fields: &[
        Field::new("graph_type", Kind::VarU16),
        Field::new("graph_subtype", Kind::VarU16),
        Field::new("title", Kind::Str),
        Field::new("subtitle", Kind::Str),
        Field::new("footnote", Kind::Str),
        Field::new("_format_mask_a", Kind::Str),
        Field::new("_format_mask_b", Kind::Str),
        Field::new("group_axis_title", Kind::Str),
        Field::new("data_axis_title", Kind::Str),
        Field::new("_reserved_title", Kind::Str),
        Field::new(
            "faces",
            Kind::Repeat {
                count: Count::Fixed(8),
                body: FACE,
            },
        ),
        Field::new("legend_flags", Kind::I16Be),
        Field::new("legend_position", Kind::VarU16),
        Field::new("is_vertical_bar", Kind::I16Be),
        Field::new("bar_size", Kind::VarU16),
        // Two enums only the pie families carry: a second copy of the circle's size, which the
        // sizing run below supersedes, and which slice the chart pulls out of the pie.
        Field::when("_pie_size_superseded", Kind::VarU16, pie_family),
        Field::when("slice_detachment", Kind::VarU16, pie_family),
        Field::new("marker_size", Kind::VarU16),
        Field::new("marker_shape", Kind::VarU16),
        Field::new("group_axis_gridlines", Kind::VarU16),
        Field::new("series_axis_gridlines", Kind::VarU16),
        Field::new("value_axis_gridlines", Kind::VarU16),
        Field::new("value_axis2_gridlines", Kind::VarU16),
        Field::new("value_axis_min", Kind::F64Be),
        Field::new("value_axis_max", Kind::F64Be),
        Field::new("value_axis2_min", Kind::F64Be),
        Field::new("value_axis2_max", Kind::F64Be),
        Field::new("series_axis_min", Kind::F64Be),
        Field::new("series_axis_max", Kind::F64Be),
        Field::new("value_axis_number_format", Kind::VarU16),
        Field::new("value_axis2_number_format", Kind::VarU16),
        Field::new("series_axis_number_format", Kind::VarU16),
        Field::new("value_axis_auto_range", Kind::VarU16),
        Field::new("value_axis2_auto_range", Kind::VarU16),
        Field::new("series_axis_auto_range", Kind::VarU16),
        Field::new("value_axis_division_method", Kind::VarU16),
        Field::new("value_axis2_division_method", Kind::VarU16),
        Field::new("series_axis_division_method", Kind::VarU16),
        Field::new("value_axis_divisions", Kind::U32Be),
        Field::new("value_axis2_divisions", Kind::U32Be),
        Field::new("series_axis_divisions", Kind::U32Be),
        Field::new("chart_color", Kind::VarU16),
        Field::new("data_labels", Kind::VarU16),
        Field::new("data_value_number_format", Kind::VarU16),
        Field::when("view_angle", Kind::VarU16, three_d_family),
        Field::new("_u2", Kind::Skip(26)),
        Field::new(
            "trailing_faces",
            Kind::Repeat {
                count: Count::Fixed(2),
                body: FACE,
            },
        ),
        Field::new(
            "element_flags",
            Kind::Repeat {
                count: Count::Fixed(10),
                body: ELEMENT_FLAG,
            },
        ),
        // A sizing run of three words. The two after the pie size are the riser width and the
        // marker extent the engine derives from `bar_size` and `marker_size`, so they move with
        // those enums rather than being authored.
        Field::new("pie_size", Kind::U16Be),
        Field::new("_u3", Kind::Skip(4)),
        Field::new(
            "element_styles",
            Kind::Repeat {
                count: Count::Fixed(9),
                body: ELEMENT_STYLE,
            },
        ),
        Field::new("_u4", Kind::Skip(18)),
        Field::new("legend_layout", Kind::U8),
        Field::new("value_axis_auto_scale", Kind::U8),
        Field::new("value_axis2_auto_scale", Kind::U8),
        Field::new("series_axis_auto_scale", Kind::U8),
        Field::new("_u5", Kind::Skip(40)),
    ],
};

/// `0x011c ChartAnalytic` — the analytic header that precedes a chart's data section.
///
/// A word, the layout enum, and a trailing word the record need not carry at all.
pub(crate) const CHART_ANALYTIC: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x011c,
    name: "ChartAnalytic",
    fields: &[
        Field::new("_u0", Kind::U16Be),
        Field::new("layout_type", Kind::VarU16),
        Field::optional("_u1", Kind::U16Be),
    ],
};

/// One risered value of a chart: a field reference to the summary it draws — the summary's display
/// form, then the pool and index that resolve it.
const CHART_DATA_VALUE_ENTRY: &[Field] = &[Field::new("summary", Kind::FieldRef)];

/// `0x011f ChartDataValue` — the chart's labeled data values, one per riser.
///
/// A count then that many summary references: a chart with two value fields stores both, and the
/// record's length follows their names'. The count is a whole `u32`, though the engine keeps only
/// its low half, so a table reading the low half alone lands two bytes into the first reference.
/// The model keeps only the first name.
pub(crate) const CHART_DATA_VALUE: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x011f,
    name: "ChartDataValue",
    fields: &[
        Field::new("_u0", Kind::U16Be),
        Field::new("value_count", Kind::U32Be),
        Field::new(
            "values",
            Kind::Repeat {
                count: Count::FromField("value_count"),
                body: CHART_DATA_VALUE_ENTRY,
            },
        ),
        Field::new("_u1", Kind::U32Be),
    ],
};

/// `0x00b4 ChartObject` — the chart's opener, and the block every record of one chart belongs to.
///
/// Unlike the other object openers it does **not** nest its `ObjectName` itself. Its one child is
/// the analytic object it draws through, and the name is two levels further down — `0x00b3` nests
/// the `0x00ae` graphic base, and that nests the `0x009e`. So the opener declares one child type,
/// and a consumer after the chart's name looks for a descendant rather than a child.
///
/// Past the child the record carries the chart's own render extent, a `TwipSize`: width then
/// height, each a narrowing twip, so the pair is four bytes only while both are under `0x8000`
/// twips (about 22.7 inches). It is the extent the analytic is drawn at, not the object's placement
/// on the page — that is in the object's `0x00be` position and `0x00fd`/`0x00fc` format records like
/// any other object's.
pub(crate) const CHART_OBJECT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00b4,
    name: "ChartObject",
    fields: &[
        Field::new("analytic_object", Kind::Child(0x00b3)),
        Field::new("width", Kind::VarU32),
        Field::new("height", Kind::VarU32),
    ],
};
