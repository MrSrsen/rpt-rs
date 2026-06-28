//! L0.5 + L1 — the byte↔record transform.
//!
//! Decodes a raw TSLV stream into the lossless record substrate and re-encodes it back to
//! bytes, with `encode(decode(x)) == x`. Byte-exact round-trip is guaranteed by retaining
//! the original framing (the `raw` safety net) for any record type not yet delimited
//! canonically.

mod archive;
mod crypto;
mod digest;
mod header;
mod mask;
mod qe_crypto;
mod tile;
mod tree;
mod tslv;

pub(crate) use archive::ReadArchive;
pub(crate) use digest::md5_base64;
pub use header::StreamHeader;
pub(crate) use tile::{tile, TiledRecord};
pub use tree::RecordNode;
pub(crate) use tree::{parse_tree, parse_tree_qe};

use crate::error::{Error, Result};

/// Decode the type-`0xffff` stream header from a raw TSLV stream.
///
/// Returns the parsed [`StreamHeader`] (flags + IV).
pub(crate) fn decode_stream_header(bytes: &[u8]) -> Result<StreamHeader> {
    ReadArchive::new(bytes).load_stream_header()
}

/// Fully decode a `Contents`-style TSLV stream into its **logical report bytes**:
/// parse the stream header, CFB-decrypt the payload, and zlib-inflate it. The result is the
/// flat record stream consumed by [`tile`].
pub(crate) fn decode_contents(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut archive = ReadArchive::new(bytes);
    let header = archive.load_stream_header()?;
    let body = &bytes[archive.top_record_end()..];

    let deflate = if header.is_encrypted {
        let iv: [u8; 16] = header
            .iv
            .as_slice()
            .try_into()
            .map_err(|_| Error::crypto_msg("stream header IV is not 16 bytes"))?;
        crypto::cfb_decrypt(&iv, body)
    } else {
        body.to_vec()
    };

    miniz_oxide::inflate::decompress_to_vec_zlib(&deflate)
        .map_err(|e| Error::codec(format!("inflate failed: {e:?}")))
}

/// The plaintext magic of a `QESession` (Query Engine) stream.
const QENG_MAGIC: &[u8; 4] = b"QENG";

/// Fully decode a `QESession` (`QENG`) stream into its **logical record bytes**: parse the
/// QENG header, AES-128-CFB decrypt the payload (fixed QE key, IV from the header), and
/// zlib-inflate it. The result is a flat TSLV record stream — the same masked record framing
/// as `Contents` — consumed by [`tile`] / [`parse_tree`].
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
        return Err(Error::codec("not a QENG stream".to_string()));
    }
    let iv: [u8; 16] = bytes[IV_OFF..BODY_OFF]
        .try_into()
        .map_err(|_| Error::crypto_msg("QENG header IV is not 16 bytes"))?;
    let deflate = qe_crypto::qe_cfb_decrypt(&iv, &bytes[BODY_OFF..]);
    miniz_oxide::inflate::decompress_to_vec_zlib(&deflate)
        .map_err(|e| Error::codec(format!("QE inflate failed: {e:?}")))
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
// TODO: temporarily `pub`; revert to `pub(crate)` once the PromptManager parameter schema is
// fully mapped.
pub fn decode_prompt_manager(bytes: &[u8]) -> Option<String> {
    let plain = crypto::cfb_decrypt(&[0u8; 16], bytes);
    let inflated = miniz_oxide::inflate::decompress_to_vec_zlib(&plain).ok()?;
    let xml = String::from_utf8_lossy(&inflated).into_owned();
    xml.contains("CRMetaObjects").then_some(xml)
}
