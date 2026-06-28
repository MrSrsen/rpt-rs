//! Generic substrate projection — the raw record DOM, inventory, and SummaryInformation.

use super::*;

/// Project the record tree into the type-strict DOM: dispatch each record to its domain DTO
/// [`Node`], falling through to [`Node::Unknown`] for types not yet modelled.
pub(super) fn raise_dom(stream: &RecordStream) -> Vec<Node> {
    let logical = stream.logical_bytes();
    stream
        .record_tree()
        .iter()
        .map(|n| build_node(n, logical))
        .collect()
}

pub(super) fn build_node(node: &RecordNode, logical: &[u8]) -> Node {
    match node.rtype {
        FIELD_DEF => {
            if let Some(field) = raise_field(node, logical) {
                return Node::FieldDef(field);
            }
            unknown_node(node, logical)
        }
        _ => unknown_node(node, logical),
    }
}

pub(super) fn unknown_node(node: &RecordNode, logical: &[u8]) -> Node {
    Node::Unknown(Unknown {
        rtype: node.rtype,
        subtype: node.subtype,
        values: decode_leaf(&node.leaf_bytes(logical)),
        children: node
            .children
            .iter()
            .map(|c| build_node(c, logical))
            .collect(),
    })
}

/// Decode a record's demasked leaf bytes into a sequence of [`Value`]s: length-prefixed
/// printable strings become [`Value::Text`]; the remaining bytes are kept verbatim as
/// [`Value::Bytes`] so the projection is lossless and the exporter sees everything.
pub(super) fn decode_leaf(bytes: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut raw: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if let Some((text, consumed)) = lp_string_at(bytes, i) {
            if !raw.is_empty() {
                out.push(Value::Bytes(std::mem::take(&mut raw)));
            }
            out.push(Value::Text(text));
            i += consumed;
        } else {
            raw.push(bytes[i]);
            i += 1;
        }
    }
    if !raw.is_empty() {
        out.push(Value::Bytes(raw));
    }
    out
}

pub(super) fn raise_summary(si: &SummaryInformation) -> SummaryInfo {
    SummaryInfo {
        title: si.title.clone().unwrap_or_default(),
        subject: si.subject.clone().unwrap_or_default(),
        author: si.author.clone().unwrap_or_default(),
        keywords: si.keywords.clone().unwrap_or_default(),
        comments: si.comments.clone().unwrap_or_default(),
        ..Default::default()
    }
}

/// Build the typed record inventory: count every record in the **full nested tree** (not just
/// the top-level tiling) per type, sorted by descending frequency then type, attaching the
/// symbolic name where the type is identified.
pub(super) fn inventory(stream: &RecordStream) -> Vec<RecordTypeCount> {
    let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
    for root in stream.record_tree() {
        root.walk(&mut |node| {
            *counts.entry(node.rtype).or_default() += 1;
        });
    }
    let mut out: Vec<RecordTypeCount> = counts
        .into_iter()
        .map(|(tag, count)| RecordTypeCount {
            tag,
            name: RecordTag(tag).name(),
            count,
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.tag.cmp(&b.tag)));
    out
}
