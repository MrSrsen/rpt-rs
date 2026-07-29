//! `inputs` — the report's external inputs (its parameter fields) and their types.

use rpt_reader::model::{
    FieldDef, FieldKindData, ParameterField, ParameterValue, ParameterValueKind, RangeBoundType,
};
use rpt_reader::Rpt;
use serde::Serialize;

use crate::util::{print_json, CliError};

pub(crate) const HELP: &str = "\
rpt inputs — the report's external inputs (parameters)

Every parameter the report defines, with its value type (String / Number / Currency / Boolean /
Date / Time / DateTime), whether it is optional or multi-valued, its default values, and the
last-used value saved with the report. A range value is written [start..end], with a round bracket
for an excluded or open end.

USAGE:
    rpt inputs <file.rpt> [--json]

OPTIONS:
    --json    emit the parameter list as JSON
";

/// The friendly value-type name of a parameter input (the data type the caller must supply).
fn input_type(kind: ParameterValueKind) -> &'static str {
    use ParameterValueKind as Vk;
    match kind {
        Vk::NumberParameter => "Number",
        Vk::CurrencyParameter => "Currency",
        Vk::BooleanParameter => "Boolean",
        Vk::DateParameter => "Date",
        Vk::TimeParameter => "Time",
        Vk::DateTimeParameter => "DateTime",
        _ => "String",
    }
}

/// One stored parameter value as text: a discrete value verbatim, a range in interval notation
/// (`[start..end]`, a square bracket for an included bound and a round one for an excluded or open
/// end, an open end written as nothing).
///
/// A range is written out because half of it is the only part that says the parameter is a range at
/// all: printing the discrete field alone renders `1..100` as a bare `1`.
fn value_text(v: &ParameterValue) -> String {
    let Some(range) = &v.range else {
        return v.value.clone();
    };
    let open = |b: RangeBoundType, closed: char, other: char| match b {
        RangeBoundType::BoundInclusive => closed,
        _ => other,
    };
    format!(
        "{}{}..{}{}",
        open(range.lower_bound, '[', '('),
        v.value,
        range.end_value,
        open(range.upper_bound, ']', ')'),
    )
}

/// Every stored value of one list, as text.
fn value_texts(values: &[ParameterValue]) -> Vec<String> {
    values.iter().map(value_text).collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InputItem {
    name: String,
    #[serde(rename = "ref")]
    reference: String,
    #[serde(rename = "type")]
    value_type: &'static str,
    value_kind: String,
    parameter_type: String,
    optional: bool,
    multi_valued: bool,
    allow_custom_values: bool,
    has_current_value: bool,
    prompt_text: Option<String>,
    default_values: Vec<String>,
    /// The last-used value(s) saved with the report, decoded from its parameter-values stream.
    /// `has_current_value` says one was recorded; this is what it is.
    current_values: Vec<String>,
}

#[derive(Serialize)]
struct InputsReport<'a> {
    file: &'a str,
    inputs: Vec<InputItem>,
}

/// The report's external inputs: its parameter field definitions, in declaration order.
fn report_inputs(report: &rpt_reader::model::Report) -> Vec<(&FieldDef, &ParameterField)> {
    report
        .data_definition
        .field_definitions
        .iter()
        .filter_map(|f| match &f.kind {
            FieldKindData::Parameter(p) => Some((f, p.as_ref())),
            _ => None,
        })
        .collect()
}

pub(crate) fn inputs(file: &str, json: bool) -> Result<(), CliError> {
    let rpt = Rpt::open(file)?;
    let items = report_inputs(rpt.report());

    if json {
        let inputs = items
            .iter()
            .map(|(f, p)| InputItem {
                name: f.name.clone(),
                reference: format!("{{?{}}}", f.name),
                value_type: input_type(p.value_kind),
                value_kind: format!("{:?}", p.value_kind),
                parameter_type: format!("{:?}", p.parameter_type),
                optional: p.optional_prompt,
                multi_valued: p.allow_multiple_values,
                allow_custom_values: p.allow_custom_values,
                has_current_value: p.has_current_value,
                prompt_text: p.prompt_text.clone(),
                default_values: value_texts(&p.default_values),
                current_values: value_texts(&p.current_values),
            })
            .collect();
        print_json(&InputsReport { file, inputs });
        return Ok(());
    }

    println!("inputs ({}):", items.len());
    for (f, p) in &items {
        let mut flags = Vec::new();
        if p.optional_prompt {
            flags.push("optional");
        }
        if p.allow_multiple_values {
            flags.push("multi-valued");
        }
        if p.allow_custom_values {
            flags.push("custom-allowed");
        }
        let flag_str = if flags.is_empty() {
            String::new()
        } else {
            format!("  [{}]", flags.join(", "))
        };
        let prompt = p
            .prompt_text
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| format!("  — {s}"))
            .unwrap_or_default();
        println!(
            "  {:<28} {:<9}{flag_str}{prompt}",
            format!("{{?{}}}", f.name),
            input_type(p.value_kind),
        );
        if !p.default_values.is_empty() {
            println!(
                "      default: {}",
                value_texts(&p.default_values).join(", ")
            );
        }
        if !p.current_values.is_empty() {
            println!(
                "      current: {}",
                value_texts(&p.current_values).join(", ")
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_reader::model::ParameterRange;

    fn discrete(value: &str) -> ParameterValue {
        ParameterValue {
            value: value.to_string(),
            ..ParameterValue::default()
        }
    }

    fn range(
        start: &str,
        end: &str,
        lower: RangeBoundType,
        upper: RangeBoundType,
    ) -> ParameterValue {
        ParameterValue {
            value: start.to_string(),
            range: Some(ParameterRange {
                end_value: end.to_string(),
                lower_bound: lower,
                upper_bound: upper,
            }),
            ..ParameterValue::default()
        }
    }

    #[test]
    fn a_discrete_value_is_written_verbatim() {
        assert_eq!(
            value_text(&discrete("Date(2001,04,24)")),
            "Date(2001,04,24)"
        );
    }

    /// A range must not be rendered as its lower bound alone: that is the whole reason it is
    /// written out, and the discrete field of a range value holds only that bound.
    #[test]
    fn a_range_is_written_as_an_interval_with_its_bound_kinds() {
        use RangeBoundType::{BoundExclusive, BoundInclusive, NoBound};
        assert_eq!(
            value_text(&range("1", "100", BoundInclusive, BoundInclusive)),
            "[1..100]"
        );
        assert_eq!(
            value_text(&range("1", "100", BoundExclusive, BoundExclusive)),
            "(1..100)"
        );
        // An open end has no bound value, so only the bracket says the end is there at all.
        assert_eq!(
            value_text(&range("1", "", BoundInclusive, NoBound)),
            "[1..)"
        );
    }
}
