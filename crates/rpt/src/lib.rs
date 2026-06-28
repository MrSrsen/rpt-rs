//! `rpt` — a library for reading, inspecting, and (eventually) editing SAP Crystal Reports
//! `.rpt` files.
//!
//! # Architecture
//!
//! The library is a stack of *invertible* layers, so reading and writing are the same code
//! run in reverse. From the bytes up:
//!
//! ```text
//! L0   container   CFB/OLE2 compound file (the `cfb` crate)
//! L0.5 codec       stream header (type 0xffff): isEnc, version, IV
//! L1   codec+records  TSLV record framing + running XOR mask → the lossless substrate
//! L2   project     raise/lower between records and the semantic model
//! L3   model        the object graph (the DOM)
//! ```
//!
//! Layers L0–L1 form a lossless substrate: every record round-trips byte-identically,
//! including record types that are not yet modelled. The semantic model is a projection on
//! top.

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
// TODO: temporary public re-export; make `pub(crate)` once the PromptManager schema is mapped.
pub use codec::decode_prompt_manager;
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
