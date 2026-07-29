//! Resolve a field's **effective** display format by merging the two layers Crystal uses:
//!
//! 1. the **locale** (`--locale` / host) — the "system default" layer: separators, month/day names,
//!    AM/PM, default date order, default decimals, currency symbol; and
//! 2. the field's **stored** [`FieldFormat`] leaf — the explicit authoring choices (decimals,
//!    negative style, currency symbol placement, date component forms, time clock base / element
//!    styles / separators / AM-PM designators, boolean word pair).
//!
//! The switch that arbitrates them lives in the field itself: [`CommonFieldFormat::use_system_defaults`]
//! (the master flag) and [`DateFieldFormat::system_default`]. When a field uses system defaults, the
//! locale supplies the effective format; otherwise the stored leaf wins for the attributes it sets.
//! An explicit date or time takes almost nothing from the locale — it stores its own separators and
//! designator strings — and only month and weekday *names* still come from there, which is all
//! Crystal never stores (the leaf holds [`MonthFormat::LongMonth`], never "January").

use rpt_format_value::{
    format_bool, format_currency, format_date_in, format_number, format_time_in, BoolFormat,
    CurrencyFormat, CurrencyPosition, DateFormat, DateOrder, FormatSpec, Locale, NegativeStyle,
    NumberFormat, TimeFormat,
};
use rpt_formula::eval::Value;
use rpt_model::{
    AMPMFormat, BooleanOutputType, CurrencySymbolFormat, DateFieldFormat, DateSystemDefaultType,
    DateTimeOrder, DayFormat, DayOfWeekFormat, DayOfWeekPosition, FieldFormat, FieldValueType,
    HourFormat, MinuteFormat, MonthFormat, NegativeFormat, SecondFormat, TimeBase, YearFormat,
};

/// Build the effective [`FormatSpec`] for a field value of type `vt`, merging the locale defaults
/// with the field's stored [`FieldFormat`] (when it does not defer to system defaults).
pub fn field_format_spec(
    fmt: Option<&FieldFormat>,
    vt: FieldValueType,
    loc: &Locale,
) -> FormatSpec {
    use FieldValueType as T;
    match vt {
        T::Int8s | T::Int16s | T::Int32s | T::Int32u | T::Number => {
            currency_or_number(fmt, vt, loc, false)
        }
        T::Currency => currency_or_number(fmt, vt, loc, true),
        T::Date => FormatSpec::Date(date_spec(fmt, loc)),
        T::Time => FormatSpec::Time(time_spec(fmt, loc)),
        T::DateTime => datetime_spec(fmt, loc),
        T::Boolean => FormatSpec::Bool(bool_spec(fmt)),
        _ => FormatSpec::String,
    }
}

/// Format a resolved [`Value`] through `spec`, taking any names/separators from `loc`. Falls back to
/// the value's default text form when the value kind and spec kind disagree (e.g. a formula whose
/// declared type does not match its runtime value).
pub fn render_value(value: &Value, spec: &FormatSpec, loc: &Locale) -> String {
    match (value, spec) {
        (Value::Number(n) | Value::Currency(n), FormatSpec::Number(nf)) => format_number(*n, nf),
        (Value::Number(n) | Value::Currency(n), FormatSpec::Currency(cf)) => {
            format_currency(*n, cf)
        }
        (Value::Number(n) | Value::Currency(n), _) => format_number(*n, &loc.number_format()),
        (Value::Date(d), FormatSpec::Date(df)) => format_date_in(*d, df, loc),
        (Value::Time(t), FormatSpec::Time(tf)) => format_time_in(*t, tf, loc),
        (
            Value::DateTime(d, t),
            FormatSpec::DateTime {
                date,
                time,
                separator,
                time_first,
            },
        ) => {
            let (d, t) = (format_date_in(*d, date, loc), format_time_in(*t, time, loc));
            if *time_first {
                format!("{t}{separator}{d}")
            } else {
                format!("{d}{separator}{t}")
            }
        }
        // A datetime narrowed to one part by the field's stored `DateTimeOrder`.
        (Value::DateTime(d, _), FormatSpec::Date(df)) => format_date_in(*d, df, loc),
        (Value::DateTime(_, t), FormatSpec::Time(tf)) => format_time_in(*t, tf, loc),
        (Value::Bool(b), FormatSpec::Bool(bf)) => format_bool(*b, bf),
        (Value::Str(s), _) => s.clone(),
        (v, _) => v.to_text_default().unwrap_or_default(),
    }
}

/// Format a [`Value`] with the locale's system defaults for its runtime kind — used for embedded
/// `{field}`/`{@formula}` references in a text object, which carry no per-field format leaf.
pub fn render_value_default(value: &Value, loc: &Locale) -> String {
    let vt = match value {
        Value::Number(_) => FieldValueType::Number,
        Value::Currency(_) => FieldValueType::Currency,
        Value::Date(_) => FieldValueType::Date,
        Value::Time(_) => FieldValueType::Time,
        Value::DateTime(..) => FieldValueType::DateTime,
        Value::Bool(_) => FieldValueType::Boolean,
        _ => FieldValueType::String,
    };
    let mut spec = field_format_spec(None, vt, loc);
    // An embedded run sits inside a sentence, not a column, and the engine prints it flush — no sign
    // cell is reserved (`Page  1`, not `Page   1`).
    match &mut spec {
        FormatSpec::Number(nf) => nf.reserve_sign = false,
        FormatSpec::Currency(cf) => cf.number.reserve_sign = false,
        _ => {}
    }
    render_value(value, &spec, loc)
}

/// The numeric-format slot the engine applies to a field of type `vt`.
///
/// A field stores **two** numeric formats — a currency-format slot and a number-format slot — and
/// the effective value type selects between them: a Currency-valued field is formatted by the
/// currency slot, everything else by the number slot. Both are decoded verbatim; the selection is a
/// runtime resolution and so lives here rather than on the model.
fn numeric_slot(f: &FieldFormat, vt: FieldValueType) -> &rpt_model::NumericFieldFormat {
    if vt == FieldValueType::Currency {
        &f.currency_numeric
    } else {
        &f.numeric
    }
}

/// Whether the field shows its currency symbol only on its first printed value of each page (SDK
/// `OneCurrencySymbolPerPage`).
///
/// Like the sign flip and the zero literal, this has no Windows regional counterpart — it is a
/// Crystal-only display rule — so a field that otherwise defers to the host settings still applies
/// it. It is not a property of a value, so it cannot be expressed in the [`FormatSpec`]: which value
/// is a page's first is only decided once pagination has settled page membership.
pub(crate) fn one_currency_symbol_per_page(fmt: Option<&FieldFormat>, vt: FieldValueType) -> bool {
    fmt.is_some_and(|f| numeric_slot(f, vt).one_currency_symbol_per_page)
}

fn numeric_spec(fmt: Option<&FieldFormat>, vt: FieldValueType, loc: &Locale) -> NumberFormat {
    let mut nf = loc.number_format();
    // The engine reserves the character cell its negative form would occupy on every value that can
    // be negative, so a column's positives line up with its negatives. An unsigned type (`PageNumber`)
    // never can be, so it gets no cell and the engine prints it flush.
    nf.reserve_sign = vt != FieldValueType::Int32u;
    // Integer value types show no decimals by default, but keep the locale's thousands grouping — the
    // engine groups an integer field (e.g. `1,002`) just like a decimal one.
    if matches!(
        vt,
        FieldValueType::Int8s
            | FieldValueType::Int16s
            | FieldValueType::Int32s
            | FieldValueType::Int32u
    ) {
        nf.decimals = 0;
    }
    // Windows keeps a negative *currency* form separate from the negative number form, so a
    // system-default amount brackets where a number would take a leading minus (en-US `($1.10)` vs
    // `-1.10`). An explicit field overrides it below.
    if vt == FieldValueType::Currency {
        nf.negative = loc.currency_negative;
    }
    if let Some(f) = fmt {
        let slot = numeric_slot(f, vt);
        if !f.common.use_system_defaults {
            if slot.decimal_places >= 0 {
                nf.decimals = slot.decimal_places as u32;
            }
            // The field's stored grouping/suppression choices win over the locale baseline.
            nf.use_thousands = slot.thousands_separator;
            nf.suppress_if_zero = slot.suppress_if_zero;
            nf.negative = map_negative(slot.negative);
            // Whether a value below one keeps its integer `0` is one of the Windows regional number
            // settings, so it belongs inside the system-defaults gate with the rest of them.
            nf.leading_zero = slot.use_lead_zero;
        }
        // The sign flip and the zero literal have no regional counterpart — they are Crystal-only
        // display rules, so a field that otherwise defers to the host settings still applies them.
        nf.reverse_sign = slot.display_reverse_sign;
        nf.zero_value = zero_value_override(&slot.zero_value_string);
    }
    nf
}

/// The `ZeroValueString` the engine writes when the field sets no zero literal. It is a marker, not
/// a string to print.
const ZERO_VALUE_UNSET: &str = "<Default Format>";

/// The stored `ZeroValueString` as a display override, or `None` to format the zero normally. Both
/// the engine's marker and an empty value (a field whose leaf carries no zero literal at all) mean
/// "no override".
fn zero_value_override(stored: &str) -> Option<String> {
    if stored.is_empty() || stored == ZERO_VALUE_UNSET {
        None
    } else {
        Some(stored.to_string())
    }
}

/// A [`CurrencyFormat`] when the field shows a symbol, else a plain [`NumberFormat`].
///
/// The symbol is stored **per field** (`currency_symbol_text`, e.g. `"€"`, `"kr "`, `"Kč"`), so two
/// fields in one report can carry two different currencies. An explicit field (not using system
/// defaults) uses its own stored symbol and NoSymbol/Fixed/Floating choice, and its own stored
/// leading/trailing placement; a system-default field resolves both the symbol and its placement
/// from the render locale (the host regional setting — Crystal keeps no report-level default
/// currency). Any spacing around a trailing symbol is baked into the stored symbol string itself.
///
/// The symbol is a property of the numeric format, not of the value type, so a plain **number**
/// field carries one too — that is how a percentage summary gets its trailing `%`. `symbol_by_default`
/// is what separates the two: only a currency-typed field falls back to the locale's symbol, so a
/// number field shows a symbol solely when it stores one explicitly.
fn currency_or_number(
    fmt: Option<&FieldFormat>,
    vt: FieldValueType,
    loc: &Locale,
    symbol_by_default: bool,
) -> FormatSpec {
    let number = numeric_spec(fmt, vt, loc);
    // Resolve whether a symbol shows, which one, and where it sits. NoSymbol on an explicit field
    // drops to a plain number; otherwise prefer the field's stored symbol string and stored
    // placement, falling back to the locale when the field stored none (or defers to system defaults).
    let (show, symbol, position) = match fmt {
        Some(f) if !f.common.use_system_defaults => {
            let slot = numeric_slot(f, vt);
            let show = slot.currency_symbol != CurrencySymbolFormat::NoSymbol;
            let symbol = if slot.currency_symbol_text.is_empty() {
                loc.currency_symbol.to_string()
            } else {
                slot.currency_symbol_text.clone()
            };
            (show, symbol, map_currency_position(slot.currency_position))
        }
        _ => (
            symbol_by_default,
            loc.currency_symbol.to_string(),
            loc.currency_position,
        ),
    };
    if show {
        FormatSpec::Currency(CurrencyFormat {
            number,
            symbol,
            position,
        })
    } else {
        FormatSpec::Number(number)
    }
}

/// Map the field's stored [`rpt_model::CurrencyPosition`] onto the renderer's leading/trailing
/// placement. The stored enum also encodes whether the symbol sits inside or outside the negative
/// sign — a distinction the renderer does not model — and any spacing lives in the stored symbol
/// string, so only the leading/trailing axis is carried across.
fn map_currency_position(pos: rpt_model::CurrencyPosition) -> CurrencyPosition {
    use rpt_model::CurrencyPosition as Stored;
    match pos {
        Stored::TrailingCurrencyInsideNegative | Stored::TrailingCurrencyOutsideNegative => {
            CurrencyPosition::TrailingNoSpace
        }
        _ => CurrencyPosition::LeadingNoSpace,
    }
}

fn map_negative(n: NegativeFormat) -> NegativeStyle {
    match n {
        NegativeFormat::TrailingMinus => NegativeStyle::TrailingMinus,
        NegativeFormat::Bracketed => NegativeStyle::Parens,
        // NotNegative (no special negative rendering) and LeadingMinus both show a leading minus.
        _ => NegativeStyle::LeadingMinus,
    }
}

/// The effective spec for a datetime-valued field, gated by the stored [`DateTimeOrder`]: it selects
/// *which* parts show and in what order, independently of the date/time sub-formats. `DateOnly` and
/// `TimeOnly` collapse the field to a single part — a datetime rendered through a date-only field
/// carries no time component at all.
fn datetime_spec(fmt: Option<&FieldFormat>, loc: &Locale) -> FormatSpec {
    let order = fmt.map(|f| f.date_time.order).unwrap_or_default();
    match order {
        DateTimeOrder::DateOnly => FormatSpec::Date(date_spec(fmt, loc)),
        DateTimeOrder::TimeOnly => FormatSpec::Time(time_spec(fmt, loc)),
        _ => FormatSpec::DateTime {
            date: date_spec(fmt, loc),
            time: time_spec(fmt, loc),
            separator: datetime_separator(fmt),
            time_first: order == DateTimeOrder::TimeThenDate,
        },
    }
}

/// The string placed between a datetime's date and time parts: the field's stored
/// `DateTimeSeparator` when it set one, else a single space (the engine's default join).
fn datetime_separator(fmt: Option<&FieldFormat>) -> String {
    match fmt {
        Some(f) if !f.date_time.separator.is_empty() => f.date_time.separator.clone(),
        _ => " ".to_string(),
    }
}

/// The effective spec for the time part of a time- or datetime-valued field. A system-default field
/// takes the locale's clock and designators; an explicit one takes its whole stored
/// [`rpt_model::TimeFieldFormat`] — clock base, element styles, designator text and placement, and
/// both element separators.
fn time_spec(fmt: Option<&FieldFormat>, loc: &Locale) -> TimeFormat {
    let Some(f) = fmt.filter(|f| !f.common.use_system_defaults) else {
        return TimeFormat {
            pattern: if loc.twelve_hour {
                "h:mm:sstt".to_string()
            } else {
                "HH:mm:ss".to_string()
            },
            am_pm: None,
        };
    };
    let t = &f.time;
    let twelve_hour = match t.time_base {
        TimeBase::TwelveHour => true,
        TimeBase::TwentyFourHour => false,
        TimeBase::Other(_) => loc.twelve_hour,
    };
    // The hour occupies a fixed two-cell field: the "no leading zero" style space-pads it rather
    // than dropping the cell, so midnight prints `" 0"` and not `"0"`.
    let hour = match t.hour {
        HourFormat::NumericHour => {
            if twelve_hour {
                "hh"
            } else {
                "HH"
            }
        }
        HourFormat::NoLeadingZeroNumericHour => {
            if twelve_hour {
                "hhh"
            } else {
                "HHH"
            }
        }
        _ => "",
    };
    let minute = match t.minute {
        MinuteFormat::NumericMinute => "mm",
        MinuteFormat::NoLeadingZeroNumericMinute => "m",
        _ => "",
    };
    let second = match t.second {
        SecondFormat::NumericSecond => "ss",
        SecondFormat::NoLeadingZeroNumericSecond => "s",
        _ => "",
    };
    let body = join_elements([
        (hour, t.hour_minute_separator.as_str()),
        (minute, t.minute_second_separator.as_str()),
        (second, ""),
    ]);
    // Only a 12-hour field shows a designator at all; the gap before or after it is baked into the
    // stored designator strings, so nothing is inserted around the token.
    let pattern = match (twelve_hour, t.am_pm_format) {
        (false, _) => body,
        (true, AMPMFormat::AMPMBefore) => format!("tt{body}"),
        (true, _) => format!("{body}tt"),
    };
    TimeFormat {
        pattern,
        am_pm: twelve_hour.then(|| (t.am_string.clone(), t.pm_string.clone())),
    }
}

/// Join a date's or a time's three ordered element tokens, dropping the elements the field does not
/// show. Each element carries the separator stored *after* it, and a separator is emitted only when
/// a later element still follows — so a time with `NoHour` takes the hour-minute separator with it,
/// while `NoMinute` leaves the hour joined to the second by the hour-minute separator, and a
/// month/day/year date with no day joins its month to its year by the *first* date separator.
fn join_elements(elements: [(&str, &str); 3]) -> String {
    let present: Vec<(&str, &str)> = elements
        .into_iter()
        .filter(|(token, _)| !token.is_empty())
        .collect();
    let mut out = String::new();
    for (i, (token, separator)) in present.iter().enumerate() {
        out.push_str(token);
        if i + 1 < present.len() {
            out.push_str(&pattern_literal(separator));
        }
    }
    out
}

/// Quote a stored separator so the pattern renderer emits it verbatim: an unquoted alphabetic run
/// would be read as a format token. The pattern grammar has no escape for an apostrophe, so one is
/// dropped.
fn pattern_literal(separator: &str) -> String {
    if separator.is_empty() {
        return String::new();
    }
    format!("'{}'", separator.replace('\'', ""))
}

fn date_spec(fmt: Option<&FieldFormat>, loc: &Locale) -> DateFormat {
    let system_default = match fmt {
        None => true,
        Some(f) => {
            f.common.use_system_defaults
                || f.date.system_default != DateSystemDefaultType::NotUsingWindowsDefaults
        }
    };
    if system_default {
        let long = matches!(
            fmt.map(|f| f.date.system_default),
            Some(DateSystemDefaultType::UseWindowsLongDate)
        );
        DateFormat {
            pattern: default_date_pattern(loc, long),
        }
    } else {
        let f = fmt.expect("non-system-default implies a stored leaf");
        DateFormat {
            pattern: pattern_from_components(&f.date, stored_date_order(f.date.date_order, loc)),
        }
    }
}

/// The component order for an explicit (non-system-default) date field: the field's own stored
/// [`rpt_model::DateOrder`], which is an authoring choice the engine honours regardless of the host
/// regional order. An unrecognized code falls back to the locale's order.
fn stored_date_order(stored: rpt_model::DateOrder, loc: &Locale) -> DateOrder {
    use rpt_model::DateOrder as Stored;
    match stored {
        Stored::YearMonthDay => DateOrder::YearMonthDay,
        Stored::DayMonthYear => DateOrder::DayMonthYear,
        Stored::MonthDayYear => DateOrder::MonthDayYear,
        Stored::Other(_) => loc.date_order,
    }
}

/// The locale's system-default date pattern: numeric day/month + long year (the form Windows' short
/// date reports), ordered per the locale, or a long form with the full month name. The short form
/// pads day/month to two digits only in locales whose Windows short date does (en-US is `M/d/yyyy`,
/// unpadded; en-GB/de-DE/… are `dd/MM/yyyy`).
fn default_date_pattern(loc: &Locale, long: bool) -> String {
    if long {
        return match loc.date_order {
            DateOrder::MonthDayYear => "MMMM d, yyyy".to_string(),
            DateOrder::DayMonthYear => "d MMMM yyyy".to_string(),
            DateOrder::YearMonthDay => "yyyy MMMM d".to_string(),
        };
    }
    if loc.short_date_leading_zero {
        order_join(loc, "dd", "MM", "yyyy")
    } else {
        order_join(loc, "d", "M", "yyyy")
    }
}

/// Assemble a `d`/`M`/`y` token triple in the locale's component order, joined by its date sep.
fn order_join(loc: &Locale, day: &str, month: &str, year: &str) -> String {
    let sep = loc.date_sep;
    match loc.date_order {
        DateOrder::MonthDayYear => format!("{month}{sep}{day}{sep}{year}"),
        DateOrder::DayMonthYear => format!("{day}{sep}{month}{sep}{year}"),
        DateOrder::YearMonthDay => format!("{year}{sep}{month}{sep}{day}"),
    }
}

/// Build a date pattern from an explicit field's stored leaf: the three day/month/year element
/// forms placed in `order` around the stored literal separators, wrapped by the stored prefix and
/// suffix, with the weekday element and its own separator outside those on the side
/// [`DayOfWeekPosition`] names.
///
/// `EraType` and `CalendarType` are decoded but not rendered: the engine prints no era designator
/// under a Gregorian calendar, and renders a Gregorian-stored date unchanged under a non-Gregorian
/// one. A calendar conversion would be a guess at an algorithm no engine output exhibits.
fn pattern_from_components(date: &DateFieldFormat, order: DateOrder) -> String {
    let d = match date.day {
        DayFormat::NumericDay => "d",
        DayFormat::LeadingZeroNumericDay => "dd",
        _ => "",
    };
    let m = match date.month {
        MonthFormat::NumericMonth => "M",
        MonthFormat::LeadingZeroNumericMonth => "MM",
        MonthFormat::ShortMonth => "MMM",
        MonthFormat::LongMonth => "MMMM",
        _ => "",
    };
    let y = match date.year {
        YearFormat::ShortYear => "yy",
        YearFormat::LongYear => "yyyy",
        _ => "",
    };
    let ordered: [&str; 3] = match order {
        DateOrder::MonthDayYear => [m, d, y],
        DateOrder::DayMonthYear => [d, m, y],
        DateOrder::YearMonthDay => [y, m, d],
    };
    let body = join_elements([
        (ordered[0], date.first_separator.as_str()),
        (ordered[1], date.second_separator.as_str()),
        (ordered[2], ""),
    ]);
    let dated = format!(
        "{}{body}{}",
        pattern_literal(&date.prefix_separator),
        pattern_literal(&date.suffix_separator)
    );
    let weekday = match date.day_of_week {
        DayOfWeekFormat::ShortDayOfWeek => "ddd",
        DayOfWeekFormat::LongDayOfWeek => "dddd",
        _ => return dated,
    };
    let sep = pattern_literal(&date.day_of_week_separator);
    match date.day_of_week_position {
        DayOfWeekPosition::TrailingPosition => format!("{dated}{sep}{weekday}"),
        _ => format!("{weekday}{sep}{dated}"),
    }
}

fn bool_spec(fmt: Option<&FieldFormat>) -> BoolFormat {
    let ty = fmt.map(|f| f.boolean.output_type).unwrap_or_default();
    let (t, f) = match ty {
        BooleanOutputType::TOrF => ("T", "F"),
        BooleanOutputType::YesOrNo => ("Yes", "No"),
        BooleanOutputType::YOrN => ("Y", "N"),
        BooleanOutputType::OneOrZero => ("1", "0"),
        _ => ("True", "False"),
    };
    BoolFormat {
        true_text: t.to_string(),
        false_text: f.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_format_value::{Date, Time};
    use rpt_model::TimeFieldFormat;

    fn de() -> Locale {
        Locale::from_tag("de-DE")
    }

    /// Build a stored field format that opts out of system defaults (so its explicit attributes win).
    /// The date and time leaves are filled in with the shape a real authored field carries — an
    /// all-default leaf stores empty separators and designators and a short weekday, none of which
    /// any authoring tool writes.
    fn explicit_fmt() -> FieldFormat {
        let mut f = FieldFormat::default();
        f.common.use_system_defaults = false;
        f.date.day_of_week = DayOfWeekFormat::NoDayOfWeek;
        f.date.first_separator = "/".to_string();
        f.date.second_separator = "/".to_string();
        f.time = TimeFieldFormat {
            time_base: TimeBase::TwelveHour,
            am_pm_format: AMPMFormat::AMPMAfter,
            hour: HourFormat::NumericHour,
            minute: MinuteFormat::NumericMinute,
            second: SecondFormat::NumericSecond,
            am_string: "AM".to_string(),
            pm_string: "PM".to_string(),
            hour_minute_separator: ":".to_string(),
            minute_second_separator: ":".to_string(),
        };
        f
    }

    #[test]
    fn number_uses_locale_when_system_default() {
        let spec = field_format_spec(None, FieldValueType::Number, &de());
        assert_eq!(
            render_value(&Value::Number(1234.5), &spec, &de()),
            " 1.234,50"
        );
    }

    #[test]
    fn integer_field_groups_thousands_with_no_decimals() {
        // A system-default integer field shows no decimals but still groups thousands, like the engine
        // (`1,002`, not `1002`).
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(None, FieldValueType::Int32s, &loc);
        assert_eq!(render_value(&Value::Number(1002.0), &spec, &loc), " 1,002");
    }

    #[test]
    fn explicit_decimals_override_locale_default() {
        let mut fmt = explicit_fmt();
        fmt.numeric.decimal_places = 0;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        // 0 explicit decimals wins over the locale's default of 2.
        assert_eq!(render_value(&Value::Number(1234.5), &spec, &loc), " 1,235");
    }

    #[test]
    fn date_system_default_uses_locale_order() {
        let spec = field_format_spec(None, FieldValueType::Date, &de());
        // de-DE system-default short date: dd.MM.yyyy.
        assert_eq!(
            render_value(&Value::Date(Date::new(2004, 3, 5)), &spec, &de()),
            "05.03.2004"
        );
    }

    #[test]
    fn date_system_default_en_us_short_date_is_unpadded() {
        // en-US's Windows short date is M/d/yyyy — the numeric month/day carry no leading zero, unlike
        // the padded dd.MM.yyyy of de-DE and other locales.
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(None, FieldValueType::Date, &loc);
        assert_eq!(
            render_value(&Value::Date(Date::new(2023, 5, 2)), &spec, &loc),
            "5/2/2023"
        );
    }

    /// An explicit field's stored component forms, order and separators are honoured; only the
    /// month *name* comes from the locale, which is all the leaf does not store.
    #[test]
    fn explicit_date_components_use_stored_order_and_locale_names() {
        let mut fmt = explicit_fmt();
        fmt.date.day = DayFormat::NumericDay;
        fmt.date.month = MonthFormat::LongMonth;
        fmt.date.year = YearFormat::LongYear;
        fmt.date.date_order = rpt_model::DateOrder::DayMonthYear;
        fmt.date.system_default = DateSystemDefaultType::NotUsingWindowsDefaults;
        let loc = de();
        let spec = field_format_spec(Some(&fmt), FieldValueType::Date, &loc);
        // DMY order, German month name, the field's own stored '/' separator.
        assert_eq!(
            render_value(&Value::Date(Date::new(2004, 3, 5)), &spec, &loc),
            "5/März/2004"
        );
    }

    /// An explicit field carrying a stored currency symbol renders with that symbol, not the
    /// locale's — the per-field currency wins.
    #[test]
    fn explicit_currency_symbol_wins_over_locale() {
        let mut fmt = explicit_fmt();
        fmt.currency_numeric.currency_symbol = CurrencySymbolFormat::FloatingSymbol;
        fmt.currency_numeric.currency_symbol_text = "€".to_string();
        let loc = Locale::from_tag("en-US"); // locale symbol is "$"
        let spec = field_format_spec(Some(&fmt), FieldValueType::Currency, &loc);
        assert_eq!(
            render_value(&Value::Currency(1234.5), &spec, &loc),
            "€1,234.50"
        );
    }

    /// Two fields in the same report, two different stored currencies — true multi-currency.
    #[test]
    fn two_fields_two_currencies() {
        let loc = Locale::from_tag("en-US");
        let mut eur = explicit_fmt();
        eur.currency_numeric.currency_symbol = CurrencySymbolFormat::FloatingSymbol;
        eur.currency_numeric.currency_symbol_text = "€".to_string();
        let mut czk = explicit_fmt();
        czk.currency_numeric.currency_symbol = CurrencySymbolFormat::FloatingSymbol;
        czk.currency_numeric.currency_symbol_text = "Kč".to_string();
        let eur_spec = field_format_spec(Some(&eur), FieldValueType::Currency, &loc);
        let czk_spec = field_format_spec(Some(&czk), FieldValueType::Currency, &loc);
        assert_eq!(
            render_value(&Value::Currency(10.0), &eur_spec, &loc),
            "€10.00"
        );
        assert_eq!(
            render_value(&Value::Currency(10.0), &czk_spec, &loc),
            "Kč10.00"
        );
    }

    /// A stored symbol string may bake in its own spacing (e.g. `"kr "`).
    #[test]
    fn stored_symbol_keeps_baked_space() {
        let mut fmt = explicit_fmt();
        fmt.currency_numeric.currency_symbol = CurrencySymbolFormat::FixedSymbol;
        fmt.currency_numeric.currency_symbol_text = "kr ".to_string();
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Currency, &loc);
        assert_eq!(
            render_value(&Value::Currency(10.0), &spec, &loc),
            "kr 10.00"
        );
    }

    /// `NoSymbol` on an explicit field drops to a plain number.
    #[test]
    fn explicit_no_symbol_renders_plain_number() {
        let mut fmt = explicit_fmt();
        fmt.numeric.currency_symbol = CurrencySymbolFormat::NoSymbol;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Currency, &loc);
        assert_eq!(render_value(&Value::Currency(10.0), &spec, &loc), " 10.00");
    }

    /// A **number** field carrying an explicit stored symbol renders it too — this is how a
    /// percentage summary gets its trailing `%`.
    #[test]
    fn number_field_renders_its_stored_symbol() {
        let mut fmt = explicit_fmt();
        fmt.numeric.currency_symbol = CurrencySymbolFormat::FloatingSymbol;
        fmt.numeric.currency_symbol_text = "%".to_string();
        fmt.numeric.currency_position =
            rpt_model::CurrencyPosition::TrailingCurrencyOutsideNegative;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(
            render_value(&Value::Number(13.5044), &spec, &loc),
            " 13.50%"
        );
    }

    /// A number field that stores no symbol choice never picks up the locale's currency symbol —
    /// only a currency-typed field falls back to it.
    #[test]
    fn number_field_does_not_borrow_the_locale_symbol() {
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(None, FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(10.0), &spec, &loc), " 10.00");
        let mut fmt = explicit_fmt();
        fmt.numeric.currency_symbol = CurrencySymbolFormat::NoSymbol;
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(10.0), &spec, &loc), " 10.00");
    }

    /// A positive value is padded into the character cells its negative form would occupy, so the
    /// column lines up: a leading cell for the minus, a trailing one for the closing bracket.
    #[test]
    fn positive_reserves_the_cells_its_negative_form_uses() {
        let loc = Locale::from_tag("en-US");
        let mut fmt = explicit_fmt();
        fmt.numeric.decimal_places = 0;
        fmt.numeric.thousands_separator = false;

        fmt.numeric.negative = NegativeFormat::LeadingMinus;
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(3080.0), &spec, &loc), " 3080");
        assert_eq!(render_value(&Value::Number(-3080.0), &spec, &loc), "-3080");

        fmt.numeric.negative = NegativeFormat::Bracketed;
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(3080.0), &spec, &loc), " 3080 ");
        assert_eq!(render_value(&Value::Number(-3080.0), &spec, &loc), "(3080)");

        fmt.numeric.negative = NegativeFormat::TrailingMinus;
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(3080.0), &spec, &loc), "3080 ");
    }

    /// A currency symbol already fills its end of the field, so that cell is not padded: a leading
    /// symbol under a bracketed negative pads only on the right, a trailing one only on the left.
    #[test]
    fn a_currency_symbol_fills_the_cell_on_its_own_side() {
        let loc = Locale::from_tag("en-US");
        let mut fmt = explicit_fmt();
        fmt.currency_numeric.currency_symbol = CurrencySymbolFormat::FloatingSymbol;
        fmt.currency_numeric.currency_symbol_text = "$".to_string();
        fmt.currency_numeric.negative = NegativeFormat::Bracketed;
        let spec = field_format_spec(Some(&fmt), FieldValueType::Currency, &loc);
        assert_eq!(
            render_value(&Value::Currency(2883902.07), &spec, &loc),
            "$2,883,902.07 "
        );

        // A *number* field reads the other stored slot — that is how a percentage summary gets its
        // trailing `%`.
        fmt.numeric.currency_symbol = CurrencySymbolFormat::FloatingSymbol;
        fmt.numeric.negative = NegativeFormat::Bracketed;
        fmt.numeric.currency_symbol_text = "%".to_string();
        fmt.numeric.currency_position =
            rpt_model::CurrencyPosition::TrailingCurrencyOutsideNegative;
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(
            render_value(&Value::Number(13.5044), &spec, &loc),
            " 13.50%"
        );
    }

    /// A system-default currency field takes the locale's negative *currency* form, which en-US
    /// brackets — so its positive reserves the closing bracket's cell where a number would not.
    #[test]
    fn system_default_currency_takes_the_locale_negative_form() {
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(None, FieldValueType::Currency, &loc);
        assert_eq!(render_value(&Value::Currency(53.9), &spec, &loc), "$53.90 ");
        assert_eq!(
            render_value(&Value::Currency(-53.9), &spec, &loc),
            "($53.90)"
        );
        // A number in the same locale keeps the leading minus, and pads on that side instead.
        let spec = field_format_spec(None, FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(53.9), &spec, &loc), " 53.90");
    }

    /// An unsigned field has no negative form, so it reserves no cell — the engine prints a page
    /// number flush.
    #[test]
    fn unsigned_field_reserves_no_cell() {
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(None, FieldValueType::Int32u, &loc);
        assert_eq!(render_value(&Value::Number(4.0), &spec, &loc), "4");
    }

    /// A suppressed zero stays blank rather than becoming a lone pad character.
    #[test]
    fn suppressed_zero_is_not_padded_into_a_space() {
        let mut fmt = explicit_fmt();
        fmt.numeric.suppress_if_zero = true;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(0.0), &spec, &loc), "");
    }

    /// An embedded run reserves no cell — it sits in a sentence, not a column.
    #[test]
    fn embedded_run_reserves_no_cell() {
        let loc = Locale::from_tag("en-US");
        assert_eq!(render_value_default(&Value::Number(1.0), &loc), "1.00");
    }

    /// A system-default currency field resolves its symbol from the render locale.
    #[test]
    fn system_default_currency_uses_locale_symbol() {
        // de-DE: "€", trailing — here we only assert the locale symbol is used, not the position.
        let spec = field_format_spec(None, FieldValueType::Currency, &de());
        let out = render_value(&Value::Currency(1234.5), &spec, &de());
        assert!(out.contains('€'), "expected locale € symbol, got {out}");
    }

    /// A field authored with grouping off renders without the thousands separator.
    #[test]
    fn stored_grouping_off_drops_thousands_separator() {
        let mut fmt = explicit_fmt();
        fmt.numeric.thousands_separator = false;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(
            render_value(&Value::Number(1234.5), &spec, &loc),
            " 1234.50"
        );
    }

    /// `EnableSuppressIfZero` blanks a zero value; a non-zero value in the same field is unaffected.
    #[test]
    fn stored_suppress_if_zero_blanks_zero() {
        let mut fmt = explicit_fmt();
        fmt.numeric.suppress_if_zero = true;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(0.0), &spec, &loc), "");
        assert_eq!(render_value(&Value::Number(12.5), &spec, &loc), " 12.50");
    }

    /// Without the flag a zero renders normally (the default, unchanged behavior).
    #[test]
    fn zero_without_suppress_renders_normally() {
        let mut fmt = explicit_fmt();
        fmt.numeric.suppress_if_zero = false;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(0.0), &spec, &loc), " 0.00");
    }

    /// A suppressed zero currency blanks the whole field, symbol included.
    #[test]
    fn stored_suppress_if_zero_blanks_currency() {
        let mut fmt = explicit_fmt();
        fmt.currency_numeric.suppress_if_zero = true;
        fmt.currency_numeric.currency_symbol = CurrencySymbolFormat::FloatingSymbol;
        fmt.currency_numeric.currency_symbol_text = "$".to_string();
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Currency, &loc);
        assert_eq!(render_value(&Value::Currency(0.0), &spec, &loc), "");
    }

    /// A field storing a trailing currency placement renders the symbol after the amount, even in a
    /// leading-symbol locale — the stored placement wins.
    #[test]
    fn stored_trailing_currency_position_honored() {
        let mut fmt = explicit_fmt();
        fmt.currency_numeric.currency_symbol = CurrencySymbolFormat::FixedSymbol;
        fmt.currency_numeric.currency_symbol_text = "kr".to_string();
        fmt.currency_numeric.currency_position =
            rpt_model::CurrencyPosition::TrailingCurrencyInsideNegative;
        let loc = Locale::from_tag("en-US"); // a leading-symbol locale
        let spec = field_format_spec(Some(&fmt), FieldValueType::Currency, &loc);
        assert_eq!(
            render_value(&Value::Currency(10.0), &spec, &loc),
            " 10.00kr"
        );
    }

    /// The stored `DateTimeSeparator` is placed between the date and time parts when present.
    #[test]
    fn stored_datetime_separator_applied() {
        let mut fmt = explicit_fmt();
        fmt.date.day = DayFormat::NumericDay;
        fmt.date.month = MonthFormat::NumericMonth;
        fmt.date.year = YearFormat::LongYear;
        fmt.date.date_order = rpt_model::DateOrder::MonthDayYear;
        fmt.date.system_default = DateSystemDefaultType::NotUsingWindowsDefaults;
        fmt.date_time.separator = " @ ".to_string();
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::DateTime, &loc);
        assert_eq!(
            render_value(
                &Value::DateTime(Date::new(2004, 1, 3), Time::new(14, 5, 6)),
                &spec,
                &loc
            ),
            "1/3/2004 @ 02:05:06PM"
        );
    }

    /// `DateTimeOrder::DateOnly` collapses a datetime field to its date part — no time component,
    /// no separator, whatever the time sub-format says.
    #[test]
    fn datetime_order_date_only_drops_the_time() {
        let mut fmt = explicit_fmt();
        fmt.date.day = DayFormat::LeadingZeroNumericDay;
        fmt.date.month = MonthFormat::LeadingZeroNumericMonth;
        fmt.date.year = YearFormat::LongYear;
        fmt.date.date_order = rpt_model::DateOrder::MonthDayYear;
        fmt.date.system_default = DateSystemDefaultType::NotUsingWindowsDefaults;
        fmt.date_time.order = DateTimeOrder::DateOnly;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::DateTime, &loc);
        assert_eq!(
            render_value(
                &Value::DateTime(Date::new(2001, 5, 26), Time::new(0, 0, 0)),
                &spec,
                &loc
            ),
            "05/26/2001"
        );
    }

    /// `TimeOnly` is the mirror image: the date part drops out.
    #[test]
    fn datetime_order_time_only_drops_the_date() {
        let mut fmt = explicit_fmt();
        fmt.date_time.order = DateTimeOrder::TimeOnly;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::DateTime, &loc);
        assert_eq!(
            render_value(
                &Value::DateTime(Date::new(2001, 5, 26), Time::new(14, 5, 6)),
                &spec,
                &loc
            ),
            "02:05:06PM"
        );
    }

    /// `TimeThenDate` renders the parts in the stored order, around the same separator.
    #[test]
    fn datetime_order_time_then_date_swaps_the_parts() {
        let mut fmt = explicit_fmt();
        fmt.date.day = DayFormat::NumericDay;
        fmt.date.month = MonthFormat::NumericMonth;
        fmt.date.year = YearFormat::LongYear;
        fmt.date.date_order = rpt_model::DateOrder::MonthDayYear;
        fmt.date.system_default = DateSystemDefaultType::NotUsingWindowsDefaults;
        fmt.date_time.order = DateTimeOrder::TimeThenDate;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::DateTime, &loc);
        assert_eq!(
            render_value(
                &Value::DateTime(Date::new(2004, 1, 3), Time::new(14, 5, 6)),
                &spec,
                &loc
            ),
            "02:05:06PM 1/3/2004"
        );
    }

    /// An explicit date field's stored `DateOrder` is an authoring choice: it wins over the render
    /// locale's component order, so a US-ordered field stays MM/DD/YYYY on an en-GB host. The
    /// separator still comes from the locale (the format leaf stores none).
    #[test]
    fn stored_date_order_wins_over_locale_order() {
        let mut fmt = explicit_fmt();
        fmt.date.day = DayFormat::LeadingZeroNumericDay;
        fmt.date.month = MonthFormat::LeadingZeroNumericMonth;
        fmt.date.year = YearFormat::LongYear;
        fmt.date.date_order = rpt_model::DateOrder::MonthDayYear;
        fmt.date.system_default = DateSystemDefaultType::NotUsingWindowsDefaults;
        let gb = Locale::from_tag("en-GB"); // a day-month-year locale
        let spec = field_format_spec(Some(&fmt), FieldValueType::Date, &gb);
        assert_eq!(
            render_value(&Value::Date(Date::new(2001, 5, 26)), &spec, &gb),
            "05/26/2001"
        );
    }

    /// A *system-default* date field takes its order from the locale — the stored order is inert.
    #[test]
    fn system_default_date_order_follows_locale_not_leaf() {
        let mut fmt = FieldFormat::default();
        fmt.date.date_order = rpt_model::DateOrder::MonthDayYear;
        fmt.date.system_default = DateSystemDefaultType::UseWindowsShortDate;
        let gb = Locale::from_tag("en-GB");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Date, &gb);
        assert_eq!(
            render_value(&Value::Date(Date::new(2001, 5, 26)), &spec, &gb),
            "26/05/2001"
        );
    }

    /// With no stored separator the date and time join with a single space (the default).
    #[test]
    fn datetime_default_separator_is_space() {
        let mut fmt = explicit_fmt();
        fmt.date.day = DayFormat::NumericDay;
        fmt.date.month = MonthFormat::NumericMonth;
        fmt.date.year = YearFormat::LongYear;
        fmt.date.date_order = rpt_model::DateOrder::MonthDayYear;
        fmt.date.system_default = DateSystemDefaultType::NotUsingWindowsDefaults;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::DateTime, &loc);
        assert_eq!(
            render_value(
                &Value::DateTime(Date::new(2004, 1, 3), Time::new(14, 5, 6)),
                &spec,
                &loc
            ),
            "1/3/2004 02:05:06PM"
        );
    }

    /// The date half of the datetime fixtures the stored time cases below are drawn from: an
    /// explicit `MM/DD/YYYY` date joined to the time by the stored two-space separator.
    fn explicit_datetime_fmt(time: TimeFieldFormat) -> FieldFormat {
        let mut fmt = explicit_fmt();
        fmt.date.day = DayFormat::LeadingZeroNumericDay;
        fmt.date.month = MonthFormat::LeadingZeroNumericMonth;
        fmt.date.year = YearFormat::LongYear;
        fmt.date.date_order = rpt_model::DateOrder::MonthDayYear;
        fmt.date.system_default = DateSystemDefaultType::NotUsingWindowsDefaults;
        fmt.date_time.separator = "  ".to_string();
        fmt.time = time;
        fmt
    }

    /// The 24-hour time leaf the datetime fixtures store, with the hour/minute/second elements and
    /// the minute-second separator left to the caller.
    fn stored_time(
        hour: HourFormat,
        minute: MinuteFormat,
        second: SecondFormat,
        minute_second_separator: &str,
    ) -> TimeFieldFormat {
        TimeFieldFormat {
            time_base: TimeBase::TwentyFourHour,
            am_pm_format: AMPMFormat::AMPMAfter,
            hour,
            minute,
            second,
            am_string: " am".to_string(),
            pm_string: " pm".to_string(),
            hour_minute_separator: ":".to_string(),
            minute_second_separator: minute_second_separator.to_string(),
        }
    }

    fn midnight() -> Value {
        Value::DateTime(Date::new(2001, 5, 26), Time::new(0, 0, 0))
    }

    fn render_explicit(time: TimeFieldFormat) -> String {
        let loc = Locale::from_tag("en-US");
        let fmt = explicit_datetime_fmt(time);
        let spec = field_format_spec(Some(&fmt), FieldValueType::DateTime, &loc);
        render_value(&midnight(), &spec, &loc)
    }

    /// A 24-hour field emits no AM/PM designator at all, whatever its stored designator strings
    /// hold — so midnight prints `0:00`, not `12:00AM`, even on a 12-hour host locale.
    #[test]
    fn stored_clock_base_wins_over_the_locale() {
        assert_eq!(
            render_explicit(stored_time(
                HourFormat::NoLeadingZeroNumericHour,
                MinuteFormat::NumericMinute,
                SecondFormat::NoSecond,
                ":",
            )),
            "05/26/2001   0:00"
        );
    }

    /// The "no leading zero" hour still occupies two cells: it is space-padded, not narrowed, so
    /// midnight prints `" 0"` where the leading-zero style prints `"00"`.
    #[test]
    fn no_leading_zero_hour_fills_two_cells() {
        let padded = render_explicit(stored_time(
            HourFormat::NoLeadingZeroNumericHour,
            MinuteFormat::NumericMinute,
            SecondFormat::NoSecond,
            ":",
        ));
        let zeroed = render_explicit(stored_time(
            HourFormat::NumericHour,
            MinuteFormat::NumericMinute,
            SecondFormat::NoSecond,
            ":",
        ));
        assert_eq!(padded, "05/26/2001   0:00");
        assert_eq!(zeroed, "05/26/2001  00:00");
    }

    /// A stored minute-second separator is genuinely the empty string on some fields, which butts
    /// the minute against the second (`0:0000`). Substituting a default `:` would hide it.
    #[test]
    fn empty_minute_second_separator_butts_the_elements() {
        assert_eq!(
            render_explicit(stored_time(
                HourFormat::NoLeadingZeroNumericHour,
                MinuteFormat::NumericMinute,
                SecondFormat::NumericSecond,
                "",
            )),
            "05/26/2001   0:0000"
        );
    }

    /// `NoHour` drops the hour *and* the hour-minute separator that follows it.
    #[test]
    fn no_hour_drops_the_hour_and_its_separator() {
        assert_eq!(
            render_explicit(stored_time(
                HourFormat::NoHour,
                MinuteFormat::NumericMinute,
                SecondFormat::NoSecond,
                "",
            )),
            "05/26/2001  00"
        );
    }

    /// `NoMinute` drops the minute and the (empty) minute-second separator after it; the
    /// hour-minute separator belongs to the still-present hour and stays.
    #[test]
    fn no_minute_keeps_the_hour_minute_separator() {
        assert_eq!(
            render_explicit(stored_time(
                HourFormat::NoLeadingZeroNumericHour,
                MinuteFormat::NoMinute,
                SecondFormat::NumericSecond,
                "",
            )),
            "05/26/2001   0:00"
        );
    }

    /// `NoSecond` leaves no trailing minute-second separator behind.
    #[test]
    fn no_second_drops_the_trailing_separator() {
        assert_eq!(
            render_explicit(stored_time(
                HourFormat::NumericHour,
                MinuteFormat::NumericMinute,
                SecondFormat::NoSecond,
                ":",
            )),
            "05/26/2001  00:00"
        );
    }

    /// A 12-hour field prints its own stored designator text, whose leading space is the whole gap
    /// between the time and the designator.
    #[test]
    fn twelve_hour_uses_the_stored_designator_text() {
        let mut time = stored_time(
            HourFormat::NumericHour,
            MinuteFormat::NumericMinute,
            SecondFormat::NumericSecond,
            ":",
        );
        time.time_base = TimeBase::TwelveHour;
        assert_eq!(render_explicit(time), "05/26/2001  12:00:00 am");
    }

    /// `AMPMBefore` puts the designator ahead of the time.
    #[test]
    fn stored_am_pm_placement_moves_the_designator() {
        let mut time = stored_time(
            HourFormat::NumericHour,
            MinuteFormat::NumericMinute,
            SecondFormat::NoSecond,
            ":",
        );
        time.time_base = TimeBase::TwelveHour;
        time.am_pm_format = AMPMFormat::AMPMBefore;
        time.am_string = "am".to_string();
        assert_eq!(render_explicit(time), "05/26/2001  am12:00");
    }

    /// A system-default field ignores the stored time leaf entirely and takes the locale's clock —
    /// a 24-hour leaf still renders 12-hour with the locale's designator on an en-US host.
    #[test]
    fn system_default_time_follows_the_locale_not_the_leaf() {
        let mut fmt = explicit_datetime_fmt(stored_time(
            HourFormat::NoHour,
            MinuteFormat::NoMinute,
            SecondFormat::NoSecond,
            "",
        ));
        fmt.common.use_system_defaults = true;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::DateTime, &loc);
        assert_eq!(
            render_value(&midnight(), &spec, &loc),
            "5/26/2001  12:00:00AM"
        );
    }

    /// A stored separator is emitted verbatim, not read as a pattern token — an alphabetic one
    /// would otherwise resolve as an hour/minute field.
    #[test]
    fn alphabetic_separator_is_emitted_literally() {
        let mut time = stored_time(
            HourFormat::NumericHour,
            MinuteFormat::NumericMinute,
            SecondFormat::NoSecond,
            "",
        );
        time.hour_minute_separator = "h".to_string();
        assert_eq!(render_explicit(time), "05/26/2001  00h00");
    }

    /// A `Time`-valued field resolves through the same stored leaf as a datetime's time part.
    #[test]
    fn time_valued_field_uses_its_stored_leaf() {
        let loc = Locale::from_tag("en-US");
        let fmt = explicit_datetime_fmt(stored_time(
            HourFormat::NoLeadingZeroNumericHour,
            MinuteFormat::NumericMinute,
            SecondFormat::NoSecond,
            ":",
        ));
        let spec = field_format_spec(Some(&fmt), FieldValueType::Time, &loc);
        assert_eq!(
            render_value(&Value::Time(Time::new(0, 0, 0)), &spec, &loc),
            " 0:00"
        );
    }

    /// A date-only field whose leaf stores all three elements, both element separators, and the
    /// prefix/suffix literals renders them in the stored order: prefix, element, separator, …,
    /// suffix.
    #[test]
    fn stored_date_separators_wrap_and_join_the_elements() {
        let mut fmt = explicit_fmt();
        fmt.date.system_default = DateSystemDefaultType::NotUsingWindowsDefaults;
        fmt.date.date_order = rpt_model::DateOrder::YearMonthDay;
        fmt.date.year = YearFormat::LongYear;
        fmt.date.month = MonthFormat::NumericMonth;
        fmt.date.day = DayFormat::NumericDay;
        fmt.date.prefix_separator = "CCCC".to_string();
        fmt.date.first_separator = "AA".to_string();
        fmt.date.second_separator = "BBB".to_string();
        fmt.date.suffix_separator = "DDDDD".to_string();
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Date, &loc);
        assert_eq!(
            render_value(&Value::Date(Date::new(2023, 12, 31)), &spec, &loc),
            "CCCC2023AA12BBB31DDDDD"
        );
    }

    /// Each ordered slot carries the separator stored after it, so *which* separator survives a
    /// dropped element depends on where that element sat: with no day, a month/day/year field joins
    /// month to year by the first separator and a day/month/year field by the second.
    #[test]
    fn a_dropped_date_element_takes_its_own_separator() {
        let base = || {
            let mut fmt = explicit_fmt();
            fmt.date.system_default = DateSystemDefaultType::NotUsingWindowsDefaults;
            fmt.date.year = YearFormat::LongYear;
            fmt.date.month = MonthFormat::NumericMonth;
            fmt.date.day = DayFormat::NoDay;
            fmt.date.first_separator = "AA".to_string();
            fmt.date.second_separator = "BBB".to_string();
            fmt
        };
        let loc = Locale::from_tag("en-US");
        let render = |fmt: &FieldFormat| {
            let spec = field_format_spec(Some(fmt), FieldValueType::Date, &loc);
            render_value(&Value::Date(Date::new(2024, 1, 7)), &spec, &loc)
        };

        let mut mdy = base();
        mdy.date.date_order = rpt_model::DateOrder::MonthDayYear;
        assert_eq!(render(&mdy), "1AA2024");

        let mut dmy = base();
        dmy.date.date_order = rpt_model::DateOrder::DayMonthYear;
        assert_eq!(render(&dmy), "1BBB2024");
    }

    /// The weekday element and its own separator sit outside the prefix/suffix literals, on the side
    /// the stored `DayOfWeekPosition` names.
    #[test]
    fn stored_day_of_week_sits_outside_the_date() {
        let base = || {
            let mut fmt = explicit_fmt();
            fmt.date.system_default = DateSystemDefaultType::NotUsingWindowsDefaults;
            fmt.date.date_order = rpt_model::DateOrder::YearMonthDay;
            fmt.date.year = YearFormat::LongYear;
            fmt.date.month = MonthFormat::NumericMonth;
            fmt.date.day = DayFormat::NumericDay;
            fmt.date.prefix_separator = "CCCC".to_string();
            fmt.date.first_separator = "AA".to_string();
            fmt.date.second_separator = "BBB".to_string();
            fmt.date.suffix_separator = "DDDDD".to_string();
            fmt.date.day_of_week = DayOfWeekFormat::LongDayOfWeek;
            fmt.date.day_of_week_separator = "EEEEEE".to_string();
            fmt
        };
        let loc = Locale::from_tag("en-US");
        let render = |fmt: &FieldFormat| {
            let spec = field_format_spec(Some(fmt), FieldValueType::Date, &loc);
            render_value(&Value::Date(Date::new(2023, 12, 31)), &spec, &loc)
        };

        let leading = base();
        assert_eq!(render(&leading), "SundayEEEEEECCCC2023AA12BBB31DDDDD");

        let mut trailing = base();
        trailing.date.day_of_week_position = DayOfWeekPosition::TrailingPosition;
        assert_eq!(render(&trailing), "CCCC2023AA12BBB31DDDDDEEEEEESunday");

        // A short weekday abbreviates; `NoDayOfWeek` drops the element and its separator together.
        let mut short = base();
        short.date.day_of_week = DayOfWeekFormat::ShortDayOfWeek;
        assert_eq!(render(&short), "SunEEEEEECCCC2023AA12BBB31DDDDD");
        let mut none = base();
        none.date.day_of_week = DayOfWeekFormat::NoDayOfWeek;
        assert_eq!(render(&none), "CCCC2023AA12BBB31DDDDD");
    }

    /// An alphabetic date separator is emitted verbatim rather than read as a pattern token — `AA`
    /// would otherwise resolve as nothing and `d` as the day.
    #[test]
    fn alphabetic_date_separator_is_emitted_literally() {
        let mut fmt = explicit_fmt();
        fmt.date.system_default = DateSystemDefaultType::NotUsingWindowsDefaults;
        fmt.date.date_order = rpt_model::DateOrder::DayMonthYear;
        fmt.date.year = YearFormat::LongYear;
        fmt.date.month = MonthFormat::NumericMonth;
        fmt.date.day = DayFormat::NumericDay;
        fmt.date.first_separator = " de ".to_string();
        fmt.date.second_separator = " del ".to_string();
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Date, &loc);
        assert_eq!(
            render_value(&Value::Date(Date::new(2024, 1, 7)), &spec, &loc),
            "7 de 1 del 2024"
        );
    }

    /// A *system-default* date field takes the locale's separator, not the stored one — the whole
    /// stored leaf is inert under Windows defaults.
    #[test]
    fn system_default_date_ignores_the_stored_separators() {
        let mut fmt = explicit_fmt();
        fmt.date.system_default = DateSystemDefaultType::UseWindowsShortDate;
        fmt.date.first_separator = "AA".to_string();
        fmt.date.second_separator = "BBB".to_string();
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Date, &loc);
        assert_eq!(
            render_value(&Value::Date(Date::new(2024, 1, 7)), &spec, &loc),
            "1/7/2024"
        );
    }

    /// The era and calendar the leaf stores are not rendered: the engine prints no era designator
    /// under a Gregorian calendar and leaves a Gregorian-stored date unconverted under a Hijri one.
    #[test]
    fn stored_era_and_calendar_do_not_change_the_rendering() {
        let base = || {
            let mut fmt = explicit_fmt();
            fmt.date.system_default = DateSystemDefaultType::NotUsingWindowsDefaults;
            fmt.date.date_order = rpt_model::DateOrder::YearMonthDay;
            fmt.date.year = YearFormat::LongYear;
            fmt.date.month = MonthFormat::NumericMonth;
            fmt.date.day = DayFormat::NumericDay;
            fmt
        };
        let loc = Locale::from_tag("en-US");
        let render = |fmt: &FieldFormat| {
            let spec = field_format_spec(Some(fmt), FieldValueType::Date, &loc);
            render_value(&Value::Date(Date::new(2023, 12, 31)), &spec, &loc)
        };
        let plain = render(&base());
        assert_eq!(plain, "2023/12/31");

        let mut era = base();
        era.date.era = rpt_model::EraFormat::LongEra;
        assert_eq!(render(&era), plain);

        let mut hijri = base();
        hijri.date.calendar = rpt_model::CalendarType::HijriCalendar;
        assert_eq!(render(&hijri), plain);
    }

    /// `UseLeadZero` off drops the integer `0` of a value below one.
    #[test]
    fn stored_use_lead_zero_off_drops_the_leading_zero() {
        let mut fmt = explicit_fmt();
        fmt.numeric.use_lead_zero = false;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(0.25), &spec, &loc), " .25");
        // The same field with the flag on keeps it.
        fmt.numeric.use_lead_zero = true;
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(0.25), &spec, &loc), " 0.25");
    }

    /// `DisplayReverseSign` flips the rendered sign, and does so before the stored negative style is
    /// applied — so a reversed positive is bracketed.
    #[test]
    fn stored_display_reverse_sign_flips_the_rendering() {
        let mut fmt = explicit_fmt();
        fmt.numeric.display_reverse_sign = true;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(1.0), &spec, &loc), "-1.00");
        assert_eq!(render_value(&Value::Number(-1.0), &spec, &loc), " 1.00");

        fmt.numeric.negative = NegativeFormat::Bracketed;
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(1.0), &spec, &loc), "(1.00)");
    }

    /// A stored `ZeroValueString` replaces a zero value, on a currency field symbol and all.
    #[test]
    fn stored_zero_value_string_replaces_the_zero() {
        let mut fmt = explicit_fmt();
        fmt.numeric.zero_value_string = "n/a".to_string();
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(0.0), &spec, &loc), "n/a");
        assert_eq!(render_value(&Value::Number(1.0), &spec, &loc), " 1.00");

        fmt.currency_numeric.zero_value_string = "n/a".to_string();
        fmt.currency_numeric.currency_symbol = CurrencySymbolFormat::FloatingSymbol;
        fmt.currency_numeric.currency_symbol_text = "$".to_string();
        let spec = field_format_spec(Some(&fmt), FieldValueType::Currency, &loc);
        assert_eq!(render_value(&Value::Currency(0.0), &spec, &loc), "n/a");
    }

    /// A field that stores both a zero literal and `EnableSuppressIfZero` prints neither: the
    /// suppression wins and the zero row is blank.
    #[test]
    fn stored_suppress_if_zero_beats_the_stored_zero_value_string() {
        let mut fmt = explicit_fmt();
        fmt.numeric.zero_value_string = "n/a".to_string();
        fmt.numeric.suppress_if_zero = true;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(0.0), &spec, &loc), "");
    }

    /// `<Default Format>` is the engine's marker for "no zero literal", not a literal — a zero must
    /// render as a number, never as the marker text.
    #[test]
    fn the_default_format_marker_is_never_printed() {
        let mut fmt = explicit_fmt();
        fmt.numeric.zero_value_string = "<Default Format>".to_string();
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(0.0), &spec, &loc), " 0.00");
    }

    #[test]
    fn boolean_output_type_maps_words() {
        let mut fmt = FieldFormat::default();
        fmt.boolean.output_type = BooleanOutputType::YesOrNo;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Boolean, &loc);
        assert_eq!(render_value(&Value::Bool(true), &spec, &loc), "Yes");
    }
}
