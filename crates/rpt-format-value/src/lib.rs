//! # Value formatting — the icu-equivalent layer
//!
//! Turns a typed value + a [`FormatSpec`] into the display string Crystal would produce.
//! Structs mirror the SDK `INumericFieldFormat`/`IDateFieldFormat`/…
//! members, so a decoded per-field `FieldFormat` leaf maps straight onto these specs; the type
//! [`Default`]s reproduce Crystal's en-US defaults.
//!
//! The crate is Value-agnostic (it formats `f64`/[`Date`]/[`Time`]/`bool` + a spec), so it sits
//! *below* the formula evaluator with no circular dependency — the evaluator re-exports
//! [`Date`]/[`Time`] from here and calls these functions for its `ToText`/`&` coercions.
//!
//! ## Locale
//!
//! [`Locale`] carries the concrete separators, month/day names, and AM/PM designators for a set of
//! built-in locales (en-US/en-GB/de-DE/fr-FR/es-ES/it-IT + en-US fallback). The `*_in` formatters
//! ([`format_date_in`], [`format_time_in`], [`Locale::number_format`]) take names/separators from a
//! [`Locale`]; the plain `format_date`/`format_number` entry points use the en-US baseline. The
//! render layer (`rpt-layout`) merges a [`Locale`] with each field's stored format leaf to pick the
//! effective display format.

mod civil;
mod picture;

pub use civil::{Date, Time};
pub use picture::parse_number_picture;

/// How negative numbers/currency are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum NegativeStyle {
    /// `-1234.00`
    #[default]
    LeadingMinus,
    /// `(1234.00)`
    Parens,
    /// `1234.00-`
    TrailingMinus,
}

/// Where a currency symbol sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CurrencyPosition {
    /// `$1,234.00`
    #[default]
    LeadingNoSpace,
    /// `$ 1,234.00`
    LeadingSpace,
    /// `1,234.00$`
    TrailingNoSpace,
    /// `1,234.00 $`
    TrailingSpace,
}

/// The order a locale lays out a date's day/month/year components (native `RDDateOrder`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DateOrder {
    /// `2004-01-03`
    YearMonthDay,
    /// `03/01/2004`
    DayMonthYear,
    /// `1/3/2004`
    #[default]
    MonthDayYear,
}

/// A render locale: the concrete separators, month/day **names**, AM/PM designators, default date
/// order/separator, and currency symbol a value's display string is built from.
///
/// This is the "system default" layer of Crystal's two-layer format resolution: a field's stored
/// [`crate`]-agnostic format leaf supplies the explicit *choices* (decimals, which components, 12/24h)
/// while the locale supplies the *names and separators* — see the `rpt-layout` format resolver, which
/// merges the two. All fields are `&'static` so a [`Locale`] is a cheap `Copy` handle into a built-in
/// table (see [`Locale::lookup`]); there is no allocation and no serde (it is a runtime render input,
/// not a stored fact).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locale {
    /// BCP-47-ish tag this locale answers to (`"en-US"`, `"de-DE"`, …).
    pub tag: &'static str,
    /// Decimal separator (`.` in en-US, `,` in de-DE).
    pub decimal_sep: char,
    /// Thousands (grouping) separator (`,` in en-US, `.` in de-DE, ` ` in fr-FR).
    pub thousands_sep: char,
    /// Separator between a system-default date's components (`/` in en-US, `.` in de-DE).
    pub date_sep: char,
    /// Component order for a system-default date.
    pub date_order: DateOrder,
    /// Full month names, January … December.
    pub months: &'static [&'static str; 12],
    /// Abbreviated month names, Jan … Dec.
    pub months_abbrev: &'static [&'static str; 12],
    /// Full weekday names, Sunday … Saturday.
    pub days: &'static [&'static str; 7],
    /// Abbreviated weekday names, Sun … Sat.
    pub days_abbrev: &'static [&'static str; 7],
    /// `true` => 12-hour clock with AM/PM; `false` => 24-hour (AM/PM designators unused).
    pub twelve_hour: bool,
    /// AM designator (`"AM"`); rendered by the `tt` token.
    pub am: &'static str,
    /// PM designator (`"PM"`).
    pub pm: &'static str,
    /// Default currency symbol (`"$"`, `"€"`, `"£"`).
    pub currency_symbol: &'static str,
    /// Where the currency symbol sits by default.
    pub currency_position: CurrencyPosition,
    /// Decimal places a value type formats with by default (Crystal's en-US default is `2`).
    pub default_decimals: u32,
}

impl Default for Locale {
    fn default() -> Locale {
        EN_US
    }
}

impl Locale {
    /// Look up a built-in locale by BCP-47-ish tag (case-insensitive), trying the full `ll-CC` tag
    /// then the language subtag alone (`"de"` → `de-DE`). `None` for an unsupported tag — callers
    /// (the CLI) warn and fall back via [`Locale::from_tag`].
    pub fn lookup(tag: &str) -> Option<Locale> {
        let t = tag.trim().replace('_', "-");
        let lang = t.split('-').next().unwrap_or("").to_ascii_lowercase();
        BUILTIN
            .iter()
            .find(|l| l.tag.eq_ignore_ascii_case(&t))
            .or_else(|| BUILTIN.iter().find(|l| lang_of(l.tag) == lang))
            .copied()
    }

    /// Like [`Locale::lookup`] but falls back to en-US for an unsupported tag (documented default).
    pub fn from_tag(tag: &str) -> Locale {
        Locale::lookup(tag).unwrap_or(EN_US)
    }

    /// A [`NumberFormat`] carrying this locale's separators (2 decimals, grouped) — the numeric
    /// "system default" baseline the field format then overlays.
    pub fn number_format(&self) -> NumberFormat {
        NumberFormat {
            decimals: self.default_decimals,
            use_thousands: true,
            thousands_sep: self.thousands_sep,
            decimal_sep: self.decimal_sep,
            negative: NegativeStyle::LeadingMinus,
            leading_zero: true,
        }
    }
}

fn lang_of(tag: &str) -> String {
    tag.split('-').next().unwrap_or("").to_ascii_lowercase()
}

const EN_MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const EN_MONTHS_ABBR: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const EN_DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const EN_DAYS_ABBR: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

const DE_MONTHS: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];
const DE_MONTHS_ABBR: [&str; 12] = [
    "Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
];
const DE_DAYS: [&str; 7] = [
    "Sonntag",
    "Montag",
    "Dienstag",
    "Mittwoch",
    "Donnerstag",
    "Freitag",
    "Samstag",
];
const DE_DAYS_ABBR: [&str; 7] = ["So", "Mo", "Di", "Mi", "Do", "Fr", "Sa"];

const FR_MONTHS: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];
const FR_MONTHS_ABBR: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];
const FR_DAYS: [&str; 7] = [
    "dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi",
];
const FR_DAYS_ABBR: [&str; 7] = ["dim.", "lun.", "mar.", "mer.", "jeu.", "ven.", "sam."];

const ES_MONTHS: [&str; 12] = [
    "enero",
    "febrero",
    "marzo",
    "abril",
    "mayo",
    "junio",
    "julio",
    "agosto",
    "septiembre",
    "octubre",
    "noviembre",
    "diciembre",
];
const ES_MONTHS_ABBR: [&str; 12] = [
    "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic",
];
const ES_DAYS: [&str; 7] = [
    "domingo",
    "lunes",
    "martes",
    "miércoles",
    "jueves",
    "viernes",
    "sábado",
];
const ES_DAYS_ABBR: [&str; 7] = ["dom", "lun", "mar", "mié", "jue", "vie", "sáb"];

const IT_MONTHS: [&str; 12] = [
    "gennaio",
    "febbraio",
    "marzo",
    "aprile",
    "maggio",
    "giugno",
    "luglio",
    "agosto",
    "settembre",
    "ottobre",
    "novembre",
    "dicembre",
];
const IT_MONTHS_ABBR: [&str; 12] = [
    "gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic",
];
const IT_DAYS: [&str; 7] = [
    "domenica",
    "lunedì",
    "martedì",
    "mercoledì",
    "giovedì",
    "venerdì",
    "sabato",
];
const IT_DAYS_ABBR: [&str; 7] = ["dom", "lun", "mar", "mer", "gio", "ven", "sab"];

/// The en-US baseline locale (the documented fallback for an unsupported tag).
pub const EN_US: Locale = Locale {
    tag: "en-US",
    decimal_sep: '.',
    thousands_sep: ',',
    date_sep: '/',
    date_order: DateOrder::MonthDayYear,
    months: &EN_MONTHS,
    months_abbrev: &EN_MONTHS_ABBR,
    days: &EN_DAYS,
    days_abbrev: &EN_DAYS_ABBR,
    twelve_hour: true,
    am: "AM",
    pm: "PM",
    currency_symbol: "$",
    currency_position: CurrencyPosition::LeadingNoSpace,
    default_decimals: 2,
};

const EN_GB: Locale = Locale {
    tag: "en-GB",
    decimal_sep: '.',
    thousands_sep: ',',
    date_sep: '/',
    date_order: DateOrder::DayMonthYear,
    months: &EN_MONTHS,
    months_abbrev: &EN_MONTHS_ABBR,
    days: &EN_DAYS,
    days_abbrev: &EN_DAYS_ABBR,
    twelve_hour: false,
    am: "AM",
    pm: "PM",
    currency_symbol: "£",
    currency_position: CurrencyPosition::LeadingNoSpace,
    default_decimals: 2,
};

const DE_DE: Locale = Locale {
    tag: "de-DE",
    decimal_sep: ',',
    thousands_sep: '.',
    date_sep: '.',
    date_order: DateOrder::DayMonthYear,
    months: &DE_MONTHS,
    months_abbrev: &DE_MONTHS_ABBR,
    days: &DE_DAYS,
    days_abbrev: &DE_DAYS_ABBR,
    twelve_hour: false,
    am: "",
    pm: "",
    currency_symbol: "€",
    currency_position: CurrencyPosition::TrailingSpace,
    default_decimals: 2,
};

const FR_FR: Locale = Locale {
    tag: "fr-FR",
    decimal_sep: ',',
    thousands_sep: '\u{202f}', // narrow no-break space
    date_sep: '/',
    date_order: DateOrder::DayMonthYear,
    months: &FR_MONTHS,
    months_abbrev: &FR_MONTHS_ABBR,
    days: &FR_DAYS,
    days_abbrev: &FR_DAYS_ABBR,
    twelve_hour: false,
    am: "",
    pm: "",
    currency_symbol: "€",
    currency_position: CurrencyPosition::TrailingSpace,
    default_decimals: 2,
};

const ES_ES: Locale = Locale {
    tag: "es-ES",
    decimal_sep: ',',
    thousands_sep: '.',
    date_sep: '/',
    date_order: DateOrder::DayMonthYear,
    months: &ES_MONTHS,
    months_abbrev: &ES_MONTHS_ABBR,
    days: &ES_DAYS,
    days_abbrev: &ES_DAYS_ABBR,
    twelve_hour: false,
    am: "",
    pm: "",
    currency_symbol: "€",
    currency_position: CurrencyPosition::TrailingSpace,
    default_decimals: 2,
};

const IT_IT: Locale = Locale {
    tag: "it-IT",
    decimal_sep: ',',
    thousands_sep: '.',
    date_sep: '/',
    date_order: DateOrder::DayMonthYear,
    months: &IT_MONTHS,
    months_abbrev: &IT_MONTHS_ABBR,
    days: &IT_DAYS,
    days_abbrev: &IT_DAYS_ABBR,
    twelve_hour: false,
    am: "",
    pm: "",
    currency_symbol: "€",
    currency_position: CurrencyPosition::TrailingNoSpace,
    default_decimals: 2,
};

/// The built-in locale table (the corpus locales + a documented en-US fallback).
pub const BUILTIN: &[Locale] = &[EN_US, EN_GB, DE_DE, FR_FR, ES_ES, IT_IT];

/// Number formatting spec (SDK `INumericFieldFormat`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NumberFormat {
    /// Number of digits shown after the decimal separator.
    pub decimals: u32,
    /// Group the integer part with the thousands separator.
    pub use_thousands: bool,
    /// The thousands-group separator character.
    pub thousands_sep: char,
    /// The decimal-point character.
    pub decimal_sep: char,
    /// How negative values are rendered.
    pub negative: NegativeStyle,
    /// Show a leading zero for values in (-1, 1) (`0.50` vs `.50`).
    pub leading_zero: bool,
}

impl Default for NumberFormat {
    fn default() -> NumberFormat {
        NumberFormat {
            decimals: 2,
            use_thousands: true,
            thousands_sep: ',',
            decimal_sep: '.',
            negative: NegativeStyle::LeadingMinus,
            leading_zero: true,
        }
    }
}

impl NumberFormat {
    /// A plain integer view: 0 decimals, no grouping (what `ToNumber`-style debug shows).
    pub fn integer() -> NumberFormat {
        NumberFormat {
            decimals: 0,
            use_thousands: false,
            ..NumberFormat::default()
        }
    }
}

/// Currency formatting spec (a [`NumberFormat`] plus symbol placement).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CurrencyFormat {
    /// The underlying numeric formatting (decimals, grouping, negatives).
    pub number: NumberFormat,
    /// The currency symbol string.
    pub symbol: String,
    /// Where the symbol sits relative to the amount.
    pub position: CurrencyPosition,
}

impl Default for CurrencyFormat {
    fn default() -> CurrencyFormat {
        CurrencyFormat {
            number: NumberFormat::default(),
            symbol: "$".to_string(),
            position: CurrencyPosition::LeadingNoSpace,
        }
    }
}

/// Date formatting spec — an `strftime`-ish pattern over the supported fields (see [`format_date`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DateFormat {
    /// The date pattern (`y`/`M`/`d` fields; see [`format_date`]).
    pub pattern: String,
}

impl Default for DateFormat {
    fn default() -> DateFormat {
        // Crystal en-US short date.
        DateFormat {
            pattern: "M/d/yyyy".to_string(),
        }
    }
}

/// Time formatting spec.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TimeFormat {
    /// The time pattern (`h`/`m`/`s`/`tt` fields; see [`format_time`]).
    pub pattern: String,
}

impl Default for TimeFormat {
    fn default() -> TimeFormat {
        TimeFormat {
            pattern: "h:mm:sstt".to_string(),
        }
    }
}

/// Boolean formatting spec (the word pair).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BoolFormat {
    /// Text rendered for a true value.
    pub true_text: String,
    /// Text rendered for a false value.
    pub false_text: String,
}

impl Default for BoolFormat {
    fn default() -> BoolFormat {
        BoolFormat {
            true_text: "True".to_string(),
            false_text: "False".to_string(),
        }
    }
}

/// The type-tagged union of all field-format specs.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FormatSpec {
    /// Numeric formatting.
    Number(NumberFormat),
    /// Currency formatting.
    Currency(CurrencyFormat),
    /// Date formatting.
    Date(DateFormat),
    /// Time formatting.
    Time(TimeFormat),
    /// Combined date-and-time formatting.
    DateTime(DateFormat, TimeFormat),
    /// Boolean formatting.
    Bool(BoolFormat),
    /// String values pass through unchanged.
    String,
}

/// Format a number per `spec`. Rounds half away from zero (the engine's rule).
pub fn format_number(value: f64, spec: &NumberFormat) -> String {
    let neg = value < 0.0;
    let scale = 10f64.powi(spec.decimals as i32);
    let scaled = (value.abs() * scale).round();
    let int_part = (scaled / scale).trunc() as u128;
    let frac_part = (scaled % scale) as u128;

    let mut int_str = int_part.to_string();
    if !spec.leading_zero && int_part == 0 && spec.decimals > 0 {
        int_str.clear();
    } else if spec.use_thousands {
        int_str = group_thousands(&int_str, spec.thousands_sep);
    }

    let mut body = int_str;
    if spec.decimals > 0 {
        body.push(spec.decimal_sep);
        body.push_str(&format!(
            "{frac_part:0width$}",
            width = spec.decimals as usize
        ));
    }
    apply_negative(&body, neg, spec.negative)
}

/// Format a currency value per `spec`.
pub fn format_currency(value: f64, spec: &CurrencyFormat) -> String {
    // The sign wraps the whole thing; format the magnitude with the symbol, then apply the sign.
    let neg = value < 0.0;
    let magnitude = format_number(
        value.abs(),
        &NumberFormat {
            negative: NegativeStyle::LeadingMinus, // unused (abs is non-negative)
            ..spec.number.clone()
        },
    );
    let with_symbol = match spec.position {
        CurrencyPosition::LeadingNoSpace => format!("{}{}", spec.symbol, magnitude),
        CurrencyPosition::LeadingSpace => format!("{} {}", spec.symbol, magnitude),
        CurrencyPosition::TrailingNoSpace => format!("{}{}", magnitude, spec.symbol),
        CurrencyPosition::TrailingSpace => format!("{} {}", magnitude, spec.symbol),
    };
    apply_negative(&with_symbol, neg, spec.number.negative)
}

/// Format a boolean per `spec`.
pub fn format_bool(value: bool, spec: &BoolFormat) -> String {
    if value {
        spec.true_text.clone()
    } else {
        spec.false_text.clone()
    }
}

/// Format a date per `spec.pattern` using en-US names ([`format_date_in`] with [`EN_US`]).
pub fn format_date(date: Date, spec: &DateFormat) -> String {
    format_date_in(date, spec, &EN_US)
}

/// Format a date per `spec.pattern`, taking month/day **names** from `loc`. Supported tokens
/// (longest-match): `yyyy` `yy` `MMMM` `MMM` `MM` `M` `dddd` `ddd` `dd` `d`. Any other run is a
/// literal.
pub fn format_date_in(date: Date, spec: &DateFormat, loc: &Locale) -> String {
    render_pattern(&spec.pattern, |tok| date_token(date, tok, loc))
}

/// Format a time per `spec.pattern` using en-US AM/PM ([`format_time_in`] with [`EN_US`]).
pub fn format_time(time: Time, spec: &TimeFormat) -> String {
    format_time_in(time, spec, &EN_US)
}

/// Format a time per `spec.pattern`, taking AM/PM designators from `loc`. Supported tokens: `HH`
/// `H` (24h) `hh` `h` (12h) `mm` `m` `ss` `s` `tt` (AM/PM) `t` (A/P).
pub fn format_time_in(time: Time, spec: &TimeFormat, loc: &Locale) -> String {
    render_pattern(&spec.pattern, |tok| time_token(time, tok, loc))
}

/// Format a datetime as `<date> <time>` (en-US names).
pub fn format_datetime(date: Date, time: Time, dspec: &DateFormat, tspec: &TimeFormat) -> String {
    format_datetime_in(date, time, dspec, tspec, &EN_US)
}

/// Format a datetime as `<date> <time>`, taking names/designators from `loc`.
pub fn format_datetime_in(
    date: Date,
    time: Time,
    dspec: &DateFormat,
    tspec: &TimeFormat,
    loc: &Locale,
) -> String {
    format!(
        "{} {}",
        format_date_in(date, dspec, loc),
        format_time_in(time, tspec, loc)
    )
}

// --- helpers ---

fn group_thousands(digits: &str, sep: char) -> String {
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(*b as char);
    }
    out
}

fn apply_negative(body: &str, neg: bool, style: NegativeStyle) -> String {
    if !neg {
        return body.to_string();
    }
    match style {
        NegativeStyle::LeadingMinus => format!("-{body}"),
        NegativeStyle::TrailingMinus => format!("{body}-"),
        NegativeStyle::Parens => format!("({body})"),
    }
}

/// Tokenizer shared by date/time pattern rendering: greedily consumes runs of the same letter,
/// resolves each run via `resolve`; a `None` from `resolve` (unknown token) emits the run verbatim.
/// A single-quoted run is a literal (`'at'`).
fn render_pattern(pattern: &str, resolve: impl Fn(&str) -> Option<String>) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            // Literal until the next quote.
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                out.push(chars[i]);
                i += 1;
            }
            i += 1; // closing quote
            continue;
        }
        if c.is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i] == c {
                i += 1;
            }
            let tok: String = chars[start..i].iter().collect();
            match resolve(&tok) {
                Some(s) => out.push_str(&s),
                None => out.push_str(&tok),
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn date_token(d: Date, tok: &str, loc: &Locale) -> Option<String> {
    let mi = (d.month.clamp(1, 12) - 1) as usize;
    let di = (d.day_of_week() - 1) as usize;
    Some(match tok {
        "yyyy" => format!("{:04}", d.year),
        "yy" => format!("{:02}", (d.year % 100 + 100) % 100),
        "MMMM" => loc.months[mi].to_string(),
        "MMM" => loc.months_abbrev[mi].to_string(),
        "MM" => format!("{:02}", d.month),
        "M" => d.month.to_string(),
        "dddd" => loc.days[di].to_string(),
        "ddd" => loc.days_abbrev[di].to_string(),
        "dd" => format!("{:02}", d.day),
        "d" => d.day.to_string(),
        _ => return None,
    })
}

fn time_token(t: Time, tok: &str, loc: &Locale) -> Option<String> {
    let (h12, pm) = to_12h(t.hour);
    Some(match tok {
        "HH" => format!("{:02}", t.hour),
        "H" => t.hour.to_string(),
        "hh" => format!("{:02}", h12),
        "h" => h12.to_string(),
        "mm" => format!("{:02}", t.minute),
        "m" => t.minute.to_string(),
        "ss" => format!("{:02}", t.second),
        "s" => t.second.to_string(),
        "tt" => if pm { loc.pm } else { loc.am }.to_string(),
        "t" => if pm { "P" } else { "A" }.to_string(),
        _ => return None,
    })
}

/// 12-hour clock: returns `(hour_1_to_12, is_pm)`.
fn to_12h(hour24: u8) -> (u8, bool) {
    match hour24 {
        0 => (12, false),
        h @ 1..=11 => (h, false),
        12 => (12, true),
        h => (h - 12, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_defaults_and_grouping() {
        assert_eq!(format_number(1234.5, &NumberFormat::default()), "1,234.50");
        assert_eq!(
            format_number(-1234.5, &NumberFormat::default()),
            "-1,234.50"
        );
        assert_eq!(format_number(0.5, &NumberFormat::default()), "0.50");
        assert_eq!(format_number(1234.5, &NumberFormat::integer()), "1235");
    }

    #[test]
    fn number_negative_styles() {
        let parens = NumberFormat {
            negative: NegativeStyle::Parens,
            ..NumberFormat::default()
        };
        assert_eq!(format_number(-12.0, &parens), "(12.00)");
        let trailing = NumberFormat {
            negative: NegativeStyle::TrailingMinus,
            ..NumberFormat::default()
        };
        assert_eq!(format_number(-12.0, &trailing), "12.00-");
    }

    #[test]
    fn number_no_leading_zero() {
        let spec = NumberFormat {
            leading_zero: false,
            ..NumberFormat::default()
        };
        assert_eq!(format_number(0.5, &spec), ".50");
        assert_eq!(format_number(-0.5, &spec), "-.50");
    }

    #[test]
    fn number_alt_separators() {
        let euro = NumberFormat {
            thousands_sep: '.',
            decimal_sep: ',',
            ..NumberFormat::default()
        };
        assert_eq!(format_number(1234.5, &euro), "1.234,50");
    }

    #[test]
    fn currency_positions() {
        assert_eq!(
            format_currency(1234.5, &CurrencyFormat::default()),
            "$1,234.50"
        );
        let trailing = CurrencyFormat {
            position: CurrencyPosition::TrailingSpace,
            symbol: "€".to_string(),
            ..CurrencyFormat::default()
        };
        assert_eq!(format_currency(1234.5, &trailing), "1,234.50 €");
        // Negative wraps the symbol.
        let parens = CurrencyFormat {
            number: NumberFormat {
                negative: NegativeStyle::Parens,
                ..NumberFormat::default()
            },
            ..CurrencyFormat::default()
        };
        assert_eq!(format_currency(-5.0, &parens), "($5.00)");
    }

    #[test]
    fn date_patterns() {
        let d = Date::new(2004, 1, 3); // Saturday
        assert_eq!(format_date(d, &DateFormat::default()), "1/3/2004");
        assert_eq!(
            format_date(
                d,
                &DateFormat {
                    pattern: "yyyy-MM-dd".into()
                }
            ),
            "2004-01-03"
        );
        assert_eq!(
            format_date(
                d,
                &DateFormat {
                    pattern: "dddd, MMMM d, yyyy".into()
                }
            ),
            "Saturday, January 3, 2004"
        );
        assert_eq!(
            format_date(
                d,
                &DateFormat {
                    pattern: "ddd MMM yy".into()
                }
            ),
            "Sat Jan 04"
        );
    }

    #[test]
    fn time_patterns() {
        let t = Time::new(14, 5, 6);
        assert_eq!(format_time(t, &TimeFormat::default()), "2:05:06PM");
        assert_eq!(
            format_time(
                t,
                &TimeFormat {
                    pattern: "HH:mm".into()
                }
            ),
            "14:05"
        );
        assert_eq!(
            format_time(
                Time::new(0, 0, 0),
                &TimeFormat {
                    pattern: "h:mm tt".into()
                }
            ),
            "12:00 AM"
        );
    }

    #[test]
    fn datetime_and_bool() {
        assert_eq!(
            format_datetime(
                Date::new(2004, 1, 3),
                Time::new(14, 5, 6),
                &DateFormat::default(),
                &TimeFormat::default()
            ),
            "1/3/2004 2:05:06PM"
        );
        assert_eq!(format_bool(true, &BoolFormat::default()), "True");
    }

    #[test]
    fn locale_lookup_and_fallback() {
        assert_eq!(Locale::lookup("de-DE").unwrap().tag, "de-DE");
        assert_eq!(Locale::lookup("de_DE.UTF-8").unwrap().tag, "de-DE"); // underscore + charset
        assert_eq!(Locale::lookup("de").unwrap().tag, "de-DE"); // language subtag only
        assert_eq!(Locale::lookup("EN-us").unwrap().tag, "en-US"); // case-insensitive
        assert!(Locale::lookup("zz-ZZ").is_none()); // unsupported
        assert_eq!(Locale::from_tag("zz-ZZ").tag, "en-US"); // documented fallback
    }

    #[test]
    fn number_uses_locale_separators() {
        let de = Locale::from_tag("de-DE");
        assert_eq!(format_number(1234.5, &de.number_format()), "1.234,50");
        let en = Locale::from_tag("en-US");
        assert_eq!(format_number(1234.5, &en.number_format()), "1,234.50");
    }

    #[test]
    fn date_uses_locale_month_names() {
        let d = Date::new(2004, 3, 3);
        let de = Locale::from_tag("de-DE");
        assert_eq!(
            format_date_in(
                d,
                &DateFormat {
                    pattern: "d. MMMM yyyy".into()
                },
                &de
            ),
            "3. März 2004"
        );
        let fr = Locale::from_tag("fr-FR");
        assert_eq!(
            format_date_in(
                d,
                &DateFormat {
                    pattern: "MMM".into()
                },
                &fr
            ),
            "mars"
        );
    }

    #[test]
    fn time_designators_follow_locale() {
        let t = Time::new(14, 5, 6);
        // en-US: PM designator; de-DE: empty (24h locale) — the tt token yields "".
        assert_eq!(
            format_time_in(
                t,
                &TimeFormat {
                    pattern: "h:mmtt".into()
                },
                &EN_US
            ),
            "2:05PM"
        );
        assert_eq!(
            format_time_in(
                t,
                &TimeFormat {
                    pattern: "h:mmtt".into()
                },
                &Locale::from_tag("de-DE")
            ),
            "2:05"
        );
    }

    #[test]
    fn literal_in_pattern() {
        let d = Date::new(2004, 1, 3);
        assert_eq!(
            format_date(
                d,
                &DateFormat {
                    pattern: "yyyy 'year'".into()
                }
            ),
            "2004 year"
        );
    }
}
