//! Summarising the report's embedded OLE objects.
//!
//! An embedding's bytes are not part of the semantic model — an [`Embed`] keeps only the stream's
//! name, its byte size, and a content fingerprint, which is enough to tell two reports' embeddings
//! apart without carrying the objects themselves.

use crate::container::{Container, EmbeddingStream};
use crate::digest::md5_base64;
use crate::model::Embed;

/// Summarise the OLE data streams of every top-level `Embedding N` storage, in directory order.
///
/// The engine reports the object data streams — `Ole`, `OlePres000`, `Ole10Native` — but not the
/// `CompObj` (the OLE1 class moniker) or a `CONTENTS` sub-storage, so those are skipped.
pub(crate) fn build_embeds(container: &Container) -> Vec<Embed> {
    const SKIP: [&str; 2] = ["CompObj", "CONTENTS"];
    container
        .embedding_streams()
        .into_iter()
        .filter(|s| !SKIP.contains(&s.name.as_str()))
        .map(|EmbeddingStream { name, bytes }| Embed {
            name,
            size: bytes.len() as u64,
            md5_hash: md5_base64(bytes),
        })
        .collect()
}
