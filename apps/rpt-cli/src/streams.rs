//! `streams` — raw record-substrate coverage per stream (the decode-coverage meter).

use rpt::Rpt;
use serde::Serialize;

use crate::util::{print_json, CliError};

pub(crate) const HELP: &str = "\
rpt streams — raw record-substrate coverage per stream

For each stream: record count, how many are still Unknown (undecoded), logical vs on-disk byte
sizes, and the top record types — the meter for record-type decode coverage.

USAGE:
    rpt streams <file.rpt> [--json]

OPTIONS:
    --json    emit the per-stream coverage as JSON
";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamStat<'a> {
    id: &'a str,
    records: usize,
    unknown: usize,
    /// The distinct unrecognized record types — what a machine consumer needs in order to act.
    unknown_types: &'a [u16],
    /// Logical bytes belonging to no decoded record.
    uncovered_bytes: usize,
    /// Why the stream would not decode at all, when it would not.
    decode_error: Option<&'a str>,
    logical_bytes: usize,
    raw_bytes: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamsReport<'a> {
    file: &'a str,
    /// Whether every stream decoded with nothing unrecognized and nothing left over.
    complete: bool,
    streams: Vec<StreamStat<'a>>,
}

pub(crate) fn streams(file: &str, json: bool) -> Result<(), CliError> {
    let rpt = Rpt::open(file)?;
    // The same figures the export and render paths warn from, so the meter and the warning can never
    // disagree.
    let coverage = rpt.decode_coverage();
    if json {
        let raw: Vec<usize> = rpt.streams().map(|(_, s)| s.raw_bytes().len()).collect();
        let streams = coverage
            .streams
            .iter()
            .zip(raw)
            .map(|(c, raw_bytes)| StreamStat {
                id: &c.stream,
                records: c.records,
                unknown: c.unknown_records,
                unknown_types: &c.unknown_types,
                uncovered_bytes: c.uncovered_bytes,
                decode_error: c.decode_error.as_deref(),
                logical_bytes: c.logical_bytes,
                raw_bytes,
            })
            .collect();
        print_json(&StreamsReport {
            file,
            complete: coverage.is_complete(),
            streams,
        });
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
        } else if !stream.logical_bytes().is_empty() {
            // A stream whose payload is decoded to logical bytes but not tiled into flat records
            // (the `DataSourceManager` saved-data catalog); its record tree is available via `dump`.
            println!(
                "{id:?}: {} logical bytes [{} compressed on disk] (not tiled; dump with the QE dialect)",
                stream.logical_bytes().len(),
                stream.raw_bytes().len(),
            );
        } else {
            println!("{id:?}: {} bytes (opaque)", stream.raw_bytes().len());
        }
    }
    // The bottom line: is this decode complete? Otherwise the per-stream numbers above are easy to
    // read past.
    match coverage.warning() {
        Some(w) => println!("\nINCOMPLETE: {w}"),
        None => println!("\ncomplete: every stream decoded with nothing unrecognized"),
    }
    Ok(())
}
