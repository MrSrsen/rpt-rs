//! L0.5 — the stream header record (TSLV type `0xffff`).
//!
//! The first record of a TSLV stream carries the stream-level crypto parameters. Read
//! through the masked `load_block` path, its body is:
//!
//! ```text
//! isEnc(2)  version(2)  useFixed(2)  IV(16, only if isEnc)
//! ```

/// The decoded type-`0xffff` stream header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamHeader {
    /// `isEnc` — whether the stream declares encryption (the XOR mask / cipher path).
    pub is_encrypted: bool,
    /// Format version word.
    pub version: u16,
    /// `useFixed` — whether the universal fixed AES key is in play.
    pub use_fixed_key: bool,
    /// The 16-byte initialization vector (empty when not encrypted).
    pub iv: Vec<u8>,
}

impl StreamHeader {
    /// The record type of the stream header.
    pub(crate) const RECORD_TYPE: u16 = 0xFFFF;
}
