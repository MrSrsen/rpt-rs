//! The single error type shared across every layer.
//!
//! Variants are *layer-tagged* (`Container`, `Codec`, `Crypto`, `NotAReport`, `Project`, `Io`)
//! so a caller can tell "the CFB container is malformed" from "the record framing is
//! malformed" from "this edit can't be safely written yet". Each layer error carries
//! **structured context** — the stream, byte offset, and/or record type it failed at — so a
//! bug report says *where* and *what* failed, not just a sentence. Context is best-effort: a
//! construction site fills in only what it genuinely has and never fabricates an offset.
//!
//! # Projection does not fail
//!
//! There is deliberately no variant for "a record could not be interpreted". The `raise` layer
//! (records → semantic model) is **infallible by design**: it returns defaults for anything it
//! cannot interpret, so a report the Crystal engine opens happily is never refused here over a
//! record this reader does not yet model. That is the right trade for a reader of an undocumented
//! format — but it means a decode gap is *invisible* in the error type, so it is
//! reported as a **diagnostic** instead (see [`Rpt::decode_coverage`](crate::Rpt::decode_coverage)).
//!
//! `Project` therefore covers only the write path, where refusing is the whole point.
//!
//! # Displaying an error
//!
//! A variant that carries a [`source`](std::error::Error::source) **never interpolates it** into
//! its own `Display`, so printing `{e}` alone shows this layer's message and printing the
//! `source()` chain shows each cause exactly once. Use
//! [`error_chain`](crate::error_chain) to render the whole chain — a bare `{e}` on a
//! wrapping variant is the layer's message *without* the underlying cause.

use std::fmt;
use std::path::{Path, PathBuf};

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong opening, decoding, projecting, or saving an `.rpt`.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// L0 — the CFB/OLE2 compound-file container is malformed or a stream is missing.
    #[error(transparent)]
    Container(#[from] ContainerError),

    /// L0.5/L1 — the stream header or TSLV record framing could not be decoded.
    #[error(transparent)]
    Codec(#[from] CodecError),

    /// L0.5 — the cipher path for a password-protected stream failed.
    #[error(transparent)]
    Crypto(#[from] CryptoError),

    /// L2 — the write path refused an edit. `kind` says which rule refused it; `detail` says why in
    /// terms the caller can act on.
    #[error("project: {kind}: {detail}")]
    Project {
        /// What kind of projection problem this is.
        kind: ProjectErrorKind,
        /// Human-readable context.
        detail: String,
    },

    /// L0 — the input is not a Crystal Reports report at all. Carries a diagnosis of what it
    /// appears to be instead, so the commonest mistake (the wrong file) answers itself.
    ///
    /// Boxed to keep [`Error`] — and so every `Result` in the crate — small; the diagnosis is the
    /// one variant that carries several strings.
    #[error(transparent)]
    NotAReport(#[from] Box<NotAReportError>),

    /// Underlying I/O failure, naming the operation and (when one was in scope) the path.
    #[error(transparent)]
    Io(#[from] IoError),
}

impl From<NotAReportError> for Error {
    fn from(e: NotAReportError) -> Error {
        Error::NotAReport(Box::new(e))
    }
}

impl Error {
    /// Attach `path` to a diagnosis raised without one.
    ///
    /// [`Rpt::from_bytes`](crate::Rpt) works on bytes and has no path to name; [`Rpt::open`] does,
    /// and fills it in on the way out.
    pub(crate) fn at_path(mut self, path: &Path) -> Error {
        if let Error::NotAReport(e) = &mut self {
            e.path = Some(path.to_path_buf());
        }
        self
    }
}

/// The input is not a Crystal Reports report, with a diagnosis of what it is instead.
///
/// Every field beyond `reason` is best-effort and omitted when the reader genuinely cannot tell —
/// a wrong guess about the format is worse than no guess.
#[derive(Debug)]
pub struct NotAReportError {
    /// The file that was opened, when a path was in scope (absent for the byte/reader entry points).
    pub path: Option<PathBuf>,
    /// Why the input was rejected.
    pub reason: String,
    /// What the leading bytes suggest the file actually is (e.g. `a PDF document`), if recognized.
    pub looks_like: Option<&'static str>,
    /// What the user can do next, when there is something concrete to suggest.
    pub hint: Option<String>,
    /// The underlying container failure, kept only where its message is the useful detail (a file
    /// that really is a compound file, but a malformed one).
    pub source: Option<ContainerError>,
}

impl fmt::Display for NotAReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(f, "`{}` is not a Crystal Reports report", path.display())?,
            None => write!(f, "the input is not a Crystal Reports report")?,
        }
        write!(f, ": {}", self.reason)?;
        if let Some(looks_like) = self.looks_like {
            write!(f, "; it looks like {looks_like}")?;
        }
        if let Some(hint) = &self.hint {
            write!(f, ". {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for NotAReportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

/// An I/O failure that names *what* was being done and *which path* it targeted, so the most
/// common user error — a wrong or missing filename — produces a message that identifies the file.
///
/// `path` is `None` only where there genuinely is no path: [`Rpt::read`](crate::Rpt::read) and
/// [`Rpt::write`](crate::Rpt::write) take a caller-supplied reader/writer.
#[derive(Debug)]
pub struct IoError {
    /// The operation that failed, as an infinitive phrase (`read`, `write`).
    pub op: &'static str,
    /// The path the operation targeted, when one was in scope.
    pub path: Option<PathBuf>,
    /// The underlying I/O error. Also reported as this error's [`source`](std::error::Error::source),
    /// so it is never interpolated into `Display`.
    pub source: std::io::Error,
}

impl IoError {
    /// An I/O failure for `op` against `path`.
    pub fn at(op: &'static str, path: impl AsRef<Path>, source: std::io::Error) -> Self {
        IoError {
            op,
            path: Some(path.as_ref().to_path_buf()),
            source,
        }
    }

    /// An I/O failure for `op` with no path in scope (a caller-supplied reader or writer).
    pub fn new(op: &'static str, source: std::io::Error) -> Self {
        IoError {
            op,
            path: None,
            source,
        }
    }
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(f, "cannot {} `{}`", self.op, path.display()),
            None => write!(f, "cannot {}", self.op),
        }
    }
}

impl std::error::Error for IoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Which write-path rule refused an edit, so a caller can match on the reason rather than on a
/// message string. `#[non_exhaustive]`: more rules will be added as more of the format is cleared for
/// editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProjectErrorKind {
    /// An edit would touch in-record offset tables/counts/checksums of a record type that is
    /// not yet cleared for safe editing. Refused, never written.
    UnclearedRecordEdit,
}

impl fmt::Display for ProjectErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ProjectErrorKind::UnclearedRecordEdit => "uncleared record edit",
        };
        f.write_str(s)
    }
}

/// Best-effort location of a decode failure within a stream. A construction site fills in only
/// the context it genuinely has; every field is optional and never fabricated.
#[derive(Debug, Default, Clone)]
pub struct StreamLoc {
    /// The stream the failure occurred in (e.g. `Contents`, `QESession`), if known.
    pub stream: Option<String>,
    /// The byte offset within the decoded stream, if known.
    pub offset: Option<usize>,
    /// The TSLV record type in play, if known.
    pub rtype: Option<u16>,
}

impl StreamLoc {
    fn is_empty(&self) -> bool {
        self.stream.is_none() && self.offset.is_none() && self.rtype.is_none()
    }
}

impl fmt::Display for StreamLoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut sep = "";
        if let Some(stream) = &self.stream {
            write!(f, "{sep}stream `{stream}`")?;
            sep = ", ";
        }
        if let Some(offset) = self.offset {
            write!(f, "{sep}offset {offset:#x}")?;
            sep = ", ";
        }
        if let Some(rtype) = self.rtype {
            write!(f, "{sep}record {rtype:#06x}")?;
        }
        Ok(())
    }
}

/// L0 — a CFB/OLE2 container operation (open/read/write/resize/find a stream, flush the file) failed.
#[derive(Debug, Clone)]
pub struct ContainerError {
    /// The container operation that failed (e.g. `open stream`, `flush compound file`).
    pub op: &'static str,
    /// The stream the operation targeted, if it was stream-scoped.
    pub stream: Option<String>,
    /// Underlying cause or extra context (may be empty).
    pub detail: String,
}

impl ContainerError {
    /// A container failure for `op` with a human-readable `detail` (the underlying cause).
    pub fn new(op: &'static str, detail: impl Into<String>) -> Self {
        ContainerError {
            op,
            stream: None,
            detail: detail.into(),
        }
    }

    /// Note the stream the failing operation targeted.
    pub fn stream(mut self, stream: impl fmt::Display) -> Self {
        self.stream = Some(stream.to_string());
        self
    }
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "container: {}", self.op)?;
        if let Some(stream) = &self.stream {
            write!(f, " (stream `{stream}`)")?;
        }
        if !self.detail.is_empty() {
            write!(f, ": {}", self.detail)?;
        }
        Ok(())
    }
}

impl std::error::Error for ContainerError {}

/// L0.5 — the cipher path for a password-protected stream failed at a named `stage`.
#[derive(Debug, Clone)]
pub struct CryptoError {
    /// Which crypto stage failed (e.g. `stream header`, `QENG header`).
    pub stage: &'static str,
    /// What specifically went wrong.
    pub detail: String,
}

impl CryptoError {
    /// A crypto failure at `stage` with a human-readable `detail`.
    pub fn new(stage: &'static str, detail: impl Into<String>) -> Self {
        CryptoError {
            stage,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "crypto: {}: {}", self.stage, self.detail)
    }
}

impl std::error::Error for CryptoError {}

/// Define a stream-located decode error (`{loc}` + `detail`) with builder-style context setters.
macro_rules! located_error {
    ($(#[$meta:meta])* $name:ident, $label:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone)]
        pub struct $name {
            /// Where in the stream decoding failed (best-effort).
            pub loc: StreamLoc,
            /// What specifically went wrong.
            pub detail: String,
        }

        impl $name {
            /// A failure described by `detail`, with no location context yet.
            pub fn new(detail: impl Into<String>) -> Self {
                $name {
                    loc: StreamLoc::default(),
                    detail: detail.into(),
                }
            }

            /// Note the byte offset within the decoded stream where the failure occurred.
            pub fn at(mut self, offset: usize) -> Self {
                self.loc.offset = Some(offset);
                self
            }

            /// Note the stream the failure occurred in.
            pub fn in_stream(mut self, stream: impl fmt::Display) -> Self {
                self.loc.stream = Some(stream.to_string());
                self
            }

            /// Note the TSLV record type in play when the failure occurred.
            pub fn record(mut self, rtype: u16) -> Self {
                self.loc.rtype = Some(rtype);
                self
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if self.loc.is_empty() {
                    write!(f, concat!($label, ": {}"), self.detail)
                } else {
                    write!(f, concat!($label, " at {}: {}"), self.loc, self.detail)
                }
            }
        }

        impl std::error::Error for $name {}
    };
}

located_error!(
    /// L0.5/L1 — the stream header or TSLV record framing could not be decoded.
    CodecError,
    "codec"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_display_includes_op_and_stream() {
        let e = ContainerError::new("open stream", "not found").stream("/Contents");
        assert_eq!(
            e.to_string(),
            "container: open stream (stream `/Contents`): not found"
        );
    }

    #[test]
    fn container_display_without_stream_or_detail() {
        let e = ContainerError::new("flush compound file", "");
        assert_eq!(e.to_string(), "container: flush compound file");
    }

    #[test]
    fn crypto_display_includes_stage() {
        let e = CryptoError::new("QENG header", "IV is 12 bytes, expected 16");
        assert_eq!(
            e.to_string(),
            "crypto: QENG header: IV is 12 bytes, expected 16"
        );
    }

    #[test]
    fn codec_display_bare_has_no_location() {
        let e = CodecError::new("inflate failed");
        assert_eq!(e.to_string(), "codec: inflate failed");
    }

    #[test]
    fn codec_display_renders_full_location() {
        let e = CodecError::new("record not found")
            .in_stream("Contents")
            .at(0x1234)
            .record(0x00fa);
        assert_eq!(
            e.to_string(),
            "codec at stream `Contents`, offset 0x1234, record 0x00fa: record not found"
        );
    }

    #[test]
    fn located_error_converts_into_error_via_from() {
        let err: Error = CodecError::new("boom").record(0x10).into();
        assert!(matches!(err, Error::Codec(_)));
    }

    fn not_found() -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory")
    }

    #[test]
    fn io_error_names_the_path_and_the_operation() {
        let e = IoError::at("read", "/nope/missing.rpt", not_found());
        assert_eq!(e.to_string(), "cannot read `/nope/missing.rpt`");
    }

    #[test]
    fn io_error_without_a_path_still_names_the_operation() {
        let e = IoError::new("read the report from the supplied reader", not_found());
        assert_eq!(
            e.to_string(),
            "cannot read the report from the supplied reader"
        );
    }

    // The convention this module documents: a variant carrying a `source` never interpolates it, so
    // the cause appears exactly once when the chain is walked.
    #[test]
    fn io_error_keeps_the_cause_as_a_source_and_never_interpolates_it() {
        let err: Error = IoError::at("read", "/nope/missing.rpt", not_found()).into();
        let top = err.to_string();
        assert_eq!(top, "cannot read `/nope/missing.rpt`");
        assert!(!top.contains("No such file"), "cause leaked into Display");

        let cause = std::error::Error::source(&err).expect("the io::Error is the source");
        assert_eq!(cause.to_string(), "No such file or directory");
        assert_eq!(
            crate::error_chain(&err),
            "cannot read `/nope/missing.rpt`: No such file or directory"
        );
    }
}
