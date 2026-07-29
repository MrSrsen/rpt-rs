//! Decode coverage over the committed corpus.
//!
//! The point of the coverage meter is to be *specific*: it must fire on a report the decoder did not
//! fully read and stay silent on one it did. A metric that warns about everything is worse than no
//! metric, because it trains the user to ignore it — so this asserts the signal-to-noise ratio over
//! the whole corpus, not just that the API returns something.

use std::path::PathBuf;

use rpt_reader::{BatchProblem, Rpt, SavedBatchKind, SavedDataStatus};

/// Every report of the corpus — discovered, not named, so a newly added tree is swept without
/// anyone remembering this file exists.
fn corpus() -> Vec<PathBuf> {
    rpt_test_support::corpus_reports()
}

#[test]
fn coverage_is_specific_not_noisy_across_the_corpus() {
    let files = corpus();
    let incomplete: Vec<(String, String)> = files
        .iter()
        .filter_map(|f| {
            let rpt = Rpt::open(f).ok()?;
            let w = rpt.decode_coverage().warning()?;
            Some((f.file_name()?.to_string_lossy().into_owned(), w))
        })
        .collect();

    // The overwhelming majority of the corpus decodes completely; a meter that flagged a large share
    // of it would be measuring its own blind spots rather than the reports'.
    assert!(
        incomplete.len() * 5 < files.len(),
        "{} of {} fixtures report an incomplete decode — the metric is too noisy to act on:\n{}",
        incomplete.len(),
        files.len(),
        incomplete
            .iter()
            .map(|(f, w)| format!("  {f}: {w}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // The silent half of the claim is the one a floor can state: a run in which the meter was never
    // observed staying quiet has measured nothing. The noisy half cannot be floored the same way —
    // requiring a minimum number of warnings would make an improvement to the decoder fail the test —
    // so the actionability loop below is conditional coverage by design.
    let complete = files.len() - incomplete.len();
    eprintln!(
        "[coverage] {complete} complete, {} incomplete of {} report(s)",
        incomplete.len(),
        files.len()
    );
    assert!(
        complete > 100,
        "only {complete} report(s) decoded completely — the meter's silent case is not covered"
    );

    // Whatever it does flag, it must flag *actionably*: the record type to go and decode, the bytes
    // left over, or the saved-data batch that would not decode — and where to see the breakdown.
    for (file, warning) in &incomplete {
        assert!(
            warning.contains("rpt streams"),
            "{file}: the warning must point at the breakdown command: {warning}"
        );
        assert!(
            warning.contains("0x")
                || warning.contains("byte(s)")
                || warning.contains("saved-data")
                || warning.contains("saved row"),
            "{file}: the warning must name a record type, a byte count, or a saved-data \
             shortfall: {warning}"
        );
    }
}

#[test]
fn a_report_whose_saved_batch_will_not_decrypt_says_so() {
    // The failure this names is invisible from the model alone: the report's descriptor claims 800
    // stored rows and `saved_data()` returns nothing, which on its own is indistinguishable from a
    // report saved without data.
    let f = corpus()
        .into_iter()
        .find(|f| f.file_name().is_some_and(|n| n == "02_product_catalog.rpt"))
        .expect("the committed product-catalog fixture is in the corpus");
    let rpt = Rpt::open(&f).expect("open");
    assert!(rpt.report().has_saved_data);
    assert!(rpt.saved_data().is_none());

    let status = rpt.saved_data_status();
    assert_eq!(
        status,
        SavedDataStatus::BatchUndecodable {
            kind: SavedBatchKind::Index,
            index: 0,
            problem: BatchProblem::NotDecrypted,
        },
        "the reason the rows did not decode must survive the decode"
    );
    assert!(!status.is_complete());
    let shortfall = status.shortfall().expect("a lost batch is a shortfall");
    assert!(shortfall.contains("record-index batch #0"), "{shortfall}");
    // And it reaches the one warning a caller is expected to read.
    assert!(!rpt.decode_coverage().is_complete());
}

#[test]
fn a_report_saved_without_data_is_not_a_failure() {
    // The silent half: a report that stores no rows must not warn, or the diagnostic is noise.
    let f = corpus()
        .into_iter()
        .find(|f| f.file_name().is_some_and(|n| n == "SportsTeams.rpt"))
        .expect("the committed SportsTeams fixture is in the corpus");
    let rpt = Rpt::open(&f).expect("open");
    let status = rpt.saved_data_status();
    assert!(status.is_complete(), "{status}");
    assert_eq!(status.shortfall(), None);
}

#[test]
fn a_completely_decoded_report_reports_complete() {
    // A committed fixture, so its absence is the corpus having moved rather than a check being
    // unavailable — and skipping would leave the complete-decode case unasserted.
    let f = corpus()
        .into_iter()
        .find(|f| f.file_name().is_some_and(|n| n == "SportsTeams.rpt"))
        .expect("the committed SportsTeams fixture is in the corpus");
    let coverage = Rpt::open(&f).expect("open").decode_coverage();
    assert!(
        coverage.is_complete(),
        "{} should decode completely: {:?}",
        f.display(),
        coverage.warning()
    );
    assert_eq!(coverage.unknown_records(), 0);
    assert_eq!(coverage.uncovered_bytes(), 0);
    // Every stream is accounted for, including the ones that decode by a non-record route.
    assert!(!coverage.streams.is_empty());
}
