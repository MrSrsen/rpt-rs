//! Record-tree walking: select the streams the `--stream` selector picks and collect every record
//! of a given type (with its path and both byte views) into `DumpMatch`.

use rpt_reader::raw::{Dialect, RecordNode, RecordStream};
use rpt_reader::{Rpt, StreamId};

use super::{DumpMatch, DumpOpts};

/// The streams the `--stream` selector picks, each paired with the record vocabulary it is written
/// in. `contents` (default) is the main `Contents` stream; `qe` is `QESession`; `all` is every
/// stream with a decoded payload; anything else is a case-insensitive substring of the stream id.
pub(super) fn select_streams<'a>(
    rpt: &'a Rpt,
    sel: Option<&str>,
) -> Vec<(String, &'a RecordStream, Dialect)> {
    let sel = sel.unwrap_or("contents");
    rpt.streams()
        .filter(|(id, s)| {
            let name = format!("{id:?}");
            match sel {
                "contents" => matches!(id, StreamId::Contents),
                "qe" => matches!(id, StreamId::QESession),
                "all" => !s.logical_bytes().is_empty(),
                other => name.to_lowercase().contains(&other.to_lowercase()),
            }
        })
        .map(|(id, s)| (format!("{id:?}"), s, s.dialect()))
        .collect()
}

/// The vocabulary the `--stream` selector reads records in, for the labels printed before any
/// record has been selected. `all` and a substring selector can pick streams of more than one, so
/// this answers for the selector; every record carries the dialect of the stream it came from.
pub(super) fn selector_dialect(sel: Option<&str>) -> Dialect {
    let sel = sel.unwrap_or("contents").to_lowercase();
    if sel == "qe" || sel.contains("qesession") {
        Dialect::QeSession
    } else if sel.contains("datasourcemanager") {
        Dialect::Catalog
    } else if sel.contains("reportparameters") {
        Dialect::ReportParameters
    } else {
        Dialect::Contents
    }
}

/// Collect every record of type `want` (pre-order) under `node`, recording its path and both byte
/// views. `path` is the ancestor-type chain of `node`.
fn collect_matches(
    node: &RecordNode,
    source: &RecordStream,
    stream: &str,
    path: &[u16],
    depth: usize,
    want: u16,
    out: &mut Vec<DumpMatch>,
) {
    let logical = source.logical_bytes();
    let dialect = source.dialect();
    if node.rtype == want {
        let end = node.content_end.min(logical.len());
        let whole = logical.get(node.offset..end).unwrap_or(&[]).to_vec();
        out.push(DumpMatch {
            stream: stream.to_string(),
            dialect,
            rtype: node.rtype,
            schema: node.schema,
            offset: node.offset,
            content_start: node.content_start,
            content_end: node.content_end,
            mask: node.mask,
            depth,
            path: path.to_vec(),
            children: node.children.iter().map(|c| c.rtype).collect(),
            joined_runs: node.joined_runs(logical),
            run_lengths: node.runs(logical).map(|r| r.len()).collect(),
            whole,
            table: source.fields(node),
        });
    }
    let mut child_path = path.to_vec();
    child_path.push(node.rtype);
    for c in &node.children {
        collect_matches(c, source, stream, &child_path, depth + 1, want, out);
    }
}

/// Gather the matches for one file, applying the type and `--nth` selectors.
pub(super) fn gather(rpt: &Rpt, opts: &DumpOpts, want: u16) -> Vec<DumpMatch> {
    let mut matches = Vec::new();
    for (name, stream, _) in select_streams(rpt, opts.stream.as_deref()) {
        for root in &stream.record_tree() {
            collect_matches(root, stream, &name, &[], 0, want, &mut matches);
        }
    }
    if let Some(n) = opts.nth {
        matches.into_iter().skip(n).take(1).collect()
    } else {
        matches
    }
}
