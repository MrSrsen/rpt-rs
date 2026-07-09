//! Turn `--param Name=Value` CLI pairs into resolved [`Parameters`], coercing each value to the
//! parameter's declared type and collapsing repeats of one name into a multi-value array.
//! Extracted from `main.rs` so the coercion has its own unit tests.

use crystal_formula::eval::{Date, Time, Value};
use rpt::model::{ParameterValueKind as Vk, Report};
use rpt_data::{normalize_param_name, Parameters};
use rpt_render::RenderError;

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
                // Fidelity warning: a param the report doesn't declare almost always means a typo.
                log.warn(
                    Comp::Entry,
                    format!("parameter {key:?} is not declared by the report (coercing as String)"),
                );
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
    Ok((params, resolved))
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
