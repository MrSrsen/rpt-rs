//! Corpus invariants of the TSLV record header.
//!
//! The header's schema word is `dialect << 8 | version`. Both halves are asserted here because a
//! byte-swapped read is self-consistent — it parses the same records and only misreports the
//! value — so nothing but a claim about which half is which can catch it.
//!
//! Every assertion here is of the form "no record violates X", which an empty sweep satisfies, so
//! each test also asserts how much it actually looked at.

use std::path::PathBuf;

use rpt_reader::raw::RecordNode;
use rpt_reader::{Rpt, StreamId};

/// The `Contents` dialect marker: the high byte of every `Contents` record's schema word.
const CONTENTS_DIALECT: u16 = 0x07;

/// Every report of the corpus — discovered, not named, so a newly added tree is swept without
/// anyone remembering this file exists.
fn corpus() -> Vec<PathBuf> {
    rpt_test_support::corpus_reports()
}

/// Every `Contents` record of every fixture, as `(rtype, schema)`.
///
/// # Panics
///
/// If any corpus report contributes no records. The assertions below are all of the form "no record
/// violates X", which an empty or partial sweep satisfies, so the sweep's own completeness is
/// asserted here rather than left to each caller.
fn contents_records() -> Vec<(u16, u16)> {
    let mut out = Vec::new();
    let mut missing = Vec::new();
    let files = corpus();
    for path in &files {
        let before = out.len();
        let Ok(rpt) = Rpt::open(path) else {
            missing.push(format!("  {}: does not open", path.display()));
            continue;
        };
        let Some(stream) = rpt.stream(&StreamId::Contents) else {
            missing.push(format!("  {}: no Contents stream", path.display()));
            continue;
        };
        fn walk(n: &RecordNode, out: &mut Vec<(u16, u16)>) {
            out.push((n.rtype, n.schema));
            for c in &n.children {
                walk(c, out);
            }
        }
        for root in &stream.record_tree() {
            walk(root, &mut out);
        }
        if out.len() == before {
            missing.push(format!("  {}: contributed no records", path.display()));
        }
    }
    assert!(
        missing.is_empty(),
        "{} of {} corpus report(s) contributed nothing to the record sweep:\n{}",
        missing.len(),
        files.len(),
        missing.join("\n")
    );
    out
}

/// The schema word splits into a dialect marker and a version, not into two arbitrary bytes: in a
/// `Contents` stream the high byte is always the dialect and the low byte is a small version
/// counter. Swap the read and the halves swap with it — the dialect becomes the version.
#[test]
fn contents_schema_is_a_dialect_marker_over_a_version() {
    let records = contents_records();

    let foreign: Vec<(u16, u16)> = records
        .iter()
        .copied()
        .filter(|(_, schema)| schema >> 8 != CONTENTS_DIALECT)
        .collect();
    assert!(
        foreign.is_empty(),
        "{} Contents record(s) carry a non-Contents dialect marker: {:x?}",
        foreign.len(),
        &foreign[..foreign.len().min(8)]
    );

    // The low half is a per-record-type revision counter, so it stays small; the high half never
    // could, which is what makes this a byte-order assertion and not a tautology.
    let wild: Vec<(u16, u16)> = records
        .iter()
        .copied()
        .filter(|(_, schema)| schema & 0xff > 0x0f)
        .collect();
    assert!(
        wild.is_empty(),
        "{} Contents record(s) carry an implausible schema version: {:x?}",
        wild.len(),
        &wild[..wild.len().min(8)]
    );
    assert_sweep_covers_the_corpus(records.len(), "Contents record");
}

/// Assert a sweep looked at enough of the corpus to mean something. Every claim in this file is
/// "no record violates X", which a small sweep satisfies, so the count is asserted alongside it.
/// The floors sit under what the committed fixtures alone reach.
fn assert_sweep_covers_the_corpus(seen: usize, unit: &str) {
    eprintln!("[record header] {seen} {unit}(s)");
    assert!(
        seen > 10_000,
        "only {seen} {unit}(s) swept — the corpus walk is not covering the corpus"
    );
}

/// The reader accepts a header with no length field because the engine writes empty records in
/// that form, both between top-level records and inside them. Rejecting them is silent — their
/// bytes become field data of whatever record contains them, and every field the decoder reads
/// past that point shifts by the four bytes of framing.
#[test]
fn empty_records_are_framed_without_a_length_field() {
    let files = corpus();
    let mut empty = 0usize;
    let mut carriers = 0usize;
    for path in &files {
        let Ok(rpt) = Rpt::open(path) else { continue };
        let Some(stream) = rpt.stream(&StreamId::Contents) else {
            continue;
        };
        let logical = stream.logical_bytes();
        let mut here = 0usize;
        fn walk(n: &RecordNode, logical: &[u8], here: &mut usize) {
            // The length field's width lives in the flag byte's top two bits; zero means the record
            // is empty and the header stops after the schema word.
            let header_mask = n.mask ^ (n.rtype as u8);
            let flag = logical.get(n.offset).map(|b| b ^ header_mask).unwrap_or(0);
            if flag & 0b1100_0000 == 0 {
                *here += 1;
                assert_eq!(
                    n.content_start, n.content_end,
                    "a record with no length field must be empty"
                );
                assert_eq!(
                    n.content_start,
                    n.offset + 4,
                    "its header is flag + type + schema and nothing else"
                );
            }
            for c in &n.children {
                walk(c, logical, here);
            }
        }
        for root in &stream.record_tree() {
            walk(root, logical, &mut here);
        }
        empty += here;
        carriers += usize::from(here > 0);
    }
    assert_eq!(
        carriers,
        files.len(),
        "every report should carry empty records ({empty} found across {} files)",
        files.len()
    );
    assert_sweep_covers_the_corpus(empty, "empty record");
}

/// A record type's schema version is a property of the type: two records of the same type in a
/// `Contents` stream never disagree about it.
#[test]
fn contents_schema_version_is_fixed_per_record_type() {
    let records = contents_records();
    let mut seen: std::collections::BTreeMap<u16, u16> = std::collections::BTreeMap::new();
    for (rtype, schema) in records {
        let first = *seen.entry(rtype).or_insert(schema);
        assert_eq!(
            first, schema,
            "record type {rtype:#06x} carries two schema words ({first:#06x} and {schema:#06x})"
        );
    }
    // The pairwise assertion above holds vacuously on a corpus covering few types, so the breadth of
    // the sweep is asserted too. The floor sits just under the types the corpus reaches.
    eprintln!(
        "[record header] {} distinct Contents record type(s)",
        seen.len()
    );
    assert!(
        seen.len() >= 180,
        "the corpus covers only {} record type(s); the schema-fixity check is barely exercised",
        seen.len()
    );
}
