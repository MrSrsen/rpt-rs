//! `rpt` — a read-only CLI over the `rpt` library for inspecting `.rpt` files.
//!
//! Four subcommands: `inspect` (report + per-stream summary), `inputs` (the report's external
//! inputs — its parameters — and their types), `streams` (raw record-substrate coverage), and
//! `strings` (readable strings from the decoded Contents tree). Any command accepts `--json` to
//! emit machine-readable output. Run `rpt --help` for the full description.

use std::process::ExitCode;

use rpt::model::{FieldDef, FieldKindData, ParameterField, ParameterValueKind};
use rpt::Rpt;
use serde::Serialize;

const USAGE: &str = "\
rpt — inspect SAP Crystal Reports (.rpt) files

A read-only inspector for the .rpt binary format. It opens the OLE/CFB compound file,
decrypts and decodes its streams (Contents, QESession, PromptManager, …) into the record
substrate, and reports what is inside — no SAP runtime and no database connection needed.

USAGE:
    rpt <COMMAND> <file.rpt> [--json]
    rpt -h | --help

COMMANDS:
    inspect <file>    one-screen summary: report version, summary-info (title / author /
                      subject), record + field counts, the most common record types, and a
                      per-stream listing (size, encryption, IV length, version)
    inputs <file>     the report's external inputs — every parameter the report defines, with
                      its value type (String / Number / Currency / Boolean / Date / Time /
                      DateTime), whether it is optional or multi-valued, and any default values
    streams <file>    raw substrate per stream: record count, how many are still Unknown
                      (undecoded), logical vs on-disk byte sizes, and the top record types
    strings <file>    every readable string (4+ chars) recovered from the decoded Contents
                      record tree — a quick way to eyeball SQL, formulas, and field names

OPTIONS:
    --json            emit the command's output as JSON instead of text
    -h, --help        show this help and exit

NOTES:
    All commands are read-only and work from the decoded records alone. To export the whole
    report as XML, use the companion `rpt-to-xml` binary.
";

fn main() -> ExitCode {
    // Always emit a full backtrace on panic, regardless of RUST_BACKTRACE. The hook also exits
    // quietly on a closed output pipe (`… | head`, or `… | less` then `q`).
    rpt::install_panic_hook();
    let mut json = false;
    let mut help = false;
    let mut pos: Vec<String> = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--json" => json = true,
            "-h" | "--help" => help = true,
            _ => pos.push(arg),
        }
    }
    // Explicit help goes to stdout with a success code; a malformed invocation prints the same
    // text to stderr with a non-zero code (the `_` arm below).
    if help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match pos.as_slice() {
        [cmd, file] if cmd == "inspect" => run(inspect(file, json)),
        [cmd, file] if cmd == "inputs" => run(inputs(file, json)),
        [cmd, file] if cmd == "streams" => run(streams(file, json)),
        [cmd, file] if cmd == "strings" => run(strings(file, json)),
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run(r: rpt::Result<()>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Print a serializable value as a single line of JSON on stdout.
fn print_json<T: Serialize>(value: &T) {
    // These fixed shapes never fail to serialize (no non-string map keys, no custom impls).
    println!(
        "{}",
        serde_json::to_string(value).expect("JSON serialization cannot fail here")
    );
}

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

// --- inputs: the report's external inputs (parameters) ---

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

fn inputs(file: &str, json: bool) -> rpt::Result<()> {
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

// --- inspect: report + per-stream summary ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SummaryJson {
    title: Option<String>,
    author: Option<String>,
    subject: Option<String>,
}

#[derive(Serialize)]
struct TopType {
    #[serde(rename = "type")]
    name: String,
    count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldJson {
    name: String,
    value_type: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamHeadJson {
    id: String,
    bytes: usize,
    encrypted: bool,
    version: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectReport<'a> {
    file: &'a str,
    version: u16,
    record_count: usize,
    distinct_record_types: usize,
    summary_info: SummaryJson,
    top_record_types: Vec<TopType>,
    fields: Vec<FieldJson>,
    streams: Vec<StreamHeadJson>,
}

fn inspect(file: &str, json: bool) -> rpt::Result<()> {
    let rpt = Rpt::open(file)?;
    let si = rpt.summary_info();
    let report = rpt.report();
    let top: Vec<(String, usize)> = report
        .record_inventory
        .iter()
        .take(8)
        .map(|e| match e.name {
            Some(n) => (n.to_string(), e.count),
            None => (format!("{:#06x}", e.tag), e.count),
        })
        .collect();
    let fields = &report.data_definition.field_definitions;

    if json {
        print_json(&InspectReport {
            file,
            version: report.version,
            record_count: report.record_count(),
            distinct_record_types: report.distinct_record_types(),
            summary_info: SummaryJson {
                title: si.and_then(|s| s.title.clone()),
                author: si.and_then(|s| s.author.clone()),
                subject: si.and_then(|s| s.subject.clone()),
            },
            top_record_types: top
                .iter()
                .map(|(n, c)| TopType {
                    name: n.clone(),
                    count: *c,
                })
                .collect(),
            fields: fields
                .iter()
                .map(|f| FieldJson {
                    name: f.name.clone(),
                    value_type: format!("{:?}", f.value_type),
                })
                .collect(),
            streams: rpt
                .streams()
                .map(|(id, s)| StreamHeadJson {
                    id: format!("{id:?}"),
                    bytes: s.raw_bytes().len(),
                    encrypted: s.header().is_some_and(|h| h.is_encrypted),
                    version: s.header().map_or(0, |h| h.version),
                })
                .collect(),
        });
        return Ok(());
    }

    println!("file: {file}");
    if let Some(si) = si {
        if let Some(t) = &si.title {
            println!("title:  {t}");
        }
        if let Some(a) = &si.author {
            println!("author: {a}");
        }
        if let Some(s) = &si.subject {
            println!("subject: {s}");
        }
    }
    println!(
        "report: version={} · {} records · {} distinct types",
        report.version,
        report.record_count(),
        report.distinct_record_types()
    );
    let top_str: Vec<String> = top.iter().map(|(n, c)| format!("{n}×{c}")).collect();
    println!("  top record types: {}", top_str.join("  "));
    if !fields.is_empty() {
        let desc: Vec<String> = fields
            .iter()
            .map(|f| format!("{}:{:?}", f.name, f.value_type))
            .collect();
        println!("  fields ({}): {}", desc.len(), desc.join(", "));
    }
    println!("streams:");
    for (id, stream) in rpt.streams() {
        let hdr = match stream.header() {
            Some(h) => format!(
                " [enc={} iv={}B ver={}]",
                h.is_encrypted,
                h.iv.len(),
                h.version
            ),
            None => String::new(),
        };
        println!(
            "  {:<28} {:>6} bytes{hdr}",
            format!("{id:?}"),
            stream.raw_bytes().len()
        );
    }
    Ok(())
}

// --- strings ---

fn strings(file: &str, json: bool) -> rpt::Result<()> {
    let rpt = Rpt::open(file)?;
    let contents = rpt
        .stream(&rpt::StreamId::Contents)
        .ok_or_else(|| rpt::Error::Container("no Contents stream".into()))?;
    let strings: Vec<String> = contents.strings(4).into_iter().collect();
    if json {
        print_json(&strings);
    } else {
        for s in &strings {
            println!("{s}");
        }
    }
    Ok(())
}

// --- streams: raw record-substrate coverage ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamStat {
    id: String,
    records: usize,
    unknown: usize,
    logical_bytes: usize,
    raw_bytes: usize,
}

#[derive(Serialize)]
struct StreamsReport<'a> {
    file: &'a str,
    streams: Vec<StreamStat>,
}

fn streams(file: &str, json: bool) -> rpt::Result<()> {
    let rpt = Rpt::open(file)?;
    if json {
        let streams = rpt
            .streams()
            .map(|(id, stream)| StreamStat {
                id: format!("{id:?}"),
                records: stream.len(),
                unknown: stream.unknown_count(),
                logical_bytes: stream.logical_bytes().len(),
                raw_bytes: stream.raw_bytes().len(),
            })
            .collect();
        print_json(&StreamsReport { file, streams });
        return Ok(());
    }
    for (id, stream) in rpt.streams() {
        if !stream.records().is_empty() {
            // A fully decoded TSLV stream: header -> decrypt -> inflate -> flat records.
            println!(
                "{id:?}: {} records ({} unknown) from {} logical bytes [{} compressed on disk]",
                stream.len(),
                stream.unknown_count(),
                stream.logical_bytes().len(),
                stream.raw_bytes().len(),
            );
            let mut counts: std::collections::BTreeMap<u16, usize> = Default::default();
            for r in stream.records() {
                *counts.entry(r.tag().value()).or_default() += 1;
            }
            let mut top: Vec<_> = counts.into_iter().collect();
            top.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
            let hist: Vec<String> = top
                .iter()
                .take(8)
                .map(|(t, n)| format!("{t:#06x}×{n}"))
                .collect();
            println!("    top types: {}", hist.join("  "));
        } else if let Some(h) = stream.header() {
            println!(
                "{id:?}: stream-header [enc={} ver={} iv={}B], {} bytes (payload not decoded)",
                h.is_encrypted,
                h.version,
                h.iv.len(),
                stream.raw_bytes().len()
            );
        } else {
            println!("{id:?}: {} bytes (opaque)", stream.raw_bytes().len());
        }
    }
    Ok(())
}
