//! Object formatting DTOs (SDK: `IObjectFormat`, `IBorder`, `IFont`, `IFieldFormat`).

use super::enums::{Alignment, HyperlinkType, LineStyle, TextRotationAngle};
use super::primitives::{Color, Conditioned};

/// SDK: `IObjectFormat` (XML `<ObjectFormat>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ObjectFormat {
    pub suppress: Conditioned<bool>,
    pub can_grow: bool,
    pub keep_together: bool,
    pub close_at_page_break: bool,
    pub horizontal_alignment: Alignment,
    pub css_class: Option<String>,
    pub hyperlink: Option<Hyperlink>,
    pub tooltip_text: Option<String>,
    pub text_rotation: TextRotationAngle,
    /// Conditional-format formulas attached to this object, as `(attribute name, formula text)`
    /// pairs in `<ObjectFormatConditionFormulas>` emit order (e.g. `("EnableSuppress", "…")`).
    pub condition_formulas: Vec<(String, String)>,
}

/// SDK: object hyperlink (HyperlinkText/HyperlinkType).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Hyperlink {
    pub text: String,
    pub kind: HyperlinkType,
}

/// SDK: `IBorder` (XML `<Border>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Border {
    pub top: LineStyle,
    pub bottom: LineStyle,
    pub left: LineStyle,
    pub right: LineStyle,
    pub has_drop_shadow: bool,
    pub border_color: Option<Color>,
    pub background_color: Option<Color>,
    pub tight_horizontal: bool,
    /// Conditional-format formulas for the border, as `(attribute name, formula text)` pairs in
    /// `<BorderConditionFormulas>` emit order (e.g. `("BackgroundColor", "…")`,
    /// `("BorderColor", "…")`). From the `0xed` wrapper that parents the `0xec` border.
    pub condition_formulas: Vec<(String, String)>,
}

/// SDK: `IFont` (XML `<Font>`).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Font {
    pub name: String,
    pub size_pt: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    /// 400 = normal, 700 = bold.
    pub weight: i32,
    pub charset: i16,
}

/// SDK: `IFontColor` (XML `<Color>`).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FontColor {
    pub color: Color,
    pub font: Font,
    /// Conditional-format formulas for the font, as `(attribute name, formula text)` pairs in
    /// `<FontColorConditionFormulas>` emit order (e.g. `("Color", "…")`, `("Style", "…")`).
    pub condition_formulas: Vec<(String, String)>,
}

/// SDK: `IFieldFormat` — selector of one type-specific sub-format (sub-formats deferred).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct FieldFormat {
    pub common: CommonFieldFormat,
    pub numeric: Option<NumericFieldFormat>,
    pub date: Option<DateFieldFormat>,
    pub time: Option<TimeFieldFormat>,
    pub string: Option<StringFieldFormat>,
    pub boolean: Option<BooleanFieldFormat>,
}

/// SDK: `ICommonFieldFormat` — options common to all field formats.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CommonFieldFormat {
    pub suppress_if_duplicated: bool,
    pub system_default: bool,
}

macro_rules! empty_format {
    ($($(#[$m:meta])* $name:ident),+ $(,)?) => {$(
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Default)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        #[non_exhaustive]
        pub struct $name {}
    )+};
}

empty_format!(
    /// SDK: `INumericFieldFormat` (members deferred).
    NumericFieldFormat,
    /// SDK: `IDateFieldFormat` (deferred).
    DateFieldFormat,
    /// SDK: `ITimeFieldFormat` (deferred).
    TimeFieldFormat,
    /// SDK: `IStringFieldFormat` (deferred).
    StringFieldFormat,
    /// SDK: `IBooleanFieldFormat` (deferred).
    BooleanFieldFormat,
);
