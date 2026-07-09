//! Report objects (SDK: `IReportObject` and its kinds).

use super::enums::{
    FieldValueType, ImageFormat, LineStyle, PictureType, ReadingOrder, SummaryOperation,
};
use super::format::{Border, FieldFormat, Font, FontColor, ObjectFormat};
use super::primitives::{Color, Formula, RecordRef, Rect, Twips};

/// SDK: `IReportObject` — the base every object shares; `kind` carries the per-type data.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReportObject {
    /// The object's name (SDK `ReportObject.Name`).
    pub name: String,
    /// The object's bounding box on the section, in twips.
    pub bounds: Rect,
    /// The object's border (line styles, colour, tightness).
    pub border: Border,
    /// The object's shared formatting (suppress, keep-together, colours, …).
    pub format: ObjectFormat,
    /// Parent section id this object lives in (SDK: SectionCode).
    pub section_code: i32,
    /// The concrete object subtype and its per-type data.
    pub kind: ReportObjectKind,
    /// Back-reference to the substrate record this object was raised from.
    pub origin: RecordRef,
}

/// SDK: the concrete report-object subtype + its extra members.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ReportObjectKind {
    /// SDK `IFieldObject` — a single field/formula/parameter value.
    Field(FieldObject),
    /// SDK `ITextObject` — a rich-text paragraph block with embedded field references.
    Text(TextObject),
    /// SDK `IFieldHeadingObject` — a column heading text object bound to a field object.
    FieldHeading(FieldHeadingObject),
    /// SDK `IBoxObject` — a rectangle/box shape.
    Box(BoxShape),
    /// SDK `ILineObject` — a straight line shape.
    Line(LineShape),
    /// SDK `IPictureObject` — an embedded image.
    Picture(PictureObject),
    /// SDK `ISubreportObject` — a placeholder for a nested report.
    Subreport(SubreportObject),
    /// SDK: `IBlobFieldObject` — an image/blob database field rendered as a picture.
    BlobField(BlobFieldObject),
    /// SDK `IChartObject` — a chart (boxed; the definition is large).
    Chart(Box<ChartObject>),
    /// SDK: `ICrossTabObject` — a cross-tab grid. Carries its decoded row/column dimension field
    /// bindings ([`CrossTabObject`]) for `Field.UseCount`.
    CrossTab(CrossTabObject),
    /// SDK: `IOlapGridObject` — an OLAP grid. Typed marker; opener rtype not yet identified, so it
    /// is not yet produced by the decoder.
    OlapGrid,
    /// SDK: `IMapObject` — a geographic map. Typed marker; opener rtype not yet identified.
    Map,
    /// SDK: `IFlashObject` — an embedded Flash/Xcelsius object. Typed marker; opener rtype not yet
    /// identified.
    Flash,
    /// A deferred but not-yet-identified object kind — the raw opener code is preserved.
    Deferred(u16),
    /// An unmodelled object kind — the raw code is preserved.
    #[default]
    Unknown,
}

/// The kind of field a [`FieldObject`] displays. Determines how the engine renders the object's
/// `DataSource`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FieldRefKind {
    /// A database field — `{table.field}`.
    #[default]
    DatabaseField,
    /// A formula field — `{@name}`.
    Formula,
    /// A summary — `Sum ({field})`, `Count ({field})`, etc.
    Summary,
    /// A special field (print date, page-N-of-M, …) — a spaceless kind name.
    Special,
    /// An automatic group-name field — `GroupName ({group condition field})`.
    GroupName,
    /// A running-total field — `{#name}`.
    RunningTotal,
    /// A parameter field — `{?name}`.
    Parameter,
    /// A SQL expression field — `{%name}`.
    SqlExpression,
    /// Anything else; the raw reference is used verbatim.
    Unknown,
}

impl FieldRefKind {
    /// Map the field-object opener's type byte to a [`FieldRefKind`].
    pub fn from_code(code: u8) -> Self {
        match code {
            0x00 => Self::DatabaseField,
            0x01 => Self::Formula,
            0x02 => Self::Summary,
            0x03 => Self::Special,
            0x04 => Self::GroupName,
            0x06 => Self::Parameter,
            0x09 => Self::RunningTotal,
            0x0a => Self::SqlExpression,
            _ => Self::Unknown,
        }
    }
}

/// SDK: `IFieldObject`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FieldObject {
    /// The field reference this object displays (SDK `DataSource`), e.g. `{Command.name}` or `{@f}`.
    pub data_source: String,
    /// Which kind of reference `data_source` is (database field, formula, parameter, …).
    pub ref_kind: FieldRefKind,
    /// The value type the field resolves to.
    pub value_type: FieldValueType,
    /// The object's font and colour.
    pub font_color: FontColor,
    /// The field's display format leaf, when it overrides the value-type defaults.
    pub format: Option<FieldFormat>,
    /// For a summary field object (`ref_kind == Summary`), the index of its summary definition.
    /// Two placements of the same summary share a code; two summaries that render
    /// identically have distinct codes — so this is the identity used to deduplicate
    /// `<SummaryFields>`. `None` for non-summary objects.
    pub summary_code: Option<u16>,
}

/// One run within a text-object paragraph — the engine's `ISCRParagraphElement`. A run is either a
/// literal text element (`field_ref == None`) or an embedded field reference (`field_ref ==
/// Some(raw)`). Both carry their own font, so a paragraph can mix fonts across runs.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TextRun {
    /// The rendered content of this run: the literal text for a literal run, or the engine-rendered
    /// reference (`{alias.field}` / `{@formula}` / `{?param}` / a special-field name) for a field run.
    /// Concatenating every run's `text`, paragraph by paragraph, reconstructs [`TextObject::display`].
    pub text: String,
    /// For an embedded-field run, the raw reference exactly as stored — `alias.name` for a database
    /// field, `@name` for a formula, `?name` for a parameter, or the plain display name for a Crystal
    /// special field. `None` for a literal text run.
    pub field_ref: Option<String>,
    /// The run's own font, when one was streamed for it. `None` inherits the object font.
    pub font: Option<Font>,
}

/// One paragraph (line) of a text object — the engine's `ISCRParagraph`. Its `runs` are the
/// literal/field elements streamed until the next paragraph or the end of the object.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Paragraph {
    /// The literal-text and embedded-field runs that make up this line, in document order.
    pub runs: Vec<TextRun>,
}

/// SDK: `ITextObject`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TextObject {
    /// The last literal text run (retained for compatibility; see [`display`](Self::display) for the
    /// full rendered content).
    pub text: String,
    /// Maximum lines the object may grow to (0 = unlimited).
    pub max_lines: i32,
    /// The object's default font and colour (runs may override it).
    pub font_color: FontColor,
    /// The text's reading order (left-to-right or right-to-left).
    pub reading_order: ReadingOrder,
    /// Embedded field/formula/parameter references: `alias.name` for database fields, `@name` for
    /// formulas, `?name` for parameters — counted toward each field's UseCount.
    pub embedded_fields: Vec<String>,
    /// The object's rendered content for `<Text>`: literal runs and embedded references (wrapped
    /// `{alias.field}`/`{@formula}`/`{?param}`) concatenated in document order. `text` keeps only the
    /// last literal run, so this is what the exporter emits.
    pub display: String,
    /// The structured paragraph→run tree, preserving per-run formatting
    /// and embedded field references for the renderer. [`display`](Self::display) is the flattened
    /// projection of this tree (runs joined, paragraphs joined by `\n`); the exporter emits `display`,
    /// so this field is purely additive.
    pub paragraphs: Vec<Paragraph>,
}

impl TextObject {
    /// Flatten the paragraph→run tree back to a single string: each paragraph's runs concatenated, a
    /// `\n` inserted before each paragraph once any content has been emitted. This exactly mirrors how
    /// [`display`](Self::display) is built (a paragraph break adds `\n` only when the text so far is
    /// non-empty, so leading empty paragraphs collapse), so `flattened_text() == display`. It is the
    /// accessor a renderer uses when it does not walk the run tree directly.
    pub fn flattened_text(&self) -> String {
        let mut out = String::new();
        for p in &self.paragraphs {
            if !out.is_empty() {
                out.push('\n');
            }
            for r in &p.runs {
                out.push_str(&r.text);
            }
        }
        out
    }
}

/// SDK: `IFieldHeadingObject` (a text object bound to a `FieldObject`).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FieldHeadingObject {
    /// The name of the field object this heading labels.
    pub field_object_name: String,
    /// The heading's literal text.
    pub text: String,
    /// Maximum lines the heading may grow to (0 = unlimited).
    pub max_lines: i32,
    /// The heading's font and colour.
    pub font_color: FontColor,
}

/// SDK: `IDrawingObject` members shared by box and line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DrawingShape {
    /// The shape's right edge, in twips.
    pub right: Twips,
    /// The shape's bottom edge, in twips.
    pub bottom: Twips,
    /// The line style (solid / dashed / dotted / …).
    pub line_style: LineStyle,
    /// The line thickness, in twips.
    pub line_thickness: Twips,
    /// The line colour.
    pub line_color: Color,
    /// Whether the shape stretches to the bottom of its section as the section grows.
    pub extend_to_bottom_of_section: bool,
}

/// SDK: `ILineObject`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LineShape {
    /// The line's geometry and stroke.
    pub shape: DrawingShape,
    /// The name of the section the line ends in (for lines that extend across sections).
    pub end_section_name: String,
}

/// SDK: `IBoxObject`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BoxShape {
    /// The box's geometry and stroke.
    pub shape: DrawingShape,
    /// The name of the section the box ends in (for boxes that extend across sections).
    pub end_section_name: String,
    /// The rounded-corner ellipse width, in twips (0 = square corners).
    pub corner_ellipse_width: Twips,
    /// The rounded-corner ellipse height, in twips (0 = square corners).
    pub corner_ellipse_height: Twips,
    /// The fill colour, or `None` for a transparent (unfilled) box.
    pub fill_color: Option<Color>,
}

/// SDK: `IBlobFieldObject` — a database blob/image field shown as a picture. It has no exported
/// `DataSource`, but the bound field reference (`{table.field}`) is kept so the derived analytics can
/// count it toward `Field.UseCount`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BlobFieldObject {
    /// The bound database field reference, brace-wrapped (`{Command.some_field}`).
    pub data_source: String,
}

/// SDK: `IPictureObject`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PictureObject {
    /// The picture's kind (OLE object / bitmap / metafile / …).
    pub picture_type: PictureType,
    /// The embedded image bytes, verbatim from the OLE `Embedding N/CONTENTS` stream (the whole
    /// picture file — a full `BM` bitmap in the entire public corpus). Empty when the picture has
    /// no OLE embedding (e.g. a chart drawn *through* a picture object, or a blob-field picture).
    /// Use [`Self::image_format`] to identify the wire format of these bytes.
    pub data: Vec<u8>,
    /// The 1-based `Embedding N` storage ordinal this picture's [`Self::data`] was loaded from.
    /// `None` for pictures with no OLE embedding.
    pub ole_ordinal: Option<u32>,
    /// The "graphic location" formula that conditionally swaps the picture at runtime, when set.
    pub location_formula: Option<Formula>,
}

impl PictureObject {
    /// The wire format of [`Self::data`], sniffed from its leading magic bytes.
    pub fn image_format(&self) -> ImageFormat {
        ImageFormat::sniff(&self.data)
    }

    /// The image bytes as a self-contained, browser-renderable file. For a bare
    /// [`ImageFormat::Dib`] this prepends a reconstructed 14-byte `BITMAPFILEHEADER` so the result
    /// is a valid `.bmp`; every other format (including a `Bmp` that already carries the file
    /// header) is returned unchanged. `None` for an empty payload.
    pub fn to_bmp(&self) -> Option<std::borrow::Cow<'_, [u8]>> {
        use std::borrow::Cow;
        if self.data.is_empty() {
            return None;
        }
        if self.image_format() != ImageFormat::Dib {
            return Some(Cow::Borrowed(&self.data));
        }
        // Prepend a BITMAPFILEHEADER: "BM", u32 total file size, 2×u16 reserved, u32 pixel offset.
        // The DIB header size is the leading u32; the colour table (if any) follows it. Assume a
        // packed DIB (pixels immediately after header + palette) — true for engine-produced DIBs.
        let dib = &self.data;
        let header_size = u32::from_le_bytes([dib[0], dib[1], dib[2], dib[3]]) as usize;
        // Palette entries: BITMAPINFOHEADER stores biClrUsed at offset 32 (u32); 0 ⇒ 2^biBitCount
        // for ≤8bpp, else none. BITMAPCOREHEADER (size 12) has no biClrUsed.
        let palette_bytes = if header_size >= 40 && dib.len() >= 36 {
            let bit_count = u16::from_le_bytes([dib[14], dib[15]]);
            let clr_used = u32::from_le_bytes([dib[32], dib[33], dib[34], dib[35]]) as usize;
            let colors = if clr_used != 0 {
                clr_used
            } else if bit_count <= 8 {
                1usize << bit_count
            } else {
                0
            };
            colors * 4
        } else {
            0
        };
        let pixel_offset = 14 + header_size + palette_bytes;
        let file_size = 14 + dib.len();
        let mut out = Vec::with_capacity(file_size);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(file_size as u32).to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]); // reserved
        out.extend_from_slice(&(pixel_offset as u32).to_le_bytes());
        out.extend_from_slice(dib);
        Some(Cow::Owned(out))
    }
}

/// SDK: `ISubreportObject` (the placeholder; the report itself is in `Report::subreports`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SubreportObject {
    /// The name of the subreport this placeholder renders.
    pub subreport_name: String,
    /// Whether the subreport is rendered on demand (SDK `EnableOnDemand`) rather than inline.
    pub on_demand: bool,
    /// Index `N` of the backing `Subdocument N` storage. Used to resolve [`Self::subreport_name`]
    /// from that subdocument's report-header record after the subreports are decoded.
    pub subdoc_index: u32,
}

/// SDK: `IChartObject` — chart definition + style (deferred detail).
///
/// The chart's persistent field bindings, decoded from its binding block. Each is the raw engine
/// reference form (`Table.field` for a database field, `@name` for a formula). These are not
/// exported to XML; they exist only for the derived `Field.UseCount`.
///
/// They are split by role because the engine references them a different number of times:
/// - `data_refs` — the chart's "show value" data bindings. A DB data binding is referenced **once**
///   (like a placed field object).
/// - `category_refs` — the chart's "on change of" category bindings. A DB category is built as an
///   internal group (condition + sort) per chart, so it is referenced **twice per chart**.
///
/// The derived analytics aggregate these into `Field.UseCount`; a formula binding instead makes that formula
/// *live* (its own references then count).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChartObject {
    /// Field references bound to the chart's data (value) axis, each a raw engine reference
    /// (`Table.field` or `@name`).
    pub data_refs: Vec<String>,
    /// Field references bound to the chart's category ("on change of") axis, each a raw engine
    /// reference (`Table.field` or `@name`).
    pub category_refs: Vec<String>,
    /// The chart's decoded type + titles + analytic data-value label. Not exported; a pure stored-fact
    /// decode for the model and future rendering.
    pub definition: ChartDefinition,
}

/// SDK: `ICrossTabObject` — the row/column dimension field bindings of a cross-tab grid.
///
/// `field_refs` are the cross-tab's persistent row/column ("on change of") dimension bindings,
/// carrying a `@Column #…`/`@Row #…` order marker. Each is the raw engine reference form
/// (`Table.field` or `@name`). The data-cell summaries (e.g.
/// `Sum of {Table.x}`) are counted via `<SummaryFields>` and are NOT included here. A DB dimension
/// is built as an internal group (condition + sort) plus an OLAP-grid registration, so the engine
/// references it **three times per dimension**; the derived analytics aggregate these into `Field.UseCount`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CrossTabObject {
    /// The cross-tab's persistent row/column dimension bindings, each a raw engine reference
    /// (`Table.field` or `@name`) carrying a `@Column #…`/`@Row #…` order marker.
    pub field_refs: Vec<String>,
    /// The cross-tab's combined row/column **dimension structure**. One [`CrossTabDimension`] per
    /// level, in stream order (all column levels then all row levels) — a superset of
    /// [`field_refs`](Self::field_refs) that also preserves the grand-total (empty-`field_ref`) levels.
    /// See [`columns`](Self::columns) / [`rows`](Self::rows) for the axis-split view. STRUCTURAL:
    /// not exported; decoded for the model.
    pub dimensions: Vec<CrossTabDimension>,
    /// The **column** dimension levels (the "across" axis), in nesting order — the axis whose
    /// generated field objects are named `Column #N Name`. The first level is the grand-total column
    /// (empty [`field_ref`](CrossTabDimension::field_ref)); the remaining levels are the ordered
    /// column-grouping fields. STRUCTURAL (not exported).
    pub columns: Vec<CrossTabDimension>,
    /// The **row** dimension levels (the "down" axis), in nesting order — the axis whose generated
    /// field objects are named `Row #N Name`. The first level is the grand-total row (empty
    /// [`field_ref`](CrossTabDimension::field_ref)); the remaining levels are the ordered
    /// row-grouping fields. STRUCTURAL (not exported).
    pub rows: Vec<CrossTabDimension>,
    /// The cross-tab's **measures** (data-cell summaries) — the aggregation applied to each
    /// row×column intersection, in stacking order (see [`CrossTabMeasure`]). STRUCTURAL (not exported
    /// surface).
    pub measures: Vec<CrossTabMeasure>,
    /// The grid's cell/region formatting — the grid-level format word plus one format per fixed
    /// formattable grid region (see [`CrossTabGridFormat`]). Decoded from the `0x0143`/`0x0145`
    /// records inside the `0xb9` wrapper. STRUCTURAL (not exported).
    pub grid_format: CrossTabGridFormat,
    /// The column-axis option word (the `0x00ce` level record's 2-byte leaf, big-endian; shared by
    /// every column level). Raw — its bit meanings are not yet confirmed; `0` throughout the corpus.
    /// STRUCTURAL.
    pub column_axis_options: u16,
    /// The row-axis option word (the `0x00d2` level record's 2-byte leaf, big-endian; shared by every
    /// row level). Raw — its bit meanings are not yet confirmed (`0x0003` vs `0x0000` in the corpus).
    /// STRUCTURAL.
    pub row_axis_options: u16,
    /// The cross-tab's grid **display options** and grand-total background colours — the RAS
    /// `ISCRCrossTabStyle` view. Decoded from the `0xb8`/`0xb9` cross-tab records and the grand-total
    /// `0x00cb` dimension levels (see [`CrossTabGridOptions`]). STRUCTURAL.
    pub options: CrossTabGridOptions,
}

/// SDK: `ISCRCrossTabStyle` — a cross-tab's grid display options and grand-total background colours.
///
/// The six booleans are decoded from the `0xb8 CrossTabObject` leaf (offsets 1/10/22/24/26/28); the
/// two grand-total suppress flags from the `0xb9 CrossTabObjectWrapper` leaf (offsets 1/3); and the
/// two grand-total colours from the first (grand-total) `0x00cb` dimension level of each axis
/// (leaf `[0..4]`, big-endian `COLORREF`). The colour axes are **cross-wired** as the SDK exposes
/// them: RAS `RowGrandTotalColor` is the colour of the first *column*-axis level, and
/// `ColumnGrandTotalColor` the first *row*-axis level. Verified field-by-field against RAS
/// `CrossTabStyle` on the synthetic crosstab fixtures. STRUCTURAL (no RptToXml surface).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CrossTabGridOptions {
    /// RAS `EnableShowGrid` — draw the cell grid lines.
    pub show_grid: bool,
    /// RAS `EnableShowCellMargins`.
    pub show_cell_margins: bool,
    /// RAS `EnableKeepColumnsTogether` — keep each column block together across a page break.
    pub keep_columns_together: bool,
    /// RAS `EnableRepeatRowLabels` — repeat the row labels on each page.
    pub repeat_row_labels: bool,
    /// RAS `EnableSuppressEmptyRows`.
    pub suppress_empty_rows: bool,
    /// RAS `EnableSuppressEmptyColumns`.
    pub suppress_empty_columns: bool,
    /// RAS `EnableSuppressRowGrandTotals`.
    pub suppress_row_grand_totals: bool,
    /// RAS `EnableSuppressColumnGrandTotals`.
    pub suppress_column_grand_totals: bool,
    /// RAS `RowGrandTotalColor` — the row grand-total cells' background. `None` = auto (stored
    /// `COLORREF` `0xFFFFFFFF`).
    pub row_grand_total_color: Option<Color>,
    /// RAS `ColumnGrandTotalColor` — the column grand-total cells' background. `None` = auto.
    pub column_grand_total_color: Option<Color>,
}

/// SDK: `ICrossTabObject` grid formatting — the grid-level format word plus one format per fixed
/// formattable grid region. Decoded from the `0x0143 CrossTabGridFormat` word and the following
/// `0x0145 CrossTabGridCellFormat` records inside the cross-tab's `0xb9` wrapper. STRUCTURAL (no
/// XML/oracle surface; a stored-fact decode for the model and future rendering).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CrossTabGridFormat {
    /// The grid-level format word (`0x0143`, u16 big-endian). Constant `0x0014` across the corpus
    /// (equal to the [`cells`](Self::cells) count); its individual bit meanings are not yet
    /// confirmed, so it is preserved raw.
    pub raw: u16,
    /// Per grid-region cell formats (`0x0145`), one per fixed formattable region. The count is fixed
    /// by the cross-tab template (20 in the corpus), independent of the grid's actual row/column
    /// counts.
    pub cells: Vec<CrossTabCellFormat>,
}

/// One cross-tab grid-region cell format (`0x0145`, 11 bytes). Carries the region's background colour
/// and its format-override flags. STRUCTURAL.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CrossTabCellFormat {
    /// Format-override flags (bytes `[0..4]`, big-endian). `0` = the region uses the grid defaults; a
    /// non-zero value marks a region carrying explicit cell formatting (observed value `0x28`).
    pub flags: u32,
    /// The region background colour (bytes `[6..10]`, a little-endian `COLORREF` `[R, G, B, 0]`).
    /// `None` when unset (the colour bytes are all zero). Inferred, not oracle-verified — the
    /// grid-region template is engine-internal (not exposed by RAS or the HTML render).
    pub background_color: Option<Color>,
    /// The region's enabled/visible flag (byte `[10]`).
    pub enabled: bool,
}

/// One measure of a cross-tab grid — the summary (aggregation + summarized field) shown in every
/// row×column data cell. [`operation`] is the aggregation and [`field`] the summarized field
/// reference (`Table.field` or a `@formula`). STRUCTURAL.
///
/// [`operation`]: Self::operation
/// [`field`]: Self::field
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CrossTabMeasure {
    /// The aggregation applied to the summarized field (e.g. `Sum`, `Count`, `Maximum`).
    pub operation: SummaryOperation,
    /// The summarized field reference (`Table.field` for a database field, `@name` for a formula).
    pub field: String,
}

/// One dimension level of a cross-tab grid — its bound dimension field reference (a `Table.field` or
/// `@formula`); a grand-total level carries an empty reference. STRUCTURAL.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CrossTabDimension {
    /// The dimension's bound field reference (`Table.field` or `@formula`); empty for a grand-total
    /// level.
    pub field_ref: String,
}

/// The visual shape of a chart (bar / line / pie / …), decoded from the first enum field of
/// the `0x0121 ChartDefinition2` record (leaf `+0x4c`).
///
/// IMPORTANT — this axis has **no published numeric enum**. The RAS SDK's `CrChartTypeEnum`
/// (`crChartTypeDetail`, …) describes the chart *layout* (Detail / Group / Cross-Tab / OLAP), **not**
/// the visual shape — every corpus chart is a Group chart yet this field varies, so it cannot be the
/// layout axis. The code→name mapping below is therefore **inferred from the corpus**, not confirmed
/// against a source of truth:
/// every pie-shaped chart (a demographic breakdown with no value axis — its data-axis title is
/// empty) stores code `3`; the sole time-series (plotted over a date axis)
/// stores code `1`; every axis-bearing category chart stores code `0` (the engine default). Code `2`
/// (area) is named though the corpus never exercises it; the many other shapes with
/// no corpus evidence round-trip losslessly through [`ChartGraphType::Other`], their raw
/// subtype/variant selector preserved separately in [`ChartDefinition::graph_subtype`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartGraphType {
    /// Code `0` — a bar/column chart (the engine default and corpus majority; has both axes).
    #[default]
    Bar,
    /// Code `1` — a line chart (corpus: the single time-series plotted over a date axis).
    Line,
    /// Code `2` — an area chart (a line chart whose region to the baseline is filled).
    /// No corpus evidence, but the code is named so the renderer can draw it as an area rather than
    /// silently falling back to bars.
    Area,
    /// Code `3` — a pie chart (corpus: demographic breakdowns, which carry no value axis).
    Pie,
    /// Code `4` — a doughnut chart (a pie with an inner-radius ring). **High** confidence: the
    /// writer branch `type==3 || type==4` emits the pie-only detach/rotate enum pair for
    /// both codes, so 4 is a proven pie-family member; no corpus byte sample.
    Doughnut,
    /// Code `5` — a 3-D riser ("3-D bar") chart. **Confirmed** disk byte (the `3d`
    /// variant stores `05`); one of the inherently three-dimensional families ([`ChartDefinition::is_3d`]).
    Riser3D,
    /// Code `6` — a 3-D surface chart (a continuous meshed riser field). **High** confidence: the
    /// writer branch `type==5 || type==6` emits an extra 3-D enum for both, grouping 6 with the 3-D
    /// riser; the other inherently 3-D family.
    Surface3D,
    /// Code `7` — an XY scatter chart (two numeric axes, markers, no connecting line). Gallery-confirmed
    /// position (designer Chart Expert); code = gallery index. No byte sample.
    Scatter,
    /// Code `8` — a radar / polar chart (angular category + radial value axis). Gallery-confirmed
    /// position (Chart Expert: Radar follows XY Scatter). No byte sample.
    Radar,
    /// Code `9` — a bubble chart (an XY scatter whose marker size encodes a third value).
    /// Gallery-confirmed position. No byte sample.
    Bubble,
    /// Code `10` — a stock (hi-lo / OHLC) chart. Gallery-confirmed position. No byte sample.
    Stock,
    /// Code `11` — a numeric-axis chart (bar/line/area over a numeric or date X axis rather than an
    /// ordinal category). Gallery-confirmed position. No byte sample.
    NumericAxis,
    /// Code `12` — a gauge (a dial/needle over an arc scale). Gallery-confirmed position. No byte sample.
    Gauge,
    /// Code `13` — a Gantt chart (horizontal time bars per record). Gallery-confirmed position.
    Gantt,
    /// Code `14` — a funnel chart (stacked proportional trapezoids; internal `XBI2_FUNNEL`).
    /// Gallery-confirmed position. No byte sample.
    Funnel,
    /// Code `15` — a histogram (frequency bars over binned value ranges). Gallery-confirmed position.
    Histogram,
    /// Any code with no corpus evidence and no named renderer dispatch, preserved verbatim.
    Other(i32),
}

impl ChartGraphType {
    /// Map the raw `+0x4c` enum code to a [`ChartGraphType`]. The code equals the
    /// designer's Chart Expert gallery index (Bar 0 … Histogram 15). Every named code is
    /// byte-confirmed from a real report fixture except `9` (bubble) and `11` (numeric-axis), which
    /// rest on the confirmed gallery order. Every code past the gallery round-trips through
    /// [`ChartGraphType::Other`].
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::Bar,
            1 => Self::Line,
            2 => Self::Area,
            3 => Self::Pie,
            4 => Self::Doughnut,
            5 => Self::Riser3D,
            6 => Self::Surface3D,
            7 => Self::Scatter,
            8 => Self::Radar,
            9 => Self::Bubble,
            10 => Self::Stock,
            11 => Self::NumericAxis,
            12 => Self::Gauge,
            13 => Self::Gantt,
            14 => Self::Funnel,
            15 => Self::Histogram,
            other => Self::Other(other),
        }
    }
}

/// How a multi-series axis chart (bar / area / line) arranges its series within each category slot —
/// the "chart subtype" arrangement axis, distinct from the visual [`ChartGraphType`].
///
/// At render time the native engine uses this to drive both the axis limits ([stacked/percent
/// scaling](ChartArrangement::Percent)) and the riser placement. **On disk it is the low digit of
/// `graph_subtype`** (the
/// variant slot within the type's gallery band): for the Bar family `0`/`1`/`2` = clustered / stacked
/// / percent, confirmed by the bar minimal pairs. The slot is reused for unrelated per-family variants
/// (e.g. the Area family's depth-effect bit), so [`ChartDefinition::arrangement`] decodes stacked/
/// percent only for the Bar family and reports [`Clustered`](Self::Clustered) elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartArrangement {
    /// Series drawn side-by-side within each category slot (the engine default).
    #[default]
    Clustered,
    /// Series accumulated on top of one another (stacked bars/areas).
    Stacked,
    /// Series stacked and normalized to 100% (the render-time percent mode).
    Percent,
}

/// Where a chart's legend is placed relative to the plot area, decoded from the high byte of the
/// legend `short` at the start of the `0x0121` styling struct (`+0x410`).
///
/// Confidence: `Right` (the engine default) and `Bottom` are **CONFIRMED** against single-property
/// synthetic reports; `Left` and `Top` are **conjecture** — the natural Crystal
/// legend-placement ordering (the designer Legend tab exposes Right / Left / Top / Bottom), but the
/// codes `1`/`3` are not yet sampled. Any unrecognized code defaults to `Right`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartLegendPosition {
    /// Code `0` — legend to the right of the plot (the engine default). CONFIRMED.
    #[default]
    Right,
    /// Code `1` — legend to the left of the plot. Conjecture (unsampled).
    Left,
    /// Code `2` — legend below the plot. CONFIRMED.
    Bottom,
    /// Code `3` — legend above the plot. Conjecture (unsampled).
    Top,
}

impl ChartLegendPosition {
    /// Map the raw `0x0121` legend-position enum byte (the high byte of the `+0x410` short) to a
    /// [`ChartLegendPosition`]. `0`/`2` are confirmed; `1`/`3` are conjectured; anything else falls
    /// back to the default `Right`.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Right,
            1 => Self::Left,
            2 => Self::Bottom,
            3 => Self::Top,
            _ => Self::Right,
        }
    }
}

/// The gridline mode of one of a chart's axes (the Axes-tab "Gridlines" choice), decoded from the
/// `0x0121` styling struct's per-axis gridline bytes (`CrGridTypeEnum`). A bitmask: bit0 = minor
/// gridlines, bit1 = major gridlines, so [`Both`](ChartGridType::Both) draws both. RAS-confirmed
/// against the corpus: the group (category) axis is [`None`](ChartGridType::None) and the value axis
/// [`Major`](ChartGridType::Major) on the default charts, and both are [`Both`](ChartGridType::Both)
/// on the `*_legend_*` fixtures — the only stored gridline variation the corpus exercises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartGridType {
    /// Code `0` — no gridlines on this axis (the group-axis default).
    #[default]
    None,
    /// Code `1` — minor gridlines only (bit0).
    Minor,
    /// Code `2` — major gridlines only (bit1); the value-axis default.
    Major,
    /// Code `3` — both major and minor gridlines (bit0 | bit1).
    Both,
}

impl ChartGridType {
    /// Map the raw `CrGridTypeEnum` byte (bit0 minor, bit1 major) to a [`ChartGridType`]. Values above
    /// `3` keep only the low two bits; a value with neither bit set is [`None`](ChartGridType::None).
    pub fn from_code(code: u8) -> Self {
        match code & 0x03 {
            1 => Self::Minor,
            2 => Self::Major,
            3 => Self::Both,
            _ => Self::None,
        }
    }
}

/// The date-grouping period of a chart's "on change of `<date>`" category — the interval each
/// category bucket spans (weekly, monthly, …), the chart-side analogue of a report group's
/// [`Group::date_condition`](crate::Group::date_condition).
///
/// Decoded from the chart's category grid `0xe5` group record (the same record a report group uses),
/// at the SDK `CrGroupConditionEnum` ordinal byte `used + 3` (the fourth byte after the category
/// field reference), via [`from_sdk_ordinal`](Self::from_sdk_ordinal). This is the identical
/// encoding proven for report groups (weekly = 1, monthly = 4, cross-checked against the engine's
/// `GroupName` XML), reused here for the chart's category axis.
///
/// **Confidence / corpus coverage.** `Weekly` and `Monthly` are byte-confirmed against the parking
/// chart fixtures *and* against the engine's own rendered category-axis labels (`orders_weekly` →
/// 25 weekly buckets, `orders` → 6 monthly buckets). `Biweekly`/`Semimonthly`/`Quarterly`/
/// `SemiAnnually`/`Annually` follow the confirmed SDK enum ordering but have no non-confounded
/// fixture. `Daily` (ordinal 0) is deliberately **not** decoded from this byte: ordinal 0 also means
/// "no period" on a discrete category, and the legacy daily flag (`used + 4 == 0x02`) doubles as a
/// sort attribute on non-date fields, so mapping it here without a field-type gate would produce
/// false positives — a daily chart category currently reads `None`.
///
/// **Known residual (biweekly is not byte-isolable here).** The engine renders `chart_stock` /
/// `chart_stock_open` with **biweekly** (13) category buckets, yet their category grid `0xe5`
/// stores `used + 3 == 1` (weekly) — the exact byte `orders_weekly` (a genuinely weekly, 25-bucket
/// chart) stores. Every byte that differs between them is confounded with those being *stock* charts
/// (`0x011c[4] == 3`, a longer `0x0126`) or having *no report group* (grid `used + 2 == 0`), and no
/// record byte reads the biweekly ordinal `2`. So a stock chart's biweekly grouping decodes as
/// `Weekly` (its stored value); resolving it needs native-engine RE or a non-confounded fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartCategoryPeriod {
    /// SDK ordinal 0 — one bucket per day. Not decoded from `used + 3` (see the type docs); listed
    /// for completeness / render-side use.
    Daily,
    /// SDK ordinal 1 — one bucket per week. CONFIRMED (`orders_weekly`, engine axis labels).
    Weekly,
    /// SDK ordinal 2 — one bucket per two weeks. Confirmed SDK ordering; no non-confounded fixture.
    Biweekly,
    /// SDK ordinal 3 — one bucket per half-month. Confirmed SDK ordering; no fixture.
    Semimonthly,
    /// SDK ordinal 4 — one bucket per month. CONFIRMED (`orders`, engine axis labels).
    Monthly,
    /// SDK ordinal 5 — one bucket per quarter. Confirmed SDK ordering; no fixture.
    Quarterly,
    /// SDK ordinal 6 — one bucket per half-year. Confirmed SDK ordering; no fixture.
    SemiAnnually,
    /// SDK ordinal 7 — one bucket per year. Confirmed SDK ordering; no fixture.
    Annually,
}

impl ChartCategoryPeriod {
    /// Map the SDK `CrGroupConditionEnum` ordinal (the chart category grid `0xe5` leaf byte
    /// `used + 3`) to a period. Ordinals `1..=7` map to the eight date periods; ordinal `0` returns
    /// `None` (it means daily *or* no-period and cannot be told apart here without a field-type gate —
    /// see the type docs), as does any out-of-range ordinal.
    pub fn from_sdk_ordinal(ordinal: u8) -> Option<Self> {
        match ordinal {
            1 => Some(Self::Weekly),
            2 => Some(Self::Biweekly),
            3 => Some(Self::Semimonthly),
            4 => Some(Self::Monthly),
            5 => Some(Self::Quarterly),
            6 => Some(Self::SemiAnnually),
            7 => Some(Self::Annually),
            _ => None,
        }
    }

    /// The lowercase canonical token for this period — the same token stored on a report group's
    /// [`Group::date_condition`](crate::Group::date_condition) and matched by the render
    /// pipeline's date bucketer.
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Biweekly => "biweekly",
            Self::Semimonthly => "semimonthly",
            Self::Monthly => "monthly",
            Self::Quarterly => "quarterly",
            Self::SemiAnnually => "semiannually",
            Self::Annually => "annually",
        }
    }
}

/// The 3-D camera preset a chart is drawn with — the SDK `CrViewingAngleEnum`
/// (`ISCRChartStyleInternal.ViewingAngle`). Sixteen named presets; each selects a fixed
/// elevation/rotation and per-axis box aspect the native engine bakes into a 3-D rotation +
/// perspective (one fixed geometry block per preset). The
/// render side maps each variant to a concrete projection view angle.
///
/// The ordinal matches the SDK enum order (Standard = 0). Only [`Standard`](Self::Standard) is
/// exercised by the corpus (every chart fixture reports `crViewingAngleStandard` via RAS), so it is
/// the sole confirmed value and the default; the other fifteen presets' geometry is recovered from
/// the native preset blocks but their disk byte→preset mapping is not yet isolated (needs a report
/// that actually sets a non-default angle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChartViewAngle {
    /// Ordinal 0 — the default corner view (elevation ~36°, rotation ~42°, square floor). CONFIRMED
    /// default; the only preset any corpus chart uses.
    #[default]
    Standard,
    /// Ordinal 1 — a taller value axis.
    TallView,
    /// Ordinal 2 — looking down from nearly overhead (elevation ~80°).
    TopView,
    /// Ordinal 3 — a distorted (stretched value axis) standard view.
    DistortedView,
    /// Ordinal 4 — a shorter value axis.
    ShortView,
    /// Ordinal 5 — rotated to emphasise the group (category) axis from the side.
    GroupEyeView,
    /// Ordinal 6 — a low, group-emphasising view.
    GroupEmphasisView,
    /// Ordinal 7 — deepened series axis for a chart with few series.
    FewSeriesView,
    /// Ordinal 8 — shallow series axis for a chart with few groups.
    FewGroupsView,
    /// Ordinal 9 — a milder-elevation distorted standard view.
    DistortedStdView,
    /// Ordinal 10 — thickened group axis, low elevation.
    ThickGroupsView,
    /// Ordinal 11 — an even shorter value axis than [`ShortView`](Self::ShortView).
    ShorterView,
    /// Ordinal 12 — thickened series axis, strongly rotated.
    ThickSeriesView,
    /// Ordinal 13 — a thicker standard view.
    ThickStdView,
    /// Ordinal 14 — a steep overhead "bird's-eye" view.
    BirdsEyeView,
    /// Ordinal 15 — the widest/deepest overall box.
    MaxView,
}

impl ChartViewAngle {
    /// Map the SDK `CrViewingAngleEnum` ordinal to a preset. Out-of-range ordinals fall back to the
    /// default [`Standard`](Self::Standard).
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Standard,
            1 => Self::TallView,
            2 => Self::TopView,
            3 => Self::DistortedView,
            4 => Self::ShortView,
            5 => Self::GroupEyeView,
            6 => Self::GroupEmphasisView,
            7 => Self::FewSeriesView,
            8 => Self::FewGroupsView,
            9 => Self::DistortedStdView,
            10 => Self::ThickGroupsView,
            11 => Self::ShorterView,
            12 => Self::ThickSeriesView,
            13 => Self::ThickStdView,
            14 => Self::BirdsEyeView,
            15 => Self::MaxView,
            _ => Self::Standard,
        }
    }
}

/// SDK: `IChartObject.ChartDefinition` — the semantically meaningful, byte-legible slice of a
/// chart's configuration: its visual type, subtype, and the title/axis-title/data-label strings.
///
/// Decoded from the `0x0121 ChartDefinition2` record,
/// whose leaf is `[enum +0x4c][enum +0x50]` (one byte each) followed by a run of length-prefixed
/// (`u32` big-endian byte count, NUL-terminated) strings: title (`+0x54`), subtitle (`+0x58`),
/// footnote (`+0x5c`), two format-mask strings, then the group-axis (`+0x60`) and data-axis titles;
/// [`data_label`](Self::data_label) comes from the sibling `0x011f ChartDataValue` record.
///
/// Internal (not exported). The byte-legible strings plus the legend
/// visible/position flags (which open the fixed styling struct after the string run) are modeled;
/// the remainder of the `0x0121` leaf (~380–500 bytes) is opaque fixed-schema render styling — axis
/// scale/min-max, marker/riser/colour state — left deliberately undecoded because it has no reader
/// value and no way to validate.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ChartDefinition {
    /// The chart's visual shape (see [`ChartGraphType`]); from the first `0x0121` enum (`+0x4c`).
    pub graph_type: ChartGraphType,
    /// The chart's subtype/variant selector (e.g. side-by-side vs. stacked vs. percent for a bar
    /// chart); the second `0x0121` enum (`+0x50`). No confirmed taxonomy exists (unconfirmed), so the
    /// raw engine code is preserved verbatim.
    pub graph_subtype: i32,
    /// The chart title (`0x0121` field `+0x54`).
    pub title: String,
    /// The chart subtitle (`+0x58`; empty across the corpus).
    pub subtitle: String,
    /// The chart footnote (`+0x5c`; empty across the corpus).
    pub footnote: String,
    /// The group ("on change of") axis title — the category / X axis (`+0x60`).
    pub group_axis_title: String,
    /// The data ("show value") axis title — the value / Y axis.
    pub data_axis_title: String,
    /// The chart's data-value description from the `0x011f ChartDataValue` analytic record (e.g.
    /// `"Count of Command.some_field"`); empty for the grouped `0x0126` analytic variant, which has
    /// no such labeled value.
    pub data_label: String,
    /// The date-grouping period of the chart's "on change of `<date>`" category axis, when the
    /// category is a date field grouped by a period (see [`ChartCategoryPeriod`]). Decoded from the
    /// chart's category grid `0xe5` group record's SDK-ordinal byte; `None` when the category is a
    /// discrete (non-periodic) field, a daily period (not decodable here), or the chart has no
    /// category. Lets the renderer bucket the chart's date axis by the authored period instead of a
    /// fixed default.
    pub category_period: Option<ChartCategoryPeriod>,
    /// Whether the chart's legend is shown — bit0 of the low byte of the `0x0121` styling struct's
    /// leading legend `short` (`+0x410`). CONFIRMED against legend on/off minimal pairs.
    pub legend_visible: bool,
    /// Where the legend is placed (see [`ChartLegendPosition`]) — the high byte of the same legend
    /// `short`. `Right`/`Left`/`Bottom` are confirmed; `Top` is conjecture. When the legend is
    /// hidden the stored position is not meaningful (the engine resets it on save).
    pub legend_position: ChartLegendPosition,
    /// The group (category, X) axis gridline mode (see [`ChartGridType`]) — the `0x0121` styling
    /// struct's group-axis gridline byte. RAS-confirmed; the axis families only. `None` for the
    /// Pie/Doughnut/Funnel/Gauge families, which carry no group axis.
    pub group_axis_gridlines: ChartGridType,
    /// The value (Y) axis gridline mode (see [`ChartGridType`]) — the `0x0121` styling struct's
    /// value-axis gridline byte. RAS-confirmed; the axis families only. `None` for the
    /// Pie/Doughnut/Funnel/Gauge families, which carry no value axis.
    pub value_axis_gridlines: ChartGridType,
    /// Whether the chart shows per-point data-value labels — bit1 of the data-labels enum byte in the
    /// `0x0121` styling struct (`+0x4a8`), reached by a fixed-width walk from the legend
    /// `short` (`00`→`02` when enabled). CONFIRMED bit; the corpus samples are all
    /// off (`false`), so the positive case rests on the differential byte-map, not a fixture here.
    pub data_labels_show_value: bool,
    /// Per-series RGB fill colours, in series order — empty when the chart uses automatic colouring
    /// (the common case) or when no explicit colours are byte-recoverable.
    ///
    /// **Not byte-recoverable from the `.rpt` in general.** Crystal has no
    /// fixed series palette: the engine default is a runtime `rand()`-seeded assignment, and the
    /// designer bakes chosen colours into the auto-recomputed colour/GDI-handle state at `0x0121`
    /// `+0x49c..+0x4a4` (flagged "auto-recomputed" — it changes on every dialog re-save and is not a
    /// stable stored RGB triple) and pushes them into runtime chart state, not as a decodable disk field.
    /// This field is the render-side hook: it is populated only if a stable, explicit per-series RGB
    /// is ever recovered; otherwise the render side must supply a built-in fallback palette.
    /// Currently always empty across the corpus.
    pub series_colors: Vec<Color>,
    /// The 3-D camera preset (see [`ChartViewAngle`]) — only meaningful for a 3-D chart
    /// ([`is_3d`](Self::is_3d)). Always [`ChartViewAngle::Standard`] across the corpus (the sole
    /// preset any fixture uses); the field is the render-side hook for the per-chart angle. The disk
    /// byte that would select a non-default preset lives in the `0x0121` styling struct but is not
    /// yet byte-isolated, so this currently always decodes to the default.
    pub view_angle: ChartViewAngle,
}

impl ChartDefinition {
    /// Whether the chart is drawn with a genuine 3-D scene (perspective-projected risers/walls or a
    /// meshed surface) rather than flat 2-D geometry — the signal a renderer needs to pick the 3-D
    /// geometry path (the engine routes real 3-D math for the 3-D families, not a
    /// 2-D offset).
    ///
    /// Decoded from the [`graph_type`](Self::graph_type) disk enum: `true` only for the inherently
    /// three-dimensional families [`Riser3D`](ChartGraphType::Riser3D) (code 5, **confirmed**) and
    /// [`Surface3D`](ChartGraphType::Surface3D) (code 6, high).
    ///
    /// **This is distinct from the 2-D "depth effect" checkbox** (a shallow-z look on an otherwise
    /// flat chart), which does *not* make a chart 3-D and is reported separately by
    /// [`has_depth_effect`](Self::has_depth_effect). `is_3d` stays `false` for a 2-D type with depth
    /// enabled.
    pub fn is_3d(&self) -> bool {
        matches!(
            self.graph_type,
            ChartGraphType::Riser3D | ChartGraphType::Surface3D
        )
    }

    /// Whether the 2-D "depth effect" checkbox is on — a shallow-z (extruded) look drawn on an
    /// otherwise flat chart, distinct from a genuine 3-D chart type ([`is_3d`](Self::is_3d) stays
    /// `false`). A renderer can use this to give a 2-D chart a shallow-3-D appearance.
    ///
    /// Decoded as **bit `0x02` of [`graph_subtype`](Self::graph_subtype)**: the Area minimal pair
    /// toggles the subtype `0x14`→`0x16` (gallery band base 20, variant `+2`) when depth is enabled.
    ///
    /// **Scoped to the Area family.** The same bit is the [`Percent`](ChartArrangement::Percent)
    /// arrangement variant for the Bar family (subtype `2`), so a bar-percent chart must not read as
    /// depth. Depth-effect on the Bar/Line families cannot be isolated from this bit (it collides
    /// with the arrangement variant) and awaits its own minimal pair.
    pub fn has_depth_effect(&self) -> bool {
        self.graph_type == ChartGraphType::Area && (self.graph_subtype & 0x02 != 0)
    }

    /// How the chart arranges multiple series within each category slot (clustered / stacked /
    /// percent) — the render dispatch for the axis families (bar/area/line).
    ///
    /// Decoded from the low digit of [`graph_subtype`](Self::graph_subtype) — the variant slot within
    /// the type's 10-wide gallery band. For the **Bar family** the slot selects the
    /// series arrangement: `0` = clustered (side-by-side), `1` = stacked, `2` = percent, confirmed by
    /// the bar minimal pairs. Other families reuse the slot for unrelated variants (e.g. the Area
    /// family's depth-effect bit, see [`has_depth_effect`](Self::has_depth_effect)), so the
    /// stacked/percent read is scoped to Bar; every other family defaults to
    /// [`Clustered`](ChartArrangement::Clustered) until its own stacked/percent fixture pins the slot.
    pub fn arrangement(&self) -> ChartArrangement {
        if self.graph_type == ChartGraphType::Bar {
            match self.graph_subtype % 10 {
                1 => ChartArrangement::Stacked,
                2 => ChartArrangement::Percent,
                _ => ChartArrangement::Clustered,
            }
        } else {
            ChartArrangement::Clustered
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(s: &str) -> TextRun {
        TextRun {
            text: s.to_string(),
            field_ref: None,
            font: None,
        }
    }

    fn fref(rendered: &str, raw: &str) -> TextRun {
        TextRun {
            text: rendered.to_string(),
            field_ref: Some(raw.to_string()),
            font: None,
        }
    }

    /// A two-paragraph object with a mixed literal+field run reconstructs its `display` verbatim.
    #[test]
    fn flattened_text_joins_paragraphs_and_runs() {
        let t = TextObject {
            paragraphs: vec![
                Paragraph {
                    runs: vec![lit("Hello "), fref("{?name}", "?name")],
                },
                Paragraph {
                    runs: vec![lit("world")],
                },
            ],
            ..Default::default()
        };
        assert_eq!(t.flattened_text(), "Hello {?name}\nworld");
    }

    /// Leading empty paragraphs collapse exactly as `display` builds it (a paragraph break only adds
    /// `\n` once content exists), so a leading empty line does not produce a spurious `\n`.
    #[test]
    fn flattened_text_collapses_leading_empty_paragraphs() {
        let t = TextObject {
            paragraphs: vec![
                Paragraph::default(),
                Paragraph {
                    runs: vec![lit("text")],
                },
            ],
            ..Default::default()
        };
        assert_eq!(t.flattened_text(), "text");
    }

    /// A trailing empty paragraph keeps its blank line (its break fires after content exists).
    #[test]
    fn flattened_text_keeps_trailing_blank_line() {
        let t = TextObject {
            paragraphs: vec![
                Paragraph {
                    runs: vec![lit("a")],
                },
                Paragraph::default(),
            ],
            ..Default::default()
        };
        assert_eq!(t.flattened_text(), "a\n");
    }

    /// `ChartViewAngle::from_code` maps each SDK `CrViewingAngleEnum` ordinal (0..=15) to its preset
    /// in order, and any out-of-range ordinal falls back to the default `Standard`.
    #[test]
    fn chart_view_angle_from_code_maps_ordinals() {
        use ChartViewAngle::*;
        let order = [
            Standard,
            TallView,
            TopView,
            DistortedView,
            ShortView,
            GroupEyeView,
            GroupEmphasisView,
            FewSeriesView,
            FewGroupsView,
            DistortedStdView,
            ThickGroupsView,
            ShorterView,
            ThickSeriesView,
            ThickStdView,
            BirdsEyeView,
            MaxView,
        ];
        for (i, want) in order.iter().enumerate() {
            assert_eq!(ChartViewAngle::from_code(i as u8), *want, "ordinal {i}");
        }
        assert_eq!(
            ChartViewAngle::from_code(16),
            Standard,
            "out-of-range → default"
        );
        assert_eq!(
            ChartViewAngle::from_code(255),
            Standard,
            "out-of-range → default"
        );
        assert_eq!(ChartViewAngle::default(), Standard);
    }
}
