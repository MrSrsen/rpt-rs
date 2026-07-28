//! Turn `--param Name=Value` CLI pairs into resolved [`Parameters`], coercing each value to the
//! parameter's declared type and collapsing repeats of one name into a multi-value array.
//! Extracted from `main.rs` so the coercion has its own unit tests.

use crate::error::RenderError;
use crystal_formula::eval::{Date, Time, Value};
use rpt::model::{
    ParameterField, ParameterValue, ParameterValueKind as Vk, RangeBoundType, Report,
};
use rpt_data::{normalize_param_name, Parameters};

use crate::applog::{Comp, Log};

/// A coerced parameter and the declared/coerced type name, for logging what the render actually saw.
pub struct ResolvedParam {
    pub name: String,
    pub type_name: &'static str,
    pub display: String,
}

/// A parameter the report declares — used to list what's expected when none were supplied.
pub struct DeclaredParam {
    pub name: String,
    pub type_name: &'static str,
    pub optional: bool,
    pub multi: bool,
}

/// The multi-line "no parameters supplied" warning: the report declares parameters but none were
/// passed, so the render uses each parameter's default. Lists every declared parameter with its type
/// and any `optional`/`multi-valued` flags. `declared` must be non-empty (the caller checks first).
/// The logger indents the continuation lines under the message column.
pub fn missing_values_warning(declared: &[DeclaredParam]) -> String {
    let mut msg = format!(
        "no parameters supplied; the report declares {} — rendering with defaults \
         (set with -p Name=Value):",
        declared.len()
    );
    for d in declared {
        let mut flags = Vec::new();
        if d.optional {
            flags.push("optional");
        }
        if d.multi {
            flags.push("multi-valued");
        }
        let suffix = if flags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", flags.join(", "))
        };
        msg.push_str(&format!("\n  - {} : {}{suffix}", d.name, d.type_name));
    }
    msg
}

/// The parameters the report declares, in declaration order (for the "expected inputs" listing).
pub fn declared(report: &Report) -> Vec<DeclaredParam> {
    report
        .data_definition
        .parameter_fields()
        .map(|(fd, pf)| DeclaredParam {
            name: fd.name.clone(),
            type_name: kind_name(pf.value_kind),
            optional: pf.optional_prompt,
            multi: pf.allow_multiple_values,
        })
        .collect()
}

/// Build [`Parameters`] from `Name=Value` pairs, logging (at NORMAL) the effective value of each and
/// warning when a supplied name isn't declared by the report (usually a typo). Returns the params
/// plus the resolved list for the caller's summary.
pub fn build(
    report: &Report,
    raw: &[(String, String)],
    log: &Log,
) -> Result<(Parameters, Vec<ResolvedParam>), RenderError> {
    // Group values by normalized name, preserving first-seen order.
    let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
    for (name, value) in raw {
        let key = normalize_param_name(name);
        match grouped.iter_mut().find(|(k, _)| *k == key) {
            Some((_, values)) => values.push(value.clone()),
            None => grouped.push((key, vec![value.clone()])),
        }
    }

    let mut params = Parameters::new();
    let mut resolved = Vec::new();
    for (key, values) in grouped {
        let def = report
            .data_definition
            .parameter_fields()
            .find(|(fd, _)| normalize_param_name(&fd.name) == key);
        let (kind, allow_multiple) = match &def {
            Some((_, pf)) => (pf.value_kind, pf.allow_multiple_values),
            None => {
                // A parameter the report does not declare is almost always a typo — and unlike the
                // pipeline's fail-open cases there is no correct behaviour to fall back to: the value
                // cannot reach the render, so the output is for different criteria than the user
                // asked for. Reported at ERROR severity so `-q` (errors only) cannot hide it from a
                // scripted run; the render still proceeds, since the report's own defaults remain a
                // defined result.
                let declared: Vec<&str> = report
                    .data_definition
                    .parameter_fields()
                    .map(|(fd, _)| fd.name.as_str())
                    .collect();
                let mut msg = format!(
                    "parameter {key:?} is not declared by the report, so its value is ignored"
                );
                match nearest(&key, &declared) {
                    Some(near) => msg.push_str(&format!(" — did you mean {near:?}?")),
                    None => msg.push('.'),
                }
                msg.push_str(&match declared.len() {
                    0 => " The report declares no parameters.".to_string(),
                    _ => format!(" The report declares: {}.", declared.join(", ")),
                });
                log.error_at(Comp::Entry, msg);
                (Vk::StringParameter, false)
            }
        };
        let coerced = values
            .iter()
            .map(|v| coerce(kind, v))
            .collect::<Result<Vec<Value>, String>>()
            .map_err(|e| RenderError::Params(format!("parameter {key:?}: {e}")))?;
        // A multi-value parameter is always an array (even with one value); a single-value parameter
        // errors if given more than one.
        let value = if allow_multiple {
            Value::Array(coerced)
        } else if coerced.len() == 1 {
            coerced.into_iter().next().unwrap()
        } else {
            return Err(RenderError::Params(format!(
                "parameter {key:?} is single-value but {} values were given",
                coerced.len()
            )));
        };
        resolved.push(ResolvedParam {
            name: key.clone(),
            type_name: kind_name(kind),
            display: format!("{value:?}"),
        });
        params.insert(key, value);
    }

    // A declared parameter the caller did not supply binds to its stored value — the engine's
    // current value if it has one, else its default value(s) — exactly as the engine does when a
    // report is run accepting defaults. Selection/grouping formulas then filter on the defaults
    // (and `HasValue({?Param})` reports true) rather than over-rendering every row. A parameter with
    // no stored value binds to `Null` so the reference still resolves (rather than erroring as an
    // unknown name) and `HasValue` reports false.
    for (fd, pf) in report.data_definition.parameter_fields() {
        let key = normalize_param_name(&fd.name);
        if params.contains_key(&key) {
            continue;
        }
        let value = stored_default(pf).unwrap_or(Value::Null);
        params.insert(key, value);
    }
    Ok((params, resolved))
}

/// The value the engine binds for a declared parameter the caller left unset: its stored **current
/// value** if it has one, else its stored **default value(s)**. Returns `None` when the parameter
/// carries neither, so the reference stays `Null` (`HasValue` false). A multi-value parameter always
/// yields a [`Value::Array`]; a range default yields a [`Value::Range`]. Values that fail to coerce
/// to the declared kind are skipped (an all-unusable set yields `None`).
fn stored_default(pf: &ParameterField) -> Option<Value> {
    let kind = pf.value_kind;
    // The current value (last-used) wins over the default pick list, matching the SDK's effective
    // -value resolution; fall back to the declared defaults.
    let source: &[ParameterValue] = if pf.has_current_value && !pf.current_values.is_empty() {
        &pf.current_values
    } else {
        &pf.default_values
    };
    if source.is_empty() {
        return None;
    }
    if pf.allow_multiple_values {
        let vals: Vec<Value> = source
            .iter()
            .filter_map(|pv| default_one(kind, pv))
            .collect();
        (!vals.is_empty()).then_some(Value::Array(vals))
    } else {
        source.first().and_then(|pv| default_one(kind, pv))
    }
}

/// Coerce one stored [`ParameterValue`] to a [`Value`] of the declared kind: a discrete value maps
/// through [`coerce`]; a range maps to a [`Value::Range`] with each end's inclusivity from its bound
/// type. A range end that fails to coerce (e.g. an open, empty bound) makes the whole range unusable.
fn default_one(kind: Vk, pv: &ParameterValue) -> Option<Value> {
    match &pv.range {
        None => coerce(kind, &pv.value).ok(),
        Some(r) => Some(Value::Range {
            lo: Box::new(coerce(kind, &pv.value).ok()?),
            hi: Box::new(coerce(kind, &r.end_value).ok()?),
            lo_incl: matches!(r.lower_bound, RangeBoundType::BoundInclusive),
            hi_incl: matches!(r.upper_bound, RangeBoundType::BoundInclusive),
        }),
    }
}

/// The declared kind's short name, for the parameter log line.
fn kind_name(kind: Vk) -> &'static str {
    match kind {
        Vk::NumberParameter => "Number",
        Vk::CurrencyParameter => "Currency",
        Vk::BooleanParameter => "Boolean",
        Vk::DateParameter => "Date",
        Vk::TimeParameter => "Time",
        Vk::DateTimeParameter => "DateTime",
        Vk::StringParameter => "String",
        _ => "String",
    }
}

/// Coerce one string value to a [`Value`] of the parameter's declared kind.
fn coerce(kind: Vk, s: &str) -> Result<Value, String> {
    let parse_num = |s: &str| {
        s.trim()
            .parse::<f64>()
            .map_err(|_| format!("{s:?} is not a number"))
    };
    match kind {
        Vk::NumberParameter => Ok(Value::Number(parse_num(s)?)),
        Vk::CurrencyParameter => Ok(Value::Currency(parse_num(s)?)),
        Vk::BooleanParameter => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "y" => Ok(Value::Bool(true)),
            "false" | "0" | "no" | "n" => Ok(Value::Bool(false)),
            _ => Err(format!("{s:?} is not a boolean (true/false/1/0/yes/no)")),
        },
        Vk::DateParameter => parse_date(s).map(Value::Date),
        Vk::TimeParameter => parse_time(s).map(Value::Time),
        Vk::DateTimeParameter => {
            let (d, t) = s
                .split_once(['T', ' '])
                .ok_or_else(|| format!("{s:?} is not a datetime (YYYY-MM-DDTHH:MM:SS)"))?;
            Ok(Value::DateTime(parse_date(d)?, parse_time(t)?))
        }
        // StringParameter and any Other kind: pass through verbatim.
        _ => Ok(Value::Str(s.to_string())),
    }
}

/// Parse an ISO `YYYY-MM-DD` date.
fn parse_date(s: &str) -> Result<Date, String> {
    let p: Vec<&str> = s.trim().split('-').collect();
    match p.as_slice() {
        [y, m, d] => Ok(Date::new(
            y.parse().map_err(|_| bad_date(s))?,
            m.parse().map_err(|_| bad_date(s))?,
            d.parse().map_err(|_| bad_date(s))?,
        )),
        _ => Err(bad_date(s)),
    }
}

/// Parse an ISO `HH:MM[:SS]` time.
fn parse_time(s: &str) -> Result<Time, String> {
    let p: Vec<&str> = s.trim().split(':').collect();
    let get = |i: usize| p.get(i).unwrap_or(&"0").parse().map_err(|_| bad_time(s));
    match p.len() {
        2 | 3 => Ok(Time::new(get(0)?, get(1)?, get(2)?)),
        _ => Err(bad_time(s)),
    }
}

fn bad_date(s: &str) -> String {
    format!("{s:?} is not a date (expected YYYY-MM-DD)")
}

fn bad_time(s: &str) -> String {
    format!("{s:?} is not a time (expected HH:MM:SS)")
}

/// The declared name closest to `given`, when one is close enough to be a plausible typo.
///
/// Compared case-insensitively (the CLI already normalizes parameter names that way) and gated on a
/// distance proportional to the name's length, so a genuinely different name is not "corrected" into
/// an unrelated one — a wrong suggestion sends the user down the wrong path, which is worse than no
/// suggestion.
fn nearest<'a>(given: &str, declared: &[&'a str]) -> Option<&'a str> {
    let given = given.to_lowercase();
    let budget = (given.chars().count() / 3).max(1);
    declared
        .iter()
        .map(|name| (edit_distance(&given, &name.to_lowercase()), *name))
        .filter(|(d, _)| *d <= budget)
        .min_by_key(|(d, _)| *d)
        .map(|(_, name)| name)
}

/// Levenshtein distance, single-row DP.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod suggestion_tests {
    use super::{edit_distance, nearest};

    #[test]
    fn distance_counts_single_character_edits() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("abc", "abc"), 0);
        assert_eq!(edit_distance("abc", "abd"), 1);
        assert_eq!(edit_distance("abc", "ab"), 1);
        assert_eq!(edit_distance("abc", ""), 3);
    }

    #[test]
    fn a_typo_is_matched_to_the_declared_name() {
        let declared = ["Order_Amt_Range", "Region"];
        assert_eq!(
            nearest("order_amt_rang", &declared),
            Some("Order_Amt_Range")
        );
        assert_eq!(nearest("regio", &declared), Some("Region"));
    }

    #[test]
    fn an_unrelated_name_gets_no_suggestion() {
        // A wrong suggestion is worse than none: it sends the user after the wrong parameter.
        let declared = ["Order_Amt_Range", "Region"];
        assert_eq!(nearest("customer_id", &declared), None);
        assert_eq!(nearest("x", &declared), None);
    }

    #[test]
    fn no_declared_parameters_yields_no_suggestion() {
        assert_eq!(nearest("anything", &[]), None);
    }
}
