//! Orchestration — the [`Rpt`] facade that wires the layers together.
//!
//! `Rpt::open` runs the container and codec/records substrate, then projects the semantic DOM.
//! The facade exposes the container metadata, the lossless record substrate, and the DOM.

use std::fs;
use std::io::Read;
use std::path::Path;

use crate::codec::RecordNode;
use crate::container::{Container, SummaryInformation};
use crate::error::Result;
use crate::records::rtype::REPORT_HEADER;
use crate::records::{RecordStream, RecordTag};
use crate::StreamId;

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
    /// Returns [`Err`] if the file cannot be read, the bytes are not a valid CFB container, or the
    /// `Contents` stream fails to decode (bad header, decryption, or record framing).
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
        let bytes = fs::read(path.as_ref())?;
        Rpt::from_bytes(bytes)
    }

    /// Open an `.rpt` from any reader.
    pub fn read(mut reader: impl Read) -> Result<Rpt> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        Rpt::from_bytes(bytes)
    }

    fn from_bytes(bytes: Vec<u8>) -> Result<Rpt> {
        let container = Container::from_bytes(&bytes)?;
        let summary = container.summary_info();
        let streams: Vec<RecordStream> = container
            .streams()
            .iter()
            .map(|s| RecordStream::decode(s.id.clone(), &s.bytes))
            .collect();
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
            raise_subreports(&container, &current_values);
        report.subreports = subreports;
        report.embeds = raise_embeds(&container);
        // Static/OLE picture bytes live in the top-level `Embedding N/CONTENTS` streams, not the
        // Contents record tree; fill each `PictureObject.data` from the embedding its `0xbd`
        // ordinal points at (subreport pictures are scoped to their `Subdocument K` storage below).
        fill_picture_data(&mut report, &container, "");
        // The saved-row reader needs the raised database field types (to tell an inline `Number`
        // from an `Int32s`), so this runs after the report is raised.
        let saved = decode_saved_data(&streams, &report);
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
        Ok(Rpt {
            streams,
            summary,
            report,
            original: bytes,
        })
    }

    /// Save the report to a path. With no edits applied, this is byte-identical to the file
    /// it was opened from.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        fs::write(path.as_ref(), &self.original)?;
        Ok(())
    }

    /// Write the report to any writer (byte-identical to the source when unmodified).
    pub fn write(&self, mut writer: impl std::io::Write) -> Result<()> {
        writer.write_all(&self.original)?;
        Ok(())
    }

    /// Re-encode the report's `Contents` stream from its current logical bytes and return the new
    /// `.rpt` file bytes — a no-op run of the write pipeline (TSLV logical → deflate → AES-CFB
    /// encrypt → CFB rewrite, every other stream verbatim). The result re-opens to byte-identical
    /// logical/record bytes; the file bytes differ because deflate is non-canonical.
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
    /// Errors if fewer than `nth + 1` records of `tag` exist, or the region `[leaf_offset,
    /// leaf_offset + new_bytes.len())` overruns the record's leaf bytes.
    pub fn patch_record_leaf(
        &self,
        tag: RecordTag,
        nth: usize,
        leaf_offset: usize,
        new_bytes: &[u8],
    ) -> Result<Vec<u8>> {
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
    /// Errors (writing nothing) if the record is not found, the region is out of the leaf or straddles
    /// a nested child record, or a recomputed length prefix would overflow its on-disk field width.
    pub fn patch_record_leaf_resize(
        &self,
        tag: RecordTag,
        nth: usize,
        region: std::ops::Range<usize>,
        new_bytes: &[u8],
    ) -> Result<Vec<u8>> {
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
        crate::codec::decode_contents(&s.encode()).ok()
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
        decode_saved_data(&self.streams, &self.report)
    }

    /// A reverse-engineering view of the saved-data batch substrate: the decoded catalog schema, the
    /// batch directory, and — per batch — the derived decrypt IV and whether it yields a zlib header
    /// (and, on success, the inflated first record). This is the RE instrument behind
    /// `rpt dump --saved`; it reaches the encrypted-batch layer the plain record `dump` cannot.
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

    /// Brute-force the saved-batch IV metadata for the ciphertext at `stream[cursor..]`, searching the
    /// given `(batch_size, item_count, item_size, seq)` candidate values and returning every tuple
    /// whose IV both passes the zlib gate and inflates. The search instrument for cracking an
    /// undecoded batch class; `stream` selects `SavedRecordsStream` (false) or `MemoValuesStream`
    /// (true). Stops after `limit` hits (`0` = unbounded).
    #[allow(clippy::too_many_arguments)]
    pub fn saved_iv_search(
        &self,
        in_memo_stream: bool,
        cursor: usize,
        batch_sizes: &[u32],
        item_counts: &[u32],
        item_sizes: &[u32],
        seqs: &[u32],
        limit: usize,
    ) -> Vec<crate::model::SavedIvHit> {
        let pred: fn(&StreamId) -> bool = if in_memo_stream {
            |id| matches!(id, StreamId::MemoValuesStream(_))
        } else {
            |id| matches!(id, StreamId::SavedRecordsStream(_))
        };
        let Some(raw) = self.stream_by(pred).map(|s| s.encode()) else {
            return Vec::new();
        };
        let ct = raw.get(cursor..).unwrap_or(&[]);
        crate::codec::saved_iv_search(ct, batch_sizes, item_counts, item_sizes, seqs, limit)
    }
}

/// The `nth` (0-based, pre-order) node in `tree` whose record type is `rtype`, or `None`.
fn nth_node(tree: &[RecordNode], rtype: u16, nth: usize) -> Option<&RecordNode> {
    fn visit<'a>(n: &'a RecordNode, rtype: u16, remaining: &mut usize) -> Option<&'a RecordNode> {
        if n.rtype == rtype {
            if *remaining == 0 {
                return Some(n);
            }
            *remaining -= 1;
        }
        n.children.iter().find_map(|c| visit(c, rtype, remaining))
    }
    let mut remaining = nth;
    tree.iter()
        .find_map(|root| visit(root, rtype, &mut remaining))
}

/// Like [`nth_node`], but also returns the target's ancestor chain (root-first, excluding the
/// target itself) — the records whose length prefixes must be recomputed on a length-changing edit.
fn nth_node_path(
    tree: &[RecordNode],
    rtype: u16,
    nth: usize,
) -> Option<(&RecordNode, Vec<&RecordNode>)> {
    fn visit<'a>(
        n: &'a RecordNode,
        rtype: u16,
        remaining: &mut usize,
        path: &mut Vec<&'a RecordNode>,
    ) -> Option<(&'a RecordNode, Vec<&'a RecordNode>)> {
        if n.rtype == rtype {
            if *remaining == 0 {
                return Some((n, path.clone()));
            }
            *remaining -= 1;
        }
        path.push(n);
        let found = n
            .children
            .iter()
            .find_map(|c| visit(c, rtype, remaining, path));
        path.pop();
        found
    }
    let mut remaining = nth;
    let mut path = Vec::new();
    tree.iter()
        .find_map(|root| visit(root, rtype, &mut remaining, &mut path))
}

/// Overwrite `new_bytes.len()` bytes of `node`'s demasked leaf into `logical`, starting at
/// `leaf_offset` and re-masking with the node's stack mask. Same-size: `logical`'s length is
/// unchanged. Errors if the region overruns the record's leaf.
fn patch_leaf_region(
    node: &RecordNode,
    logical: &mut [u8],
    leaf_offset: usize,
    new_bytes: &[u8],
) -> Result<()> {
    let segments = leaf_segments(node);
    let leaf_len: usize = segments.iter().map(|(s, e)| e - s).sum();
    let end = leaf_offset.checked_add(new_bytes.len()).ok_or_else(|| {
        crate::error::CodecError::new(format!(
            "patch region offset {leaf_offset} + length {} overflows usize",
            new_bytes.len()
        ))
        .record(node.rtype)
    })?;
    if end > leaf_len {
        return Err(crate::error::CodecError::new(format!(
            "patch region [{leaf_offset}, {end}) overruns record leaf of {leaf_len} bytes"
        ))
        .record(node.rtype)
        .into());
    }
    // Walk the leaf's logical segments, writing each source byte whose leaf position lands in
    // [leaf_offset, end). The leaf maps to logical piecewise (child spans are skipped), but the
    // stack mask is uniform across the whole record.
    let mut leaf_pos = 0usize;
    let mut written = 0usize;
    for (s, e) in segments {
        for slot in &mut logical[s..e] {
            if (leaf_offset..end).contains(&leaf_pos) {
                *slot = new_bytes[written] ^ node.mask;
                written += 1;
            }
            leaf_pos += 1;
        }
    }
    debug_assert_eq!(written, new_bytes.len());
    Ok(())
}

/// The logical byte spans that make up `node`'s own leaf — the content gaps not covered by any
/// child, in order. The inverse of the concatenation [`RecordNode::leaf_bytes`] performs.
fn leaf_segments(node: &RecordNode) -> Vec<(usize, usize)> {
    node.leaf_segments()
}

/// Decode a report's stored saved data from its `SavedRecordsStream` (record index) and
/// `MemoValuesStream` (variable-length values). Returns the stored records — not the engine's
/// result rowset, which projects/reorders/groups/formats them. `None` when there is no saved data,
/// no `MemoValuesStream`, or the streams do not decode.
fn decode_saved_data(
    streams: &[RecordStream],
    report: &crate::model::Report,
) -> Option<crate::model::SavedData> {
    use crate::codec;
    use crate::model::{SavedColumn, SavedData};

    let find = |pred: fn(&StreamId) -> bool| streams.iter().find(|s| pred(s.id()));
    // The top-level `DataSourceManager` variant is inherently non-subdocument (nested streams stay
    // `StreamId::Other`), so no explicit Subdocument exclusion is needed.
    let dsm =
        codec::decode_contents(&find(|id| matches!(id, StreamId::DataSourceManager(_)))?.encode())
            .ok()?;

    // Decodable only when the field values are in an external MemoValuesStream. Reports with no memo
    // columns (all-inline) still decode: the memo stream may be absent.
    let memo_raw = find(|id| matches!(id, StreamId::MemoValuesStream(_)))
        .map(|s| s.encode())
        .unwrap_or_default();
    let srs_raw = find(|id| matches!(id, StreamId::SavedRecordsStream(_)))?.encode();

    let schema = codec::saved_schema(&dsm);
    if schema.is_empty() {
        return None;
    }
    // Each stored column's value type: a memo column is a `PersistentMemo`; every other column takes
    // its declared type from the report's database field of the same qualified name (the inline
    // packed reader keys the on-disk field width on this — a `Number` is an 8-byte double, an
    // `Int32s` is 4 bytes, a `String` is a NUL-terminated UTF-16 run). Unmatched fields fall back to
    // `Int32s`.
    let field_types = saved_field_types(&schema, report);
    // The stored rows: index batches (inline fields, packed or fixed) + memo-descriptor batches whose
    // cells point into the memo-value heaps (no delta reconstruction needed).
    let (rows, record_count) =
        codec::decode_saved_rows(&dsm, &srs_raw, &memo_raw, &schema, &field_types)?;
    if rows.is_empty() {
        return None;
    }
    let columns = schema
        .iter()
        .zip(field_types)
        .map(|(f, value_type)| SavedColumn {
            name: f.name.clone(),
            value_type,
        })
        .collect();
    Some(SavedData {
        record_count,
        columns,
        rows,
    })
}

/// Resolve each saved column's value type (schema order): a memo column is a `PersistentMemo`; every
/// other column takes the declared type of the report database field with the same qualified name
/// (`Table.Field`, matched on both the table's stored name and its alias), defaulting to `Int32s`.
/// This is what tells the inline row reader a `Number` column is an 8-byte double vs an `Int32s`
/// 4-byte scalar vs a `String` — the DSM saved-field catalog itself carries no type code.
fn saved_field_types(
    schema: &[crate::codec::SavedFieldDesc],
    report: &crate::model::Report,
) -> Vec<crate::model::FieldValueType> {
    use crate::model::FieldValueType;
    use std::collections::HashMap;
    let mut by_name: HashMap<String, FieldValueType> = HashMap::new();
    for t in &report.database.tables {
        for f in &t.data_fields {
            by_name
                .entry(format!("{}.{}", t.name, f.name))
                .or_insert(f.value_type);
            if !t.alias.is_empty() {
                by_name
                    .entry(format!("{}.{}", t.alias, f.name))
                    .or_insert(f.value_type);
            }
        }
    }
    schema
        .iter()
        .map(|f| {
            if f.is_memo {
                FieldValueType::PersistentMemo
            } else {
                by_name
                    .get(&f.name)
                    .copied()
                    .unwrap_or(FieldValueType::Int32s)
            }
        })
        .collect()
}

/// Summarise embedded OLE objects: for each top-level `Embedding N` storage, hash each of its
/// OLE data streams into an [`Embed`] (Name, byte size, Base64-MD5), in directory order. The
/// engine emits the OLE data streams — `Ole`, `OlePres000`, `Ole10Native` — but not the
/// `CompObj` (OLE class descriptor) or a `CONTENTS` sub-storage, so those are skipped.
/// Fill each static `PictureObject`'s `data` from its OLE embedding. `storage_prefix` scopes the
/// lookup to the report's own storage — empty for the main report, `Subdocument K` for a subreport
/// — and the picture's `ole_ordinal` (from the `0xbd` record) selects the `Embedding N` within it.
fn fill_picture_data(
    report: &mut crate::model::Report,
    container: &Container,
    storage_prefix: &str,
) {
    for obj in report.objects_mut() {
        if let crate::model::ReportObjectKind::Picture(pic) = &mut obj.kind {
            if pic.data.is_empty() {
                if let Some(ord) = pic.ole_ordinal {
                    if let Some(bytes) = load_embedding_contents(container, storage_prefix, ord) {
                        pic.data = bytes;
                    }
                }
            }
        }
    }
}

/// Load the `CONTENTS` stream bytes of `{storage_prefix}/Embedding {ordinal}` — the native image
/// data of a static/OLE picture. Path components are compared with OLE control-char prefixes
/// (`\x01`, `\x02`) stripped, so the `\x01Ole`/`CONTENTS` naming is matched robustly.
fn load_embedding_contents(
    container: &Container,
    storage_prefix: &str,
    ordinal: u32,
) -> Option<Vec<u8>> {
    let clean = |s: &str| -> String { s.chars().filter(|c| !c.is_control()).collect() };
    let want: Vec<String> = storage_prefix
        .split('/')
        .filter(|s| !s.is_empty())
        .map(clean)
        .chain([format!("Embedding {ordinal}"), "CONTENTS".to_owned()])
        .collect();
    container.streams().iter().find_map(|s| {
        let parts: Vec<String> = s
            .path
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .filter(|c| !c.is_empty() && *c != "/" && *c != "\\")
            .map(clean)
            .collect();
        (parts == want).then(|| s.bytes.clone())
    })
}

fn raise_embeds(container: &Container) -> Vec<crate::model::Embed> {
    // Streams present under an `Embedding N` storage that the oracle does NOT list. `CompObj` is
    // the OLE1 class-moniker blob; `CONTENTS` (when present) is a nested storage, not object data.
    const SKIP: [&str; 2] = ["CompObj", "CONTENTS"];
    let mut out = Vec::new();
    for s in container.streams() {
        // Path components below the root, e.g. `["Embedding 2", "\x01Ole"]`. Only top-level
        // embeddings count (a nested `Subdocument N/Embedding …` has three components).
        let parts: Vec<&str> = s
            .path
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .filter(|c| !c.is_empty() && *c != "/" && *c != "\\")
            .collect();
        let [storage, stream] = parts.as_slice() else {
            continue;
        };
        // The stream name carries a `\x01`/`\x02` (OLE control) prefix; strip control chars for the Name.
        let name: String = stream.chars().filter(|c| !c.is_control()).collect();
        if storage.starts_with("Embedding ") && !SKIP.contains(&name.as_str()) {
            out.push(crate::model::Embed {
                name,
                size: s.bytes.len() as u64,
                md5_hash: crate::codec::md5_base64(&s.bytes),
            });
        }
    }
    out
}

/// Per-subreport metadata used to resolve its incoming links: the parameter-index → parameter-name
/// map (joins a link's `0x0106` parameter index to the LinkedParameterName) and the parameter-name →
/// bound-field map (the SubreportFieldName for a db-field link, from the `0x0076` link selection).
#[derive(Default)]
struct SubLinkMeta {
    index_names: std::collections::HashMap<u16, String>,
    bindings: std::collections::HashMap<String, String>,
}

/// Raise each subreport (`Subdocument N` storage) into a nested [`Report`]. A subreport has its
/// own `Contents` / `QESession` / `PromptManager` streams under its storage, decoded with the
/// same pipeline as the main report.
fn raise_subreports(
    container: &Container,
    current_values: &std::collections::BTreeMap<u16, Vec<crate::model::ParameterValue>>,
) -> (
    Vec<crate::model::Subreport>,
    std::collections::BTreeMap<u32, String>,
    std::collections::BTreeMap<u32, SubLinkMeta>,
) {
    use std::collections::BTreeMap;
    // Group every `Subdocument N/…` stream by its subdocument index.
    let mut groups: BTreeMap<u32, Vec<&crate::container::LoadedStream>> = BTreeMap::new();
    for s in container.streams() {
        let first = s
            .path
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .find(|c| !c.is_empty() && *c != "/");
        if let Some(name) = first {
            if let Some(n) = name.strip_prefix("Subdocument ") {
                if let Ok(idx) = n.trim().parse::<u32>() {
                    groups.entry(idx).or_default().push(s);
                }
            }
        }
    }

    let mut out = Vec::new();
    let mut names: BTreeMap<u32, String> = BTreeMap::new();
    let mut meta: BTreeMap<u32, SubLinkMeta> = BTreeMap::new();
    for (idx, group) in groups {
        // Within a subdocument, locate its Contents / QESession / PromptManager by basename.
        let by_name = |want: &str| {
            group.iter().find(|s| {
                s.path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .map(|f| f == want)
                    .unwrap_or(false)
            })
        };
        let Some(contents_raw) = by_name("Contents") else {
            continue;
        };
        let contents = RecordStream::decode(StreamId::Contents, &contents_raw.bytes);
        let qe = by_name("QESession").map(|s| RecordStream::decode(StreamId::QESession, &s.bytes));
        let prompt = by_name("PromptManager")
            .map(|s| RecordStream::decode(StreamId::PromptManager, &s.bytes));
        // A subreport's saved parameter current values can live in its own `Contents` as `0x0031`
        // records, not the report-level `ReportParametersStream` (which may be absent entirely).
        // Merge them over the main report's current values; the subreport's own values win on index
        // collision (its `0x0031` index matches the subreport parameter's own index).
        let mut sub_current = current_values.clone();
        sub_current.extend(crate::project::parse_report_parameters(&contents));
        let report = crate::project::raise(
            Some(&contents),
            qe.as_ref(),
            prompt.as_ref(),
            &sub_current,
            None,
        );
        let name = subreport_name_from_contents(&contents);
        names.insert(idx, name.clone());
        let prompt_xml = prompt
            .as_ref()
            .and_then(|p| crate::codec::decode_prompt_manager(p.raw_bytes()));
        meta.insert(
            idx,
            SubLinkMeta {
                index_names: crate::project::subreport_param_index_names(
                    &contents,
                    prompt_xml.as_deref(),
                ),
                bindings: crate::project::subreport_link_bindings(&contents),
            },
        );
        out.push(crate::model::Subreport {
            name,
            report: Box::new(report),
            links: Vec::new(),
        });
    }
    (out, names, meta)
}

/// The subreport's friendly name, read from its `Subdocument`'s report-header record (`0x0064`):
/// leaf bytes `[7..11]` hold a big-endian `u32` length, then a NUL-terminated string (the NUL is
/// included in the length). Returns empty if absent.
fn subreport_name_from_contents(contents: &RecordStream) -> String {
    let logical = contents.logical_bytes();
    for root in contents.record_tree() {
        if root.rtype == REPORT_HEADER {
            let lb = root.leaf_bytes(logical);
            if let Some(len) = crate::bytes::u32_be(&lb, 7) {
                let len = len as usize;
                if (1..=4096).contains(&len) {
                    if let Some(raw) = lb.get(11..11 + len) {
                        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                        if end > 0 {
                            return String::from_utf8_lossy(&raw[..end]).into_owned();
                        }
                    }
                }
            }
            break;
        }
    }
    String::new()
}
