//! Static/OLE picture loading — filling a [`PictureObject`](crate::model::PictureObject) from its
//! embedding.
//!
//! The picture bytes live in the top-level `Embedding N/CONTENTS` streams (looked up via
//! [`crate::container::load_embedding_contents`]); the coarse picture type is classified from their
//! wire format. The natural size and the scale factors the engine reports are *not* stored — it
//! recomputes them at load from the embedded image's OLE extent — so they are left to the consumer
//! that needs them ([`rpt_model::natural_extent`] reads the extent from these same bytes).

use crate::container::{load_embedding_contents, Container};
use std::collections::BTreeMap;

/// Fill every static picture in the report and its subreports from the embeddings they name.
///
/// A subreport's pictures live in its own `Subdocument K` storage, so each is filled under its own
/// prefix; `subdoc_names` keys share order with `report.subreports`.
pub(crate) fn attach_pictures(
    report: &mut crate::model::Report,
    container: &Container,
    subdoc_names: &BTreeMap<u32, String>,
) {
    fill_picture_data(report, container, "");
    for (idx, sub) in subdoc_names.keys().zip(report.subreports.iter_mut()) {
        fill_picture_data(&mut sub.report, container, &format!("Subdocument {idx}"));
    }
}

/// Fill each static `PictureObject`'s `data` from its OLE embedding. `storage_prefix` scopes the
/// lookup to the report's own storage — empty for the main report, `Subdocument K` for a subreport
/// — and the picture's `ole_ordinal` (from the `0xbd` record) selects the `Embedding N` within it.
fn fill_picture_data(
    report: &mut crate::model::Report,
    container: &Container,
    storage_prefix: &str,
) {
    for obj in report.objects_mut() {
        if let crate::model::ReportObjectKind::Picture(pic) = &mut obj.kind {
            if pic.data.is_empty() {
                if let Some(ord) = pic.ole_ordinal {
                    if let Some(bytes) = load_embedding_contents(container, storage_prefix, ord) {
                        pic.data = bytes;
                    }
                }
            }
            fill_picture_type(pic);
        }
    }
}

/// Classify a static picture into the coarse SDK [`PictureType`](crate::model::PictureType) from
/// the wire format of its embedded [`data`](crate::model::PictureObject::data). A Windows/Enhanced
/// metafile embedding (`CF_METAFILEPICT`/`CF_ENHMETAFILE` static, CompObj class `StaticMetafile`)
/// reports as `Metafile`; every other static — `CF_DIB`/`StaticDib`, plus PNG/JPEG/… raster
/// imports — reports as `Bitmap`. An empty payload (a blob-field picture or an unresolved embedding)
/// keeps the `Bitmap` default. The rarer live-OLE-object case (`crPictureTypeOle`, whose `CONTENTS`
/// is server-native data rather than an image) is not distinguished here.
fn fill_picture_type(pic: &mut crate::model::PictureObject) {
    use crate::model::{ImageFormat, PictureType};
    pic.picture_type = match ImageFormat::sniff(&pic.data) {
        ImageFormat::Wmf | ImageFormat::Emf => PictureType::Metafile,
        _ => PictureType::Bitmap,
    };
}
