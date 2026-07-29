//! The stream header record (TSLV type `0xffff`).
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
    /// `useFixed` — records whether the stream was encrypted with the universal built-in AES key.
    ///
    /// **Inert, and the decode path deliberately ignores it — as the engine does.** It is not a
    /// selector between two key-derivation schemes; there is no second scheme. The engine reads it
    /// into the document and echoes it back on save, but sets the cipher up from the document's key,
    /// which is always the built-in one. Clearing it in a real report changes nothing:
    /// the designer opens such a file normally, with no password prompt. Reported here for
    /// inspection (`rpt inspect` prints it) so a file that ever deviates is visible.
    pub use_fixed_key: bool,
    /// The 16-byte initialization vector (empty when not encrypted).
    pub iv: Vec<u8>,
}

impl StreamHeader {
    /// The record type of the stream header.
    pub(crate) const RECORD_TYPE: u16 = 0xFFFF;
}
