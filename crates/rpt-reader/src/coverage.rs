//! How completely a report decoded — the observability the error type deliberately lacks.
//!
//! Building the model is infallible by design (see [`crate::error`]): a record it cannot interpret
//! becomes a default rather than an error, so a report the Crystal engine opens is never refused over
//! a record this reader does not model. The cost is that an *incomplete* decode is indistinguishable
//! from a complete one — and if the unrecognized record carried a format, a field, or an object, the
//! export or render is silently missing content while looking authoritative.
//!
//! This module makes the gap visible along the two axes a decode can fall short on: per stream,
//! how much of the record walk was understood ([`StreamCoverage`]); and for the saved-data path,
//! which spans three streams and can yield nothing for half a dozen different reasons, why it
//! yielded what it did ([`SavedDataStatus`]).
//!
//! A stream holds records two ways, and the per-stream figures deliberately measure both. The
//! **linear walk** is the stream's outermost records, each one's content spanning the records nested
//! inside it; it is the only walk whose spans partition the stream, so it is the only one "bytes
//! belonging to no record" can be asked of. The **record tree** is every record at every depth; it
//! is the only population "did every record type here get a name?" can be asked of, since a type
//! that only ever occurs nested is invisible to the linear walk. Asking either question of the other
//! walk answers nothing: a tree walk's spans nest, so subtracting them double-counts, and a linear
//! walk sees a fraction of the records: many of the corpus' report-definition records sit nested
//! inside another and so never appear in it.
//!
//! So [`StreamCoverage::uncovered_bytes`] is the linear walk's, and
//! [`StreamCoverage::unknown_records`] the tree's, and each field says which.

use std::fmt;

use crate::model::SavedBatchKind;
use crate::records::{RecordStream, RecordTag};

/// How completely every stream in a report decoded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct DecodeCoverage {
    /// Per-stream figures, in the report's stream order.
    pub streams: Vec<StreamCoverage>,
    /// What the saved-data path made of the report's stored rows.
    pub saved_data: SavedDataStatus,
}

/// How completely one stream decoded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct StreamCoverage {
    /// The stream these figures describe.
    pub stream: String,
    /// The stream's **outermost** records — its linear walk, in which a record's content spans the
    /// records nested inside it. Fewer than the record tree holds; the denominator for
    /// [`uncovered_bytes`](Self::uncovered_bytes), which is the only question this walk answers.
    pub outermost_records: usize,
    /// Records in the stream's **record tree**: every record at every depth. The denominator for
    /// [`unknown_records`](Self::unknown_records).
    ///
    /// Zero for a stream with no linear record walk — see
    /// [`unknown_records`](Self::unknown_records) for why such a stream is not censused at all.
    pub tree_records: usize,
    /// Records **anywhere in the tree** whose record type the registry does not recognize.
    ///
    /// The tree, not the linear walk, because the question is whether every record type in the
    /// stream got a name, and a type that only ever occurs nested never reaches the linear walk —
    /// the decode would report complete while a record type in the file had no name.
    ///
    /// A stream with no linear record walk is not censused: its tree is found by scanning rather
    /// than declared, so a number it yields is as likely to be field data that happened to frame as
    /// a record, and an unnamed one is no evidence of an unnamed record type. That is the
    /// `DataSourceManager` saved-data catalog, which is read through the saved-data path and has
    /// never been in this meter.
    pub unknown_records: usize,
    /// The distinct unrecognized record types, ascending — what to go and decode next. Over the
    /// same population as [`unknown_records`](Self::unknown_records).
    pub unknown_types: Vec<u16>,
    /// Logical (decrypted + inflated) bytes belonging to no record of the **linear walk**. Nonzero
    /// means that walk did not partition the stream, so there is structure here the framing did not
    /// reach.
    ///
    /// The linear walk is what this can be asked of: its records' spans lie side by side, so what is
    /// left over is what it missed. The tree's spans nest — a parent's span holds its children's —
    /// so subtracting them would count the same byte once per level.
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
    /// Read one stream's figures off its decoded records — the linear walk for the byte account,
    /// the record tree for the type census.
    pub(crate) fn of(stream: &RecordStream) -> StreamCoverage {
        let logical_bytes = stream.logical_bytes().len();
        let covered: usize = stream.records().iter().map(|r| r.origin().len).sum();
        // A stream with no linear record walk answers neither question: it has no side-by-side
        // spans to account for its bytes, and its tree is scanned rather than framed, so an unnamed
        // number in it is not evidence of an unnamed record type.
        let framed = !stream.records().is_empty();
        let uncovered_bytes = if framed {
            logical_bytes.saturating_sub(covered)
        } else {
            0
        };

        let dialect = stream.dialect();
        let mut tree_records = 0usize;
        let mut unknown_records = 0usize;
        let mut unknown_types: Vec<u16> = Vec::new();
        if framed {
            for root in stream.record_tree() {
                root.walk(&mut |node| {
                    tree_records += 1;
                    if !RecordTag(node.rtype).is_known(dialect) {
                        unknown_records += 1;
                        unknown_types.push(node.rtype);
                    }
                });
            }
        }
        unknown_types.sort_unstable();
        unknown_types.dedup();

        StreamCoverage {
            stream: format!("{:?}", stream.id()),
            outermost_records: stream.len(),
            tree_records,
            unknown_records,
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

/// What stopped one saved-data batch from decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub enum BatchProblem {
    /// The directory names a batch that lies past the end of the stream carrying it.
    Absent,
    /// The batch's first block did not decrypt to a zlib header under the initialization vector
    /// derived from its directory entry. The ciphertext is there; the metadata keying the cipher is
    /// wrong — which is what a misread record width or a lost directory entry looks like from here.
    NotDecrypted,
    /// The batch decrypted but its plaintext is not an inflatable zlib stream.
    NotInflated,
    /// The batch decoded to fewer bytes than its own row count and width call for.
    Short,
}

impl fmt::Display for BatchProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BatchProblem::Absent => "lies past the end of its stream",
            BatchProblem::NotDecrypted => "did not decrypt to a zlib stream under its derived IV",
            BatchProblem::NotInflated => "decrypted but would not inflate",
            BatchProblem::Short => "inflated to fewer bytes than its row count and width call for",
        })
    }
}

/// What the saved-data path made of a report's stored rows.
///
/// A report's saved data lives across three streams — the `DataSourceManager` catalog, the
/// `SavedRecordsStream` batches, and the `MemoValuesStream` heaps — and the decode gives up at
/// whichever of them first fails to make sense. A report that stores rows behind a batch whose
/// cipher key is wrong is otherwise indistinguishable from one saved without data: both leave
/// `rows` empty, so a decoder bug can hide behind a legitimate outcome unless the reason is named.
///
/// The reason is recorded by the reader that gave up, not reconstructed afterwards, so it cannot
/// disagree with what actually happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase", tag = "status"))]
pub enum SavedDataStatus {
    /// The file carries no saved-data catalog: no `DataSourceManager` payload to read one from.
    /// This is a report saved without its data.
    #[default]
    NoCatalog,
    /// A catalog is present but names no stored field, so no row has a shape to decode into. A
    /// template saved with the data flag set and nothing behind it reports this.
    NoStoredFields,
    /// The catalog names stored fields but the `SavedRecordsStream` holding their rows is absent, so
    /// the file describes a rowset it does not carry.
    MissingRowStream,
    /// The batch directory lists no record-index batch, so there is no row index to read.
    NoRecordBatches,
    /// Every batch of the directory is a memo-value heap. A rowset made only of variable-length
    /// values has no fixed record index, and holds no rows by construction.
    MemoValuesOnly,
    /// A batch would not decode, so the rows it holds are lost. The decode stops at the first such
    /// batch, since the batches of one kind sit back to back and the next one's offset is this
    /// one's consumed length.
    BatchUndecodable {
        /// Which of the three batch classes failed.
        kind: SavedBatchKind,
        /// Its 0-based sequence within that class.
        index: u32,
        /// What stopped it.
        problem: BatchProblem,
    },
    /// The batches decoded but yielded no row.
    NoRows,
    /// Rows decoded. `rows` short of `stored` means a batch was lost partway through the walk.
    Decoded {
        /// Rows the decode produced.
        rows: usize,
        /// Rows the batch directory claims the report stores.
        stored: u32,
    },
}

impl SavedDataStatus {
    /// Whether the saved-data path read everything the file says it holds — including the outcomes
    /// where a report genuinely stores no rows.
    ///
    /// The distinction this draws is the module's point: `NoCatalog` and `Decoded` in full are both
    /// complete, and a batch that would not decrypt is not, however plausibly its silence resembles
    /// an empty report.
    pub fn is_complete(&self) -> bool {
        match self {
            SavedDataStatus::NoCatalog
            | SavedDataStatus::NoStoredFields
            | SavedDataStatus::NoRecordBatches
            | SavedDataStatus::MemoValuesOnly => true,
            SavedDataStatus::Decoded { rows, stored } => *rows as u64 >= *stored as u64,
            SavedDataStatus::MissingRowStream
            | SavedDataStatus::BatchUndecodable { .. }
            | SavedDataStatus::NoRows => false,
        }
    }

    /// A one-line account of what the saved-data path lost, or `None` when it lost nothing — the
    /// half of [`Display`](fmt::Display) a caller should surface as a warning.
    pub fn shortfall(&self) -> Option<String> {
        (!self.is_complete()).then(|| self.to_string())
    }
}

/// The batch classes as a reader's message names them.
fn batch_kind_name(kind: SavedBatchKind) -> &'static str {
    match kind {
        SavedBatchKind::Index => "record-index",
        SavedBatchKind::Descriptor => "memo-descriptor",
        SavedBatchKind::MemoValue => "memo-value",
    }
}

impl fmt::Display for SavedDataStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SavedDataStatus::NoCatalog => f.write_str("no saved data"),
            SavedDataStatus::NoStoredFields => {
                f.write_str("a saved-data catalog naming no stored field")
            }
            SavedDataStatus::MissingRowStream => f.write_str(
                "the saved-data catalog names stored fields but the stream holding their rows is \
                 absent",
            ),
            SavedDataStatus::NoRecordBatches => {
                f.write_str("a saved-data catalog listing no record batch")
            }
            SavedDataStatus::MemoValuesOnly => {
                f.write_str("saved memo values only, with no record index")
            }
            SavedDataStatus::BatchUndecodable {
                kind,
                index,
                problem,
            } => write!(
                f,
                "saved-data {} batch #{index} {problem}",
                batch_kind_name(*kind)
            ),
            SavedDataStatus::NoRows => {
                f.write_str("the saved-data batches decoded but yielded no row")
            }
            SavedDataStatus::Decoded { rows, stored } if self.is_complete() => {
                write!(f, "{rows} saved row(s), all the report stores")
            }
            SavedDataStatus::Decoded { rows, stored } => write!(
                f,
                "only {rows} of the {stored} saved row(s) the report stores decoded"
            ),
        }
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

    /// Whether every stream decoded completely and the saved-data path read every row the file
    /// claims — the one-line test an exporter or renderer needs before presenting its output as the
    /// whole report.
    pub fn is_complete(&self) -> bool {
        self.streams.iter().all(StreamCoverage::is_complete) && self.saved_data.is_complete()
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
        parts.extend(self.saved_data.shortfall());
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
            outermost_records: 100,
            tree_records: 170,
            unknown_records,
            unknown_types,
            uncovered_bytes,
            logical_bytes: 1000,
            decode_error: None,
        }
    }

    /// A record type that only ever occurs **nested** still raises the warning. The linear walk
    /// steps over it — its parent's content spans it — so a meter read off that walk reports the
    /// decode complete while a record type in the file has no name.
    #[test]
    fn an_unnamed_type_is_counted_wherever_it_is_nested() {
        // `ReportAreaPair(0x0082)` with an unnamed `0x0199` nested in its content, under the stack
        // mask its type sets.
        let mask = 0x82u8;
        let child: Vec<u8> = [0xf9u8, 0x99, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00]
            .iter()
            .map(|b| b ^ mask)
            .collect();
        let mut logical = vec![0xf8u8, 0x82, 0x07, 0x00, 0x00, 0x00, 0x00, 0x08];
        logical.extend(child);

        let stream = RecordStream::from_logical_bytes(crate::StreamId::Contents, &logical);
        assert_eq!(stream.unknown_count(), 0, "the outermost record is named");

        let c = StreamCoverage::of(&stream);
        assert_eq!(c.outermost_records, 1);
        assert_eq!(c.tree_records, 2);
        assert_eq!(c.unknown_records, 1);
        assert_eq!(c.unknown_types, [0x0199]);
        assert_eq!(c.uncovered_bytes, 0, "the linear walk still partitions it");
        assert!(!c.is_complete());
    }

    #[test]
    fn a_complete_decode_warns_about_nothing() {
        let c = DecodeCoverage {
            streams: vec![stream(0, vec![], 0)],
            ..DecodeCoverage::default()
        };
        assert!(c.is_complete());
        assert_eq!(c.warning(), None);
    }

    #[test]
    fn an_unrecognized_record_names_its_type_and_the_next_step() {
        let c = DecodeCoverage {
            streams: vec![stream(1, vec![0x0199], 0)],
            ..DecodeCoverage::default()
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
            ..DecodeCoverage::default()
        };
        let w = c.warning().expect("warn");
        assert!(w.contains("3 record(s)"), "{w}");
        assert!(w.contains("types 0x0020, 0x0030"), "{w}");
        assert!(w.contains("8 byte(s) belong to no decoded record"), "{w}");
    }

    #[test]
    fn a_lost_saved_batch_reaches_the_warning_and_an_empty_report_does_not() {
        let complete = DecodeCoverage {
            streams: vec![stream(0, vec![], 0)],
            saved_data: SavedDataStatus::NoCatalog,
        };
        assert_eq!(complete.warning(), None);

        let lost = DecodeCoverage {
            saved_data: SavedDataStatus::BatchUndecodable {
                kind: SavedBatchKind::Index,
                index: 2,
                problem: BatchProblem::NotDecrypted,
            },
            ..complete
        };
        assert!(!lost.is_complete());
        let w = lost.warning().expect("a lost batch must warn");
        assert!(w.contains("record-index batch #2"), "{w}");
        assert!(w.contains("did not decrypt"), "{w}");
    }

    #[test]
    fn a_partly_decoded_rowset_is_not_complete() {
        // Rows that decoded do not mean every row decoded, and the count the file claims is the
        // only thing that says so.
        let full = SavedDataStatus::Decoded {
            rows: 800,
            stored: 800,
        };
        assert!(full.is_complete());
        assert_eq!(full.shortfall(), None);

        let short = SavedDataStatus::Decoded {
            rows: 540,
            stored: 800,
        };
        assert!(!short.is_complete());
        assert!(short
            .shortfall()
            .is_some_and(|s| s.contains("only 540 of the 800")));
    }

    #[test]
    fn an_undecodable_stream_is_reported_even_with_nothing_unrecognized() {
        let mut s = stream(0, vec![], 0);
        s.decode_error = Some("inflate failed".to_string());
        let c = DecodeCoverage {
            streams: vec![s],
            ..DecodeCoverage::default()
        };
        assert!(!c.is_complete());
        let w = c.warning().expect("warn");
        assert!(w.contains("could not be decoded (inflate failed)"), "{w}");
    }
}
