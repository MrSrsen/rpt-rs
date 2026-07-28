//! Enum → KDL value mappings.
//!
//! Every `sdk_enum!` type is rendered as a kebab-case string token; the `Other(i32)`/`Code(i32)`
//! fallback arm instead renders the raw engine ordinal as a bare integer, so an unmapped code
//! survives the export. One `match` per enum keeps the token vocabulary explicit and reviewable.

use kdl::KdlValue;
use rpt_model::{
    Alignment, AreaSectionKind, BooleanOutputType, ChartCategoryPeriod, ChartGraphType,
    ChartGridType, ChartLayoutType, ChartLegendPosition, ChartViewAngle, ConnectionInfoKind,
    CurrencyPosition, CurrencySymbolFormat, DateOrder, DateSystemDefaultType, DateTimeOrder,
    DayFormat, DayOfWeekFormat, DiscreteOrRangeKind, EvaluationConditionType, FieldRefKind,
    FieldValueType, FormulaSyntax, FormulaVariableScope, HourFormat, HyperlinkType, LineStyle,
    LovSourceKind, MinuteFormat, MonthFormat, NegativeFormat, PaperOrientation, PaperSize,
    PaperSource, ParameterDisplayType, ParameterSortOrder, ParameterType, ParameterValueKind,
    PictureType, PrinterDuplex, RangeBoundType, ReadingOrder, ResetConditionType, RoundingFormat,
    SecondFormat, SortDirection, SpecialFieldType, SummaryOperation, TableJoinKind,
    TableLinkOperator, TextFormat, VerticalAlignment, YearFormat,
};

/// A kebab token as a KDL string value.
fn tok(s: &str) -> KdlValue {
    KdlValue::from(s)
}

/// A raw engine ordinal as a KDL integer value (the `Other`/`Code` fallback form).
fn raw(code: i32) -> KdlValue {
    KdlValue::Integer(code as i128)
}

/// SDK `Alignment` — the object's horizontal text alignment token.
pub(crate) fn alignment(v: Alignment) -> KdlValue {
    match v {
        Alignment::DefaultAlign => tok("default"),
        Alignment::LeftAlign => tok("left"),
        Alignment::HorizontalCenterAlign => tok("center"),
        Alignment::RightAlign => tok("right"),
        Alignment::Justified => tok("justified"),
        Alignment::Decimal => tok("decimal"),
        Alignment::TopAlign => tok("top"),
        Alignment::VerticalCenterAlign => tok("vcenter"),
        Alignment::BottomAlign => tok("bottom"),
        Alignment::Other(c) => raw(c),
    }
}

/// SDK `AreaSectionKind` — which band an area/section belongs to.
pub(crate) fn area_section_kind(v: AreaSectionKind) -> KdlValue {
    match v {
        AreaSectionKind::ReportHeader => tok("report-header"),
        AreaSectionKind::PageHeader => tok("page-header"),
        AreaSectionKind::GroupHeader => tok("group-header"),
        AreaSectionKind::Detail => tok("detail"),
        AreaSectionKind::GroupFooter => tok("group-footer"),
        AreaSectionKind::ReportFooter => tok("report-footer"),
        AreaSectionKind::PageFooter => tok("page-footer"),
        AreaSectionKind::Other(c) => raw(c),
    }
}

/// SDK `LineStyle` — a border/drawing line style.
pub(crate) fn line_style(v: LineStyle) -> KdlValue {
    match v {
        LineStyle::NoLine => tok("none"),
        LineStyle::SingleLine => tok("single"),
        LineStyle::DoubleLine => tok("double"),
        LineStyle::DashLine => tok("dash"),
        LineStyle::DotLine => tok("dot"),
        LineStyle::FirstInvalidLineStyle => tok("invalid"),
        LineStyle::BlankLine => tok("blank"),
        LineStyle::Other(c) => raw(c),
    }
}

/// SDK `FieldValueType` — a field's value type.
pub(crate) fn field_value_type(v: FieldValueType) -> KdlValue {
    match v {
        FieldValueType::Unknown => tok("unknown"),
        FieldValueType::Int8s => tok("int8"),
        FieldValueType::Int16s => tok("int16"),
        FieldValueType::Int32s => tok("int32"),
        FieldValueType::Int32u => tok("uint32"),
        FieldValueType::Number => tok("number"),
        FieldValueType::Currency => tok("currency"),
        FieldValueType::Boolean => tok("boolean"),
        FieldValueType::String => tok("string"),
        FieldValueType::Date => tok("date"),
        FieldValueType::Time => tok("time"),
        FieldValueType::DateTime => tok("date-time"),
        FieldValueType::Blob => tok("blob"),
        FieldValueType::PersistentMemo => tok("memo"),
        FieldValueType::Other(c) => raw(c),
    }
}

/// [`FieldRefKind`] — which kind of reference a field object displays.
pub(crate) fn field_ref_kind(v: FieldRefKind) -> KdlValue {
    match v {
        FieldRefKind::DatabaseField => tok("database"),
        FieldRefKind::Formula => tok("formula"),
        FieldRefKind::Summary => tok("summary"),
        FieldRefKind::Special => tok("special"),
        FieldRefKind::GroupName => tok("group-name"),
        FieldRefKind::RunningTotal => tok("running-total"),
        FieldRefKind::Parameter => tok("parameter"),
        FieldRefKind::SqlExpression => tok("sql-expression"),
        FieldRefKind::Unknown => tok("unknown"),
    }
}

/// SDK `SortDirection` — a record/group sort direction.
pub(crate) fn sort_direction(v: SortDirection) -> KdlValue {
    match v {
        SortDirection::AscendingOrder => tok("ascending"),
        SortDirection::DescendingOrder => tok("descending"),
        SortDirection::NoSortOrder => tok("none"),
        SortDirection::TopNOrder => tok("top-n"),
        SortDirection::BottomNOrder => tok("bottom-n"),
        SortDirection::Other(c) => raw(c),
    }
}

/// SDK `SummaryOperation` — the aggregate operation of a summary/running total/measure.
pub(crate) fn summary_operation(v: SummaryOperation) -> KdlValue {
    match v {
        SummaryOperation::Sum => tok("sum"),
        SummaryOperation::Average => tok("average"),
        SummaryOperation::SampleVariance => tok("sample-variance"),
        SummaryOperation::SampleStandardDeviation => tok("sample-std-dev"),
        SummaryOperation::Maximum => tok("maximum"),
        SummaryOperation::Minimum => tok("minimum"),
        SummaryOperation::Count => tok("count"),
        SummaryOperation::PopVariance => tok("pop-variance"),
        SummaryOperation::PopStandardDeviation => tok("pop-std-dev"),
        SummaryOperation::DistinctCount => tok("distinct-count"),
        SummaryOperation::Correlation => tok("correlation"),
        SummaryOperation::Covariance => tok("covariance"),
        SummaryOperation::WeightedAvg => tok("weighted-average"),
        SummaryOperation::Median => tok("median"),
        SummaryOperation::Percentile => tok("percentile"),
        SummaryOperation::NthLargest => tok("nth-largest"),
        SummaryOperation::NthSmallest => tok("nth-smallest"),
        SummaryOperation::Mode => tok("mode"),
        SummaryOperation::NthMostFrequent => tok("nth-most-frequent"),
        SummaryOperation::Other(c) => raw(c),
    }
}

/// SDK `EvaluationConditionType` — a running total's evaluate condition.
pub(crate) fn evaluation_condition(v: EvaluationConditionType) -> KdlValue {
    match v {
        EvaluationConditionType::NoCondition => tok("none"),
        EvaluationConditionType::OnFormula => tok("on-formula"),
        EvaluationConditionType::OnChangeOfField => tok("on-change-of-field"),
        EvaluationConditionType::OnChangeOfGroup => tok("on-change-of-group"),
        EvaluationConditionType::Other(c) => raw(c),
    }
}

/// SDK `ResetConditionType` — a running total's reset condition.
pub(crate) fn reset_condition(v: ResetConditionType) -> KdlValue {
    match v {
        ResetConditionType::NoCondition => tok("none"),
        ResetConditionType::OnChangeOfField => tok("on-change-of-field"),
        ResetConditionType::OnChangeOfGroup => tok("on-change-of-group"),
        ResetConditionType::OnFormula => tok("on-formula"),
        ResetConditionType::Other(c) => raw(c),
    }
}

/// A table link's outer-ness.
pub(crate) fn table_join_kind(v: TableJoinKind) -> KdlValue {
    match v {
        TableJoinKind::Inner => tok("inner"),
        TableJoinKind::LeftOuter => tok("left-outer"),
        TableJoinKind::RightOuter => tok("right-outer"),
        TableJoinKind::FullOuter => tok("full-outer"),
        TableJoinKind::Other(c) => raw(c),
    }
}

/// A table link's comparison operator, stored independently of its [`table_join_kind`].
pub(crate) fn table_link_operator(v: TableLinkOperator) -> KdlValue {
    match v {
        TableLinkOperator::Equal => tok("equal"),
        TableLinkOperator::NotEqual => tok("not-equal"),
        TableLinkOperator::LessThan => tok("less-than"),
        TableLinkOperator::LessOrEqual => tok("less-or-equal"),
        TableLinkOperator::GreaterThan => tok("greater-than"),
        TableLinkOperator::GreaterOrEqual => tok("greater-or-equal"),
        TableLinkOperator::Other(c) => raw(c),
    }
}

/// SDK `ConnectionInfoKind` — how a connection is established.
pub(crate) fn connection_kind(v: ConnectionInfoKind) -> KdlValue {
    match v {
        ConnectionInfoKind::Unknown => tok("unknown"),
        ConnectionInfoKind::CRQE => tok("crqe"),
        ConnectionInfoKind::SQL => tok("sql"),
        ConnectionInfoKind::File => tok("file"),
        ConnectionInfoKind::StoreProcedure => tok("stored-procedure"),
        ConnectionInfoKind::Other(c) => raw(c),
    }
}

/// SDK `ParameterFieldType` — report vs. stored-procedure parameter.
pub(crate) fn parameter_type(v: ParameterType) -> KdlValue {
    match v {
        ParameterType::ReportParameter => tok("report"),
        ParameterType::StoreProcedureParameter => tok("stored-procedure"),
        ParameterType::Other(c) => raw(c),
    }
}

/// SDK `ParameterValueRangeKind` — a parameter's value type.
pub(crate) fn parameter_value_kind(v: ParameterValueKind) -> KdlValue {
    match v {
        ParameterValueKind::StringParameter => tok("string"),
        ParameterValueKind::NumberParameter => tok("number"),
        ParameterValueKind::CurrencyParameter => tok("currency"),
        ParameterValueKind::BooleanParameter => tok("boolean"),
        ParameterValueKind::DateParameter => tok("date"),
        ParameterValueKind::TimeParameter => tok("time"),
        ParameterValueKind::DateTimeParameter => tok("date-time"),
        ParameterValueKind::Other(c) => raw(c),
    }
}

/// SDK `SpecialVarType` — which built-in special field this is.
pub(crate) fn special_field_type(v: SpecialFieldType) -> KdlValue {
    match v {
        SpecialFieldType::RecordNumber => tok("record-number"),
        SpecialFieldType::PageNumber => tok("page-number"),
        SpecialFieldType::PageNofM => tok("page-n-of-m"),
        SpecialFieldType::GroupNumber => tok("group-number"),
        SpecialFieldType::TotalPageCount => tok("total-page-count"),
        SpecialFieldType::PrintDate => tok("print-date"),
        SpecialFieldType::PrintTime => tok("print-time"),
        SpecialFieldType::ModificationDate => tok("modification-date"),
        SpecialFieldType::ModificationTime => tok("modification-time"),
        SpecialFieldType::DataDate => tok("data-date"),
        SpecialFieldType::DataTime => tok("data-time"),
        SpecialFieldType::RecordSelection => tok("record-selection"),
        SpecialFieldType::GroupSelection => tok("group-selection"),
        SpecialFieldType::ReportTitle => tok("report-title"),
        SpecialFieldType::ReportComments => tok("report-comments"),
        SpecialFieldType::FileAuthor => tok("file-author"),
        SpecialFieldType::FilePath => tok("file-path"),
        SpecialFieldType::FileCreationDate => tok("file-creation-date"),
        SpecialFieldType::Other(c) => raw(c),
    }
}

/// SDK `PaperOrientation`.
pub(crate) fn paper_orientation(v: PaperOrientation) -> KdlValue {
    match v {
        PaperOrientation::DefaultPaperOrientation => tok("default"),
        PaperOrientation::Portrait => tok("portrait"),
        PaperOrientation::Landscape => tok("landscape"),
        PaperOrientation::Other(c) => raw(c),
    }
}

/// SDK `PrinterDuplex`.
pub(crate) fn printer_duplex(v: PrinterDuplex) -> KdlValue {
    match v {
        PrinterDuplex::Default => tok("default"),
        PrinterDuplex::Simplex => tok("simplex"),
        PrinterDuplex::Vertical => tok("vertical"),
        PrinterDuplex::Horizontal => tok("horizontal"),
        PrinterDuplex::Other(c) => raw(c),
    }
}

/// SDK `PaperSize`. The common cut-sheet/named sizes get a token; a driver-specific `Code(i)` renders
/// its raw `DMPAPER_*` ordinal. Exotic named sizes without a token are not emitted (the caller skips
/// them), keeping the vocabulary small; they remain recoverable from the record stream.
pub(crate) fn paper_size(v: PaperSize) -> Option<KdlValue> {
    let t = match v {
        PaperSize::DefaultPaperSize => return None,
        PaperSize::PaperLetter | PaperSize::PaperLetterSmall => "letter",
        PaperSize::PaperTabloid => "tabloid",
        PaperSize::PaperLedger => "ledger",
        PaperSize::PaperLegal => "legal",
        PaperSize::PaperStatement => "statement",
        PaperSize::PaperExecutive => "executive",
        PaperSize::PaperA3 => "a3",
        PaperSize::PaperA4 | PaperSize::PaperA4Small => "a4",
        PaperSize::PaperA5 => "a5",
        PaperSize::PaperB4 => "b4",
        PaperSize::PaperB5 => "b5",
        PaperSize::PaperFolio => "folio",
        PaperSize::PaperQuarto => "quarto",
        PaperSize::PaperNote => "note",
        PaperSize::Paper10x14 => "10x14",
        PaperSize::Paper11x17 => "11x17",
        PaperSize::Code(c) => return Some(raw(c)),
        _ => return None,
    };
    Some(tok(t))
}

/// SDK `PaperSource`. Named bins get a token; a driver-specific `Code(i)` renders its raw `DMBIN_*`
/// ordinal.
pub(crate) fn paper_source(v: PaperSource) -> KdlValue {
    match v {
        PaperSource::Auto => tok("auto"),
        PaperSource::Upper => tok("upper"),
        PaperSource::Lower => tok("lower"),
        PaperSource::Middle => tok("middle"),
        PaperSource::Manual => tok("manual"),
        PaperSource::Envelope => tok("envelope"),
        PaperSource::EnvManual => tok("envelope-manual"),
        PaperSource::Tractor => tok("tractor"),
        PaperSource::SmallFmt => tok("small-format"),
        PaperSource::LargeFmt => tok("large-format"),
        PaperSource::LargeCapacity => tok("large-capacity"),
        PaperSource::Cassette => tok("cassette"),
        PaperSource::FormSource => tok("form-source"),
        PaperSource::Code(c) => raw(c),
    }
}

/// SDK `NegativeType` — how negatives are displayed.
pub(crate) fn negative_format(v: NegativeFormat) -> KdlValue {
    match v {
        NegativeFormat::NotNegative => tok("none"),
        NegativeFormat::LeadingMinus => tok("leading-minus"),
        NegativeFormat::TrailingMinus => tok("trailing-minus"),
        NegativeFormat::Bracketed => tok("parentheses"),
        NegativeFormat::Other(c) => raw(c),
    }
}

/// SDK `CurrencySymbolType` — whether/where a currency symbol is shown.
pub(crate) fn currency_symbol(v: CurrencySymbolFormat) -> KdlValue {
    match v {
        CurrencySymbolFormat::NoSymbol => tok("none"),
        CurrencySymbolFormat::FixedSymbol => tok("fixed"),
        CurrencySymbolFormat::FloatingSymbol => tok("floating"),
        CurrencySymbolFormat::Other(c) => raw(c),
    }
}

/// SDK vertical text alignment within an object's box.
pub(crate) fn vertical_alignment(v: VerticalAlignment) -> KdlValue {
    match v {
        VerticalAlignment::Top => tok("top"),
        VerticalAlignment::VerticalCenter => tok("center"),
        VerticalAlignment::Bottom => tok("bottom"),
        VerticalAlignment::Other(c) => raw(c),
    }
}

/// SDK `CurrencyPositionType` — where the currency symbol sits vs. the value and negative sign.
pub(crate) fn currency_position(v: CurrencyPosition) -> KdlValue {
    match v {
        CurrencyPosition::LeadingCurrencyInsideNegative => tok("lead-in"),
        CurrencyPosition::LeadingCurrencyOutsideNegative => tok("lead-out"),
        CurrencyPosition::TrailingCurrencyInsideNegative => tok("trail-in"),
        CurrencyPosition::TrailingCurrencyOutsideNegative => tok("trail-out"),
        CurrencyPosition::Other(c) => raw(c),
    }
}

/// SDK `RoundingType`.
pub(crate) fn rounding_format(v: RoundingFormat) -> KdlValue {
    match v {
        RoundingFormat::RoundToHundredth => tok("hundredth"),
        RoundingFormat::RoundToUnit => tok("unit"),
        RoundingFormat::RoundToTenth => tok("tenth"),
        RoundingFormat::RoundToThousandth => tok("thousandth"),
        RoundingFormat::RoundToTenThousandth => tok("ten-thousandth"),
        RoundingFormat::RoundToHundredThousandth => tok("hundred-thousandth"),
        RoundingFormat::RoundToMillionth => tok("millionth"),
        RoundingFormat::RoundToTen => tok("ten"),
        RoundingFormat::RoundToHundred => tok("hundred"),
        RoundingFormat::RoundToThousand => tok("thousand"),
        RoundingFormat::Other(c) => raw(c),
    }
}

/// SDK `BooleanOutputType` — how a boolean value renders.
pub(crate) fn boolean_output(v: BooleanOutputType) -> KdlValue {
    match v {
        BooleanOutputType::TrueOrFalse => tok("true-false"),
        BooleanOutputType::TOrF => tok("t-f"),
        BooleanOutputType::YesOrNo => tok("yes-no"),
        BooleanOutputType::YOrN => tok("y-n"),
        BooleanOutputType::OneOrZero => tok("one-zero"),
        BooleanOutputType::Other(c) => raw(c),
    }
}

/// SDK `DayFormat`.
pub(crate) fn day_format(v: DayFormat) -> KdlValue {
    match v {
        DayFormat::NumericDay => tok("numeric"),
        DayFormat::LeadingZeroNumericDay => tok("leading-zero"),
        DayFormat::NoDay => tok("none"),
        DayFormat::Other(c) => raw(c),
    }
}

/// SDK `MonthFormat`.
pub(crate) fn month_format(v: MonthFormat) -> KdlValue {
    match v {
        MonthFormat::NumericMonth => tok("numeric"),
        MonthFormat::LeadingZeroNumericMonth => tok("leading-zero"),
        MonthFormat::ShortMonth => tok("short"),
        MonthFormat::LongMonth => tok("long"),
        MonthFormat::NoMonth => tok("none"),
        MonthFormat::Other(c) => raw(c),
    }
}

/// SDK `YearFormat`.
pub(crate) fn year_format(v: YearFormat) -> KdlValue {
    match v {
        YearFormat::ShortYear => tok("short"),
        YearFormat::LongYear => tok("long"),
        YearFormat::NoYear => tok("none"),
        YearFormat::Other(c) => raw(c),
    }
}

/// SDK `DateSystemDefaultType`.
pub(crate) fn date_system_default(v: DateSystemDefaultType) -> KdlValue {
    match v {
        DateSystemDefaultType::UseWindowsLongDate => tok("windows-long"),
        DateSystemDefaultType::UseWindowsShortDate => tok("windows-short"),
        DateSystemDefaultType::NotUsingWindowsDefaults => tok("none"),
        DateSystemDefaultType::Other(c) => raw(c),
    }
}

/// SDK `ReadingOrder`.
pub(crate) fn reading_order(v: ReadingOrder) -> KdlValue {
    match v {
        ReadingOrder::LeftToRight => tok("ltr"),
        ReadingOrder::RightToLeft => tok("rtl"),
        ReadingOrder::Other(c) => raw(c),
    }
}

/// SDK `DateOrder`.
pub(crate) fn date_order(v: DateOrder) -> KdlValue {
    match v {
        DateOrder::YearMonthDay => tok("ymd"),
        DateOrder::DayMonthYear => tok("dmy"),
        DateOrder::MonthDayYear => tok("mdy"),
        DateOrder::Other(c) => raw(c),
    }
}

/// SDK `HourFormat`.
pub(crate) fn hour_format(v: HourFormat) -> KdlValue {
    match v {
        HourFormat::NumericHour => tok("numeric"),
        HourFormat::NoLeadingZeroNumericHour => tok("no-leading-zero"),
        HourFormat::NoHour => tok("none"),
        HourFormat::Other(c) => raw(c),
    }
}

/// SDK `MinuteFormat`.
pub(crate) fn minute_format(v: MinuteFormat) -> KdlValue {
    match v {
        MinuteFormat::NumericMinute => tok("numeric"),
        MinuteFormat::NoLeadingZeroNumericMinute => tok("no-leading-zero"),
        MinuteFormat::NoMinute => tok("none"),
        MinuteFormat::Other(c) => raw(c),
    }
}

/// SDK `SecondFormat`.
pub(crate) fn second_format(v: SecondFormat) -> KdlValue {
    match v {
        SecondFormat::NumericSecond => tok("numeric"),
        SecondFormat::NoLeadingZeroNumericSecond => tok("no-leading-zero"),
        SecondFormat::NoSecond => tok("none"),
        SecondFormat::Other(c) => raw(c),
    }
}

/// SDK `DateTimeOrder`.
pub(crate) fn date_time_order(v: DateTimeOrder) -> KdlValue {
    match v {
        DateTimeOrder::DateThenTime => tok("date-then-time"),
        DateTimeOrder::TimeThenDate => tok("time-then-date"),
        DateTimeOrder::DateOnly => tok("date-only"),
        DateTimeOrder::TimeOnly => tok("time-only"),
        DateTimeOrder::Other(c) => raw(c),
    }
}

/// SDK `TextFormat`.
pub(crate) fn text_format(v: TextFormat) -> KdlValue {
    match v {
        TextFormat::StandardText => tok("standard"),
        TextFormat::RTFText => tok("rtf"),
        TextFormat::HTMLText => tok("html"),
        TextFormat::Other(c) => raw(c),
    }
}

/// SDK `HyperlinkType`.
pub(crate) fn hyperlink_type(v: HyperlinkType) -> KdlValue {
    match v {
        HyperlinkType::NoHyperlink => tok("none"),
        HyperlinkType::AnEMailAddress => tok("email"),
        HyperlinkType::Website => tok("url"),
        HyperlinkType::Html => tok("html"),
        HyperlinkType::CurrentWebsiteField => tok("current-website-field"),
        HyperlinkType::ReportPartDrilldown => tok("report-part-drilldown"),
        HyperlinkType::AnotherReportObject => tok("another-report-object"),
        HyperlinkType::Other(c) => raw(c),
    }
}

/// SDK `PictureType`.
pub(crate) fn picture_type(v: PictureType) -> KdlValue {
    match v {
        PictureType::Bitmap => tok("bitmap"),
        PictureType::Metafile => tok("metafile"),
        PictureType::Ole => tok("ole"),
        PictureType::OtherKnown => tok("other"),
        PictureType::Other(c) => raw(c),
    }
}

/// SDK `CrFormulaSyntaxEnum` — a formula's authoring dialect.
pub(crate) fn formula_syntax(v: FormulaSyntax) -> KdlValue {
    match v {
        FormulaSyntax::Crystal => tok("crystal"),
        FormulaSyntax::Basic => tok("basic"),
    }
}

/// [`ChartGraphType`] — the chart's visual shape.
pub(crate) fn chart_graph_type(v: ChartGraphType) -> KdlValue {
    match v {
        ChartGraphType::Bar => tok("bar"),
        ChartGraphType::Line => tok("line"),
        ChartGraphType::Area => tok("area"),
        ChartGraphType::Pie => tok("pie"),
        ChartGraphType::Doughnut => tok("doughnut"),
        ChartGraphType::Riser3D => tok("riser-3d"),
        ChartGraphType::Surface3D => tok("surface-3d"),
        ChartGraphType::Scatter => tok("scatter"),
        ChartGraphType::Radar => tok("radar"),
        ChartGraphType::Bubble => tok("bubble"),
        ChartGraphType::Stock => tok("stock"),
        ChartGraphType::NumericAxis => tok("numeric-axis"),
        ChartGraphType::Gauge => tok("gauge"),
        ChartGraphType::Gantt => tok("gantt"),
        ChartGraphType::Funnel => tok("funnel"),
        ChartGraphType::Histogram => tok("histogram"),
        ChartGraphType::Other(c) => raw(c),
    }
}

/// [`ChartLayoutType`] — the chart's data-layout axis.
pub(crate) fn chart_layout_type(v: ChartLayoutType) -> KdlValue {
    match v {
        ChartLayoutType::Group => tok("group"),
        ChartLayoutType::Detail => tok("detail"),
        ChartLayoutType::CrossTab => tok("cross-tab"),
        ChartLayoutType::OLAP => tok("olap"),
        ChartLayoutType::Other(c) => raw(c),
    }
}

/// [`ChartLegendPosition`].
pub(crate) fn chart_legend_position(v: ChartLegendPosition) -> KdlValue {
    match v {
        ChartLegendPosition::Right => tok("right"),
        ChartLegendPosition::Left => tok("left"),
        ChartLegendPosition::BottomCenter => tok("bottom-center"),
        ChartLegendPosition::Custom => tok("custom"),
    }
}

/// [`ChartGridType`] — an axis's gridline mode.
pub(crate) fn chart_grid_type(v: ChartGridType) -> KdlValue {
    match v {
        ChartGridType::None => tok("none"),
        ChartGridType::Minor => tok("minor"),
        ChartGridType::Major => tok("major"),
        ChartGridType::Both => tok("both"),
    }
}

/// [`ChartCategoryPeriod`] — the date-grouping period of a chart's category axis.
pub(crate) fn chart_category_period(v: ChartCategoryPeriod) -> KdlValue {
    tok(v.as_token())
}

/// [`ChartViewAngle`] — the 3-D camera preset a chart is drawn with.
pub(crate) fn chart_view_angle(v: ChartViewAngle) -> KdlValue {
    match v {
        ChartViewAngle::Standard => tok("standard"),
        ChartViewAngle::TallView => tok("tall"),
        ChartViewAngle::TopView => tok("top"),
        ChartViewAngle::DistortedView => tok("distorted"),
        ChartViewAngle::ShortView => tok("short"),
        ChartViewAngle::GroupEyeView => tok("group-eye"),
        ChartViewAngle::GroupEmphasisView => tok("group-emphasis"),
        ChartViewAngle::FewSeriesView => tok("few-series"),
        ChartViewAngle::FewGroupsView => tok("few-groups"),
        ChartViewAngle::DistortedStdView => tok("distorted-std"),
        ChartViewAngle::ThickGroupsView => tok("thick-groups"),
        ChartViewAngle::ShorterView => tok("shorter"),
        ChartViewAngle::ThickSeriesView => tok("thick-series"),
        ChartViewAngle::ThickStdView => tok("thick-std"),
        ChartViewAngle::BirdsEyeView => tok("birds-eye"),
        ChartViewAngle::MaxView => tok("max"),
    }
}

/// SDK `DayOfWeekType` — the weekday element of a date format.
pub(crate) fn day_of_week_format(v: DayOfWeekFormat) -> KdlValue {
    match v {
        DayOfWeekFormat::ShortDayOfWeek => tok("short"),
        DayOfWeekFormat::LongDayOfWeek => tok("long"),
        DayOfWeekFormat::NoDayOfWeek => tok("none"),
        DayOfWeekFormat::Other(c) => raw(c),
    }
}

/// SDK `FLScope` — a persisted formula variable's declared scope.
pub(crate) fn formula_variable_scope(v: FormulaVariableScope) -> KdlValue {
    match v {
        FormulaVariableScope::Shared => tok("shared"),
        FormulaVariableScope::Global => tok("global"),
        FormulaVariableScope::Local => tok("local"),
        FormulaVariableScope::Other(c) => raw(c),
    }
}

/// SDK `CrParameterDefaultValueDisplayTypeEnum` — how a parameter's default-value pick list displays.
pub(crate) fn parameter_display_type(v: ParameterDisplayType) -> KdlValue {
    match v {
        ParameterDisplayType::DescriptionAndValue => tok("description-and-value"),
        ParameterDisplayType::Description => tok("description"),
        ParameterDisplayType::Value => tok("value"),
        ParameterDisplayType::Other(c) => raw(c),
    }
}

/// SDK `@DefaultValueSortOrder` — the sort applied to a parameter's default-value pick list.
pub(crate) fn parameter_sort_order(v: ParameterSortOrder) -> KdlValue {
    match v {
        ParameterSortOrder::NoSort => tok("none"),
        ParameterSortOrder::AlphabeticalAscending => tok("alphabetical-ascending"),
        ParameterSortOrder::AlphabeticalDescending => tok("alphabetical-descending"),
        ParameterSortOrder::Other(c) => raw(c),
    }
}

/// SDK `CrDiscreteOrRangeKindEnum` — whether a parameter accepts discrete, range, or both.
pub(crate) fn discrete_or_range_kind(v: DiscreteOrRangeKind) -> KdlValue {
    match v {
        DiscreteOrRangeKind::DiscreteValue => tok("discrete"),
        DiscreteOrRangeKind::RangeValue => tok("range"),
        DiscreteOrRangeKind::DiscreteAndRangeValue => tok("discrete-and-range"),
        DiscreteOrRangeKind::Other(c) => raw(c),
    }
}

/// SDK `RangeBoundType` — the inclusivity of one end of a range parameter value.
pub(crate) fn range_bound_type(v: RangeBoundType) -> KdlValue {
    match v {
        RangeBoundType::NoBound => tok("open"),
        RangeBoundType::BoundInclusive => tok("inclusive"),
        RangeBoundType::BoundExclusive => tok("exclusive"),
        RangeBoundType::Other(c) => raw(c),
    }
}

/// The kind of database object a dynamic (list-of-values) parameter's pick list is sourced from.
pub(crate) fn lov_source_kind(v: LovSourceKind) -> KdlValue {
    match v {
        LovSourceKind::Table => tok("table"),
        LovSourceKind::View => tok("view"),
        LovSourceKind::StoredProcedure => tok("stored-procedure"),
        LovSourceKind::Command => tok("command"),
        LovSourceKind::Other(c) => raw(c),
    }
}
