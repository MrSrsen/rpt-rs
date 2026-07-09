//! XML export — walks the report's semantic DOM ([`rpt::model::Report`]) and serializes it to XML.
//!
//! Two modes: **default** (the modelled report DOM) and **`--full`** (the modelled DOM plus the
//! complete raw record tree). This is the RptToXml-compatible export surface validated against the
//! engine oracle; the derived-analytics [`analysis`] module lives here rather than on the stored
//! `rpt` model, preserving the stored-vs-derived boundary.

mod analysis;
mod colors;
mod database;
mod objects;
mod selection;
mod util;
mod xml;

use rpt::Rpt;

/// The scoped `--help` text for `rpt xml-dump`.
pub(crate) const HELP: &str = "\
rpt xml-dump — export a Crystal Reports (.rpt) file to XML

Decodes the .rpt binary directly — no Crystal Reports runtime and no database connection needed —
and serializes it to a structured XML document, for inspection or for diffing report
definitions in version control.

USAGE:
    rpt xml-dump <file.rpt> [out.xml] [--full]

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

ABOUT:
    Part of the rpt-rs project — a pure-Rust reader for the Crystal Reports (.rpt) format.
    Homepage:     https://github.com/MrSrsen/rpt-rs
    Report bugs:  https://github.com/MrSrsen/rpt-rs/issues
";

/// Export `input` to XML, written to `output` (or stdout when `None`). `full` also appends the raw
/// record tree.
pub(crate) fn run(input: &str, output: Option<&str>, full: bool) -> rpt::Result<()> {
    let rpt = Rpt::open(input)?;
    let xml = xml::to_xml(input, rpt.report(), full);
    match output {
        Some(path) => std::fs::write(path, xml)?,
        None => print!("{xml}"),
    }
    Ok(())
}
