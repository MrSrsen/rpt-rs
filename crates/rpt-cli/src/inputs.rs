//! `inputs` — the report's external inputs (its parameter fields) and their types.

use rpt::model::{FieldDef, FieldKindData, ParameterField, ParameterValueKind};
use rpt::Rpt;
use serde::Serialize;

use crate::util::print_json;

pub(crate) const HELP: &str = "\
rpt inputs — the report's external inputs (parameters)

Every parameter the report defines, with its value type (String / Number / Currency / Boolean /
Date / Time / DateTime), whether it is optional or multi-valued, and any default values.

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
}

#[derive(Serialize)]
struct InputsReport<'a> {
    file: &'a str,
    inputs: Vec<InputItem>,
}

/// The report's external inputs: its parameter field definitions, in declaration order.
fn report_inputs(report: &rpt::model::Report) -> Vec<(&FieldDef, &ParameterField)> {
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

pub(crate) fn inputs(file: &str, json: bool) -> rpt::Result<()> {
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
                default_values: p.default_values.iter().map(|v| v.value.clone()).collect(),
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
            let d: Vec<&str> = p.default_values.iter().map(|v| v.value.as_str()).collect();
            println!("      default: {}", d.join(", "));
        }
    }
    Ok(())
}
