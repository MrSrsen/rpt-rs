//! The saved-batch cipher layer: build a batch's decrypt IV, gate on the zlib magic, CFB-decrypt,
//! and inflate.
//!
//! The batch IV is four little-endian words:
//! `[batch_size | item_count | persistent_item_size | u16 batch_sequence]`. Batches of
//! one kind sit back-to-back and are walked by consumed length; the 4th IV word is the batch's 0-based
//! sequence number within its kind (0 for the first — which is why single-batch reports decode with a
//! zero tail).

use crate::codec::crypto::{cfb_decrypt, encrypt_block};
use crate::coverage::BatchProblem;

/// The zlib CMF byte (deflate, 32 KiB window) that opens every saved-data batch's decrypted stream —
/// the cheap gate that a batch's decrypt IV is correct before the full CFB-decrypt + inflate.
pub(crate) const ZLIB_CMF: u8 = 0x78;

pub(crate) fn batch_iv(batch_size: u32, item_count: u32, item_size: u32) -> [u8; 16] {
    batch_iv4(batch_size, item_count, item_size, 0)
}

/// The saved-batch decrypt IV:
/// `[batch_size | item_count | persistent_item_size | u16 batch_sequence]`. The 4th word is the
/// batch's 0-based sequence index within its group (index-group or descriptor-group); it is 0 for the
/// first batch (which is why single-batch reports decode with a zero tail) and increments per batch.
pub(crate) fn batch_iv4(batch_size: u32, item_count: u32, item_size: u32, seq: u32) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[0..4].copy_from_slice(&batch_size.to_le_bytes());
    iv[4..8].copy_from_slice(&item_count.to_le_bytes());
    iv[8..12].copy_from_slice(&item_size.to_le_bytes());
    iv[12..14].copy_from_slice(&(seq as u16).to_le_bytes());
    iv
}

/// Inflate a zlib stream, returning `(inflated_bytes, input_bytes_consumed)`. Unlike
/// `decompress_to_vec_zlib`, this reports how many input bytes the stream consumed — needed because
/// `MemoValuesStream` concatenates several batches back-to-back and the consumed length is the next
/// batch's offset. `None` if the input is not a valid zlib stream.
pub(crate) fn inflate_zlib_counted(input: &[u8]) -> Option<(Vec<u8>, usize)> {
    use miniz_oxide::inflate::core::{decompress, inflate_flags, DecompressorOxide};
    use miniz_oxide::inflate::TINFLStatus;
    let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
        | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    let mut decomp = DecompressorOxide::new();
    let mut out: Vec<u8> = vec![0; input.len().saturating_mul(2).max(8192)];
    let (mut in_pos, mut out_pos) = (0usize, 0usize);
    loop {
        let (status, in_c, out_c) =
            decompress(&mut decomp, &input[in_pos..], &mut out, out_pos, flags);
        in_pos += in_c;
        out_pos += out_c;
        match status {
            TINFLStatus::Done => {
                out.truncate(out_pos);
                return Some((out, in_pos));
            }
            TINFLStatus::HasMoreOutput => {
                let new_len = out.len().saturating_mul(2);
                out.resize(new_len, 0);
            }
            _ => return None,
        }
    }
}

/// Decode one saved-data batch at `raw[cursor..]`: build the IV from `(batch_size, item_count,
/// item_size, seq)`, gate on the zlib magic, CFB-decrypt, and inflate. Returns `(inflated_bytes,
/// consumed_ciphertext_len)`.
///
/// The failure is named rather than merely reported, because the three ways a batch can fail mean
/// different things: a batch past the end of the stream is a directory that outruns its bytes, a
/// block 0 that is not a zlib header is metadata keying the cipher wrongly, and a plaintext that
/// will not inflate is neither.
pub(crate) fn decode_batch_at(
    raw: &[u8],
    cursor: usize,
    batch_size: u32,
    item_count: u32,
    item_size: u32,
    seq: u32,
) -> Result<(Vec<u8>, usize), BatchProblem> {
    let ct = raw
        .get(cursor..)
        .filter(|c| !c.is_empty())
        .ok_or(BatchProblem::Absent)?;
    let iv = batch_iv4(batch_size, item_count, item_size, seq);
    let ks = encrypt_block(&iv);
    if ct[0] ^ ks[0] != ZLIB_CMF {
        return Err(BatchProblem::NotDecrypted);
    }
    let plain = cfb_decrypt(&iv, ct);
    inflate_zlib_counted(&plain).ok_or(BatchProblem::NotInflated)
}

/// The 4-byte zlib flag bytes that follow a `0x78` CMF byte (`(0x7800 | FLG) % 31 == 0`).
fn is_zlib_flag(second: u8) -> bool {
    (0x7800u16 | second as u16).is_multiple_of(31)
}

/// Cheap block-0 zlib-header gate: decrypt the first two bytes with `iv`'s keystream and test for a
/// `0x78`/valid-FLG zlib header. Both bytes come from block 0 (`E(iv)`), so this is one AES call.
pub(crate) fn block0_is_zlib(iv: &[u8; 16], ct: &[u8]) -> bool {
    if ct.len() < 2 {
        return false;
    }
    let ks = encrypt_block(iv);
    ct[0] ^ ks[0] == ZLIB_CMF && is_zlib_flag(ct[1] ^ ks[1])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::crypto::cfb_encrypt;

    #[test]
    fn decodes_a_batch_with_known_metadata() {
        let original: Vec<u8> = (0..2000u32)
            .flat_map(|i| (i as u16).to_le_bytes())
            .collect();
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&original, 6);
        let ct = cfb_encrypt(&batch_iv(1000, 249, 30), &z);
        let (decoded, consumed) = decode_batch_at(&ct, 0, 1000, 249, 30, 0).expect("decode");
        assert_eq!(decoded, original);
        assert_eq!(consumed, z.len());
    }

    #[test]
    fn wrong_metadata_fails() {
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&[1u8; 500], 6);
        let ct = cfb_encrypt(&batch_iv(1000, 249, 30), &z);
        assert_eq!(
            decode_batch_at(&ct, 0, 1000, 248, 30, 0),
            Err(BatchProblem::NotDecrypted)
        );
    }

    #[test]
    fn inflate_reports_consumed_length() {
        // Two zlib streams back-to-back; inflating the first must consume exactly its own bytes.
        let a = miniz_oxide::deflate::compress_to_vec_zlib(b"first batch payload", 6);
        let b = miniz_oxide::deflate::compress_to_vec_zlib(b"second", 6);
        let mut concat = a.clone();
        concat.extend_from_slice(&b);
        let (out, consumed) = inflate_zlib_counted(&concat).expect("inflate");
        assert_eq!(out, b"first batch payload");
        assert_eq!(consumed, a.len());
    }
}
