//! Orchestration — the [`Rpt`] facade that wires the layers together.
//!
//! `Rpt::open` runs the container, codec and records layers, then builds the semantic model.
//! The facade exposes the container metadata, the decoded records, and the model. Its other half —
//! producing new `.rpt` bytes from an opened report — lives in [`edit`] and [`anonymize`].

mod anonymize;
mod cleared;
mod diagnose;
mod edit;
mod patch;

use std::fs;
use std::io::Read;
use std::path::Path;

use crate::container::{Container, SummaryInformation};
use crate::error::{EditErrorKind, Error, IoError, Result};
use crate::records::{RecordStream, RecordTag};
use crate::StreamId;

pub use anonymize::{AnonymizeReport, Removal};
pub use edit::EditPolicy;

use patch::{not_found, nth_node};

/// An opened `.rpt` report.
///
/// Owns the decoded per-stream records ([`RecordStream`]s) and the original file bytes, so
/// [`Rpt::original_bytes`] hands the input back byte for byte however little of it is modelled.
#[derive(Debug, Clone)]
pub struct Rpt {
    streams: Vec<RecordStream>,
    summary: Option<SummaryInformation>,
    /// The semantic model, built from the decoded records by `build_model`.
    report: crate::model::Report,
    /// What the saved-data path made of the report's stored rows, recorded by the reader that
    /// produced (or gave up on) them. Kept because a rowset that came back empty cannot be asked
    /// afterwards why.
    saved_data_status: crate::SavedDataStatus,
    /// The exact bytes the report was opened from.
    original: Vec<u8>,
}

impl Rpt {
    /// Open an `.rpt` from a file path.
    ///
    /// Reads the whole file, opens the CFB/OLE2 container, decodes every stream's records
    /// (decrypt → inflate → TSLV framing), and builds the semantic [`Report`](crate::Report) —
    /// including subreports, embedded pictures, and any stored saved data. Use [`Rpt::report`] for
    /// the decoded model and [`Rpt::original_bytes`] for the input bytes verbatim.
    ///
    /// # Errors
    ///
    /// - [`Error::Io`] — the file cannot be read, naming `path` and the underlying reason.
    /// - [`Error::Container`] — the bytes are not a valid CFB/OLE2 compound file, or a stream in it
    ///   cannot be read.
    /// - [`Error::Codec`] / [`Error::Crypto`] — a stream header, its cipher, or its TSLV record
    ///   framing could not be decoded.
    ///
    /// [`Error::Io`]: crate::Error::Io
    /// [`Error::Container`]: crate::Error::Container
    /// [`Error::Codec`]: crate::Error::Codec
    /// [`Error::Crypto`]: crate::Error::Crypto
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rpt_reader::Rpt;
    ///
    /// let rpt = Rpt::open("report.rpt")?;
    /// println!("{} report objects", rpt.report().objects().count());
    /// # Ok::<(), rpt_reader::Error>(())
    /// ```
    pub fn open(path: impl AsRef<Path>) -> Result<Rpt> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|e| IoError::at("read", path, e))?;
        // `from_bytes` has no path to name; attach it to any diagnosis on the way out.
        Rpt::from_bytes(bytes).map_err(|e| e.at_path(path))
    }

    /// Open an `.rpt` from any reader.
    ///
    /// # Errors
    ///
    /// As [`Rpt::open`], except that an I/O failure names no path — the reader is the caller's.
    pub fn read(mut reader: impl Read) -> Result<Rpt> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|e| IoError::new("read the report from the supplied reader", e))?;
        Rpt::from_bytes(bytes)
    }

    fn from_bytes(bytes: Vec<u8>) -> Result<Rpt> {
        let container =
            Container::from_bytes(&bytes).map_err(|e| diagnose::open_failure(&bytes, e))?;
        let summary = container.summary_info();
        let streams: Vec<RecordStream> = container
            .streams()
            .iter()
            .map(|s| RecordStream::decode(s.id.clone(), &s.bytes))
            .collect();
        // No `Contents` stream means this compound file is some other OLE2 document; a `Contents`
        // that will not decrypt/inflate means there is no report definition to read. Both are
        // diagnosed rather than yielding a plausible-looking empty report.
        let Some(contents_stream) = streams.iter().find(|s| s.id() == &StreamId::Contents) else {
            let present = container
                .streams()
                .iter()
                .map(|s| s.path.display().to_string())
                .collect::<Vec<_>>();
            return Err(diagnose::no_contents_stream(&present).into());
        };
        if let Some(detail) = contents_stream.decode_error() {
            return Err(diagnose::contents_undecodable(detail).into());
        }
        let contents = streams.iter().find(|s| s.id() == &StreamId::Contents);
        let qe = streams.iter().find(|s| s.id() == &StreamId::QESession);
        let prompt = streams.iter().find(|s| s.id() == &StreamId::PromptManager);
        // Saved current parameter values live in the single top-level `ReportParametersStream`,
        // keyed by the engine's global parameter index (shared across the main report and every
        // subreport), so it is decoded once and threaded into all build_report() calls.
        let report_params = streams
            .iter()
            .find(|s| matches!(s.id(), StreamId::ReportParametersStream(_)));
        let current_values = report_params
            .map(crate::build_model::parse_report_parameters)
            .unwrap_or_default();
        let mut report = crate::build_model::build_report(
            contents,
            qe,
            prompt,
            &current_values,
            summary.as_ref(),
        );
        let (subreports, subdoc_names, sub_link_meta) =
            crate::build_model::build_subreports(&container, &current_values);
        report.subreports = subreports;
        report.embeds = crate::build_model::build_embeds(&container);
        // Static/OLE picture bytes live in the `Embedding N/CONTENTS` streams, not in the Contents
        // record tree, so they are filled from the container once the objects that name them exist.
        crate::build_model::attach_pictures(&mut report, &container, &subdoc_names);
        // A subreport link is a fact of the main report and the subreport together, so it resolves
        // only once both are raised.
        crate::build_model::attach_subreports(&mut report, contents, &subdoc_names, &sub_link_meta);
        // The saved-row reader needs the raised database field types (to tell an inline `Number`
        // from an `Int32s`), so this runs after the report is raised.
        let (saved_data, saved_data_status) =
            crate::build_model::decode_saved_data(&streams, &report);
        report.saved_data = saved_data;
        Ok(Rpt {
            streams,
            summary,
            report,
            saved_data_status,
            original: bytes,
        })
    }

    /// The semantic model — the report built from the decoded records.
    ///
    /// Which record a field came from, and where in it, is stated by the decoder that reads it and
    /// by the field table it reads through — [`crate::fields::read`] replays that table over a
    /// record to show each value and the bytes behind it.
    pub fn report(&self) -> &crate::model::Report {
        &self.report
    }

    /// The main report's typed record tree — one [`Node`](crate::raw::Node) per record (typed
    /// where decoded, [`Unknown`](crate::raw::Unknown) otherwise), built from the `Contents`
    /// records. Built on demand (this walks the record tree on each call), not stored
    /// on the model. Empty if the report has no `Contents` stream.
    pub fn typed_record_tree(&self) -> Vec<crate::raw::Node> {
        self.stream(&StreamId::Contents)
            .map(crate::build_model::build_typed_record_tree)
            .unwrap_or_default()
    }

    /// How completely this report decoded — per stream: unrecognized record count and types, logical
    /// bytes belonging to no decoded record, and any stream that would not decode at all.
    ///
    /// Because building the model is infallible by design (see [`crate::error`]), an incomplete decode raises
    /// no error and would otherwise be invisible: an export or render missing content still looks
    /// authoritative. Check [`DecodeCoverage::warning`](crate::DecodeCoverage::warning) before
    /// presenting output as the whole report.
    ///
    /// The byte account is read off the already-decoded records. The unrecognized-type census is
    /// not: it parses each framed stream's record tree, because a record type that only ever occurs
    /// nested never appears in the linear walk. A call therefore costs a parse of every stream —
    /// keep the result rather than asking twice.
    ///
    /// ```no_run
    /// # use rpt_reader::Rpt;
    /// let rpt = Rpt::open("report.rpt")?;
    /// if let Some(w) = rpt.decode_coverage().warning() {
    ///     eprintln!("warning: {w}");
    /// }
    /// # Ok::<(), rpt_reader::Error>(())
    /// ```
    pub fn decode_coverage(&self) -> crate::DecodeCoverage {
        crate::DecodeCoverage {
            streams: self
                .streams
                .iter()
                .map(crate::coverage::StreamCoverage::of)
                .collect(),
            saved_data: self.saved_data_status,
        }
    }

    /// What the saved-data path made of this report's stored rows.
    ///
    /// [`Rpt::saved_data`] returning `None` covers a report saved without data, a catalog naming no
    /// field, and a batch whose cipher key came out wrong — outcomes that mean very different
    /// things. This is which of them happened, recorded by the reader as it went rather than
    /// reconstructed after the fact.
    pub fn saved_data_status(&self) -> crate::SavedDataStatus {
        self.saved_data_status
    }

    /// A structural summary of the `Contents` record stream: how many records of each type, sorted
    /// by descending frequency then type. Built on demand from the decoded records.
    pub fn inventory(&self) -> Vec<crate::raw::RecordTypeCount> {
        self.stream(&StreamId::Contents)
            .map(crate::build_model::build_inventory)
            .unwrap_or_default()
    }

    /// Each subreport's typed record tree, in the same order as
    /// [`report().subreports`](Rpt::report). Like
    /// [`typed_record_tree`](Rpt::typed_record_tree) but for the `Subdocument N/Contents` records,
    /// decoded on demand. The `n`-th entry is the tree of the `n`-th subreport.
    pub fn subreport_typed_record_trees(&self) -> Vec<Vec<crate::raw::Node>> {
        // Locate each subreport's `Contents` stream — nested streams classify as `Other` carrying
        // their full `Subdocument N/Contents` path. Order by the subdocument index so the result
        // lines up 1:1 with `report.subreports` (raised in the same ascending-index order).
        let mut indexed: Vec<(u32, &RecordStream)> = self
            .streams
            .iter()
            .filter_map(|s| {
                let StreamId::Other(path) = s.id() else {
                    return None;
                };
                let (head, tail) = path.split_once('/')?;
                if tail != "Contents" {
                    return None;
                }
                let idx = head.strip_prefix("Subdocument ")?.trim().parse().ok()?;
                Some((idx, s))
            })
            .collect();
        indexed.sort_by_key(|(idx, _)| *idx);
        indexed
            .into_iter()
            .map(|(_, s)| {
                // The nested stream retains only its raw bytes (it is not a top-level TSLV stream),
                // so decode its logical payload here before building the typed tree.
                let decoded = RecordStream::decode(StreamId::Contents, s.raw_bytes());
                crate::build_model::build_typed_record_tree(&decoded)
            })
            .collect()
    }

    /// The `nth` (0-based, pre-order) record of type `tag` in the `Contents` record tree, read
    /// under its record type's field table.
    ///
    /// This is the reading [`Rpt::patch_record_field`] addresses an edit against, so it is also how
    /// a caller finds the names and wire types a record offers. For any other stream, ask that
    /// stream: [`RecordStream::fields`].
    ///
    /// # Errors
    ///
    /// [`Error::Edit`](crate::Error::Edit) with [`FieldEdit`](crate::EditErrorKind::FieldEdit) when
    /// no such record is in the tree or its type has no field table.
    /// [`Error::Container`](crate::Error::Container) — the report has no `Contents` stream.
    pub fn record_fields(&self, tag: RecordTag, nth: usize) -> Result<crate::fields::RecordFields> {
        let contents = self.contents_stream()?;
        let tree = contents.record_tree();
        let node = nth_node(&tree, tag.0, nth).ok_or_else(|| not_found(tag, nth))?;
        contents.fields(node).ok_or_else(|| Error::Edit {
            kind: EditErrorKind::FieldEdit,
            detail: format!(
                "record type {:#06x} has no field table, so its fields cannot be read by name",
                tag.0
            ),
        })
    }

    /// One named field of the `nth` (0-based, pre-order) record of type `tag` in `Contents`.
    ///
    /// The lookup [`Rpt::patch_record_field`] performs before it writes anything, so a caller can
    /// see a field's current value and declared wire type — and get the same refusal, in the same
    /// words — without producing a file.
    ///
    /// # Errors
    ///
    /// [`Error::Edit`](crate::Error::Edit) with [`FieldEdit`](crate::EditErrorKind::FieldEdit) when
    /// no such record is in the tree, its type has no field table, or the table read no field of
    /// that path.
    pub fn record_field(
        &self,
        tag: RecordTag,
        nth: usize,
        path: &str,
    ) -> Result<crate::fields::FieldRead> {
        let contents = self.contents_stream()?;
        let tree = contents.record_tree();
        let node = nth_node(&tree, tag.0, nth).ok_or_else(|| not_found(tag, nth))?;
        contents.field(node, path)
    }

    /// Iterate the decoded streams with their symbolic ids.
    pub fn streams(&self) -> impl Iterator<Item = (&StreamId, &RecordStream)> {
        self.streams.iter().map(|s| (s.id(), s))
    }

    /// The decoded records for a specific stream, if present.
    pub fn stream(&self, id: &StreamId) -> Option<&RecordStream> {
        self.streams.iter().find(|s| s.id() == id)
    }

    /// The top-level `Contents` stream (the primary record stream), or the refusal every
    /// `Contents`-addressed reading and edit starts from.
    fn contents_stream(&self) -> Result<&RecordStream> {
        self.stream(&StreamId::Contents).ok_or_else(|| {
            crate::error::ContainerError::new("find stream", "report has no Contents stream")
                .stream("Contents")
                .into()
        })
    }

    /// The parsed `SummaryInformation` (title/author/…), if present.
    pub fn summary_info(&self) -> Option<&SummaryInformation> {
        self.summary.as_ref()
    }

    /// The exact bytes the report was opened from.
    ///
    /// This is the crate's lossless guarantee in the only form a reader needs: whatever it could
    /// not model, it still holds verbatim, so writing these bytes back out reproduces the input
    /// file. `tests/anonymize.rs` holds the crate to it.
    pub fn original_bytes(&self) -> &[u8] {
        &self.original
    }

    /// The bytes of the first top-level stream matching `pred` (a `StreamId` variant test). Nested
    /// `Subdocument N/…` streams are classified as `StreamId::Other`, so a variant match is
    /// inherently top-level only.
    fn stream_by(&self, pred: impl Fn(&StreamId) -> bool) -> Option<&RecordStream> {
        self.streams.iter().find(|s| pred(s.id()))
    }

    /// The top-level `DataSourceManager` stream's decoded (decrypted + inflated) logical bytes, which
    /// carry the saved-data batch directory. `None` when absent or undecodable.
    fn data_source_manager_logical(&self) -> Option<Vec<u8>> {
        let s = self.stream_by(|id| matches!(id, StreamId::DataSourceManager(_)))?;
        let logical = s.logical_bytes();
        (!logical.is_empty()).then(|| logical.to_vec())
    }

    /// The report's saved record count (the SDK's saved `RecordCount`), from the `DataSourceManager`
    /// batch directory. `None` when the report carries no saved data.
    pub fn saved_record_count(&self) -> Option<u32> {
        crate::build_model::read_catalog(&self.data_source_manager_logical()?).record_count()
    }

    /// The report's decoded stored saved data (cached rows). See [`crate::model::SavedData`]. `None`
    /// when there is no saved data or the batch class is not decodable —
    /// [`Rpt::saved_data_status`] says which.
    pub fn saved_data(&self) -> Option<crate::model::SavedData> {
        crate::build_model::decode_saved_data(&self.streams, &self.report).0
    }

    /// A byte-level view of the saved-data batches: the decoded catalog schema, the batch
    /// directory, and — per batch — the derived decrypt IV and whether it yields a zlib header
    /// (and, on success, the inflated first record). This is the data behind `rpt dump --saved`; it
    /// reaches the encrypted-batch layer the plain record `dump` cannot.
    /// `None` when the report carries no saved-data directory.
    pub fn saved_batch_inspection(&self) -> Option<crate::model::SavedBatchInspection> {
        let dsm = self.data_source_manager_logical()?;
        let catalog = crate::build_model::read_catalog(&dsm);
        let srs = self
            .stream_by(|id| matches!(id, StreamId::SavedRecordsStream(_)))
            .map(|s| s.encode())
            .unwrap_or_default();
        let memo = self
            .stream_by(|id| matches!(id, StreamId::MemoValuesStream(_)))
            .map(|s| s.encode())
            .unwrap_or_default();
        Some(crate::codec::inspect_saved_batches(&catalog, &srs, &memo))
    }
}
