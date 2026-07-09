//! Object formatting DTOs (SDK: `IObjectFormat`, `IBorder`, `IFont`, `IFieldFormat`).

use super::enums::{
    Alignment, BooleanOutputType, CurrencySymbolFormat, DateSystemDefaultType, DayFormat,
    DayOfWeekFormat, HyperlinkType, LineStyle, MonthFormat, NegativeFormat, RoundingFormat,
    TextRotationAngle, YearFormat,
};
use super::primitives::{Color, Conditioned};

/// SDK: `IObjectFormat` (XML `<ObjectFormat>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ObjectFormat {
    /// SDK `EnableSuppress` — hides the object, optionally under a conditional formula.
    pub suppress: Conditioned<bool>,
    /// SDK `EnableCanGrow` — lets a text object's height expand to fit its content.
    pub can_grow: bool,
    /// SDK `EnableKeepTogether` — keeps the object from splitting across a page break.
    pub keep_together: bool,
    /// SDK `EnableCloseAtPageBreak` — closes/ends the object at a page break.
    pub close_at_page_break: bool,
    /// SDK `HorizontalAlignment` — the object's horizontal text alignment.
    pub horizontal_alignment: Alignment,
    /// SDK `CssClass` — the CSS class name applied when exporting to HTML.
    pub css_class: Option<String>,
    /// SDK `Hyperlink` — the object's drill-down/navigation hyperlink, if any.
    pub hyperlink: Option<Hyperlink>,
    /// SDK `ToolTipText` — the tooltip text shown when hovering the object.
    pub tooltip_text: Option<String>,
    /// SDK `TextRotationAngle` — the object's text rotation (0°, 90°, or 270°).
    pub text_rotation: TextRotationAngle,
    /// Conditional-format formulas attached to this object, as `(attribute name, formula text)`
    /// pairs in `<ObjectFormatConditionFormulas>` emit order (e.g. `("EnableSuppress", "…")`).
    pub condition_formulas: Vec<(String, String)>,
}

/// SDK: object hyperlink (HyperlinkText/HyperlinkType).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Hyperlink {
    /// SDK `HyperlinkText` — the link target (URL, formula text, or destination name).
    pub text: String,
    /// SDK `HyperlinkType` — what `text` represents (e.g. URL, email, report part).
    pub kind: HyperlinkType,
}

/// SDK: `IBorder` (XML `<Border>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Border {
    /// SDK `TopLineStyle` — the top edge's line style.
    pub top: LineStyle,
    /// SDK `BottomLineStyle` — the bottom edge's line style.
    pub bottom: LineStyle,
    /// SDK `LeftLineStyle` — the left edge's line style.
    pub left: LineStyle,
    /// SDK `RightLineStyle` — the right edge's line style.
    pub right: LineStyle,
    /// SDK `HasDropShadow` — draws a drop shadow behind the object.
    pub has_drop_shadow: bool,
    /// SDK `BorderColor` — the border line color.
    pub border_color: Option<Color>,
    /// SDK `BackgroundColor` — the object's background fill color.
    pub background_color: Option<Color>,
    /// SDK `EnableTightHorizontal` — removes inner horizontal padding between border and content.
    pub tight_horizontal: bool,
    /// Conditional-format formulas for the border, as `(attribute name, formula text)` pairs in
    /// `<BorderConditionFormulas>` emit order (e.g. `("BackgroundColor", "…")`,
    /// `("BorderColor", "…")`).
    pub condition_formulas: Vec<(String, String)>,
}

/// SDK: `IFont` (XML `<Font>`).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Font {
    /// SDK `Name` — the font face name (e.g. `"Arial"`).
    pub name: String,
    /// SDK `Size`/`SizeinPoints` — the font size, in points.
    pub size_pt: f32,
    /// SDK `Bold` — bold weight.
    pub bold: bool,
    /// SDK `Italic` — italic style.
    pub italic: bool,
    /// SDK `Underline` — underline decoration.
    pub underline: bool,
    /// SDK `Strikeout` — strikethrough decoration.
    pub strikethrough: bool,
    /// 400 = normal, 700 = bold.
    pub weight: i32,
    /// SDK `GdiCharSet` — the GDI character set code.
    pub charset: i16,
}

/// SDK: `IFontColor` (XML `<Color>`).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FontColor {
    /// SDK `Color` — the text/foreground color.
    pub color: Color,
    /// SDK `Font` — the font definition (face, size, style).
    pub font: Font,
    /// Conditional-format formulas for the font, as `(attribute name, formula text)` pairs in
    /// `<FontColorConditionFormulas>` emit order (e.g. `("Color", "…")`, `("Style", "…")`).
    pub condition_formulas: Vec<(String, String)>,
}

/// SDK: `IFieldFormat` — the type-specific display formatting of a field object.
///
/// The **byte-derived** sub-formats are stored here: [`CommonFieldFormat`], [`NumericFieldFormat`],
/// [`BooleanFieldFormat`], the (unvalidated) [`StringFieldFormat`], and the per-field **stored**
/// [`DateFieldFormat`].
///
/// The stored date format really does vary per field — its `dayType`/`monthType`/`yearType` enums are
/// decoded into [`DateFieldFormat`]. The engine, however, only *reports* this stored format verbatim
/// for a **date-valued** field with `EnableUseSystemDefaults == false`; for a system-default field, or
/// a non-date field, it resolves the effective date format at runtime from the field's value type
/// (and, for a date field's `windowsDefaultType`, the host locale). That resolution is the
/// derive-layer's job (the XML exporter's effective-format derivation), not a stored fact — the same
/// boundary as `NumberOfBytes`. The **time** sub-format stays fully runtime-resolved (host-locale
/// gated even for non-system-default fields), so it is *not* modelled as a stored fact.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FieldFormat {
    /// SDK `CommonFormat` — options common to all field types.
    pub common: CommonFieldFormat,
    /// SDK `NumericFormat` — the number format, applies to Number/Currency fields.
    pub numeric: NumericFieldFormat,
    /// SDK `BooleanFormat` — the boolean output format, applies to Boolean fields.
    pub boolean: BooleanFieldFormat,
    /// SDK `StringFormat` — the string format, decoded but not exported (see struct docs).
    pub string: StringFieldFormat,
    /// The per-field **stored** date format. Only meaningful (reported by the engine verbatim) for a
    /// date-valued field with `EnableUseSystemDefaults == false`; otherwise the runtime-resolved
    /// effective format wins — resolved by the XML exporter's effective-format derivation.
    pub date: DateFieldFormat,
}

/// SDK: `IDateFieldFormat` (XML `<DateFieldFormat>`) — the **stored** per-field date format. Only the
/// three elements the SDK exposes (day / month / year) are modelled here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DateFieldFormat {
    /// SDK `DayFormat` — the day-of-month display style.
    pub day: DayFormat,
    /// SDK `MonthFormat` — the month display style (numeric, short/long name).
    pub month: MonthFormat,
    /// SDK `YearFormat` — the year display style (2-digit or 4-digit).
    pub year: YearFormat,
    /// SDK `SystemDefaultType`. When not `NotUsingWindowsDefaults`, the engine renders the field's
    /// date with the host's Windows long/short date pattern, overriding the stored day/month/year —
    /// so the derive layer must resolve the effective format from this + the host locale, not report
    /// the stored enums verbatim.
    pub system_default: DateSystemDefaultType,
    /// SDK `DayOfWeekType` — the weekday element of the date. Not exported, so decoded for record
    /// completeness only.
    pub day_of_week: DayOfWeekFormat,
}

/// SDK: `ICommonFieldFormat` — options common to all field formats.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CommonFieldFormat {
    /// XML `EnableSuppressIfDuplicated`.
    pub suppress_if_duplicated: bool,
    /// XML `EnableUseSystemDefaults`.
    pub use_system_defaults: bool,
}

/// SDK: `INumericFieldFormat` — the field's stored number format: [`NegativeFormat`], decimal places,
/// [`RoundingFormat`], and [`CurrencySymbolFormat`]. `EnableUseLeadingZero` is *not* stored — the
/// engine derives it from the field's value type — so the exporter resolves it there.
///
/// The separator symbols below (decimal / thousand / currency) are also stored, but not exported, so
/// they are decoded for record completeness only.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NumericFieldFormat {
    /// SDK `NDecimalPlaces` — the number of decimal places to display.
    pub decimal_places: i32,
    /// SDK `RoundingFormat` — the rounding rule applied to the displayed value.
    pub rounding: RoundingFormat,
    /// SDK `NegativeFormat` — how negative values are displayed (sign/parens position).
    pub negative: NegativeFormat,
    /// SDK `CurrencySymbolFormat` — whether/where a currency symbol is shown.
    pub currency_symbol: CurrencySymbolFormat,
    /// SDK `DecimalSymbol` — the decimal separator string (e.g. `"."`).
    pub decimal_symbol: String,
    /// SDK `ThousandSymbol` — the thousands separator string (e.g. `","`).
    pub thousand_symbol: String,
    /// SDK `CurrencySymbol` — the currency symbol string (e.g. `"kr "`); empty when there is none.
    pub currency_symbol_text: String,
}

impl Default for NumericFieldFormat {
    /// The engine's generic default number format (2 decimals, round to hundredth, leading minus,
    /// no currency symbol) — what a non-numeric field reports.
    fn default() -> Self {
        Self {
            decimal_places: 2,
            rounding: RoundingFormat::RoundToHundredth,
            negative: NegativeFormat::LeadingMinus,
            currency_symbol: CurrencySymbolFormat::NoSymbol,
            decimal_symbol: String::new(),
            thousand_symbol: String::new(),
            currency_symbol_text: String::new(),
        }
    }
}

/// SDK: `IBooleanFieldFormat` — the boolean [`BooleanOutputType`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BooleanFieldFormat {
    /// SDK `BooleanOutputFormat` — how a boolean value is rendered (e.g. "True/False", "Y/N").
    pub output_type: BooleanOutputType,
}

/// SDK: `IStringFieldFormat`. Decoded for record coverage but the engine's managed `FieldFormat`
/// wrapper exposes no string sub-format, so it is not exported.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StringFieldFormat {}
