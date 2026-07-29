//! Export a Crystal Reports `.rpt` file to an exhaustive, deterministic JSON document — the decode
//! regression surface.
//!
//! Where [`rpt-kdl`](https://docs.rs/rpt-kdl) is a sparse, human-readable authoring projection, this
//! crate emits **everything the decoder read**: the full serde serialization of
//! [`rpt_reader::model::Report`], including defaults and the whole subreport tree. See [`export_json`].
//!
//! It emits **stored facts only**. Values the Crystal engine computes rather than stores — a field's
//! use count, a formula's runtime result length, the locale-resolved display format — are not in the
//! file and are deliberately absent, so a change in the output always means a change in the decode.
//! That is what makes the dump usable as a regression baseline.
//!
//! Reader-side: it links the `rpt-reader` decoder, so it is *not* WASM-safe, by design.

mod dump;

pub use dump::run as export_json;
pub use dump::HELP as JSON_DUMP_HELP;

/// An error raised while exporting a `.rpt` file to JSON.
///
/// Following the `rpt-reader` crate's convention, a variant that carries a
/// [`source`](std::error::Error::source) does not interpolate it — render the whole chain with
/// [`rpt_reader::error_chain`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// An error from the `rpt-reader` crate (opening or decoding the report).
    #[error(transparent)]
    Report(#[from] rpt_reader::Error),
    /// The JSON document could not be written to its destination path.
    #[error("cannot write `{path}`")]
    Write {
        /// The destination that could not be written.
        path: String,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The decoded model could not be serialized to JSON.
    #[error("cannot serialize the model decoded from `{input}` to JSON")]
    Serialize {
        /// The report the model was decoded from.
        input: String,
        /// The underlying serialization failure.
        #[source]
        source: serde_json::Error,
    },
}
