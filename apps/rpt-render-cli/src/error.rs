//! The CLI's typed render error.
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
    Rpt(#[from] rpt::Error),
    /// Resolving the datasource failed: a connection URL's scheme/format, a missing connection URL,
    /// an unimplemented driver, or a scope with no live table. A synthesized, descriptive message
    /// (there is no underlying error to chain).
    #[error("datasource error: {0}")]
    Datasource(String),
    /// A report parameter value could not be coerced to its declared type.
    #[error("parameter error: {0}")]
    Params(String),
    /// A database driver failed (connection, healthcheck, or query). Carries the driver's own error
    /// as the source, so the cause chain survives rather than being flattened to a string.
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
}

impl RenderError {
    /// Name the data source this failure happened against.
    ///
    /// A report plus its subreports can read from several servers, each configured by its own
    /// `RPT_DB_URL_<SERVER>`; without this a runtime failure does not say which one is wrong. Applied
    /// at the dispatch boundary, which is where the server key is known — the driver itself only sees
    /// a URL.
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
            RenderError::Db { hint, .. } => hint.as_deref(),
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
/// Delegates to [`rpt::error_chain`]: the reporter is shared with the `rpt` binary so both report
/// to one standard, and library logic does not live in an `apps/` crate.
pub fn full_chain(err: &dyn Error) -> String {
    rpt::error_chain(err)
}
