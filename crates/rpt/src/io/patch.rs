//! Record-tree navigation and leaf patching for the write path.
//!
//! Locating the `nth` record of a type (with or without its ancestor chain) and overwriting a
//! same-size region of its demasked leaf — the primitives behind [`crate::Rpt::patch_record_leaf`]
//! and [`crate::Rpt::patch_record_leaf_resize`].

use crate::codec::RecordNode;
use crate::error::Result;

/// The `nth` (0-based, pre-order) node in `tree` whose record type is `rtype`, or `None`.
pub(super) fn nth_node(tree: &[RecordNode], rtype: u16, nth: usize) -> Option<&RecordNode> {
    fn visit<'a>(n: &'a RecordNode, rtype: u16, remaining: &mut usize) -> Option<&'a RecordNode> {
        if n.rtype == rtype {
            if *remaining == 0 {
                return Some(n);
            }
            *remaining -= 1;
        }
        n.children.iter().find_map(|c| visit(c, rtype, remaining))
    }
    let mut remaining = nth;
    tree.iter()
        .find_map(|root| visit(root, rtype, &mut remaining))
}

/// Like [`nth_node`], but also returns the target's ancestor chain (root-first, excluding the
/// target itself) — the records whose length prefixes must be recomputed on a length-changing edit.
pub(super) fn nth_node_path(
    tree: &[RecordNode],
    rtype: u16,
    nth: usize,
) -> Option<(&RecordNode, Vec<&RecordNode>)> {
    fn visit<'a>(
        n: &'a RecordNode,
        rtype: u16,
        remaining: &mut usize,
        path: &mut Vec<&'a RecordNode>,
    ) -> Option<(&'a RecordNode, Vec<&'a RecordNode>)> {
        if n.rtype == rtype {
            if *remaining == 0 {
                return Some((n, path.clone()));
            }
            *remaining -= 1;
        }
        path.push(n);
        let found = n
            .children
            .iter()
            .find_map(|c| visit(c, rtype, remaining, path));
        path.pop();
        found
    }
    let mut remaining = nth;
    let mut path = Vec::new();
    tree.iter()
        .find_map(|root| visit(root, rtype, &mut remaining, &mut path))
}

/// Overwrite `new_bytes.len()` bytes of `node`'s demasked leaf into `logical`, starting at
/// `leaf_offset` and re-masking with the node's stack mask. Same-size: `logical`'s length is
/// unchanged. Errors if the region overruns the record's leaf.
pub(super) fn patch_leaf_region(
    node: &RecordNode,
    logical: &mut [u8],
    leaf_offset: usize,
    new_bytes: &[u8],
) -> Result<()> {
    let segments = node.leaf_segments();
    let leaf_len: usize = segments.iter().map(|(s, e)| e - s).sum();
    let end = leaf_offset.checked_add(new_bytes.len()).ok_or_else(|| {
        crate::error::CodecError::new(format!(
            "patch region offset {leaf_offset} + length {} overflows usize",
            new_bytes.len()
        ))
        .record(node.rtype)
    })?;
    if end > leaf_len {
        return Err(crate::error::CodecError::new(format!(
            "patch region [{leaf_offset}, {end}) overruns record leaf of {leaf_len} bytes"
        ))
        .record(node.rtype)
        .into());
    }
    // Walk the leaf's logical segments, writing each source byte whose leaf position lands in
    // [leaf_offset, end). The leaf maps to logical piecewise (child spans are skipped), but the
    // stack mask is uniform across the whole record.
    let mut leaf_pos = 0usize;
    let mut written = 0usize;
    for (s, e) in segments {
        for slot in &mut logical[s..e] {
            if (leaf_offset..end).contains(&leaf_pos) {
                *slot = new_bytes[written] ^ node.mask;
                written += 1;
            }
            leaf_pos += 1;
        }
    }
    debug_assert_eq!(written, new_bytes.len());
    Ok(())
}
