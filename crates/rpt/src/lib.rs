//! `rpt` — a library for reading, inspecting, and (eventually) editing SAP Crystal Reports
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
//! modelled. The semantic model is a read-only projection on top; there is no lower (model →
//! bytes) path yet — the layers are *designed* to be invertible, but only the read direction
//! ships today.

#![forbid(unsafe_code)]

pub mod diagnostics;
pub mod error;
pub mod model;

pub(crate) mod codec;
pub(crate) mod container;
pub(crate) mod project;
pub(crate) mod records;

mod io;

pub use diagnostics::install_panic_hook;
pub use error::{Error, ProjectErrorKind, Result};
pub use io::Rpt;
pub use model::Report;

pub use codec::{RecordNode, StreamHeader};
pub use container::{StreamId, SummaryInformation};
pub use records::{Origin, RawRecord, Record, RecordStream, RecordTag};

/// Curated re-exports for `use rpt::prelude::*`.
pub mod prelude {
    pub use crate::error::{Error, Result};
    pub use crate::model::Report;
    pub use crate::{
        Record, RecordStream, RecordTag, Rpt, StreamHeader, StreamId, SummaryInformation,
    };
}

pub(crate) mod bytes;
