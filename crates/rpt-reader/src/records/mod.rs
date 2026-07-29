//! One decoded stream as records — the lossless record layer.
//!
//! A [`RecordStream`] is one decoded TSLV stream. It always retains the original stream
//! bytes, so [`RecordStream::encode`] is byte-identical to the input regardless of how much
//! is semantically understood. On top of that it exposes:
//!
//! - the decoded [`StreamHeader`], and
//! - a flat list of [`Record`]s when the framing parses cleanly end-to-end, each marked
//!   [`Record::Known`] or [`Record::Unknown`] by whether its [`RecordTag`] is identified.

mod raw;
pub(crate) mod rtype;
mod tag;
mod typed_tree;

pub use raw::{Origin, RawRecord};
pub use tag::RecordTag;
pub use typed_tree::{Node, Part, RecordTypeCount, Unknown, Value};

use std::path::Path;

use crate::codec::{self, Dialect, RecordNode, StreamHeader};
use crate::container::StreamId;
use crate::field_table;

/// One record in a stream. Both arms carry the verbatim span; the distinction is
/// purely whether the record type has been identified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// A record whose type is identified (has a name in the registry).
    Known(RawRecord),
    /// A record whose type is not identified — preserved verbatim, never dropped.
    Unknown(RawRecord),
}

impl Record {
    fn from_raw(rr: RawRecord, dialect: Dialect) -> Record {
        if rr.tag.is_known(dialect) {
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

fn flat_to_record(t: &codec::FlatRecord, dialect: Dialect) -> Record {
    Record::from_raw(
        RawRecord {
            tag: RecordTag(t.rtype),
            origin: Origin {
                offset: t.offset,
                len: t.len(),
            },
        },
        dialect,
    )
}

/// The record vocabulary a stream's records are written in.
///
/// A subreport's own streams are kept by their full path rather than classified into the variants
/// their top-level counterparts get, so the name at the end of that path is what says which stream
/// each is: a `Subdocument N/DataSourceManager 5l` is a saved-data catalog wherever it sits.
/// Re-classifying that name is the whole rule, and it is what keeps the nested answer from drifting
/// away from the top-level one.
fn dialect_of(id: &StreamId) -> Dialect {
    if let StreamId::Other(path) = id {
        if let Some((_, name)) = path.rsplit_once('/') {
            return top_level_dialect(&StreamId::classify(Path::new(name)));
        }
    }
    top_level_dialect(id)
}

/// The vocabulary a top-level stream of this identity is written in.
///
/// A stream that is none of them takes the report definition's, which names nothing of it: such a
/// stream is neither TSLV-framed nor `QENG`-magic, so it decodes to no logical bytes and holds no
/// record for a vocabulary to name.
fn top_level_dialect(id: &StreamId) -> Dialect {
    match id {
        StreamId::QESession => Dialect::QeSession,
        StreamId::DataSourceManager(_) => Dialect::Catalog,
        StreamId::ReportParametersStream(_) => Dialect::ReportParameters,
        _ => Dialect::Contents,
    }
}

/// The printable-ASCII stretches of one contiguous run of field bytes, at least `min_len` bytes
/// long. Every other byte ends the stretch it follows.
fn printable_runs(run: &[u8], min_len: usize) -> impl Iterator<Item = String> + '_ {
    run.split(|b| !(0x20..0x7f).contains(b))
        .filter(move |text| text.len() >= min_len)
        .map(|text| String::from_utf8_lossy(text).into_owned())
}

/// One decoded TSLV stream: lossless bytes plus an as-far-as-understood structured view.
#[derive(Debug, Clone)]
pub struct RecordStream {
    id: StreamId,
    /// Original on-disk stream bytes — re-emitted verbatim for lossless round-trip.
    raw: Vec<u8>,
    /// The decoded **logical** report (decrypted + inflated). Record origins index into
    /// this. Empty when the stream's payload was not decoded.
    logical: Vec<u8>,
    header: Option<StreamHeader>,
    records: Vec<Record>,
    /// True if the cleanly-delimited record prefix consumed the whole stream exactly.
    fully_parsed: bool,
    /// Why the payload could not be decoded, when it could not be. Decoding is deliberately
    /// non-fatal — a stream we cannot read still round-trips byte-identically and the rest of the
    /// report stays inspectable — but the reason must not be lost, or an undecryptable report is
    /// indistinguishable from a genuinely empty one.
    decode_error: Option<String>,
}

impl RecordStream {
    /// Decode a stream's raw bytes into records.
    ///
    /// Never fails: a stream we cannot frame is still a valid (opaque) entry that round-trips
    /// byte-identically. For a TSLV stream we decode the type-`0xffff` header (flags + IV); the
    /// report records themselves live in the stream's **inflated** payload, not in these
    /// compressed bytes (see [`crate::codec::split_records`]).
    pub(crate) fn decode(id: StreamId, bytes: &[u8]) -> RecordStream {
        let dialect = dialect_of(&id);
        // `QESession` streams carry the `QENG` magic and use the Query-Engine cipher
        // (textbook AES-128-CFB, fixed key, IV in the QENG header) rather than the
        // `Contents` modified-Rijndael path. Route by the on-disk magic so subreport
        // `Subdocument N/QESession` streams (classified as `Other`) decode too.
        if codec::is_qe(bytes) {
            let mut decode_error = None;
            let (logical, records, fully_parsed) = match codec::decode_qe(bytes) {
                Ok(report) => {
                    let result = codec::split_records(&report);
                    let recs = result
                        .records
                        .iter()
                        .map(|r| flat_to_record(r, dialect))
                        .collect();
                    (report, recs, result.complete)
                }
                Err(e) => {
                    decode_error = Some(e.to_string());
                    (Vec::new(), Vec::new(), false)
                }
            };
            return RecordStream {
                id,
                raw: bytes.to_vec(),
                logical,
                header: None,
                records,
                fully_parsed,
                decode_error,
            };
        }

        if !id.is_tslv() {
            // The `DataSourceManager` (saved-data catalog) stream is encrypted with the
            // `Contents` cipher but carries QE-dialect records, so it does not split like a
            // `Contents` TSLV stream. Decode just its logical payload (decrypt + inflate) so
            // inspection can read its record tree via `record_tree`; the saved-data path
            // reads these same logical bytes.
            let logical = if matches!(id, StreamId::DataSourceManager(_)) {
                codec::decode_contents(bytes).unwrap_or_default()
            } else {
                Vec::new()
            };
            return RecordStream {
                id,
                raw: bytes.to_vec(),
                logical,
                header: None,
                records: Vec::new(),
                fully_parsed: false,
                decode_error: None,
            };
        }

        let header = codec::decode_stream_header(bytes).ok();
        // Full pipeline: decrypt + inflate the payload, then split the logical report into
        // flat TSLV records. The stream still retains the raw bytes for lossless
        // round-trip; the records are the decoded view over the logical report.
        let mut decode_error = None;
        let (logical, records, fully_parsed) = match codec::decode_contents(bytes) {
            Ok(report) => {
                let result = codec::split_records(&report);
                let recs = result
                    .records
                    .iter()
                    .map(|r| flat_to_record(r, dialect))
                    .collect();
                (report, recs, result.complete)
            }
            Err(e) => {
                decode_error = Some(e.to_string());
                (Vec::new(), Vec::new(), false)
            }
        };

        RecordStream {
            id,
            raw: bytes.to_vec(),
            logical,
            header,
            records,
            fully_parsed,
            decode_error,
        }
    }

    /// Split an already-**logical** (inflated, deframed) report stream into flat TSLV records.
    ///
    /// Exposed so a caller that already holds an inflated report stream can split it directly.
    pub fn from_logical_bytes(id: StreamId, logical: &[u8]) -> RecordStream {
        let result = codec::split_records(logical);
        let dialect = dialect_of(&id);
        let records = result
            .records
            .iter()
            .map(|r| flat_to_record(r, dialect))
            .collect();
        RecordStream {
            id,
            raw: logical.to_vec(),
            logical: logical.to_vec(),
            header: None,
            records,
            fully_parsed: result.complete,
            decode_error: None,
        }
    }

    /// The decoded logical report bytes (decrypted + inflated). Record origins index into
    /// this slice. Empty when the payload was not decoded.
    pub fn logical_bytes(&self) -> &[u8] {
        &self.logical
    }

    /// Parse the logical report into the **recursive record tree** (nested records read under
    /// the stack-XOR content mask, see the `crate::codec` layer). Empty when the payload was not
    /// decoded.
    ///
    /// Reads the stream in its own vocabulary, so a stream cannot be paired with another's reader:
    /// the parameter-values stream is framed like the report definition but numbers its records
    /// differently, and the query-engine streams are framed differently again. A tree read in the
    /// wrong one finds records that are not there and misses the ones that are.
    pub fn record_tree(&self) -> Vec<RecordNode> {
        match self.dialect() {
            Dialect::Contents => {
                codec::parse_tree(&self.logical, Some(field_table::declared_children))
            }
            Dialect::ReportParameters => codec::parse_tree_report_parameters(
                &self.logical,
                Some(field_table::declared_children),
            ),
            Dialect::QeSession | Dialect::Catalog => self.qe_record_tree(),
        }
    }

    /// The record tree, read only where this stream is written in `dialect` — empty otherwise.
    ///
    /// This is the form for a walk that reaches records by type number, because such a walk is
    /// written in one vocabulary and the number alone is no evidence that the stream is in it:
    /// `0x0031` is a parameter's saved current value in the parameter-values stream and an
    /// unrelated record in the report definition. Naming the vocabulary alongside the numbers is
    /// what keeps a stream that merely numbers a record the same from being read as if it were the
    /// stream the walk was written for.
    pub(crate) fn record_tree_in(&self, dialect: Dialect) -> Vec<RecordNode> {
        if self.dialect() != dialect {
            return Vec::new();
        }
        self.record_tree()
    }

    /// Parse the logical stream as a **`QENG`-framed** record tree.
    ///
    /// A `QESession` stream is parsed requiring its schema marker, which keeps a record's field data from
    /// being framed as a nested record; a stream that yields nothing under it falls back to the
    /// unconstrained parse rather than reading as empty. The `DataSourceManager` catalog mixes
    /// markers, so it takes the unconstrained parse directly.
    fn qe_record_tree(&self) -> Vec<RecordNode> {
        if self.id.is_qe_session() {
            let strict =
                codec::parse_tree_qe_session(&self.logical, Some(field_table::declared_children));
            if !strict.is_empty() {
                return strict;
            }
        }
        codec::parse_tree_catalog(&self.logical, Some(field_table::declared_children))
    }

    /// Extract the printable ASCII strings (length ≥ `min_len`) from the demasked field bytes
    /// of every record in the tree — field names, formulas, table/SQL metadata, etc. Useful
    /// for inspection.
    ///
    /// Each of a record's field-byte runs is scanned on its own. A run is contiguous in the file
    /// and two runs are not — a nested record sits between them — so a printable stretch that
    /// reaches the seam ends there, rather than being reported as one string with text that is
    /// nowhere near it in the file.
    pub fn strings(&self, min_len: usize) -> Vec<String> {
        let logical = &self.logical;
        let mut out = Vec::new();
        for root in self.record_tree() {
            root.walk(&mut |node| {
                for run in node.runs(logical) {
                    out.extend(printable_runs(&run, min_len));
                }
            });
        }
        out
    }

    /// Re-encode the records back to stream bytes. Re-emits the retained original framing,
    /// so `encode(decode(x)) == x` byte-for-byte.
    pub fn encode(&self) -> Vec<u8> {
        self.raw.clone()
    }

    /// Re-serialize the record tree back into the **logical** (inflated) report bytes it was parsed
    /// from — the re-serializable raw records the writer rests on. Equal to [`logical_bytes`] for
    /// a cleanly-framed stream (the tree's node spans partition the logical bytes); a structurally
    /// inconsistent tree would diverge, so it doubles as a tree-integrity check.
    ///
    /// [`logical_bytes`]: RecordStream::logical_bytes
    pub fn serialize_tree(&self) -> Vec<u8> {
        codec::serialize_tree(&self.record_tree(), &self.logical)
    }

    /// The stream's symbolic id.
    pub fn id(&self) -> &StreamId {
        &self.id
    }

    /// The record vocabulary this stream's records are written in.
    ///
    /// Every lookup keyed on a record type — its name, its field table, whether it is identified at
    /// all — needs this alongside the type number, because the number alone means different records
    /// in different streams.
    pub fn dialect(&self) -> Dialect {
        dialect_of(&self.id)
    }

    /// Read `node`'s content under its record type's field table.
    ///
    /// `None` when the type has no table. This is the primary form: the logical bytes and the
    /// dialect both come from the stream the node was read from, so pairing a node with the wrong
    /// buffer or the wrong vocabulary is not expressible.
    pub fn fields(&self, node: &RecordNode) -> Option<crate::fields::RecordFields> {
        crate::fields::read(node, self.logical_bytes(), self.dialect())
    }

    /// One named field of `node`, read under its record type's field table.
    ///
    /// # Errors
    ///
    /// [`Error::Edit`](crate::Error::Edit) with [`FieldEdit`](crate::EditErrorKind::FieldEdit) when
    /// the record type has no field table, or the table read no field of that path.
    pub fn field(
        &self,
        node: &RecordNode,
        path: &str,
    ) -> crate::error::Result<crate::fields::FieldRead> {
        crate::fields::field(node, self.logical_bytes(), self.dialect(), path)
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

    /// Number of decoded records whose type is not identified.
    pub fn unknown_count(&self) -> usize {
        self.records
            .iter()
            .filter(|r| matches!(r, Record::Unknown(_)))
            .count()
    }

    /// Why this stream's payload could not be decoded, if it could not be. `None` on success.
    ///
    /// Decoding is non-fatal by design, so a caller that reports "0 records" must check this to tell
    /// an unreadable stream (e.g. encrypted with a key that is not the built-in one) apart from a
    /// genuinely empty one.
    pub fn decode_error(&self) -> Option<&str> {
        self.decode_error.as_deref()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A printable stretch that reaches a nested record ends there. The runs either side of a
    /// child are not adjacent in the file, so scanning them joined reports text from two places in
    /// the stream as one string.
    #[test]
    fn a_string_does_not_span_a_nested_record() {
        // `0x008a` with a run of its own field bytes on each side of an empty `0x0151`.
        let mask = 0x8au8;
        let child: Vec<u8> = [0xd9u8, 0x51, 0x00, 0x00, 0x00, 0x00]
            .iter()
            .map(|b| b ^ mask)
            .collect();
        let mut logical = vec![0xf8u8, 0x8a, 0x07, 0x00, 0x00, 0x00, 0x00, 0x0a];
        logical.extend(b"AB".iter().map(|b| b ^ mask));
        logical.extend(child);
        logical.extend(b"CD".iter().map(|b| b ^ mask));

        let stream = RecordStream::from_logical_bytes(StreamId::Contents, &logical);
        let runs: Vec<Vec<u8>> = stream.record_tree()[0]
            .runs(stream.logical_bytes())
            .collect();
        assert_eq!(
            runs,
            vec![b"AB".to_vec(), b"CD".to_vec()],
            "the record must carry a run on each side of the nested one"
        );
        assert_eq!(stream.strings(2), ["AB", "CD"]);
    }

    /// A subreport's stream is written in the same vocabulary as the top-level stream of its name.
    /// Answering the report definition's for every path this reader does not recognize declares a
    /// subreport's saved-data catalog to be written in a vocabulary it is not.
    #[test]
    fn a_nested_stream_takes_the_vocabulary_of_its_name() {
        for (name, dialect) in [
            ("Contents", Dialect::Contents),
            ("QESession", Dialect::QeSession),
            ("DataSourceManager 5l", Dialect::Catalog),
            ("ReportParametersStream 1l", Dialect::ReportParameters),
        ] {
            let top = StreamId::classify(Path::new(name));
            assert_eq!(dialect_of(&top), dialect, "{name}");
            let nested = StreamId::Other(format!("Subdocument 3/{name}"));
            assert_eq!(dialect_of(&nested), dialect, "Subdocument 3/{name}");
        }
    }
}
