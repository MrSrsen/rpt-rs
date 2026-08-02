//! Semantic decoding of subreport links — the incoming-link metadata a subreport carries.
//!
//! The main report stores each subreport link in an `0x0106` record following the subreport's
//! `0xa3` object; a subreport carries its own parameter-index map and link-selection bindings.
//! These decoders turn those records into the pieces of a [`crate::model::SubreportLink`], which
//! [`attach`] assembles once the main report and its subreports are both raised.

use super::row_of;
use super::tree_search::nodes_where;
use crate::codec::{Dialect, RecordNode};
use crate::container::{Container, LoadedStream};
use crate::field_table::table::Cell;
use crate::field_table::tables as ft;
use crate::records::rtype::*;
use crate::records::RecordStream;
use crate::StreamId;
use std::collections::{BTreeMap, HashMap};

/// One decoded `0x0106` subreport-link record: the subreport-parameter index the main field feeds,
/// and the `MainReportFieldName`.
pub(crate) struct LinkRecord {
    pub(crate) param_index: u16,
    pub(crate) main_field: String,
}

/// Each subreport link, grouped by subdocument index. In the main report's `Contents`, every
/// subreport object (`0xa3`) is followed by one `0x0106` link record per link; see
/// [`crate::field_table::tables::SUBREPORT_LINK`] for the record's layout.
pub(crate) fn subreport_links(contents: &RecordStream) -> BTreeMap<u32, Vec<LinkRecord>> {
    let logical = contents.logical_bytes();
    let mut map: BTreeMap<u32, Vec<LinkRecord>> = BTreeMap::new();
    // Document order, not containment: the `0xa3` record closes over the object's name alone, so a
    // link is a *later sibling* of the object it belongs to, separated from it by the object's other
    // records. Each `0x0106` therefore belongs to the most recently opened `0xa3`.
    let mut current: Option<u32> = None;
    let mut visit = |n: &RecordNode| {
        if n.rtype == SUBREPORT_OBJECT {
            current = row_of(n, logical, &ft::SUBREPORT_OBJECT)
                .get("subdocument_index")
                .and_then(Cell::u);
        } else if n.rtype == SUBREPORT_LINK {
            let Some(idx) = current else { return };
            let row = row_of(n, logical, &ft::SUBREPORT_LINK);
            let main_field = row.text("main_field").to_owned();
            let param_index = row.get("parameter_index").and_then(Cell::u);
            if let (Some(param_index), false) = (param_index, main_field.is_empty()) {
                map.entry(idx).or_default().push(LinkRecord {
                    param_index: param_index as u16,
                    main_field,
                });
            }
        }
    };
    for root in contents.record_tree_in(Dialect::Contents) {
        root.walk(&mut visit);
    }
    map
}

/// Map each subreport parameter index to its name, joining the parameter detail records (`0x007a`,
/// which state the engine parameter index and the `crobj://{…}` identity) to the subreport's
/// `PromptManager` (identity → parameter Name). A subreport link's `0x0106` record stores this
/// parameter index, so the map turns it into the LinkedParameterName.
pub(crate) fn subreport_param_index_names(
    contents: &RecordStream,
    prompt_xml: Option<&str>,
) -> HashMap<u16, String> {
    // GUID (`crobj://…`) → parameter Name, from the PromptManager CRMetaObjects XML.
    let mut guid_name: HashMap<String, String> = HashMap::new();
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
    let tree = contents.record_tree_in(Dialect::Contents);
    let mut map = HashMap::new();
    for n in nodes_where(&tree, |n| n.rtype == ft::PARAMETER_RECORD.rtype) {
        let row = row_of(n, logical, &ft::PARAMETER_RECORD);
        // The PromptManager is keyed by the identity's body, so the scheme is dropped here.
        let Some(guid) = row.text("id").strip_prefix("crobj://") else {
            continue;
        };
        if let Some(name) = guid_name.get(guid) {
            map.insert(row.u("parameter_index") as u16, name.clone());
        }
    }
    map
}

/// Map each subreport parameter to the subreport field it binds to, decoded from the subreport's
/// `0x0076` link-selection records. When a main-report field is linked into a subreport on a db
/// field, the engine stores the join as a formula record whose first operand is the bound subreport
/// field (`Command.some_field`, `@some_param`, …) and whose second operand is the `?<parameter>` it
/// is compared with (the auto-created link parameter). Returns `{parameter-name → field}`; a link
/// parameter absent from the map binds directly to itself (no db field — the SubreportFieldName then
/// equals the LinkedParameterName).
pub(crate) fn subreport_link_bindings(contents: &RecordStream) -> HashMap<String, String> {
    let logical = contents.logical_bytes();
    let tree = contents.record_tree_in(Dialect::Contents);
    let mut map = HashMap::new();
    // Position carries nothing here: a link selection is a formula wherever it sits in the tree.
    for n in nodes_where(&tree, |n| n.rtype == FORMULA) {
        // The link-selection formula's body holds each comparison as
        // `{sub.field} <op> {?Pm-<main>}`. A single selection can join several with `and`
        // (e.g. `{T.a} = {?Pm-T.x} and {T.b} = {?Pm-@y}`), so every comparison is parsed, not
        // just the first. The body (rather than the flat reference list) keeps the comparison
        // operator, which gates non-equality clauses (see `add_link_bindings`).
        let body = row_of(n, logical, &ft::FORMULA);
        let text = body.text("text");
        // A byte-presence pre-filter, not reference parsing — the reader is pure I/O and does
        // not depend on the formula engine, so the reference recognizer stays out of it.
        if text.contains("{?") {
            add_link_bindings(text, &mut map);
        }
    }
    map
}

/// Parse a selection-formula body for `{sub.field} <op> {?Pm-<main>}` link comparisons and record
/// `{parameter → bound field}` for each. The parameter key is the text inside `{?…}` (e.g.
/// `Pm-@param`), matching the `LinkedParameterName` lookup.
///
/// Only the engine's auto-generated *equality* form (`=`) binds unconditionally. A non-equality
/// clause (`<`, `<=`, `>`, `>=`) is accepted only when its left column matches the link's column
/// (Crystal names the link parameter `Pm-<main>` and compares the *same* column on the subreport
/// side). This rejects a user filter that merely re-uses the link parameter on a *different* column
/// (e.g. `{sub.other_col} >= {?Pm-sub.link_col}`), whose SubreportFieldName stays the parameter.
/// Matches how the engine itself resolves a subreport link's bound column.
fn add_link_bindings(body: &str, map: &mut HashMap<String, String>) {
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

/// Per-subreport metadata used to resolve its incoming links: the parameter-index → parameter-name
/// map (joins a link's `0x0106` parameter index to the LinkedParameterName) and the parameter-name →
/// bound-field map (the SubreportFieldName for a db-field link, from the `0x0076` link selection).
#[derive(Default)]
pub(crate) struct SubLinkMeta {
    pub(crate) index_names: HashMap<u16, String>,
    pub(crate) bindings: HashMap<String, String>,
}

/// Raise each subreport (`Subdocument N` storage) into a nested [`Report`](crate::model::Report). A
/// subreport has its own `Contents` / `QESession` / `PromptManager` streams under its storage,
/// decoded with the same pipeline as the main report.
pub(crate) fn build_subreports(
    container: &Container,
    current_values: &BTreeMap<u16, Vec<crate::model::ParameterValue>>,
) -> (
    Vec<crate::model::Subreport>,
    BTreeMap<u32, String>,
    BTreeMap<u32, SubLinkMeta>,
) {
    // Group every `Subdocument N/…` stream by its subdocument index.
    let mut groups: BTreeMap<u32, Vec<&LoadedStream>> = BTreeMap::new();
    for s in container.streams() {
        let first = crate::container::ole_components(&s.path).into_iter().next();
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
        // The saved current values are the report's, not the subreport's: they live in the single
        // top-level `ReportParametersStream` and are keyed by an index the whole report shares, so
        // a subreport's parameters resolve against the same map.
        let report = super::build_report(
            Some(&contents),
            qe.as_ref(),
            prompt.as_ref(),
            current_values,
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

/// The subreport's friendly name: the document name its own `Subdocument`'s report-root record
/// carries. Empty where the record states none.
fn subreport_name_from_contents(contents: &RecordStream) -> String {
    let logical = contents.logical_bytes();
    // A root of the tree, not a descendant: the report root is the outermost record of the stream.
    contents
        .record_tree_in(Dialect::Contents)
        .iter()
        .find(|root| root.rtype == ft::REPORT_ROOT.rtype)
        .map(|root| {
            row_of(root, logical, &ft::REPORT_ROOT)
                .text("document_name")
                .to_owned()
        })
        .unwrap_or_default()
}

/// Resolve everything about a subreport that is only knowable once the main report and its
/// subreports have both been raised: the placeholder object's name, each subreport's incoming
/// links, and the two link facts the placeholder object mirrors.
///
/// A subreport is decoded from its own storage, but a *link* is a fact of the pair — the main
/// report holds the `0x0106` records, the subreport holds the parameter names and the bindings they
/// resolve against — so neither half can state it alone. `subdoc_names` keys share order with
/// `report.subreports`, and every lookup here is keyed on that subdocument index.
pub(crate) fn attach(
    report: &mut crate::model::Report,
    contents: Option<&RecordStream>,
    subdoc_names: &BTreeMap<u32, String>,
    sub_link_meta: &BTreeMap<u32, SubLinkMeta>,
) {
    for obj in report.objects_mut() {
        if let crate::model::ReportObjectKind::Subreport(sr) = &mut obj.kind {
            if let Some(name) = subdoc_names.get(&sr.subdoc_index) {
                sr.subreport_name = name.clone();
            }
        }
    }
    if let Some(c) = contents {
        let links = subreport_links(c);
        for (idx, sub) in subdoc_names.keys().zip(report.subreports.iter_mut()) {
            let Some(entries) = links.get(idx) else {
                continue;
            };
            let meta = sub_link_meta.get(idx);
            sub.links = entries
                .iter()
                .map(|rec| {
                    let param = meta
                        .and_then(|m| m.index_names.get(&rec.param_index))
                        .cloned();
                    // SubreportFieldName: the `0x0076` link-selection binding, else empty (the
                    // engine then reports the link parameter itself). The `0x0106` record's own
                    // trailing handle is no help here — it re-states the *main* report's field,
                    // not the subreport's.
                    let subreport_field = param
                        .as_ref()
                        .and_then(|p| meta.and_then(|m| m.bindings.get(p)).cloned())
                        .unwrap_or_default();
                    crate::model::SubreportLink {
                        main_report_field: rec.main_field.clone(),
                        subreport_field,
                        linked_parameter: param,
                    }
                })
                .collect();
        }
    }
    // The placeholder object's `IsImported` / `SubreportLinks` are facts of the subreport it names,
    // mirrored onto the object like `subreport_name` above. `IsImported` is a non-empty reimport
    // `source_path` (`0x0142`); the links are the copy just resolved. `EnableReimport` stays default
    // (`false`) — unpinned.
    let obj_facts: HashMap<u32, (bool, Vec<crate::model::SubreportLink>)> = subdoc_names
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
