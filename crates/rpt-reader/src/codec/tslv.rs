//! TSLV record-header bit-packing.
//!
//! A record header is `flag | typeLow [| typeWord] [| schema] [| length]`. The first two bytes
//! are a **bit-packed flag word** (read through `load_block`, so already demasked): the flag
//! byte proper, then the record type's low byte. The flag byte's bits:
//!
//! | bit | meaning |
//! |-----|---------|
//! | 7,6 | length-field size: `00`→0, `01`→1, `10`→2, `11`→4 bytes |
//! | 5   | a **schema** word follows |
//! | 4   | the record's **string wire format** — see [`Flags::strings_enhanced`] |
//! | 3   | `useSimpleEncryption` |
//! | 2   | the record type is an extended **value** — a separate word follows |
//! | 1,0 | the record type's **high** byte, packed inline (so `0xf9` is `0xf8` for types
//!        `0x0100`–`0x01ff`) |
//!
//! On-disk multi-byte scalars (type, schema, length) are **big-endian**, except the extended type
//! word, which is little-endian.
//!
//! This module holds the pure bit/byte semantics, and [`decode_header`] is the one decode of that
//! shape: both readers of a record sequence — the flat split and the tree scan — get their framing
//! from it and add only what is theirs (the scan its filters, the split its desync rule). The
//! stateful, record-spanning reads live in [`super::archive`], which reads one record in the whole
//! crate and keeps its own cursor to do it.

/// Per-bit masks indexed by bit number.
const BITMASKS: [u8; 8] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80];

/// Which bit of a header's first word carries which flag. The two length bits together state the
/// width of the length field; the rest are the flags [`Flags`] names.
const BIT_EXTENDED_VALUE: usize = 2;
const BIT_SIMPLE_ENCRYPTION: usize = 3;
const BIT_STRINGS_ENHANCED: usize = 4;
const BIT_HAS_SCHEMA: usize = 5;
const BIT_LENGTH_LOW: usize = 6;
const BIT_LENGTH_HIGH: usize = 7;

/// Every bit of the word that is a flag — what a word carrying an inline value is cleared of to
/// leave that value.
const FLAG_BITS: [usize; 6] = [
    BIT_LENGTH_HIGH,
    BIT_LENGTH_LOW,
    BIT_HAS_SCHEMA,
    BIT_STRINGS_ENHANCED,
    BIT_SIMPLE_ENCRYPTION,
    BIT_EXTENDED_VALUE,
];

/// Test bit `bit` of a 2-byte little-endian word buffer.
pub(crate) fn test_bit(word: &[u8; 2], bit: usize) -> bool {
    (BITMASKS[bit & 7] & word[bit >> 3]) != 0
}

/// Number of bytes the length field occupies, from the two length flag bits.
pub(crate) fn len_kind(word: &[u8; 2]) -> u8 {
    let high = test_bit(word, BIT_LENGTH_HIGH);
    let low = test_bit(word, BIT_LENGTH_LOW);
    if high {
        if low {
            4
        } else {
            2
        }
    } else if low {
        1
    } else {
        0
    }
}

/// Which of the two string wire forms a record's content is framed in.
///
/// Every record declares its own, in flag bit 4 of its header — see [`Flags::strings_enhanced`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StringFormat {
    /// A `u32` big-endian byte count, then that many bytes. The count covers the trailing NUL, so
    /// the empty string is `00 00 00 01 00`; a count of `0` is the null string and is followed by
    /// nothing.
    Enhanced,
    /// NUL-terminated bytes with no length prefix, scanned to the terminator.
    Simple,
}

/// The decoded flag bits of a record header's first word.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Flags {
    pub len_kind: u8,
    /// bit 5 — a schema word follows the type.
    pub has_schema: bool,
    /// bit 2 — an extended value (the 2-byte type) follows; else the type is inline.
    pub extended_value: bool,
    /// bit 3 — `useSimpleEncryption` (the running XOR mask is in effect). Every record a report
    /// writer emits sets it, which is why the scan uses it as its first evidence that a candidate
    /// offset is a header at all.
    pub simple_encryption: bool,
    /// bit 4 — which of the **two string wire formats** this record's content is framed in.
    ///
    /// | value | a string is |
    /// |-------|-------------|
    /// | `true` (*enhanced*) | a `u32` big-endian byte count, then that many UTF-8 bytes (the count covers the trailing NUL; a count of `0` is the null string and is followed by nothing) |
    /// | `false` (*simple*)  | NUL-terminated UTF-8 with **no** length prefix — scanned to the NUL, then read as a block |
    ///
    /// The choice is a setting on the archive doing the reading or writing, but it is not something
    /// a reader has to know out of band: the writer stamps its current setting into **every** record
    /// header as this bit, and the reader loads its own setting back out of the same bit at every
    /// header. The setting is stacked with the record, so the format in effect is always the
    /// innermost open record's, and it is restored when that record is popped.
    ///
    /// Two consequences. Reading: a reader that ignores this bit and assumes one form will read a
    /// length out of the first four characters of the text whenever it meets the other. Writing: the
    /// setting defaults to *simple* and must be turned on deliberately, so a writer that leaves it
    /// alone emits strings a length-prefixed reader mis-frames.
    pub strings_enhanced: bool,
}

impl Flags {
    pub(crate) fn decode(word: &[u8; 2]) -> Flags {
        Flags {
            len_kind: len_kind(word),
            has_schema: test_bit(word, BIT_HAS_SCHEMA),
            extended_value: test_bit(word, BIT_EXTENDED_VALUE),
            simple_encryption: test_bit(word, BIT_SIMPLE_ENCRYPTION),
            strings_enhanced: test_bit(word, BIT_STRINGS_ENHANCED),
        }
    }

    /// The string wire form this record's content is framed in.
    pub(crate) fn string_format(&self) -> StringFormat {
        if self.strings_enhanced {
            StringFormat::Enhanced
        } else {
            StringFormat::Simple
        }
    }
}

/// Clear the [`FLAG_BITS`] from an inline-value word, leaving the inline value.
pub(crate) fn clear_flag_bits(word: &mut [u8; 2]) {
    for &i in &FLAG_BITS {
        word[i >> 3] &= !BITMASKS[i & 7];
    }
}

/// Decode a big-endian scalar held in `bytes` (length 1, 2, or 4) to a `u64`.
pub(crate) fn be_scalar(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64)
}

/// What a record header states: its flags, its type, the version it declares, and the two lengths
/// that place the record on the stream.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HeaderShape {
    /// The decoded first word.
    pub flags: Flags,
    /// The record type, inline in the flag word or in the extended type word.
    pub rtype: u16,
    /// The version the header states, or `None` when it states none — the writing archive's
    /// default, which only the stream's dialect knows ([`super::dialect::Dialect::default_schema`]).
    pub schema: Option<u16>,
    /// Length of the content that follows the header. Zero for a record framed with no length
    /// field at all.
    pub content_len: usize,
    /// Length of the header itself: flag word, optional type word, optional schema, length field.
    pub header_len: usize,
}

/// Decode the record header at `at`, demasking every byte with `mask`.
///
/// This is the framing decode and nothing else: it reads what the header says and reports it, with
/// no judgement about whether a header belongs at this offset. A reader that probes for headers
/// rather than being told where they are supplies that judgement itself.
///
/// `None` when the header runs past the end of `d`.
pub(crate) fn decode_header(d: &[u8], at: usize, mask: u8) -> Option<HeaderShape> {
    let byte = |i: usize| -> Option<u8> { d.get(at + i).map(|b| b ^ mask) };

    let mut fw = [byte(0)?, byte(1)?];
    let flags = Flags::decode(&fw);
    let mut q = 2usize;

    // The type is either a separate little-endian word or packed into the flag word, whose first
    // byte is then the type's high byte.
    let rtype = if flags.extended_value {
        let v = u16::from_le_bytes([byte(q)?, byte(q + 1)?]);
        q += 2;
        v
    } else {
        clear_flag_bits(&mut fw);
        (u16::from(fw[0]) << 8) | u16::from(fw[1])
    };

    // The schema word is big-endian, and opaque: a version number, not a pair of halves. It is
    // written only when it differs from the default the writing archive was opened at.
    let schema = if flags.has_schema {
        let s = (u16::from(byte(q)?) << 8) | u16::from(byte(q + 1)?);
        q += 2;
        Some(s)
    } else {
        None
    };

    // The length is big-endian in whichever of the four widths the flag word names.
    let content_len = if flags.len_kind != 0 {
        let n = flags.len_kind as usize;
        let mut bytes = [0u8; 4];
        for (k, slot) in bytes[..n].iter_mut().enumerate() {
            *slot = byte(q + k)?;
        }
        q += n;
        be_scalar(&bytes[..n]) as usize
    } else {
        0
    };

    Some(HeaderShape {
        flags,
        rtype,
        schema,
        content_len,
        header_len: q,
    })
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

    /// Bit 4 selects the string wire form, and it is the only bit that does: the same header with
    /// that one bit flipped names the other form and changes nothing else.
    #[test]
    fn bit_four_selects_the_string_wire_form() {
        let enhanced = Flags::decode(&[0b1111_1000, 0x29]);
        let simple = Flags::decode(&[0b1110_1000, 0x29]);
        assert_eq!(enhanced.string_format(), StringFormat::Enhanced);
        assert_eq!(simple.string_format(), StringFormat::Simple);
        assert_eq!(enhanced.len_kind, simple.len_kind);
        assert_eq!(enhanced.has_schema, simple.has_schema);
        assert_eq!(enhanced.extended_value, simple.extended_value);
        assert_eq!(enhanced.simple_encryption, simple.simple_encryption);
    }

    #[test]
    fn clear_flag_bits_keeps_low_value_bits() {
        // bit2 set (flag) + low value bits 0b11 -> after clearing flags, value 0b11.
        let mut w = [0b0000_0111u8, 0x00];
        clear_flag_bits(&mut w);
        assert_eq!(w, [0b0000_0011, 0x00]);
    }
}
