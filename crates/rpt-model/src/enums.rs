//! SDK enumerations.
//!
//! Every enum carries an `Other(i32)`/`Code(i32)` arm so unmapped engine codes round-trip
//! losslessly. Variant names follow the SDK constants.

macro_rules! sdk_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $variant:ident),+ $(,)? } $(, $other:ident)?) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub enum $name {
            #[default]
            $($(#[$vm])* $variant,)+
            $(
                /// An engine code with no named variant, preserved so the value round-trips losslessly.
                $other(i32),
            )?
        }
    };
}

sdk_enum!(
    /// SDK: `AreaSectionKind`.
    AreaSectionKind {
        /// The report header area (printed once, before all data).
        ReportHeader,
        /// The page header area (printed at the top of each page).
        PageHeader,
        /// A group header area (printed once at the start of each group).
        GroupHeader,
        /// The details area (printed once per record).
        Detail,
        /// A group footer area (printed once at the end of each group).
        GroupFooter,
        /// The report footer area (printed once, after all data).
        ReportFooter,
        /// The page footer area (printed at the bottom of each page).
        PageFooter,
    }, Other);

sdk_enum!(
    /// SDK: `LineStyle` (`CrystalDecisions.Shared.LineStyle`).
    LineStyle {
        /// No line drawn.
        NoLine,
        /// A single solid line.
        SingleLine,
        /// A double solid line.
        DoubleLine,
        /// A dashed line.
        DashLine,
        /// A dotted line.
        DotLine,
        /// SDK sentinel marking the first invalid style value.
        FirstInvalidLineStyle,
        /// A blank (invisible) line that still reserves space.
        BlankLine,
    }, Other);

sdk_enum!(
    /// SDK: `Alignment` (`CrystalDecisions.Shared.Alignment`; horizontal + vertical members).
    Alignment {
        /// Default alignment (engine chooses per field type).
        DefaultAlign,
        /// Horizontally left-aligned.
        LeftAlign,
        /// Horizontally centered.
        HorizontalCenterAlign,
        /// Horizontally right-aligned.
        RightAlign,
        /// Justified (both edges flush).
        Justified,
        /// Aligned on the decimal point.
        Decimal,
        /// Vertically top-aligned.
        TopAlign,
        /// Vertically centered.
        VerticalCenterAlign,
        /// Vertically bottom-aligned.
        BottomAlign,
    }, Other);

sdk_enum!(
    /// SDK: `FieldValueType` — a field's data type.
    FieldValueType {
        /// Unknown / unmapped value type.
        Unknown,
        /// Signed 8-bit integer.
        Int8s,
        /// Signed 16-bit integer.
        Int16s,
        /// Signed 32-bit integer.
        Int32s,
        /// Unsigned 32-bit integer.
        Int32u,
        /// Floating-point number.
        Number,
        /// Currency (fixed-scale decimal) value.
        Currency,
        /// Boolean value.
        Boolean,
        /// String value.
        String,
        /// Date value.
        Date,
        /// Time-of-day value.
        Time,
        /// Combined date-and-time value.
        DateTime,
        /// Binary large object.
        Blob,
        /// Persistent (stored) memo text.
        PersistentMemo,
    }, Other);

sdk_enum!(
    /// Crystal formula-language variable scope (`FLScope`): the declared reach of a `Global`/`Shared`
    /// variable declared in a formula and persisted with the report. `Local` variables are not
    /// persisted (the engine asserts against writing them), so only `Shared`/`Global` appear in files.
    FormulaVariableScope {
        /// `Shared` scope — visible across the main report and its subreports.
        Shared,
        /// `Global` scope — visible throughout one report (the default).
        Global,
        /// `Local` scope — confined to one formula (not persisted).
        Local,
    }, Other);

sdk_enum!(
    /// SDK: `SortDirection`. `TopNOrder`/`BottomNOrder` are group Top N / Bottom N sort directions.
    SortDirection {
        /// Ascending sort.
        AscendingOrder,
        /// Descending sort.
        DescendingOrder,
        /// No sort applied (original order).
        NoSortOrder,
        /// Group Top N ordering.
        TopNOrder,
        /// Group Bottom N ordering.
        BottomNOrder,
    }, Other);

sdk_enum!(
/// XML `@SortType` — which collection a sort came from.
SortKind {
    /// A record-level sort field.
    RecordSortField,
    /// A group-level sort field.
    GroupSortField,
});

sdk_enum!(
    /// SDK: `SummaryOperation` (`CrystalDecisions.Shared.SummaryOperation`, full table).
    SummaryOperation {
        /// Sum of values.
        Sum,
        /// Arithmetic mean.
        Average,
        /// Sample variance.
        SampleVariance,
        /// Sample standard deviation.
        SampleStandardDeviation,
        /// Largest value.
        Maximum,
        /// Smallest value.
        Minimum,
        /// Count of values.
        Count,
        /// Population variance.
        PopVariance,
        /// Population standard deviation.
        PopStandardDeviation,
        /// Count of distinct values.
        DistinctCount,
        /// Correlation between two fields.
        Correlation,
        /// Covariance between two fields.
        Covariance,
        /// Weighted average.
        WeightedAvg,
        /// Median value.
        Median,
        /// Value at a given percentile.
        Percentile,
        /// Nth-largest value.
        NthLargest,
        /// Nth-smallest value.
        NthSmallest,
        /// Most frequently occurring value.
        Mode,
        /// Nth most frequently occurring value.
        NthMostFrequent,
    }, Other);

sdk_enum!(
    /// SDK: `EvaluationConditionType`.
    EvaluationConditionType {
        /// No evaluation condition.
        NoCondition,
        /// Evaluate on a boolean formula.
        OnFormula,
        /// Evaluate on change of a field's value.
        OnChangeOfField,
        /// Evaluate on change of a group.
        OnChangeOfGroup,
    }, Other);

sdk_enum!(
    /// SDK: `ResetConditionType`.
    ResetConditionType {
        /// Never reset.
        NoCondition,
        /// Reset on change of a field's value.
        OnChangeOfField,
        /// Reset on change of a group.
        OnChangeOfGroup,
        /// Reset when a boolean formula becomes true.
        OnFormula,
    }, Other);

sdk_enum!(
    /// SDK: `TableJoinType`.
    TableJoinType {
        /// Equi-join (`=`).
        Equal,
        /// Left outer join.
        LeftOuter,
        /// Right outer join.
        RightOuter,
        /// Not-equal join (`<>`).
        NotEqual,
        /// Greater-than join (`>`).
        GreaterThan,
        /// Less-than join (`<`).
        LessThan,
    }, Other);

sdk_enum!(
    /// SDK: `ParameterFieldType`.
    ParameterType {
        /// A report parameter prompted at refresh.
        ReportParameter,
        /// A stored-procedure input parameter.
        StoreProcedureParameter,
    }, Other);

sdk_enum!(
    /// SDK: `ParameterValueRangeKind`.
    ParameterValueKind {
        /// String-valued parameter.
        StringParameter,
        /// Number-valued parameter.
        NumberParameter,
        /// Currency-valued parameter.
        CurrencyParameter,
        /// Boolean-valued parameter.
        BooleanParameter,
        /// Date-valued parameter.
        DateParameter,
        /// Time-valued parameter.
        TimeParameter,
        /// DateTime-valued parameter.
        DateTimeParameter,
    }, Other);

sdk_enum!(
    /// SDK: `RoundingType` (`CrystalDecisions.Shared.RoundingType`). The stored rounding also encodes
    /// the decimal-place count as `11 - places` (RoundToUnit = 0 places = code 11, RoundToHundredth = 2
    /// places = code 9); see [`RoundingFormat::from_code`].
    RoundingFormat {
        /// Round to 2 decimal places (0.01).
        RoundToHundredth,
        /// Round to a whole unit (0 decimal places).
        RoundToUnit,
        /// Round to 1 decimal place (0.1).
        RoundToTenth,
        /// Round to 3 decimal places (0.001).
        RoundToThousandth,
        /// Round to 4 decimal places (0.0001).
        RoundToTenThousandth,
        /// Round to 5 decimal places (0.00001).
        RoundToHundredThousandth,
        /// Round to 6 decimal places (0.000001).
        RoundToMillionth,
        /// Round to the nearest ten.
        RoundToTen,
        /// Round to the nearest hundred.
        RoundToHundred,
        /// Round to the nearest thousand.
        RoundToThousand,
    }, Other);

sdk_enum!(
    /// SDK: `NegativeType` (`CrystalDecisions.Shared.NegativeType`). Byte value = ordinal.
    NegativeFormat {
        /// Value is not negative (no negative styling).
        NotNegative,
        /// Leading minus sign (`-123`).
        LeadingMinus,
        /// Trailing minus sign (`123-`).
        TrailingMinus,
        /// Parenthesized (`(123)`).
        Bracketed,
    }, Other);

sdk_enum!(
    /// SDK: `CurrencySymbolType` (`CrystalDecisions.Shared.CurrencySymbolType`). Byte value = ordinal.
    CurrencySymbolFormat {
        /// No currency symbol shown.
        NoSymbol,
        /// Fixed symbol pinned to the field edge.
        FixedSymbol,
        /// Floating symbol adjacent to the first significant digit.
        FloatingSymbol,
    }, Other);

sdk_enum!(
    /// SDK: `BooleanOutputType` (`CrystalDecisions.Shared.BooleanOutputType`). Byte value = ordinal.
    BooleanOutputType {
        /// `True` / `False`.
        TrueOrFalse,
        /// `T` / `F`.
        TOrF,
        /// `Yes` / `No`.
        YesOrNo,
        /// `Y` / `N`.
        YOrN,
        /// `1` / `0`.
        OneOrZero,
    }, Other);

sdk_enum!(
    /// SDK: `DayFormat` (`<DateFieldFormat DayFormat>`). Native `RDDayType`.
    DayFormat {
        /// Numeric day without a leading zero (`5`).
        NumericDay,
        /// Numeric day with a leading zero (`05`).
        LeadingZeroNumericDay,
        /// Day not shown.
        NoDay,
    }, Other);

sdk_enum!(
    /// SDK: `MonthFormat` (`<DateFieldFormat MonthFormat>`). Native `RDMonthType`.
    MonthFormat {
        /// Numeric month without a leading zero (`3`).
        NumericMonth,
        /// Numeric month with a leading zero (`03`).
        LeadingZeroNumericMonth,
        /// Abbreviated month name (`Mar`).
        ShortMonth,
        /// Full month name (`March`).
        LongMonth,
        /// Month not shown.
        NoMonth,
    }, Other);

sdk_enum!(
    /// SDK: `YearFormat` (`<DateFieldFormat YearFormat>`). Native `RDYearType`.
    YearFormat {
        /// Two-digit year (`24`).
        ShortYear,
        /// Four-digit year (`2024`).
        LongYear,
        /// Year not shown.
        NoYear,
    }, Other);

sdk_enum!(
    /// SDK: `DateSystemDefaultType` (`DateFieldFormat.SystemDefaultType`). Native
    /// `RDDateWindowsDefaultType`. When not `NotUsingWindowsDefaults`, the engine renders the date with
    /// the host's Windows long/short date pattern instead of the field's stored day/month/year enums.
    DateSystemDefaultType {
        /// Render with the host's Windows long-date pattern.
        UseWindowsLongDate,
        /// Render with the host's Windows short-date pattern.
        UseWindowsShortDate,
        /// Use the field's stored day/month/year format instead of a Windows default.
        NotUsingWindowsDefaults,
    }, Other);

sdk_enum!(
    /// SDK: `DayOfWeekType` (`DateFieldFormat.DayOfWeekType`). Native `RDDayOfWeekType`:
    /// 0 = `ShortDayOfWeek`, 1 = `LongDayOfWeek`, 2 = `NoDayOfWeek` (the only corpus value — no weekday
    /// shown). Not exported, so decoded for record completeness only.
    DayOfWeekFormat {
        /// Abbreviated weekday name (`Wed`).
        ShortDayOfWeek,
        /// Full weekday name (`Wednesday`).
        LongDayOfWeek,
        /// Weekday not shown.
        NoDayOfWeek,
    }, Other);

impl DayFormat {
    /// Decode the `dayType` byte (SDK ordinal).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::NumericDay,
            1 => Self::LeadingZeroNumericDay,
            2 => Self::NoDay,
            other => Self::Other(other),
        }
    }
}

impl MonthFormat {
    /// Decode the `monthType` byte (SDK ordinal).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::NumericMonth,
            1 => Self::LeadingZeroNumericMonth,
            2 => Self::ShortMonth,
            3 => Self::LongMonth,
            4 => Self::NoMonth,
            other => Self::Other(other),
        }
    }
}

impl YearFormat {
    /// Decode the `yearType` byte (SDK ordinal).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::ShortYear,
            1 => Self::LongYear,
            2 => Self::NoYear,
            other => Self::Other(other),
        }
    }
}

impl DateSystemDefaultType {
    /// Decode the `windowsDefaultType` byte (SDK ordinal).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::UseWindowsLongDate,
            1 => Self::UseWindowsShortDate,
            2 => Self::NotUsingWindowsDefaults,
            other => Self::Other(other),
        }
    }
}

impl DayOfWeekFormat {
    /// Decode the `dayOfWeekType` byte (SDK ordinal).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::ShortDayOfWeek,
            1 => Self::LongDayOfWeek,
            2 => Self::NoDayOfWeek,
            other => Self::Other(other),
        }
    }
}

impl RoundingFormat {
    /// Decode the rounding byte. The engine stores `11 - decimalPlaces`,
    /// so code 11 = round to unit (0 dp), 9 = round to hundredth (2 dp), 12 = round to ten, etc.
    pub fn from_code(code: i32) -> Self {
        match code {
            11 => Self::RoundToUnit,
            10 => Self::RoundToTenth,
            9 => Self::RoundToHundredth,
            8 => Self::RoundToThousandth,
            7 => Self::RoundToTenThousandth,
            6 => Self::RoundToHundredThousandth,
            5 => Self::RoundToMillionth,
            12 => Self::RoundToTen,
            13 => Self::RoundToHundred,
            14 => Self::RoundToThousand,
            other => Self::Other(other),
        }
    }
}

impl NegativeFormat {
    /// Decode the negative byte (SDK ordinal).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::NotNegative,
            1 => Self::LeadingMinus,
            2 => Self::TrailingMinus,
            3 => Self::Bracketed,
            other => Self::Other(other),
        }
    }
}

impl CurrencySymbolFormat {
    /// Decode the currency-symbol byte (SDK ordinal).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::NoSymbol,
            1 => Self::FixedSymbol,
            2 => Self::FloatingSymbol,
            other => Self::Other(other),
        }
    }
}

impl BooleanOutputType {
    /// Decode the boolean-output byte (SDK ordinal).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::TrueOrFalse,
            1 => Self::TOrF,
            2 => Self::YesOrNo,
            3 => Self::YOrN,
            4 => Self::OneOrZero,
            other => Self::Other(other),
        }
    }
}

sdk_enum!(
    /// SDK: `PaperOrientation`.
    PaperOrientation {
        /// Printer/driver default orientation.
        DefaultPaperOrientation,
        /// Portrait (tall) orientation.
        Portrait,
        /// Landscape (wide) orientation.
        Landscape,
    }, Other);

sdk_enum!(
    /// SDK: `PrinterDuplex`.
    PrinterDuplex {
        /// Printer default duplex setting.
        Default,
        /// Single-sided printing.
        Simplex,
        /// Double-sided, flipped on the long (vertical) edge.
        Vertical,
        /// Double-sided, flipped on the short (horizontal) edge.
        Horizontal,
    }, Other);

sdk_enum!(
    /// SDK: `HyperlinkType`.
    HyperlinkType {
        /// No hyperlink.
        NoHyperlink,
        /// A `mailto:` e-mail address.
        AnEMailAddress,
        /// A file path or web URL.
        AFileOrWebSite,
        /// A website built from the current field's value.
        CurrentWebsiteField,
        /// A drill-down into a report part.
        ReportPartDrilldown,
        /// A link to another report object.
        AnotherReportObject,
    }, Other);

sdk_enum!(
    /// SDK: `TextRotationAngle`.
    TextRotationAngle {
        /// No rotation.
        Rotate0,
        /// Rotated 90° counter-clockwise.
        Rotate90,
        /// Rotated 270° counter-clockwise.
        Rotate270,
    }, Other);

sdk_enum!(
    /// SDK: `ReadingOrder`.
    ReadingOrder {
        /// Left-to-right reading order.
        LeftToRight,
        /// Right-to-left reading order.
        RightToLeft,
    }, Other);

sdk_enum!(
    /// SDK: `PictureType`.
    PictureType {
        /// A raster bitmap.
        Bitmap,
        /// A Windows metafile.
        Metafile,
        /// An OLE-embedded picture.
        Ole,
        /// A picture of another kind.
        Other_,
    }, Other);

/// The concrete media/container format of an embedded picture's bytes, sniffed from the leading
/// magic of [`PictureObject::data`](crate::PictureObject::data).
///
/// This is the *wire* image format of the bytes stored in the report (the OLE `Embedding N/CONTENTS`
/// stream), not the coarse SDK [`PictureType`]. The native engine supports importing this whole set
/// (its extension/MIME table lists `jpg gif tif pct pic iff dib tga pcx png jpeg tiff bmp`, with
/// dedicated loaders for DIB/BMP via `CSDIB`, TIFF via `TIFFGraph`, and Windows/Enhanced metafiles);
/// which of them actually appear *embedded* depends on the designer version (older builds transcode
/// raster imports to a DIB/BMP; the entire public corpus is `Bmp`). Detected by magic so the
/// renderer can pick the right data-URI MIME type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ImageFormat {
    /// Windows Bitmap — a full `BM` file (14-byte `BITMAPFILEHEADER` + DIB). The corpus default.
    Bmp,
    /// A bare device-independent bitmap (`BITMAPINFOHEADER`/`BITMAPCOREHEADER`, no `BM` file header).
    /// A valid `.bmp` needs a reconstructed 14-byte file header prepended (see
    /// [`PictureObject::to_bmp`](crate::PictureObject::to_bmp)).
    Dib,
    /// Portable Network Graphics (`89 50 4E 47`).
    Png,
    /// JPEG / JFIF (`FF D8 FF`).
    Jpeg,
    /// GIF (`GIF87a` / `GIF89a`).
    Gif,
    /// Tagged Image File Format (`II*\0` little-endian or `MM\0*` big-endian).
    Tiff,
    /// Truevision TGA / Targa.
    Tga,
    /// ZSoft PCX.
    Pcx,
    /// Apple QuickDraw PICT.
    Pict,
    /// Windows Metafile (placeable `D7 CD C6 9A` or bare `WMF`).
    Wmf,
    /// Windows Enhanced Metafile (`EMF`, record type `0x00000001` + signature ` EMF`).
    Emf,
    /// Unrecognised / empty payload.
    #[default]
    Unknown,
}

impl ImageFormat {
    /// Classify a picture payload by its leading magic bytes.
    pub fn sniff(data: &[u8]) -> ImageFormat {
        match data {
            [0x42, 0x4d, ..] => ImageFormat::Bmp,
            [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, ..] => ImageFormat::Png,
            [0xff, 0xd8, 0xff, ..] => ImageFormat::Jpeg,
            [0x47, 0x49, 0x46, 0x38, ..] => ImageFormat::Gif, // "GIF8"
            [0x49, 0x49, 0x2a, 0x00, ..] | [0x4d, 0x4d, 0x00, 0x2a, ..] => ImageFormat::Tiff,
            [0x0a, ..] => ImageFormat::Pcx, // ZSoft manufacturer byte
            [0xd7, 0xcd, 0xc6, 0x9a, ..] => ImageFormat::Wmf, // placeable WMF (APM header)
            // Enhanced metafile: EMR_HEADER record type 1, then the ` EMF` signature at offset 40.
            [0x01, 0x00, 0x00, 0x00, ..] if data.len() >= 44 && &data[40..44] == b" EMF" => {
                ImageFormat::Emf
            }
            // Bare DIB: BITMAPINFOHEADER (40) or BITMAPCOREHEADER (12) little-endian header size.
            [0x28, 0x00, 0x00, 0x00, ..] | [0x0c, 0x00, 0x00, 0x00, ..] => ImageFormat::Dib,
            _ => ImageFormat::Unknown,
        }
    }

    /// The IANA media (MIME) type, suitable for a `data:` URI. Metafiles and unknown payloads have
    /// no registered image MIME type and fall back to `application/octet-stream`.
    pub fn mime_type(self) -> &'static str {
        match self {
            ImageFormat::Bmp | ImageFormat::Dib => "image/bmp",
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Gif => "image/gif",
            ImageFormat::Tiff => "image/tiff",
            ImageFormat::Tga => "image/x-tga",
            ImageFormat::Pcx => "image/x-pcx",
            ImageFormat::Pict => "image/x-pict",
            ImageFormat::Wmf => "image/wmf",
            ImageFormat::Emf => "image/emf",
            ImageFormat::Unknown => "application/octet-stream",
        }
    }
}

sdk_enum!(
    /// SDK: `ConnectionInfoKind`.
    ConnectionInfoKind {
        /// Unknown / unspecified connection kind.
        Unknown,
        /// Crystal Reports Query Engine connection.
        CRQE,
        /// A SQL database connection.
        SQL,
        /// A file-based data source.
        File,
        /// A stored-procedure data source.
        StoreProcedure,
    }, Other);

sdk_enum!(
    /// SDK: `SpecialVarType` (special-field kind).
    SpecialFieldType {
        /// The current record number.
        RecordNumber,
        /// The current page number.
        PageNumber,
        /// The current group number.
        GroupNumber,
        /// The total page count of the report.
        TotalPageCount,
        /// The date the report was printed/generated.
        PrintDate,
        /// The time the report was printed/generated.
        PrintTime,
        /// The report's last modification date.
        ModificationDate,
        /// The report's last modification time.
        ModificationTime,
        /// The date the report's data was read.
        DataDate,
        /// The time the report's data was read.
        DataTime,
        /// The record-selection formula.
        RecordSelection,
        /// The group-selection formula.
        GroupSelection,
        /// The report title (summary info).
        ReportTitle,
        /// The report comments (summary info).
        ReportComments,
        /// The report file's author.
        FileAuthor,
        /// The report file's path.
        FilePath,
        /// The report file's creation date.
        FileCreationDate,
    }, Other);

/// SDK: `PaperSize` (`CrystalDecisions.Shared.PaperSize`). The enum's integer values equal the
/// Windows `DMPAPER_*` codes stored in the record, so the code maps straight onto the variant.
/// Driver-specific sizes outside the SDK table (code > 41) keep their raw code in
/// [`PaperSize::Code`] and surface as the bare integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PaperSize {
    /// Printer/driver default paper size.
    #[default]
    DefaultPaperSize,
    /// US Letter (8.5 × 11 in).
    PaperLetter,
    /// US Letter Small (8.5 × 11 in).
    PaperLetterSmall,
    /// Tabloid (11 × 17 in).
    PaperTabloid,
    /// Ledger (17 × 11 in).
    PaperLedger,
    /// US Legal (8.5 × 14 in).
    PaperLegal,
    /// Statement (5.5 × 8.5 in).
    PaperStatement,
    /// Executive (7.25 × 10.5 in).
    PaperExecutive,
    /// ISO A3 (297 × 420 mm).
    PaperA3,
    /// ISO A4 (210 × 297 mm).
    PaperA4,
    /// ISO A4 Small (210 × 297 mm).
    PaperA4Small,
    /// ISO A5 (148 × 210 mm).
    PaperA5,
    /// JIS B4 (250 × 354 mm).
    PaperB4,
    /// JIS B5 (182 × 257 mm).
    PaperB5,
    /// Folio (8.5 × 13 in).
    PaperFolio,
    /// Quarto (215 × 275 mm).
    PaperQuarto,
    /// 10 × 14 in.
    Paper10x14,
    /// 11 × 17 in.
    Paper11x17,
    /// Note (8.5 × 11 in).
    PaperNote,
    /// #9 Envelope (3.875 × 8.875 in).
    PaperEnvelope9,
    /// #10 Envelope (4.125 × 9.5 in).
    PaperEnvelope10,
    /// #11 Envelope (4.5 × 10.375 in).
    PaperEnvelope11,
    /// #12 Envelope (4.75 × 11 in).
    PaperEnvelope12,
    /// #14 Envelope (5 × 11.5 in).
    PaperEnvelope14,
    /// C-size sheet (17 × 22 in).
    PaperCsheet,
    /// D-size sheet (22 × 34 in).
    PaperDsheet,
    /// E-size sheet (34 × 44 in).
    PaperEsheet,
    /// DL Envelope (110 × 220 mm).
    PaperEnvelopeDL,
    /// C5 Envelope (162 × 229 mm).
    PaperEnvelopeC5,
    /// C3 Envelope (324 × 458 mm).
    PaperEnvelopeC3,
    /// C4 Envelope (229 × 324 mm).
    PaperEnvelopeC4,
    /// C6 Envelope (114 × 162 mm).
    PaperEnvelopeC6,
    /// C65 Envelope (114 × 229 mm).
    PaperEnvelopeC65,
    /// B4 Envelope (250 × 353 mm).
    PaperEnvelopeB4,
    /// B5 Envelope (176 × 250 mm).
    PaperEnvelopeB5,
    /// B6 Envelope (176 × 125 mm).
    PaperEnvelopeB6,
    /// Italy Envelope (110 × 230 mm).
    PaperEnvelopeItaly,
    /// Monarch Envelope (3.875 × 7.5 in).
    PaperEnvelopeMonarch,
    /// Personal (6.75) Envelope (3.625 × 6.5 in).
    PaperEnvelopePersonal,
    /// US Standard Fanfold (14.875 × 11 in).
    PaperFanfoldUS,
    /// German Standard Fanfold (8.5 × 12 in).
    PaperFanfoldStdGerman,
    /// German Legal Fanfold (8.5 × 13 in).
    PaperFanfoldLegalGerman,
    /// A driver-specific paper size outside the SDK table, keyed by its raw `DMPAPER_*` code.
    Code(i32),
}

impl PaperSize {
    /// The standard cut-sheet dimensions of this paper size as `(short, long)` edges in twips, or
    /// `None` for sizes without a fixed sheet rectangle (custom/default, envelopes, fanfold). Used to
    /// recognise a stored page rectangle as a *standard* sheet so its width/height can be oriented
    /// to the report's `PaperOrientation` (the rect is stored in either order).
    pub fn std_dims(self) -> Option<(i32, i32)> {
        // 1 inch = 1440 twips; metric sizes rounded to the nearest twip (mm × 1440 / 25.4).
        let dims = match self {
            PaperSize::PaperLetter | PaperSize::PaperLetterSmall | PaperSize::PaperNote => {
                (12240, 15840)
            }
            PaperSize::PaperTabloid | PaperSize::PaperLedger | PaperSize::Paper11x17 => {
                (15840, 24480)
            }
            PaperSize::PaperLegal => (12240, 20160),
            PaperSize::PaperStatement => (7920, 12240),
            PaperSize::PaperExecutive => (10440, 15120),
            PaperSize::PaperA3 => (16838, 23811),
            PaperSize::PaperA4 | PaperSize::PaperA4Small => (11906, 16838),
            PaperSize::PaperA5 => (8391, 11906),
            PaperSize::PaperB4 => (14173, 20069),
            PaperSize::PaperB5 => (10319, 14571),
            PaperSize::PaperFolio => (12240, 18720),
            PaperSize::PaperQuarto => (12701, 15309),
            PaperSize::Paper10x14 => (14400, 20160),
            _ => return None,
        };
        Some(dims)
    }
}

/// SDK: `PaperSource` (`CrystalDecisions.Shared.PaperSource`). The enum's integer values equal the
/// Windows `DMBIN_*` codes stored in the record. Codes outside the SDK table keep their raw value
/// in [`PaperSource::Code`] and surface as the bare integer. The model default is `Auto` (the SDK
/// enum has no zero member, so an absent paper source surfaces as `Auto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PaperSource {
    /// Automatically select the source bin (the model default).
    #[default]
    Auto,
    /// Upper / primary bin.
    Upper,
    /// Lower bin.
    Lower,
    /// Middle bin.
    Middle,
    /// Manual feed.
    Manual,
    /// Envelope feeder.
    Envelope,
    /// Manual envelope feed.
    EnvManual,
    /// Continuous-form tractor feed.
    Tractor,
    /// Small-format bin.
    SmallFmt,
    /// Large-format bin.
    LargeFmt,
    /// Large-capacity bin.
    LargeCapacity,
    /// Cassette bin.
    Cassette,
    /// Form-source bin.
    FormSource,
    /// A driver-specific source outside the SDK table, keyed by its raw `DMBIN_*` code.
    Code(i32),
}

sdk_enum!(
    /// SDK: `CrParameterDefaultValueDisplayTypeEnum` (XML `@DefaultValueDisplayType`) — how the
    /// parameter's default-value pick list is displayed: `Description`, otherwise `DescriptionAndValue`
    /// (the engine default).
    ParameterDisplayType {
        /// Show each pick-list entry's description and value (the engine default).
        DescriptionAndValue,
        /// Show each pick-list entry's description only.
        Description,
        /// Show each pick-list entry's value only.
        Value,
    }, Other);

sdk_enum!(
    /// SDK: `@DefaultValueSortOrder` — sort applied to the parameter's default-value pick list:
    /// `AlphabeticalAscending`, otherwise `NoSort`.
    ParameterSortOrder {
        /// No sort applied to the pick list.
        NoSort,
        /// Sort the pick list alphabetically ascending.
        AlphabeticalAscending,
        /// Sort the pick list alphabetically descending.
        AlphabeticalDescending,
    }, Other);

sdk_enum!(
    /// SDK: `CrDiscreteOrRangeKindEnum` (XML `@DiscreteOrRangeKind`) — whether a parameter accepts
    /// discrete values, a range value, or both. RptToXml emits this attribute; every observed corpus
    /// report is `DiscreteValue`.
    DiscreteOrRangeKind {
        /// Accepts discrete values only.
        DiscreteValue,
        /// Accepts a range value only.
        RangeValue,
        /// Accepts both discrete and range values.
        DiscreteAndRangeValue,
    }, Other);

sdk_enum!(
    /// SDK: `RangeBoundType` (`CrystalDecisions.Shared`) — the inclusivity of one end of a range
    /// parameter value. `NoBound` = that end is open (unbounded); `BoundInclusive`/`BoundExclusive`
    /// = the bound value is included / excluded.
    RangeBoundType {
        /// The end is open (unbounded).
        NoBound,
        /// The bound value is included.
        BoundInclusive,
        /// The bound value is excluded.
        BoundExclusive,
    }, Other);

sdk_enum!(
    /// The kind of database object a dynamic (list-of-values) parameter's pick list is sourced from.
    /// STRUCTURAL: not exposed by the SDK; modeled so a dynamic LOV binding can be
    /// represented if a future reader decodes it.
    LovSourceKind {
        /// Sourced from a database table.
        Table,
        /// Sourced from a database view.
        View,
        /// Sourced from a stored procedure.
        StoredProcedure,
        /// Sourced from a Crystal SQL command.
        Command,
    }, Other);

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `from_code` mapping: known code→variant pairs plus the out-of-range fallback. Catches
    /// enum-ordinal drift without fixtures.
    #[test]
    fn day_format_from_code() {
        assert_eq!(DayFormat::from_code(0), DayFormat::NumericDay);
        assert_eq!(DayFormat::from_code(1), DayFormat::LeadingZeroNumericDay);
        assert_eq!(DayFormat::from_code(2), DayFormat::NoDay);
        assert_eq!(DayFormat::from_code(99), DayFormat::Other(99));
        assert_eq!(DayFormat::from_code(-1), DayFormat::Other(-1));
    }

    #[test]
    fn month_format_from_code() {
        assert_eq!(MonthFormat::from_code(0), MonthFormat::NumericMonth);
        assert_eq!(
            MonthFormat::from_code(1),
            MonthFormat::LeadingZeroNumericMonth
        );
        assert_eq!(MonthFormat::from_code(2), MonthFormat::ShortMonth);
        assert_eq!(MonthFormat::from_code(3), MonthFormat::LongMonth);
        assert_eq!(MonthFormat::from_code(4), MonthFormat::NoMonth);
        assert_eq!(MonthFormat::from_code(5), MonthFormat::Other(5));
    }

    #[test]
    fn year_format_from_code() {
        assert_eq!(YearFormat::from_code(0), YearFormat::ShortYear);
        assert_eq!(YearFormat::from_code(1), YearFormat::LongYear);
        assert_eq!(YearFormat::from_code(2), YearFormat::NoYear);
        assert_eq!(YearFormat::from_code(3), YearFormat::Other(3));
    }

    #[test]
    fn date_system_default_type_from_code() {
        assert_eq!(
            DateSystemDefaultType::from_code(0),
            DateSystemDefaultType::UseWindowsLongDate
        );
        assert_eq!(
            DateSystemDefaultType::from_code(1),
            DateSystemDefaultType::UseWindowsShortDate
        );
        assert_eq!(
            DateSystemDefaultType::from_code(2),
            DateSystemDefaultType::NotUsingWindowsDefaults
        );
        assert_eq!(
            DateSystemDefaultType::from_code(7),
            DateSystemDefaultType::Other(7)
        );
    }

    #[test]
    fn day_of_week_format_from_code() {
        assert_eq!(
            DayOfWeekFormat::from_code(0),
            DayOfWeekFormat::ShortDayOfWeek
        );
        assert_eq!(
            DayOfWeekFormat::from_code(1),
            DayOfWeekFormat::LongDayOfWeek
        );
        assert_eq!(DayOfWeekFormat::from_code(2), DayOfWeekFormat::NoDayOfWeek);
        assert_eq!(DayOfWeekFormat::from_code(9), DayOfWeekFormat::Other(9));
    }

    /// RoundingFormat stores `11 - decimalPlaces`, so the mapping is deliberately non-contiguous and
    /// worth pinning: 11=unit(0dp) up through 5=millionth, plus 12..=14 for the tens/hundreds/thousands.
    #[test]
    fn rounding_format_from_code() {
        assert_eq!(RoundingFormat::from_code(11), RoundingFormat::RoundToUnit);
        assert_eq!(RoundingFormat::from_code(10), RoundingFormat::RoundToTenth);
        assert_eq!(
            RoundingFormat::from_code(9),
            RoundingFormat::RoundToHundredth
        );
        assert_eq!(
            RoundingFormat::from_code(8),
            RoundingFormat::RoundToThousandth
        );
        assert_eq!(
            RoundingFormat::from_code(7),
            RoundingFormat::RoundToTenThousandth
        );
        assert_eq!(
            RoundingFormat::from_code(6),
            RoundingFormat::RoundToHundredThousandth
        );
        assert_eq!(
            RoundingFormat::from_code(5),
            RoundingFormat::RoundToMillionth
        );
        assert_eq!(RoundingFormat::from_code(12), RoundingFormat::RoundToTen);
        assert_eq!(
            RoundingFormat::from_code(13),
            RoundingFormat::RoundToHundred
        );
        assert_eq!(
            RoundingFormat::from_code(14),
            RoundingFormat::RoundToThousand
        );
        assert_eq!(RoundingFormat::from_code(0), RoundingFormat::Other(0));
    }

    #[test]
    fn negative_format_from_code() {
        assert_eq!(NegativeFormat::from_code(0), NegativeFormat::NotNegative);
        assert_eq!(NegativeFormat::from_code(1), NegativeFormat::LeadingMinus);
        assert_eq!(NegativeFormat::from_code(2), NegativeFormat::TrailingMinus);
        assert_eq!(NegativeFormat::from_code(3), NegativeFormat::Bracketed);
        assert_eq!(NegativeFormat::from_code(4), NegativeFormat::Other(4));
    }

    #[test]
    fn currency_symbol_format_from_code() {
        assert_eq!(
            CurrencySymbolFormat::from_code(0),
            CurrencySymbolFormat::NoSymbol
        );
        assert_eq!(
            CurrencySymbolFormat::from_code(1),
            CurrencySymbolFormat::FixedSymbol
        );
        assert_eq!(
            CurrencySymbolFormat::from_code(2),
            CurrencySymbolFormat::FloatingSymbol
        );
        assert_eq!(
            CurrencySymbolFormat::from_code(3),
            CurrencySymbolFormat::Other(3)
        );
    }

    #[test]
    fn boolean_output_type_from_code() {
        assert_eq!(
            BooleanOutputType::from_code(0),
            BooleanOutputType::TrueOrFalse
        );
        assert_eq!(BooleanOutputType::from_code(1), BooleanOutputType::TOrF);
        assert_eq!(BooleanOutputType::from_code(2), BooleanOutputType::YesOrNo);
        assert_eq!(BooleanOutputType::from_code(3), BooleanOutputType::YOrN);
        assert_eq!(
            BooleanOutputType::from_code(4),
            BooleanOutputType::OneOrZero
        );
        assert_eq!(BooleanOutputType::from_code(5), BooleanOutputType::Other(5));
    }
}
