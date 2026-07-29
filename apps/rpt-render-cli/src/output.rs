//! Output: turning a paginated document into a file or stdout.
//!
//! PDF is the only output format, and it is a single self-contained document, so writing is one
//! blob to one destination.

use std::io::{IsTerminal, Write};

use crate::error::RenderError;

use crate::applog::{Comp, Log};
use crate::Dest;

/// Write the paginated document to the destination. `opts` carries the face library the backend embeds
/// from — the same one the layout pass measured with, so the advances text was placed to are the
/// advances it is drawn with — and the archival level, if any, the bytes must satisfy.
pub(crate) fn write_output(
    dest: &Dest,
    doc: &rpt_pages::PagedDocument,
    opts: &rpt_render::PdfOptions,
    log: &Log,
) -> Result<(), RenderError> {
    // The fallible entry point, not the backend seam: a resource krilla will not embed, or an unmet
    // archival requirement, should fail the command with the cause rather than write a document that
    // reports it on a page.
    let pdf = rpt_render::try_render_document(doc, opts).map_err(backend_error)?;
    write_bytes(dest, &pdf, opts.conformance, log)
}

/// The backend's failure as the CLI's error.
///
/// A conformance failure gets its own variant rather than a flattened string: it carries one message
/// per unmet requirement, and those belong on their own lines beneath the error the way a driver's
/// hint does — a single `; `-joined line is unreadable once a document misses more than one.
fn backend_error(err: rpt_render::PdfError) -> RenderError {
    match err {
        rpt_render::PdfError::Conformance { level, reasons } => RenderError::Conformance {
            hint: conformance_hint(level, &reasons),
            level,
            unmet: reasons.len(),
        },
        other => RenderError::Output(other.to_string()),
    }
}

/// The lines printed beneath a conformance failure: every unmet requirement, then the note that
/// makes the level's own peculiarity legible — for a tagging level, which flag supplies each thing
/// the report does not state; for PDF/A-1b, that the two things it alone forbids are legal at the
/// later levels, so a report that fails `--pdfa 1b` and passes `--pdfa 2b` reads as the standards
/// differing rather than a bug.
fn conformance_hint(level: rpt_render::Conformance, reasons: &[String]) -> String {
    let mut out = String::new();
    for r in reasons {
        out.push_str(&format!("  - {r}\n"));
    }
    // The backend names each missing fact in its own API's terms, because it has no command line.
    // Translate once, here, into the flags that supply them.
    if level.requires_tagging() {
        out.push_str(
            "A .rpt does not state all of this, so it has to be supplied: --lang <tag> for the \
             document's natural language, --title <text> for its title, and --alt <Object>=<text> \
             for each figure named above (repeatable; an empty text marks a purely decorative \
             graphic). Drop the conformance flag and pass --tagged for a structure tree that claims \
             nothing and needs none of it.\n",
        );
    }
    if level == rpt_render::Conformance::PdfA1b {
        out.push_str(
            "PDF/A-1b is PDF 1.4: it forbids transparency and 16-bit images outright, both of which \
             PDF/A-2b and -3b allow. If the report legitimately uses either, export against \
             --pdfa 2b.",
        );
    }
    out.trim_end().to_string()
}

/// Write bytes to a file or stdout. PDF is the only single-document format, so the bytes are always
/// binary and a terminal destination is refused rather than filled with control codes.
fn write_bytes(
    dest: &Dest,
    bytes: &[u8],
    conformance: rpt_render::Conformance,
    log: &Log,
) -> Result<(), RenderError> {
    // A file that got this far met the level it claims, so the written line names the standard rather
    // than the generic format.
    let kind = match conformance {
        rpt_render::Conformance::None => "PDF",
        other => other.as_str(),
    };
    match dest {
        Dest::File(path) => {
            std::fs::write(path, bytes)
                .map_err(|e| RenderError::Io(format!("cannot write {path:?}"), e))?;
            log.info(
                Comp::Render,
                format!("wrote {path} ({kind}, {} bytes)", bytes.len()),
            );
            Ok(())
        }
        Dest::Stdout => {
            if std::io::stdout().is_terminal() {
                return Err(RenderError::Output(
                    "refusing to write binary PDF to a terminal; redirect to a file or use -o <path>"
                        .to_string(),
                ));
            }
            std::io::stdout()
                .write_all(bytes)
                .map_err(|e| RenderError::Io("cannot write to stdout".to_string(), e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_render::{Conformance, PdfError};

    /// A conformance failure becomes its own error, not an opaque output string: the message counts
    /// the unmet requirements and the hint lists each on its own line, so a document missing several
    /// is readable.
    #[test]
    fn a_conformance_failure_lists_every_unmet_requirement() {
        let err = backend_error(PdfError::Conformance {
            level: Conformance::PdfA2b,
            reasons: vec![
                "the first thing".to_string(),
                "the second thing".to_string(),
            ],
        });
        let RenderError::Conformance { level, unmet, .. } = &err else {
            panic!("expected a conformance error, got {err:?}");
        };
        assert_eq!(*level, Conformance::PdfA2b);
        assert_eq!(*unmet, 2);

        let msg = err.to_string();
        assert!(msg.contains("PDF/A-2B") && msg.contains('2'), "{msg}");
        assert!(
            msg.contains("no file written"),
            "the message must say nothing was written: {msg}"
        );

        let hint = err.hint().expect("the reasons ride in the hint");
        assert_eq!(
            hint.lines().count(),
            2,
            "one line per unmet requirement: {hint}"
        );
        assert!(
            hint.contains("- the first thing") && hint.contains("- the second thing"),
            "{hint}"
        );
    }

    /// Failing at PDF/A-1b is the standard being older, not the renderer being broken, so that level
    /// alone explains itself and points at the level that permits what it forbids.
    #[test]
    fn the_1b_failure_explains_that_2b_allows_what_1b_forbids() {
        let one_b = conformance_hint(
            Conformance::PdfA1b,
            &["the document paints with transparency".to_string()],
        );
        assert!(
            one_b.contains("--pdfa 2b") && one_b.contains("transparency"),
            "{one_b}"
        );

        // The later levels forbid nothing peculiar to themselves, so they get no such note.
        for level in [Conformance::PdfA2b, Conformance::PdfA3b] {
            let hint = conformance_hint(level, &["something".to_string()]);
            assert!(!hint.contains("--pdfa 2b"), "{level}: {hint}");
        }
    }

    /// A tagging level's refusal is written in the backend's API terms, because the backend has no
    /// command line. The hint translates it: every unmet requirement, then the flag that supplies
    /// each — and the way out for a caller who wanted a structure tree rather than a claim.
    #[test]
    fn a_tagging_refusal_names_the_flags_that_supply_what_is_missing() {
        let hint = conformance_hint(
            Conformance::PdfUa1,
            &[
                "the document declares no natural language (PdfOptions::semantics.language)"
                    .to_string(),
                "the figure \"Chart1\" has no alternate text describing it".to_string(),
            ],
        );
        assert!(
            hint.contains("- the document declares no natural language"),
            "{hint}"
        );
        assert!(
            hint.contains("Chart1"),
            "the figure to describe stays named: {hint}"
        );
        for flag in ["--lang", "--title", "--alt", "--tagged"] {
            assert!(hint.contains(flag), "{flag} missing from: {hint}");
        }
    }

    /// The archival level-B standards need none of that, so their failure says nothing about it.
    #[test]
    fn an_archival_refusal_does_not_mention_the_tagging_flags() {
        let hint = conformance_hint(Conformance::PdfA2b, &["something".to_string()]);
        assert!(
            !hint.contains("--lang") && !hint.contains("--alt"),
            "{hint}"
        );
    }

    /// Every other backend failure keeps its existing shape — an output error carrying the cause.
    #[test]
    fn other_backend_failures_stay_output_errors() {
        let err = backend_error(PdfError::Font("no outline table".to_string()));
        assert!(matches!(err, RenderError::Output(_)), "{err:?}");
        assert!(err.to_string().contains("no outline table"));
        assert_eq!(err.hint(), None);
    }
}
