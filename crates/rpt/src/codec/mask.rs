//! L2 — the running XOR mask.
//!
//! Every logical byte read by `load_block` is XORed with a 1-byte mask. The mask is
//! **stateful**: after each record header it updates as `mask ^= (recordType & 0xFF)`.
//! Because the update depends only on the *record-type sequence* — not on lengths, offsets,
//! or content — demasking and remasking are symmetric. `apply` is its own inverse for a
//! fixed mask value.

/// The running XOR mask state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Mask(u8);

impl Mask {
    /// Initial mask value (`0`).
    pub(crate) const INITIAL: Mask = Mask(0);

    /// The current mask byte.
    #[allow(dead_code)] // used by tests and the write (remask) path
    pub(crate) fn value(self) -> u8 {
        self.0
    }

    /// XOR `buf` in place with the current mask (demask on read / remask on write —
    /// identical operation, since XOR is an involution for a fixed mask).
    pub(crate) fn apply(self, buf: &mut [u8]) {
        for b in buf.iter_mut() {
            *b ^= self.0;
        }
    }

    /// Advance the mask by a record type (`mask ^= recordType & 0xFF`).
    pub(crate) fn advance(&mut self, record_type: u16) {
        self.0 ^= (record_type & 0xFF) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_is_self_inverse() {
        let m = Mask(0x64);
        let original = [0x00u8, 0x01, 0xff, 0x80, 0x64];
        let mut buf = original;
        m.apply(&mut buf);
        assert_ne!(buf, original, "mask should change the bytes");
        m.apply(&mut buf);
        assert_eq!(buf, original, "applying twice restores the original");
    }

    #[test]
    fn advance_matches_chained_xor() {
        // mask 0 -> after type 0xffff -> 0xff -> after type 0x019b -> 0xff ^ 0x9b = 0x64
        let mut m = Mask::INITIAL;
        assert_eq!(m.value(), 0x00);
        m.advance(0xffff);
        assert_eq!(m.value(), 0xff);
        m.advance(0x019b);
        assert_eq!(m.value(), 0x64);
    }
}
