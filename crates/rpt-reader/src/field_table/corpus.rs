//! The corpus walk every field-table harness runs.
//!
//! Opening each report, classifying its streams by dialect and decoding the ones that are not
//! decoded yet is one procedure, and every harness that sweeps the corpus needs all of it. Written
//! out per test it drifts: a sweep that reads a subreport's stream without decoding it under the
//! identity it is written in walks an empty tree and still reports a corpus-wide floor.

use std::path::Path;

use crate::codec::{Dialect, RecordNode};
use crate::records::RecordStream;
use crate::StreamId;

/// The vocabulary a stream's records are written in, and the identity it must be decoded under, or
/// `None` for a stream no field table describes.
///
/// A subreport's own stream is not a top-level stream — the container keeps only its raw bytes —
/// so it is named by its full path and re-decoded here. A sweep that skips that step walks an
/// empty tree and measures the main report alone.
fn classify(id: &StreamId) -> Option<(Dialect, Option<StreamId>)> {
    match id {
        StreamId::Contents => Some((Dialect::Contents, None)),
        StreamId::QESession => Some((Dialect::QeSession, None)),
        StreamId::DataSourceManager(_) => Some((Dialect::Catalog, None)),
        StreamId::ReportParametersStream(_) => Some((Dialect::ReportParameters, None)),
        StreamId::Other(path) => match path.rsplit_once('/')?.1 {
            "Contents" => Some((Dialect::Contents, Some(StreamId::Contents))),
            "QESession" => Some((Dialect::QeSession, Some(StreamId::QESession))),
            _ => None,
        },
        _ => None,
    }
}

/// Every stream of every corpus report that a field table describes, decoded and classified.
///
/// The report is passed alongside so a harness can adjudicate a table's reading against the
/// semantic model built from the same bytes.
pub(crate) fn for_each_stream(mut f: impl FnMut(&crate::Rpt, Dialect, &RecordStream, &Path)) {
    for path in &rpt_test_support::corpus_reports() {
        let Ok(rpt) = crate::Rpt::open(path) else {
            continue;
        };
        for (id, stream) in rpt.streams() {
            let Some((dialect, inner)) = classify(id) else {
                continue;
            };
            let nested;
            let stream = match inner {
                Some(i) => {
                    nested = RecordStream::decode(i, stream.raw_bytes());
                    &nested
                }
                None => stream,
            };
            f(&rpt, dialect, stream, path);
        }
    }
}

/// Every record of every stream a field table describes, in every corpus report — with the logical
/// bytes its spans index into, and the report it came from.
pub(crate) fn for_each_record(mut f: impl FnMut(Dialect, &RecordNode, &[u8], &Path)) {
    for_each_stream(|_, dialect, stream, path| {
        let logical = stream.logical_bytes();
        for root in stream.record_tree() {
            root.walk(&mut |node| f(dialect, node, logical, path));
        }
    });
}

/// Every record of one type, in one dialect. A type number is per dialect, so both are the key.
pub(crate) fn for_each_record_of(
    dialect: Dialect,
    rtype: u16,
    mut f: impl FnMut(&RecordNode, &[u8], &Path),
) {
    for_each_record(|d, node, logical, path| {
        if d == dialect && node.rtype == rtype {
            f(node, logical, path);
        }
    });
}
