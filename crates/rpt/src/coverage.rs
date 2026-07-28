//! How completely a report decoded — the observability the error type deliberately lacks.
//!
//! The `raise` layer is infallible by design (see [`crate::error`]): a record it cannot interpret
//! becomes a default rather than an error, so a report the Crystal engine opens is never refused over
//! a record this reader does not model. The cost is that an *incomplete* decode is indistinguishable
//! from a complete one — and if the unrecognized record carried a format, a field, or an object, the
//! export or render is silently missing content while looking authoritative.
//!
//! This module makes the gap visible. Every figure is read off the already-decoded substrate (record
//! tags and their byte spans), not recomputed from the bytes.

use crate::records::{Record, RecordStream};

/// How completely every stream in a report decoded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct DecodeCoverage {
    /// Per-stream figures, in the report's stream order.
    pub streams: Vec<StreamCoverage>,
}

/// How completely one stream decoded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct StreamCoverage {
    /// The stream these figures describe.
    pub stream: String,
    /// Records decoded from the stream.
    pub records: usize,
    /// Of those, how many had a record type the registry does not recognize.
    pub unknown_records: usize,
    /// The distinct unrecognized record types, ascending — what to go and decode next.
    pub unknown_types: Vec<u16>,
    /// Logical (decrypted + inflated) bytes belonging to no decoded record. Nonzero means the record
    /// walk did not partition the stream, so there is structure here the framing did not reach.
    ///
    /// Reported only for streams that *have* a flat record walk. A stream decoded by another route —
    /// `DataSourceManager` carries QE-dialect records read through the saved-data path, not a TSLV
    /// record list — legitimately has no records covering its bytes, and counting that as a gap would
    /// make every report warn.
    pub uncovered_bytes: usize,
    /// Logical bytes in the stream, as the denominator for `uncovered_bytes`.
    pub logical_bytes: usize,
    /// Why the stream's payload could not be decoded at all, when it could not be. Distinguishes an
    /// unreadable stream from a genuinely empty one.
    pub decode_error: Option<String>,
}

impl StreamCoverage {
    /// Read one stream's figures off its already-decoded substrate.
    pub(crate) fn of(stream: &RecordStream) -> StreamCoverage {
        let mut unknown_types: Vec<u16> = stream
            .records()
            .iter()
            .filter(|r| matches!(r, Record::Unknown(_)))
            .map(|r| r.tag().value())
            .collect();
        unknown_types.sort_unstable();
        unknown_types.dedup();

        let logical_bytes = stream.logical_bytes().len();
        let covered: usize = stream.records().iter().map(|r| r.origin().len).sum();
        // Only a stream that was actually walked as records can have bytes *missed* by that walk.
        let uncovered_bytes = if stream.records().is_empty() {
            0
        } else {
            logical_bytes.saturating_sub(covered)
        };
        StreamCoverage {
            stream: format!("{:?}", stream.id()),
            records: stream.len(),
            unknown_records: stream.unknown_count(),
            unknown_types,
            uncovered_bytes,
            logical_bytes,
            decode_error: stream.decode_error().map(str::to_string),
        }
    }

    /// Whether this stream decoded with nothing unrecognized and nothing left over.
    pub fn is_complete(&self) -> bool {
        self.unknown_records == 0 && self.uncovered_bytes == 0 && self.decode_error.is_none()
    }
}

impl DecodeCoverage {
    /// Unrecognized records across every stream.
    pub fn unknown_records(&self) -> usize {
        self.streams.iter().map(|s| s.unknown_records).sum()
    }

    /// Logical bytes belonging to no decoded record, across every stream.
    pub fn uncovered_bytes(&self) -> usize {
        self.streams.iter().map(|s| s.uncovered_bytes).sum()
    }

    /// Whether every stream decoded completely — the one-line test an exporter or renderer needs
    /// before presenting its output as the whole report.
    pub fn is_complete(&self) -> bool {
        self.streams.iter().all(StreamCoverage::is_complete)
    }

    /// A one-line warning for a caller to show the user, or `None` when the decode was complete.
    ///
    /// Names the counts, the unrecognized record types, and the command that breaks the figures down,
    /// so the user can tell an incomplete export from a complete one without knowing to go looking.
    pub fn warning(&self) -> Option<String> {
        if self.is_complete() {
            return None;
        }
        let mut parts = Vec::new();
        let unknown = self.unknown_records();
        if unknown > 0 {
            let mut types: Vec<u16> = self
                .streams
                .iter()
                .flat_map(|s| s.unknown_types.iter().copied())
                .collect();
            types.sort_unstable();
            types.dedup();
            let listed = types
                .iter()
                .map(|t| format!("{t:#06x}"))
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!(
                "{unknown} record(s) were not recognized (type{} {listed})",
                if types.len() == 1 { "" } else { "s" }
            ));
        }
        let uncovered = self.uncovered_bytes();
        if uncovered > 0 {
            parts.push(format!("{uncovered} byte(s) belong to no decoded record"));
        }
        for s in &self.streams {
            if let Some(e) = &s.decode_error {
                parts.push(format!("stream `{}` could not be decoded ({e})", s.stream));
            }
        }
        Some(format!(
            "{}; some report content may be missing from this output. \
             Run `rpt streams <file>` for the coverage breakdown.",
            parts.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(
        unknown_records: usize,
        unknown_types: Vec<u16>,
        uncovered_bytes: usize,
    ) -> StreamCoverage {
        StreamCoverage {
            stream: "Contents".to_string(),
            records: 100,
            unknown_records,
            unknown_types,
            uncovered_bytes,
            logical_bytes: 1000,
            decode_error: None,
        }
    }

    #[test]
    fn a_complete_decode_warns_about_nothing() {
        let c = DecodeCoverage {
            streams: vec![stream(0, vec![], 0)],
        };
        assert!(c.is_complete());
        assert_eq!(c.warning(), None);
    }

    #[test]
    fn an_unrecognized_record_names_its_type_and_the_next_step() {
        let c = DecodeCoverage {
            streams: vec![stream(1, vec![0x0199], 0)],
        };
        assert!(!c.is_complete());
        let w = c.warning().expect("an incomplete decode must warn");
        assert!(w.contains("1 record(s) were not recognized"), "{w}");
        assert!(w.contains("type 0x0199"), "{w}");
        assert!(w.contains("rpt streams"), "{w}");
    }

    #[test]
    fn several_types_across_streams_are_merged_and_deduplicated() {
        let c = DecodeCoverage {
            streams: vec![stream(2, vec![0x30, 0x20], 0), stream(1, vec![0x20], 8)],
        };
        let w = c.warning().expect("warn");
        assert!(w.contains("3 record(s)"), "{w}");
        assert!(w.contains("types 0x0020, 0x0030"), "{w}");
        assert!(w.contains("8 byte(s) belong to no decoded record"), "{w}");
    }

    #[test]
    fn an_undecodable_stream_is_reported_even_with_nothing_unrecognized() {
        let mut s = stream(0, vec![], 0);
        s.decode_error = Some("inflate failed".to_string());
        let c = DecodeCoverage { streams: vec![s] };
        assert!(!c.is_complete());
        let w = c.warning().expect("warn");
        assert!(w.contains("could not be decoded (inflate failed)"), "{w}");
    }
}
