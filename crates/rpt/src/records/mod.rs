//! L1 data — the lossless record substrate.
//!
//! A [`RecordStream`] is one decoded TSLV stream. It always retains the original stream
//! bytes, so [`RecordStream::encode`] is byte-identical to the input regardless of how much
//! is semantically understood. On top of that it exposes:
//!
//! - the decoded [`StreamHeader`], and
//! - a flat list of [`Record`]s when the framing parses cleanly end-to-end, each marked
//!   [`Record::Known`] or [`Record::Unknown`] by whether its [`RecordTag`] is identified.

mod raw;
mod tag;

pub use raw::{Origin, RawRecord};
pub use tag::RecordTag;

use crate::codec::{self, RecordNode, StreamHeader};
use crate::container::StreamId;

/// One record in the substrate. Both arms carry the verbatim span; the distinction is
/// purely whether the record type has been identified yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// A record whose type is identified (has a name in the registry).
    Known(RawRecord),
    /// A record whose type is not yet identified — preserved verbatim, never dropped.
    Unknown(RawRecord),
}

impl Record {
    fn from_raw(rr: RawRecord) -> Record {
        if rr.tag.is_known() {
            Record::Known(rr)
        } else {
            Record::Unknown(rr)
        }
    }

    /// The record's type tag.
    pub fn tag(&self) -> RecordTag {
        match self {
            Record::Known(r) | Record::Unknown(r) => r.tag,
        }
    }

    /// The record's on-disk span.
    pub fn origin(&self) -> Origin {
        match self {
            Record::Known(r) | Record::Unknown(r) => r.origin,
        }
    }
}

fn tiled_to_record(t: &codec::TiledRecord) -> Record {
    Record::from_raw(RawRecord {
        tag: RecordTag(t.rtype),
        origin: Origin {
            offset: t.offset,
            len: t.len(),
        },
    })
}

/// One decoded TSLV stream: lossless bytes plus an as-far-as-understood structured view.
#[derive(Debug, Clone)]
pub struct RecordStream {
    id: StreamId,
    /// Original on-disk stream bytes — re-emitted verbatim for lossless round-trip.
    raw: Vec<u8>,
    /// The decoded **logical** report (decrypted + inflated). Record origins index into
    /// this. Empty when the stream's payload was not (yet) decoded.
    logical: Vec<u8>,
    header: Option<StreamHeader>,
    records: Vec<Record>,
    /// True if the cleanly-delimited record prefix consumed the whole stream exactly.
    fully_parsed: bool,
}

impl RecordStream {
    /// Decode a stream's raw bytes into the substrate.
    ///
    /// Never fails: a stream we cannot frame is still a valid (opaque) substrate entry that
    /// round-trips byte-identically. For a TSLV stream we decode the type-`0xffff` header
    /// (flags + IV); the report records themselves live in the stream's **inflated** payload,
    /// not in these compressed bytes (see [`crate::codec::tile`]).
    pub(crate) fn decode(id: StreamId, bytes: &[u8]) -> RecordStream {
        // `QESession` streams carry the `QENG` magic and use the Query-Engine cipher
        // (textbook AES-128-CFB, fixed key, IV in the QENG header) rather than the
        // `Contents` modified-Rijndael path. Route by the on-disk magic so subreport
        // `Subdocument N/QESession` streams (classified as `Other`) decode too.
        if codec::is_qe(bytes) {
            let (logical, records, fully_parsed) = match codec::decode_qe(bytes) {
                Ok(report) => {
                    let result = codec::tile(&report);
                    let recs = result.records.iter().map(tiled_to_record).collect();
                    (report, recs, result.complete)
                }
                Err(_) => (Vec::new(), Vec::new(), false),
            };
            return RecordStream {
                id,
                raw: bytes.to_vec(),
                logical,
                header: None,
                records,
                fully_parsed,
            };
        }

        if !id.is_tslv() {
            return RecordStream {
                id,
                raw: bytes.to_vec(),
                logical: Vec::new(),
                header: None,
                records: Vec::new(),
                fully_parsed: false,
            };
        }

        let header = codec::decode_stream_header(bytes).ok();
        // Full pipeline: decrypt + inflate the payload, then tile the logical report into
        // flat TSLV records. The substrate still retains the raw bytes for lossless
        // round-trip; the records are the decoded view over the logical report.
        let (logical, records, fully_parsed) = match codec::decode_contents(bytes) {
            Ok(report) => {
                let result = codec::tile(&report);
                let recs = result.records.iter().map(tiled_to_record).collect();
                (report, recs, result.complete)
            }
            Err(_) => (Vec::new(), Vec::new(), false),
        };

        RecordStream {
            id,
            raw: bytes.to_vec(),
            logical,
            header,
            records,
            fully_parsed,
        }
    }

    /// Tile an already-**logical** (inflated, deframed) report stream into flat TSLV records.
    ///
    /// Exposed so a caller that already holds an inflated report stream can tile it directly.
    pub fn tile_logical(id: StreamId, logical: &[u8]) -> RecordStream {
        let result = codec::tile(logical);
        let records = result.records.iter().map(tiled_to_record).collect();
        RecordStream {
            id,
            raw: logical.to_vec(),
            logical: logical.to_vec(),
            header: None,
            records,
            fully_parsed: result.complete,
        }
    }

    /// The decoded logical report bytes (decrypted + inflated). Record origins index into
    /// this slice. Empty when the payload was not decoded.
    pub fn logical_bytes(&self) -> &[u8] {
        &self.logical
    }

    /// Parse the logical report into the **recursive record tree** (nested records read under
    /// the stack-XOR content mask, see [`crate::codec`]). Empty when the payload was not
    /// decoded.
    pub fn record_tree(&self) -> Vec<RecordNode> {
        codec::parse_tree(&self.logical)
    }

    /// Parse the logical stream as a **`QESession`** record tree (the relaxed-subtype dialect).
    /// Use this for `QENG` streams; [`RecordStream::record_tree`] is for `Contents`.
    pub fn qe_record_tree(&self) -> Vec<RecordNode> {
        codec::parse_tree_qe(&self.logical)
    }

    /// Extract the printable ASCII strings (length ≥ `min_len`) from the demasked leaf bytes
    /// of every record in the tree — field names, formulas, table/SQL metadata, etc. Useful
    /// for inspection.
    pub fn strings(&self, min_len: usize) -> Vec<String> {
        let logical = &self.logical;
        let mut out = Vec::new();
        for root in self.record_tree() {
            root.walk(&mut |node| {
                let leaf = node.leaf_bytes(logical);
                let mut run = Vec::new();
                let flush = |run: &mut Vec<u8>, out: &mut Vec<String>| {
                    if run.len() >= min_len {
                        out.push(String::from_utf8_lossy(run).into_owned());
                    }
                    run.clear();
                };
                for &b in &leaf {
                    if (0x20..0x7f).contains(&b) {
                        run.push(b);
                    } else {
                        flush(&mut run, &mut out);
                    }
                }
                flush(&mut run, &mut out);
            });
        }
        out
    }

    /// Re-encode the substrate back to stream bytes. Re-emits the retained original framing,
    /// so `encode(decode(x)) == x` byte-for-byte.
    pub fn encode(&self) -> Vec<u8> {
        self.raw.clone()
    }

    /// The stream's symbolic id.
    pub fn id(&self) -> &StreamId {
        &self.id
    }

    /// The decoded stream header (type `0xffff`), if this is a TSLV stream.
    pub fn header(&self) -> Option<&StreamHeader> {
        self.header.as_ref()
    }

    /// The cleanly-delimited top-level record prefix. May be a prefix of the stream when
    /// [`RecordStream::is_fully_parsed`] is false; the remaining bytes are an opaque tail
    /// preserved verbatim for round-trip.
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Number of decoded records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True when there are no decoded records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Number of decoded records whose type is not yet identified.
    pub fn unknown_count(&self) -> usize {
        self.records
            .iter()
            .filter(|r| matches!(r, Record::Unknown(_)))
            .count()
    }

    /// Whether the flat record walk consumed the whole stream exactly.
    pub fn is_fully_parsed(&self) -> bool {
        self.fully_parsed
    }

    /// The original on-disk bytes of the stream.
    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw
    }
}
