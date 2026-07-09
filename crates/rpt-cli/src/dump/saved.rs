//! The saved-data batch substrate view (`--saved`) — the encrypted-batch layer the record `dump`
//! cannot reach: the decoded catalog schema, the batch directory, and, per batch, the derived
//! decrypt IV, whether it inflates, and (on failure) an IV search that brute-forces the metadata.

use std::fmt::Write as _;

use rpt::Rpt;

use crate::util::{paint, BOLD};

use super::parse::{hexdump_lines, probe_cap};
use super::DumpOpts;

/// Inspect the saved-data batch substrate for reverse-engineering — the encrypted-batch layer the
/// record `dump` cannot reach. Surfaces the decoded catalog schema (each field's record offset +
/// memo flag), the batch directory (per batch: role, count, on-disk item size), and, per batch, the
/// decrypt IV the decoder derives, whether it yields a zlib header, and — on success — the inflated
/// first record with a scalar probe, or on failure the raw ciphertext head plus an IV search that
/// brute-forces the `(batch_size, item_count, item_size, seq)` metadata that inflates it.
pub(super) fn dump_saved(files: &[String], opts: &DumpOpts) -> rpt::Result<()> {
    for file in files {
        let rpt = Rpt::open(file)?;
        let Some(insp) = rpt.saved_batch_inspection() else {
            println!("{file}: no saved-data directory");
            continue;
        };
        let mut out = String::new();
        let heading = format!("── {file} · saved-data batch substrate ──");
        let _ = writeln!(out, "{}", paint(opts.color, BOLD, &heading));
        let _ = writeln!(
            out,
            "   SavedRecordsStream {} B · MemoValuesStream {} B · {} memo column(s)",
            insp.srs_len, insp.memo_len, insp.memo_cols
        );

        let _ = writeln!(out, "   schema ({} field(s)):", insp.schema.len());
        for (i, f) in insp.schema.iter().enumerate() {
            let _ = writeln!(
                out,
                "     [{i}] rec_offset={:<5} memo={} {}",
                f.rec_offset, f.is_memo, f.name
            );
        }

        let _ = writeln!(out, "   batches ({}):", insp.batches.len());
        for (i, b) in insp.batches.iter().enumerate() {
            let kind = match b.kind {
                rpt::SavedBatchKind::Index => "index",
                rpt::SavedBatchKind::Descriptor => "descriptor",
                rpt::SavedBatchKind::MemoValue => "memo-value",
            };
            let stream = if b.in_memo_stream {
                "MemoValuesStream"
            } else {
                "SavedRecordsStream"
            };
            let _ = writeln!(
                out,
                "   ── batch #{i} · {kind} · dir(count={}, item_size={}) · {stream}@0x{:x}",
                b.dir_count, b.dir_item_size, b.cursor
            );
            let _ = writeln!(
                out,
                "      IV = (batch_size={}, item_count={}, item_size={}, seq={})",
                b.iv_batch_size, b.iv_item_count, b.iv_item_size, b.seq
            );
            let iv_hex: String = b.iv.iter().map(|x| format!("{x:02x}")).collect();
            let _ = writeln!(out, "      IV bytes: {iv_hex}");
            if !b.dir_leaf.is_empty() {
                let _ = writeln!(out, "      directory leaf 0x6d ({} B):", b.dir_leaf.len());
                for line in hexdump_lines(&b.dir_leaf) {
                    let _ = writeln!(out, "     {line}");
                }
                // A packed index batch carries a per-column table after byte 16: a u16 entry count
                // then `3 × string_columns` big-endian u32s; every third is the on-disk offset of the
                // field after a compacted string (the per-column slot boundary).
                let l = &b.dir_leaf;
                if l.len() >= 18 {
                    let be32 = |o: usize| {
                        u32::from_be_bytes([l[o], l[o + 1], l[o + 2], l[o + 3]]) as usize
                    };
                    let n_entries = u16::from_be_bytes([l[16], l[17]]) as usize;
                    if n_entries > 0 {
                        let bounds: Vec<usize> = (0..n_entries / 3)
                            .filter_map(|k| {
                                let o = 18 + (3 * k + 2) * 4;
                                (o + 4 <= l.len()).then(|| be32(o))
                            })
                            .collect();
                        let _ = writeln!(
                            out,
                            "      column table: {n_entries} entries ({} string col(s)); on-disk slot boundaries {bounds:?}",
                            n_entries / 3
                        );
                    }
                }
            }
            if b.decrypts_zlib {
                let _ = writeln!(
                    out,
                    "      decrypts to zlib ✓  inflated {} B · consumed {} B",
                    b.inflated_len.unwrap_or(0),
                    b.consumed.unwrap_or(0),
                );
                if !b.first_record.is_empty() {
                    let _ = writeln!(out, "      first record ({} B):", b.first_record.len());
                    for line in hexdump_lines(&b.first_record) {
                        let _ = writeln!(out, "     {line}");
                    }
                    let cap = probe_cap(opts.probe.as_deref(), b.first_record.len());
                    if cap > 0 {
                        let _ = writeln!(out, "      scalar probe (off: u16le · u32le):");
                        for off in 0..cap {
                            let u16le = b
                                .first_record
                                .get(off..off + 2)
                                .map(|s| format!("0x{:04x}", u16::from_le_bytes([s[0], s[1]])))
                                .unwrap_or_default();
                            let u32le = b
                                .first_record
                                .get(off..off + 4)
                                .map(|s| {
                                    format!(
                                        "0x{:08x}",
                                        u32::from_le_bytes([s[0], s[1], s[2], s[3]])
                                    )
                                })
                                .unwrap_or_default();
                            let _ = writeln!(out, "        0x{off:04x}: {u16le} · {u32le}");
                        }
                    }
                }
            } else {
                let _ = writeln!(out, "      does NOT decrypt to a zlib header ✗");
                let ct: String = b.ct_head.iter().map(|x| format!("{x:02x}")).collect();
                let _ = writeln!(out, "      ciphertext head: {ct}");
                // Brute-force the IV metadata that inflates this batch: pin batch_size to the fixed
                // index cap and sweep item_count near the directory count and item_size widely (the
                // in-memory/persistent width is often larger than the on-disk item_size).
                let batch_sizes = [1000u32, b.dir_count];
                let lo = b.dir_count.saturating_sub(1);
                let item_counts: Vec<u32> = (lo..=b.dir_count + 1).collect();
                let item_sizes: Vec<u32> = (1..=4096).collect();
                let seqs = [0u32, 1, 2, 3];
                let hits = rpt.saved_iv_search(
                    b.in_memo_stream,
                    b.cursor,
                    &batch_sizes,
                    &item_counts,
                    &item_sizes,
                    &seqs,
                    8,
                );
                if hits.is_empty() {
                    let _ = writeln!(out, "      IV search: no (batch_size,item_count,item_size,seq) in range inflated it");
                } else {
                    let _ = writeln!(out, "      IV search hits:");
                    for h in &hits {
                        let _ = writeln!(
                            out,
                            "        (batch_size={}, item_count={}, item_size={}, seq={}) → inflated {} B",
                            h.batch_size, h.item_count, h.item_size, h.seq, h.inflated_len
                        );
                    }
                }
            }
        }
        let _ = writeln!(out);
        print!("{out}");
    }
    Ok(())
}
