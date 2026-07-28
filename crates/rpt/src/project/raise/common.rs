//! Shared raise-layer helpers over the record tree: string scans across a node's leaves, tree
//! queries, and coordinate/colour decoding.
//!
//! The pure byte/scan vocabulary ([`lp_scan`], [`Cursor`], the checked scalar reads, [`read_lp_string`])
//! lives in [`crate::bytes`] and is re-exported here so every raise decoder has it in scope via
//! `use super::*`. This module adds the [`RecordNode`]-aware helpers on top.

use super::*;

pub(super) use crate::bytes::{
    first_lp, i32_be, longest_lp, lp_scan, lp_string_at, read_be_lp_string_lossy,
    read_be_lp_string_lossy_at, read_lp_string, u16_be, u16_le, u32_be, Cursor, Scan,
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
        "Avg" => "Average",
        other => other,
    }
}

/// Whether a string is an engine field reference: a database field (`Table.field`) or a formula
/// (`@name`). Excludes literals like `Others` and localized order/name marker strings.
pub(super) fn is_field_ref(s: &str) -> bool {
    s.starts_with('@') || s.contains('.')
}

/// A three-byte `BGR` colour triple at `off` — the on-disk order of an object border/background
/// colour (byte `off` = Blue, `off+1` = Green, `off+2` = Red). `None` if the triple runs past the end.
pub(super) fn bgr(b: &[u8], off: usize) -> Option<Color> {
    Some(Color {
        a: 255,
        r: *b.get(off + 2)?,
        g: *b.get(off + 1)?,
        b: *b.get(off)?,
    })
}

/// Decode a `COLORREF` value (`0x00BBGGRR`) into a [`Color`]: red in the low byte, then green, then
/// blue. The caller reads the `u32` in the record's own endianness and applies its own sentinel.
pub(super) fn colorref(v: u32) -> Color {
    Color {
        a: 255,
        r: (v & 0xff) as u8,
        g: ((v >> 8) & 0xff) as u8,
        b: ((v >> 16) & 0xff) as u8,
    }
}

/// Decode a big-endian `COLORREF` (`0x00BBGGRR`) at a leaf's start into a [`Color`]; `0xffffffff` is
/// the "default / no colour" sentinel, treated as White.
pub(super) fn raise_colorref(b: &[u8]) -> Color {
    let v = u32_be(b, 0).unwrap_or(0);
    if v == 0xffff_ffff {
        Color::WHITE
    } else {
        colorref(v)
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

/// The summary-definition records (`0x7e` **not** immediately preceded by its `0x80` running-total
/// reset), in document order. When `until_area`, the scan stops at the first area marker (summary
/// definitions all precede the layout region); otherwise the whole tree is scanned (a superset that
/// also picks up definitions inside the layout).
pub(super) fn summary_def_nodes(tree: &[RecordNode], until_area: bool) -> Vec<&RecordNode> {
    let mut out = Vec::new();
    let mut prev = 0u16;
    for n in flatten(tree) {
        if until_area && n.rtype == AREA_MARKER {
            break;
        }
        if n.rtype == SUMMARY_DEF && prev != RT_RESET {
            out.push(n);
        }
        prev = n.rtype;
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

#[cfg(test)]
mod tests {
    use super::{read_coord, summary_op_full};

    #[test]
    fn read_coord_plain_and_escaped() {
        // Below 32768: a plain big-endian u16, consuming 2 bytes.
        assert_eq!(read_coord(&[0x01, 0x00], 0), Some((256, 2)));
        assert_eq!(read_coord(&[0x7f, 0xff], 0), Some((0x7fff, 2)));
        // High bit set: escape — value is (word & 0x7fff) << 16 | next u16, consuming 4 bytes.
        // 0x8001 << 16 masked = 0x0001_0000, plus 0x0002 low word = 0x0001_0002.
        assert_eq!(
            read_coord(&[0x80, 0x01, 0x00, 0x02], 0),
            Some((0x0001_0002, 4))
        );
        // Respects the offset and reports the post-coordinate cursor.
        assert_eq!(read_coord(&[0xaa, 0x01, 0x00], 1), Some((256, 3)));
        // Truncated inputs yield None rather than panicking.
        assert_eq!(read_coord(&[0x00], 0), None);
        assert_eq!(read_coord(&[0x80, 0x01], 0), None); // escape needs a second word
    }

    #[test]
    fn summary_op_full_expands_only_min_max() {
        assert_eq!(summary_op_full("Max"), "Maximum");
        assert_eq!(summary_op_full("Min"), "Minimum");
        assert_eq!(summary_op_full("Avg"), "Average");
        // Any other token passes through unchanged.
        assert_eq!(summary_op_full("Sum"), "Sum");
        assert_eq!(summary_op_full("DistinctCount"), "DistinctCount");
    }
}
