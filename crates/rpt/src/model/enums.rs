//! SDK enumerations.
//!
//! Every enum carries an `Other(i32)`/`Code(i32)` arm so unmapped engine codes round-trip
//! losslessly. Variant names follow the SDK constants.

macro_rules! sdk_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident),+ $(,)? } $(, $other:ident)?) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub enum $name {
            #[default]
            $($variant,)+
            $($other(i32),)?
        }
    };
}

sdk_enum!(
    /// SDK: `AreaSectionKind`.
    AreaSectionKind { ReportHeader, PageHeader, GroupHeader, Detail, GroupFooter, ReportFooter, PageFooter }, Other);

sdk_enum!(
    /// SDK: `LineStyle` (`CrystalDecisions.Shared.LineStyle`).
    LineStyle { NoLine, SingleLine, DoubleLine, DashLine, DotLine, FirstInvalidLineStyle, BlankLine }, Other);

sdk_enum!(
    /// SDK: `Alignment` (`CrystalDecisions.Shared.Alignment`; horizontal + vertical members).
    Alignment {
        DefaultAlign, LeftAlign, HorizontalCenterAlign, RightAlign, Justified,
        Decimal, TopAlign, VerticalCenterAlign, BottomAlign
    }, Other);

sdk_enum!(
    /// SDK: `FieldValueType` — a field's data type.
    FieldValueType {
        Unknown, Int8s, Int16s, Int32s, Int32u, Number, Currency, Boolean,
        String, Date, Time, DateTime, Blob, PersistentMemo
    }, Other);

sdk_enum!(
    /// SDK: `SortDirection`.
    SortDirection { AscendingOrder, DescendingOrder, NoSortOrder }, Other);

sdk_enum!(
    /// XML `@SortType` — which collection a sort came from.
    SortKind { RecordSortField, GroupSortField });

sdk_enum!(
    /// SDK: `SummaryOperation` (`CrystalDecisions.Shared.SummaryOperation`, full table).
    SummaryOperation {
        Sum, Average, SampleVariance, SampleStandardDeviation, Maximum, Minimum, Count,
        PopVariance, PopStandardDeviation, DistinctCount, Correlation, Covariance, WeightedAvg,
        Median, Percentile, NthLargest, NthSmallest, Mode, NthMostFrequent
    }, Other);

sdk_enum!(
    /// SDK: `EvaluationConditionType`.
    EvaluationConditionType { NoCondition, OnFormula, OnChangeOfField, OnChangeOfGroup }, Other);

sdk_enum!(
    /// SDK: `ResetConditionType`.
    ResetConditionType { NoCondition, OnChangeOfField, OnChangeOfGroup, OnFormula }, Other);

sdk_enum!(
    /// SDK: `TableJoinType`.
    TableJoinType { Equal, LeftOuter, RightOuter, NotEqual, GreaterThan, LessThan }, Other);

sdk_enum!(
    /// SDK: `ParameterFieldType`.
    ParameterType { ReportParameter, StoreProcedureParameter }, Other);

sdk_enum!(
    /// SDK: `ParameterValueRangeKind`.
    ParameterValueKind {
        StringParameter, NumberParameter, CurrencyParameter, BooleanParameter,
        DateParameter, TimeParameter, DateTimeParameter
    }, Other);

sdk_enum!(
    /// SDK: `PaperOrientation`.
    PaperOrientation { DefaultPaperOrientation, Portrait, Landscape }, Other);

sdk_enum!(
    /// SDK: `PrinterDuplex`.
    PrinterDuplex { Default, Simplex, Vertical, Horizontal }, Other);

sdk_enum!(
    /// SDK: `HyperlinkType`.
    HyperlinkType { NoHyperlink, AnEMailAddress, AFileOrWebSite, CurrentWebsiteField, ReportPartDrilldown, AnotherReportObject }, Other);

sdk_enum!(
    /// SDK: `TextRotationAngle`.
    TextRotationAngle { Rotate0, Rotate90, Rotate270 }, Other);

sdk_enum!(
    /// SDK: `ReadingOrder`.
    ReadingOrder { LeftToRight, RightToLeft }, Other);

sdk_enum!(
    /// SDK: `PictureType`.
    PictureType { Bitmap, Metafile, Ole, Other_ }, Other);

sdk_enum!(
    /// SDK: `ConnectionInfoKind`.
    ConnectionInfoKind { Unknown, CRQE, SQL, File, StoreProcedure }, Other);

sdk_enum!(
    /// SDK: `SpecialVarType` (special-field kind).
    SpecialFieldType {
        RecordNumber, PageNumber, GroupNumber, TotalPageCount, PrintDate, PrintTime,
        ModificationDate, ModificationTime, DataDate, DataTime, RecordSelection,
        GroupSelection, ReportTitle, ReportComments, FileAuthor, FilePath, FileCreationDate
    }, Other);

/// SDK: `PaperSize` (`CrystalDecisions.Shared.PaperSize`). The enum's integer values equal the
/// Windows `DMPAPER_*` codes stored in the record, so the code maps straight onto the variant.
/// Driver-specific sizes outside the SDK table (code > 41) keep their raw code in
/// [`PaperSize::Code`] and surface as the bare integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PaperSize {
    #[default]
    DefaultPaperSize,
    PaperLetter,
    PaperLetterSmall,
    PaperTabloid,
    PaperLedger,
    PaperLegal,
    PaperStatement,
    PaperExecutive,
    PaperA3,
    PaperA4,
    PaperA4Small,
    PaperA5,
    PaperB4,
    PaperB5,
    PaperFolio,
    PaperQuarto,
    Paper10x14,
    Paper11x17,
    PaperNote,
    PaperEnvelope9,
    PaperEnvelope10,
    PaperEnvelope11,
    PaperEnvelope12,
    PaperEnvelope14,
    PaperCsheet,
    PaperDsheet,
    PaperEsheet,
    PaperEnvelopeDL,
    PaperEnvelopeC5,
    PaperEnvelopeC3,
    PaperEnvelopeC4,
    PaperEnvelopeC6,
    PaperEnvelopeC65,
    PaperEnvelopeB4,
    PaperEnvelopeB5,
    PaperEnvelopeB6,
    PaperEnvelopeItaly,
    PaperEnvelopeMonarch,
    PaperEnvelopePersonal,
    PaperFanfoldUS,
    PaperFanfoldStdGerman,
    PaperFanfoldLegalGerman,
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
    #[default]
    Auto,
    Upper,
    Lower,
    Middle,
    Manual,
    Envelope,
    EnvManual,
    Tractor,
    SmallFmt,
    LargeFmt,
    LargeCapacity,
    Cassette,
    FormSource,
    Code(i32),
}
