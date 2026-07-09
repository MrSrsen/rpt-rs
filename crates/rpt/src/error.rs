//! The single error type shared across every layer.
//!
//! Variants are *layer-tagged* (`Container`, `Codec`, `Crypto`, `Record`, `Project`, `Io`)
//! so a caller can tell "the CFB container is malformed" from "the record framing is
//! malformed" from "this edit can't be safely written yet". Each layer error carries
//! **structured context** — the stream, byte offset, and/or record type it failed at — so a
//! bug report says *where* and *what* failed, not just a sentence. Context is best-effort: a
//! construction site fills in only what it genuinely has and never fabricates an offset.

use std::fmt;

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

    /// L1 — a record's logical content was malformed for its (known) tag.
    #[error(transparent)]
    Record(#[from] RecordError),

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

located_error!(
    /// L1 — a record's logical content was malformed for its (known) tag.
    RecordError,
    "record"
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
}
