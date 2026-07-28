//! The `--json` machine output for the per-record dump: each match's metadata, hex, LP-strings,
//! and scalar-probe grid serialized as JSON.

use serde::Serialize;

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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DumpMatchJson {
    stream: String,
    #[serde(rename = "type")]
    type_name: String,
    tag: String,
    subtype: u16,
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
    scalars: Vec<ScalarJson>,
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
    let bytes = if opts.whole { &m.whole } else { &m.leaf };
    let cap = probe_cap(opts.probe.as_deref(), bytes.len());
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
        type_name: type_label(m.rtype),
        tag: format!("0x{:04x}", m.rtype),
        subtype: m.subtype,
        offset: m.offset,
        content_start: m.content_start,
        content_end: m.content_end,
        len: bytes.len(),
        mask: format!("0x{:02x}", m.mask),
        depth: m.depth,
        path: m.path.iter().map(|&t| type_label(t)).collect(),
        children: m.children.iter().map(|&t| type_label(t)).collect(),
        view: if opts.whole { "whole" } else { "leaf" },
        hex: bytes.iter().map(|b| format!("{b:02x}")).collect(),
        strings: lp_strings(bytes)
            .into_iter()
            .map(|(offset, text, consumed)| LpStringJson {
                offset,
                text,
                consumed,
            })
            .collect(),
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
            view: if opts.whole { "whole" } else { "leaf" },
            matches: matches.iter().map(|m| match_json(m, opts)).collect(),
        })
        .collect()
}
