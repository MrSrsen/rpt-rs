//! `json-dump` — export the decoded report as an exhaustive, deterministic JSON document.
//!
//! The dump is the **full** serde serialization of [`rpt_reader::model::Report`] — every field, including
//! defaults, and the whole subreport tree via [`rpt_reader::model::Report::subreports`]. It is a mirror of
//! the decode and nothing else: only **stored facts**, values actually read out of the `.rpt` bytes.
//! Nothing here is inferred, recomputed, or reconstructed.
//!
//! That restriction is what makes the output usable as a regression baseline. A dump that also
//! carried derived analytics would move whenever a *derivation* changed, with the decode untouched —
//! so a baseline diff would no longer mean "the decoder's reading of this file changed", which is
//! the only thing it is there to tell you.
//!
//! The report sits under a single `model` key rather than at the top level, leaving room for
//! container-level stored facts (stream inventory, header flags) to join it as siblings later.
//!
//! Embedded binary payloads are carried whole but compactly: a picture's bytes serialize as a
//! lowercase hex string (two characters per byte) rather than serde's default array of integers,
//! which a pretty-printer spends a line each on.
//!
//! Output is deterministic — byte-identical across runs for the same input. The dump is materialized
//! through [`serde_json::Value`], whose objects are sorted-key maps, so every key is emitted in a
//! stable order; it is pretty-printed with a two-space indent and ends with a trailing newline.

use rpt_reader::model::Report;
use rpt_reader::Rpt;
use serde::Serialize;

use crate::Error;

/// The scoped `--help` text for `rpt json-dump`.
pub const HELP: &str = "\
rpt json-dump — export a Crystal Reports (.rpt) file to JSON

Decodes the .rpt binary directly — the file alone, no database connection made —
and serializes the full decoded semantic model to a single deterministic JSON document. Every model
field is emitted, including defaults, unlike the sparse KDL authoring projection.

The document holds only STORED facts — values read out of the file's bytes. Values the Crystal
engine computes rather than stores (a field's use count, a formula's runtime result length, the
locale-resolved display format) are deliberately absent: they are not properties of the file.

    model      the decoded report model (recursively includes the subreport tree)

An embedded binary payload — a picture's bytes — is emitted as a lowercase hex string, two
characters per byte, rather than an array of integers. Every byte is present either way.

Output is byte-identical across runs for the same input: stable field order, maps emitted in
sorted-key order, two-space indent, trailing newline.

USAGE:
    rpt json-dump <file.rpt> [out.json] [--strict]

ARGS:
    <file.rpt>    the report to read
    [out.json]    output path; if omitted, JSON is written to stdout

OPTIONS:
    --strict      fail instead of warning when the report did not decode completely
    -h, --help    show this help

ABOUT:
    Part of the rpt-rs project — a pure-Rust reader for the Crystal Reports (.rpt) format.
    Homepage:     https://github.com/MrSrsen/rpt-rs
    Report bugs:  https://github.com/MrSrsen/rpt-rs/issues
";

/// Export `input` to JSON, written to `output` (or stdout when `None`).
///
/// Returns how completely `input` decoded. Because projection is infallible by design, an export that
/// is missing content looks exactly like a faithful one — so the caller is handed the coverage rather
/// than this library deciding whether to warn, and should surface
/// [`DecodeCoverage::warning`](rpt_reader::DecodeCoverage::warning) when it is `Some`.
///
/// # Errors
///
/// - [`Error::Report`] — `input` could not be opened or decoded.
/// - [`Error::Serialize`] — the decoded model could not be serialized.
/// - [`Error::Write`] — `output` could not be written.
pub fn run(input: &str, output: Option<&str>) -> Result<rpt_reader::DecodeCoverage, Error> {
    let rpt = Rpt::open(input)?;
    let dump = JsonDump {
        model: rpt.report(),
    };
    // Round-trip through `Value` (sorted-key maps) so key order is stable regardless of struct
    // field order or map iteration order.
    let serialize_err = |source| Error::Serialize {
        input: input.to_string(),
        source,
    };
    let doc = serde_json::to_value(&dump).map_err(serialize_err)?;
    let mut json = serde_json::to_string_pretty(&doc).map_err(serialize_err)?;
    json.push('\n');
    match output {
        None => print!("{json}"),
        Some(path) => {
            std::fs::write(path, &json).map_err(|source| Error::Write {
                path: path.to_string(),
                source,
            })?;
            eprintln!("json-dump: {input} -> {path} ({} bytes)", json.len());
        }
    }
    Ok(rpt.decode_coverage())
}

/// The exhaustive dump: the decoded model, and nothing derived from it.
#[derive(Serialize)]
struct JsonDump<'a> {
    /// The decoded semantic model (recursively includes subreports).
    model: &'a Report,
}
