//! Output: turning a paginated document into files or stdout in the chosen format.
//!
//! HTML and PDF are a single self-contained document (safe to pipe to stdout); SVG and PNG are one
//! file per page, written as `<base>-N.<ext>` with `--force`-guarded cleanup of a prior run's stale
//! higher-numbered pages.

use std::io::{IsTerminal, Write};
use std::path::Path;

use crate::error::RenderError;

use crate::applog::{Comp, Log};
use crate::{Dest, Format};

/// Write the paginated document to the destination in the chosen format.
pub(crate) fn write_output(
    dest: &Dest,
    format: Format,
    doc: &rpt_pages::PagedDocument,
    force: bool,
    log: &Log,
) -> Result<(), RenderError> {
    match format {
        Format::Html => {
            let html =
                rpt_render::render_backend(doc, &rpt_render::HtmlBackend, &rpt_render::HtmlOptions);
            write_bytes(dest, html.as_bytes(), format, log)
        }
        Format::Pdf => {
            let pdf = rpt_render::render_backend(
                doc,
                &rpt_render::PdfBackend,
                &rpt_render::PdfOptions::default(),
            );
            write_bytes(dest, &pdf, format, log)
        }
        // SVG is text (pipeable to a terminal); PNG is binary (guarded). Both embed the document's
        // out-of-band image assets: SVG inlines each distinct image once per page, PNG decodes each
        // distinct image once for the whole document (a shared cache) and composites it per placement.
        Format::Svg => {
            let pages = doc
                .pages
                .iter()
                .map(|p| rpt_render_svg::render_page_with_assets(p, &doc.assets).into_bytes())
                .collect();
            write_numbered_pages(dest, "svg", false, force, log, pages)
        }
        Format::Png => {
            let pages = rpt_render_raster::render_pages_with_assets(
                &doc.pages,
                &doc.assets,
                rpt_render_raster::DEFAULT_DPI,
            );
            write_numbered_pages(dest, "png", true, force, log, pages)
        }
    }
}

/// Write bytes to a file or stdout, guarding against dumping binary to a terminal.
fn write_bytes(dest: &Dest, bytes: &[u8], format: Format, log: &Log) -> Result<(), RenderError> {
    match dest {
        Dest::File(path) => {
            std::fs::write(path, bytes)
                .map_err(|e| RenderError::Io(format!("cannot write {path:?}"), e))?;
            log.info(
                Comp::Render,
                format!("wrote {path} ({}, {} bytes)", format.name(), bytes.len()),
            );
            Ok(())
        }
        Dest::Stdout => {
            if format == Format::Pdf && std::io::stdout().is_terminal() {
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

/// Write a one-file-per-page format (SVG/PNG) from pre-rendered per-page byte strings. To stdout it
/// only works for a single-page report (you cannot pipe multiple files), and a `binary` format
/// additionally refuses a terminal. To a file `base` it writes `<base>-N.<ext>`; because a shorter
/// re-render would otherwise leave the previous run's higher-numbered pages behind, it refuses a base
/// that already has `<base>-N.<ext>` pages unless `force`, and with `force` deletes the stale
/// siblings first so the directory reflects exactly this render.
fn write_numbered_pages(
    dest: &Dest,
    ext: &str,
    binary: bool,
    force: bool,
    log: &Log,
    pages: Vec<Vec<u8>>,
) -> Result<(), RenderError> {
    let name = ext.to_ascii_uppercase();
    match dest {
        Dest::Stdout => match pages.as_slice() {
            [page] => {
                if binary && std::io::stdout().is_terminal() {
                    return Err(RenderError::Output(format!(
                        "refusing to write binary {name} to a terminal; redirect to a file \
                         or use -o <path>"
                    )));
                }
                std::io::stdout()
                    .write_all(page)
                    .map_err(|e| RenderError::Io("cannot write to stdout".to_string(), e))
            }
            pages => Err(RenderError::Output(format!(
                "{name} is one file per page ({} pages) and multiple files cannot be piped; \
                 specify -o <base> to write <base>-N.{ext}",
                pages.len()
            ))),
        },
        Dest::File(path) => {
            let base = path.strip_suffix(&format!(".{ext}")).unwrap_or(path);
            let stale = existing_numbered_pages(base, ext);
            if !stale.is_empty() && !force {
                return Err(RenderError::Output(format!(
                    "{} existing {base}-N.{ext} page(s) would be overwritten; pass --force to \
                     replace them (stale higher-numbered pages are removed first)",
                    stale.len()
                )));
            }
            for stale_name in &stale {
                let _ = std::fs::remove_file(stale_name);
            }
            for (i, page) in pages.iter().enumerate() {
                let page_name = format!("{base}-{}.{ext}", i + 1);
                std::fs::write(&page_name, page)
                    .map_err(|e| RenderError::Io(format!("cannot write {page_name:?}"), e))?;
            }
            log.info(
                Comp::Render,
                format!(
                    "wrote {} {name} page(s) as {base}-N.{ext}{}",
                    pages.len(),
                    if stale.is_empty() {
                        String::new()
                    } else {
                        format!(" (replaced {} stale page file(s))", stale.len())
                    }
                ),
            );
            Ok(())
        }
    }
}

/// The existing `<base>-N.<ext>` sibling files for a given base name (page number `N` ≥ 1), so a
/// re-render can detect and clean a prior run's pages. Returns full paths; empty if none/unreadable.
fn existing_numbered_pages(base: &str, ext: &str) -> Vec<String> {
    let path = Path::new(base);
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let prefix = format!(
        "{}-",
        path.file_name().and_then(|n| n.to_str()).unwrap_or(base)
    );
    let suffix = format!(".{ext}");
    let read = match std::fs::read_dir(dir.unwrap_or_else(|| Path::new("."))) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let fname = entry.file_name();
        let Some(name) = fname.to_str() else { continue };
        if let Some(mid) = name
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix(&suffix))
        {
            if !mid.is_empty() && mid.bytes().all(|b| b.is_ascii_digit()) {
                out.push(entry.path().to_string_lossy().into_owned());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn existing_numbered_pages_matches_only_numbered_siblings() {
        // Unique scratch dir per test run (no time/rand needed).
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rpt-render-svgtest-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("pg");
        let base = base.to_str().unwrap();

        // Matches: pg-1.svg, pg-12.svg. Non-matches: different stem, non-numeric, wrong ext.
        for f in [
            "pg-1.svg",
            "pg-12.svg",
            "pg-x.svg",
            "pgg-3.svg",
            "pg-2.txt",
            "pg.svg",
        ] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let mut found: Vec<String> = existing_numbered_pages(base, "svg")
            .into_iter()
            .map(|p| {
                Path::new(&p)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        found.sort();
        assert_eq!(found, vec!["pg-1.svg".to_string(), "pg-12.svg".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn existing_numbered_pages_empty_when_none() {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rpt-render-svgtest-empty-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("pg");
        assert!(existing_numbered_pages(base.to_str().unwrap(), "svg").is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
