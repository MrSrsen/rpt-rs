//! Orchestration — the [`Rpt`] facade that wires the layers together.
//!
//! `Rpt::open` runs the container and codec/records substrate, then projects the semantic DOM.
//! The facade exposes the container metadata, the lossless record substrate, and the DOM.

use std::fs;
use std::io::Read;
use std::path::Path;

use crate::container::{Container, SummaryInformation};
use crate::error::Result;
use crate::records::RecordStream;
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
        let report_params = streams.iter().find(
            |s| matches!(s.id(), StreamId::Other(n) if n.starts_with("ReportParametersStream")),
        );
        let current_values = report_params
            .map(crate::project::parse_report_parameters)
            .unwrap_or_default();
        let mut report =
            crate::project::raise(contents, qe, prompt, &current_values, summary.as_ref());
        let (subreports, subdoc_names, sub_link_meta) =
            raise_subreports(&container, &current_values);
        report.subreports = subreports;
        report.embeds = raise_embeds(&container);
        report.saved_data = decode_saved_data(&streams);
        // Resolve each SubreportObject's name from its backing subdocument (linked by index).
        for area in &mut report.report_definition.areas {
            for section in &mut area.sections {
                for obj in &mut section.objects {
                    if let crate::model::ReportObjectKind::Subreport(sr) = &mut obj.kind {
                        if let Some(name) = subdoc_names.get(&sr.subdoc_index) {
                            sr.subreport_name = name.clone();
                        }
                    }
                }
            }
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
            let links = subreport_links(c);
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
                            .and_then(|(k, i)| resolve_sf_handle(&sub.report, k, i))
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

    /// The semantic DOM — the report projected from the record substrate.
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

    /// The top-level `DataSourceManager` stream's decoded (decrypted + inflated) logical bytes, which
    /// carry the saved-data batch directory. `None` when absent or undecodable.
    fn data_source_manager_logical(&self) -> Option<Vec<u8>> {
        let s = self.streams.iter().find(|s| {
            let n = format!("{:?}", s.id());
            n.contains("DataSourceManager") && !n.contains("Subdocument")
        })?;
        crate::codec::decode_contents(&s.encode()).ok()
    }

    fn saved_stream(&self, name: &str) -> Option<Vec<u8>> {
        self.streams
            .iter()
            .find(|s| format!("{:?}", s.id()).contains(name))
            .map(|s| s.encode())
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
        // The record-index batch is the directory's primary (largest-count) batch.
        let primary = crate::codec::batch_directory(&dsm)
            .into_iter()
            .max_by_key(|b| b.count)?;
        let srs = self.saved_stream("SavedRecordsStream")?;
        crate::codec::decode_saved_batch(
            &srs,
            crate::codec::INDEX_BATCH_SIZE,
            primary.count,
            primary.item_size,
        )
    }

    /// The report's decoded stored saved data (cached rows). See [`crate::model::SavedData`]. `None`
    /// when there is no saved data or the batch class is not decodable.
    pub fn saved_data(&self) -> Option<crate::model::SavedData> {
        decode_saved_data(&self.streams)
    }
}

/// Decode a report's stored saved data from its `SavedRecordsStream` (record index) and
/// `MemoValuesStream` (variable-length values). Returns the stored records — not the engine's
/// result rowset, which projects/reorders/groups/formats them. `None` when there is no saved data,
/// no `MemoValuesStream`, or the streams do not decode.
fn decode_saved_data(streams: &[RecordStream]) -> Option<crate::model::SavedData> {
    use crate::codec;
    use crate::model::{FieldValueType, SavedColumn, SavedData};

    let find = |needle: &str, excl: Option<&str>| {
        streams.iter().find(|s| {
            let n = format!("{:?}", s.id());
            n.contains(needle) && excl.is_none_or(|e| !n.contains(e))
        })
    };
    let dsm =
        codec::decode_contents(&find("DataSourceManager", Some("Subdocument"))?.encode()).ok()?;
    let primary = codec::batch_directory(&dsm)
        .into_iter()
        .max_by_key(|b| b.count)
        .filter(|b| b.count > 0)?;

    // Decodable only when the field values are in an external MemoValuesStream.
    let memo_raw = find("MemoValuesStream", None)?.encode();
    let srs_raw = find("SavedRecordsStream", None)?.encode();

    let index_plain = codec::decode_saved_batch(
        &srs_raw,
        codec::INDEX_BATCH_SIZE,
        primary.count,
        primary.item_size,
    )?;
    let bs = codec::memo_batch_size(&dsm)?;
    let memo_plain = codec::decode_saved_batch(&memo_raw, bs, bs.checked_mul(12)?, 12)?;
    let memos = codec::parse_memo_values(&memo_plain);

    let schema = codec::saved_schema(&dsm);
    if schema.is_empty() {
        return None;
    }
    let rows = codec::decode_worrall_rows(
        &index_plain,
        &memos,
        &schema,
        primary.count,
        primary.item_size,
    );
    if rows.is_empty() {
        return None;
    }
    let columns = schema
        .iter()
        .map(|f| SavedColumn {
            name: f.name.clone(),
            value_type: if f.is_memo {
                FieldValueType::PersistentMemo
            } else {
                FieldValueType::Int32s
            },
        })
        .collect();
    Some(SavedData {
        record_count: primary.count,
        columns,
        rows,
    })
}

/// Summarise embedded OLE objects: for each top-level `Embedding N` storage, hash its `\x01Ole`
/// stream into an [`Embed`] (Name `Ole`, byte size, Base64-MD5), in directory order. Only the
/// `Ole` stream is listed (not `CompObj` / `CONTENTS`).
fn raise_embeds(container: &Container) -> Vec<crate::model::Embed> {
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
        // The stream name carries a `\x01` (OLE control) prefix; strip control chars for the Name.
        let name: String = stream.chars().filter(|c| !c.is_control()).collect();
        if storage.starts_with("Embedding ") && name == "Ole" {
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
                index_names: subreport_param_index_names(&contents, prompt_xml.as_deref()),
                bindings: subreport_link_bindings(&contents),
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

/// Map each subreport parameter index to its name, joining the parameter detail records (`0x007a`,
/// whose leaf begins with the `u16` engine parameter index and embeds the `crobj://{…}` GUID) to the
/// subreport's `PromptManager` (GUID → parameter Name). A subreport link's `0x0106` record stores
/// this parameter index, so the map turns it into the LinkedParameterName.
fn subreport_param_index_names(
    contents: &RecordStream,
    prompt_xml: Option<&str>,
) -> std::collections::HashMap<u16, String> {
    use crate::codec::RecordNode;
    const PARAM_RECORD: u16 = 0x007a;
    // GUID (`crobj://…`) → parameter Name, from the PromptManager CRMetaObjects XML.
    let mut guid_name: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Some(xml) = prompt_xml {
        for chunk in xml.split("<MetaObject").skip(1) {
            let Some(id) = chunk
                .split("<ID>crobj://")
                .nth(1)
                .and_then(|t| t.split("</ID>").next())
            else {
                continue;
            };
            // The parameter's own Name lives in its `<Object xsi:type="Parameter">` element.
            let Some((_, obj)) = chunk.split_once("<Object xsi:type=\"Parameter\"") else {
                continue;
            };
            if let Some(name) = obj
                .split_once("<Name>")
                .and_then(|(_, t)| t.split_once("</Name>"))
                .map(|(n, _)| n)
            {
                guid_name.insert(id.to_string(), name.to_string());
            }
        }
    }
    let logical = contents.logical_bytes();
    let mut map = std::collections::HashMap::new();
    let mut visit = |n: &RecordNode| {
        if n.rtype != PARAM_RECORD {
            return;
        }
        let leaf = n.leaf_bytes(logical);
        let Some(index) = leaf
            .get(0..2)
            .and_then(|b| <[u8; 2]>::try_from(b).ok())
            .map(u16::from_be_bytes)
        else {
            return;
        };
        // Extract the `crobj://{…}` GUID body (the text after `crobj://`, up to NUL).
        if let Some(pos) = leaf.windows(8).position(|w| w == b"crobj://") {
            let start = pos + 8;
            let end = leaf[start..]
                .iter()
                .position(|&b| b == 0)
                .map(|e| start + e)
                .unwrap_or(leaf.len());
            let guid = String::from_utf8_lossy(&leaf[start..end]).into_owned();
            if let Some(name) = guid_name.get(&guid) {
                map.insert(index, name.clone());
            }
        }
    };
    for root in contents.record_tree() {
        root.walk(&mut visit);
    }
    map
}

/// Map each subreport parameter to the subreport field it binds to, decoded from the subreport's
/// `0x0076` link-selection records. When a main-report field is linked into a subreport on a db
/// field, the engine stores the join as a formula record whose first operand is the bound subreport
/// field (`Command.client_id`, `@period_from`, …) and whose second operand is the `?<parameter>` it
/// is compared with (the auto-created link parameter). Returns `{parameter-name → field}`; a link
/// parameter absent from the map binds directly to itself (no db field — the SubreportFieldName then
/// equals the LinkedParameterName).
fn subreport_link_bindings(contents: &RecordStream) -> std::collections::HashMap<String, String> {
    use crate::codec::RecordNode;
    const FORMULA_BODY: u16 = 0x0076;
    let logical = contents.logical_bytes();
    let mut map = std::collections::HashMap::new();
    fn visit(n: &RecordNode, logical: &[u8], map: &mut std::collections::HashMap<String, String>) {
        if n.rtype == FORMULA_BODY {
            let leaf = n.leaf_bytes(logical);
            // The link-selection formula's body (the human-readable operand) holds each comparison
            // as `{sub.field} <op> {?Pm-<main>}`. A single selection can join several with `and`
            // (e.g. `{T.a} = {?Pm-T.x} and {T.b} = {?Pm-@y}`), so every comparison is parsed, not
            // just the first. Parsing the body (not the flat operand list) keeps the comparison
            // operator, which gates non-equality clauses (see `add_link_bindings`).
            for s in u32_lp_strings(&leaf) {
                if s.contains("{?") {
                    add_link_bindings(&s, map);
                }
            }
        }
        for c in &n.children {
            visit(c, logical, map);
        }
    }
    for root in contents.record_tree() {
        visit(&root, logical, &mut map);
    }
    map
}

/// Parse a selection-formula body for `{sub.field} <op> {?Pm-<main>}` link comparisons and record
/// `{parameter → bound field}` for each. The parameter key is the text inside `{?…}` (e.g.
/// `Pm-@p_search_date`), matching the `LinkedParameterName` lookup.
///
/// Only the engine's auto-generated *equality* form (`=`) binds unconditionally. A non-equality
/// clause (`<`, `<=`, `>`, `>=`) is accepted only when its left column matches the link's column
/// (Crystal names the link parameter `Pm-<main>` and compares the *same* column on the subreport
/// side). This rejects a user filter that merely re-uses the link parameter on a *different* column
/// (e.g. `{T.leave_date} >= {?Pm-T.service_date}`), whose SubreportFieldName stays the parameter.
/// Mirrors the `Field.UseCount` rule in `rpt-engine::subreport_link_field`.
fn add_link_bindings(body: &str, map: &mut std::collections::HashMap<String, String>) {
    let pat = "{?";
    for (idx, _) in body.match_indices(pat) {
        let rest = &body[idx + 1..]; // starts at "?…}"
        let Some(close) = rest.find('}') else {
            continue;
        };
        let param = &rest[1..close]; // inside braces, sans leading '?'
        if param.is_empty() {
            continue;
        }
        // The comparison operator immediately before `{?…}`. `=` may be the tail of `<=`/`>=`/`<>`.
        let before = body[..idx].trim_end();
        let (lhs, is_equality) = if let Some(l) = before.strip_suffix('=') {
            match l.chars().last() {
                Some(c @ ('<' | '>' | '!' | '=')) => (l.trim_end_matches(c).trim_end(), false),
                _ => (l.trim_end(), true),
            }
        } else if let Some(l) = before
            .strip_suffix('<')
            .or_else(|| before.strip_suffix('>'))
        {
            (l.trim_end(), false)
        } else {
            continue;
        };
        // The left operand must be a `{table.field}` database reference.
        let Some(field) = lhs
            .strip_suffix('}')
            .and_then(|l| l.rfind('{').map(|b| &l[b + 1..]))
        else {
            continue;
        };
        if field.is_empty() || field.starts_with(['?', '@']) {
            continue;
        }
        // Non-equality only counts when comparing the same column the link is on. The link
        // parameter is `Pm-<main>`, so the link column is `<main>`'s column (text after its last `.`).
        let link_col = param
            .strip_prefix("Pm-")
            .unwrap_or(param)
            .rsplit('.')
            .next()
            .unwrap_or("");
        let field_col = field.rsplit('.').next().unwrap_or(field);
        if is_equality || field_col == link_col {
            map.entry(param.to_string())
                .or_insert_with(|| field.to_string());
        }
    }
}

/// Extract the `u32`-big-endian length-prefixed, NUL-terminated strings from a record leaf (the
/// operand encoding of a `0x0076` formula record). Scans byte-by-byte, accepting a run only when the
/// declared length yields a NUL-terminated span of printable text — robust to the variable filler
/// bytes between operands. Tab/CR/LF are accepted alongside printable ASCII so the multi-line
/// **formula body** operand (which carries the comparison operators) is captured, not just the
/// single-token field/parameter operands.
fn u32_lp_strings(leaf: &[u8]) -> Vec<String> {
    let printable = |b: u8| (0x20..0x7f).contains(&b) || matches!(b, b'\t' | b'\r' | b'\n');
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 < leaf.len() {
        let len = u32::from_be_bytes([leaf[i], leaf[i + 1], leaf[i + 2], leaf[i + 3]]) as usize;
        if (2..=512).contains(&len) && i + 4 + len <= leaf.len() {
            let span = &leaf[i + 4..i + 4 + len];
            if span[len - 1] == 0 && span[..len - 1].iter().all(|&b| printable(b)) {
                out.push(String::from_utf8_lossy(&span[..len - 1]).into_owned());
                i += 4 + len;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// One decoded `0x0106` subreport-link record: the subreport-parameter index the main field feeds,
/// the `MainReportFieldName`, and the **SubreportFieldName handle** — the `(kind, index)` pair
/// resolved against the subreport's field pool.
struct LinkRecord {
    param_index: u16,
    main_field: String,
    /// `(field-kind, pool-index)` from the link record's trailing descriptor: `kind` selects the
    /// subreport field pool (`0` = database field, `1` = formula), `index` the entry within it.
    /// `None` when the link carries no distinct subreport field (the short trailing form, or kind
    /// `0xffff`), in which case `SubreportFieldName` falls back to the link parameter.
    sf_handle: Option<(u16, u16)>,
}

/// Each subreport link, grouped by subdocument index. In the main report's `Contents`, every
/// subreport object (`0xa3`) is followed by one `0x0106` link record per link. The leaf is
/// `[u16 linked-parameter-index][u32-BE namelen][main-report field name…NUL][trailing descriptor]`:
/// the leading `u16` is the **subreport parameter index** the main field feeds (the join key that
/// pairs a link to the auto-created subreport parameter), and the length-prefixed name is the
/// `MainReportFieldName`. The **trailing descriptor** (when 8+ bytes:
/// `[main-field-kind/index ×4][u16-BE SF-kind][u16-BE SF-index]`) carries the SubreportFieldName
/// handle. The engine counts one `Field.UseCount` per link.
fn subreport_links(contents: &RecordStream) -> std::collections::BTreeMap<u32, Vec<LinkRecord>> {
    use crate::codec::RecordNode;
    use std::collections::BTreeMap;
    const SUBREPORT_OBJECT: u16 = 0xa3;
    const SUBREPORT_LINK: u16 = 0x0106;
    let logical = contents.logical_bytes();
    let mut map: BTreeMap<u32, Vec<LinkRecord>> = BTreeMap::new();
    let mut current: Option<u32> = None;
    // Pre-order walk: each `0x0106` belongs to the most recently seen `0xa3` subreport object.
    fn visit(
        n: &RecordNode,
        logical: &[u8],
        current: &mut Option<u32>,
        map: &mut BTreeMap<u32, Vec<LinkRecord>>,
    ) {
        if n.rtype == SUBREPORT_OBJECT {
            *current = n
                .leaf_bytes(logical)
                .get(0..4)
                .and_then(|b| <[u8; 4]>::try_from(b).ok())
                .map(u32::from_be_bytes);
        } else if n.rtype == SUBREPORT_LINK {
            if let Some(idx) = *current {
                let lb = n.leaf_bytes(logical);
                let param_index = lb
                    .get(0..2)
                    .and_then(|b| <[u8; 2]>::try_from(b).ok())
                    .map(u16::from_be_bytes);
                if let (Some(param_index), Some(len)) = (
                    param_index,
                    lb.get(2..6)
                        .and_then(|b| <[u8; 4]>::try_from(b).ok())
                        .map(|b| u32::from_be_bytes(b) as usize),
                ) {
                    if let Some(raw) = lb.get(6..6 + len) {
                        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                        if end > 0 {
                            // The SubreportFieldName handle lives in the trailing descriptor after
                            // the name: SF-kind = BE u16 at `[4..6]`, SF-index = BE u16 at `[6..8]`.
                            // Absent (short trailing) or `0xffff` → no distinct subreport field.
                            let trailing = &lb[6 + len..];
                            let sf_handle = if trailing.len() >= 8 {
                                let kind = u16::from_be_bytes([trailing[4], trailing[5]]);
                                let index = u16::from_be_bytes([trailing[6], trailing[7]]);
                                (kind != 0xffff).then_some((kind, index))
                            } else {
                                None
                            };
                            map.entry(idx).or_default().push(LinkRecord {
                                param_index,
                                main_field: String::from_utf8_lossy(&raw[..end]).into_owned(),
                                sf_handle,
                            });
                        }
                    }
                }
            }
        }
        for c in &n.children {
            visit(c, logical, current, map);
        }
    }
    for root in contents.record_tree() {
        visit(&root, logical, &mut current, &mut map);
    }
    map
}

/// Resolve a SubreportFieldName `(kind, index)` handle against the subreport's field pool.
/// `kind 0` = the index-th **database** field definition → `{table}.{field}`; `kind 1` = the
/// index-th **formula** → `@{name}`. The pools are `data_definition.field_definitions` filtered by
/// kind, in stored order. Other kinds (group, summary, …) are not yet mapped and return `None`
/// (the caller falls back).
fn resolve_sf_handle(report: &crate::model::Report, kind: u16, index: u16) -> Option<String> {
    use crate::model::FieldKindData;
    let idx = index as usize;
    match kind {
        0 => {
            let fd = report
                .data_definition
                .field_definitions
                .iter()
                .filter(|f| matches!(f.kind, FieldKindData::Database(_)))
                .nth(idx)?;
            // Qualify with the table: the DB field's own `table_alias` if present, else the table in
            // the database whose field list contains this name (single-table subreports are exact;
            // first match otherwise).
            let table = match &fd.kind {
                FieldKindData::Database(db) if !db.table_alias.is_empty() => {
                    Some(db.table_alias.clone())
                }
                _ => report
                    .database
                    .tables
                    .iter()
                    .find(|t| t.data_fields.iter().any(|d| d.name == fd.name))
                    .map(|t| {
                        if t.alias.is_empty() {
                            t.name.clone()
                        } else {
                            t.alias.clone()
                        }
                    }),
            };
            Some(match table {
                Some(t) => format!("{t}.{}", fd.name),
                None => fd.name.clone(),
            })
        }
        1 => {
            let fd = report
                .data_definition
                .field_definitions
                .iter()
                .filter(|f| matches!(f.kind, FieldKindData::Formula(_)))
                .nth(idx)?;
            Some(format!("@{}", fd.name))
        }
        _ => None,
    }
}

/// The subreport's friendly name, read from its `Subdocument`'s report-header record (`0x0064`):
/// leaf bytes `[7..11]` hold a big-endian `u32` length, then a NUL-terminated string (the NUL is
/// included in the length). Returns empty if absent.
fn subreport_name_from_contents(contents: &RecordStream) -> String {
    const REPORT_HEADER: u16 = 0x0064;
    let logical = contents.logical_bytes();
    for root in contents.record_tree() {
        if root.rtype == REPORT_HEADER {
            let lb = root.leaf_bytes(logical);
            if let Some(len) = lb
                .get(7..11)
                .and_then(|b| <[u8; 4]>::try_from(b).ok())
                .map(u32::from_be_bytes)
            {
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
