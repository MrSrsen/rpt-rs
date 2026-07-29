//! The CLI's typed render error, and how a failure reaches stderr — the cause-chain reporter and
//! the crash/backtrace panic hook.
//!
//! This lives in the CLI rather than the `rpt-render` library because the library's render path is
//! infallible — every failure a render can hit (datasource resolution, parameter coercion, a DB
//! driver, output I/O) is raised here, by the CLI. Each variant that wraps a real underlying error
//! keeps it as the [`source`](std::error::Error::source), so the top-level reporter can print the
//! whole cause chain instead of a flattened string.

use std::error::Error;

/// A render failure with a typed cause, so a caller can tell a datasource problem from a parameter,
/// database, or output failure — instead of matching on message strings.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RenderError {
    /// Opening or decoding the `.rpt` failed.
    #[error(transparent)]
    Rpt(#[from] rpt_reader::Error),
    /// Resolving the datasource failed: a connection URL's scheme/format, a missing connection URL,
    /// an unimplemented driver, or a scope with no live table. A synthesized, descriptive message
    /// (there is no underlying error to chain).
    #[error("datasource error: {0}")]
    Datasource(String),
    /// A report parameter value could not be coerced to its declared type.
    #[error("parameter error: {0}")]
    Params(String),
    /// A database driver failed (connection, healthcheck, or query). Carries the driver's own error
    /// as the source, so the cause chain survives rather than being flattened to a string. Only a
    /// driver raises it, so a DB-free build has no way to reach this state and does not carry it.
    #[cfg(feature = "db")]
    #[error("{context}")]
    Db {
        /// What was being attempted.
        context: String,
        /// The driver's error.
        #[source]
        source: Box<dyn Error + Send + Sync>,
        /// Extra lines a reporter prints *beneath* the message — the failing statement, and the
        /// pointer at `rpt sql` when a table or column is missing. Several lines long, so they are
        /// kept out of `Display` and the one-line cause chain stays one line.
        hint: Option<String>,
    },
    /// Writing the rendered output to a file or stdout failed. Carries the underlying I/O error.
    #[error("{0}")]
    Io(String, #[source] std::io::Error),
    /// An output request that cannot be satisfied (piping binary to a terminal, multi-page output to
    /// stdout, or a refused overwrite). A synthesized message with no underlying error.
    #[error("output error: {0}")]
    Output(String),
    /// The document did not meet the archival standard `--pdfa` asked for, so nothing was written: a
    /// file carrying a conformance claim it does not honour is worse than no file. The unmet
    /// requirements ride in `hint`, one per line, since a document can miss several at once.
    #[error("{level} conformance failed: {unmet} unmet requirement(s), no file written")]
    Conformance {
        /// The standard the render was exported against.
        level: rpt_render::Conformance,
        /// How many requirements the document did not satisfy.
        unmet: usize,
        /// One line per unmet requirement, printed beneath the message.
        hint: String,
    },
}

impl RenderError {
    /// Name the data source this failure happened against.
    ///
    /// A report plus its subreports can read from several servers, each configured by its own
    /// `RPT_DB_URL_<SERVER>`; without this a runtime failure does not say which one is wrong. Applied
    /// at the dispatch boundary, which is where the server key is known — the driver itself only sees
    /// a URL.
    #[cfg(feature = "db")]
    #[must_use]
    pub fn against_source(mut self, label: &str) -> RenderError {
        if let RenderError::Db { context, .. } = &mut self {
            *context = format!("{context} from data source `{label}`");
        }
        self
    }

    /// Extra lines a reporter should print beneath this error's message, if it carries any.
    pub fn hint(&self) -> Option<&str> {
        match self {
            #[cfg(feature = "db")]
            RenderError::Db { hint, .. } => hint.as_deref(),
            RenderError::Conformance { hint, .. } => Some(hint),
            _ => None,
        }
    }
}

#[cfg(feature = "db-postgres")]
impl From<rpt_db_postgres::DbError> for RenderError {
    fn from(e: rpt_db_postgres::DbError) -> RenderError {
        RenderError::Db {
            context: "database error".to_string(),
            hint: e.hint(),
            source: Box::new(e),
        }
    }
}

#[cfg(feature = "db-sqlite")]
impl From<rpt_db_sqlite::DbError> for RenderError {
    fn from(e: rpt_db_sqlite::DbError) -> RenderError {
        RenderError::Db {
            context: "database error".to_string(),
            hint: e.hint(),
            source: Box::new(e),
        }
    }
}

/// Render `err` and its whole `source()` chain as one line — `top: cause: root-cause` — so the
/// underlying I/O / driver error surfaces instead of being hidden behind the top-level message.
///
/// Delegates to [`rpt_reader::error_chain`]: the reporter is shared with the `rpt` binary so both report
/// to one standard, and library logic does not live in an `apps/` crate.
pub fn full_chain(err: &dyn Error) -> String {
    rpt_reader::error_chain(err)
}

/// Install a panic hook that always prints the panic message **and a full backtrace** to stderr,
/// regardless of the `RUST_BACKTRACE` environment variable.
///
/// [`std::backtrace::Backtrace::force_capture`] captures a trace even when `RUST_BACKTRACE` is
/// unset. The release profile keeps line-table debug info so frames carry function names and source
/// locations.
///
/// A panic hook is process-global state and this one exits the process on a closed pipe, so it
/// belongs to a binary entry point — `main` calls it first, and no library installs one. The `rpt`
/// binary carries the same few lines: neither `apps/` crate has a library target to share one
/// through, and pushing it back into a library is what this replaces.
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
