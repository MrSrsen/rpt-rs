//! The single error type shared across every layer.
//!
//! Variants are *layer-tagged* (`Container`, `Codec`, `Crypto`, `Record`, `Project`, `Io`)
//! so a caller can tell "the CFB container is malformed" from "the record framing is
//! malformed" from "this edit can't be safely written yet".

use std::fmt;

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong opening, decoding, projecting, or saving an `.rpt`.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// L0 — the CFB/OLE2 compound-file container is malformed or a stream is missing.
    #[error("container: {0}")]
    Container(String),

    /// L0.5/L1 — the stream header or TSLV record framing could not be decoded.
    #[error("codec: {0}")]
    Codec(String),

    /// L0.5 — the cipher path for a password-protected stream failed.
    #[error("crypto: {0}")]
    Crypto(String),

    /// L1 — a record's logical content was malformed for its (known) tag.
    #[error("record: {0}")]
    Record(String),

    /// L2 — projecting records ⇄ model failed. `kind` distinguishes a genuine failure
    /// from an edit we refuse to write because the record type isn't cleared yet.
    #[error("project: {kind}: {detail}")]
    Project {
        /// What kind of projection problem this is.
        kind: ProjectErrorKind,
        /// Human-readable context.
        detail: String,
    },

    /// Underlying I/O failure.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Distinguishes projection failures so callers can match on "can't safely write this yet"
/// versus genuine corruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProjectErrorKind {
    /// `raise` met a record it could not interpret where it expected a known one.
    UnknownRecord,
    /// An edit would touch in-record offset tables/counts/checksums of a record type that is
    /// not yet cleared for safe editing. Refused, never written.
    UnclearedRecordEdit,
}

impl fmt::Display for ProjectErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ProjectErrorKind::UnknownRecord => "unknown record",
            ProjectErrorKind::UnclearedRecordEdit => "uncleared record edit",
        };
        f.write_str(s)
    }
}

impl Error {
    pub(crate) fn container(msg: impl Into<String>) -> Self {
        Error::Container(msg.into())
    }

    pub(crate) fn codec(msg: impl Into<String>) -> Self {
        Error::Codec(msg.into())
    }

    pub(crate) fn crypto_msg(msg: impl Into<String>) -> Self {
        Error::Crypto(msg.into())
    }
}
