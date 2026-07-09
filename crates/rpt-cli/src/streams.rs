//! `streams` — raw record-substrate coverage per stream (the decode-coverage meter).

use rpt::Rpt;
use serde::Serialize;

use crate::util::print_json;

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

pub(crate) fn streams(file: &str, json: bool) -> rpt::Result<()> {
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
