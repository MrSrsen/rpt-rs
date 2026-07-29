//! `rpt-reader` — a library for reading, inspecting, and editing Crystal Reports `.rpt` files.
//!
//! # Architecture
//!
//! The library is a stack of layers, each a module. From the bytes up:
//!
//! ```text
//! container    CFB/OLE2 compound file → its streams (the `cfb` crate)
//! codec        stream bytes ↔ records: the stream header (type 0xffff), the cipher,
//!              deflate, TSLV framing, and the running XOR mask
//! records      one decoded stream as records — flat, and as a nested tree
//! field_table  what a record type's content *is*: its fields declared as a sequence,
//!              one table per type, walked by one reader and one writer
//! build_model  records → the semantic model, reading each record through its table
//! model        the report object graph (the `rpt-model` crate)
//! ```
//!
//! Five modules sit beside the stack rather than in it, because what they support is not any one
//! layer's: `bytes` is the by-hand vocabulary a record type with no declared sequence still needs;
//! `digest` is the dependency-free MD5/Base64 an OLE embedding is fingerprinted with;
//! [`fields`] is the public view of what a field table made of a record; [`annotate`] summarizes a
//! single record through the same decoders `build_model` reads it with, so inspection tooling can
//! show what a record means; and [`coverage`] reports how completely a report decoded, which the
//! deliberately infallible model build cannot otherwise signal. `io` is the [`Rpt`] facade that
//! drives the whole stack.
//!
//! The `container`/`codec`/`records` layers are **lossless**: every stream retains its original
//! bytes, including record types that are not yet modelled, and [`Rpt::original_bytes`] hands them
//! back verbatim. They are also invertible at the record level: [`Rpt::reencode`] and
//! [`Rpt::patch_record_bytes`] run the same path backwards (re-serialize → deflate → encrypt →
//! CFB rewrite), producing a valid `.rpt` that re-opens to byte-identical logical bytes. Above
//! them, `build_model` is read-only: there is no model→bytes path.
//!
//! # The public surface
//!
//! The crate root is the [`Rpt::open`] path: a name is flat here when a caller meets it while
//! driving [`Rpt`] — the handle, what its methods hand back, and what is needed to build an
//! argument or read a failure. A module that contributes at all contributes its whole public
//! vocabulary, so [`error`], [`coverage`], `io` and `container` are flat in full and there is
//! nothing to remember about which of their types made it. The first two stay `pub` modules
//! regardless, because a module doc is part of the API: [`error`] states the layer-tagging
//! convention, [`coverage`] states why an incomplete decode is a diagnostic and not an error.
//!
//! What sits a level *below* that path is reached through its own module, entered deliberately —
//! the records the model was built from ([`raw`]), and a field table's own reading of one
//! ([`fields`], [`annotate`]). There is no prelude: a glob that reached past the root would hand
//! over in one line exactly what entering those modules deliberately is for.
//!
//! [`model`] is the one module that contributes a subset, because it is a whole crate and
//! flattening it would bury [`Rpt`] under the model's every type. Flat from it are exactly the
//! types walking a [`Report`] cannot reach: [`Report`] itself, the walk's root, and the
//! saved-batch inspection types, which hang off an [`Rpt`] method and nothing else.
//!
//! # Quick start
//!
//! [`Rpt::open`] decodes a file into the semantic [`Report`]; [`Rpt::report`] borrows it.
//!
//! ```no_run
//! use rpt_reader::Rpt;
//!
//! let rpt = Rpt::open("report.rpt")?;
//! let report = rpt.report();
//! println!("format version {}", report.version);
//! println!(
//!     "{} objects across {} record types",
//!     report.objects().count(),
//!     rpt.inventory().len(),
//! );
//! # Ok::<(), rpt_reader::Error>(())
//! ```
//!
//! # Where a model field came from
//!
//! Which record a model field is decoded from, and where in that record it sits, is stated by the
//! code that reads it: the `build_model` function for its domain, and the field table that function
//! reads the record through — a table names the record's content as a sequence, and [`fields::read`]
//! replays it over real bytes to show each value and the bytes it came from. The same relation for
//! the format itself, record by record, is `docs/format/06-block-catalog.md`.

#![forbid(unsafe_code)]

pub mod annotate;
pub mod coverage;
pub mod error;
pub mod fields;

/// The format-neutral semantic report model, re-exported from the [`rpt_model`] crate.
///
/// `rpt-reader` decodes `.rpt` bytes into these types; downstream consumers (the render pipeline and the
/// derive/export layer) read them. The types are identical to `rpt_model`'s, so a [`Report`]
/// decoded here is the same type the pipeline crates depend on directly.
pub use rpt_model as model;

pub(crate) mod build_model;
pub(crate) mod bytes;
pub(crate) mod codec;
pub(crate) mod container;
pub(crate) mod digest;
pub(crate) mod field_table;
pub(crate) mod records;

mod io;

pub use container::{StreamId, SummaryInformation};
pub use coverage::{BatchProblem, DecodeCoverage, SavedDataStatus, StreamCoverage};
pub use error::{
    error_chain, CodecError, ContainerError, CryptoError, EditErrorKind, Error, IoError,
    NotAReportError, Result, StreamLoc,
};
pub use io::{AnonymizeReport, EditPolicy, Removal, Rpt};
pub use model::{Report, SavedBatchInfo, SavedBatchInspection, SavedBatchKind, SavedFieldInfo};

/// The record layer below the semantic model: the raw record tree, its stream header, the
/// record-type registry, and the typed record tree ([`Node`](raw::Node)/[`Unknown`](raw::Unknown),
/// built on demand by [`Rpt::typed_record_tree`]). A consumer of the semantic model (`Rpt::open` →
/// [`Report`]) never needs these — they back the byte-inspection tooling (`rpt dump`, `rpt tree`,
/// re-encode). Kept out of the crate root so the default public surface is `Rpt` + the model.
///
/// For the record types decoded from a declarative field table, [`fields::read`] gives that
/// table's own reading of a record — each field's value and the bytes it came from, which is how a
/// model field maps back onto the record it was decoded from. For bytes no table describes,
/// [`raw::lp_strings`] is this reader's own reading of what in them is text.
pub mod raw {
    pub use crate::bytes::{lp_strings, LpScan, LpString};
    pub use crate::codec::{Dialect, RecordNode, RecordSearch, StreamHeader};
    pub use crate::records::{
        Node, Origin, Part, RawRecord, Record, RecordStream, RecordTag, RecordTypeCount, Unknown,
        Value,
    };
}
