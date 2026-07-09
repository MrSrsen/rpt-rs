//! Derived **effective** `FieldObject.FieldFormat` — the date/time/numeric display format the
//! Crystal engine *reports* for a placed field at runtime.
//!
//! `rpt` decodes the **stored** format leaf (records `0x00ee–0x00fb`) faithfully. But when a field
//! **uses system defaults** (`CommonFieldFormat.EnableUseSystemDefaults == true`) the engine ignores
//! the stored date/time leaf and **resolves** the effective format at runtime from the field's
//! **value type** (plus, for the clock/AM-PM strings, the host locale): a date-valued field reports
//! `NumericDay`/`NumericMonth`/`LongYear`, and a non-numeric field reports
//! `DecimalPlaces=2`/`RoundToHundredth`, regardless of the stored leaf. A field with
//! `EnableUseSystemDefaults == false` instead carries an explicit **per-field** stored date/time
//! format (the `0x00f2`/`0x00f6` leaf bytes) which the engine reports verbatim — decoded in the `rpt`
//! layer, and this derivation must not override it. See [`effective_field_format`].
//!
//! This module is the derive-layer counterpart to `rpt`'s stored decode (the same boundary as
//! [`formula::string_max_bytes`](super::formula::string_max_bytes) is for `NumberOfBytes`). It first
//! resolves a placed field object's **effective value type** — DB/summary objects already carry it;
//! formula / parameter / running-total / SQL-expression / special objects have it in their field
//! *definition* (or, for special fields, in their kind) but not on the object — then maps that value
//! type to the effective date/time/numeric display format.
//!
//! ## Reproducible vs locale-specific
//! For **system-default** fields, the **date** format (`DayFormat`/`MonthFormat`/`YearFormat`) and
//! the **numeric defaults** (`DecimalPlaces`/`RoundingFormat`) are a pure function of the value type
//! and reproducible from the file alone. The **time** format (`TimeBase`/`AMString`/`PMString`/
//! `HourFormat`…) is **host-locale** dependent — byte-identical stored leaves render differently
//! depending on the authoring machine's locale — so it is not reproducible from the file; the
//! [`RenderLocale`] parameter (default: US 12-hour) selects it for eval/render consumers.
//!
//! For **non**-system-default fields, a **date-valued** field's effective date is its **stored**
//! `0x00f2` day/month/year (decoded into [`DateFieldFormat`]); a non-date field reports the generic
//! default. Value-type resolution ([`field_object_value_type`], which follows the object's
//! `data_source` because the object's `ref_kind` is often `Unknown`) makes the whole
//! `DateFieldFormat.{Day,Month,Year}Format` surface file-derivable. `TimeFieldFormat.*` and the
//! numeric decimal/leading-zero display stay host-locale gated (the [`RenderLocale`] selects them).

use rpt::model::{
    DateFieldFormat, DayFormat, FieldObject, FieldValueType, MonthFormat, NegativeFormat,
    NumericFieldFormat, Report, RoundingFormat, YearFormat,
};
use rpt_format_value::DateOrder;

/// Host-locale rendering parameters for the runtime-resolved time/clock strings and numeric
/// precision defaults that are *not* stored in the `.rpt`. The engine reads
/// these from the authoring machine's locale at load; byte-identical stored leaves render
/// differently depending on that locale (e.g. 24-hour / lowercase `am` vs. US 12-hour / ` AM`), so
/// they cannot be reproduced from the file alone. The [`Default`] is the US 12-hour locale; a
/// consumer that knows the target machine's locale supplies a different one so the derived time /
/// numeric format matches that machine.
///
/// No locale id is persisted in the report itself (`PrintOptions` carries paper/margins, not a
/// locale) — this is an **external render input**, the counterpart of a live datasource for
/// `NumberOfBytes`.
// Some fields are the locale surface a locale-aware consumer supplies but the XML exporter's
// default (US 12-hour) resolution path does not yet read; kept as part of the effective-format model.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RenderLocale {
    /// `true` => 12-hour clock with AM/PM (`On12Hour`); `false` => 24-hour (`On24Hour`, empty AM/PM).
    pub twelve_hour: bool,
    /// The AM designator string (US default `" AM"`; a 24-hour locale reports `""`).
    pub am_string: &'static str,
    /// The PM designator string (US default `" PM"`; a 24-hour locale reports `""`).
    pub pm_string: &'static str,
    /// Display order of the date components when the engine resolves a *system-default* date format
    /// from the locale (`DayMonthYear` / `MonthDayYear` / `YearMonthDay`). Currently informational —
    /// the exporter's system-default date form is component-order-independent (it only reports
    /// day/month/year *formats*, not their order) — but carried so a locale-aware consumer has it.
    pub date_order: DateOrder,
    /// The decimal places the engine reports for a non-numeric field's system-default number format
    /// (US default `2`). Locale-gated: some locales report a different value.
    pub default_decimal_places: i32,
    /// The day component the host's Windows date pattern renders (default `LeadingZeroNumericDay`).
    /// Used when a date field's stored `SystemDefaultType` says to follow the Windows long/short
    /// pattern.
    pub windows_day: DayFormat,
    /// The month component of the host's Windows date pattern (default `LeadingZeroNumericMonth`).
    pub windows_month: MonthFormat,
}

impl Default for RenderLocale {
    fn default() -> Self {
        Self {
            twelve_hour: true,
            am_string: " AM",
            pm_string: " PM",
            date_order: DateOrder::MonthDayYear,
            default_decimal_places: 2,
            windows_day: DayFormat::LeadingZeroNumericDay,
            windows_month: MonthFormat::LeadingZeroNumericMonth,
        }
    }
}

/// The runtime-resolved date sub-format (`<DateFieldFormat>`). Carries the SDK enum values the engine
/// reports — either the field's **stored** `0x00f2` day/month/year (a date-valued non-system-default
/// field) or a value-type-derived default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveDateFormat {
    pub day: DayFormat,
    pub month: MonthFormat,
    pub year: YearFormat,
}

/// The runtime-resolved time sub-format (`<TimeFieldFormat>`). Locale-selected fields are `&str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveTimeFormat {
    pub hour: &'static str,
    pub minute: &'static str,
    pub second: &'static str,
    pub ampm: &'static str,
    pub am_string: &'static str,
    pub pm_string: &'static str,
    pub hour_minute_sep: &'static str,
    pub minute_second_sep: &'static str,
    pub time_base: &'static str,
}

/// The runtime effective numeric sub-format: the stored numeric enums (kept as-is) plus the
/// value-type-resolved defaults the engine reports for a non-numeric field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveNumericFormat {
    pub decimal_places: i32,
    pub rounding: RoundingFormat,
    pub negative: NegativeFormat,
    pub currency_symbol: rpt::model::CurrencySymbolFormat,
    pub leading_zero: bool,
}

/// The full runtime-resolved `<FieldFormat>` display, keyed on the field's effective value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveFieldFormat {
    pub value_type: FieldValueType,
    pub date: EffectiveDateFormat,
    pub time: EffectiveTimeFormat,
    pub numeric: EffectiveNumericFormat,
}

/// Whether a value type renders through the numeric formatter (drives the numeric defaults + the
/// `EnableUseLeadingZero` flag).
fn is_numeric(vt: FieldValueType) -> bool {
    use FieldValueType as V;
    matches!(
        vt,
        V::Int8s | V::Int16s | V::Int32s | V::Int32u | V::Number | V::Currency
    )
}

/// The value type a **special** field (`PrintDate`, `PageNofM`, …) renders as. The date/time
/// specials carry a real Date/Time value; page/record/group counters are Number; everything else
/// (page-N-of-M, titles, paths) renders as a String.
fn special_value_type(name: &str) -> FieldValueType {
    use FieldValueType as V;
    match name {
        "PrintDate" | "ModificationDate" | "DataDate" => V::Date,
        "PrintTime" | "ModificationTime" | "DataTime" => V::Time,
        "PageNumber" | "RecordNumber" | "GroupNumber" | "TotalPageCount" => V::Number,
        _ => V::String,
    }
}

/// Strip the display braces/prefix of a field reference to the bare definition name:
/// `{@From Date}` → `From Date`, `{?p}` → `p`, `{#rt}` → `rt`, `{%sql}` → `sql`.
fn ref_name(ds: &str) -> &str {
    super::formula::split_reference(ds.trim_start_matches('{').trim_end_matches('}')).1
}

/// Look up a field definition's value type by name, preferring a definition of `prefer` kind when
/// several defs share the name (e.g. a `String` DB field and a `Date` formula both named
/// `reportDate` — a `{@reportDate}` reference means the formula). Matches the reference name exactly,
/// then falls back to its last `.`-separated segment (`{Command.plannedDispenseDate}` →
/// `plannedDispenseDate`), so a table-qualified DB reference still resolves to its bare-named def.
fn lookup_value_type(
    report: &Report,
    name: &str,
    prefer: Option<rpt::model::FieldKind>,
) -> FieldValueType {
    let defs = &report.data_definition.field_definitions;
    let by =
        |n: &str| -> Vec<&rpt::model::FieldDef> { defs.iter().filter(|d| d.name == n).collect() };
    let mut matches = by(name);
    if matches.is_empty() {
        if let Some(short) = name.rsplit('.').next() {
            matches = by(short);
        }
    }
    if let Some(k) = prefer {
        if let Some(d) = matches.iter().find(|d| d.kind.field_kind() == k) {
            return d.value_type;
        }
    }
    matches
        .iter()
        .map(|d| d.value_type)
        .find(|v| *v != FieldValueType::Unknown)
        .or_else(|| matches.first().map(|d| d.value_type))
        .unwrap_or(FieldValueType::Unknown)
}

/// Resolve a placed field object's **effective value type** — the type the engine uses to pick the
/// runtime display format. DB and summary objects already carry it on the object; every other kind
/// is resolved from its `data_source` reference (the object's `ref_kind` is frequently `Unknown` —
/// the sigil lives only in the `data_source` string), by looking up the referenced definition.
pub fn field_object_value_type(report: &Report, field: &FieldObject) -> FieldValueType {
    use rpt::model::FieldKind;
    // DB / summary field objects are already bound by `rpt` (their `value_type` is set); trust it.
    if field.value_type != FieldValueType::Unknown {
        return field.value_type;
    }
    let ds = field.data_source.trim();
    // `GroupName ({group field}, ["token"])` renders as its grouped field's type.
    if let Some(rest) = ds.strip_prefix("GroupName") {
        return match first_brace_ref(rest) {
            Some(inner) => lookup_value_type(report, inner, None),
            None => FieldValueType::String,
        };
    }
    // A braced reference — the sigil selects which definition kind to prefer on a name collision.
    if let Some(inner) = ds.strip_prefix('{') {
        let (name, prefer) = match inner.as_bytes().first().copied() {
            Some(b'@') => (ref_name(ds), Some(FieldKind::FormulaField)),
            Some(b'?') => (ref_name(ds), Some(FieldKind::ParameterField)),
            Some(b'#') => (ref_name(ds), Some(FieldKind::RunningTotalField)),
            Some(b'%') => (ref_name(ds), Some(FieldKind::SqlExpressionField)),
            _ => (ref_name(ds), None), // a DB field `{table.field}`
        };
        return lookup_value_type(report, name, prefer);
    }
    // A bare spaceless kind name is a special field (`PrintDate`, `PageNofM`, …).
    special_value_type(ds)
}

/// The bare definition name inside the first `{…}` of `s` (`" ({Command.d}, "Daily")"` →
/// `Command.d`), or `None` if there is no brace pair.
fn first_brace_ref(s: &str) -> Option<&str> {
    super::formula::brace_groups(s)
        .next()
        .map(|g| g[1..g.len() - 1].trim())
}

/// Resolve the effective date sub-format the engine reports:
/// - **non-date** field (any flags): the generic default date form (leading-zero numeric day/month,
///   short year), ignoring the stored leaf.
/// - **date-valued, `CommonFieldFormat.EnableUseSystemDefaults == true`**: the active system form
///   (numeric day/month, long year), locale-invariant.
/// - **date-valued, non-system-default**: the field's **stored** `0x00f2` day/month/year, verbatim.
///
/// The stored [`DateSystemDefaultType`](rpt::model::DateSystemDefaultType) does *not* gate this: a
/// non-system-default date field's stored day/month/year already reflect the resolved Windows
/// long/short pattern when that flag is set, so the engine reports them as-is — the flag is decoded
/// but otherwise inert here.
fn effective_date(
    vt: FieldValueType,
    use_system_defaults: bool,
    stored: &DateFieldFormat,
) -> EffectiveDateFormat {
    if !matches!(vt, FieldValueType::Date | FieldValueType::DateTime) {
        EffectiveDateFormat {
            day: DayFormat::LeadingZeroNumericDay,
            month: MonthFormat::LeadingZeroNumericMonth,
            year: YearFormat::ShortYear,
        }
    } else if use_system_defaults {
        EffectiveDateFormat {
            day: DayFormat::NumericDay,
            month: MonthFormat::NumericMonth,
            year: YearFormat::LongYear,
        }
    } else {
        EffectiveDateFormat {
            day: stored.day,
            month: stored.month,
            year: stored.year,
        }
    }
}

/// Resolve the effective time sub-format from a value type + host locale. A Time/DateTime field drops
/// the leading zero on the hour; the clock base / AM-PM strings come from [`RenderLocale`].
fn effective_time(vt: FieldValueType, locale: &RenderLocale) -> EffectiveTimeFormat {
    let hour = if matches!(vt, FieldValueType::Time | FieldValueType::DateTime) {
        "NumericHourNoLeadingZero"
    } else {
        "NumericHour"
    };
    let (am, pm, base) = if locale.twelve_hour {
        (locale.am_string, locale.pm_string, "On12Hour")
    } else {
        ("", "", "On24Hour")
    };
    EffectiveTimeFormat {
        hour,
        minute: "NumericMinute",
        second: "NumericSecond",
        ampm: "AMPMAfter",
        am_string: am,
        pm_string: pm,
        hour_minute_sep: ":",
        minute_second_sep: ":",
        time_base: base,
    }
}

/// Resolve the effective numeric sub-format. A numeric field reports its **stored** enums (byte-exact
/// in `rpt`); a non-numeric field reports the engine defaults (2 decimals, `RoundToHundredth`). The
/// leading-zero flag is value-type-derived (numeric ⇒ on).
fn effective_numeric(vt: FieldValueType, stored: &NumericFieldFormat) -> EffectiveNumericFormat {
    if is_numeric(vt) {
        EffectiveNumericFormat {
            decimal_places: stored.decimal_places,
            rounding: stored.rounding,
            negative: stored.negative,
            currency_symbol: stored.currency_symbol,
            leading_zero: true,
        }
    } else {
        EffectiveNumericFormat {
            decimal_places: 2,
            rounding: RoundingFormat::RoundToHundredth,
            negative: stored.negative,
            currency_symbol: stored.currency_symbol,
            leading_zero: false,
        }
    }
}

/// Compute the full runtime-resolved `<FieldFormat>` display for a placed field object.
///
/// - **Date** (`DayFormat`/`MonthFormat`/`YearFormat`): a date-valued **non-system-default** field
///   reports its **stored** `0x00f2` day/month/year (decoded in `rpt`); a
///   date-valued **system-default** field reports the active locale form (numeric day/month, long
///   year); a non-date field reports the generic default (leading-zero day/month, short year). Fully
///   file-derivable once the field's value type is resolved — see [`effective_date`].
/// - **Time**: host-locale gated even for non-system-default fields (not file-derivable); the
///   [`RenderLocale`] selects the clock/AM-PM strings, resolved from the value type only when
///   `use_system_defaults` is set.
/// - **Numeric**: numeric fields report their stored enums; non-numeric fields report the engine
///   defaults.
pub fn effective_field_format(
    report: &Report,
    field: &FieldObject,
    locale: &RenderLocale,
) -> EffectiveFieldFormat {
    let vt = field_object_value_type(report, field);
    let stored_numeric = field
        .format
        .as_ref()
        .map(|f| f.numeric.clone())
        .unwrap_or_default();
    let stored_date = field.format.as_ref().map(|f| f.date).unwrap_or_default();
    let use_system_defaults = field
        .format
        .as_ref()
        .map(|f| f.common.use_system_defaults)
        .unwrap_or(true);
    // Time is resolved from the value type only when system defaults are in effect (it is locale-gated).
    let time_vt = if use_system_defaults {
        vt
    } else {
        FieldValueType::Unknown
    };
    EffectiveFieldFormat {
        value_type: vt,
        date: effective_date(vt, use_system_defaults, &stored_date),
        time: effective_time(time_vt, locale),
        numeric: effective_numeric(vt, &stored_numeric),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt::model::FieldValueType as V;

    #[test]
    fn ref_name_strips_sigils() {
        assert_eq!(ref_name("{@From Date}"), "From Date");
        assert_eq!(ref_name("{?param}"), "param");
        assert_eq!(ref_name("{#rt}"), "rt");
        assert_eq!(ref_name("{%sql}"), "sql");
    }

    #[test]
    fn special_field_value_types() {
        assert_eq!(special_value_type("PrintDate"), V::Date);
        assert_eq!(special_value_type("ModificationDate"), V::Date);
        assert_eq!(special_value_type("PrintTime"), V::Time);
        assert_eq!(special_value_type("PageNumber"), V::Number);
        assert_eq!(special_value_type("PageNofM"), V::String);
    }

    #[test]
    fn date_stored_vs_system_default_vs_non_date() {
        let stored = DateFieldFormat {
            day: DayFormat::NoDay,
            month: MonthFormat::LongMonth,
            year: YearFormat::LongYear,
            ..Default::default()
        };
        // Date/DateTime + system default → active locale form (numeric day/month, long year).
        for vt in [V::Date, V::DateTime] {
            let d = effective_date(vt, true, &stored);
            assert_eq!(
                (d.day, d.month, d.year),
                (
                    DayFormat::NumericDay,
                    MonthFormat::NumericMonth,
                    YearFormat::LongYear
                )
            );
        }
        // Date/DateTime + NOT system default → the field's stored 0x00f2 format, verbatim.
        for vt in [V::Date, V::DateTime] {
            let d = effective_date(vt, false, &stored);
            assert_eq!(
                (d.day, d.month, d.year),
                (stored.day, stored.month, stored.year)
            );
        }
        // Non-date → generic default (leading-zero, short year), ignoring the stored leaf.
        for vt in [V::String, V::Number, V::Time, V::Unknown, V::Boolean] {
            let d = effective_date(vt, false, &stored);
            assert_eq!(
                (d.day, d.month, d.year),
                (
                    DayFormat::LeadingZeroNumericDay,
                    MonthFormat::LeadingZeroNumericMonth,
                    YearFormat::ShortYear
                )
            );
        }
    }

    #[test]
    fn time_hour_leading_zero_and_locale() {
        let us = RenderLocale::default();
        // Time/DateTime drop the hour leading zero.
        assert_eq!(
            effective_time(V::Time, &us).hour,
            "NumericHourNoLeadingZero"
        );
        assert_eq!(effective_time(V::String, &us).hour, "NumericHour");
        // US locale = 12-hour with AM/PM.
        let t = effective_time(V::Time, &us);
        assert_eq!(
            (t.time_base, t.am_string, t.pm_string),
            ("On12Hour", " AM", " PM")
        );
        // A 24-hour locale clears AM/PM and reports On24Hour.
        let eu = RenderLocale {
            twelve_hour: false,
            am_string: "",
            pm_string: "",
            ..RenderLocale::default()
        };
        let t = effective_time(V::Time, &eu);
        assert_eq!(
            (t.time_base, t.am_string, t.pm_string),
            ("On24Hour", "", "")
        );
    }

    #[test]
    fn numeric_defaults_for_non_numeric_types() {
        let stored = NumericFieldFormat {
            decimal_places: 4,
            rounding: RoundingFormat::RoundToUnit,
            ..Default::default()
        };
        // A numeric field keeps its stored decimals/rounding and enables leading zero.
        let n = effective_numeric(V::Currency, &stored);
        assert_eq!(n.decimal_places, 4);
        assert_eq!(n.rounding, RoundingFormat::RoundToUnit);
        assert!(n.leading_zero);
        // A non-numeric field reports the engine defaults (2 decimals, RoundToHundredth, no leading zero).
        let n = effective_numeric(V::String, &stored);
        assert_eq!(n.decimal_places, 2);
        assert_eq!(n.rounding, RoundingFormat::RoundToHundredth);
        assert!(!n.leading_zero);
    }
}
