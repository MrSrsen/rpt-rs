//! `rpt` — a library for reading, inspecting, and (eventually) editing Crystal Reports
//! `.rpt` files.
//!
//! # Architecture
//!
//! The library is a stack of layers. From the bytes up:
//!
//! ```text
//! L0   container   CFB/OLE2 compound file (the `cfb` crate)
//! L0.5 codec       stream header (type 0xffff): isEnc, version, IV
//! L1   codec+records  TSLV record framing + running XOR mask → the lossless substrate
//! L2   project     raise records → the semantic model
//! L3   model        the object graph (the DOM)
//! ```
//!
//! Layers L0–L1 form a lossless substrate: every stream retains its original bytes, so
//! [`Rpt::save`] is byte-identical to the input, including record types that are not yet
//! modelled. The substrate is invertible at the record level: [`Rpt::reencode`] and
//! [`Rpt::patch_record_leaf`] run the L1→L0 write pipeline (re-serialize → deflate → encrypt →
//! CFB rewrite), producing a valid `.rpt` that re-opens to byte-identical logical bytes. The
//! semantic model above L2 is still a read-only projection; there is no model→bytes path yet.
//!
//! # Quick start
//!
//! [`Rpt::open`] decodes a file into the semantic [`Report`]; [`Rpt::report`] borrows it.
//!
//! ```no_run
//! use rpt::Rpt;
//!
//! let rpt = Rpt::open("report.rpt")?;
//! let report = rpt.report();
//! println!("format version {}", report.version);
//! println!(
//!     "{} objects across {} record types",
//!     report.objects().count(),
//!     rpt.inventory().len(),
//! );
//! # Ok::<(), rpt::Error>(())
//! ```
//!
//! # Provenance
//!
//! Byte-layout provenance for every model field — which `Contents` record it is decoded from and
//! its on-disk leaf layout — is documented in [`provenance`].

#![forbid(unsafe_code)]

pub mod coverage;
pub mod diagnostics;
pub mod error;
pub mod provenance;

/// The format-neutral semantic report model, re-exported from the [`rpt_model`] crate.
///
/// `rpt` decodes `.rpt` bytes into these types; downstream consumers (the render pipeline and the
/// derive/export layer) read them. The types are identical to `rpt_model`'s, so a [`Report`]
/// decoded here is the same type the pipeline crates depend on directly.
pub use rpt_model as model;

pub(crate) mod codec;
pub(crate) mod container;
pub(crate) mod project;
pub(crate) mod records;

mod io;

pub use coverage::{DecodeCoverage, StreamCoverage};
pub use diagnostics::{error_chain, install_panic_hook};
pub use error::{
    CodecError, ContainerError, CryptoError, Error, IoError, NotAReportError, ProjectErrorKind,
    Result, StreamLoc,
};
pub use io::{AnonymizeReport, EditPolicy, Removal, Rpt};
pub use model::{Report, SavedBatchInfo, SavedBatchInspection, SavedBatchKind, SavedFieldInfo};

pub use container::{StreamId, SummaryInformation};

/// Per-record decoded summaries for byte-inspection tooling. [`annotate::summarize`] takes a single
/// DOM [`raw::Node`] and, for the record types the decoder understands (format records, group-area
/// options, summary definitions), returns a concise [`annotate::RecordSummary`] of the decoded
/// values — so an inspection view (`rpt tree`) can show what a record means, not only its bytes.
pub mod annotate {
    pub use crate::project::raise::annotate::{summarize, RecordSummary};
}

/// The low-level record substrate (the L0–L1 layer): the raw record tree, its stream header, the
/// record-type registry, and the type-strict record DOM ([`Node`](raw::Node)/[`Unknown`](raw::Unknown),
/// projected on demand by [`Rpt::record_dom`]). A consumer of the semantic model (`Rpt::open` →
/// [`Report`]) never needs these — they back the byte-inspection tooling (`rpt dump`, `rpt tree`,
/// re-encode). Kept out of the crate root so the default public surface is `Rpt` + the model.
///
/// For how a given model field maps onto these records and their leaf layout, see [`provenance`].
pub mod raw {
    pub use crate::codec::{RecordNode, StreamHeader};
    pub use crate::records::{
        Node, Origin, RawRecord, Record, RecordStream, RecordTag, RecordTypeCount, Unknown, Value,
    };
}

/// Curated re-exports for `use rpt::prelude::*`.
pub mod prelude {
    pub use crate::error::{Error, Result};
    pub use crate::model::Report;
    pub use crate::raw::{Record, RecordStream, RecordTag, StreamHeader};
    pub use crate::{Rpt, StreamId, SummaryInformation};
}

pub(crate) mod bytes;
