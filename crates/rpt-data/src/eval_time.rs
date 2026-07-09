//! Formula **evaluation-time** classification.
//!
//! The native engine assigns every formula an evaluation-time class that decides *when* it (re)runs
//! relative to the record pass: [`BeforeReadingRecords`](EvalTime::BeforeReadingRecords)
//! (once, before any row — constants / parameter-only), [`WhileReadingRecords`](EvalTime::WhileReadingRecords)
//! (per row as it is read, in read order — the default for data-driven formulas), and
//! [`WhilePrintingRecords`](EvalTime::WhilePrintingRecords) (during the format/print pass, in print
//! order — anything that reads print state: page/record numbers, `Previous`/`Next`, running totals).
//!
//! The class is either **declared** (a formula body may open with `WhilePrintingRecords`,
//! `WhileReadingRecords`, or `BeforeReadingRecords`) or **inferred** from what the body reads. This
//! is the read-vs-print-order signal running `Global`/`Shared` variables depend on; the persistent
//! store lives in [`SharedState`](crate::SharedState).

use crystal_formula::refs::references;
use crystal_formula::RefKind;

/// When a formula is (re)evaluated relative to the record pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalTime {
    /// Once, before the first record is read (constants, parameter-only formulas).
    BeforeReadingRecords,
    /// Per record, in **read** order — before sort/group. The default for formulas that read
    /// database fields but no print state.
    WhileReadingRecords,
    /// During the format pass, in **print** order (after sort/group). Forced by any read of print
    /// state — page/record position, record navigation, or an explicit marker.
    WhilePrintingRecords,
}

/// The three declared evaluation-time markers a Crystal body may open with.
const MARKERS: &[(&str, EvalTime)] = &[
    ("beforereadingrecords", EvalTime::BeforeReadingRecords),
    ("whilereadingrecords", EvalTime::WhileReadingRecords),
    ("whileprintingrecords", EvalTime::WhilePrintingRecords),
];

/// Classify a formula body's evaluation time. A **declared** marker wins; otherwise it is inferred:
/// reading print state ⇒ `WhilePrintingRecords`, else reading a database field ⇒ `WhileReadingRecords`,
/// else (constants / parameters / other formulas only) ⇒ `BeforeReadingRecords`.
pub fn classify_eval_time(body: &str) -> EvalTime {
    // References (`{field}`/`{@formula}`/`{?param}`) are found on the raw body; print-state markers
    // are matched on the body with `{...}` runs stripped, so a field name that happens to contain
    // `next`/`previous` cannot be mistaken for the record-navigation functions.
    let outside = strip_brace_runs(body).to_lowercase();

    for (kw, when) in MARKERS {
        if contains_word(&outside, kw) {
            return *when;
        }
    }
    // A read of print/record state (page/record/group position, `Previous`/`Next`) forces the print
    // pass. `crystal-formula` owns the name classification (print-state specials ∪ record-nav); scan
    // the brace-stripped body's identifier words so a field name that merely contains such a word
    // cannot be mistaken for it.
    if outside
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|w| crystal_formula::is_print_state_special(w) || crystal_formula::is_record_nav(w))
    {
        return EvalTime::WhilePrintingRecords;
    }
    let refs = references(body);
    if refs.iter().any(|r| r.kind == RefKind::Field) {
        return EvalTime::WhileReadingRecords;
    }
    // A formula that only chains other formulas is read-time if any of them is data-driven; without
    // full dependency analysis we treat a formula reference as read-time (conservative — it runs in
    // the record pass, never later than a print-state formula which is already handled above).
    if refs.iter().any(|r| r.kind == RefKind::Formula) {
        return EvalTime::WhileReadingRecords;
    }
    EvalTime::BeforeReadingRecords
}

/// Remove every `{...}` reference run (fields/formulas/parameters) from `body`, leaving the bare
/// identifiers, operators, and literals — so a word scan sees only out-of-reference tokens.
fn strip_brace_runs(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut depth = 0usize;
    for ch in body.chars() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                out.push(' '); // keep tokens on either side separated
            }
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Whether `word` (already lowercase) appears in `haystack` (already lowercase) delimited by
/// non-identifier characters — a Crystal identifier is `[A-Za-z0-9_]`.
fn contains_word(haystack: &str, word: &str) -> bool {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let bytes = haystack.as_bytes();
    let mut from = 0;
    while let Some(pos) = haystack[from..].find(word) {
        let start = from + pos;
        let end = start + word.len();
        let before_ok = start == 0 || !is_ident(bytes[start - 1] as char);
        let after_ok = end >= bytes.len() || !is_ident(bytes[end] as char);
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_marker_wins() {
        assert_eq!(
            classify_eval_time("WhilePrintingRecords; {@x} + 1"),
            EvalTime::WhilePrintingRecords
        );
        assert_eq!(
            classify_eval_time("BeforeReadingRecords; {?Threshold}"),
            EvalTime::BeforeReadingRecords
        );
    }

    #[test]
    fn print_state_forces_print_time() {
        assert_eq!(
            classify_eval_time("PageNumber + 1"),
            EvalTime::WhilePrintingRecords
        );
        assert_eq!(
            classify_eval_time("Previous({orders.amount})"),
            EvalTime::WhilePrintingRecords
        );
    }

    #[test]
    fn field_reference_is_read_time() {
        assert_eq!(
            classify_eval_time("{orders.amount} * 2"),
            EvalTime::WhileReadingRecords
        );
    }

    #[test]
    fn constant_only_is_before_reading() {
        assert_eq!(classify_eval_time("2 + 2"), EvalTime::BeforeReadingRecords);
        assert_eq!(
            classify_eval_time("{?Region}"),
            EvalTime::BeforeReadingRecords
        );
    }

    #[test]
    fn field_name_containing_next_is_not_print_state() {
        // `next_ship_date` must not be read as the `Next` navigation function.
        assert_eq!(
            classify_eval_time("{orders.next_ship_date}"),
            EvalTime::WhileReadingRecords
        );
    }
}
