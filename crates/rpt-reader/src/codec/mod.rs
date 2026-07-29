//! The byte ↔ record transform.
//!
//! Decodes a raw TSLV stream into records and re-encodes it back to bytes, with
//! `encode(decode(x)) == x`. Byte-exact round-trip is guaranteed by retaining the original
//! framing (the `raw` safety net) for any record type not delimited canonically.

mod aes128;
mod archive;
mod containers;
mod crypto;
mod dialect;
mod edit;
mod flat_records;
mod header;
mod mask;
mod qe_crypto;
mod saved;
mod tree;
pub(crate) mod tslv;

pub(crate) use archive::ReadArchive;
pub use containers::RecordSearch;
pub use dialect::Dialect;
pub(crate) use edit::resize_joined_region;
pub(crate) use flat_records::{split_records, FlatRecord};
pub use header::StreamHeader;
pub(crate) use saved::{
    decode_saved_rows, inspect_saved_batches, BatchDesc, SavedBatch, SavedCatalog, SavedFieldDesc,
};
pub use tree::RecordNode;
pub(crate) use tree::{
    parse_tree, parse_tree_catalog, parse_tree_qe_session, parse_tree_report_parameters,
    serialize_tree, PieceSpan,
};

use crate::error::{CodecError, CryptoError, Result};

/// Decode the type-`0xffff` stream header from a raw TSLV stream.
///
/// Returns the parsed [`StreamHeader`] (flags + IV).
pub(crate) fn decode_stream_header(bytes: &[u8]) -> Result<StreamHeader> {
    ReadArchive::new(bytes).load_stream_header()
}

/// Fully decode a `Contents`-style TSLV stream into its **logical report bytes**:
/// parse the stream header, CFB-decrypt the payload, and zlib-inflate it. The result is the
/// flat record stream consumed by [`split_records`].
pub(crate) fn decode_contents(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut archive = ReadArchive::new(bytes);
    let header = archive.load_stream_header()?;
    // The payload starts where the header record's own declared length ends — a length read from the
    // stream, so on a damaged or non-report stream it can point past the end. Report that as a
    // decode error rather than slicing out of bounds.
    let body_start = archive.top_record_end();
    let body = bytes.get(body_start..).ok_or_else(|| {
        CodecError::new(format!(
            "the stream header declares a payload starting at {body_start}, past the end of the \
             {}-byte stream",
            bytes.len()
        ))
        .at(body_start)
    })?;

    let deflate = if header.is_encrypted {
        // The header's `useFixed` flag is deliberately NOT consulted here: the engine ignores it too.
        // The cipher always uses the same universal built-in key regardless of the flag — a report
        // whose only change is `useFixed` cleared to 0 opens normally in the designer, with no
        // password prompt. Branching on the flag here would make us refuse a file the engine reads
        // happily.
        let iv: [u8; 16] = header.iv.as_slice().try_into().map_err(|_| {
            CryptoError::new(
                "stream header",
                format!("IV is {} bytes, expected 16", header.iv.len()),
            )
        })?;
        crypto::cfb_decrypt(&iv, body)
    } else {
        body.to_vec()
    };

    miniz_oxide::inflate::decompress_to_vec_zlib(&deflate).map_err(|e| {
        // An encrypted stream that decrypts to non-zlib data almost always means the payload was not
        // keyed with the universal built-in key. Crystal itself never produces such a file — but a
        // third-party application with its own copy of the encryption library can key a report under
        // its own key and round-trip it, and those are unreadable here because that key lives in the
        // application, not in the file. Say so, instead of blaming zlib for a key problem.
        //
        // The header's `useFixed` flag is reported as a hint but is NOT the test: it and the key are
        // independent setters, so a foreign key can perfectly well arrive with the flag still set.
        if header.is_encrypted {
            let hint = if header.use_fixed_key {
                "the header claims the built-in key"
            } else {
                "the header declares a non-built-in key (useFixed = 0)"
            };
            return CryptoError::new(
                "Contents",
                format!(
                    "decrypted payload is not zlib data ({hint}) — the stream is most likely \
                     encrypted with a key this reader does not have. Crystal never writes such a \
                     file; a third-party application with its own copy of the encryption key can, \
                     and that key is not stored in the report. Underlying inflate status: {:?}",
                    e.status
                ),
            )
            .into();
        }
        // `DecompressError`'s Debug carries the whole partial output buffer — log only the status.
        CodecError::new(format!(
            "zlib inflate of Contents payload failed: {:?}",
            e.status
        ))
        .in_stream("Contents")
        .into()
    })
}

/// Re-encode a `Contents`-style TSLV stream from new **logical report bytes** — the exact inverse
/// of [`decode_contents`]. The original stream-header prefix (record type `0xffff`, carrying the
/// crypto flags + IV) is retained verbatim; the payload is zlib-deflated, then CFB-encrypted with
/// the header IV when the stream declares encryption.
///
/// `raw` is the original on-disk stream (its header + payload); `new_logical` is the replacement
/// inflated record stream. The result re-decodes (`decode_contents`) to `new_logical` byte-for-byte.
/// Deflate is non-canonical, so the output is **not** byte-identical to `raw` even for an unchanged
/// `new_logical` — only the decoded logical bytes round-trip.
pub(crate) fn encode_contents(raw: &[u8], new_logical: &[u8]) -> Result<Vec<u8>> {
    let mut archive = ReadArchive::new(raw);
    let header = archive.load_stream_header()?;
    let body_off = archive.top_record_end();
    let prefix = raw.get(..body_off).ok_or_else(|| {
        CodecError::new(format!(
            "stream-header prefix ends at offset {body_off} but stream is only {} bytes",
            raw.len()
        ))
    })?;

    // Any valid zlib level round-trips at the inflated level; the engine accepts our re-deflate.
    let deflate = miniz_oxide::deflate::compress_to_vec_zlib(new_logical, DEFLATE_LEVEL);
    let body = if header.is_encrypted {
        let iv: [u8; 16] = header.iv.as_slice().try_into().map_err(|_| {
            CryptoError::new(
                "stream header",
                format!("IV is {} bytes, expected 16", header.iv.len()),
            )
        })?;
        crypto::cfb_encrypt(&iv, &deflate)
    } else {
        deflate
    };

    let mut out = Vec::with_capacity(prefix.len() + body.len());
    out.extend_from_slice(prefix);
    out.extend_from_slice(&body);
    Ok(out)
}

/// zlib level for re-deflating an encoded payload. Any valid level decodes identically; 6 is the
/// zlib default and keeps the output size close to the engine's own.
const DEFLATE_LEVEL: u8 = 6;

/// The plaintext magic of a `QESession` (Query Engine) stream.
const QENG_MAGIC: &[u8; 4] = b"QENG";

/// Fully decode a `QESession` (`QENG`) stream into its **logical record bytes**: parse the
/// QENG header, AES-128-CFB decrypt the payload (fixed QE key, IV from the header), and
/// zlib-inflate it. The result is a flat TSLV record stream — the same masked record framing
/// as `Contents` — consumed by [`split_records`] / [`parse_tree`].
///
/// Header layout (little-endian):
/// ```text
/// 0x00  "QENG"            magic
/// 0x04  u32  = 1          version
/// 0x08  u32, u32          constant flags
/// 0x10  u32               (per-file; checksum-like, unused)
/// 0x14  u16  = 1          isEncrypted
/// 0x16  u8[16]            AES-128-CFB IV
/// 0x26  …                 encrypted + zlib-compressed record stream
/// ```
pub(crate) fn decode_qe(bytes: &[u8]) -> Result<Vec<u8>> {
    const IV_OFF: usize = 0x16;
    const BODY_OFF: usize = 0x26;
    if bytes.len() < BODY_OFF || &bytes[0..4] != QENG_MAGIC {
        let found = &bytes[..4.min(bytes.len())];
        return Err(CodecError::new(format!(
            "expected QENG magic {QENG_MAGIC:?} at offset 0, found {found:02x?}"
        ))
        .in_stream("QESession")
        .into());
    }
    let iv: [u8; 16] = bytes[IV_OFF..BODY_OFF].try_into().map_err(|_| {
        CryptoError::new(
            "QENG header",
            format!("IV is {} bytes, expected 16", BODY_OFF - IV_OFF),
        )
    })?;
    let deflate = qe_crypto::qe_cfb_decrypt(&iv, &bytes[BODY_OFF..]);
    miniz_oxide::inflate::decompress_to_vec_zlib(&deflate).map_err(|e| {
        CodecError::new(format!("zlib inflate of QESession payload failed: {e:?}"))
            .in_stream("QESession")
            .into()
    })
}

/// True if `bytes` begins with the `QENG` (Query Engine session) magic.
pub(crate) fn is_qe(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[0..4] == QENG_MAGIC
}

/// Decode the `PromptManager` stream into its `<CRMetaObjects>` XML (parameter / prompt-group
/// metadata). The `PromptManager` payload uses the *same* modified-Rijndael as `Contents`
/// but with a **zero IV** at offset 0, and inflates to one (or more) zlib-wrapped
/// CRMetaObjects documents. Returns the first document — the parameter definitions
/// — as a UTF-8 string. (A multi-record prompt manager keeps subsequent prompt-group records in
/// later blocks; the parameter objects we need are in the first.)
pub(crate) fn decode_prompt_manager(bytes: &[u8]) -> Option<String> {
    let plain = crypto::cfb_decrypt(&[0u8; 16], bytes);
    let inflated = miniz_oxide::inflate::decompress_to_vec_zlib(&plain).ok()?;
    let xml = String::from_utf8_lossy(&inflated).into_owned();
    xml.contains("CRMetaObjects").then_some(xml)
}
