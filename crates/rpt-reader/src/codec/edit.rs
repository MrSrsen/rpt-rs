//! The length-changing write primitive.
//!
//! The record tree ([`super::tree`]) only ever reads: it frames the bytes it was handed and leaves
//! them as they are. This is the one operation that rewrites them — a record's field bytes are
//! replaced by bytes of a different length — and so the one that can fail, because a length prefix
//! that cannot hold the new length must stop the edit rather than truncate it.

use super::tree::RecordNode;
use crate::error::{CodecError, Result};

/// Replace a region of `target`'s own field bytes with `new_bytes` of a **possibly different
/// length**, returning the rewritten logical stream.
///
/// TSLV length prefixes are relative and nested: a record's stored length covers its whole content
/// span, *including* every nested child. So growing/shrinking a record's field bytes by `Δ`
/// grows/shrinks the
/// length prefix of the edited record **and every ancestor up the chain** by exactly `Δ` — nothing
/// else. There are no absolute byte offsets in the `Contents` record tree, so no other stored value
/// needs fixing.
///
/// `joined_region` is in the coordinate space of `target`'s runs **joined**, and must lie within a
/// single run (it may not straddle a child record — that would splice bytes into a child's
/// framing). `ancestors` is `target`'s ancestor chain (any order; each is a record whose
/// content span encloses `target`). `new_bytes` are demasked; they are re-masked with `target`'s
/// stack mask before insertion. Every rewritten length prefix must still fit its on-disk field
/// width, or the edit is rejected (an `Err`, never a corrupt stream).
pub(crate) fn resize_joined_region(
    logical: &[u8],
    target: &RecordNode,
    ancestors: &[&RecordNode],
    joined_region: std::ops::Range<usize>,
    new_bytes: &[u8],
) -> Result<Vec<u8>> {
    if joined_region.start > joined_region.end {
        return Err(CodecError::new(format!(
            "resize region start {} > end {}",
            joined_region.start, joined_region.end
        ))
        .record(target.rtype)
        .into());
    }
    // Map the region to a contiguous logical byte range within one run.
    let (log_start, log_end) = map_joined_region(target, &joined_region).ok_or_else(|| {
        CodecError::new(format!(
            "resize region [{}, {}) is out of the record's field bytes or straddles a child record",
            joined_region.start, joined_region.end
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

/// Map a region `[start, end)` of `node`'s concatenated runs to a contiguous logical byte range, or
/// `None` if it is out of them or straddles a child record (crosses a run boundary).
fn map_joined_region(node: &RecordNode, region: &std::ops::Range<usize>) -> Option<(usize, usize)> {
    let spans: Vec<(usize, usize)> = node.run_spans().collect();
    let mut base = 0usize; // position in the concatenation at the start of the current run
    for (s, e) in &spans {
        let run_len = e - s;
        // Both endpoints must land within this one run (end may equal the run's end).
        if region.start >= base && region.end <= base + run_len {
            return Some((s + (region.start - base), s + (region.end - base)));
        }
        base += run_len;
    }
    // Allow an empty region at the very end of a record with no runs at all: only [0,0).
    (spans.is_empty() && region.start == 0 && region.end == 0)
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
