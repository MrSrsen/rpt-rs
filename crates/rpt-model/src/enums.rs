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

        impl $name {
            /// The SDK constant name for this variant, as a stable `&'static str`. It equals the
            /// variant identifier (matching the derived `Debug` of a fieldless variant), giving
            /// serializers an explicit, greppable spelling to emit instead of relying on `Debug`;
            /// the `sdk_name_matches_wire_vocabulary` test pins a few names so a rename fails loudly.
            /// The unmapped-code arm reports its wrapper name (`Other`/`Code`) without the raw code.
            pub fn sdk_name(self) -> &'static str {
                match self {
                    $(Self::$variant => stringify!($variant),)+
                    $(Self::$other(_) => stringify!($other),)?
                }
            }
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
    /// Vertical text alignment of an object within its box (SDK `crAlignmentVerticalAlignment`;
    /// `crAlignmentTop` / `crAlignmentVerticalCenter` / `crAlignmentBottom`). Stored in the `0x00fc`
    /// ObjectFormat leaf (byte 3) using the shared [`Alignment`] ordinals (`TopAlign` = 6,
    /// `VerticalCenterAlign` = 7, `BottomAlign` = 8).
    VerticalAlignment {
        /// Top-aligned (the engine default).
        Top,
        /// Vertically centered.
        VerticalCenter,
        /// Bottom-aligned.
        Bottom,
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
/// Which collection a sort came from.
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
    /// SDK: `TableJoinType` — the engine's single, lossy join descriptor, which folds a link's
    /// outer-ness and its comparison operator into one enum (so it cannot express "left outer
    /// **and** `>`"). The file stores the two independently; see [`TableJoinKind`] and
    /// [`TableLinkOperator`], which a consumer folds into this only for the SDK-shaped surface.
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
    /// A table link's **outer-ness** — the `Join Type` half of the designer's Link Options dialog,
    /// orthogonal to its [`TableLinkOperator`]. Stored as its own word in the `0x000a` link record.
    TableJoinKind {
        /// Inner join.
        Inner,
        /// Left outer join.
        LeftOuter,
        /// Right outer join.
        RightOuter,
        /// Full outer join.
        FullOuter,
    }, Other);

sdk_enum!(
    /// A table link's **comparison operator** — the other half of the designer's Link Options
    /// dialog, orthogonal to its [`TableJoinKind`]. Stored as its own word in the `0x000a` link
    /// record. `GreaterOrEqual`/`LessOrEqual` have no [`TableJoinType`] counterpart.
    TableLinkOperator {
        /// `=`.
        Equal,
        /// `<>`.
        NotEqual,
        /// `<`.
        LessThan,
        /// `<=`.
        LessOrEqual,
        /// `>`.
        GreaterThan,
        /// `>=`.
        GreaterOrEqual,
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
    /// SDK: `CurrencyPositionType` (`CrystalDecisions.Shared.CurrencyPositionType`). Byte value =
    /// ordinal; stored in the numeric `0x00f8` leaf (byte 13). Describes where the currency symbol
    /// sits relative to the value and the negative sign/brackets.
    CurrencyPosition {
        /// Leading currency symbol, inside the negative sign/brackets (the engine default).
        LeadingCurrencyInsideNegative,
        /// Leading currency symbol, outside the negative sign/brackets.
        LeadingCurrencyOutsideNegative,
        /// Trailing currency symbol, inside the negative sign/brackets.
        TrailingCurrencyInsideNegative,
        /// Trailing currency symbol, outside the negative sign/brackets.
        TrailingCurrencyOutsideNegative,
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
    /// 0 = `ShortDayOfWeek`, 1 = `LongDayOfWeek`, 2 = `NoDayOfWeek` (the usual value — no weekday
    /// shown). Not exported, so decoded for record completeness only.
    DayOfWeekFormat {
        /// Abbreviated weekday name (`Wed`).
        ShortDayOfWeek,
        /// Full weekday name (`Wednesday`).
        LongDayOfWeek,
        /// Weekday not shown.
        NoDayOfWeek,
    }, Other);

sdk_enum!(
    /// SDK: `DateOrder` (`DateFieldFormat.DateOrder`). Native `RDDateOrderType`. The relative order of
    /// the day/month/year elements. Decoded from the `0x00f2` date leaf **byte 0**.
    /// Disk codes: `1` = `DayMonthYear` (European reports), `2` = `MonthDayYear`
    /// (US reports); `0` = `YearMonthDay` (never surfaced on a real date field — the stored default of
    /// non-date fields). Like the rest of `DateFieldFormat`, only reported verbatim for an explicit
    /// (non-system-default) date-valued field; system-default fields resolve it from the host locale.
    DateOrder {
        /// Year, then month, then day (`2024/03/05`).
        YearMonthDay,
        /// Day, then month, then year (`05/03/2024`).
        DayMonthYear,
        /// Month, then day, then year (`03/05/2024`).
        MonthDayYear,
    }, Other);

sdk_enum!(
    /// SDK: `HourFormat` (`TimeFieldFormat.HourFormat`). Native `RDHourType`. Decoded from the
    /// `0x00f6` time leaf **byte 2**. Disk codes (on explicit-format
    /// fields): `0` = `NumericHour` (leading-zero), `1` = `NoLeadingZeroNumericHour`, `2` = `NoHour`.
    /// A stored fact only for an explicit (non-system-default) time/datetime field; system-default
    /// fields resolve it at runtime from the host locale.
    HourFormat {
        /// Numeric hour with a leading zero (`09`).
        NumericHour,
        /// Numeric hour without a leading zero (`9`).
        NoLeadingZeroNumericHour,
        /// Hour not shown.
        NoHour,
    }, Other);

sdk_enum!(
    /// SDK: `MinuteFormat` (`TimeFieldFormat.MinuteFormat`). Native `RDMinuteType`. Decoded from the
    /// `0x00f6` time leaf **byte 3**. Disk codes: `0` = `NumericMinute`,
    /// `2` = `NoMinute` (`1` = `LeadingZeroNumericMinute`, unobserved). Explicit-field stored
    /// fact; system-default resolves from the host locale.
    MinuteFormat {
        /// Numeric minute (`05`).
        NumericMinute,
        /// Numeric minute without a leading zero (`5`).
        NoLeadingZeroNumericMinute,
        /// Minute not shown.
        NoMinute,
    }, Other);

sdk_enum!(
    /// SDK: `SecondFormat` (`TimeFieldFormat.SecondFormat`). Native `RDSecondType`. Decoded from the
    /// `0x00f6` time leaf **byte 4**. Disk codes: `0` = `NumericSecond`,
    /// `2` = `NoSecond` (`1` = `LeadingZeroNumericSecond`, unobserved). Explicit-field stored
    /// fact; system-default resolves from the host locale.
    SecondFormat {
        /// Numeric second (`05`).
        NumericSecond,
        /// Numeric second without a leading zero (`5`).
        NoLeadingZeroNumericSecond,
        /// Second not shown.
        NoSecond,
    }, Other);

sdk_enum!(
    /// SDK: `DateTimeOrder` (`DateTimeFieldFormat.DateTimeOrder`). Native `RDDateTimeOrderType`. Which
    /// of the date/time parts is shown and in what order. Decoded from the `0x00f4` datetime leaf
    /// **byte 0**. Disk codes: `0` = `DateThenTime`, `2` = `DateOnly`
    /// (`1` = `TimeThenDate`, `3` = `TimeOnly`, both unobserved). A genuine
    /// stored layout choice (independent of the date/time system-default sub-formats).
    DateTimeOrder {
        /// The date part, then the time part.
        DateThenTime,
        /// The time part, then the date part.
        TimeThenDate,
        /// Only the date part is shown.
        DateOnly,
        /// Only the time part is shown.
        TimeOnly,
    }, Other);

sdk_enum!(
    /// SDK: `TextFormat` (`StringFieldFormat.TextFormat`). Native `RDTextFormatType`. How a string
    /// field's text is interpreted when rendered. Decoded from the `0x00fa` string leaf **byte 15**.
    /// Disk codes: `0` = `StandardText`,
    /// `2` = `HTMLText` (`1` = `RTFText`, unobserved). A genuine stored layout fact on every
    /// string field, not runtime-resolved — render-relevant (HTML/RTF interpretation).
    TextFormat {
        /// Plain text.
        StandardText,
        /// Rich Text Format markup.
        RTFText,
        /// HTML markup.
        HTMLText,
    }, Other);

sdk_enum!(
    /// SDK: `CrChartTypeEnum` — a chart's **data-layout** axis (how the chart is bound to data),
    /// exposed by RAS as `ChartDefinition.ChartType`. This is **orthogonal** to the visual shape
    /// ([`ChartGraphType`](crate::ChartGraphType), bar/pie/line): a Group chart can be any shape.
    ///
    /// Decoded from the chart's `0x011c` analytic-header record, **leaf byte 2**. The disk encoding
    /// is `0` = Detail, `1` = Group, `2` = CrossTab — its own ordering, **not** the SDK
    /// declaration order below (see [`from_code`](Self::from_code)).
    ///
    /// `Detail`, `Group`, and `CrossTab` are established. `OLAP`'s disk code is unknown (see
    /// [`from_code`](Self::from_code)).
    ChartLayoutType {
        /// The chart summarizes an existing report **group** — placed in a group/report header or
        /// footer, it charts the report's own group(s) over the report's summary field(s) (the
        /// "Top-N"/group-layout charts). Disk byte 2 = `1`.
        Group,
        /// The **Advanced** ("detail") layout — the chart defines its own "on change of" category
        /// and "show value" data bindings directly, independent of any report group. Disk byte 2 =
        /// `0` (the engine default).
        Detail,
        /// The chart is bound to a **cross-tab** (its data comes from a cross-tab grid). Disk byte
        /// 2 = `2`.
        CrossTab,
        /// The chart is bound to an **OLAP** grid. Its disk code is unknown,
        /// so [`from_code`](Self::from_code) never yields it (an unknown byte 2
        /// round-trips through [`Other`](Self::Other)); the variant mirrors the SDK enum surface.
        OLAP,
    }, Other);

impl ChartLayoutType {
    /// Decode the chart layout from the `0x011c` analytic-header **leaf byte 2**. The disk codes are
    /// `0` = Detail, `1` = Group, `2` = CrossTab — the engine's own ordering (Detail, the Advanced
    /// default, is `0`), distinct from the SDK's `{Group, Detail, CrossTab, OLAP}` declaration order.
    ///
    /// OLAP's disk code is unknown, so any other byte round-trips through [`Other`](Self::Other)
    /// rather than being guessed onto [`OLAP`](Self::OLAP).
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Detail,
            1 => Self::Group,
            2 => Self::CrossTab,
            other => Self::Other(i32::from(other)),
        }
    }
}

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

impl DateOrder {
    /// Decode the `0x00f2` date leaf byte 0 (`dateOrder`).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::YearMonthDay,
            1 => Self::DayMonthYear,
            2 => Self::MonthDayYear,
            other => Self::Other(other),
        }
    }
}

impl HourFormat {
    /// Decode the `0x00f6` time leaf byte 2 (`hourType`). On explicit-format fields:
    /// `0` = `NumericHour` (leading-zero), `1` = `NoLeadingZeroNumericHour`, `2` = `NoHour`.
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::NumericHour,
            1 => Self::NoLeadingZeroNumericHour,
            2 => Self::NoHour,
            other => Self::Other(other),
        }
    }
}

impl MinuteFormat {
    /// Decode the `0x00f6` time leaf byte 3 (`minuteType`).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::NumericMinute,
            1 => Self::NoLeadingZeroNumericMinute,
            2 => Self::NoMinute,
            other => Self::Other(other),
        }
    }
}

impl SecondFormat {
    /// Decode the `0x00f6` time leaf byte 4 (`secondType`).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::NumericSecond,
            1 => Self::NoLeadingZeroNumericSecond,
            2 => Self::NoSecond,
            other => Self::Other(other),
        }
    }
}

impl DateTimeOrder {
    /// Decode the `0x00f4` datetime leaf byte 0 (`dateTimeOrder`).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::DateThenTime,
            1 => Self::TimeThenDate,
            2 => Self::DateOnly,
            3 => Self::TimeOnly,
            other => Self::Other(other),
        }
    }
}

impl TextFormat {
    /// Decode the `0x00fa` string leaf byte 15 (`textFormat`).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::StandardText,
            1 => Self::RTFText,
            2 => Self::HTMLText,
            other => Self::Other(other),
        }
    }
}

impl ReadingOrder {
    /// Decode a reading-order byte (SDK ordinal): `0` = left-to-right, `1` = right-to-left.
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::LeftToRight,
            1 => Self::RightToLeft,
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

impl CurrencyPosition {
    /// Decode the currency-position byte (`0x00f8` byte 13; SDK ordinal).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::LeadingCurrencyInsideNegative,
            1 => Self::LeadingCurrencyOutsideNegative,
            2 => Self::TrailingCurrencyInsideNegative,
            3 => Self::TrailingCurrencyOutsideNegative,
            other => Self::Other(other),
        }
    }
}

impl VerticalAlignment {
    /// Decode the object-format vertical-alignment byte (`0x00fc` byte 3), which reuses the shared
    /// [`Alignment`] ordinals: `6` = top, `7` = vertical centre, `8` = bottom.
    pub fn from_code(code: i32) -> Self {
        match code {
            6 => Self::Top,
            7 => Self::VerticalCenter,
            8 => Self::Bottom,
            other => Self::Other(other),
        }
    }
}

impl TextRotationAngle {
    /// Decode the object-format text-rotation value (`0x00fc` bytes 20-21, `u16` BE), which stores the
    /// angle in degrees directly: `0` = upright, `90` / `270` = the two quarter-turn rotations.
    pub fn from_code(degrees: i32) -> Self {
        match degrees {
            0 => Self::Rotate0,
            90 => Self::Rotate90,
            270 => Self::Rotate270,
            other => Self::Other(other),
        }
    }

    /// The rotation in degrees counter-clockwise (`0` / `90` / `270`; an unmapped code is returned
    /// verbatim). This is the angle a renderer applies to the text run.
    pub fn degrees(self) -> i32 {
        match self {
            Self::Rotate0 => 0,
            Self::Rotate90 => 90,
            Self::Rotate270 => 270,
            Self::Other(d) => d,
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
        /// An e-mail address.
        AnEMailAddress,
        /// A website / file URL (RAS `Website`, stored code 0).
        Website,
        /// An HTML link (RAS `Html`, stored code 2). The RAS/JSON model keeps this distinct from
        /// [`Website`](Self::Website) even though the designer Format Editor groups both under its single
        /// "A File or Web Site" radio.
        Html,
        /// A website built from the current field's value.
        CurrentWebsiteField,
        /// A drill-down into a report part.
        ReportPartDrilldown,
        /// A link to another report object.
        AnotherReportObject,
    }, Other);

impl HyperlinkType {
    /// Decode the stored hyperlink-type byte — the RAS `CrHyperlinkTypeEnum` ordinal held in the
    /// `0x00fc` ObjectFormat leaf (`Website=0, Email=1, Html=2, CrystalReport=3, WebsiteFieldValue=4,
    /// EmailFieldValue=5, Undefined=6, Drilldown=7, ReportObject=8`). `Website` and `Html` are kept
    /// distinct to mirror the RAS scheme (the designer Format Editor groups both under one radio, but the
    /// stored byte — and the RAS/JSON model — distinguishes them). `EmailFieldValue` still folds onto
    /// `CurrentWebsiteField`. `Undefined` (6) is the engine's "no hyperlink" sentinel. `CrystalReport`
    /// (3), a programmatic-only link to another `.rpt`, has no designer variant and is `Other(3)`.
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::Website,
            2 => Self::Html,
            1 => Self::AnEMailAddress,
            4 | 5 => Self::CurrentWebsiteField,
            6 => Self::NoHyperlink,
            7 => Self::ReportPartDrilldown,
            8 => Self::AnotherReportObject,
            other => Self::Other(other),
        }
    }
}

sdk_enum!(
    /// SDK: `LineSpacingType` — how a paragraph's stored line-spacing value is applied.
    LineSpacingType {
        /// The value is a multiple of the font's natural line height (`1.0` = single, `2.0` = double).
        Multiple,
        /// The value is an exact line pitch in twips, independent of the font's size.
        Exact,
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
        /// A named SDK picture kind other than bitmap/metafile/OLE (distinct from the `Other(i32)`
        /// catch-all that preserves an unmapped engine code).
        OtherKnown,
    }, Other);

/// The concrete media/container format of an embedded picture's bytes, sniffed from the leading
/// magic of [`PictureObject::data`](crate::PictureObject::data).
///
/// This is the *wire* image format of the bytes stored in the report (the OLE `Embedding N/CONTENTS`
/// stream), not the coarse SDK [`PictureType`]. The native engine supports importing this whole set
/// (its extension/MIME table lists `jpg gif tif pct pic iff dib tga pcx png jpeg tiff bmp`); which of
/// them actually appear *embedded* depends on the designer version (older builds transcode raster
/// imports to a DIB/BMP, so `Bmp` is the norm). Detected by magic so the renderer can
/// pick the right data-URI MIME type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ImageFormat {
    /// Windows Bitmap — a full `BM` file (14-byte `BITMAPFILEHEADER` + DIB). The usual form.
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

/// The natural extent (SDK `OriginalWidth`/`OriginalHeight`) of an embedded picture, in twips, as
/// the native engine's OLE-extent computation yields it — or `None` when the payload's pixel
/// dimensions can't be read from a header we parse.
///
/// This is a *derived* value: the natural size is not stored in the report, the engine recomputes
/// it at load from the embedded image. Pixel dimensions are read from the format header
/// (BMP/DIB `BITMAPINFOHEADER`, PNG `IHDR`, JPEG `SOFn`) and converted to twips through HIMETRIC
/// exactly as the engine does: `himetric = MulDiv(pixels, 100000, pixelsPerMetre)`, falling back to
/// 96 dpi (`MulDiv(pixels, 2540, 96)`) when the header carries no resolution, then
/// `twips = MulDiv(himetric, 1440, 2540)`. PNG/JPEG pixel dimensions are always taken at the 96-dpi
/// fallback (their headers do not carry the DIB pixels-per-metre resolution the extent math keys
/// on). Formats whose header is not parsed here — metafiles (WMF/EMF), the legacy 12-byte
/// `BITMAPCOREHEADER` DIB, and the rarer raster containers — return `None`.
pub fn natural_extent(image_bytes: &[u8]) -> Option<(crate::Twips, crate::Twips)> {
    let (w_px, h_px, x_ppm, y_ppm) = match ImageFormat::sniff(image_bytes) {
        // A `BM` file's BITMAPINFOHEADER starts at offset 14 (after the 14-byte BITMAPFILEHEADER).
        ImageFormat::Bmp => info_header_dimensions(image_bytes, 14)?,
        // A bare BITMAPINFOHEADER DIB (40-byte header). The legacy BITMAPCOREHEADER (`0x0c`) has no
        // resolution and a different dimension layout, so it is left unparsed.
        ImageFormat::Dib if image_bytes.starts_with(&[0x28, 0x00, 0x00, 0x00]) => {
            info_header_dimensions(image_bytes, 0)?
        }
        ImageFormat::Png => png_dimensions(image_bytes)?,
        ImageFormat::Jpeg => jpeg_dimensions(image_bytes)?,
        _ => return None,
    };
    Some((
        crate::Twips(himetric_twips(w_px, x_ppm)),
        crate::Twips(himetric_twips(h_px, y_ppm)),
    ))
}

/// Read pixel dimensions and pixels-per-metre resolution from a `BITMAPINFOHEADER` located at
/// `base` (0 for a bare DIB, 14 for a `BM` file). Returns `(width, height, x_ppm, y_ppm)`.
fn info_header_dimensions(data: &[u8], base: usize) -> Option<(i32, i32, i32, i32)> {
    if data.len() < base + 32 {
        return None;
    }
    let rd = |o: usize| i32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    Some((
        rd(base + 4).abs(),
        rd(base + 8).abs(),
        rd(base + 24),
        rd(base + 28),
    ))
}

/// Read pixel dimensions from a PNG `IHDR` chunk. The header carries no DIB resolution, so the
/// returned pixels-per-metre are `0` (the 96-dpi extent fallback).
fn png_dimensions(data: &[u8]) -> Option<(i32, i32, i32, i32)> {
    if data.len() < 24 || &data[12..16] != b"IHDR" {
        return None;
    }
    let rd = |o: usize| u32::from_be_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let w = i32::try_from(rd(16)).ok()?;
    let h = i32::try_from(rd(20)).ok()?;
    Some((w, h, 0, 0))
}

/// Read pixel dimensions from a JPEG frame header (`SOFn`), scanning the marker segments from the
/// `SOI`. The header carries no DIB resolution, so the returned pixels-per-metre are `0`.
fn jpeg_dimensions(data: &[u8]) -> Option<(i32, i32, i32, i32)> {
    let mut i = 2; // Skip the SOI marker (FF D8).
    while i + 1 < data.len() {
        if data[i] != 0xff {
            return None;
        }
        // A marker may be preceded by any number of 0xFF fill bytes.
        let mut j = i + 1;
        while data.get(j) == Some(&0xff) {
            j += 1;
        }
        let marker = *data.get(j)?;
        i = j + 1;
        match marker {
            // Standalone markers (RSTn, SOI, EOI, TEM) carry no payload.
            0xd0..=0xd9 | 0x01 => continue,
            // Frame headers (SOFn) carry the dimensions; DHT/JPG/DAC are not frame headers.
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                // Segment: 2-byte length, 1-byte precision, 2-byte height, 2-byte width.
                if i + 7 > data.len() {
                    return None;
                }
                let h = i32::from(u16::from_be_bytes([data[i + 3], data[i + 4]]));
                let w = i32::from(u16::from_be_bytes([data[i + 5], data[i + 6]]));
                return Some((w, h, 0, 0));
            }
            // Any other marker is length-prefixed; skip its payload.
            _ => {
                if i + 2 > data.len() {
                    return None;
                }
                let len = usize::from(u16::from_be_bytes([data[i], data[i + 1]]));
                if len < 2 {
                    return None;
                }
                i += len;
            }
        }
    }
    None
}

/// Convert a pixel count to twips via HIMETRIC, matching the engine's OLE-extent computation:
/// `himetric = MulDiv(px, 100000, ppm)` (96-dpi fallback when the resolution is absent), then
/// `twips = MulDiv(himetric, 1440, 2540)`.
fn himetric_twips(px: i32, ppm: i32) -> i32 {
    let himetric = if ppm > 0 {
        muldiv(px, 100_000, ppm)
    } else {
        muldiv(px, 2540, 96)
    };
    muldiv(himetric, 1440, 2540)
}

/// `MulDiv(a, b, c)` — `round(a * b / c)` in 64-bit, matching the Win32 rounding the engine uses.
fn muldiv(a: i32, b: i32, c: i32) -> i32 {
    if c == 0 {
        return 0;
    }
    ((i64::from(a) * i64::from(b) + i64::from(c) / 2) / i64::from(c)) as i32
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
        /// The "Page N of M" composite (page number and total page count).
        PageNofM,
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
    /// The SDK `PaperSize` constant name for this variant, as a stable `&'static str` — an explicit
    /// spelling for serializers to emit instead of relying on `Debug`. The
    /// [`Code`](PaperSize::Code) arm reports its wrapper name only; a caller surfaces the raw code
    /// as a bare integer instead.
    pub fn sdk_name(self) -> &'static str {
        match self {
            Self::DefaultPaperSize => stringify!(DefaultPaperSize),
            Self::PaperLetter => stringify!(PaperLetter),
            Self::PaperLetterSmall => stringify!(PaperLetterSmall),
            Self::PaperTabloid => stringify!(PaperTabloid),
            Self::PaperLedger => stringify!(PaperLedger),
            Self::PaperLegal => stringify!(PaperLegal),
            Self::PaperStatement => stringify!(PaperStatement),
            Self::PaperExecutive => stringify!(PaperExecutive),
            Self::PaperA3 => stringify!(PaperA3),
            Self::PaperA4 => stringify!(PaperA4),
            Self::PaperA4Small => stringify!(PaperA4Small),
            Self::PaperA5 => stringify!(PaperA5),
            Self::PaperB4 => stringify!(PaperB4),
            Self::PaperB5 => stringify!(PaperB5),
            Self::PaperFolio => stringify!(PaperFolio),
            Self::PaperQuarto => stringify!(PaperQuarto),
            Self::Paper10x14 => stringify!(Paper10x14),
            Self::Paper11x17 => stringify!(Paper11x17),
            Self::PaperNote => stringify!(PaperNote),
            Self::PaperEnvelope9 => stringify!(PaperEnvelope9),
            Self::PaperEnvelope10 => stringify!(PaperEnvelope10),
            Self::PaperEnvelope11 => stringify!(PaperEnvelope11),
            Self::PaperEnvelope12 => stringify!(PaperEnvelope12),
            Self::PaperEnvelope14 => stringify!(PaperEnvelope14),
            Self::PaperCsheet => stringify!(PaperCsheet),
            Self::PaperDsheet => stringify!(PaperDsheet),
            Self::PaperEsheet => stringify!(PaperEsheet),
            Self::PaperEnvelopeDL => stringify!(PaperEnvelopeDL),
            Self::PaperEnvelopeC5 => stringify!(PaperEnvelopeC5),
            Self::PaperEnvelopeC3 => stringify!(PaperEnvelopeC3),
            Self::PaperEnvelopeC4 => stringify!(PaperEnvelopeC4),
            Self::PaperEnvelopeC6 => stringify!(PaperEnvelopeC6),
            Self::PaperEnvelopeC65 => stringify!(PaperEnvelopeC65),
            Self::PaperEnvelopeB4 => stringify!(PaperEnvelopeB4),
            Self::PaperEnvelopeB5 => stringify!(PaperEnvelopeB5),
            Self::PaperEnvelopeB6 => stringify!(PaperEnvelopeB6),
            Self::PaperEnvelopeItaly => stringify!(PaperEnvelopeItaly),
            Self::PaperEnvelopeMonarch => stringify!(PaperEnvelopeMonarch),
            Self::PaperEnvelopePersonal => stringify!(PaperEnvelopePersonal),
            Self::PaperFanfoldUS => stringify!(PaperFanfoldUS),
            Self::PaperFanfoldStdGerman => stringify!(PaperFanfoldStdGerman),
            Self::PaperFanfoldLegalGerman => stringify!(PaperFanfoldLegalGerman),
            Self::Code(_) => stringify!(Code),
        }
    }

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

impl PaperSource {
    /// The SDK `PaperSource` constant name for this variant, as a stable `&'static str` — an
    /// explicit spelling for serializers to emit instead of relying on `Debug`. The
    /// [`Code`](PaperSource::Code) arm reports its wrapper name only; a caller surfaces the raw code
    /// as a bare integer instead.
    pub fn sdk_name(self) -> &'static str {
        match self {
            Self::Auto => stringify!(Auto),
            Self::Upper => stringify!(Upper),
            Self::Lower => stringify!(Lower),
            Self::Middle => stringify!(Middle),
            Self::Manual => stringify!(Manual),
            Self::Envelope => stringify!(Envelope),
            Self::EnvManual => stringify!(EnvManual),
            Self::Tractor => stringify!(Tractor),
            Self::SmallFmt => stringify!(SmallFmt),
            Self::LargeFmt => stringify!(LargeFmt),
            Self::LargeCapacity => stringify!(LargeCapacity),
            Self::Cassette => stringify!(Cassette),
            Self::FormSource => stringify!(FormSource),
            Self::Code(_) => stringify!(Code),
        }
    }
}

sdk_enum!(
    /// SDK: `CrParameterDefaultValueDisplayTypeEnum` — how the
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
    /// SDK: `CrDiscreteOrRangeKindEnum` — whether a parameter accepts
    /// discrete values, a range value, or both.
    DiscreteOrRangeKind {
        /// Accepts discrete values only.
        DiscreteValue,
        /// Accepts a range value only.
        RangeValue,
        /// Accepts both discrete and range values.
        DiscreteAndRangeValue,
    }, Other);

sdk_enum!(
/// SDK: `CrFormulaNullTreatmentEnum` — how a formula treats a null database-field value. The
/// per-formula editor setting ("If any field value is null" / "default value for its type" vs.
/// exception). Stored in the `0x76` formula record's trailer.
FormulaNullTreatment {
    /// `crTreatNullAsException` (the engine default) — a null operand propagates.
    Exception,
    /// `crTreatNullAsDefaultValue` — a null operand is replaced by its type's default value.
    DefaultValue,
});

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
    use crate::Twips;

    /// `sdk_name` is the stable serialized spelling of each variant. It must equal the variant
    /// identifier verbatim, so a rename breaks this test loudly instead of silently changing
    /// downstream output. Covers a macro-generated enum, the two hand-written enums, the
    /// unmapped-code arm, and a composed constant name built from `FieldValueType::sdk_name`.
    #[test]
    fn sdk_name_matches_wire_vocabulary() {
        assert_eq!(PaperSize::PaperLetter.sdk_name(), "PaperLetter");
        assert_eq!(PaperSource::FormSource.sdk_name(), "FormSource");
        assert_eq!(Alignment::LeftAlign.sdk_name(), "LeftAlign");
        assert_eq!(SortDirection::AscendingOrder.sdk_name(), "AscendingOrder");
        assert_eq!(FieldValueType::String.sdk_name(), "String");
        assert_eq!(
            format!("crFieldValueType{}Field", FieldValueType::String.sdk_name()),
            "crFieldValueTypeStringField"
        );
        // The unmapped-code arm reports its wrapper name, without the raw code.
        assert_eq!(PaperSize::Code(999).sdk_name(), "Code");
        assert_eq!(LineStyle::Other(7).sdk_name(), "Other");
    }

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

    #[test]
    fn chart_layout_type_from_code() {
        // Disk byte 2 of the 0x011c analytic header — its own ordering (Detail = 0, the engine
        // default), distinct from the SDK declaration order.
        assert_eq!(ChartLayoutType::from_code(0), ChartLayoutType::Detail);
        assert_eq!(ChartLayoutType::from_code(1), ChartLayoutType::Group);
        assert_eq!(ChartLayoutType::from_code(2), ChartLayoutType::CrossTab);
        // OLAP's disk code is unknown; any other byte round-trips through Other rather than being
        // guessed onto OLAP.
        assert_eq!(ChartLayoutType::from_code(3), ChartLayoutType::Other(3));
    }

    /// A `BM` file: 14-byte BITMAPFILEHEADER + a `BITMAPINFOHEADER` of `width`×`height` pixels at
    /// `ppm` pixels-per-metre (0 = no stored resolution → the 96-dpi extent fallback).
    fn bmp(width: i32, height: i32, ppm: i32) -> Vec<u8> {
        let mut b = vec![0u8; 54];
        b[0..2].copy_from_slice(b"BM");
        b[14..18].copy_from_slice(&40i32.to_le_bytes());
        b[18..22].copy_from_slice(&width.to_le_bytes());
        b[22..26].copy_from_slice(&height.to_le_bytes());
        b[38..42].copy_from_slice(&ppm.to_le_bytes());
        b[42..46].copy_from_slice(&ppm.to_le_bytes());
        b
    }

    #[test]
    fn natural_extent_bmp_matches_ole_himetric() {
        // 96 px at 96 dpi = 1 inch = 1440 twips; 48 px = 720 twips.
        assert_eq!(
            natural_extent(&bmp(96, 48, 0)),
            Some((Twips(1440), Twips(720)))
        );
        // A stored ~96-dpi resolution (pixels-per-metre) yields the same extent.
        assert_eq!(
            natural_extent(&bmp(96, 48, 3780)),
            Some((Twips(1440), Twips(720)))
        );
        // A negative (top-down) height is read by magnitude.
        assert_eq!(
            natural_extent(&bmp(96, -48, 0)),
            Some((Twips(1440), Twips(720)))
        );
    }

    #[test]
    fn natural_extent_png_reads_ihdr() {
        // 8-byte signature, IHDR chunk (length 13, type, then width/height big-endian u32).
        let mut b = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        b.extend_from_slice(&13u32.to_be_bytes());
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&192u32.to_be_bytes());
        b.extend_from_slice(&96u32.to_be_bytes());
        // 192 px = 2 inch = 2880 twips; 96 px = 1440 twips (96-dpi fallback).
        assert_eq!(natural_extent(&b), Some((Twips(2880), Twips(1440))));
    }

    #[test]
    fn natural_extent_jpeg_reads_sof0() {
        // SOI then a SOF0 frame header: FF C0, length, precision, height, width (big-endian u16).
        let mut b = vec![0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08];
        b.extend_from_slice(&48u16.to_be_bytes()); // height
        b.extend_from_slice(&96u16.to_be_bytes()); // width
        b.extend_from_slice(&[0u8; 6]); // component data (ignored)
                                        // 96 px = 1440 twips, 48 px = 720 twips (96-dpi fallback).
        assert_eq!(natural_extent(&b), Some((Twips(1440), Twips(720))));
    }

    #[test]
    fn natural_extent_jpeg_skips_app_segments() {
        // An APP0/JFIF segment precedes the frame header; the marker scan must step over it.
        let mut b = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00];
        b.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08]);
        b.extend_from_slice(&48u16.to_be_bytes());
        b.extend_from_slice(&96u16.to_be_bytes());
        b.extend_from_slice(&[0u8; 6]);
        assert_eq!(natural_extent(&b), Some((Twips(1440), Twips(720))));
    }

    #[test]
    fn natural_extent_unknown_formats_are_none() {
        assert_eq!(natural_extent(&[]), None);
        assert_eq!(natural_extent(b"GIF89a....."), None);
        // A legacy 12-byte BITMAPCOREHEADER DIB is not parsed for an extent.
        assert_eq!(natural_extent(&[0x0c, 0x00, 0x00, 0x00]), None);
        // A truncated BMP header yields no extent rather than reading out of bounds.
        assert_eq!(natural_extent(b"BM\x00\x00"), None);
    }
}
