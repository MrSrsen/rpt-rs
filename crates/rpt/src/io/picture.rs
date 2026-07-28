//! Static/OLE picture derivation — filling a [`PictureObject`](crate::model::PictureObject) from
//! its embedding and reconstructing the values the engine recomputes at load rather than storing.
//!
//! The picture bytes live in the top-level `Embedding N/CONTENTS` streams (looked up via
//! [`crate::container::load_embedding_contents`]); the picture type, natural size, and scale
//! factors are derived from those bytes.

use crate::container::{load_embedding_contents, Container};

/// Fill each static `PictureObject`'s `data` from its OLE embedding. `storage_prefix` scopes the
/// lookup to the report's own storage — empty for the main report, `Subdocument K` for a subreport
/// — and the picture's `ole_ordinal` (from the `0xbd` record) selects the `Embedding N` within it.
pub(super) fn fill_picture_data(
    report: &mut crate::model::Report,
    container: &Container,
    storage_prefix: &str,
) {
    for obj in report.objects_mut() {
        // The placed bounds drive the scale factors; copy them before borrowing `kind` mutably.
        let bounds = obj.bounds;
        if let crate::model::ReportObjectKind::Picture(pic) = &mut obj.kind {
            if pic.data.is_empty() {
                if let Some(ord) = pic.ole_ordinal {
                    if let Some(bytes) = load_embedding_contents(container, storage_prefix, ord) {
                        pic.data = bytes;
                    }
                }
            }
            fill_picture_type(pic);
            derive_picture_geometry(pic, bounds);
        }
    }
}

/// Classify a static picture into the coarse SDK [`PictureType`](crate::model::PictureType) from
/// the wire format of its embedded [`data`](crate::model::PictureObject::data). A Windows/Enhanced
/// metafile embedding (`CF_METAFILEPICT`/`CF_ENHMETAFILE` static, CompObj class `StaticMetafile`)
/// reports as `Metafile`; every other static — `CF_DIB`/`StaticDib`, plus PNG/JPEG/… raster
/// imports — reports as `Bitmap`. An empty payload (a blob-field picture or an unresolved embedding)
/// keeps the `Bitmap` default. The rarer live-OLE-object case (`crPictureTypeOle`, whose `CONTENTS`
/// is server-native data rather than an image) is unobserved and not distinguished here.
fn fill_picture_type(pic: &mut crate::model::PictureObject) {
    use crate::model::{ImageFormat, PictureType};
    pic.picture_type = match ImageFormat::sniff(&pic.data) {
        ImageFormat::Wmf | ImageFormat::Emf => PictureType::Metafile,
        _ => PictureType::Bitmap,
    };
}

/// Derive a static picture's natural size (SDK `OriginalWidth`/`OriginalHeight`) and the scale
/// factors it is drawn at (`XScaling`/`YScaling`) from its embedded image. These are *not* stored in
/// the report — the engine recomputes them at load from the embedded image's OLE extent — so they
/// are a derived value, reconstructed here via [`rpt_model::natural_extent`] from the just-loaded
/// image bytes. When the natural size is unknown (no OLE embedding, or a format whose header that
/// derivation does not parse), the size stays `0` and the scale factors default to `1.0`.
fn derive_picture_geometry(pic: &mut crate::model::PictureObject, bounds: crate::model::Rect) {
    let Some((ow, oh)) = crate::model::natural_extent(&pic.data) else {
        pic.original_width = crate::model::Twips(0);
        pic.original_height = crate::model::Twips(0);
        pic.x_scaling = 1.0;
        pic.y_scaling = 1.0;
        return;
    };
    pic.original_width = ow;
    pic.original_height = oh;
    pic.x_scaling = if ow.0 > 0 {
        f64::from(bounds.width.0) / f64::from(ow.0)
    } else {
        1.0
    };
    pic.y_scaling = if oh.0 > 0 {
        f64::from(bounds.height.0) / f64::from(oh.0)
    } else {
        1.0
    };
}
