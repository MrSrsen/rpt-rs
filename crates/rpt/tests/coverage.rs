//! Decode coverage over the committed corpus.
//!
//! The point of the coverage meter is to be *specific*: it must fire on a report the decoder did not
//! fully read and stay silent on one it did. A metric that warns about everything is worse than no
//! metric, because it trains the user to ignore it — so this asserts the signal-to-noise ratio over
//! the whole corpus, not just that the API returns something.
//!
//! Fixture-gated: no fixtures means skip.

use std::path::{Path, PathBuf};

use rpt::Rpt;

fn corpus() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rpt") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(
        &rpt_test_support::fixture(Path::new("tests/fixtures/reports")),
        &mut out,
    );
    out.sort();
    out
}

#[test]
fn coverage_is_specific_not_noisy_across_the_corpus() {
    let files = corpus();
    if files.is_empty() {
        return;
    }
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

    // Whatever it does flag, it must flag *actionably*: the record type to go and decode, and where
    // to see the breakdown.
    for (file, warning) in &incomplete {
        assert!(
            warning.contains("rpt streams"),
            "{file}: the warning must point at the breakdown command: {warning}"
        );
        assert!(
            warning.contains("0x") || warning.contains("byte(s)"),
            "{file}: the warning must name a record type or a byte count: {warning}"
        );
    }
}

#[test]
fn a_completely_decoded_report_reports_complete() {
    let Some(f) = corpus()
        .into_iter()
        .find(|f| f.file_name().is_some_and(|n| n == "SportsTeams.rpt"))
    else {
        return;
    };
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
