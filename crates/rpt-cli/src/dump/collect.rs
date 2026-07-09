//! Record-tree walking: select the streams the `--stream` selector picks and collect every record
//! of a given type (with its path and both byte views) into `DumpMatch`.

use rpt::raw::{RecordNode, RecordStream};
use rpt::{Rpt, StreamId};

use super::{DumpMatch, DumpOpts};

/// True if a stream carries the `QESession` (QE) record dialect — the main `QESession` stream and
/// every subreport's `Subdocument N/QESession`, both of which must be parsed via `qe_record_tree`.
fn is_qe_stream(id: &StreamId) -> bool {
    matches!(id, StreamId::QESession) || format!("{id:?}").contains("QESession")
}

/// The streams the `--stream` selector picks, each paired with whether it uses the QE dialect.
/// `contents` (default) is the main `Contents` stream; `qe` is `QESession`; `all` is every stream
/// with a decoded payload; anything else is a case-insensitive substring of the stream id.
pub(super) fn select_streams<'a>(
    rpt: &'a Rpt,
    sel: Option<&str>,
) -> Vec<(String, &'a RecordStream, bool)> {
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
        .map(|(id, s)| (format!("{id:?}"), s, is_qe_stream(id)))
        .collect()
}

/// Collect every record of type `want` (pre-order) under `node`, recording its path and both byte
/// views. `path` is the ancestor-type chain of `node`.
fn collect_matches(
    node: &RecordNode,
    logical: &[u8],
    stream: &str,
    path: &[u16],
    depth: usize,
    want: u16,
    out: &mut Vec<DumpMatch>,
) {
    if node.rtype == want {
        let end = node.content_end.min(logical.len());
        let whole = logical.get(node.offset..end).unwrap_or(&[]).to_vec();
        out.push(DumpMatch {
            stream: stream.to_string(),
            rtype: node.rtype,
            subtype: node.subtype,
            offset: node.offset,
            content_start: node.content_start,
            content_end: node.content_end,
            mask: node.mask,
            depth,
            path: path.to_vec(),
            children: node.children.iter().map(|c| c.rtype).collect(),
            leaf: node.leaf_bytes(logical),
            whole,
        });
    }
    let mut child_path = path.to_vec();
    child_path.push(node.rtype);
    for c in &node.children {
        collect_matches(c, logical, stream, &child_path, depth + 1, want, out);
    }
}

/// Gather the matches for one file, applying the type and `--nth` selectors.
pub(super) fn gather(rpt: &Rpt, opts: &DumpOpts, want: u16) -> Vec<DumpMatch> {
    let mut matches = Vec::new();
    for (name, stream, is_qe) in select_streams(rpt, opts.stream.as_deref()) {
        let logical = stream.logical_bytes();
        let tree = if is_qe {
            stream.qe_record_tree()
        } else {
            stream.record_tree()
        };
        for root in &tree {
            collect_matches(root, logical, &name, &[], 0, want, &mut matches);
        }
    }
    if let Some(n) = opts.nth {
        matches.into_iter().skip(n).take(1).collect()
    } else {
        matches
    }
}
