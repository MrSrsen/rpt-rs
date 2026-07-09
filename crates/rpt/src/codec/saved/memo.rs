//! The memo (variable-length / blob) values: each row's memo cells point into the per-batch heaps
//! decoded from `MemoValuesStream`, whose entries are `(u32 len)(utf16z)`.

use crate::codec::crypto::{cfb_decrypt, encrypt_block};

use super::crack::{batch_iv, inflate_zlib_counted};

/// Decode the per-batch memo heaps from `MemoValuesStream`: each batch is `zlib`-then-CFB with the
/// shared memo IV `(memo_cols, memo_cols*12, 12, 0)`, laid back-to-back. Returns each batch's inflated
/// heap bytes (the raw `(u32 len)(utf16z)` entry region, indexed by the descriptor-cell offsets). The
/// k-th heap aligns 1:1 with the k-th descriptor batch (both split by the same memo batch size).
pub(crate) fn decode_memo_heaps(memo_raw: &[u8], memo_cols: u32) -> Vec<Vec<u8>> {
    let iv = batch_iv(memo_cols, memo_cols.saturating_mul(12), 12);
    let ks = encrypt_block(&iv);
    let mut heaps = Vec::new();
    let mut off = 0usize;
    while off + 16 <= memo_raw.len() {
        let ct = &memo_raw[off..];
        if ct[0] ^ ks[0] != 0x78 {
            break;
        }
        let plain = cfb_decrypt(&iv, ct);
        let Some((inflated, consumed)) = inflate_zlib_counted(&plain) else {
            break;
        };
        if consumed == 0 {
            break;
        }
        heaps.push(inflated);
        off += consumed;
    }
    heaps
}

/// Read one memo value out of a batch heap given a descriptor cell's `(offset, length)`. The offset
/// points at the entry's 4-byte length prefix; `length` is the value's UTF-16LE byte length including
/// the trailing NUL. Returns `None` for an empty value (length 0) or an out-of-bounds slice — an empty
/// persistent-memo is modelled as `None`.
pub(crate) fn read_memo_cell(heap: &[u8], offset: usize, length: usize) -> Option<String> {
    if length == 0 || offset + 4 + length > heap.len() {
        return None;
    }
    let data = &heap[offset + 4..offset + 4 + length];
    let s: String = char::decode_utf16(
        data.chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0),
    )
    .map(|r| r.unwrap_or('\u{FFFD}'))
    .collect();
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::super::crack::cfb_encrypt;
    use super::*;

    #[test]
    fn decode_memo_heaps_walks_all_batches_and_cells_resolve() {
        // Two concatenated memo batches, each `zlib` then CFB with the shared memo IV
        // `(memo_cols, memo_cols*12, 12)`, then read a value out via a descriptor cell.
        let memo_cols = 5u32;
        let iv = batch_iv(memo_cols, memo_cols * 12, 12);
        let entry = |s: &str| {
            let mut v = (s.len() as u32 * 2 + 2).to_le_bytes().to_vec();
            for c in s.encode_utf16() {
                v.extend_from_slice(&c.to_le_bytes());
            }
            v.extend_from_slice(&[0, 0]); // trailing UTF-16 NUL
            v
        };
        let mut b1 = Vec::new();
        let a_off = b1.len();
        b1.extend(entry("alpha"));
        let beta_off = b1.len();
        b1.extend(entry("beta"));
        let mut b2 = Vec::new();
        b2.extend(entry("gamma"));
        let mut stream = cfb_encrypt(&iv, &miniz_oxide::deflate::compress_to_vec_zlib(&b1, 6));
        stream.extend(cfb_encrypt(
            &iv,
            &miniz_oxide::deflate::compress_to_vec_zlib(&b2, 6),
        ));
        let heaps = decode_memo_heaps(&stream, memo_cols);
        assert_eq!(heaps.len(), 2);
        // A descriptor cell's `(offset, length)` points at the entry (length prefix), length is the
        // value's UTF-16 byte count including the trailing NUL.
        assert_eq!(
            read_memo_cell(&heaps[0], a_off, 12).as_deref(),
            Some("alpha")
        );
        assert_eq!(
            read_memo_cell(&heaps[0], beta_off, 10).as_deref(),
            Some("beta")
        );
        assert_eq!(read_memo_cell(&heaps[1], 0, 12).as_deref(), Some("gamma"));
    }
}
