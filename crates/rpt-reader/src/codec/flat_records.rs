//! Splitting a logical report stream into its flat sequence of records.
//!
//! The `Contents` stream is **not** a flat record stream. It is:
//!
//! ```text
//! Contents = [ stream header (type 0xffff, ~34 B) ] [ obfuscated+deflated payload ]
//! payload  --de-obfuscate--> deflate stream --inflate--> logical report bytes
//! logical report bytes --split_records()--> a FLAT sequence of TSLV records
//! ```
//!
//! The TSLV records live in the **inflated** report, not in the compressed `Contents`. This
//! module walks those logical bytes:
//!
//! - Each record's **header is read at mask 0** (raw), and its **content at mask
//!   `rtype & 0xff`**. The mask does not chain across records; it is per-record.
//! - Records are **flat**: content is `length` bytes; the next record follows immediately.
//!
//! The header's own bit-packing is decoded in [`super::tslv`], which this walk reads through
//! rather than repeating.

use super::tslv;

/// A record delimited by [`split_records`]: its type/schema and byte spans within the logical
/// stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FlatRecord {
    pub rtype: u16,
    pub schema: Option<u16>,
    /// Offset of the record (its header) within the logical stream.
    pub offset: usize,
    /// Length of the header (flag word + optional type word + schema + length field).
    pub header_len: usize,
    /// Length of the content (the masked field data).
    pub content_len: usize,
}

impl FlatRecord {
    /// Total on-stream length of the record (header + content).
    pub fn len(&self) -> usize {
        self.header_len + self.content_len
    }
}

/// The outcome of splitting a logical stream.
#[derive(Debug, Clone)]
pub(crate) struct SplitResult {
    pub records: Vec<FlatRecord>,
    /// True if the records covered the whole stream exactly.
    pub complete: bool,
}

/// Split a logical (inflated, demasked-header) report stream into a flat record sequence.
///
/// The records are flat, so every offset in the stream is a header offset and the framing decode
/// ([`tslv::decode_header`]) is the whole reading — this walk adds only where the sequence stops.
///
/// Never panics: a stream that desyncs returns the clean prefix with `complete == false`.
pub(crate) fn split_records(d: &[u8]) -> SplitResult {
    let mut records = Vec::new();
    let mut pos = 0usize;

    while pos < d.len() {
        // Headers are read raw here (mask 0); the content is masked and is not read at all.
        let Some(h) = tslv::decode_header(d, pos, 0) else {
            break;
        };
        // A header or content length that overshoots the stream ends the clean prefix.
        let Some(end) = pos
            .checked_add(h.header_len)
            .and_then(|p| p.checked_add(h.content_len))
            .filter(|end| *end <= d.len())
        else {
            break;
        };

        records.push(FlatRecord {
            rtype: h.rtype,
            schema: h.schema,
            offset: pos,
            header_len: h.header_len,
            content_len: h.content_len,
        });
        pos = end;
    }

    let complete = pos == d.len();
    SplitResult { records, complete }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_single_inline_record() {
        // A len_kind=1 inline-type header (flag bit6 set) followed by 3 content bytes.
        let stream = [
            0x40u8,
            0x05, // flag: bit6 -> len_kind 1, inline; cleared low byte 0x00,hi 0x05 -> type 0x0005
            0x03, // length = 3
            0xaa, 0xbb, 0xcc, // content (3 bytes)
        ];
        let r = split_records(&stream);
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
        let r = split_records(&stream);
        assert!(!r.complete);
        assert_eq!(r.records.len(), 1);
        assert_eq!(r.records[0].len(), 4);
    }
}
