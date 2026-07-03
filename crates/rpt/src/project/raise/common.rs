//! Shared raise-layer helpers over the record tree: string scans across a node's leaves, tree
//! queries, and coordinate/colour decoding.
//!
//! The pure byte/scan vocabulary ([`lp_scan`], [`Cursor`], the checked scalar reads, [`read_lp_string`])
//! lives in [`crate::bytes`] and is re-exported here so every raise decoder has it in scope via
//! `use super::*`. This module adds the [`RecordNode`]-aware helpers on top.

use super::*;

pub(super) use crate::bytes::{
    first_lp, i32_be, longest_lp, lp_scan, lp_string_at, read_lp_string, u16_be, u16_le, u32_be,
    Cursor, Scan,
};

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

/// The leaf bytes of every node of type `rtype`, anywhere in the tree, in pre-order.
pub(super) fn leaves_of(tree: &[RecordNode], logical: &[u8], rtype: u16) -> Vec<Vec<u8>> {
    nodes_where(tree, |n| n.rtype == rtype)
        .into_iter()
        .map(|n| n.leaf_bytes(logical))
        .collect()
}

pub(super) fn own_lp_strings(node: &RecordNode, logical: &[u8]) -> Vec<String> {
    lp_scan(&node.leaf_bytes(logical), Scan::Consume)
        .map(|(_, s, _)| s)
        .collect()
}

/// Find the first length-prefixed string in a record's content: each node's **own** leaf bytes
/// (a container record like a field object holds its data-source string in its own bytes,
/// alongside child records), scanning every offset, in pre-order.
pub(super) fn first_string(node: &RecordNode, logical: &[u8]) -> Option<String> {
    let mut found = None;
    node.walk(&mut |n| {
        if found.is_none() {
            found = first_lp(&n.leaf_bytes(logical));
        }
    });
    found
}

/// Expand an abbreviated summary-operation token to the engine's full operator name as it appears
/// in a rendered summary expression (`Max` → `Maximum`, `Min` → `Minimum`); any other token is
/// returned unchanged. The stored/display form abbreviates `Maximum`/`Minimum` (see
/// `summary_op_token`), but the rendered `Op (…)` expression spells them out.
pub(super) fn summary_op_full(token: &str) -> &str {
    match token {
        "Max" => "Maximum",
        "Min" => "Minimum",
        other => other,
    }
}

/// The trailing run of ASCII decimal digits of `s` (`"GroupHeaderArea4"` → `"4"`,
/// `"PageHeader"` → `""`) — used to link a group header to its matching footer.
pub(super) fn trailing_digits(s: &str) -> String {
    let digits: String = s.chars().rev().take_while(char::is_ascii_digit).collect();
    digits.chars().rev().collect()
}

/// Decode a `COLORREF` (`0x00BBGGRR`, big-endian) into a [`Color`]; `0xffffffff` is the
/// "default / no colour" sentinel, treated as White.
pub(super) fn raise_colorref(b: &[u8]) -> Color {
    let v = u32_be(b, 0).unwrap_or(0);
    if v == 0xffff_ffff {
        return Color::WHITE;
    }
    Color {
        a: 255,
        r: (v & 0xff) as u8,
        g: ((v >> 8) & 0xff) as u8,
        b: ((v >> 16) & 0xff) as u8,
    }
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

/// Read a variable-length object coordinate (big-endian twips) at `off`, returning its value and
/// the offset past it. A coordinate below 32768 is a plain `u16`; at or above 32768 the high bit of
/// the first word is set as an escape and the value is `(word & 0x7fff) << 16 | next u16` — wide
/// export reports place objects past 32768 (and past 65536) twips.
pub(super) fn read_coord(b: &[u8], off: usize) -> Option<(i32, usize)> {
    let w = i32::from(u16_be(b, off)?);
    if w & 0x8000 != 0 {
        let low = i32::from(u16_be(b, off + 2)?);
        Some((((w & 0x7fff) << 16) | low, off + 4))
    } else {
        Some((w, off + 2))
    }
}

/// All length-prefixed strings in a record's content (every node's own leaf bytes), scanning
/// every offset, in pre-order.
pub(super) fn all_strings(node: &RecordNode, logical: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    node.walk(&mut |n| {
        out.extend(lp_scan(&n.leaf_bytes(logical), Scan::Consume).map(|(_, s, _)| s));
    });
    out
}
