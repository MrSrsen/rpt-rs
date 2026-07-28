//! Crash and error-reporting diagnostics for the CLI entry points.

/// Render `err` and its whole [`source`](std::error::Error::source) chain as one line —
/// `top: cause: root-cause` — so the underlying I/O or driver error surfaces instead of being
/// hidden behind the top-level message.
///
/// This is the reporting half of the crate's error convention: a variant that carries a `source`
/// does not interpolate it, so *only* a chain walk shows the full story. Both binaries print
/// through this so they report to the same standard.
///
/// A cause whose text is already a suffix of what has been accumulated is skipped, so a foreign
/// error type that *does* interpolate its own source cannot produce a doubled segment.
///
/// ```
/// # use std::fmt;
/// #[derive(Debug)]
/// struct Inner;
/// impl fmt::Display for Inner {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("disk is full") }
/// }
/// impl std::error::Error for Inner {}
///
/// #[derive(Debug)]
/// struct Outer(Inner);
/// impl fmt::Display for Outer {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("cannot write `out.rpt`") }
/// }
/// impl std::error::Error for Outer {
///     fn source(&self) -> Option<&(dyn std::error::Error + 'static)> { Some(&self.0) }
/// }
///
/// assert_eq!(rpt::error_chain(&Outer(Inner)), "cannot write `out.rpt`: disk is full");
/// ```
pub fn error_chain(err: &dyn std::error::Error) -> String {
    let mut msg = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        // Guard against a foreign error type that interpolates its own source into its Display:
        // appending would repeat a segment already present at the tail of `msg`.
        if !text.is_empty() && !msg.ends_with(&text) {
            msg.push_str(": ");
            msg.push_str(&text);
        }
        source = cause.source();
    }
    msg
}

/// Install a panic hook that always prints the panic message **and a full backtrace** to stderr,
/// regardless of the `RUST_BACKTRACE` environment variable.
///
/// [`std::backtrace::Backtrace::force_capture`] captures a trace even when `RUST_BACKTRACE` is
/// unset. Build the release profile with line-table debug info
/// (`[profile.release] debug = "line-tables-only", strip = false`) so frames carry function
/// names and source locations.
///
/// A panic hook is global process state, so libraries must not install one implicitly — only the
/// binary entry points should call this (as the first thing in `main`).
pub fn install_panic_hook() {
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

#[cfg(test)]
mod tests {
    use super::error_chain;
    use std::fmt;

    #[derive(Debug)]
    struct Layer {
        msg: &'static str,
        cause: Option<Box<Layer>>,
    }

    impl fmt::Display for Layer {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.msg)
        }
    }

    impl std::error::Error for Layer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.cause
                .as_deref()
                .map(|c| c as &(dyn std::error::Error + 'static))
        }
    }

    fn layer(msg: &'static str, cause: Option<Layer>) -> Layer {
        Layer {
            msg,
            cause: cause.map(Box::new),
        }
    }

    #[test]
    fn chains_every_cause_in_order() {
        let err = layer("cannot write `out.rpt`", Some(layer("disk full", None)));
        assert_eq!(error_chain(&err), "cannot write `out.rpt`: disk full");
    }

    #[test]
    fn a_lone_error_is_its_own_message() {
        assert_eq!(error_chain(&layer("bad flag", None)), "bad flag");
    }

    // A foreign error type that interpolates its own source would otherwise repeat the segment the
    // walk is about to append.
    #[test]
    fn does_not_repeat_a_cause_already_interpolated_by_its_parent() {
        let err = layer(
            "database error: connection failed: connection refused",
            Some(layer("connection refused", None)),
        );
        assert_eq!(
            error_chain(&err),
            "database error: connection failed: connection refused"
        );
    }
}
