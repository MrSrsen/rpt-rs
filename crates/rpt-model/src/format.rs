//! Object formatting DTOs (SDK: `IObjectFormat`, `IBorder`, `IFont`, `IFieldFormat`).

use super::enums::{
    Alignment, BooleanOutputType, CurrencyPosition, CurrencySymbolFormat, DateOrder,
    DateSystemDefaultType, DateTimeOrder, DayFormat, DayOfWeekFormat, HourFormat, HyperlinkType,
    LineStyle, MinuteFormat, MonthFormat, NegativeFormat, ReadingOrder, RoundingFormat,
    SecondFormat, TextFormat, TextRotationAngle, VerticalAlignment, YearFormat,
};
use super::primitives::{Color, Conditioned};

/// SDK: `IObjectFormat`.
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
    /// SDK `VerticalAlignment` — the object's vertical text alignment within its box. Stored in the
    /// `0x00fc` ObjectFormat leaf (byte 3); defaults to [`VerticalAlignment::Top`].
    pub vertical_alignment: VerticalAlignment,
    /// SDK `CssClass` — the CSS class name applied when exporting to HTML.
    pub css_class: Option<String>,
    /// SDK `Hyperlink` — the object's drill-down/navigation hyperlink, if any.
    pub hyperlink: Option<Hyperlink>,
    /// SDK `ToolTipText` — the tooltip text shown when hovering the object.
    pub tooltip_text: Option<String>,
    /// SDK `TextRotationAngle` — the object's text rotation (0°, 90°, or 270°).
    pub text_rotation: TextRotationAngle,
    /// Conditional-format formulas attached to this object, as `(reserved formula name, formula
    /// text)` pairs in record order (e.g. `("Object_Visibility", "…")`, `("Display_String", "…")`).
    /// The key is the stored Crystal reserved formula name, not any output-surface attribute name.
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

/// SDK: `IBorder`.
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
    /// Conditional-format formulas for the border, as `(reserved formula name, formula text)` pairs
    /// in record order (e.g. `("Back_Color", "…")`, `("Fore_Color", "…")`). The key is the stored
    /// Crystal reserved formula name, not any output-surface attribute name.
    pub condition_formulas: Vec<(String, String)>,
}

/// SDK: `IFont`.
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

/// SDK: `IFontColor`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FontColor {
    /// SDK `Color` — the text/foreground color.
    pub color: Color,
    /// SDK `Font` — the font definition (face, size, style).
    pub font: Font,
    /// Conditional-format formulas for the font, as `(reserved formula name, formula text)` pairs
    /// in record order (e.g. `("Font_Color", "…")`, `("Font_Style", "…")`). The key is the stored
    /// Crystal reserved formula name, not any output-surface attribute name.
    pub condition_formulas: Vec<(String, String)>,
}

/// SDK: `IFieldFormat` — the type-specific display formatting of a field object.
///
/// The **byte-derived** sub-formats are stored here: [`CommonFieldFormat`], [`NumericFieldFormat`],
/// [`BooleanFieldFormat`], [`StringFieldFormat`], and the per-field **stored** [`DateFieldFormat`],
/// [`TimeFieldFormat`], and [`DateTimeFieldFormat`].
///
/// The stored date format really does vary per field — its `dayType`/`monthType`/`yearType` enums are
/// decoded into [`DateFieldFormat`] (and, likewise, the time elements into [`TimeFieldFormat`]). The
/// engine, however, only *reports* this stored format verbatim for an explicit field with
/// `EnableUseSystemDefaults == false`; for a system-default field, or a value type the format doesn't
/// apply to, it resolves the effective format at runtime from the field's value type (and, for a date
/// field's `windowsDefaultType`, the host locale). That resolution belongs to the consumer that needs
/// it — `rpt-layout` does it for rendering — not to the decoder: it is not a stored fact, the same
/// boundary as a formula's runtime `NumberOfBytes`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FieldFormat {
    /// SDK `CommonFormat` — options common to all field types.
    pub common: CommonFieldFormat,
    /// SDK `NumericFormat` — the number format the engine reports for this field. Each field stores
    /// *two* numeric-format slots (a currency-format slot and a number-format slot); the engine
    /// surfaces the currency slot for a Currency-valued field and the number slot otherwise. This
    /// holds the number slot as decoded; the value-type resolution pass swaps in
    /// [`currency_numeric`](Self::currency_numeric) for Currency-valued fields, so this ends up
    /// holding the reported format.
    pub numeric: NumericFieldFormat,
    /// The stored **currency-format** numeric slot (the first `0x00f8` of the pair). A stored fact
    /// used only to resolve [`numeric`](Self::numeric) for Currency-valued fields; not part of the
    /// exported surface.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub currency_numeric: NumericFieldFormat,
    /// SDK `BooleanFormat` — the boolean output format, applies to Boolean fields.
    pub boolean: BooleanFieldFormat,
    /// SDK `StringFormat` — the string format (text-format / word-wrap / reading-order / max-lines).
    pub string: StringFieldFormat,
    /// The per-field **stored** date format. Only meaningful (reported by the engine verbatim) for a
    /// date-valued field with `EnableUseSystemDefaults == false`; otherwise the runtime-resolved
    /// effective format wins, resolved by the consumer (`rpt-layout` for the render path).
    pub date: DateFieldFormat,
    /// The per-field **stored** time format (hour / minute / second element display). Meaningful only
    /// for an explicit (non-system-default) time/datetime field; system-default fields resolve the
    /// effective format from the host locale (as do `TimeBase`/`AMString`/… which are not stored in
    /// the leaf at all).
    pub time: TimeFieldFormat,
    /// The per-field **stored** date-time format: its `DateTimeOrder` (which of the date/time parts
    /// show, and in what order) and `DateTimeSeparator` (the string placed between them).
    pub date_time: DateTimeFieldFormat,
}

/// SDK: `IDateTimeFieldFormat` — the **stored** per-field date-time format. The nested date and time
/// renderings are resolved at runtime from the host locale; only the two stored facts the SDK
/// surfaces at this level are modelled: `DateTimeOrder` and `DateTimeSeparator`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DateTimeFieldFormat {
    /// SDK `DateTimeOrder` — which of the date/time parts is shown and in what order (date-only,
    /// date-then-time, …). Stored in the `0x00f4` leaf byte 0.
    pub order: DateTimeOrder,
    /// SDK `DateTimeSeparator` — the string placed between the date and time parts (e.g. `"  "`).
    pub separator: String,
}

/// SDK: `ITimeFieldFormat` — the **stored** per-field time format. Only the hour/minute/second
/// element-display enums are modelled: they are stored in the `0x00f6` leaf (bytes 2/3/4) and are a
/// genuine per-field fact on an explicit (non-system-default) field. The rest of the SDK's
/// `TimeFieldFormat` surface (`TimeBase`, `AMString`/`PMString`, `AMPMFormat`, the hour/minute/second
/// separators) is **not** in the stored leaf — the engine resolves it at runtime from the host
/// locale — so it is not modelled as a stored fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TimeFieldFormat {
    /// SDK `HourFormat` — the hour element's display style.
    pub hour: HourFormat,
    /// SDK `MinuteFormat` — the minute element's display style.
    pub minute: MinuteFormat,
    /// SDK `SecondFormat` — the second element's display style.
    pub second: SecondFormat,
}

/// SDK: `IDateFieldFormat` — the **stored** per-field date format. Only the
/// three elements the SDK exposes (day / month / year) are modelled here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DateFieldFormat {
    /// SDK `DateOrder` — the relative order of the day/month/year elements. Stored in the `0x00f2`
    /// leaf byte 0.
    pub date_order: DateOrder,
    /// SDK `DayFormat` — the day-of-month display style.
    pub day: DayFormat,
    /// SDK `MonthFormat` — the month display style (numeric, short/long name).
    pub month: MonthFormat,
    /// SDK `YearFormat` — the year display style (2-digit or 4-digit).
    pub year: YearFormat,
    /// SDK `SystemDefaultType`. When not `NotUsingWindowsDefaults`, the engine renders the field's
    /// date with the host's Windows long/short date pattern, overriding the stored day/month/year —
    /// so a consumer that needs the displayed format must resolve it from this + the host locale
    /// rather than reading the stored enums verbatim.
    pub system_default: DateSystemDefaultType,
    /// SDK `DayOfWeekType` — the weekday element of the date. Not exported, so decoded for record
    /// completeness only.
    pub day_of_week: DayOfWeekFormat,
}

/// SDK: `ICommonFieldFormat` — options common to all field formats.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CommonFieldFormat {
    /// SDK `EnableSuppressIfDuplicated`.
    pub suppress_if_duplicated: bool,
    /// SDK `EnableUseSystemDefaults`.
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
    /// SDK `CurrencyPosition` — where the currency symbol sits relative to the value and the
    /// negative sign/brackets. Stored at leaf byte 13.
    pub currency_position: CurrencyPosition,
    /// SDK `ThousandsSeparator` — whether the thousands grouping separator is shown. Stored at leaf
    /// byte 4 (default `true`).
    pub thousands_separator: bool,
    /// SDK `EnableSuppressIfZero` — hide the field when its value is zero. Stored at leaf byte 1.
    pub suppress_if_zero: bool,
    /// SDK `DecimalSymbol` — the decimal separator string (e.g. `"."`).
    pub decimal_symbol: String,
    /// SDK `ThousandSymbol` — the thousands separator string (e.g. `","`).
    pub thousand_symbol: String,
    /// SDK `CurrencySymbol` — the currency symbol string (e.g. `"kr "`); empty when there is none.
    pub currency_symbol_text: String,
}

impl Default for NumericFieldFormat {
    /// The engine's generic default number format (2 decimals, round to hundredth, leading minus,
    /// no currency symbol, thousands separator on, leading currency inside the negative) — what a
    /// non-numeric field reports.
    fn default() -> Self {
        Self {
            decimal_places: 2,
            rounding: RoundingFormat::RoundToHundredth,
            negative: NegativeFormat::LeadingMinus,
            currency_symbol: CurrencySymbolFormat::NoSymbol,
            currency_position: CurrencyPosition::LeadingCurrencyInsideNegative,
            thousands_separator: true,
            suppress_if_zero: false,
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

/// SDK: `IStringFieldFormat` — a string field's stored layout format. Decoded from the `0x00fa` leaf:
/// `EnableWordWrap` (byte 0), the three indent longs (bytes 1-12, `u32` BE, into
/// [`indent`](StringFieldFormat::indent)),
/// `MaxNumberOfLines` (bytes 13-14, `u16` BE), `TextFormat` (byte 15), `ReadingOrder` (byte 16).
/// These are genuine stored facts on every string field (not runtime-resolved like the date/time
/// effective formats). `TextFormat` is the render-relevant one (plain / HTML / RTF interpretation).
/// The leaf's trailing spacing members (`LineSpacing` at bytes 17-20 as a 16.16 fixed-point multiple —
/// `0x00010000` = `1.0`; `CharacterSpacing` at bytes 21-24; `LineSpacingType` at byte 25 — `0` =
/// `crLineSpacingTypeMultiple`) are invariant and not modelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StringFieldFormat {
    /// SDK `TextFormat` — how the text is interpreted when rendered (plain / HTML / RTF).
    pub text_format: TextFormat,
    /// SDK `EnableWordWrap` — whether the text wraps within the object.
    pub enable_word_wrap: bool,
    /// SDK `MaxNumberOfLines` — the maximum number of lines to display (`0` = unlimited).
    pub max_number_of_lines: u16,
    /// SDK `ReadingOrder` — the text reading order (left-to-right / right-to-left).
    pub reading_order: ReadingOrder,
    /// SDK `IndentAndSpacingFormat` — the field's left/right/first-line indentation, in twips.
    /// Only `right_indent` is ever non-zero on a placed field.
    pub indent: crate::objects::IndentAndSpacingFormat,
}
