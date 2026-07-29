//! Corpus invariants of the public field-table reading (`rpt_reader::fields`).
//!
//! The reading exists to answer *where a field's bytes are*, so what is asserted here is the
//! coordinate arithmetic: every field must land inside the record it came from, the two coordinates
//! it reports must stay consistent with each other, and the bytes a field names must be the bytes
//! its value was decoded from.
//!
//! Fixture-gated: no fixtures means skip.

use std::path::PathBuf;

use rpt_reader::fields::{self, FieldKind, FieldValue, RecordFields};
use rpt_reader::raw::{Dialect, RecordNode};
use rpt_reader::{Rpt, StreamId};

/// Every report of the corpus — discovered, not named, so a newly added tree is swept without
/// anyone remembering this file exists.
fn corpus() -> Vec<PathBuf> {
    rpt_test_support::corpus_reports()
}

/// Every field-table reading in the corpus, tagged with the fixture it came from.
fn readings() -> Vec<(String, RecordFields)> {
    let mut out = Vec::new();
    for path in corpus() {
        let Ok(rpt) = Rpt::open(&path) else { continue };
        let Some(stream) = rpt.stream(&StreamId::Contents) else {
            continue;
        };
        let logical = stream.logical_bytes();
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        fn walk(n: &RecordNode, logical: &[u8], name: &str, out: &mut Vec<(String, RecordFields)>) {
            if let Some(r) = fields::read(n, logical, Dialect::Contents) {
                out.push((name.to_string(), r));
            }
            for c in &n.children {
                walk(c, logical, name, out);
            }
        }
        for root in &stream.record_tree() {
            walk(root, logical, &name, &mut out);
        }
    }
    out
}

/// Assert the sweep actually read the corpus.
///
/// Every check in this file is of the form "no reading violates X", which an empty or partial walk
/// satisfies, so each states how many readings it looked at. The floor sits well under what the
/// committed fixtures alone produce, and far above what any single report contributes.
fn assert_readings_cover_the_corpus(readings: &[(String, RecordFields)]) {
    eprintln!("[field view] {} tabled record reading(s)", readings.len());
    assert!(
        readings.len() >= 10_000,
        "only {} tabled record(s) found — the field-table sweep is not covering the corpus",
        readings.len()
    );
}

/// A field's bytes belong to the record it was read from, and the two coordinates describe the
/// same run: the same length, advancing together, never running past the content.
#[test]
fn every_field_lands_inside_the_record_it_came_from() {
    let readings = readings();
    assert_readings_cover_the_corpus(&readings);

    for (file, r) in &readings {
        let where_ = |f: &str| format!("{file}: {}.{f}", r.table);
        let mut prev = r.content.start;
        for f in &r.fields {
            assert!(
                f.span.start >= r.content.start && f.span.end <= r.content.end,
                "{}: [{:#x}..{:#x}) escapes the content [{:#x}..{:#x})",
                where_(&f.path),
                f.span.start,
                f.span.end,
                r.content.start,
                r.content.end
            );
            // A repeat's span covers its rows, which are reported after it, so only the scalar
            // fields have to advance monotonically.
            if f.kind != FieldKind::Repeat {
                assert!(
                    f.span.start >= prev,
                    "{}: starts at {:#x}, before the previous field's end {prev:#x}",
                    where_(&f.path),
                    f.span.start
                );
                prev = f.span.end;
            }
            if !f.joined.is_empty() && f.kind != FieldKind::Repeat {
                assert_eq!(
                    f.span.len(),
                    f.joined.len(),
                    "{}: the two coordinates disagree on the field's length",
                    where_(&f.path)
                );
            }
        }
    }
}

/// The bytes a field names are the bytes its value came from.
///
/// The joined coordinate indexes the record's joined runs, so a fixed-width scalar's value must be
/// exactly what the bytes under it say, and an undecoded run must be exactly those bytes. This is
/// what the coordinate is for: an edit addressed by field name writes there.
#[test]
fn a_fields_reported_bytes_are_the_bytes_its_value_came_from() {
    /// A big-endian run as an unsigned number, and as a two's-complement one of its own width.
    fn be(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b))
    }
    fn signed(bytes: &[u8]) -> i64 {
        let shift = 64 - 8 * bytes.len() as u32;
        ((be(bytes) << shift) as i64) >> shift
    }

    fn walk(n: &RecordNode, logical: &[u8], file: &str, checked: &mut usize) {
        if let Some(r) = fields::read(n, logical, Dialect::Contents) {
            let joined = n.joined_runs(logical);
            for f in &r.fields {
                let Some(bytes) = joined.get(f.joined.start..f.joined.end) else {
                    continue;
                };
                let where_ = || format!("{file}: {}.{}", r.table, f.path);
                match (&f.value, f.kind) {
                    (FieldValue::Bytes(b), FieldKind::Skip) => {
                        assert_eq!(b, bytes, "{}", where_());
                        *checked += 1;
                    }
                    (
                        FieldValue::Uint(v),
                        FieldKind::U8 | FieldKind::U16Be | FieldKind::U32Be | FieldKind::Bool,
                    ) => {
                        assert_eq!(be(bytes), u64::from(*v), "{}", where_());
                        *checked += 1;
                    }
                    (FieldValue::Int(v), FieldKind::I8 | FieldKind::I16Be | FieldKind::I32Be) => {
                        assert_eq!(signed(bytes), i64::from(*v), "{}", where_());
                        *checked += 1;
                    }
                    _ => {}
                }
            }
        }
        for c in &n.children {
            walk(c, logical, file, checked);
        }
    }

    let mut checked = 0usize;
    for path in corpus() {
        let Ok(rpt) = Rpt::open(&path) else { continue };
        let Some(stream) = rpt.stream(&StreamId::Contents) else {
            continue;
        };
        let logical = stream.logical_bytes();
        let file = path.file_stem().unwrap_or_default().to_string_lossy();
        for root in &stream.record_tree() {
            walk(root, logical, &file, &mut checked);
        }
    }
    eprintln!("[field view] {checked} field(s) checked against their own bytes");
    assert!(
        checked >= 10_000,
        "only {checked} field(s) checked — the sweep is not covering the corpus"
    );
}

/// The joined-runs coordinate is not the file coordinate: for a record with children the two
/// diverge, by exactly the framed length of every child before the field. Nothing else in the
/// corpus proves that the accessor is worth having over an offset into the joined buffer.
#[test]
fn a_record_with_children_makes_the_two_coordinates_diverge() {
    let readings = readings();
    assert_readings_cover_the_corpus(&readings);
    // Within one record the distance between the two coordinates must *change* — that is what a
    // spliced-out child does to it, and no single offset correction can reproduce it.
    let diverged = readings.iter().any(|(_, r)| {
        let deltas: Vec<usize> = r
            .fields
            .iter()
            .filter(|f| !f.joined.is_empty() && f.kind != FieldKind::Repeat)
            .map(|f| f.span.start - f.joined.start)
            .collect();
        deltas.windows(2).any(|w| w[0] != w[1])
    });
    assert!(
        diverged,
        "no record in the corpus has a field past a child record"
    );
}

/// Every query-engine session reading in the corpus, tagged with the fixture it came from.
fn qe_readings() -> Vec<(String, RecordFields)> {
    let mut out = Vec::new();
    for path in corpus() {
        let Ok(rpt) = Rpt::open(&path) else { continue };
        let Some(stream) = rpt.stream(&StreamId::QESession) else {
            continue;
        };
        let logical = stream.logical_bytes();
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        fn walk(n: &RecordNode, logical: &[u8], name: &str, out: &mut Vec<(String, RecordFields)>) {
            if let Some(r) = fields::read(n, logical, Dialect::QeSession) {
                out.push((name.to_string(), r));
            }
            for c in &n.children {
                walk(c, logical, name, out);
            }
        }
        for root in &stream.record_tree() {
            walk(root, logical, &name, &mut out);
        }
    }
    out
}

/// A session record reads under the query engine's own table, not under the report definition's.
///
/// The two vocabularies overlap on the numbers that matter: `0x0003` is a table here and a saved
/// printer there, `0x0004` a column here and a document header there. Read in the wrong one, a
/// session record either decodes as an unrelated record or, where the definition has no table for
/// the number, reports no reading at all.
#[test]
fn a_session_record_reads_under_the_query_engines_own_table() {
    let readings = qe_readings();
    eprintln!("[field view] {} session reading(s)", readings.len());
    assert!(
        readings.len() >= 100,
        "only {} session record(s) found — the query-engine sweep is not covering the corpus",
        readings.len()
    );

    let qe_tables: Vec<&'static str> = fields::tabled_types(Dialect::QeSession)
        .into_iter()
        .map(|(_, n)| n)
        .collect();
    let mut seen: Vec<&'static str> = Vec::new();
    for (file, r) in &readings {
        assert!(
            qe_tables.contains(&r.table),
            "{file}: {:#06x} read under {}, which is not a session table",
            r.rtype,
            r.table
        );
        if !seen.contains(&r.table) {
            seen.push(r.table);
        }
    }
    // The corpus reaches the session's connection/table/field records; naming them from the report
    // definition gave three of them the wrong table and the rest none.
    for want in ["QeConnection", "QeTable", "QeField"] {
        assert!(seen.contains(&want), "no {want} reading in the corpus");
    }
}

/// Only the record types with a table read this way; everything else is `None`, which is what makes
/// the presence of a reading a usable selector for a caller choosing a view.
#[test]
fn a_record_type_without_a_table_has_no_reading() {
    let tabled: Vec<u16> = fields::tabled_types(Dialect::Contents)
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    assert!(!tabled.is_empty());
    let readings = readings();
    assert_readings_cover_the_corpus(&readings);
    for (_, r) in readings {
        assert!(tabled.contains(&r.rtype), "{:#06x} has no table", r.rtype);
        assert_eq!(
            fields::table_name(r.rtype, Dialect::Contents),
            Some(r.table)
        );
    }
}
