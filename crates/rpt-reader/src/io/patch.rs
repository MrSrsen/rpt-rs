//! Addressing a record by type and ordinal, and overwriting its bytes.
//!
//! Locating the `nth` record of a type (with or without its ancestor chain) is how every
//! `(tag, nth)`-addressed operation finds its target, read or write. Overwriting a same-size region
//! of a record's demasked field bytes is the primitive behind [`crate::Rpt::patch_record_bytes`]
//! and the same-length edits [`crate::Rpt::anonymize`] makes.

use crate::codec::RecordNode;
use crate::error::Result;
use crate::records::RecordTag;

/// The record a `(tag, nth)` address names is not in the tree — the refusal [`nth_node`] and
/// [`nth_node_path`] returning `None` becomes.
pub(super) fn not_found(tag: RecordTag, nth: usize) -> crate::error::Error {
    crate::error::CodecError::new(format!(
        "record #{nth} of type {tag:?} not found in Contents record tree"
    ))
    .in_stream("Contents")
    .record(tag.0)
    .into()
}

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

/// Overwrite `new_bytes.len()` bytes of `node`'s demasked field bytes into `logical`, starting at
/// `joined_offset` and re-masking with the node's stack mask. Same-size: `logical`'s length is
/// unchanged. Errors if the region overruns the record's field bytes.
///
/// The offset is into the record's runs **joined** — the buffer
/// [`joined_runs`](RecordNode::joined_runs) builds — so on a record with a nested child an offset
/// past the first run addresses bytes that are not adjacent in the file. The write is still correct:
/// it walks the runs and skips the child spans exactly as the join does.
pub(super) fn patch_joined_region(
    node: &RecordNode,
    logical: &mut [u8],
    joined_offset: usize,
    new_bytes: &[u8],
) -> Result<()> {
    let runs: Vec<(usize, usize)> = node.run_spans().collect();
    let field_byte_len: usize = runs.iter().map(|(s, e)| e - s).sum();
    let end = joined_offset.checked_add(new_bytes.len()).ok_or_else(|| {
        crate::error::CodecError::new(format!(
            "patch region offset {joined_offset} + length {} overflows usize",
            new_bytes.len()
        ))
        .record(node.rtype)
    })?;
    if end > field_byte_len {
        return Err(crate::error::CodecError::new(format!(
            "patch region [{joined_offset}, {end}) overruns the record's {field_byte_len} field bytes"
        ))
        .record(node.rtype)
        .into());
    }
    // Walk the record's runs, writing each source byte whose position in their concatenation lands
    // in [joined_offset, end). That concatenation maps to logical piecewise (child spans are skipped),
    // but the stack mask is uniform across the whole record.
    let mut joined_pos = 0usize;
    let mut written = 0usize;
    for (s, e) in runs {
        for slot in &mut logical[s..e] {
            if (joined_offset..end).contains(&joined_pos) {
                *slot = new_bytes[written] ^ node.mask;
                written += 1;
            }
            joined_pos += 1;
        }
    }
    debug_assert_eq!(written, new_bytes.len());
    Ok(())
}
