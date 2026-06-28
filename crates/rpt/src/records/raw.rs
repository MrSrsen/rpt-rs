//! A record's identity plus its exact on-disk byte span.
//!
//! A [`RawRecord`] records *where* a record lives in the stream (`origin`) and its
//! [`RecordTag`], so the substrate can re-emit it verbatim while still exposing its identity
//! for inspection.

use super::tag::RecordTag;

/// The on-disk location of a record within its stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Origin {
    /// Byte offset of the record (its header) within the decoded stream.
    pub offset: usize,
    /// Total on-disk byte length of the record (header + content).
    pub len: usize,
}

/// A record located in a stream: its type and its verbatim on-disk span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRecord {
    /// The record type.
    pub tag: RecordTag,
    /// Where the record's bytes live in the original stream.
    pub origin: Origin,
}

impl RawRecord {
    /// Borrow this record's exact on-disk bytes out of its owning stream's buffer.
    pub fn bytes_in<'a>(&self, stream: &'a [u8]) -> &'a [u8] {
        &stream[self.origin.offset..self.origin.offset + self.origin.len]
    }
}
