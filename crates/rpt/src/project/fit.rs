//! Fitting enums: mapping low-level engine codes (read from the record substrate) onto the
//! high-level [`crate::model`] enum variants.
//!
//! These live in the projection layer rather than [`crate::model`] (which holds domain
//! structures only). They are inherent `from_code` methods so call sites stay ergonomic
//! (`FieldValueType::from_code(..)`).

use crate::model::{
    Alignment, FieldValueType, LineStyle, ResetConditionType, SortDirection, SummaryOperation,
    TableJoinType,
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

impl crate::model::EvaluationConditionType {
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

impl FieldValueType {
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

impl crate::model::PaperOrientation {
    /// Map the page-setup orientation code (`0x07` u16[2]) to the variant.
    pub fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Portrait,
            2 => Self::Landscape,
            0 => Self::DefaultPaperOrientation,
            other => Self::Other(other),
        }
    }
}

impl crate::model::PaperSize {
    /// Map the page-setup paper-size code (`0x07` u16[3], a Windows `DMPAPER_*` value) to the
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

impl crate::model::PrinterDuplex {
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

impl crate::model::PaperSource {
    /// Map the page-setup paper-source code (`0x07` u16[5], a Windows `DMBIN_*` value) to the
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
