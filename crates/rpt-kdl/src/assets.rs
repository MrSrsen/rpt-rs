//! Binary asset extraction — the picture bytes a report embeds, kept *out* of the KDL.
//!
//! The KDL never carries raw binary: a picture node instead emits a scope-relative `source="…"`
//! reference (see [`picture_reference`]), and this module returns the matching bytes in memory so a
//! caller can write them as sidecar files. The crate performs no I/O itself, so it stays WASM-safe;
//! the reference in the KDL and the [`Asset::path`] here are produced from the same key, so they line
//! up file-for-file.

use rpt_model::{ImageFormat, PictureObject, Report, ReportObjectKind};

/// One extracted binary payload: the sidecar file path a caller should write it to, plus its bytes.
///
/// [`path`](Self::path) is scope-qualified (a subreport's assets sit under a `sub-N/` prefix) so it is
/// unique across the whole report tree; the KDL reference on the picture node is the same path
/// relative to that picture's report scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// The suggested sidecar file path, relative to the KDL document (e.g. `embed-1.bmp`,
    /// `sub-1/embed-2.png`).
    pub path: String,
    /// The renderable image file bytes (a bare DIB is wrapped into a valid `.bmp`).
    pub bytes: Vec<u8>,
}

/// Every embedded picture payload in `report` and its subreports, in document order.
///
/// Pair this with [`to_kdl_string`](crate::to_kdl_string): write the KDL to `report.kdl`, then write
/// each asset's [`bytes`](Asset::bytes) to its [`path`](Asset::path) alongside it. Reports with no
/// embedded picture bytes yield an empty vector.
pub fn assets(report: &Report) -> Vec<Asset> {
    let mut out = Vec::new();
    collect(report, "", &mut out);
    out
}

fn collect(report: &Report, prefix: &str, out: &mut Vec<Asset>) {
    for obj in report.objects() {
        if let ReportObjectKind::Picture(p) = &obj.kind {
            if let (Some(reference), Some(bytes)) = (picture_reference(p, &obj.name), p.to_bmp()) {
                out.push(Asset {
                    path: format!("{prefix}{reference}"),
                    bytes: bytes.into_owned(),
                });
            }
        }
    }
    for (i, sub) in report.subreports.iter().enumerate() {
        collect(&sub.report, &format!("{prefix}sub-{}/", i + 1), out);
    }
}

/// The scope-relative reference (and sidecar file name) for a picture's bytes, or `None` when the
/// picture carries no embedded payload. Keyed by the OLE embedding ordinal when present (unique within
/// a report scope), else by the object name.
pub(crate) fn picture_reference(p: &PictureObject, name: &str) -> Option<String> {
    if p.data.is_empty() {
        return None;
    }
    let ext = extension(p.image_format());
    let base = match p.ole_ordinal {
        Some(n) => format!("embed-{n}"),
        None => sanitize(name),
    };
    Some(format!("{base}.{ext}"))
}

/// A filesystem-safe stem from an object name (non-alphanumerics collapse to `-`).
fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = s.trim_matches('-');
    if trimmed.is_empty() {
        "picture".to_string()
    } else {
        trimmed.to_string()
    }
}

/// The file extension for an [`ImageFormat`] (a bare DIB is written as `.bmp`).
fn extension(fmt: ImageFormat) -> &'static str {
    match fmt {
        ImageFormat::Bmp | ImageFormat::Dib => "bmp",
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Gif => "gif",
        ImageFormat::Tiff => "tif",
        ImageFormat::Tga => "tga",
        ImageFormat::Pcx => "pcx",
        ImageFormat::Pict => "pct",
        ImageFormat::Wmf => "wmf",
        ImageFormat::Emf => "emf",
        _ => "bin",
    }
}
