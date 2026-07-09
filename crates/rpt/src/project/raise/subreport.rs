//! Semantic decoding of subreport links — the incoming-link metadata a subreport carries.
//!
//! The main report stores each subreport link in an `0x0106` record following the subreport's
//! `0xa3` object; a subreport carries its own parameter-index map and link-selection bindings.
//! These decoders raise those records into the pieces of a [`crate::model::SubreportLink`], which
//! the [`crate::io`] facade assembles after both the main report and its subreports are raised.

use crate::records::rtype::{FORMULA, PARAM_RECORD, SUBREPORT_LINK, SUBREPORT_OBJECT};
use crate::records::RecordStream;

/// One decoded `0x0106` subreport-link record: the subreport-parameter index the main field feeds,
/// the `MainReportFieldName`, and the **SubreportFieldName handle** — the `(kind, index)` pair
/// resolved against the subreport's field pool.
pub(crate) struct LinkRecord {
    pub(crate) param_index: u16,
    pub(crate) main_field: String,
    /// `(field-kind, pool-index)` from the link record's trailing descriptor: `kind` selects the
    /// subreport field pool (`0` = database field, `1` = formula), `index` the entry within it.
    /// `None` when the link carries no distinct subreport field (the short trailing form, or kind
    /// `0xffff`), in which case `SubreportFieldName` falls back to the link parameter.
    pub(crate) sf_handle: Option<(u16, u16)>,
}

/// Each subreport link, grouped by subdocument index. In the main report's `Contents`, every
/// subreport object (`0xa3`) is followed by one `0x0106` link record per link. The leaf is
/// `[u16 linked-parameter-index][u32-BE namelen][main-report field name…NUL][trailing descriptor]`:
/// the leading `u16` is the **subreport parameter index** the main field feeds (the join key that
/// pairs a link to the auto-created subreport parameter), and the length-prefixed name is the
/// `MainReportFieldName`. The **trailing descriptor** (when 8+ bytes:
/// `[main-field-kind/index ×4][u16-BE SF-kind][u16-BE SF-index]`) carries the SubreportFieldName
/// handle. The engine counts one `Field.UseCount` per link.
pub(crate) fn subreport_links(
    contents: &RecordStream,
) -> std::collections::BTreeMap<u32, Vec<LinkRecord>> {
    use crate::codec::RecordNode;
    use std::collections::BTreeMap;
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
        use crate::bytes::u16_be;
        if n.rtype == SUBREPORT_OBJECT {
            *current = crate::bytes::u32_be(&n.leaf_bytes(logical), 0);
        } else if n.rtype == SUBREPORT_LINK {
            if let Some(idx) = *current {
                let lb = n.leaf_bytes(logical);
                let param_index = u16_be(&lb, 0);
                if let (Some(param_index), Some(len)) = (
                    param_index,
                    crate::bytes::u32_be(&lb, 2).map(|n| n as usize),
                ) {
                    if let Some(raw) = lb.get(6..6 + len) {
                        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                        if end > 0 {
                            // The SubreportFieldName handle lives in the trailing descriptor after
                            // the name: SF-kind = BE u16 at `[4..6]`, SF-index = BE u16 at `[6..8]`.
                            // Absent (short trailing) or `0xffff` → no distinct subreport field.
                            let trailing = &lb[6 + len..];
                            let sf_handle = if trailing.len() >= 8 {
                                let kind = u16_be(trailing, 4).unwrap_or(0);
                                let index = u16_be(trailing, 6).unwrap_or(0);
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
pub(crate) fn resolve_sf_handle(
    report: &crate::model::Report,
    kind: u16,
    index: u16,
) -> Option<String> {
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

/// Map each subreport parameter index to its name, joining the parameter detail records (`0x007a`,
/// whose leaf begins with the `u16` engine parameter index and embeds the `crobj://{…}` GUID) to the
/// subreport's `PromptManager` (GUID → parameter Name). A subreport link's `0x0106` record stores
/// this parameter index, so the map turns it into the LinkedParameterName.
pub(crate) fn subreport_param_index_names(
    contents: &RecordStream,
    prompt_xml: Option<&str>,
) -> std::collections::HashMap<u16, String> {
    use crate::codec::RecordNode;
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
        let Some(index) = crate::bytes::u16_be(&leaf, 0) else {
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
/// field (`Command.some_field`, `@some_param`, …) and whose second operand is the `?<parameter>` it
/// is compared with (the auto-created link parameter). Returns `{parameter-name → field}`; a link
/// parameter absent from the map binds directly to itself (no db field — the SubreportFieldName then
/// equals the LinkedParameterName).
pub(crate) fn subreport_link_bindings(
    contents: &RecordStream,
) -> std::collections::HashMap<String, String> {
    use crate::codec::RecordNode;
    let logical = contents.logical_bytes();
    let mut map = std::collections::HashMap::new();
    fn visit(n: &RecordNode, logical: &[u8], map: &mut std::collections::HashMap<String, String>) {
        if n.rtype == FORMULA {
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
/// `Pm-@param`), matching the `LinkedParameterName` lookup.
///
/// Only the engine's auto-generated *equality* form (`=`) binds unconditionally. A non-equality
/// clause (`<`, `<=`, `>`, `>=`) is accepted only when its left column matches the link's column
/// (Crystal names the link parameter `Pm-<main>` and compares the *same* column on the subreport
/// side). This rejects a user filter that merely re-uses the link parameter on a *different* column
/// (e.g. `{sub.other_col} >= {?Pm-sub.link_col}`), whose SubreportFieldName stays the parameter.
/// Mirrors the `Field.UseCount` rule the XML exporter applies for subreport-link fields.
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

/// A deliberate printable-filtered variant of the shared LP-string readers: it keeps only runs of
/// printable ASCII (plus tab/CR/LF) and skips filler between them, which the lossy
/// `read_be_lp_string_lossy` would not reject — so it is not collapsed onto that primitive.
///
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
