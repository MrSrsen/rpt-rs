//! The bridge from the data pipeline's diagnostic vocabulary to the Page IR's.
//!
//! The record pipeline's fail-open sites use [`rpt_data::EvalDiagnostic`]; layout/render fidelity gaps
//! use [`rpt_pages::Diagnostic`]. Only the second reaches the caller, and on its own it has no way to
//! *say* "your record selection failed" — so a sink attached to it alone cannot express the data
//! pipeline's most user-relevant failures.
//!
//! This module is the one conversion point. It lives here because `rpt-layout` is the only crate that
//! depends on both, which keeps `rpt-data` free of a `rpt-pages` dependency (it must stay WASM-safe
//! with minimal deps).
//!
//! Nothing is dropped in the hand-off: kind maps to kind, `detail` to `message`, `source` to `source`,
//! and the record index to the structured location. **Severity is added**, not derived from the data
//! side — which failures are errors and which are warnings is a presentation decision, and this is the
//! presentation layer. The rule: a fail-open that *discards data* is an error, one that keeps data but
//! formats it differently is a warning.

use rpt_data::{DiagnosticKind as DataKind, EvalDiagnostic};
use rpt_pages::{Diagnostic, DiagnosticKind as PageKind, Severity};

/// Convert one data-pipeline diagnostic into a Page-IR diagnostic.
pub fn from_eval(d: &EvalDiagnostic) -> Diagnostic {
    let (kind, severity) = match d.kind {
        // A dropped row is lost output. If the selection fails on every row the report renders empty,
        // which is the single most misleading thing this pipeline can do.
        DataKind::RecordSelection => (PageKind::RecordSelection, Severity::Error),
        // No data was lost to a failure — the selection did its job. Still reported, because it is
        // the explanation for an empty report.
        DataKind::AllRowsExcluded => (PageKind::RecordSelection, Severity::Warning),
        // The group is kept, so nothing is lost — but the report shows groups the selection was
        // supposed to remove.
        DataKind::GroupSelection => (PageKind::GroupSelection, Severity::Warning),
        // The field resolves to Null: the value is gone, even though the row survives.
        DataKind::Formula => (PageKind::FormulaError, Severity::Error),
        // The formula did not parse, so whatever it evaluated to is meaningless — and unlike a runtime
        // error, this one the report's author can fix.
        DataKind::FormulaParse => (PageKind::FormulaParse, Severity::Error),
        // Rows are all still present, grouped by raw value rather than by the requested condition.
        DataKind::UnsupportedGroupCondition => {
            (PageKind::UnsupportedGroupCondition, Severity::Warning)
        }
        // The rows are all present; their type is not what the report declared, which changes how
        // they sort, group, and summarize.
        DataKind::TypeCoercion => (PageKind::TypeCoercion, Severity::Warning),
    };
    let mut out = Diagnostic {
        severity,
        kind,
        message: d.detail.clone(),
        source: d.source.clone(),
        location: rpt_pages::DiagnosticLocation::default(),
    };
    out.location.record_index = d.record_index;
    out.location.span = d.span.clone();
    out
}

/// Convert a whole batch, preserving report order.
pub fn from_evals(diagnostics: &[EvalDiagnostic]) -> Vec<Diagnostic> {
    diagnostics.iter().map(from_eval).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(kind: DataKind) -> EvalDiagnostic {
        EvalDiagnostic {
            kind,
            detail: "boom".to_string(),
            source: Some("Order Total".to_string()),
            record_index: Some(41),
            span: Some(3..9),
        }
    }

    #[test]
    fn every_data_kind_maps_to_a_page_kind_that_says_the_same_thing() {
        // Exhaustive by construction: adding a data-side kind without a mapping fails to compile in
        // `from_eval`, and this asserts each existing one keeps its meaning rather than collapsing
        // into `Other`.
        let cases = [
            (DataKind::RecordSelection, PageKind::RecordSelection),
            (DataKind::AllRowsExcluded, PageKind::RecordSelection),
            (DataKind::GroupSelection, PageKind::GroupSelection),
            (DataKind::Formula, PageKind::FormulaError),
            (DataKind::FormulaParse, PageKind::FormulaParse),
            (
                DataKind::UnsupportedGroupCondition,
                PageKind::UnsupportedGroupCondition,
            ),
            (DataKind::TypeCoercion, PageKind::TypeCoercion),
        ];
        for (data_kind, page_kind) in cases {
            assert_eq!(from_eval(&eval(data_kind)).kind, page_kind);
        }
    }

    #[test]
    fn nothing_is_lost_in_the_handoff() {
        let d = from_eval(&eval(DataKind::Formula));
        assert_eq!(d.message, "boom");
        assert_eq!(d.source.as_deref(), Some("Order Total"));
        assert_eq!(d.location.record_index, Some(41));
        assert_eq!(d.location.span, Some(3..9));
    }

    /// A fail-open that discards data is an error; one that keeps data is a warning.
    #[test]
    fn severity_follows_whether_data_was_discarded() {
        assert_eq!(
            from_eval(&eval(DataKind::RecordSelection)).severity,
            Severity::Error
        );
        assert_eq!(
            from_eval(&eval(DataKind::Formula)).severity,
            Severity::Error
        );
        assert_eq!(
            from_eval(&eval(DataKind::GroupSelection)).severity,
            Severity::Warning
        );
        assert_eq!(
            from_eval(&eval(DataKind::AllRowsExcluded)).severity,
            Severity::Warning
        );
        assert_eq!(
            from_eval(&eval(DataKind::UnsupportedGroupCondition)).severity,
            Severity::Warning
        );
    }
}
