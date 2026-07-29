//! Finding records — and the strings inside them — in a stream's record tree.
//!
//! Every decoder in this layer starts by locating the records it cares about. These helpers do the
//! walking (pre-order, the order the engine wrote the records) and hand back nodes, field bytes, or
//! the length-prefixed strings a node carries. Reading values *out of* a run is
//! [`super::record_values`].
//!
//! # Choosing a traversal
//!
//! Several traversals coexist in this layer, one per *relationship* a decoder can need between the
//! record it wants and the records around it. Choose by the relationship, and say which one it is:
//!
//! - **Descendant-anywhere** — position carries nothing, the record is wanted wherever it sits:
//!   [`nodes_where`].
//! - **Containment** — the record's meaning depends on the record it is nested in, as a `0x7e`
//!   summary definition does: a walk that carries the parent's rtype, like [`summary_def_nodes`].
//! - **Document order** — the record's meaning depends on what precedes it, because the record it
//!   belongs to opened a run rather than nesting it: [`flatten`], or `binding_scopes` where that run
//!   is object-scoped (a chart or cross-tab opener, until the next layout marker).
//! - **A single-pass build** — [`RecordNode::walk`] directly, with a closure over the state being
//!   built, where the collector is one function's private business and no other decoder shares it.
//!
//! Containment and adjacency are the pair to be careful with, because a passing test does not tell
//! them apart: in pre-order a parent immediately precedes its first child, so indexing the previous
//! node reads the parent for a first child and the previous sibling for every other one. Whichever
//! is meant must be written down, or the next reader inherits a relationship the code never stated.
//!
//! **This is our method, not the format's.** The format asks for a record by type, bounded by the
//! marker that closes the container being read ([`crate::raw::RecordSearch`]); a whole-tree walk
//! has no container to be bounded by, so where a type occurs inside more than one container it
//! finds records the format's own reader would not. Moving a decoder from here to a bounded search
//! is therefore a decode change, per record type, with a baseline diff to read.

use crate::codec::RecordNode;
use crate::records::rtype::*;

/// All nodes of the tree (pre-order) satisfying `pred`.
pub(super) fn nodes_where(
    tree: &[RecordNode],
    pred: impl Fn(&RecordNode) -> bool,
) -> Vec<&RecordNode> {
    let mut out = Vec::new();
    for root in tree {
        root.walk(&mut |n| {
            if pred(n) {
                out.push(n);
            }
        });
    }
    out
}

/// Flatten the record tree into document (pre-order) order — the order the engine wrote the
/// records, in which an object and its name/position records are adjacent.
pub(super) fn flatten(tree: &[RecordNode]) -> Vec<&RecordNode> {
    let mut out = Vec::new();
    for root in tree {
        root.walk(&mut |n| out.push(n));
    }
    out
}

/// The summary-definition records (`0x7e`) that define a **summary**, in document order.
///
/// A running total stores its own operation as a `0x7e` inside its `0x80` record, so the two are
/// told apart by containment: a `0x7e` a running total holds is that running total's operation, and
/// every other one is a summary in its own right.
///
/// When `until_area`, the scan stops at the first area marker (summary definitions all precede the
/// layout region); otherwise the whole tree is scanned (a superset that also picks up definitions
/// inside the layout).
pub(super) fn summary_def_nodes(tree: &[RecordNode], until_area: bool) -> Vec<&RecordNode> {
    /// Walks `nodes` in document order, returning `false` once the area marker has stopped it.
    fn walk<'a>(
        nodes: &'a [RecordNode],
        parent: u16,
        until_area: bool,
        out: &mut Vec<&'a RecordNode>,
    ) -> bool {
        for n in nodes {
            if until_area && n.rtype == AREA {
                return false;
            }
            if n.rtype == SUMMARY_FIELD_DEFINITION && parent != RUNNING_TOTAL_FIELD {
                out.push(n);
            }
            if !walk(&n.children, n.rtype, until_area, out) {
                return false;
            }
        }
        true
    }

    let mut out = Vec::new();
    walk(tree, 0, until_area, &mut out);
    out
}
