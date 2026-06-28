//! Crash diagnostics for the CLI entry points.

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
