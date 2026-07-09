//! Human-readable text rendering of the dump: the per-record annotated view (heading, meta,
//! path/children, hex, LP-strings, scalar-probe grid), the stream's record-type index, and the raw
//! `--offset`/`--len` span escape hatch.

use std::fmt::Write as _;

use rpt::Rpt;

use crate::util::{paint, BOLD};

use super::collect::select_streams;
use super::parse::{anchor_string_index, hexdump_lines, lp_strings, probe_cap, type_label};
use super::{DumpMatch, DumpOpts};

/// Render one match as annotated text: heading, meta line, path/children, the hex dump, the
/// length-prefixed strings, and the scalar-probe grid.
pub(super) fn render_match(
    out: &mut String,
    m: &DumpMatch,
    idx: usize,
    total: usize,
    opts: &DumpOpts,
) {
    let bytes = if opts.whole { &m.whole } else { &m.leaf };
    let view = if opts.whole {
        "whole span (header+children, masked)"
    } else {
        "own leaf (demasked)"
    };
    let heading = format!(
        "── {} · {} · subtype=0x{:02x} · #{idx}/{total} ──",
        m.stream,
        type_label(m.rtype),
        m.subtype
    );
    let _ = writeln!(out, "{}", paint(opts.color, BOLD, &heading));
    let _ = writeln!(
        out,
        "   logical offset 0x{:x} · content [0x{:x}..0x{:x}) · mask 0x{:02x} · depth {}",
        m.offset, m.content_start, m.content_end, m.mask, m.depth
    );
    if !m.path.is_empty() {
        let path: Vec<String> = m.path.iter().map(|&t| type_label(t)).collect();
        let _ = writeln!(out, "   path: {}", path.join(" › "));
    }
    if !m.children.is_empty() {
        let kids: Vec<String> = m.children.iter().map(|&t| type_label(t)).collect();
        let _ = writeln!(out, "   children: {}", kids.join(", "));
    }
    let _ = writeln!(out, "   {view}: {} bytes", bytes.len());
    if bytes.is_empty() {
        let _ = writeln!(out, "   (empty)");
        return;
    }
    let _ = writeln!(out, "   hex:");
    for line in hexdump_lines(bytes) {
        let _ = writeln!(out, "  {line}");
    }
    let strings = lp_strings(bytes);
    if !strings.is_empty() {
        let _ = writeln!(out, "   length-prefixed strings (read_lp_string):");
        for (off, text, consumed) in &strings {
            let _ = writeln!(
                out,
                "     @0x{off:04x}  {:?}  (len prefix 0x{:08x}, {consumed}B)",
                text,
                consumed - 4
            );
        }
    }
    // The `used` anchor: the end of the anchoring LP-string (the `--anchor-string` marker match, or
    // the last string). The probe grid then annotates each offset as `used±N`, so a trailing tail
    // can be read relative to a field-ref end — its distance from that end is stable even when the
    // field name's length shifts the absolute offsets.
    let anchor = anchor_string_index(&strings, opts.anchor_string.as_deref())
        .map(|k| strings[k].0 + strings[k].2);
    if let Some(a) = anchor {
        let _ = writeln!(
            out,
            "   used anchor @0x{a:04x} (probe offsets annotated used±N)"
        );
    }
    let cap = probe_cap(opts.probe.as_deref(), bytes.len());
    if cap > 0 {
        let _ = writeln!(
            out,
            "   scalar probe (off: u16be u16le · u32be u32le)  [Cursor / u16_be / u32_be]:"
        );
        for off in 0..cap {
            let u16be = bytes
                .get(off..off + 2)
                .map(|s| format!("0x{:04x}", u16::from_be_bytes([s[0], s[1]])))
                .unwrap_or_else(|| "     -".into());
            let u16le = bytes
                .get(off..off + 2)
                .map(|s| format!("0x{:04x}", u16::from_le_bytes([s[0], s[1]])))
                .unwrap_or_else(|| "     -".into());
            let u32be = bytes
                .get(off..off + 4)
                .map(|s| format!("0x{:08x}", u32::from_be_bytes([s[0], s[1], s[2], s[3]])))
                .unwrap_or_else(|| "         -".into());
            let u32le = bytes
                .get(off..off + 4)
                .map(|s| format!("0x{:08x}", u32::from_le_bytes([s[0], s[1], s[2], s[3]])))
                .unwrap_or_else(|| "         -".into());
            let rel = anchor.map_or_else(String::new, |a| {
                format!("  (used{:+})", off as isize - a as isize)
            });
            let _ = writeln!(
                out,
                "     0x{off:04x}: {u16be} {u16le} · {u32be} {u32le}{rel}"
            );
        }
    }
    let _ = writeln!(out);
}

/// The record-type index of the selected streams: each type, its count, and its name — the table
/// of contents an agent reads to decide what to dump.
pub(super) fn dump_type_index(out: &mut String, rpt: &Rpt, opts: &DumpOpts) {
    for (name, stream, is_qe) in select_streams(rpt, opts.stream.as_deref()) {
        let tree = if is_qe {
            stream.qe_record_tree()
        } else {
            stream.record_tree()
        };
        let mut counts: std::collections::BTreeMap<u16, usize> = Default::default();
        for root in &tree {
            root.walk(&mut |n| *counts.entry(n.rtype).or_default() += 1);
        }
        if counts.is_empty() {
            continue;
        }
        let mut rows: Vec<(u16, usize)> = counts.into_iter().collect();
        rows.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
        let _ = writeln!(out, "{name}: {} record type(s)", rows.len());
        for (rtype, count) in rows {
            let _ = writeln!(out, "   {:<28} ×{count}", type_label(rtype));
        }
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "select a type to dump its bytes: --type 0xNNNN");
}

/// Dump a raw `[offset, offset+len)` span of each file's first selected stream, verbatim.
pub(super) fn dump_raw_span(
    files: &[String],
    opts: &DumpOpts,
    off: usize,
    len: usize,
) -> rpt::Result<()> {
    for file in files {
        let rpt = Rpt::open(file)?;
        let streams = select_streams(&rpt, opts.stream.as_deref());
        let Some((name, stream, _)) = streams.first() else {
            eprintln!("{file}: no stream matched --stream");
            continue;
        };
        // Opaque streams (saved-data batches, embeddings, …) are never decoded into a logical
        // record buffer, so `logical_bytes()` is empty; fall back to the raw on-disk bytes so the
        // raw-span escape hatch can reach them too.
        let logical = stream.logical_bytes();
        let (bytes_src, kind) = if logical.is_empty() {
            (stream.raw_bytes(), "raw on-disk")
        } else {
            (logical, "logical/masked")
        };
        let end = (off + len).min(bytes_src.len());
        let bytes = bytes_src.get(off..end).unwrap_or(&[]);
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{file} · {name} · [0x{off:x}..0x{end:x}] ({} bytes, {kind}):",
            bytes.len()
        );
        for line in hexdump_lines(bytes) {
            let _ = writeln!(out, "{line}");
        }
        print!("{out}");
    }
    Ok(())
}
