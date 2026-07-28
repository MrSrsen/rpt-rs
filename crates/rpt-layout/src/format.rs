//! Resolve a field's **effective** display format by merging the two layers Crystal uses:
//!
//! 1. the **locale** (`--locale` / host) — the "system default" layer: separators, month/day names,
//!    AM/PM, default date order, default decimals, currency symbol; and
//! 2. the field's **stored** [`FieldFormat`] leaf — the explicit authoring choices (decimals,
//!    negative style, currency symbol placement, date component forms, boolean word pair).
//!
//! The switch that arbitrates them lives in the field itself: [`CommonFieldFormat::use_system_defaults`]
//! (the master flag) and [`DateFieldFormat::system_default`]. When a field uses system defaults, the
//! locale supplies the effective format; otherwise the stored leaf wins for the attributes it sets,
//! with names/separators still taken from the locale (Crystal never stores "January", only
//! [`MonthFormat::LongMonth`]).

use crystal_formula::eval::Value;
use rpt_format_value::{
    format_bool, format_currency, format_date_in, format_number, format_time_in, BoolFormat,
    CurrencyFormat, CurrencyPosition, DateFormat, DateOrder, FormatSpec, Locale, NegativeStyle,
    NumberFormat, TimeFormat,
};
use rpt_model::{
    BooleanOutputType, CurrencySymbolFormat, DateSystemDefaultType, DayFormat, FieldFormat,
    FieldValueType, MonthFormat, NegativeFormat, YearFormat,
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
            FormatSpec::Number(numeric_spec(fmt, vt, loc))
        }
        T::Currency => currency_or_number(fmt, vt, loc),
        T::Date => FormatSpec::Date(date_spec(fmt, loc)),
        T::Time => FormatSpec::Time(time_spec(loc)),
        T::DateTime => {
            FormatSpec::DateTime(date_spec(fmt, loc), time_spec(loc), datetime_separator(fmt))
        }
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
        (Value::DateTime(d, t), FormatSpec::DateTime(df, tf, sep)) => {
            format!(
                "{}{}{}",
                format_date_in(*d, df, loc),
                sep,
                format_time_in(*t, tf, loc)
            )
        }
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
    let spec = field_format_spec(None, vt, loc);
    render_value(value, &spec, loc)
}

fn numeric_spec(fmt: Option<&FieldFormat>, vt: FieldValueType, loc: &Locale) -> NumberFormat {
    let mut nf = loc.number_format();
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
    if let Some(f) = fmt {
        if !f.common.use_system_defaults {
            if f.numeric.decimal_places >= 0 {
                nf.decimals = f.numeric.decimal_places as u32;
            }
            // The field's stored grouping/suppression choices win over the locale baseline.
            nf.use_thousands = f.numeric.thousands_separator;
            nf.suppress_if_zero = f.numeric.suppress_if_zero;
            nf.negative = map_negative(f.numeric.negative);
        }
    }
    nf
}

/// A [`CurrencyFormat`] when the field shows a symbol, else a plain [`NumberFormat`].
///
/// The symbol is stored **per field** (`currency_symbol_text`, e.g. `"€"`, `"kr "`, `"Kč"`), so two
/// fields in one report can carry two different currencies. An explicit field (not using system
/// defaults) uses its own stored symbol and NoSymbol/Fixed/Floating choice, and its own stored
/// leading/trailing placement; a system-default field resolves both the symbol and its placement
/// from the render locale (the host regional setting — Crystal keeps no report-level default
/// currency). Any spacing around a trailing symbol is baked into the stored symbol string itself.
fn currency_or_number(fmt: Option<&FieldFormat>, vt: FieldValueType, loc: &Locale) -> FormatSpec {
    let number = numeric_spec(fmt, vt, loc);
    // Resolve whether a symbol shows, which one, and where it sits. NoSymbol on an explicit field
    // drops to a plain number; otherwise prefer the field's stored symbol string and stored
    // placement, falling back to the locale when the field stored none (or defers to system defaults).
    let (show, symbol, position) = match fmt {
        Some(f) if !f.common.use_system_defaults => {
            let show = f.numeric.currency_symbol != CurrencySymbolFormat::NoSymbol;
            let symbol = if f.numeric.currency_symbol_text.is_empty() {
                loc.currency_symbol.to_string()
            } else {
                f.numeric.currency_symbol_text.clone()
            };
            (
                show,
                symbol,
                map_currency_position(f.numeric.currency_position),
            )
        }
        _ => (true, loc.currency_symbol.to_string(), loc.currency_position),
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

/// The string placed between a datetime's date and time parts: the field's stored
/// `DateTimeSeparator` when it set one, else a single space (the engine's default join).
fn datetime_separator(fmt: Option<&FieldFormat>) -> String {
    match fmt {
        Some(f) if !f.date_time.separator.is_empty() => f.date_time.separator.clone(),
        _ => " ".to_string(),
    }
}

fn time_spec(loc: &Locale) -> TimeFormat {
    // Time is host-locale gated even for explicit fields, so the effective time format comes from
    // the locale's clock, not the stored leaf.
    TimeFormat {
        pattern: if loc.twelve_hour {
            "h:mm:sstt".to_string()
        } else {
            "HH:mm:ss".to_string()
        },
    }
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
            pattern: pattern_from_components(f.date.day, f.date.month, f.date.year, loc),
        }
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

/// Build a date pattern from the field's stored day/month/year component forms, ordered per locale.
/// A `No*` component drops out (and takes an adjacent separator with it).
fn pattern_from_components(
    day: DayFormat,
    month: MonthFormat,
    year: YearFormat,
    loc: &Locale,
) -> String {
    let d = match day {
        DayFormat::NumericDay => "d",
        DayFormat::LeadingZeroNumericDay => "dd",
        _ => "",
    };
    let m = match month {
        MonthFormat::NumericMonth => "M",
        MonthFormat::LeadingZeroNumericMonth => "MM",
        MonthFormat::ShortMonth => "MMM",
        MonthFormat::LongMonth => "MMMM",
        _ => "",
    };
    let y = match year {
        YearFormat::ShortYear => "yy",
        YearFormat::LongYear => "yyyy",
        _ => "",
    };
    // Order the present components and join with the locale separator.
    let ordered: [&str; 3] = match loc.date_order {
        DateOrder::MonthDayYear => [m, d, y],
        DateOrder::DayMonthYear => [d, m, y],
        DateOrder::YearMonthDay => [y, m, d],
    };
    let parts: Vec<&str> = ordered.into_iter().filter(|s| !s.is_empty()).collect();
    parts.join(&loc.date_sep.to_string())
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

    fn de() -> Locale {
        Locale::from_tag("de-DE")
    }

    /// Build a stored field format that opts out of system defaults (so its explicit attributes win).
    fn explicit_fmt() -> FieldFormat {
        let mut f = FieldFormat::default();
        f.common.use_system_defaults = false;
        f
    }

    #[test]
    fn number_uses_locale_when_system_default() {
        let spec = field_format_spec(None, FieldValueType::Number, &de());
        assert_eq!(
            render_value(&Value::Number(1234.5), &spec, &de()),
            "1.234,50"
        );
    }

    #[test]
    fn integer_field_groups_thousands_with_no_decimals() {
        // A system-default integer field shows no decimals but still groups thousands, like the engine
        // (`1,002`, not `1002`).
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(None, FieldValueType::Int32s, &loc);
        assert_eq!(render_value(&Value::Number(1002.0), &spec, &loc), "1,002");
    }

    #[test]
    fn explicit_decimals_override_locale_default() {
        let mut fmt = explicit_fmt();
        fmt.numeric.decimal_places = 0;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        // 0 explicit decimals wins over the locale's default of 2.
        assert_eq!(render_value(&Value::Number(1234.5), &spec, &loc), "1,235");
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

    #[test]
    fn explicit_date_components_ordered_per_locale() {
        let mut fmt = explicit_fmt();
        fmt.date.day = DayFormat::NumericDay;
        fmt.date.month = MonthFormat::LongMonth;
        fmt.date.year = YearFormat::LongYear;
        fmt.date.system_default = DateSystemDefaultType::NotUsingWindowsDefaults;
        let loc = de();
        let spec = field_format_spec(Some(&fmt), FieldValueType::Date, &loc);
        // DMY order, German month name, '.' separator.
        assert_eq!(
            render_value(&Value::Date(Date::new(2004, 3, 5)), &spec, &loc),
            "5.März.2004"
        );
    }

    /// An explicit field carrying a stored currency symbol renders with that symbol, not the
    /// locale's — the per-field currency wins.
    #[test]
    fn explicit_currency_symbol_wins_over_locale() {
        let mut fmt = explicit_fmt();
        fmt.numeric.currency_symbol = CurrencySymbolFormat::FloatingSymbol;
        fmt.numeric.currency_symbol_text = "€".to_string();
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
        eur.numeric.currency_symbol = CurrencySymbolFormat::FloatingSymbol;
        eur.numeric.currency_symbol_text = "€".to_string();
        let mut czk = explicit_fmt();
        czk.numeric.currency_symbol = CurrencySymbolFormat::FloatingSymbol;
        czk.numeric.currency_symbol_text = "Kč".to_string();
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
        fmt.numeric.currency_symbol = CurrencySymbolFormat::FixedSymbol;
        fmt.numeric.currency_symbol_text = "kr ".to_string();
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
        assert_eq!(render_value(&Value::Currency(10.0), &spec, &loc), "10.00");
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
        assert_eq!(render_value(&Value::Number(1234.5), &spec, &loc), "1234.50");
    }

    /// `EnableSuppressIfZero` blanks a zero value; a non-zero value in the same field is unaffected.
    #[test]
    fn stored_suppress_if_zero_blanks_zero() {
        let mut fmt = explicit_fmt();
        fmt.numeric.suppress_if_zero = true;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(0.0), &spec, &loc), "");
        assert_eq!(render_value(&Value::Number(12.5), &spec, &loc), "12.50");
    }

    /// Without the flag a zero renders normally (the default, unchanged behavior).
    #[test]
    fn zero_without_suppress_renders_normally() {
        let mut fmt = explicit_fmt();
        fmt.numeric.suppress_if_zero = false;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Number, &loc);
        assert_eq!(render_value(&Value::Number(0.0), &spec, &loc), "0.00");
    }

    /// A suppressed zero currency blanks the whole field, symbol included.
    #[test]
    fn stored_suppress_if_zero_blanks_currency() {
        let mut fmt = explicit_fmt();
        fmt.numeric.suppress_if_zero = true;
        fmt.numeric.currency_symbol = CurrencySymbolFormat::FloatingSymbol;
        fmt.numeric.currency_symbol_text = "$".to_string();
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Currency, &loc);
        assert_eq!(render_value(&Value::Currency(0.0), &spec, &loc), "");
    }

    /// A field storing a trailing currency placement renders the symbol after the amount, even in a
    /// leading-symbol locale — the stored placement wins.
    #[test]
    fn stored_trailing_currency_position_honored() {
        let mut fmt = explicit_fmt();
        fmt.numeric.currency_symbol = CurrencySymbolFormat::FixedSymbol;
        fmt.numeric.currency_symbol_text = "kr".to_string();
        fmt.numeric.currency_position = rpt_model::CurrencyPosition::TrailingCurrencyInsideNegative;
        let loc = Locale::from_tag("en-US"); // a leading-symbol locale
        let spec = field_format_spec(Some(&fmt), FieldValueType::Currency, &loc);
        assert_eq!(render_value(&Value::Currency(10.0), &spec, &loc), "10.00kr");
    }

    /// The stored `DateTimeSeparator` is placed between the date and time parts when present.
    #[test]
    fn stored_datetime_separator_applied() {
        let mut fmt = explicit_fmt();
        fmt.date.day = DayFormat::NumericDay;
        fmt.date.month = MonthFormat::NumericMonth;
        fmt.date.year = YearFormat::LongYear;
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
            "1/3/2004 @ 2:05:06PM"
        );
    }

    /// With no stored separator the date and time join with a single space (the default).
    #[test]
    fn datetime_default_separator_is_space() {
        let mut fmt = explicit_fmt();
        fmt.date.day = DayFormat::NumericDay;
        fmt.date.month = MonthFormat::NumericMonth;
        fmt.date.year = YearFormat::LongYear;
        fmt.date.system_default = DateSystemDefaultType::NotUsingWindowsDefaults;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::DateTime, &loc);
        assert_eq!(
            render_value(
                &Value::DateTime(Date::new(2004, 1, 3), Time::new(14, 5, 6)),
                &spec,
                &loc
            ),
            "1/3/2004 2:05:06PM"
        );
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
