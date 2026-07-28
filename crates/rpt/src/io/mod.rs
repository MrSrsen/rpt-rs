//! Orchestration — the [`Rpt`] facade that wires the layers together.
//!
//! `Rpt::open` runs the container and codec/records substrate, then projects the semantic DOM.
//! The facade exposes the container metadata, the lossless record substrate, and the DOM.

mod anonymize;
mod cleared;
mod diagnose;
mod patch;
mod picture;

use std::fs;
use std::io::Read;
use std::path::Path;

use crate::container::{Container, SummaryInformation};
use crate::error::{IoError, Result};
use crate::records::{RecordStream, RecordTag};
use crate::StreamId;

pub use anonymize::{AnonymizeReport, Removal};
pub use cleared::EditPolicy;

use patch::{nth_node, nth_node_path, patch_leaf_region};
use picture::fill_picture_data;

/// An opened `.rpt` report.
///
/// Owns the decoded per-stream substrate ([`RecordStream`]s) and the original file bytes, so
/// an unmodified [`Rpt::save`] is byte-identical to the input.
#[derive(Debug, Clone)]
pub struct Rpt {
    streams: Vec<RecordStream>,
    summary: Option<SummaryInformation>,
    /// The semantic DOM, projected from the substrate by `raise`.
    report: crate::model::Report,
    /// The exact bytes the report was opened from — re-emitted verbatim by `save`.
    original: Vec<u8>,
}

impl Rpt {
    /// Open an `.rpt` from a file path.
    ///
    /// Reads the whole file, opens the CFB/OLE2 container, decodes every stream's record substrate
    /// (decrypt → inflate → TSLV framing), and projects the semantic [`Report`](crate::Report) —
    /// including subreports, embedded pictures, and any stored saved data. Use [`Rpt::report`] for
    /// the decoded model and [`Rpt::save`] to round-trip the original bytes.
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
    /// use rpt::Rpt;
    ///
    /// let rpt = Rpt::open("report.rpt")?;
    /// println!("{} report objects", rpt.report().objects().count());
    /// # Ok::<(), rpt::Error>(())
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
        // subreport), so it is decoded once and threaded into all raise() calls.
        let report_params = streams
            .iter()
            .find(|s| matches!(s.id(), StreamId::ReportParametersStream(_)));
        let current_values = report_params
            .map(crate::project::parse_report_parameters)
            .unwrap_or_default();
        let mut report =
            crate::project::raise(contents, qe, prompt, &current_values, summary.as_ref());
        let (subreports, subdoc_names, sub_link_meta) =
            crate::project::raise_subreports(&container, &current_values);
        report.subreports = subreports;
        report.embeds = crate::container::raise_embeds(&container);
        // Static/OLE picture bytes live in the top-level `Embedding N/CONTENTS` streams, not the
        // Contents record tree; fill each `PictureObject.data` from the embedding its `0xbd`
        // ordinal points at (subreport pictures are scoped to their `Subdocument K` storage below).
        fill_picture_data(&mut report, &container, "");
        // The saved-row reader needs the raised database field types (to tell an inline `Number`
        // from an `Int32s`), so this runs after the report is raised.
        let saved = crate::codec::decode_saved_data(&streams, &report);
        report.saved_data = saved;
        // Resolve each SubreportObject's name from its backing subdocument (linked by index).
        for obj in report.objects_mut() {
            if let crate::model::ReportObjectKind::Subreport(sr) = &mut obj.kind {
                if let Some(name) = subdoc_names.get(&sr.subdoc_index) {
                    sr.subreport_name = name.clone();
                }
            }
        }
        // Fill any subreport picture bytes from that subreport's own `Subdocument K/Embedding N`
        // storage (`subdoc_names` keys share order with `report.subreports`).
        for (idx, sub) in subdoc_names.keys().zip(report.subreports.iter_mut()) {
            fill_picture_data(&mut sub.report, &container, &format!("Subdocument {idx}"));
        }
        // Subreport links: the main report stores each link in an `0x0106` record that follows the
        // subreport's `0xa3` object (grouped by subdocument index). Attach them to the matching
        // subreport. `subdoc_names` and `report.subreports` share key order.
        //
        // Each link carries: MainReportFieldName (the `0x0106` name), the LinkedParameterName (the
        // subreport parameter the main field feeds — joined by the parameter index stored at the head
        // of the `0x0106` leaf), and the SubreportFieldName (the subreport field that parameter binds
        // to, for a db-field link — equal to the parameter itself otherwise; recovered from the
        // `{field} <op> {?param}` comparisons in the subreport's `0x0076` link-selection body, see
        // `add_link_bindings`).
        if let Some(c) = contents {
            let links = crate::project::subreport_links(c);
            for (idx, sub) in subdoc_names.keys().zip(report.subreports.iter_mut()) {
                let Some(entries) = links.get(idx) else {
                    continue;
                };
                let meta = sub_link_meta.get(idx);
                let new_links: Vec<crate::model::SubreportLink> = entries
                    .iter()
                    .map(|rec| {
                        let param = meta
                            .and_then(|m| m.index_names.get(&rec.param_index))
                            .cloned();
                        // SubreportFieldName: prefer the stored `(kind, index)` handle resolved
                        // against the subreport's field pool; else the `0x0076` link-selection
                        // binding (the stored-selection case, e.g. multi-comparison links); else
                        // empty (the engine then reports the link parameter itself).
                        let subreport_field = rec
                            .sf_handle
                            .and_then(|(k, i)| crate::project::resolve_sf_handle(&sub.report, k, i))
                            .or_else(|| {
                                param
                                    .as_ref()
                                    .and_then(|p| meta.and_then(|m| m.bindings.get(p)).cloned())
                            })
                            .unwrap_or_default();
                        crate::model::SubreportLink {
                            main_report_field: rec.main_field.clone(),
                            subreport_field,
                            linked_parameter: param,
                        }
                    })
                    .collect();
                sub.links = new_links;
            }
        }
        // Resolve the subreport placeholder objects' `IsImported` / `SubreportLinks`. Both are facts
        // of a sibling structure (the subreport's `0x0142` reimport record and its resolved links),
        // keyed by the placeholder's `subdoc_index` — mirror them onto the object like `subreport_name`
        // above. `IsImported` is a non-empty reimport `source_path`; the links are the copy just
        // resolved onto `Subreport.links`. `EnableReimport` stays default (`false`) — unpinned.
        {
            let obj_facts: std::collections::HashMap<
                u32,
                (bool, Vec<crate::model::SubreportLink>),
            > = subdoc_names
                .keys()
                .zip(report.subreports.iter())
                .map(|(idx, sub)| {
                    let is_imported = sub
                        .report
                        .reimport
                        .as_ref()
                        .is_some_and(|r| !r.source_path.is_empty());
                    (*idx, (is_imported, sub.links.clone()))
                })
                .collect();
            for obj in report.objects_mut() {
                if let crate::model::ReportObjectKind::Subreport(sr) = &mut obj.kind {
                    if let Some((is_imported, links)) = obj_facts.get(&sr.subdoc_index) {
                        sr.is_imported = *is_imported;
                        sr.links = links.clone();
                    }
                }
            }
        }
        Ok(Rpt {
            streams,
            summary,
            report,
            original: bytes,
        })
    }

    /// Save the report to a path. With no edits applied, this is byte-identical to the file
    /// it was opened from.
    ///
    /// # Errors
    ///
    /// [`Error::Io`](crate::Error::Io) if the file cannot be written, naming `path` and the reason.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        fs::write(path, &self.original).map_err(|e| IoError::at("write", path, e))?;
        Ok(())
    }

    /// Write the report to any writer (byte-identical to the source when unmodified).
    ///
    /// # Errors
    ///
    /// [`Error::Io`](crate::Error::Io) if the writer fails. Names no path — the writer is the
    /// caller's.
    pub fn write(&self, mut writer: impl std::io::Write) -> Result<()> {
        writer
            .write_all(&self.original)
            .map_err(|e| IoError::new("write the report to the supplied writer", e))?;
        Ok(())
    }

    /// Re-encode the report's `Contents` stream from its current logical bytes and return the new
    /// `.rpt` file bytes — a no-op run of the write pipeline (TSLV logical → deflate → AES-CFB
    /// encrypt → CFB rewrite, every other stream verbatim). The result re-opens to byte-identical
    /// logical/record bytes; the file bytes differ because deflate is non-canonical.
    ///
    /// # Errors
    ///
    /// - [`Error::Container`](crate::Error::Container) — the report has no `Contents` stream, or the
    ///   container cannot be rewritten.
    /// - [`Error::Codec`](crate::Error::Codec) / [`Error::Crypto`](crate::Error::Crypto) — the stream
    ///   could not be re-framed, deflated, or encrypted.
    pub fn reencode(&self) -> Result<Vec<u8>> {
        let contents = self.contents_stream()?;
        self.reencode_contents(contents.logical_bytes())
    }

    /// Change a same-size region of a decoded record's leaf and return the new `.rpt` file bytes.
    ///
    /// Locates the `nth` (0-based, pre-order) record whose type is `tag` in the `Contents` record
    /// tree, then overwrites `new_bytes.len()` bytes of its **demasked leaf** starting at
    /// `leaf_offset`, re-masking each byte with the record's stack mask. Phase-1 writer: **same-size
    /// only** — `new_bytes` replaces an equal-length region, so the logical stream length never
    /// changes (a length-changing edit needs the not-yet-built record-length recompute path).
    ///
    /// Refuses an edit to a record type that is not cleared for safe editing; see
    /// [`Rpt::patch_record_leaf_with`] to override that.
    ///
    /// # Errors
    ///
    /// - [`Error::Project`](crate::Error::Project) with
    ///   [`UnclearedRecordEdit`](crate::ProjectErrorKind::UnclearedRecordEdit) — `tag` is not cleared
    ///   for editing. Refused before any bytes are produced.
    /// - [`Error::Codec`](crate::Error::Codec) — fewer than `nth + 1` records of `tag` exist, or the
    ///   region `[leaf_offset, leaf_offset + new_bytes.len())` overruns the record's leaf.
    /// - [`Error::Container`](crate::Error::Container) — the report has no `Contents` stream.
    pub fn patch_record_leaf(
        &self,
        tag: RecordTag,
        nth: usize,
        leaf_offset: usize,
        new_bytes: &[u8],
    ) -> Result<Vec<u8>> {
        self.patch_record_leaf_with(tag, nth, leaf_offset, new_bytes, EditPolicy::default())
    }

    /// As [`Rpt::patch_record_leaf`], with an explicit [`EditPolicy`].
    ///
    /// [`EditPolicy::Forced`] skips the clearance check, for callers whose purpose *is* to write a
    /// record the reader does not fully model.
    ///
    /// # Errors
    ///
    /// As [`Rpt::patch_record_leaf`], less the clearance refusal when `policy` is
    /// [`EditPolicy::Forced`].
    pub fn patch_record_leaf_with(
        &self,
        tag: RecordTag,
        nth: usize,
        leaf_offset: usize,
        new_bytes: &[u8],
        policy: EditPolicy,
    ) -> Result<Vec<u8>> {
        cleared::check(tag.0, policy)?;
        let contents = self.contents_stream()?;
        let logical = contents.logical_bytes();
        let tree = contents.record_tree();
        let node = nth_node(&tree, tag.0, nth).ok_or_else(|| {
            crate::error::CodecError::new(format!(
                "record #{nth} of type {tag:?} not found in Contents record tree"
            ))
            .in_stream("Contents")
            .record(tag.0)
        })?;

        let mut new_logical = logical.to_vec();
        patch_leaf_region(node, &mut new_logical, leaf_offset, new_bytes)?;
        self.reencode_contents(&new_logical)
    }

    /// Replace a leaf **region** of a decoded record with `new_bytes` of a **possibly different
    /// length**, and return the new `.rpt` file bytes — the phase-2 length-changing writer.
    ///
    /// Locates the `nth` (0-based, pre-order) record of type `tag` in the `Contents` record tree and
    /// replaces its demasked-leaf bytes `[region.start, region.end)` with `new_bytes` (any length).
    /// The record's own length prefix and every enclosing record's length prefix are recomputed by
    /// the size delta; because the `Contents` tree holds no absolute byte offsets, nothing else needs
    /// fixing. When `region.len() ==
    /// new_bytes.len()` this is an in-place overwrite (equivalent to [`Rpt::patch_record_leaf`]).
    ///
    /// Refuses an edit to a record type that is not cleared for safe editing; see
    /// [`Rpt::patch_record_leaf_resize_with`] to override that.
    ///
    /// # Errors
    ///
    /// Nothing is written in any of these cases:
    ///
    /// - [`Error::Project`](crate::Error::Project) with
    ///   [`UnclearedRecordEdit`](crate::ProjectErrorKind::UnclearedRecordEdit) — `tag` is not cleared
    ///   for editing.
    /// - [`Error::Codec`](crate::Error::Codec) — the record is not found, `region` is outside the leaf
    ///   or straddles a nested child record, or a recomputed length prefix would overflow its on-disk
    ///   field width.
    /// - [`Error::Container`](crate::Error::Container) — the report has no `Contents` stream.
    pub fn patch_record_leaf_resize(
        &self,
        tag: RecordTag,
        nth: usize,
        region: std::ops::Range<usize>,
        new_bytes: &[u8],
    ) -> Result<Vec<u8>> {
        self.patch_record_leaf_resize_with(tag, nth, region, new_bytes, EditPolicy::default())
    }

    /// As [`Rpt::patch_record_leaf_resize`], with an explicit [`EditPolicy`].
    ///
    /// # Errors
    ///
    /// As [`Rpt::patch_record_leaf_resize`], less the clearance refusal when `policy` is
    /// [`EditPolicy::Forced`].
    pub fn patch_record_leaf_resize_with(
        &self,
        tag: RecordTag,
        nth: usize,
        region: std::ops::Range<usize>,
        new_bytes: &[u8],
        policy: EditPolicy,
    ) -> Result<Vec<u8>> {
        cleared::check(tag.0, policy)?;
        let contents = self.contents_stream()?;
        let logical = contents.logical_bytes();
        let tree = contents.record_tree();
        let (node, ancestors) = nth_node_path(&tree, tag.0, nth).ok_or_else(|| {
            crate::error::CodecError::new(format!(
                "record #{nth} of type {tag:?} not found in Contents record tree"
            ))
            .in_stream("Contents")
            .record(tag.0)
        })?;
        let new_logical =
            crate::codec::resize_leaf_region(logical, node, &ancestors, region, new_bytes)?;
        self.reencode_contents(&new_logical)
    }

    /// The top-level `Contents` substrate stream (the primary record stream).
    fn contents_stream(&self) -> Result<&RecordStream> {
        self.stream(&StreamId::Contents).ok_or_else(|| {
            crate::error::ContainerError::new("find stream", "report has no Contents stream")
                .stream("Contents")
                .into()
        })
    }

    /// Re-encode `Contents` from replacement logical bytes and splice it into a fresh copy of the
    /// container. Shared by [`Rpt::reencode`] and [`Rpt::patch_record_leaf`].
    fn reencode_contents(&self, new_logical: &[u8]) -> Result<Vec<u8>> {
        let raw = self.contents_stream()?.raw_bytes();
        let new_stream = crate::codec::encode_contents(raw, new_logical)?;
        crate::container::rewrite_stream(&self.original, &StreamId::Contents, &new_stream)
    }

    /// The semantic DOM — the report projected from the record substrate.
    ///
    /// The byte-layout origin of each model field (its source `Contents` record and leaf layout) is
    /// documented in [`crate::provenance`].
    pub fn report(&self) -> &crate::model::Report {
        &self.report
    }

    /// The main report's raw record DOM — the type-strict tree of [`Node`](crate::raw::Node)s
    /// (typed where decoded, [`Unknown`](crate::raw::Unknown) otherwise) projected from the
    /// `Contents` substrate. Built on demand (this walks the record tree on each call), not stored
    /// on the model. Empty if the report has no `Contents` stream.
    pub fn record_dom(&self) -> Vec<crate::raw::Node> {
        self.stream(&StreamId::Contents)
            .map(crate::project::build_record_dom)
            .unwrap_or_default()
    }

    /// How completely this report decoded — per stream: unrecognized record count and types, logical
    /// bytes belonging to no decoded record, and any stream that would not decode at all.
    ///
    /// Because projection is infallible by design (see [`crate::error`]), an incomplete decode raises
    /// no error and would otherwise be invisible: an export or render missing content still looks
    /// authoritative. Check [`DecodeCoverage::warning`](crate::DecodeCoverage::warning) before
    /// presenting output as the whole report.
    ///
    /// Read off the already-decoded substrate, so this is cheap — no second pass over the bytes.
    ///
    /// ```no_run
    /// # use rpt::Rpt;
    /// let rpt = Rpt::open("report.rpt")?;
    /// if let Some(w) = rpt.decode_coverage().warning() {
    ///     eprintln!("warning: {w}");
    /// }
    /// # Ok::<(), rpt::Error>(())
    /// ```
    pub fn decode_coverage(&self) -> crate::DecodeCoverage {
        crate::DecodeCoverage {
            streams: self
                .streams
                .iter()
                .map(crate::coverage::StreamCoverage::of)
                .collect(),
        }
    }

    /// A structural summary of the `Contents` record stream: how many records of each type, sorted
    /// by descending frequency then type. Built on demand from the substrate.
    pub fn inventory(&self) -> Vec<crate::raw::RecordTypeCount> {
        self.stream(&StreamId::Contents)
            .map(crate::project::build_inventory)
            .unwrap_or_default()
    }

    /// Each subreport's raw record DOM, in the same order as [`report().subreports`](Rpt::report).
    /// Like [`record_dom`](Rpt::record_dom) but for the `Subdocument N/Contents` substrate, decoded
    /// on demand. The `n`-th entry is the DOM of the `n`-th subreport.
    pub fn subreport_record_doms(&self) -> Vec<Vec<crate::raw::Node>> {
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
                // so decode its logical payload here before projecting the DOM.
                let decoded = RecordStream::decode(StreamId::Contents, s.raw_bytes());
                crate::project::build_record_dom(&decoded)
            })
            .collect()
    }

    /// Iterate the decoded substrate streams with their symbolic ids.
    pub fn streams(&self) -> impl Iterator<Item = (&StreamId, &RecordStream)> {
        self.streams.iter().map(|s| (s.id(), s))
    }

    /// The substrate for a specific stream, if present.
    pub fn stream(&self, id: &StreamId) -> Option<&RecordStream> {
        self.streams.iter().find(|s| s.id() == id)
    }

    /// The parsed `SummaryInformation` (title/author/…), if present.
    pub fn summary_info(&self) -> Option<&SummaryInformation> {
        self.summary.as_ref()
    }

    /// The raw bytes the report was opened from.
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
        crate::codec::saved_record_count(&self.data_source_manager_logical()?)
    }

    /// Decode the saved record index (`SavedRecordsStream`) to its inflated record bytes, using the
    /// item count and width from the primary `DataSourceManager` batch header. `None` when there is no
    /// saved data or the index cannot be decoded.
    pub fn saved_index(&self) -> Option<Vec<u8>> {
        let dsm = self.data_source_manager_logical()?;
        // The record index is the leading run of same-item_size batches, possibly multi-batch.
        let index_batches = crate::codec::index_directory(&dsm);
        let srs = self
            .stream_by(|id| matches!(id, StreamId::SavedRecordsStream(_)))?
            .encode();
        crate::codec::decode_index_stream(&srs, &index_batches)
    }

    /// The report's decoded stored saved data (cached rows). See [`crate::model::SavedData`]. `None`
    /// when there is no saved data or the batch class is not decodable.
    pub fn saved_data(&self) -> Option<crate::model::SavedData> {
        crate::codec::decode_saved_data(&self.streams, &self.report)
    }

    /// A byte-level view of the saved-data batch substrate: the decoded catalog schema, the batch
    /// directory, and — per batch — the derived decrypt IV and whether it yields a zlib header
    /// (and, on success, the inflated first record). This is the data behind `rpt dump --saved`; it
    /// reaches the encrypted-batch layer the plain record `dump` cannot.
    /// `None` when the report carries no saved-data directory.
    pub fn saved_batch_inspection(&self) -> Option<crate::model::SavedBatchInspection> {
        let dsm = self.data_source_manager_logical()?;
        let schema = crate::codec::saved_schema(&dsm);
        let srs = self
            .stream_by(|id| matches!(id, StreamId::SavedRecordsStream(_)))
            .map(|s| s.encode())
            .unwrap_or_default();
        let memo = self
            .stream_by(|id| matches!(id, StreamId::MemoValuesStream(_)))
            .map(|s| s.encode())
            .unwrap_or_default();
        Some(crate::codec::inspect_saved_batches(
            &dsm, &srs, &memo, &schema,
        ))
    }
}
