//! Fitting enums: mapping low-level engine codes (as read from a record substrate) onto the
//! model enum variants.
//!
//! Inherent `from_code` constructors on the enums, so call sites stay ergonomic
//! (`FieldValueType::from_code(..)`). Pure integer→variant conversions with no I/O.

use crate::{
    Alignment, FieldValueType, FormulaVariableScope, LineStyle, ResetConditionType, SortDirection,
    SummaryOperation, TableJoinType,
};

impl SummaryOperation {
    /// Map the running-total/summary operation code (`0x7e` byte 0) to the variant. These are the
    /// SDK's `CrSummaryOperationEnum` values: Sum=0, Average=1, Maximum=4, Minimum=5, Count=6,
    /// DistinctCount=9. Other operations fall through to [`SummaryOperation::Other`].
    pub fn from_code(code: i32) -> SummaryOperation {
        match code {
            0 => SummaryOperation::Sum,
            1 => SummaryOperation::Average,
            2 => SummaryOperation::SampleVariance,
            3 => SummaryOperation::SampleStandardDeviation,
            4 => SummaryOperation::Maximum,
            5 => SummaryOperation::Minimum,
            6 => SummaryOperation::Count,
            7 => SummaryOperation::PopVariance,
            8 => SummaryOperation::PopStandardDeviation,
            9 => SummaryOperation::DistinctCount,
            10 => SummaryOperation::Correlation,
            11 => SummaryOperation::Covariance,
            12 => SummaryOperation::WeightedAvg,
            13 => SummaryOperation::Median,
            14 => SummaryOperation::Percentile,
            15 => SummaryOperation::NthLargest,
            16 => SummaryOperation::NthSmallest,
            17 => SummaryOperation::Mode,
            18 => SummaryOperation::NthMostFrequent,
            other => SummaryOperation::Other(other),
        }
    }
}

impl ResetConditionType {
    /// Map the running-total reset-condition code (`0x80` byte 0) to the variant.
    pub fn from_code(code: i32) -> ResetConditionType {
        match code {
            0 => ResetConditionType::NoCondition,
            1 => ResetConditionType::OnChangeOfField,
            2 => ResetConditionType::OnChangeOfGroup,
            3 => ResetConditionType::OnFormula,
            other => ResetConditionType::Other(other),
        }
    }
}

impl crate::EvaluationConditionType {
    /// Map the running-total evaluation-condition code (`0x80` byte 3) to the variant. Same numeric
    /// coding as the reset condition.
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::NoCondition,
            1 => Self::OnChangeOfField,
            2 => Self::OnChangeOfGroup,
            3 => Self::OnFormula,
            other => Self::Other(other),
        }
    }
}

impl SortDirection {
    /// Map the record-sort direction byte (`0x29` trailing byte) to the variant.
    pub fn from_code(code: i32) -> SortDirection {
        match code {
            0 => SortDirection::AscendingOrder,
            1 => SortDirection::DescendingOrder,
            // Original-order (2) and specified-order (3) both surface as no automatic sort
            // direction.
            2 | 3 => SortDirection::NoSortOrder,
            other => SortDirection::Other(other),
        }
    }
}

impl LineStyle {
    /// Map the border line-style code (`0xec` bytes 0-3) to the variant.
    pub fn from_code(code: i32) -> LineStyle {
        match code {
            0 => LineStyle::NoLine,
            1 => LineStyle::SingleLine,
            2 => LineStyle::DoubleLine,
            3 => LineStyle::DashLine,
            4 => LineStyle::DotLine,
            5 => LineStyle::FirstInvalidLineStyle,
            6 => LineStyle::BlankLine,
            other => LineStyle::Other(other),
        }
    }
}

impl Alignment {
    /// Map the object-format alignment code (`0xfc` byte 2) to the variant.
    pub fn from_code(code: i32) -> Alignment {
        match code {
            0 => Alignment::DefaultAlign,
            1 => Alignment::LeftAlign,
            2 => Alignment::HorizontalCenterAlign,
            3 => Alignment::RightAlign,
            4 => Alignment::Justified,
            5 => Alignment::Decimal,
            6 => Alignment::TopAlign,
            7 => Alignment::VerticalCenterAlign,
            8 => Alignment::BottomAlign,
            other => Alignment::Other(other),
        }
    }
}

impl FormulaVariableScope {
    /// Map a formula-variable `0x0118` scope byte (`FLScope`) to the variant.
    pub fn from_code(code: i32) -> FormulaVariableScope {
        match code {
            0 => FormulaVariableScope::Shared,
            1 => FormulaVariableScope::Global,
            2 => FormulaVariableScope::Local,
            other => FormulaVariableScope::Other(other),
        }
    }
}

impl FieldValueType {
    /// Map a formula **result-kind** code (the FL type enum, distinct from
    /// the `CrFieldValueTypeEnum` used by [`from_code`](Self::from_code)) to a [`FieldValueType`]. This
    /// is the type byte a formula variable (`0x0118`) declares: `1`=Number, `2`=Currency, `3`=Boolean,
    /// `4`=Date, `5`=Time, `6`=DateTime, `7`=String. Non-scalar result kinds (function-ref `8`, ranges
    /// `9..=0xe`, arrays `0xf..=0x1b`) have no scalar `FieldValueType` and become [`FieldValueType::Other`].
    pub fn from_result_kind(code: i32) -> FieldValueType {
        match code {
            1 => FieldValueType::Number,
            2 => FieldValueType::Currency,
            3 => FieldValueType::Boolean,
            4 => FieldValueType::Date,
            5 => FieldValueType::Time,
            6 => FieldValueType::DateTime,
            7 => FieldValueType::String,
            other => FieldValueType::Other(other),
        }
    }

    /// Map a raw engine field-type code to a [`FieldValueType`]. These are the values of the SDK's
    /// `CrFieldValueTypeEnum`: `0`=Int8s, `2`=Int16s, `4`=Int32s, `5`=Int32u, `6`=Number,
    /// `7`=Currency, `8`=Boolean, `9`=Date, `10`=Time, `11`=String, `13`=PersistentMemo, `14`=Blob,
    /// `15`=DateTime. Codes for types not in the model (Int8u=1, Int16u=3, TransientMemo=12,
    /// Decimal=16, Int64s/u=17/18, …) become [`FieldValueType::Other`].
    pub fn from_code(code: i32) -> FieldValueType {
        match code {
            0 => FieldValueType::Int8s,
            2 => FieldValueType::Int16s,
            4 => FieldValueType::Int32s,
            5 => FieldValueType::Int32u,
            6 => FieldValueType::Number,
            7 => FieldValueType::Currency,
            8 => FieldValueType::Boolean,
            9 => FieldValueType::Date,
            10 => FieldValueType::Time,
            11 => FieldValueType::String,
            13 => FieldValueType::PersistentMemo,
            14 => FieldValueType::Blob,
            15 => FieldValueType::DateTime,
            // The engine reports the wide numeric types (Decimal=16, Int64s=17, Int64u=18), which
            // SQL command tables produce, as plain NumberField (8 bytes).
            16..=18 => FieldValueType::Number,
            _ => FieldValueType::Other(code),
        }
    }

    /// Whether this is a numeric value type. A field heading left at `DefaultAlign` (with its field
    /// also at `DefaultAlign`) is right-aligned when the underlying field is numeric, left-aligned
    /// otherwise.
    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            FieldValueType::Int8s
                | FieldValueType::Int16s
                | FieldValueType::Int32s
                | FieldValueType::Int32u
                | FieldValueType::Number
                | FieldValueType::Currency
        )
    }

    /// The intrinsic `Length` (in bytes) for a fixed-size value type. Variable-length types
    /// (`String`, and anything unmodelled) return `None`, in which case the field's stored byte
    /// length is used instead. `Blob`/`PersistentMemo` are unbounded and report `-1`.
    pub fn byte_length(self) -> Option<i32> {
        Some(match self {
            FieldValueType::Int8s => 1,
            FieldValueType::Int16s => 2,
            FieldValueType::Int32s | FieldValueType::Int32u => 4,
            FieldValueType::Number | FieldValueType::Currency | FieldValueType::DateTime => 8,
            FieldValueType::Boolean => 2,
            FieldValueType::Date | FieldValueType::Time => 4,
            FieldValueType::Blob | FieldValueType::PersistentMemo => -1,
            FieldValueType::String | FieldValueType::Unknown | FieldValueType::Other(_) => {
                return None
            }
        })
    }
}

impl crate::PaperOrientation {
    /// Map the page-setup orientation code (record `0x07`, `u16[2]`) to the variant.
    pub fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Portrait,
            2 => Self::Landscape,
            0 => Self::DefaultPaperOrientation,
            other => Self::Other(other),
        }
    }
}

impl crate::PaperSize {
    /// Map the page-setup paper-size code (record `0x07`, `u16[3]`, a Windows `DMPAPER_*` value) to the
    /// variant; unmodelled sizes keep their raw code (emitted as the bare number).
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::DefaultPaperSize,
            1 => Self::PaperLetter,
            2 => Self::PaperLetterSmall,
            3 => Self::PaperTabloid,
            4 => Self::PaperLedger,
            5 => Self::PaperLegal,
            6 => Self::PaperStatement,
            7 => Self::PaperExecutive,
            8 => Self::PaperA3,
            9 => Self::PaperA4,
            10 => Self::PaperA4Small,
            11 => Self::PaperA5,
            12 => Self::PaperB4,
            13 => Self::PaperB5,
            14 => Self::PaperFolio,
            15 => Self::PaperQuarto,
            16 => Self::Paper10x14,
            17 => Self::Paper11x17,
            18 => Self::PaperNote,
            19 => Self::PaperEnvelope9,
            20 => Self::PaperEnvelope10,
            21 => Self::PaperEnvelope11,
            22 => Self::PaperEnvelope12,
            23 => Self::PaperEnvelope14,
            24 => Self::PaperCsheet,
            25 => Self::PaperDsheet,
            26 => Self::PaperEsheet,
            27 => Self::PaperEnvelopeDL,
            28 => Self::PaperEnvelopeC5,
            29 => Self::PaperEnvelopeC3,
            30 => Self::PaperEnvelopeC4,
            31 => Self::PaperEnvelopeC6,
            32 => Self::PaperEnvelopeC65,
            33 => Self::PaperEnvelopeB4,
            34 => Self::PaperEnvelopeB5,
            35 => Self::PaperEnvelopeB6,
            36 => Self::PaperEnvelopeItaly,
            37 => Self::PaperEnvelopeMonarch,
            38 => Self::PaperEnvelopePersonal,
            39 => Self::PaperFanfoldUS,
            40 => Self::PaperFanfoldStdGerman,
            41 => Self::PaperFanfoldLegalGerman,
            other => Self::Code(other),
        }
    }
}

impl crate::PrinterDuplex {
    /// Map the DEVMODE `dmDuplex` value (`0x07` record, bit 12 of dmFields) to the variant. When
    /// the field is absent the struct keeps its `Default`, so only 1/2/3 reach here.
    pub fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Simplex,
            2 => Self::Vertical,
            3 => Self::Horizontal,
            _ => Self::Default,
        }
    }
}

impl crate::PaperSource {
    /// Map the page-setup paper-source code (record `0x07`, `u16[5]`, a Windows `DMBIN_*` value) to the
    /// variant; unmodelled sources keep their raw code.
    pub fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Upper,
            2 => Self::Lower,
            3 => Self::Middle,
            4 => Self::Manual,
            5 => Self::Envelope,
            6 => Self::EnvManual,
            7 => Self::Auto,
            8 => Self::Tractor,
            9 => Self::SmallFmt,
            10 => Self::LargeFmt,
            11 => Self::LargeCapacity,
            14 => Self::Cassette,
            15 => Self::FormSource,
            other => Self::Code(other),
        }
    }
}

impl TableJoinType {
    /// Map the QE join-type code (Crystal `CRTableJoinTypeEnum`) to the variant.
    pub fn from_code(code: i32) -> TableJoinType {
        match code {
            1 => TableJoinType::Equal,
            2 => TableJoinType::LeftOuter,
            3 => TableJoinType::RightOuter,
            4 => TableJoinType::GreaterThan,
            5 => TableJoinType::LessThan,
            8 => TableJoinType::NotEqual,
            _ => TableJoinType::Other(code),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    /// `SummaryOperation::from_code` maps every SDK `CrSummaryOperationEnum` ordinal `0..=18` to its
    /// variant in order, and anything past the table round-trips through `Other`.
    #[test]
    fn summary_operation_from_code() {
        use SummaryOperation::*;
        let order = [
            Sum,
            Average,
            SampleVariance,
            SampleStandardDeviation,
            Maximum,
            Minimum,
            Count,
            PopVariance,
            PopStandardDeviation,
            DistinctCount,
            Correlation,
            Covariance,
            WeightedAvg,
            Median,
            Percentile,
            NthLargest,
            NthSmallest,
            Mode,
            NthMostFrequent,
        ];
        for (i, want) in order.iter().enumerate() {
            assert_eq!(SummaryOperation::from_code(i as i32), *want, "code {i}");
        }
        assert_eq!(SummaryOperation::from_code(19), Other(19));
        assert_eq!(SummaryOperation::from_code(-1), Other(-1));
        assert_eq!(SummaryOperation::default(), Sum);
    }

    /// `ResetConditionType::from_code`: `0..=3` map in order, else `Other`.
    #[test]
    fn reset_condition_from_code() {
        use ResetConditionType::*;
        assert_eq!(ResetConditionType::from_code(0), NoCondition);
        assert_eq!(ResetConditionType::from_code(1), OnChangeOfField);
        assert_eq!(ResetConditionType::from_code(2), OnChangeOfGroup);
        assert_eq!(ResetConditionType::from_code(3), OnFormula);
        assert_eq!(ResetConditionType::from_code(4), Other(4));
        assert_eq!(ResetConditionType::from_code(-5), Other(-5));
    }

    /// `EvaluationConditionType::from_code` shares the reset-condition numeric coding: `0..=3` in
    /// order, else `Other`.
    #[test]
    fn evaluation_condition_from_code() {
        use EvaluationConditionType::*;
        assert_eq!(EvaluationConditionType::from_code(0), NoCondition);
        assert_eq!(EvaluationConditionType::from_code(1), OnChangeOfField);
        assert_eq!(EvaluationConditionType::from_code(2), OnChangeOfGroup);
        assert_eq!(EvaluationConditionType::from_code(3), OnFormula);
        assert_eq!(EvaluationConditionType::from_code(99), Other(99));
    }

    /// `SortDirection::from_code`: `0`/`1` are ascending/descending, both `2` and `3` collapse to
    /// `NoSortOrder`, else `Other`.
    #[test]
    fn sort_direction_from_code() {
        use SortDirection::*;
        assert_eq!(SortDirection::from_code(0), AscendingOrder);
        assert_eq!(SortDirection::from_code(1), DescendingOrder);
        assert_eq!(SortDirection::from_code(2), NoSortOrder);
        assert_eq!(SortDirection::from_code(3), NoSortOrder);
        assert_eq!(SortDirection::from_code(4), Other(4));
    }

    /// `LineStyle::from_code`: `0..=6` map in order, else `Other`.
    #[test]
    fn line_style_from_code() {
        use LineStyle::*;
        let order = [
            NoLine,
            SingleLine,
            DoubleLine,
            DashLine,
            DotLine,
            FirstInvalidLineStyle,
            BlankLine,
        ];
        for (i, want) in order.iter().enumerate() {
            assert_eq!(LineStyle::from_code(i as i32), *want, "code {i}");
        }
        assert_eq!(LineStyle::from_code(7), Other(7));
    }

    /// `Alignment::from_code`: `0..=8` map in order, else `Other`.
    #[test]
    fn alignment_from_code() {
        use Alignment::*;
        let order = [
            DefaultAlign,
            LeftAlign,
            HorizontalCenterAlign,
            RightAlign,
            Justified,
            Decimal,
            TopAlign,
            VerticalCenterAlign,
            BottomAlign,
        ];
        for (i, want) in order.iter().enumerate() {
            assert_eq!(Alignment::from_code(i as i32), *want, "code {i}");
        }
        assert_eq!(Alignment::from_code(9), Other(9));
    }

    /// `FormulaVariableScope::from_code`: `0`=Shared, `1`=Global, `2`=Local, else `Other`.
    #[test]
    fn formula_variable_scope_from_code() {
        use FormulaVariableScope::*;
        assert_eq!(FormulaVariableScope::from_code(0), Shared);
        assert_eq!(FormulaVariableScope::from_code(1), Global);
        assert_eq!(FormulaVariableScope::from_code(2), Local);
        assert_eq!(FormulaVariableScope::from_code(3), Other(3));
    }

    /// `FieldValueType::from_result_kind`: scalar result kinds `1..=7` map to their value type; the
    /// non-scalar kinds (function refs, ranges, arrays) become `Other`.
    #[test]
    fn field_value_type_from_result_kind() {
        use FieldValueType::*;
        assert_eq!(FieldValueType::from_result_kind(1), Number);
        assert_eq!(FieldValueType::from_result_kind(2), Currency);
        assert_eq!(FieldValueType::from_result_kind(3), Boolean);
        assert_eq!(FieldValueType::from_result_kind(4), Date);
        assert_eq!(FieldValueType::from_result_kind(5), Time);
        assert_eq!(FieldValueType::from_result_kind(6), DateTime);
        assert_eq!(FieldValueType::from_result_kind(7), String);
        assert_eq!(FieldValueType::from_result_kind(0), Other(0));
        assert_eq!(FieldValueType::from_result_kind(8), Other(8));
        assert_eq!(FieldValueType::from_result_kind(0x1b), Other(0x1b));
    }

    /// `FieldValueType::from_code` maps the `CrFieldValueTypeEnum` codes, folds the wide numeric
    /// types `16..=18` onto `Number`, and sends unmodelled codes (`1`, `3`, `12`, negatives) to
    /// `Other`.
    #[test]
    fn field_value_type_from_code() {
        use FieldValueType::*;
        assert_eq!(FieldValueType::from_code(0), Int8s);
        assert_eq!(FieldValueType::from_code(2), Int16s);
        assert_eq!(FieldValueType::from_code(4), Int32s);
        assert_eq!(FieldValueType::from_code(5), Int32u);
        assert_eq!(FieldValueType::from_code(6), Number);
        assert_eq!(FieldValueType::from_code(7), Currency);
        assert_eq!(FieldValueType::from_code(8), Boolean);
        assert_eq!(FieldValueType::from_code(9), Date);
        assert_eq!(FieldValueType::from_code(10), Time);
        assert_eq!(FieldValueType::from_code(11), String);
        assert_eq!(FieldValueType::from_code(13), PersistentMemo);
        assert_eq!(FieldValueType::from_code(14), Blob);
        assert_eq!(FieldValueType::from_code(15), DateTime);
        // Wide numeric types collapse to Number.
        assert_eq!(FieldValueType::from_code(16), Number);
        assert_eq!(FieldValueType::from_code(17), Number);
        assert_eq!(FieldValueType::from_code(18), Number);
        // Unmodelled codes keep their raw value.
        assert_eq!(FieldValueType::from_code(1), Other(1));
        assert_eq!(FieldValueType::from_code(3), Other(3));
        assert_eq!(FieldValueType::from_code(12), Other(12));
        assert_eq!(FieldValueType::from_code(19), Other(19));
        assert_eq!(FieldValueType::from_code(-1), Other(-1));
        assert_eq!(FieldValueType::default(), Unknown);
    }

    /// `FieldValueType::is_numeric` is true for the integer/number/currency types and false for the
    /// rest (including `Unknown`/`Other`).
    #[test]
    fn field_value_type_is_numeric() {
        use FieldValueType::*;
        for t in [Int8s, Int16s, Int32s, Int32u, Number, Currency] {
            assert!(t.is_numeric(), "{t:?} should be numeric");
        }
        for t in [
            Boolean,
            String,
            Date,
            Time,
            DateTime,
            Blob,
            PersistentMemo,
            Unknown,
            Other(7),
        ] {
            assert!(!t.is_numeric(), "{t:?} should not be numeric");
        }
    }

    /// `FieldValueType::byte_length`: fixed-size types report their intrinsic length, blobs/memos
    /// report `-1`, and variable/unknown types report `None`.
    #[test]
    fn field_value_type_byte_length() {
        use FieldValueType::*;
        assert_eq!(Int8s.byte_length(), Some(1));
        assert_eq!(Int16s.byte_length(), Some(2));
        assert_eq!(Int32s.byte_length(), Some(4));
        assert_eq!(Int32u.byte_length(), Some(4));
        assert_eq!(Number.byte_length(), Some(8));
        assert_eq!(Currency.byte_length(), Some(8));
        assert_eq!(DateTime.byte_length(), Some(8));
        assert_eq!(Boolean.byte_length(), Some(2));
        assert_eq!(Date.byte_length(), Some(4));
        assert_eq!(Time.byte_length(), Some(4));
        assert_eq!(Blob.byte_length(), Some(-1));
        assert_eq!(PersistentMemo.byte_length(), Some(-1));
        assert_eq!(String.byte_length(), None);
        assert_eq!(Unknown.byte_length(), None);
        assert_eq!(Other(9).byte_length(), None);
    }

    /// `PaperOrientation::from_code`: `1`=Portrait, `2`=Landscape, `0`=default, else `Other`.
    #[test]
    fn paper_orientation_from_code() {
        use PaperOrientation::*;
        assert_eq!(PaperOrientation::from_code(0), DefaultPaperOrientation);
        assert_eq!(PaperOrientation::from_code(1), Portrait);
        assert_eq!(PaperOrientation::from_code(2), Landscape);
        assert_eq!(PaperOrientation::from_code(3), Other(3));
    }

    /// `PaperSize::from_code`: `0..=41` map to the SDK table in order; codes past it keep their raw
    /// value in `Code`.
    #[test]
    fn paper_size_from_code() {
        use PaperSize::*;
        assert_eq!(PaperSize::from_code(0), DefaultPaperSize);
        assert_eq!(PaperSize::from_code(1), PaperLetter);
        assert_eq!(PaperSize::from_code(9), PaperA4);
        assert_eq!(PaperSize::from_code(41), PaperFanfoldLegalGerman);
        assert_eq!(PaperSize::from_code(42), Code(42));
        assert_eq!(PaperSize::from_code(-1), Code(-1));
        assert_eq!(PaperSize::default(), DefaultPaperSize);
    }

    /// `PrinterDuplex::from_code`: only `1`/`2`/`3` are meaningful; every other code (including `0`
    /// and out-of-range) falls back to `Default`, not `Other`.
    #[test]
    fn printer_duplex_from_code() {
        use PrinterDuplex::*;
        assert_eq!(PrinterDuplex::from_code(1), Simplex);
        assert_eq!(PrinterDuplex::from_code(2), Vertical);
        assert_eq!(PrinterDuplex::from_code(3), Horizontal);
        assert_eq!(PrinterDuplex::from_code(0), Default);
        assert_eq!(PrinterDuplex::from_code(4), Default);
        assert_eq!(PrinterDuplex::from_code(-1), Default);
    }

    /// `PaperSource::from_code`: the `DMBIN_*` codes `1..=11`, `14`, `15` map to their variants; every
    /// other code (including `0`, `12`, `13`) keeps its raw value in `Code` — note `from_code` never
    /// yields the `Auto` default.
    #[test]
    fn paper_source_from_code() {
        use PaperSource::*;
        assert_eq!(PaperSource::from_code(1), Upper);
        assert_eq!(PaperSource::from_code(2), Lower);
        assert_eq!(PaperSource::from_code(3), Middle);
        assert_eq!(PaperSource::from_code(4), Manual);
        assert_eq!(PaperSource::from_code(5), Envelope);
        assert_eq!(PaperSource::from_code(6), EnvManual);
        assert_eq!(PaperSource::from_code(7), Auto);
        assert_eq!(PaperSource::from_code(8), Tractor);
        assert_eq!(PaperSource::from_code(9), SmallFmt);
        assert_eq!(PaperSource::from_code(10), LargeFmt);
        assert_eq!(PaperSource::from_code(11), LargeCapacity);
        assert_eq!(PaperSource::from_code(14), Cassette);
        assert_eq!(PaperSource::from_code(15), FormSource);
        // Gaps and out-of-range keep the raw code.
        assert_eq!(PaperSource::from_code(0), Code(0));
        assert_eq!(PaperSource::from_code(12), Code(12));
        assert_eq!(PaperSource::from_code(13), Code(13));
        assert_eq!(PaperSource::from_code(99), Code(99));
        // The struct default is Auto even though code 0 is not Auto.
        assert_eq!(PaperSource::default(), Auto);
    }

    /// `TableJoinType::from_code`: `1..=5` and `8` map to their join variants; the gaps (`6`, `7`)
    /// and out-of-range codes become `Other`.
    #[test]
    fn table_join_type_from_code() {
        use TableJoinType::*;
        assert_eq!(TableJoinType::from_code(1), Equal);
        assert_eq!(TableJoinType::from_code(2), LeftOuter);
        assert_eq!(TableJoinType::from_code(3), RightOuter);
        assert_eq!(TableJoinType::from_code(4), GreaterThan);
        assert_eq!(TableJoinType::from_code(5), LessThan);
        assert_eq!(TableJoinType::from_code(8), NotEqual);
        assert_eq!(TableJoinType::from_code(6), Other(6));
        assert_eq!(TableJoinType::from_code(7), Other(7));
        assert_eq!(TableJoinType::from_code(0), Other(0));
    }
}
