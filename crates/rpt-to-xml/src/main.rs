//! `rpt-to-xml` — walks the report's semantic DOM ([`rpt::model::Report`]) and serializes it to XML.
//!
//! Two modes: **default** (the modelled report DOM) and **`--full`** (the modelled DOM plus the
//! complete raw record tree).

mod colors;
mod database;
mod objects;
mod selection;
mod util;
mod xml;

use std::process::ExitCode;

use rpt::Rpt;

use crate::xml::to_xml;

const HELP: &str = "\
rpt-to-xml — export a SAP Crystal Reports (.rpt) file to XML

Decodes the .rpt binary directly — no SAP runtime and no database connection needed —
and serializes it to a structured XML document, for inspection or for diffing report
definitions in version control.

USAGE:
    rpt-to-xml [OPTIONS] <file.rpt> [out.xml]

ARGS:
    <file.rpt>    the report to read
    [out.xml]     output path; if omitted, XML is written to stdout

OPTIONS:
    --full        export ALL decoded data: the modelled DOM plus the complete raw
                  record tree (every record, with its decoded leaf values)
    -h, --help    show this help

MODES:
    default       a structured document (Summaryinfo, ReportOptions, PrintOptions,
                  Database, DataDefinition, ReportDefinition). Only the modelled
                  subset of the report is emitted.
    --full        the above plus a <Records> tree of every decoded record.

NOTES:
    The record stream is the source of truth; the default output covers the modelled
    subset of the report definition, not every stored byte.
";

fn main() -> ExitCode {
    // Always emit a full backtrace on panic, regardless of RUST_BACKTRACE. The hook also exits
    // quietly on a closed output pipe (`… | head`, or `… | less` then `q`).
    rpt::install_panic_hook();
    let mut full = false;
    let mut paths: Vec<String> = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return ExitCode::SUCCESS;
            }
            "--full" => full = true,
            _ => paths.push(arg),
        }
    }

    let (input, output) = match paths.as_slice() {
        [input] => (input.clone(), None),
        [input, output] => (input.clone(), Some(output.clone())),
        _ => {
            eprint!("{HELP}");
            return ExitCode::from(2);
        }
    };

    let rpt = match Rpt::open(&input) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let xml = to_xml(&input, rpt.report(), full);
    match output {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, xml) {
                eprintln!("error writing {path}: {e}");
                return ExitCode::FAILURE;
            }
        }
        None => print!("{xml}"),
    }
    ExitCode::SUCCESS
}
