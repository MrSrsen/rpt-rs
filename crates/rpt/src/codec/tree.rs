//! L1 — the recursive record-tree reader.
//!
//! A report record's content is itself a sequence of **nested** TSLV records (or leaf field
//! data). Content is read under a **stack-XOR mask**: the per-byte XOR mask is the XOR of the
//! record types currently on the parse stack (it un-XORs on pop).
//!
//! Header detection uses the bit-packed TSLV header (the same [`super::tslv::Flags`] decoding
//! [`super::tile`] uses), flag byte `0xf8`/`0xf9`, demasked with the current stack mask. The
//! [`Dialect`] selects the validation: `Contents` records always carry subtype low byte `0x07`
//! (suppressing false-positive headers in leaf data), while `QESession` records use varied
//! subtypes and relax it. Decoding never panics and is bounded by each record's declared
//! length.

/// A record in the nested tree: its type, the stack-XOR mask its content is read under, the
/// content span within the logical stream, and any nested child records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordNode {
    /// The record's type tag (the TSLV rtype word).
    pub rtype: u16,
    /// The record's subtype word (`0x07` low byte for `Contents`; varied for `QESession`).
    pub subtype: u16,
    /// Offset of the record header within the logical stream.
    pub offset: usize,
    /// The content byte span `[content_start, content_end)` within the logical stream.
    pub content_start: usize,
    /// End (exclusive) of the content byte span within the logical stream.
    pub content_end: usize,
    /// The XOR mask the content (and this record's own leaf bytes) are read under.
    pub mask: u8,
    /// Nested child records (empty for a leaf).
    pub children: Vec<RecordNode>,
}

impl RecordNode {
    /// The record's type tag.
    pub fn tag(&self) -> crate::records::RecordTag {
        crate::records::RecordTag(self.rtype)
    }

    /// True if this record has no nested records (its content is leaf field data).
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }

    /// Visit this node and all descendants in pre-order.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a RecordNode)) {
        f(self);
        for child in &self.children {
            child.walk(f);
        }
    }

    /// This record's own leaf bytes (the content spans not covered by any child), demasked
    /// with its stack mask. For a leaf record this is the whole content.
    pub fn leaf_bytes(&self, logical: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cursor = self.content_start;
        for child in &self.children {
            push_demasked(&mut out, logical, cursor, child.offset, self.mask);
            cursor = child.content_end;
        }
        push_demasked(&mut out, logical, cursor, self.content_end, self.mask);
        out
    }

    /// The logical byte spans that make up this record's own leaf — the content gaps not covered by
    /// any child, in order. The inverse of the concatenation [`RecordNode::leaf_bytes`] performs:
    /// concatenating these spans (each demasked) yields `leaf_bytes`.
    pub(crate) fn leaf_segments(&self) -> Vec<(usize, usize)> {
        let mut segs = Vec::new();
        let mut cursor = self.content_start;
        for child in &self.children {
            if child.offset > cursor {
                segs.push((cursor, child.offset));
            }
            cursor = child.content_end;
        }
        if self.content_end > cursor {
            segs.push((cursor, self.content_end));
        }
        segs
    }
}

fn push_demasked(out: &mut Vec<u8>, data: &[u8], from: usize, to: usize, mask: u8) {
    if let Some(slice) = data.get(from..to) {
        out.extend(slice.iter().map(|b| b ^ mask));
    }
}

/// Which record dialect a stream uses — selects how record headers are validated. `Contents`
/// records all carry subtype low byte `0x07` (the constraint suppresses false-positive headers
/// in leaf field data); `QESession` records use varied subtypes, so its parse relaxes that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Contents,
    Qe,
}

/// Try to read a record header at `pos` under stack mask `m`, bounded by `limit`.
/// Returns `(rtype, subtype, content_len, header_len)` if the bytes match the bit-packed TSLV
/// header shape (the same `Flags` decoding [`super::tile`] uses, generalised over the subtype
/// word so it serves both `Contents` and `QESession` records).
fn read_header(
    d: &[u8],
    pos: usize,
    m: u8,
    limit: usize,
    dialect: Dialect,
) -> Option<(u16, u16, usize, usize)> {
    use super::tslv::{self, Flags};
    // Demask a byte of the header (headers are read under the current stack mask).
    let at = |i: usize| -> Option<u8> { d.get(pos + i).map(|b| b ^ m) };

    let mut fw = [at(0)?, at(1)?];
    // flag: bits 3..7 set (0xf8/0xf9) — extended type + 4-byte length, the report-record shape.
    if fw[0] & 0xf8 != 0xf8 {
        return None;
    }
    if dialect == Dialect::Contents && at(2)? != 0x07 {
        return None;
    }
    let flags = Flags::decode(&fw);
    let mut q = 2usize;

    let rtype = if flags.extended_value {
        let v = u16::from_le_bytes([at(q)?, at(q + 1)?]);
        q += 2;
        v
    } else {
        tslv::clear_flag_bits(&mut fw);
        (u16::from(fw[0]) << 8) | u16::from(fw[1])
    };

    let subtype = if flags.extended_type {
        let st = u16::from_le_bytes([at(q)?, at(q + 1)?]);
        q += 2;
        st
    } else {
        0
    };

    let len = if flags.len_kind != 0 {
        let n = flags.len_kind as usize;
        let mut bytes = [0u8; 4];
        for (k, slot) in bytes[..n].iter_mut().enumerate() {
            *slot = at(q + k)?;
        }
        q += n;
        tslv::be_scalar(&bytes[..n]) as usize
    } else {
        0
    };

    let header_len = q;
    if pos + header_len + len > limit {
        return None;
    }
    Some((rtype, subtype, len, header_len))
}

/// Parse `logical[start..end)` as a sequence of records under stack mask `m`.
fn parse_seq(
    d: &[u8],
    start: usize,
    end: usize,
    m: u8,
    depth: usize,
    dialect: Dialect,
) -> Vec<RecordNode> {
    let mut out = Vec::new();
    let mut p = start;
    while p < end {
        let Some((rtype, subtype, len, header_len)) = read_header(d, p, m, end, dialect) else {
            // Leaf byte: not a record header here. Advance; the surrounding record's declared
            // length keeps us anchored.
            p += 1;
            continue;
        };
        let content_start = p + header_len;
        let content_end = content_start + len;
        let child_mask = m ^ (rtype as u8);
        let children = if depth < MAX_DEPTH {
            parse_seq(
                d,
                content_start,
                content_end,
                child_mask,
                depth + 1,
                dialect,
            )
        } else {
            Vec::new()
        };
        out.push(RecordNode {
            rtype,
            subtype,
            offset: p,
            content_start,
            content_end,
            mask: child_mask,
            children,
        });
        p = content_end;
    }
    out
}

const MAX_DEPTH: usize = 32;

/// Re-serialize a record tree back into the logical byte stream it was parsed from.
///
/// The tree is a structural view over the retained `logical` bytes: each node's header
/// (`offset..content_start`), leaf gaps, and children partition the stream contiguously. Walking
/// that structure and copying each span reconstructs `logical` byte-for-byte — the re-serializable
/// substrate the writer (`encode_contents`) rests on. A structurally inconsistent tree (overlapping
/// or out-of-parent spans) produces bytes that differ from `logical`, so the round-trip doubles as a
/// tree-integrity check.
pub(crate) fn serialize_tree(nodes: &[RecordNode], logical: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(logical.len());
    serialize_seq(nodes, logical, 0, logical.len(), &mut out);
    out
}

/// Emit `logical[start..end)` as delimited by `nodes`: bytes before/between/after nodes (leaf data
/// at this level) verbatim, each node's header verbatim, and its content recursively.
fn serialize_seq(
    nodes: &[RecordNode],
    logical: &[u8],
    start: usize,
    end: usize,
    out: &mut Vec<u8>,
) {
    let mut cursor = start;
    for node in nodes {
        if let Some(gap) = logical.get(cursor..node.offset) {
            out.extend_from_slice(gap);
        }
        if let Some(head) = logical.get(node.offset..node.content_start) {
            out.extend_from_slice(head);
        }
        serialize_seq(
            &node.children,
            logical,
            node.content_start,
            node.content_end,
            out,
        );
        cursor = node.content_end;
    }
    if let Some(tail) = logical.get(cursor..end) {
        out.extend_from_slice(tail);
    }
}

/// Replace a leaf region of `target` with `new_bytes` of a **possibly different length**, returning
/// the rewritten logical stream — the length-changing writer primitive (phase-2).
///
/// TSLV length prefixes are relative and nested: a record's stored length covers its whole content
/// span, *including* every nested child. So growing/shrinking a leaf by `Δ` grows/shrinks the
/// length prefix of the edited record **and every ancestor up the chain** by exactly `Δ` — nothing
/// else. There are no absolute byte offsets in the `Contents` record tree, so no other stored value
/// needs fixing.
///
/// `leaf_region` is in `target`'s demasked-leaf coordinate space and must lie within a single
/// contiguous leaf segment (it may not straddle a child record — that would splice bytes into a
/// child's framing). `ancestors` is `target`'s ancestor chain (any order; each is a record whose
/// content span encloses `target`). `new_bytes` are demasked; they are re-masked with `target`'s
/// stack mask before insertion. Every rewritten length prefix must still fit its on-disk field
/// width, or the edit is rejected (an `Err`, never a corrupt stream).
pub(crate) fn resize_leaf_region(
    logical: &[u8],
    target: &RecordNode,
    ancestors: &[&RecordNode],
    leaf_region: std::ops::Range<usize>,
    new_bytes: &[u8],
) -> crate::error::Result<Vec<u8>> {
    use crate::error::CodecError;

    if leaf_region.start > leaf_region.end {
        return Err(CodecError::new(format!(
            "resize leaf region start {} > end {}",
            leaf_region.start, leaf_region.end
        ))
        .record(target.rtype)
        .into());
    }
    // Map the demasked-leaf region to a contiguous logical byte range within one leaf segment.
    let (log_start, log_end) = map_leaf_region(target, &leaf_region).ok_or_else(|| {
        CodecError::new(format!(
            "resize region [{}, {}) is out of the leaf or straddles a child record",
            leaf_region.start, leaf_region.end
        ))
        .record(target.rtype)
    })?;

    let old_len = log_end - log_start;
    let delta = new_bytes.len() as i64 - old_len as i64;

    // Recompute the length prefix of the target and each ancestor. All their headers precede
    // `log_start` (a record's header precedes its content, and every ancestor encloses the target),
    // so patching them leaves the splice point untouched.
    let mut out = logical.to_vec();
    for node in std::iter::once(target).chain(ancestors.iter().copied()) {
        let (pos, width, header_mask) = length_field(logical, node).ok_or_else(|| {
            CodecError::new("record has no recomputable length field").record(node.rtype)
        })?;
        debug_assert!(
            pos + width <= log_start,
            "length field must precede the splice"
        );
        let new_content = node.content_end as i64 - node.content_start as i64 + delta;
        if new_content < 0 {
            return Err(CodecError::new(format!(
                "resize shrinks record content to {new_content} bytes (below zero)"
            ))
            .record(node.rtype)
            .into());
        }
        let max = if width >= 8 {
            u64::MAX
        } else {
            (1u64 << (8 * width)) - 1
        };
        if new_content as u64 > max {
            return Err(CodecError::new(format!(
                "resized length {new_content} overflows the {width}-byte length field (max {max})"
            ))
            .record(node.rtype)
            .into());
        }
        let be = (new_content as u64).to_be_bytes();
        for (k, slot) in out[pos..pos + width].iter_mut().enumerate() {
            *slot = be[8 - width + k] ^ header_mask;
        }
    }

    // Splice the re-masked replacement bytes over the (old) region. Everything from `log_start`
    // onward shifts by `delta`; the already-patched length prefixes all sit before `log_start`.
    let masked: Vec<u8> = new_bytes.iter().map(|b| b ^ target.mask).collect();
    out.splice(log_start..log_end, masked);
    Ok(out)
}

/// Map a demasked-leaf region `[start, end)` of `node` to a contiguous logical byte range, or
/// `None` if it is out of the leaf or straddles a child record (crosses a leaf-segment boundary).
fn map_leaf_region(node: &RecordNode, region: &std::ops::Range<usize>) -> Option<(usize, usize)> {
    let mut base = 0usize; // leaf position at the start of the current segment
    for (s, e) in node.leaf_segments() {
        let seg_len = e - s;
        // Both endpoints must land within this one segment (end may equal the segment end).
        if region.start >= base && region.end <= base + seg_len {
            return Some((s + (region.start - base), s + (region.end - base)));
        }
        base += seg_len;
    }
    // Allow an empty region at the very end of an empty leaf (no segments): only [0,0).
    (node.leaf_segments().is_empty() && region.start == 0 && region.end == 0)
        .then_some((node.content_end, node.content_end))
}

/// The length prefix of `node` within `logical`: `(byte position, field width, header mask)`.
/// The length field is the last `len_kind` bytes of the record header (`[content_start - w,
/// content_start)`), big-endian, masked with the header's stack mask (`node.mask ^ node.rtype`,
/// the mask in effect when the header itself was read). `None` if the header has no length field.
fn length_field(logical: &[u8], node: &RecordNode) -> Option<(usize, usize, u8)> {
    use super::tslv::Flags;
    let header_mask = node.mask ^ (node.rtype as u8);
    let fw = [
        logical.get(node.offset)? ^ header_mask,
        logical.get(node.offset + 1)? ^ header_mask,
    ];
    let width = Flags::decode(&fw).len_kind as usize;
    if width == 0 || node.content_start < node.offset + width {
        return None;
    }
    Some((node.content_start - width, width, header_mask))
}

/// Parse the whole logical report into a recursive record tree (top-level records read under
/// mask 0). For `Contents`-style streams (strict subtype-`0x07` headers).
pub(crate) fn parse_tree(logical: &[u8]) -> Vec<RecordNode> {
    parse_seq(logical, 0, logical.len(), 0, 0, Dialect::Contents)
}

/// Parse a `QESession` logical record stream into a recursive tree. Same bit-packed TSLV
/// framing + stack-XOR mask as [`parse_tree`], but with the relaxed (non-`0x07`) subtype
/// constraint that `QESession` records use.
pub(crate) fn parse_tree_qe(logical: &[u8]) -> Vec<RecordNode> {
    parse_seq(logical, 0, logical.len(), 0, 0, Dialect::Qe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_nested_record() {
        // Outer record type 0x10 (flag f8, subtype 07 00, len = 8) whose content is one inner
        // record type 0x03 with 0 content. Inner header is masked by the outer mask 0x10.
        let inner_mask = 0x10u8;
        let inner: Vec<u8> = [0xf8u8, 0x03, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00]
            .iter()
            .map(|b| b ^ inner_mask)
            .collect();
        let mut stream = vec![0xf8u8, 0x10, 0x07, 0x00, 0x00, 0x00, 0x00, 0x08];
        stream.extend(inner);

        let tree = parse_tree(&stream);
        assert_eq!(tree.len(), 1);
        let outer = &tree[0];
        assert_eq!(outer.rtype, 0x10);
        assert_eq!(outer.mask, 0x10);
        assert_eq!(outer.children.len(), 1);
        let child = &outer.children[0];
        assert_eq!(child.rtype, 0x03);
        assert_eq!(child.mask, 0x10 ^ 0x03);
        assert!(child.is_leaf());
    }

    #[test]
    fn serialize_tree_round_trips_nested_records() {
        let inner_mask = 0x10u8;
        let inner: Vec<u8> = [0xf8u8, 0x03, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00]
            .iter()
            .map(|b| b ^ inner_mask)
            .collect();
        let mut stream = vec![0xf8u8, 0x10, 0x07, 0x00, 0x00, 0x00, 0x00, 0x08];
        stream.extend(inner);

        let tree = parse_tree(&stream);
        assert_eq!(serialize_tree(&tree, &stream), stream);
    }
}
