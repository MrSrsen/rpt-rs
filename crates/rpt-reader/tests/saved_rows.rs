//! Saved-data row completeness over the committed corpus.
//!
//! Saved data is one of the few decode paths that can be wrong without any other test noticing: the
//! rows are both the input and the ground truth, so a decoder that stops early just yields a shorter
//! rowset, and the pipeline, the layout and every baseline downstream see a smaller report rather
//! than a broken one. Nothing else in the suite can tell "this report has N rows" from "this report
//! has more and we dropped some".
//!
//! The guard is that the two counts come from **different** places and must agree: `record_count` is
//! summed from the `DataSourceManager` batch directory (metadata, read without decrypting anything),
//! while `rows` is what actually decoded out of the batch ciphertext. A batch that fails to decrypt,
//! or a directory walk that stops at the first entry, moves one and not the other.

use std::path::PathBuf;

use rpt_reader::Rpt;

/// Every report of the corpus — discovered, not named, so a newly added tree is swept without
/// anyone remembering this file exists.
fn corpus() -> Vec<PathBuf> {
    rpt_test_support::corpus_reports()
}

#[test]
fn every_fixture_decodes_all_of_its_saved_rows() {
    let files = corpus();
    let mut with_saved = 0usize;
    let mut short = Vec::new();
    for f in &files {
        let Ok(rpt) = Rpt::open(f) else { continue };
        let Some(saved) = rpt.report().saved_data.as_ref() else {
            continue;
        };
        with_saved += 1;
        if saved.rows.len() as u32 != saved.record_count {
            short.push(format!(
                "  {}: stored {} record(s), decoded {}",
                f.file_name().unwrap_or_default().to_string_lossy(),
                saved.record_count,
                saved.rows.len()
            ));
        }
    }

    assert!(
        short.is_empty(),
        "{} of {} fixtures carrying saved data decode fewer rows than they store:\n{}",
        short.len(),
        with_saved,
        short.join("\n")
    );

    assert_saved_data_coverage(with_saved, files.len());
}

/// A corpus with no saved data at all satisfies both row-count checks vacuously, so each states how
/// many reports it actually read a batch out of. The floor sits just under what the corpus reaches,
/// which is what makes a batch class that stops decoding a failure rather than a smaller sweep.
fn assert_saved_data_coverage(with_saved: usize, files: usize) {
    eprintln!("[saved rows] {with_saved} of {files} report(s) carry saved data");
    assert!(
        with_saved >= 100,
        "only {with_saved} of {files} report(s) carry saved data — the check is not covering the corpus"
    );
}

#[test]
fn the_directory_record_count_agrees_with_the_decoded_rows() {
    let files = corpus();

    // `saved_record_count` walks the batch directory alone and never decrypts a batch, so it is an
    // independent reading of how many rows the report stores. Where it disagrees with the decode,
    // one of the two walks the directory wrongly — which is how a whole trailing batch goes missing.
    let mut with_saved = 0usize;
    let mut disagree = Vec::new();
    for f in &files {
        let Ok(rpt) = Rpt::open(f) else { continue };
        let Some(saved) = rpt.report().saved_data.as_ref() else {
            continue;
        };
        with_saved += 1;
        let from_directory = rpt.saved_record_count().unwrap_or(0);
        if from_directory != saved.record_count {
            disagree.push(format!(
                "  {}: directory says {from_directory}, decode says {}",
                f.file_name().unwrap_or_default().to_string_lossy(),
                saved.record_count
            ));
        }
    }

    assert!(
        disagree.is_empty(),
        "{} fixture(s) where the batch directory and the decode disagree on the record count:\n{}",
        disagree.len(),
        disagree.join("\n")
    );
    assert_saved_data_coverage(with_saved, files.len());
}

/// A report whose rows span **several** batches, pinned exactly.
///
/// Its 249 rows sit in one record-index batch but two memo-descriptor batches, 170 + 79. The 170 is
/// not a property of the data: a descriptor batch holds `10224 / item_size` rows (a fixed byte
/// budget), which at this report's 60-byte descriptor is 170. A decoder that reads only a batch
/// class's first batch therefore returns exactly 170 rows and silently drops the rest, which is the
/// shape this fixture exists to catch — an equality, so the count cannot drift in either direction.
#[test]
fn a_multi_batch_report_decodes_every_batch() {
    let path = rpt_test_support::fixture("tests/fixtures/reports/worrall/AlphaISOsByCountry.rpt");
    let rpt = Rpt::open(&path).expect("open");
    let saved = rpt.report().saved_data.as_ref().expect("saved data");

    assert_eq!(saved.record_count, 249);
    assert_eq!(saved.rows.len(), 249);

    // Rows past the first descriptor batch must be real data, not a zero-filled tail: every row
    // carries a distinct alpha-3 code, so a duplicated or blank tail collapses this count.
    let alpha3: std::collections::HashSet<_> = saved
        .rows
        .iter()
        .filter_map(|r| r.get(3).cloned().flatten())
        .collect();
    assert_eq!(alpha3.len(), 249, "the tail rows are not distinct values");
}
