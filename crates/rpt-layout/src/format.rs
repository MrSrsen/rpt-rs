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
    format_bool, format_currency, format_date_in, format_datetime_in, format_number,
    format_time_in, BoolFormat, CurrencyFormat, DateFormat, DateOrder, FormatSpec, Locale,
    NegativeStyle, NumberFormat, TimeFormat,
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
        T::DateTime => FormatSpec::DateTime(date_spec(fmt, loc), time_spec(loc)),
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
        (Value::DateTime(d, t), FormatSpec::DateTime(df, tf)) => {
            format_datetime_in(*d, *t, df, tf, loc)
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
    // Integer value types show no decimals and no grouping by default (Crystal's integer default).
    if matches!(
        vt,
        FieldValueType::Int8s
            | FieldValueType::Int16s
            | FieldValueType::Int32s
            | FieldValueType::Int32u
    ) {
        nf.decimals = 0;
        nf.use_thousands = false;
    }
    if let Some(f) = fmt {
        if !f.common.use_system_defaults {
            if f.numeric.decimal_places >= 0 {
                nf.decimals = f.numeric.decimal_places as u32;
                nf.use_thousands = true;
            }
            nf.negative = map_negative(f.numeric.negative);
        }
    }
    nf
}

/// A [`CurrencyFormat`] when the field shows a symbol, else a plain [`NumberFormat`].
fn currency_or_number(fmt: Option<&FieldFormat>, vt: FieldValueType, loc: &Locale) -> FormatSpec {
    let number = numeric_spec(fmt, vt, loc);
    // System-default Currency shows the symbol; an explicit field shows it unless set to NoSymbol.
    let show = match fmt {
        Some(f) if !f.common.use_system_defaults => {
            f.numeric.currency_symbol != CurrencySymbolFormat::NoSymbol
        }
        _ => true,
    };
    if show {
        FormatSpec::Currency(CurrencyFormat {
            number,
            symbol: loc.currency_symbol.to_string(),
            position: loc.currency_position,
        })
    } else {
        FormatSpec::Number(number)
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

/// The locale's system-default date pattern: leading-zero numeric day/month + long year (the form
/// Windows' short date reports), ordered per the locale, or a long form with
/// the full month name.
fn default_date_pattern(loc: &Locale, long: bool) -> String {
    if long {
        return match loc.date_order {
            DateOrder::MonthDayYear => "MMMM d, yyyy".to_string(),
            DateOrder::DayMonthYear => "d MMMM yyyy".to_string(),
            DateOrder::YearMonthDay => "yyyy MMMM d".to_string(),
        };
    }
    order_join(loc, "dd", "MM", "yyyy")
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
    use rpt_format_value::Date;

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

    #[test]
    fn boolean_output_type_maps_words() {
        let mut fmt = FieldFormat::default();
        fmt.boolean.output_type = BooleanOutputType::YesOrNo;
        let loc = Locale::from_tag("en-US");
        let spec = field_format_spec(Some(&fmt), FieldValueType::Boolean, &loc);
        assert_eq!(render_value(&Value::Bool(true), &spec, &loc), "Yes");
    }
}
