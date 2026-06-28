//! L1 — the record tiler.
//!
//! The `Contents` stream is **not** a flat record stream. It is:
//!
//! ```text
//! Contents = [ stream header (type 0xffff, ~34 B) ] [ obfuscated+deflated payload ]
//! payload  --de-obfuscate--> deflate stream --inflate--> logical report bytes
//! logical report bytes --tile()--> a FLAT sequence of TSLV records
//! ```
//!
//! The TSLV records live in the **inflated** report, not in the compressed `Contents`. This
//! module tiles those logical bytes:
//!
//! - Each record's **header is read at mask 0** (raw), and its **content at mask
//!   `rtype & 0xff`**. The mask does not chain across records; it is per-record.
//! - The **inline type** is the cleared flag word read **big-endian** (`f8 64` → `0x0064`).
//! - The length is **big-endian**, `len_kind` bytes.
//! - Records are **flat**: content is `length` bytes; the next record follows immediately.

use super::tslv::{self, Flags};

/// A record delimited by [`tile`]: its type/subtype and byte spans within the logical stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TiledRecord {
    pub rtype: u16,
    pub subtype: Option<u16>,
    /// Offset of the record (its header) within the logical stream.
    pub offset: usize,
    /// Length of the header (flag word + optional type/subtype + length field).
    pub header_len: usize,
    /// Length of the content (the masked field data).
    pub content_len: usize,
}

impl TiledRecord {
    /// Total on-stream length of the record (header + content).
    pub fn len(&self) -> usize {
        self.header_len + self.content_len
    }
}

/// The outcome of tiling a logical stream.
#[derive(Debug, Clone)]
pub(crate) struct TileResult {
    pub records: Vec<TiledRecord>,
    /// True if the records tiled the whole stream exactly.
    pub complete: bool,
}

/// Read a big-endian scalar of `n` bytes at `pos` (header bytes are at mask 0, i.e. raw).
/// Used for the length field.
fn read_be(d: &[u8], pos: usize, n: usize) -> Option<u64> {
    d.get(pos..pos + n).map(tslv::be_scalar)
}

/// Read a 2-byte **little-endian** word (the extended type/subtype encoding — first byte low).
fn read_le16(d: &[u8], pos: usize) -> Option<u16> {
    d.get(pos..pos + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}

/// Tile a logical (inflated, demasked-header) report stream into a flat record sequence.
///
/// Never panics: a stream that desyncs returns the clean prefix with `complete == false`.
pub(crate) fn tile(d: &[u8]) -> TileResult {
    let mut records = Vec::new();
    let mut pos = 0usize;

    while pos < d.len() {
        let start = pos;

        // Flag word (2 bytes, mask 0).
        let Some(fw_slice) = d.get(pos..pos + 2) else {
            break;
        };
        let mut fw = [fw_slice[0], fw_slice[1]];
        let flags = Flags::decode(&fw);
        pos += 2;

        // Type: extended (2-byte little-endian word) or inline (cleared flag word, where the
        // first byte is the high byte).
        let rtype = if flags.extended_value {
            let Some(v) = read_le16(d, pos) else { break };
            pos += 2;
            v
        } else {
            tslv::clear_flag_bits(&mut fw);
            ((fw[0] as u16) << 8) | fw[1] as u16
        };

        // Optional subtype (bit 5), little-endian.
        let subtype = if flags.extended_type {
            let Some(st) = read_le16(d, pos) else { break };
            pos += 2;
            Some(st)
        } else {
            None
        };

        // Length: big-endian, len_kind bytes.
        let length = if flags.len_kind != 0 {
            let Some(l) = read_be(d, pos, flags.len_kind as usize) else {
                break;
            };
            pos += flags.len_kind as usize;
            l as usize
        } else {
            0
        };

        let header_len = pos - start;
        // A content length that overshoots the stream marks the end of the clean prefix.
        if pos + length > d.len() {
            pos = start;
            break;
        }
        pos += length;

        records.push(TiledRecord {
            rtype,
            subtype,
            offset: start,
            header_len,
            content_len: length,
        });
    }

    let complete = pos == d.len();
    TileResult { records, complete }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_a_single_inline_record() {
        // A len_kind=1 inline-type header (flag bit6 set) followed by 3 content bytes.
        let stream = [
            0x40u8,
            0x05, // flag: bit6 -> len_kind 1, inline; cleared low byte 0x00,hi 0x05 -> type 0x0005
            0x03, // length = 3
            0xaa, 0xbb, 0xcc, // content (3 bytes)
        ];
        let r = tile(&stream);
        assert!(r.complete, "should consume the whole stream");
        assert_eq!(r.records.len(), 1);
        let rec = &r.records[0];
        assert_eq!(rec.rtype, 0x0005);
        assert_eq!(rec.content_len, 3);
        assert_eq!(rec.len(), 6); // 3-byte header + 3-byte content
    }

    #[test]
    fn desync_returns_clean_prefix() {
        // One valid record, then a header whose length overshoots.
        let stream = [
            0x40u8, 0x05, 0x01, 0x11, // record: type 5, len 1, 1 content byte
            0x40, 0x00, 0x7f, // header claiming 127 content bytes but none present
        ];
        let r = tile(&stream);
        assert!(!r.complete);
        assert_eq!(r.records.len(), 1);
        assert_eq!(r.records[0].len(), 4);
    }
}
