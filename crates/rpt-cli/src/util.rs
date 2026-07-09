//! Shared helpers for the `rpt` CLI: process-exit plumbing, JSON printing, the color decision, and
//! the small prominence palette the `tree` and `dump` renderers paint with.

use std::io::IsTerminal as _;
use std::process::ExitCode;

use serde::Serialize;

/// Turn a command result into a process exit code, printing any error to stderr.
pub(crate) fn run(r: rpt::Result<()>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
