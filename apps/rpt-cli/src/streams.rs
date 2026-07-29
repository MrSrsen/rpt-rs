//! `streams` — record coverage per stream (the decode-coverage meter).

use rpt_reader::raw::RecordTag;
use rpt_reader::Rpt;
use serde::Serialize;

use crate::util::{print_json, CliError};

pub(crate) const HELP: &str = "\
rpt streams — record coverage per stream

For each stream: how many records it holds, how many are still Unknown (undecoded), logical vs
on-disk byte sizes, and the top record types — the meter for record-type decode coverage.

A stream holds records two ways, and the figures name which they are for. Its OUTERMOST records are
the linear walk, where a record's content spans the records nested inside it; that walk is what the
uncovered-byte account is read off, since only its spans lie side by side. The TREE is every record
at every depth, and it is what the unrecognized-type census is read off — a type that only ever
occurs nested never reaches the linear walk. `rpt tree` and `rpt dump` (with no --type) count the
tree too.

USAGE:
    rpt streams <file.rpt> [--json]

OPTIONS:
    --json    emit the per-stream coverage as JSON
";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamStat<'a> {
    id: &'a str,
    /// Outermost records: the stream's linear record walk, in which a record's content spans the
    /// records nested inside it. Fewer than the record tree holds.
    outermost_records: usize,
    /// Records in the stream's record tree: every record at every depth. The denominator for
    /// `unknownRecords`.
    tree_records: usize,
    /// Records anywhere in the tree whose type the registry does not recognize.
    unknown_records: usize,
    /// The distinct unrecognized record types — what a machine consumer needs in order to act.
    unknown_types: &'a [u16],
    /// Logical bytes belonging to no record of the linear walk.
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
    /// Whether every stream decoded with nothing unrecognized and nothing left over, and the
    /// saved-data path read every row the file claims.
    complete: bool,
    streams: Vec<StreamStat<'a>>,
    /// What the saved-data path made of the report's stored rows. It spans three streams, so it has
    /// no per-stream row of its own.
    saved_data: rpt_reader::SavedDataStatus,
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
                outermost_records: c.outermost_records,
                tree_records: c.tree_records,
                unknown_records: c.unknown_records,
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
            saved_data: coverage.saved_data,
        });
        return Ok(());
    }
    // The coverage rows are the streams, in the same order, so each stream's line is the meter's own
    // figures rather than a second reading of the same stream.
    for ((id, stream), c) in rpt.streams().zip(&coverage.streams) {
        if !stream.records().is_empty() {
            // A fully decoded TSLV stream: header -> decrypt -> inflate -> flat records. Both
            // populations are named, because they answer different questions: the linear walk is
            // what the byte account is read off, the tree what the unrecognized-type census is.
            println!(
                "{id:?}: {} outermost records, {} in the tree ({} unrecognized) from {} logical bytes [{} compressed on disk]",
                c.outermost_records,
                c.tree_records,
                c.unknown_records,
                stream.logical_bytes().len(),
                stream.raw_bytes().len(),
            );
            // Counted over the tree, the same population the census above is read off — a histogram
            // of the outermost records alone would omit whole record families that only nest.
            let mut counts: std::collections::BTreeMap<u16, usize> = Default::default();
            for root in stream.record_tree() {
                root.walk(&mut |node| *counts.entry(node.rtype).or_default() += 1);
            }
            let mut top: Vec<_> = counts.into_iter().collect();
            top.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
            // Named in the vocabulary this stream is written in: a type number alone answers for no
            // stream, and the same number is an unrelated record in each.
            let hist: Vec<String> = top
                .iter()
                .take(8)
                .map(|(t, n)| format!("{}×{n}", RecordTag(*t).label(stream.dialect())))
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
            // The vocabulary is the stream's own, so it is read off the stream rather than named
            // here — a stream read in one is not a stream read in another's.
            println!(
                "{id:?}: {} logical bytes [{} compressed on disk] (not tiled; `rpt dump --stream` reads its record tree in the {:?} vocabulary)",
                stream.logical_bytes().len(),
                stream.raw_bytes().len(),
                stream.dialect(),
            );
        } else {
            println!("{id:?}: {} bytes (opaque)", stream.raw_bytes().len());
        }
    }
    // The saved-data path spans three streams, so it gets a line of its own rather than a row above.
    println!("saved data: {}", coverage.saved_data);
    // The bottom line: is this decode complete? Otherwise the per-stream numbers above are easy to
    // read past.
    match coverage.warning() {
        Some(w) => println!("\nINCOMPLETE: {w}"),
        None => println!("\ncomplete: every stream decoded with nothing unrecognized"),
    }
    Ok(())
}
