//! Report objects (SDK: `IReportObject` and its kinds).

use super::data_def::GroupCondition;
use super::enums::{
    FieldValueType, ImageFormat, LineSpacingType, LineStyle, PictureType, ReadingOrder,
    SummaryOperation,
};
use super::format::{Border, FieldFormat, Font, FontColor, ObjectFormat};
use super::primitives::{Color, Formula, RecordRef, Rect, Twips};

mod chart;

pub use chart::{
    ChartArrangement, ChartCategoryPeriod, ChartDefinition, ChartElementFont, ChartGraphType,
    ChartGridType, ChartLegendPosition, ChartViewAngle,
};

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
    /// SDK `IFieldObject` — a single field/formula/parameter value (boxed; `FieldObject` is large).
    Field(Box<FieldObject>),
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
    /// SDK: `ICrossTabObject` — a cross-tab grid, carrying its decoded row/column dimension field
    /// bindings ([`CrossTabObject`]).
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
    /// The paragraph's indentation (SDK `IParagraphFormat.IndentAndSpacingFormat`), in twips.
    /// Decoded from the `0x00c0` paragraph-format record; `LeftIndent` is the only member that
    /// is ever non-zero in practice.
    pub indent: IndentAndSpacingFormat,
}

/// SDK: `IIndentAndSpacingFormat` — a paragraph's left/right/first-line indentation, in twips.
/// Right- and first-line indents are `0` on a text paragraph (they carry paragraph-numbering
/// geometry that authored text objects do not use); only `left_indent` varies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct IndentAndSpacingFormat {
    /// The paragraph's left indent, in twips.
    pub left_indent: Twips,
    /// The paragraph's right indent, in twips.
    pub right_indent: Twips,
    /// The paragraph's first-line indent, in twips.
    pub first_line_indent: Twips,
    /// The paragraph's line spacing (SDK `LineSpacing` + `LineSpacingType`).
    pub line_spacing: LineSpacing,
}

/// SDK: `IIndentAndSpacingFormat.LineSpacing` + `.LineSpacingType` — a paragraph's line pitch.
///
/// The stored value's meaning depends on [`spacing_type`](Self::spacing_type): a `Multiple` value is a
/// 16.16 fixed-point multiplier of the font's natural line height (`0x0001_0000` = single, `0x0001_8000`
/// = one-and-a-half, `0x0002_0000` = double); an `Exact` value is a line pitch in twips. The raw stored
/// form is kept so the struct stays `Copy`/`Eq`; use [`multiple`](Self::multiple) / [`exact_twips`](Self::exact_twips)
/// to read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LineSpacing {
    /// Whether [`raw`](Self::raw) is a multiple of the natural line height or an exact twip pitch.
    pub spacing_type: LineSpacingType,
    /// The stored value: a 16.16 fixed-point multiplier when `Multiple`, or a twip pitch when `Exact`.
    pub raw: u32,
}

/// A 16.16 fixed-point `1.0` — single spacing, the engine default when no spacing is authored.
const LINE_SPACING_SINGLE: u32 = 0x0001_0000;

impl Default for LineSpacing {
    fn default() -> Self {
        LineSpacing {
            spacing_type: LineSpacingType::Multiple,
            raw: LINE_SPACING_SINGLE,
        }
    }
}

impl LineSpacing {
    /// The line-height multiplier for a `Multiple` spacing (`1.0`/`1.5`/`2.0`), or `None` when the
    /// spacing is `Exact`.
    pub fn multiple(&self) -> Option<f64> {
        matches!(self.spacing_type, LineSpacingType::Multiple)
            .then(|| self.raw as f64 / f64::from(LINE_SPACING_SINGLE))
    }

    /// The exact line pitch in twips for an `Exact` spacing, or `None` when the spacing is a
    /// `Multiple`.
    pub fn exact_twips(&self) -> Option<i32> {
        matches!(self.spacing_type, LineSpacingType::Exact).then_some(self.raw as i32)
    }
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
    /// formulas, `?name` for parameters.
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
    /// The heading text's reading order (left-to-right or right-to-left).
    pub reading_order: ReadingOrder,
}

/// SDK: `IDrawingObject` members shared by box and line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DrawingShape {
    /// The shape's right edge, in twips.
    pub right: Twips,
    /// The shape's bottom edge, in twips.
    pub bottom: Twips,
    /// The line style (solid / dashed / dotted / …). A **mirror** of the authoritative value in the
    /// object's [`Border`](crate::format::Border) (a line stores its style on the edge matching its
    /// orientation; a box's edges are uniform): the decoder redirects it from the border after decode,
    /// so read [`Border`] when both are available.
    pub line_style: LineStyle,
    /// The line thickness, in twips. Decoded directly from the border record (`0xec` byte 21); this
    /// is the one drawing-shape stroke field that is authoritative rather than a border mirror.
    pub line_thickness: Twips,
    /// The line colour. A **mirror** of [`Border::border_color`](crate::format::Border::border_color);
    /// the decoder redirects it from the border after decode. Read the border when both are available.
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
    /// The fill colour, or `None` for a transparent (unfilled) box. A **mirror** of
    /// [`Border::background_color`](crate::format::Border::background_color); the decoder redirects it
    /// from the border after decode, so a consumer that reads this field gets the real fill rather
    /// than a dead `None`. [`Border`](crate::format::Border) remains authoritative.
    pub fill_color: Option<Color>,
}

/// SDK: `IBlobFieldObject` — a database blob/image field shown as a picture. The object carries no
/// `DataSource`, but the bound field reference (`{table.field}`) is decoded and kept: it is the only
/// record of which column the picture comes from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BlobFieldObject {
    /// The bound database field reference, brace-wrapped (`{Command.some_field}`).
    pub data_source: String,
}

/// SDK: `IPictureObject`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PictureObject {
    /// The picture's kind (OLE object / bitmap / metafile / …).
    pub picture_type: PictureType,
    /// The embedded image bytes, verbatim from the OLE `Embedding N/CONTENTS` stream (the whole
    /// picture file — normally a full `BM` bitmap). Empty when the picture has
    /// no OLE embedding (e.g. a chart drawn *through* a picture object, or a blob-field picture).
    /// Use [`Self::image_format`] to identify the wire format of these bytes.
    pub data: Vec<u8>,
    /// The 1-based `Embedding N` storage ordinal this picture's [`Self::data`] was loaded from.
    /// `None` for pictures with no OLE embedding.
    pub ole_ordinal: Option<u32>,
    /// The "graphic location" formula that conditionally swaps the picture at runtime, when set.
    pub location_formula: Option<Formula>,
    /// The picture's natural width, in twips (SDK `OriginalWidth`). *Not stored in the report* — a
    /// derived value the engine recomputes at load from the embedded image's OLE extent. A producer
    /// fills it from the image bytes via [`natural_extent`](crate::natural_extent); `0` is the
    /// honest default when the natural size is unknown (no OLE embedding, or a format whose header
    /// that derivation does not parse — e.g. a metafile).
    pub original_width: Twips,
    /// The picture's natural height, in twips (SDK `OriginalHeight`). Derived like
    /// [`Self::original_width`].
    pub original_height: Twips,
    /// The horizontal scale factor the object is drawn at (SDK `XScaling`) — the placed width
    /// divided by [`Self::original_width`]. Derived alongside [`Self::original_width`]; `1.0` when
    /// the picture is drawn at natural size or the natural size is unknown.
    pub x_scaling: f64,
    /// The vertical scale factor the object is drawn at (SDK `YScaling`) — the placed height
    /// divided by [`Self::original_height`]. Derived alongside [`Self::original_height`]; `1.0` when
    /// the picture is drawn at natural size or the natural size is unknown.
    pub y_scaling: f64,
    /// Cropping applied to each edge of the source image (SDK `PictureFormat.*Cropping`), in twips.
    /// Unobserved — no stored picture has been seen cropped; latent for the renderer.
    pub crop_top: Twips,
    /// Bottom-edge cropping, in twips. See [`Self::crop_top`].
    pub crop_bottom: Twips,
    /// Left-edge cropping, in twips. See [`Self::crop_top`].
    pub crop_left: Twips,
    /// Right-edge cropping, in twips. See [`Self::crop_top`].
    pub crop_right: Twips,
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
    /// SDK `SubreportObject.IsImported` — the subreport was imported from an external `.rpt` file
    /// rather than authored inline. Resolved from the subreport's
    /// [`SubreportReimportInfo::source_path`](super::SubreportReimportInfo) (record `0x0142`): a
    /// non-empty source path means it was imported.
    pub is_imported: bool,
    /// SDK `SubreportObject.EnableReimport` — the "re-import subreport when opening" policy is on.
    /// Its stored source byte is unlocated — the designer's `reimport_when_opening` enum is a
    /// constant `1` everywhere, giving no positive example — so this is carried as a resolved fact
    /// but always reads `false`.
    pub enable_reimport: bool,
    /// SDK `SubreportObject.SubreportLinks` — the field/parameter links binding this placeholder's
    /// main-report fields into the subreport. A copy of the backing
    /// [`Subreport::links`](super::Subreport), resolved onto the placeholder object.
    pub links: Vec<super::SubreportLink>,
}

/// SDK: `IChartObject` — chart definition + style (deferred detail).
///
/// The chart's persistent field bindings, decoded from its binding block. Each is the raw engine
/// reference form (`Table.field` for a database field, `@name` for a formula), split by the axis it
/// binds to:
/// - `data_refs` — the chart's "show value" data bindings, the series the chart plots.
/// - `category_refs` — the chart's "on change of" category bindings. A DB category is built as an
///   internal group (condition + sort) per chart, which is why one category binding appears twice in
///   the record stream.
///
/// The render pipeline aggregates the dataset over these to build the chart's series (see
/// `rpt_layout::aggregate`).
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
/// references it **three times per dimension**.
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
    /// every column level). Raw — its bit meanings are unknown; every observed cross-tab stores
    /// `0x0000`, and the cross-tab dimension/group structure is exposed on no reader surface (RAS
    /// `cubeDefinition` comes back null).
    pub column_axis_options: u16,
    /// The row-axis option word (the `0x00d2` level record's 2-byte leaf, big-endian; shared by every
    /// row level). Raw — `0x0000` for a plain-field row group; the one observed non-zero value,
    /// `0x0003`, accompanies a row dimension that is a code+name *formula*. A single distinct data
    /// point, so the value stays unmapped. Likely lives in the group
    /// sort/condition option space the cross-tab axes reuse (see [`Self::column_axis_options`]).
    pub row_axis_options: u16,
    /// The cross-tab's grid **display options** and grand-total background colours — the RAS
    /// `ISCRCrossTabStyle` view. Decoded from the `0xb8`/`0xb9` cross-tab records and the grand-total
    /// `0x00cb` dimension levels (see [`CrossTabGridOptions`]). STRUCTURAL.
    pub options: CrossTabGridOptions,
}

/// SDK: `ISCRCrossTabStyle` — a cross-tab's grid display options and grand-total background colours.
///
/// The colour axes are **cross-wired** as the SDK exposes them: RAS `RowGrandTotalColor` is the
/// colour of the first *column*-axis grand-total level, and `ColumnGrandTotalColor` the first
/// *row*-axis level. STRUCTURAL (no output surface).
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
/// output surface; a stored-fact decode for the model and future rendering).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CrossTabGridFormat {
    /// The grid-level format word (`0x0143`, u16 big-endian). Invariant at `0x0014` (equal to the
    /// [`cells`](Self::cells) count); its individual bit meanings are unknown, so it is preserved raw.
    pub raw: u16,
    /// The RAS `CrossTabFormat.CrossTabStyle` view — the grid display flags and grand-total
    /// background colours in the shape RAS reflects them. It mirrors the object-level
    /// [`CrossTabObject::options`](CrossTabObject::options), with one difference: the grand-total
    /// colours here are the concrete engine `COLORREF` colours, so the engine's "auto" default
    /// (`0xFFFFFFFF`) surfaces as white rather than the `None` sentinel `options` carries. RAS nests
    /// this style block under the cross-tab's format object, so it is exposed here on the grid
    /// format.
    pub style: CrossTabGridOptions,
    /// Per grid-region cell formats (`0x0145`), one per fixed formattable region. The count is fixed
    /// by the cross-tab template (20), independent of the grid's actual row/column counts.
    pub cells: Vec<CrossTabCellFormat>,
}

/// One cross-tab grid-region cell format. Carries the region's background colour and its
/// format-override flags. STRUCTURAL.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CrossTabCellFormat {
    /// Format-override flags. `0` = the region uses the grid defaults; a non-zero value marks a
    /// region carrying explicit cell formatting.
    pub flags: u32,
    /// The region background colour, or `None` when unset. The grid-region template is
    /// engine-internal (not exposed by RAS or the HTML render).
    pub background_color: Option<Color>,
    /// The region's enabled/visible flag.
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
    /// The dimension's date/time grouping period — the interval each dimension bucket spans (weekly,
    /// monthly, …), decoded from the level's `0x00e5` grid group at the SDK `CrGroupConditionEnum`
    /// ordinal byte (`used + 3`), the same encoding a report group's
    /// [`Group::date_condition`](crate::Group::date_condition) uses. `None` for a discrete dimension
    /// (a text/number field, a `@formula`, or a daily date axis — ordinal `0` is ambiguous with
    /// "no period", so the raw value is used, which yields per-day buckets anyway). Drives the
    /// render-side pivot bucketing so a monthly date column collapses to one bucket per calendar
    /// month rather than one per distinct date. STRUCTURAL (not exported).
    pub period: Option<GroupCondition>,
    /// Whether this level's subtotal row/column is hidden — RAS
    /// `ICrossTabGroup.EnableSuppressSubtotal`. STRUCTURAL (not exported).
    pub suppress_subtotal: bool,
    /// Whether this level's group label is hidden — RAS `ICrossTabGroup.EnableSuppressLabel`.
    /// STRUCTURAL (not exported).
    pub suppress_label: bool,
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
                    ..Default::default()
                },
                Paragraph {
                    runs: vec![lit("world")],
                    ..Default::default()
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
                    ..Default::default()
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
                    ..Default::default()
                },
                Paragraph::default(),
            ],
            ..Default::default()
        };
        assert_eq!(t.flattened_text(), "a\n");
    }
}
