//! L1 — TSLV record-header bit-packing.
//!
//! A record header starts with a 2-byte little-endian **bit-packed flag word** (read
//! through `load_block`, so already demasked). The bits:
//!
//! | bit | meaning |
//! |-----|---------|
//! | 7,6 | length-field size: `00`→0, `01`→1, `10`→2, `11`→4 bytes |
//! | 5   | extended **type** word follows |
//! | 4   | flag (unused on the read path) |
//! | 3   | `useSimpleEncryption` |
//! | 2   | extended **value** follows (else the low bits are the inline value) |
//!
//! On-disk multi-byte scalars (type, length) are **big-endian**. This module holds the
//! pure bit/byte semantics; the stateful, record-spanning reads live in [`super::archive`].

/// Per-bit masks indexed by bit number.
const BITMASKS: [u8; 8] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80];

/// Test bit `bit` of a 2-byte little-endian word buffer.
pub(crate) fn test_bit(word: &[u8; 2], bit: usize) -> bool {
    (BITMASKS[bit & 7] & word[bit >> 3]) != 0
}

/// Number of bytes the length field occupies, from flag bits 7 and 6.
pub(crate) fn len_kind(word: &[u8; 2]) -> u8 {
    let b7 = test_bit(word, 7);
    let b6 = test_bit(word, 6);
    if b7 {
        if b6 {
            4
        } else {
            2
        }
    } else if b6 {
        1
    } else {
        0
    }
}

/// The decoded flag bits of a record header's first word.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Flags {
    pub len_kind: u8,
    /// bit 5 — an extended type/subtype word follows.
    pub extended_type: bool,
    /// bit 2 — an extended value (the 2-byte type) follows; else the type is inline.
    pub extended_value: bool,
    /// bit 3 — `useSimpleEncryption` (the running XOR mask is in effect).
    #[allow(dead_code)] // consulted by the write path; decoded here for completeness
    pub simple_encryption: bool,
}

impl Flags {
    pub(crate) fn decode(word: &[u8; 2]) -> Flags {
        Flags {
            len_kind: len_kind(word),
            extended_type: test_bit(word, 5),
            extended_value: test_bit(word, 2),
            simple_encryption: test_bit(word, 3),
        }
    }
}

/// Clear the flag bits (7,6,5,4,3,2) from an inline-value word, leaving the inline value.
pub(crate) fn clear_flag_bits(word: &mut [u8; 2]) {
    for &i in &[7usize, 6, 5, 4, 3, 2] {
        word[i >> 3] &= !BITMASKS[i & 7];
    }
}

/// Decode a big-endian scalar held in `bytes` (length 1, 2, or 4) to a `u64`.
pub(crate) fn be_scalar(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn len_kind_decodes_all_four_sizes() {
        assert_eq!(len_kind(&[0b0000_0000, 0]), 0);
        assert_eq!(len_kind(&[0b0100_0000, 0]), 1);
        assert_eq!(len_kind(&[0b1000_0000, 0]), 2);
        assert_eq!(len_kind(&[0b1100_0000, 0]), 4);
    }

    #[test]
    fn be_scalar_is_big_endian() {
        assert_eq!(be_scalar(&[0x00, 0x18]), 24);
        assert_eq!(be_scalar(&[0x01, 0x00]), 256);
        assert_eq!(be_scalar(&[0xff]), 255);
    }

    #[test]
    fn clear_flag_bits_keeps_low_value_bits() {
        // bit2 set (flag) + low value bits 0b11 -> after clearing flags, value 0b11.
        let mut w = [0b0000_0111u8, 0x00];
        clear_flag_bits(&mut w);
        assert_eq!(w, [0b0000_0011, 0x00]);
    }
}
