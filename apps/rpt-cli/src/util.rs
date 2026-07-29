//! Shared helpers for the `rpt` CLI: process-exit plumbing (including the crash/backtrace hook),
//! JSON printing, the color decision, and the small prominence palette the `tree` and `dump`
//! renderers paint with.

use std::io::IsTerminal as _;
use std::process::ExitCode;

use serde::Serialize;

/// An error raised by a `rpt` CLI command. `Usage` marks a bad invocation — a malformed flag or
/// argument value — and is the only variant that must not be attributed to I/O; every other variant
/// is a genuine reader or output error.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CliError {
    /// A CLI-argument / usage error: a malformed flag or positional argument value.
    #[error("{0}")]
    Usage(String),
    /// An error from `rpt-reader` (opening, decoding, or the write path).
    #[error(transparent)]
    Report(#[from] rpt_reader::Error),
    /// An I/O error writing a command's output: what was being attempted, plus the underlying
    /// failure as the [`source`](std::error::Error::source) (never interpolated — see
    /// [`rpt_reader::error_chain`]).
    #[error("{0}")]
    Io(String, #[source] std::io::Error),
    /// An error from the `rpt-json` export surface (the `json-dump` command).
    #[error(transparent)]
    Json(#[from] rpt_json::Error),
    /// `--strict` was requested and the report did not decode completely. Not a usage error: the
    /// invocation was fine, the *input* was not fully understood.
    #[error("{0}")]
    Strict(String),
}

impl CliError {
    /// A usage error from a message describing the bad flag or argument value.
    pub(crate) fn usage(msg: impl Into<String>) -> Self {
        CliError::Usage(msg.into())
    }

    /// An I/O error describing what was being attempted (e.g. ``cannot write `out.json` ``), with
    /// the underlying failure kept as the cause.
    pub(crate) fn io(msg: impl Into<String>, source: std::io::Error) -> Self {
        CliError::Io(msg.into(), source)
    }
}

/// Turn a command result into a process exit code, printing any error to stderr. A usage error
/// exits with code 2 (a malformed invocation); any other error exits with code 1.
///
/// The whole `source()` chain is printed (via [`rpt_reader::error_chain`]), so the underlying I/O or
/// decode cause surfaces rather than only this layer's message — the same standard `rpt-render`
/// reports to.
pub(crate) fn run(r: Result<(), CliError>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", rpt_reader::error_chain(&e));
            match e {
                CliError::Usage(_) => ExitCode::from(2),
                _ => ExitCode::FAILURE,
            }
        }
    }
}

/// Install a panic hook that always prints the panic message **and a full backtrace** to stderr,
/// regardless of the `RUST_BACKTRACE` environment variable.
///
/// [`std::backtrace::Backtrace::force_capture`] captures a trace even when `RUST_BACKTRACE` is
/// unset. The release profile keeps line-table debug info so frames carry function names and source
/// locations.
///
/// A panic hook is process-global state and this one exits the process on a closed pipe, so it
/// belongs to a binary entry point — `main` calls it first, and no library installs one. The
/// `rpt-render` binary carries its own copy: neither `apps/` crate has a library target to share
/// one through, and pushing it back into a library is what this replaces.
pub(crate) fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        // `info`'s `Display` is the standard "panicked at <location>:\n<message>" text; the
        // closure parameter type is left inferred so this builds across rustc versions (the hook
        // signature's payload type changed between releases).
        let info = info.to_string();
        // A closed output pipe (the reader quit early, e.g. `… | head`, or `… | less` then `q`)
        // makes the `print!`/`println!` macros panic with std's "failed printing to std…" message.
        // That is a benign end-of-consumer condition, not a crash — exit quietly instead of dumping
        // a backtrace. This is platform-agnostic (Windows has no SIGPIPE) and needs no signal
        // handling.
        if info.contains("failed printing to std") {
            std::process::exit(0);
        }
        let backtrace = std::backtrace::Backtrace::force_capture();
        eprintln!("{info}");
        eprintln!("\nstack backtrace:\n{backtrace}");
    }));
}

/// Warn on stderr when `rpt` did not decode the whole report, so the user can tell an incomplete
/// export from a complete one.
///
/// Projection is infallible by design — an unrecognized record becomes a default, not an error — so
/// without this an export missing content is indistinguishable from a faithful one. With `strict`, an
/// incomplete decode is an error instead: for CI, where a silent partial export is worse than a
/// failure.
///
/// The warning goes to stderr, never stdout, so it cannot contaminate a piped export. Callers report
/// coverage *after* producing their output: the export is still written and still useful for working
/// out what is missing, and `strict` changes only the exit status.
pub(crate) fn report_coverage(
    coverage: &rpt_reader::DecodeCoverage,
    file: &str,
    strict: bool,
) -> Result<(), CliError> {
    let Some(warning) = coverage.warning() else {
        return Ok(());
    };
    if strict {
        return Err(CliError::Strict(format!("{file}: {warning}")));
    }
    eprintln!("warning: {file}: {warning}");
    Ok(())
}

/// Print a serializable value as a single line of JSON on stdout.
pub(crate) fn print_json<T: Serialize>(value: &T) {
    // These fixed shapes never fail to serialize (no non-string map keys, no custom impls).
    println!(
        "{}",
        serde_json::to_string(value).expect("JSON serialization cannot fail here")
    );
}

/// Decide whether to colorize output. Precedence: an explicit `--no-color` (or the `NO_COLOR`
/// convention) turns it off; an explicit `--color` (or `CLICOLOR_FORCE`) turns it on even when
/// piped — so `rpt tree … --color | less -R` keeps its colors; otherwise it is on only when stdout
/// is a terminal.
pub(crate) fn use_color(force_color: bool, no_color: bool) -> bool {
    // NO_COLOR (no-color.org): any non-empty value disables color.
    let no_color_env = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    if no_color || no_color_env {
        return false;
    }
    // CLICOLOR_FORCE: a non-empty, non-zero value forces color even when not a terminal.
    let force_env = std::env::var_os("CLICOLOR_FORCE").is_some_and(|v| !v.is_empty() && v != "0");
    force_color || force_env || std::io::stdout().is_terminal()
}

// ANSI SGR codes for the prominence palette. High-prominence content (field names, recognized
// types, text) is bright; low-prominence scaffolding (unknown types, small raw byte runs,
// connectors) is dimmed so the eye lands on the parts that carry meaning.
pub(crate) const RESET: &str = "\x1b[0m";
pub(crate) const DIM: &str = "\x1b[2m"; // scaffolding: connectors, hex tags, small byte runs
pub(crate) const BOLD: &str = "\x1b[1m"; // a stream group header (the first tier of the tree)
pub(crate) const CYAN: &str = "\x1b[36m"; // a recognized (named) record type
pub(crate) const YELLOW: &str = "\x1b[33m"; // decoded text content
pub(crate) const BOLD_GREEN: &str = "\x1b[1;32m"; // a field definition — the most prominent node
pub(crate) const BRIGHT_MAGENTA: &str = "\x1b[95m"; // a big embedded data blob (image / attachment)

/// Wrap `s` in the SGR `code` when `on`, else return it unchanged.
pub(crate) fn paint(on: bool, code: &str, s: &str) -> String {
    if on {
        format!("{code}{s}{RESET}")
    } else {
        s.to_string()
    }
}

/// Truncate to at most `max` characters, appending `…` when clipped.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

/// A note naming the subdocuments that carry their own saved-data streams, for a report whose
/// **top-level** saved data is absent or empty.
///
/// The saved-data streams are found by `StreamId` variant, and a stream inside a `Subdocument N`
/// storage classifies as `StreamId::Other`, so a subreport's own batch is not reached — which
/// otherwise reads as a report with no saved data, or as one whose batch class did not decode.
pub(crate) fn subdocument_saved_data(rpt: &rpt_reader::Rpt) -> Option<String> {
    let mut carriers: Vec<String> = rpt
        .streams()
        .filter_map(|(id, s)| match id {
            rpt_reader::StreamId::Other(path)
                if path.starts_with("Subdocument ") && path.contains("SavedRecordsStream") =>
            {
                let subdoc = path.split('/').next().unwrap_or(path);
                Some(format!("{subdoc} ({} B)", s.raw_bytes().len()))
            }
            _ => None,
        })
        .collect();
    if carriers.is_empty() {
        return None;
    }
    carriers.sort();
    Some(format!(
        "the saved data is in a subdocument, which is not decoded yet — {}. The report itself \
         carries none",
        carriers.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A report whose saved data lives in a subdocument is named as such, and one whose does not
    /// yields nothing — the difference the two saved-data commands report on. Without it, a report
    /// carrying a subreport's batch reads as one whose batch class did not decode.
    #[test]
    fn a_subdocument_batch_is_named_and_a_plain_report_is_not() {
        let with_sub =
            rpt_test_support::fixture("tests/fixtures/reports/benbrahim777/Top5USAwithSub.rpt");
        let rpt = rpt_reader::Rpt::open(&with_sub)
            .unwrap_or_else(|e| panic!("open {}: {e}", with_sub.display()));
        let note = subdocument_saved_data(&rpt).expect("its subreport carries its own batch");
        assert!(note.contains("Subdocument"), "{note}");

        let plain = rpt_test_support::fixture("tests/fixtures/reports/synthetic/blank_report.rpt");
        let rpt = rpt_reader::Rpt::open(&plain)
            .unwrap_or_else(|e| panic!("open {}: {e}", plain.display()));
        assert!(subdocument_saved_data(&rpt).is_none());
    }

    #[test]
    fn truncate_leaves_short_or_exact_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
        // Exactly at the cap is not clipped (no ellipsis).
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("", 0), "");
    }

    #[test]
    fn truncate_clips_and_appends_ellipsis() {
        assert_eq!(truncate("hello", 4), "hell…");
        // The `…` is appended, not counted toward `max`.
        assert_eq!(truncate("hello", 0), "…");
    }

    #[test]
    fn truncate_counts_characters_not_bytes() {
        // Multi-byte characters: the cap is in `char`s, and clipping never splits one mid-byte.
        assert_eq!(truncate("áéíóú", 5), "áéíóú"); // 5 chars, 10 bytes — unchanged
        assert_eq!(truncate("áéíóú", 3), "áéí…");
        // Wider-than-one-byte scripts and emoji stay on char boundaries.
        assert_eq!(truncate("日本語テキスト", 3), "日本語…");
        assert_eq!(truncate("😀😁😂", 2), "😀😁…");
    }

    // `use_color` reads the process-global `NO_COLOR` / `CLICOLOR_FORCE` env vars, so these tests
    // serialize on one lock and restore the prior values, and start from a known-clean environment.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_clean_color_env(f: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_no = std::env::var_os("NO_COLOR");
        let prev_force = std::env::var_os("CLICOLOR_FORCE");
        std::env::remove_var("NO_COLOR");
        std::env::remove_var("CLICOLOR_FORCE");
        f();
        match prev_no {
            Some(v) => std::env::set_var("NO_COLOR", v),
            None => std::env::remove_var("NO_COLOR"),
        }
        match prev_force {
            Some(v) => std::env::set_var("CLICOLOR_FORCE", v),
            None => std::env::remove_var("CLICOLOR_FORCE"),
        }
    }

    #[test]
    fn use_color_explicit_no_color_wins_over_force() {
        with_clean_color_env(|| {
            // `--no-color` disables even when `--color` is also passed.
            assert!(!use_color(true, true));
            assert!(!use_color(false, true));
        });
    }

    #[test]
    fn use_color_force_flag_enables_when_not_a_terminal() {
        with_clean_color_env(|| {
            // `--color` forces on even though the test's stdout is not a terminal.
            assert!(use_color(true, false));
        });
    }

    #[test]
    fn use_color_no_color_env_disables_and_beats_force() {
        with_clean_color_env(|| {
            std::env::set_var("NO_COLOR", "1");
            assert!(!use_color(true, false));
            // Per no-color.org, an *empty* value does not disable — `--color` then still wins.
            std::env::set_var("NO_COLOR", "");
            assert!(use_color(true, false));
        });
    }

    #[test]
    fn use_color_clicolor_force_env() {
        with_clean_color_env(|| {
            // A non-empty, non-"0" value forces color on with no flags (independent of the terminal).
            std::env::set_var("CLICOLOR_FORCE", "1");
            assert!(use_color(false, false));
            // A "0" value does not force — but the remaining fallback is `is_terminal()`, which is
            // not deterministic under the test harness, so that branch is intentionally not asserted.
            std::env::set_var("CLICOLOR_FORCE", "0");
            let _ = use_color(false, false);
        });
    }
}
