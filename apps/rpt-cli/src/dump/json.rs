//! The `--json` machine output for the per-record dump: each match's metadata, hex, LP-strings,
//! and — as in the text view — either the record type's field-table reading or the scalar-probe
//! grid, serialized as JSON.

use serde::Serialize;

use rpt_reader::fields::RecordFields;
use rpt_reader::raw::Dialect;

use super::fields::range_pair;
use super::parse::{lp_strings, probe_cap, type_label};
use super::{DumpMatch, DumpOpts};

#[derive(Serialize)]
struct LpStringJson {
    offset: usize,
    text: String,
    consumed: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScalarJson {
    offset: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    u16_be: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    u16_le: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    u32_be: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    u32_le: Option<u32>,
}

/// One field of a record's field-table reading. The value lands in the key its wire type calls
/// for, so a consumer reads a number as a number.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldJson {
    field: String,
    kind: &'static str,
    /// Absolute span in the decoded stream, `[start, end)`.
    range: [usize; 2],
    /// The same bytes' offset in the joined-runs buffer (the `hex` above); absent for a field that
    /// occupies no field bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    joined: Option<[usize; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uint: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    int: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    float: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    child: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rows: Option<usize>,
}

/// A record's field-table reading: the fields, and whether the table accounted for the record.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldTableJson {
    table: &'static str,
    content: [usize; 2],
    child_count: usize,
    exact: bool,
    complete: bool,
    /// The record's schema is newer than any layout known for its type, so the table was never
    /// applied and the counts below describe an empty reading.
    schema_too_new: bool,
    unread: usize,
    undeclared_children: usize,
    ended: bool,
    blocked_by_child: bool,
    child_mismatch: bool,
    fields: Vec<FieldJson>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DumpMatchJson {
    stream: String,
    #[serde(rename = "type")]
    type_name: String,
    tag: String,
    schema: String,
    offset: usize,
    content_start: usize,
    content_end: usize,
    len: usize,
    mask: String,
    depth: usize,
    path: Vec<String>,
    children: Vec<String>,
    view: &'static str,
    hex: String,
    strings: Vec<LpStringJson>,
    /// The record type's field-table reading, when it has a table.
    #[serde(skip_serializing_if = "Option::is_none")]
    field_table: Option<FieldTableJson>,
    /// The scalar-probe grid, empty when the field-table view replaced it.
    scalars: Vec<ScalarJson>,
}

/// Project one field-table reading into its JSON form.
fn field_table_json(r: &RecordFields, dialect: Dialect) -> FieldTableJson {
    use rpt_reader::fields::FieldValue;
    FieldTableJson {
        table: r.table,
        content: range_pair(r.content),
        child_count: r.child_count,
        exact: r.exact(),
        complete: r.complete,
        schema_too_new: r.schema_too_new,
        unread: r.unread,
        undeclared_children: r.undeclared_children,
        ended: r.stop.ended,
        blocked_by_child: r.stop.blocked_by_child,
        child_mismatch: r.stop.child_mismatch,
        fields: r
            .fields
            .iter()
            .map(|f| FieldJson {
                field: f.path.clone(),
                kind: f.kind.label(),
                range: range_pair(f.span),
                joined: (!f.joined.is_empty()).then(|| range_pair(f.joined)),
                uint: match &f.value {
                    FieldValue::Uint(v) => Some(*v),
                    _ => None,
                },
                int: match &f.value {
                    FieldValue::Int(v) => Some(*v),
                    _ => None,
                },
                float: match &f.value {
                    FieldValue::Float(v) => Some(*v),
                    _ => None,
                },
                text: match &f.value {
                    FieldValue::Text(t) => Some(t.clone()),
                    FieldValue::FieldRef { name, .. } => Some(name.clone()),
                    _ => None,
                },
                bytes: match &f.value {
                    FieldValue::Bytes(b) => {
                        Some(b.iter().map(|x| format!("{x:02x}")).collect::<String>())
                    }
                    _ => None,
                },
                child: match &f.value {
                    FieldValue::Child { rtype, .. } => Some(type_label(*rtype, dialect)),
                    _ => None,
                },
                rows: match &f.value {
                    FieldValue::Repeat(n) => Some(*n),
                    _ => None,
                },
            })
            .collect(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DumpFileJson {
    file: String,
    view: &'static str,
    matches: Vec<DumpMatchJson>,
}

/// Build the annotated JSON record for one match.
fn match_json(m: &DumpMatch, opts: &DumpOpts) -> DumpMatchJson {
    let bytes = if opts.whole { &m.whole } else { &m.joined_runs };
    // The field-table view replaces the probe grid for the types that have one, exactly as in the
    // text view, so the two outputs answer the same question.
    let field_table = m
        .table
        .as_ref()
        .filter(|_| !opts.grid)
        .map(|t| field_table_json(t, m.dialect));
    let cap = if field_table.is_some() {
        0
    } else {
        probe_cap(opts.probe.as_deref(), bytes.len())
    };
    let scalars = (0..cap)
        .map(|off| ScalarJson {
            offset: off,
            u16_be: bytes
                .get(off..off + 2)
                .map(|s| u16::from_be_bytes([s[0], s[1]])),
            u16_le: bytes
                .get(off..off + 2)
                .map(|s| u16::from_le_bytes([s[0], s[1]])),
            u32_be: bytes
                .get(off..off + 4)
                .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])),
            u32_le: bytes
                .get(off..off + 4)
                .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])),
        })
        .collect();
    DumpMatchJson {
        stream: m.stream.clone(),
        type_name: type_label(m.rtype, m.dialect),
        tag: format!("0x{:04x}", m.rtype),
        schema: format!("0x{:04x}", m.schema),
        offset: m.offset,
        content_start: m.content_start,
        content_end: m.content_end,
        len: bytes.len(),
        mask: format!("0x{:02x}", m.mask),
        depth: m.depth,
        path: m.path.iter().map(|&t| type_label(t, m.dialect)).collect(),
        children: m
            .children
            .iter()
            .map(|&t| type_label(t, m.dialect))
            .collect(),
        view: if opts.whole { "whole" } else { "joined" },
        hex: bytes.iter().map(|b| format!("{b:02x}")).collect(),
        strings: lp_strings(bytes)
            .into_iter()
            .map(|s| LpStringJson {
                offset: s.offset,
                text: s.text,
                consumed: s.len,
            })
            .collect(),
        field_table,
        scalars,
    }
}

/// Build the per-file JSON dump for every file's matches (the `--json` payload).
pub(super) fn build_dump_json(
    per_file: &[(String, Vec<DumpMatch>)],
    opts: &DumpOpts,
) -> Vec<DumpFileJson> {
    per_file
        .iter()
        .map(|(file, matches)| DumpFileJson {
            file: file.clone(),
            view: if opts.whole { "whole" } else { "joined" },
            matches: matches.iter().map(|m| match_json(m, opts)).collect(),
        })
        .collect()
}
