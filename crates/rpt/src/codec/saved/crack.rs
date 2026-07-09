//! The saved-batch cipher layer: build a batch's decrypt IV, gate on the zlib magic, CFB-decrypt,
//! and inflate — plus the brute-force IV search used to crack an undecoded batch class.
//!
//! The batch IV is four little-endian words:
//! `[batch_size | item_count | persistent_item_size | u16 batch_sequence]`. Batches of
//! one kind sit back-to-back and are walked by consumed length; the 4th IV word is the batch's 0-based
//! sequence number within its kind (0 for the first — which is why single-batch reports decode with a
//! zero tail).

use crate::codec::crypto::{cfb_decrypt, encrypt_block};

use super::schema::{BatchDesc, INDEX_BATCH_SIZE};

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

/// Decode one saved-data batch to its inflated record bytes: CFB-decrypt with the IV built from the
/// batch metadata, then zlib-inflate. `None` if the metadata is wrong (decrypted block 0 is not a
/// zlib header) or the stream is not a saved batch. (Single-batch helper retained for the cipher
/// round-trip unit tests; the pipeline uses [`decode_batch_at`].)
#[cfg(test)]
pub(crate) fn decode_saved_batch(
    ciphertext: &[u8],
    batch_size: u32,
    item_count: u32,
    item_size: u32,
) -> Option<Vec<u8>> {
    if ciphertext.len() < 16 {
        return None;
    }
    let iv = batch_iv(batch_size, item_count, item_size);
    // Cheap block-0 zlib-magic check before the full decrypt + inflate.
    let ks = encrypt_block(&iv);
    if ciphertext[0] ^ ks[0] != 0x78 {
        return None;
    }
    let plain = cfb_decrypt(&iv, ciphertext);
    miniz_oxide::inflate::decompress_to_vec_zlib(&plain).ok()
}

/// Decode the whole `SavedRecordsStream` — the record index, possibly split across several
/// back-to-back batches — into the concatenated fixed-width record bytes (each batch's header and
/// allocation region stripped). `index_batches` is [`super::schema::index_directory`] (each batch's
/// `count` and shared `item_size`); every batch is an independent `zlib`-then-CFB stream with IV
/// `(INDEX_BATCH_SIZE, count, item_size)`, sitting back-to-back, so the consumed (ciphertext =
/// plaintext, CFB is a stream cipher) length gives the next batch's offset. Returns the record bytes
/// (`decoded_count * item_size` long — see below). `None` if not even the first batch decodes.
///
/// **Tolerant:** it stops at the first batch that fails to decode and returns the records gathered so
/// far. Each batch's IV tail is its 0-based sequence number, so a multi-batch index decodes fully
/// regardless of how many batches it spans. The caller derives the reconstructed record
/// count from the returned length (`len / item_size`).
pub(crate) fn decode_index_stream(srs_raw: &[u8], index_batches: &[BatchDesc]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut off = 0usize;
    for (k, b) in index_batches.iter().enumerate() {
        let Some(ct) = srs_raw.get(off..) else { break };
        // The k-th index batch's IV tail is its 0-based sequence number.
        let iv = batch_iv4(INDEX_BATCH_SIZE, b.count, b.item_size, k as u32);
        let ks = encrypt_block(&iv);
        if ct.first().copied().map(|c| c ^ ks[0]) != Some(0x78) {
            break;
        }
        let plain = cfb_decrypt(&iv, ct);
        let Some((inflated, consumed)) = inflate_zlib_counted(&plain) else {
            break;
        };
        // Records sit at the tail of the batch, after the header + allocation region.
        let Some(need) = (b.count as usize).checked_mul(b.item_size as usize) else {
            break;
        };
        let Some(data_start) = inflated.len().checked_sub(need) else {
            break;
        };
        out.extend_from_slice(&inflated[data_start..]);
        off += consumed;
    }
    (!out.is_empty()).then_some(out)
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
/// consumed_ciphertext_len)`; `None` if the IV is wrong (block 0 is not a zlib header) or inflate
/// fails, so the caller can stop the run.
pub(crate) fn decode_batch_at(
    raw: &[u8],
    cursor: usize,
    batch_size: u32,
    item_count: u32,
    item_size: u32,
    seq: u32,
) -> Option<(Vec<u8>, usize)> {
    let ct = raw.get(cursor..)?;
    let iv = batch_iv4(batch_size, item_count, item_size, seq);
    let ks = encrypt_block(&iv);
    if *ct.first()? ^ ks[0] != 0x78 {
        return None;
    }
    let plain = cfb_decrypt(&iv, ct);
    inflate_zlib_counted(&plain)
}

/// The 4-byte zlib flag bytes that follow a `0x78` CMF byte (`(0x7800 | FLG) % 31 == 0`).
fn is_zlib_flag(second: u8) -> bool {
    (0x7800u16 | second as u16) % 31 == 0
}

/// Cheap block-0 zlib-header gate: decrypt the first two bytes with `iv`'s keystream and test for a
/// `0x78`/valid-FLG zlib header. Both bytes come from block 0 (`E(iv)`), so this is one AES call.
pub(crate) fn block0_is_zlib(iv: &[u8; 16], ct: &[u8]) -> bool {
    if ct.len() < 2 {
        return false;
    }
    let ks = encrypt_block(iv);
    ct[0] ^ ks[0] == 0x78 && is_zlib_flag(ct[1] ^ ks[1])
}

/// Brute-force the saved-batch IV metadata `(batch_size, item_count, item_size, seq)` over the given
/// candidate values, returning every tuple whose IV both passes the zlib-magic gate and fully
/// inflates `ct`. This is the instrument that cracks an undecoded batch class: when the directory's
/// item metadata does not match the real IV words, search the neighbourhood to recover them. Stops
/// after `limit` hits (`0` = unbounded).
pub(crate) fn saved_iv_search(
    ct: &[u8],
    batch_sizes: &[u32],
    item_counts: &[u32],
    item_sizes: &[u32],
    seqs: &[u32],
    limit: usize,
) -> Vec<crate::model::SavedIvHit> {
    let mut hits = Vec::new();
    for &bs in batch_sizes {
        for &ic in item_counts {
            for &is in item_sizes {
                for &seq in seqs {
                    let iv = batch_iv4(bs, ic, is, seq);
                    if !block0_is_zlib(&iv, ct) {
                        continue;
                    }
                    let plain = cfb_decrypt(&iv, ct);
                    if let Some((inflated, _consumed)) = inflate_zlib_counted(&plain) {
                        hits.push(crate::model::SavedIvHit {
                            batch_size: bs,
                            item_count: ic,
                            item_size: is,
                            seq,
                            inflated_len: inflated.len(),
                        });
                        if limit != 0 && hits.len() >= limit {
                            return hits;
                        }
                    }
                }
            }
        }
    }
    hits
}

/// CFB-encrypt, for the round-trip tests only.
#[cfg(test)]
pub(crate) fn cfb_encrypt(iv: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(plaintext.len());
    let mut feedback = *iv;
    for chunk in plaintext.chunks(16) {
        let ks = encrypt_block(&feedback);
        let mut block = [0u8; 16];
        for (i, &p) in chunk.iter().enumerate() {
            let c = p ^ ks[i];
            out.push(c);
            block[i] = c;
        }
        if chunk.len() == 16 {
            feedback = block;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_batch_with_known_metadata() {
        let original: Vec<u8> = (0..2000u32)
            .flat_map(|i| (i as u16).to_le_bytes())
            .collect();
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&original, 6);
        let ct = cfb_encrypt(&batch_iv(1000, 249, 30), &z);
        let decoded = decode_saved_batch(&ct, 1000, 249, 30).expect("decode");
        assert_eq!(decoded, original);
    }

    #[test]
    fn wrong_metadata_fails() {
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&[1u8; 500], 6);
        let ct = cfb_encrypt(&batch_iv(1000, 249, 30), &z);
        assert!(decode_saved_batch(&ct, 1000, 248, 30).is_none());
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
