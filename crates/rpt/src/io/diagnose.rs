//! Turning "this is not a report" into an answer the user can act on.
//!
//! The failure classes are cheaply distinguishable at the point the container opens, and each maps
//! to a different next step: a file with no OLE2/CFB signature is the wrong file (so say what it
//! looks like instead), a compound file with no `Contents` stream is some other OLE2 document (so
//! list what it *does* carry), and a `Contents` stream that will not decrypt or inflate is either
//! damaged or a format this reader does not handle.
//!
//! Without this layer the user sees the CFB library's internal complaint — `Invalid CFB file (wrong
//! magic number)` — which is meaningless to anyone who does not already know that `.rpt` is an OLE2
//! compound file.

use crate::error::{ContainerError, Error, NotAReportError};

/// The OLE2/CFB compound-file signature every `.rpt` starts with.
const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Diagnose a failure to open `bytes` as a compound file.
///
/// Only a *container*-layer failure is re-framed as a diagnosis — that is the one whose raw message
/// (`Invalid CFB file (wrong magic number)`) is unusable. Anything else passes through untouched
/// rather than being relabelled as "not a report" on thin evidence.
pub(crate) fn open_failure(bytes: &[u8], cause: Error) -> Error {
    match cause {
        Error::Container(ce) => not_a_report(bytes, ce).into(),
        other => other,
    }
}

/// Explain why `bytes` are not a report.
///
/// Distinguishes "you pointed the tool at the wrong file" (no CFB signature — the common case, and
/// the one where naming the *actual* format is the whole answer) from "this really is a compound
/// file but a malformed one", where the CFB library's message is the useful detail and is kept as
/// the cause.
fn not_a_report(bytes: &[u8], cause: ContainerError) -> NotAReportError {
    if bytes.starts_with(&CFB_MAGIC) {
        return NotAReportError {
            reason: format!(
                "the file carries the OLE2/CFB signature but the compound file is malformed \
                 or truncated ({} bytes)",
                bytes.len()
            ),
            looks_like: None,
            hint: Some(
                "if the file was copied or downloaded, check it transferred completely.".into(),
            ),
            source: Some(cause),
            path: None,
        };
    }
    NotAReportError {
        reason: if bytes.len() < CFB_MAGIC.len() {
            format!("it is only {} bytes — far too small to be one", bytes.len())
        } else {
            "it has no OLE2/CFB signature, which every `.rpt` starts with".to_string()
        },
        looks_like: sniff(bytes),
        hint: None,
        source: None,
        path: None,
    }
}

/// A compound file that carries no `Contents` stream is some other OLE2 document, not a report.
/// Listing the streams it *does* carry is what identifies it (a `WordDocument` stream says `.doc`,
/// a `Workbook` stream says `.xls`).
pub(crate) fn no_contents_stream(present: &[String]) -> NotAReportError {
    let listed = if present.is_empty() {
        "it has no streams at all".to_string()
    } else {
        format!("the streams it does carry are: {}", present.join(", "))
    };
    NotAReportError {
        reason: format!(
            "it is a valid OLE2 compound file but carries no `Contents` stream, which holds the \
             report definition — {listed}"
        ),
        looks_like: None,
        hint: None,
        source: None,
        path: None,
    }
}

/// The `Contents` stream is present but its cipher or deflate layer failed, so there is no report
/// definition to read. Damage and an unhandled format are both possible and the reader cannot tell
/// them apart, so the message offers both rather than guessing.
pub(crate) fn contents_undecodable(detail: &str) -> NotAReportError {
    NotAReportError {
        reason: format!(
            "its `Contents` stream could not be decoded ({detail}), so the report definition \
             cannot be read"
        ),
        looks_like: None,
        hint: Some(
            "the stream is either damaged or written in a format this reader does not handle; \
             `rpt streams <file>` shows what did decode."
                .into(),
        ),
        source: None,
        path: None,
    }
}

/// What the leading bytes say the file actually is, for the "wrong file" case. `None` when nothing
/// recognizable matches — a guess would be worse than silence.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    const MAGIC: &[(&[u8], &str)] = &[
        (
            b"PK\x03\x04",
            "a ZIP archive (or a ZIP-based format such as .docx/.xlsx)",
        ),
        (b"PK\x05\x06", "an empty ZIP archive"),
        (b"%PDF-", "a PDF document"),
        (b"\x89PNG\r\n\x1a\n", "a PNG image"),
        (b"\xff\xd8\xff", "a JPEG image"),
        (b"GIF87a", "a GIF image"),
        (b"GIF89a", "a GIF image"),
        (b"BM", "a BMP image"),
        (b"\x01\x00\x00\x00", "a Windows EMF metafile"),
        (b"{\\rtf", "an RTF document"),
        (b"\x1f\x8b", "a gzip stream"),
        (b"SQLite format 3\0", "a SQLite database"),
        (b"\x7fELF", "an ELF executable"),
        (b"MZ", "a DOS/Windows executable"),
        (b"<?xml", "an XML document"),
    ];
    for (magic, what) in MAGIC {
        if bytes.starts_with(magic) {
            return Some(what);
        }
    }
    // HTML has no fixed magic; match the two openings that actually occur, case-insensitively.
    let head: Vec<u8> = bytes.iter().take(64).map(u8::to_ascii_lowercase).collect();
    if head.starts_with(b"<!doctype html") || head.starts_with(b"<html") {
        return Some("an HTML document");
    }
    // Anything left that decodes as UTF-8 and is mostly printable is plain text — the single most
    // common wrong file (a CSV, a log, a SQL script).
    let sample = &bytes[..bytes.len().min(512)];
    let printable = |b: &u8| b.is_ascii_graphic() || matches!(b, b' ' | b'\t' | b'\r' | b'\n');
    if !bytes.is_empty()
        && std::str::from_utf8(sample).is_ok()
        && sample.iter().filter(|b| printable(b)).count() * 10 >= sample.len() * 9
    {
        return Some("plain text");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_the_formats_users_actually_point_at_the_tool() {
        assert_eq!(sniff(b"%PDF-1.7\n..."), Some("a PDF document"));
        assert_eq!(
            sniff(b"PK\x03\x04rest"),
            Some("a ZIP archive (or a ZIP-based format such as .docx/.xlsx)")
        );
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n"), Some("a PNG image"));
        assert_eq!(sniff(b"hello world"), Some("plain text"));
        assert_eq!(sniff(b"<!DOCTYPE HTML><body>"), Some("an HTML document"));
        assert_eq!(sniff(b"id,name\n1,foo\n"), Some("plain text"));
    }

    #[test]
    fn sniff_stays_silent_rather_than_guessing() {
        // High-entropy binary matches nothing — no guess is better than a wrong one.
        assert_eq!(sniff(&[0x9f, 0x00, 0xe3, 0xff, 0x81, 0x12]), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn a_short_file_is_diagnosed_by_its_length() {
        let e = not_a_report(
            b"hello",
            ContainerError::new("open compound file", "too small"),
        );
        let msg = e.to_string();
        assert!(msg.contains("only 5 bytes"), "{msg}");
        assert!(msg.contains("plain text"), "{msg}");
        // The CFB library's own complaint is not the answer here, so it is not the cause either.
        assert!(std::error::Error::source(&e).is_none());
    }

    #[test]
    fn a_real_compound_file_keeps_the_cfb_complaint_as_the_cause() {
        let mut bytes = CFB_MAGIC.to_vec();
        bytes.extend_from_slice(b"truncated here");
        let e = not_a_report(
            &bytes,
            ContainerError::new("open compound file", "unexpected EOF"),
        );
        assert!(e.to_string().contains("malformed or truncated"));
        let cause = std::error::Error::source(&e).expect("the CFB message is the useful detail");
        assert!(cause.to_string().contains("unexpected EOF"));
    }

    #[test]
    fn a_compound_file_without_contents_lists_what_it_does_carry() {
        let e = no_contents_stream(&["WordDocument".to_string(), "1Table".to_string()]);
        let msg = e.to_string();
        assert!(msg.contains("no `Contents` stream"), "{msg}");
        assert!(msg.contains("WordDocument, 1Table"), "{msg}");
    }
}
