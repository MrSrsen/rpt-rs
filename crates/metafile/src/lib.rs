//! # metafile — a Windows Metafile vector parser
//!
//! `metafile` decodes Windows metafiles — **EMF** (Enhanced Metafile) today, with **WMF** and
//! **EMF+** planned — into device-independent vector primitives. It is a pure-Rust,
//! dependency-free, WASM-safe library: it interprets the metafile's own coordinate machinery
//! (world transform, window/viewport mapping, object selection, graphics-state stack) and hands a
//! consumer resolved shapes, never pixels.
//!
//! ## Backend-agnostic output
//!
//! Drawing is delivered through the [`MetafileSink`] visitor trait: implement the callbacks you care
//! about ([`polyline`](MetafileSink::polyline), [`polygon`](MetafileSink::polygon),
//! [`rectangle`](MetafileSink::rectangle), [`ellipse`](MetafileSink::ellipse),
//! [`text`](MetafileSink::text)) and map each into your own scene. Every callback receives geometry
//! already transformed into the metafile's device space plus a resolved [`GraphicsState`] (pen,
//! brush, font, text colour). The [`MetafileHeader`] reported first carries the device-space
//! [`bounds`](MetafileHeader::bounds) you map onto your target box, so the crate stays device- and
//! backend-independent.
//!
//! For consumers that prefer data over callbacks, [`Recording`] is a ready-made sink that collects
//! every primitive into a flat [`Primitive`] list.
//!
//! ## Example
//!
//! ```no_run
//! use metafile::{collect_emf, Primitive};
//!
//! let bytes = std::fs::read("picture.emf").unwrap();
//! let recording = collect_emf(&bytes).expect("valid EMF");
//! let header = recording.header.unwrap();
//! println!("bounds: {:?}", header.bounds);
//! for prim in &recording.primitives {
//!     if let Primitive::Text { text, .. } = prim {
//!         println!("text: {text}");
//!     }
//! }
//! ```
//!
//! ## Supported EMF records
//!
//! The parser covers the drawing and state records that appear in real embedded pictures:
//!
//! - **Geometry:** polyline/polygon/polypolyline/polypolygon (16- and 32-bit points), Bézier splines
//!   (flattened to polylines), `MOVETOEX`/`LINETO`, rectangle, round-rect (as a plain rectangle),
//!   ellipse.
//! - **Raster:** `STRETCHDIBITS`, `SETDIBITSTODEVICE`, `BITBLT`, `STRETCHBLT`, `ALPHABLEND`,
//!   `TRANSPARENTBLT` — each delivered as a [`Bitmap`] (a self-contained BMP/PNG/JPEG file) via
//!   [`MetafileSink::image`].
//! - **State:** world transform (set/left/right-multiply), window/viewport origin & extent,
//!   `SAVEDC`/`RESTOREDC`, pen/brush/font create-select-delete, stock objects, text colour.
//! - **Text:** `EXTTEXTOUTA`/`EXTTEXTOUTW` at their reference point.
//!
//! ### Not yet rendered
//!
//! Embedded **EMF+** (GDI+) content is *detected* — reported through
//! [`MetafileSink::unsupported`]`(`[`Feature::EmfPlus`]`)` so a consumer can flag partial output —
//! but not interpreted. Clipping regions, GDI path brackets (`BEGINPATH`/`FILLPATH`/…), native arcs
//! (`ARC`/`PIE`/`CHORD`), gradient fills, and per-character text spacing are skipped by length, and
//! the older **WMF** format is not yet parsed. Unsupported records never abort a parse.
//!
//! ## Robustness
//!
//! Every field read is bounds-checked: a truncated or garbage stream returns an [`Error`], never a
//! panic. Unknown records are skipped by their length, so an unsupported record never aborts a parse.

#![forbid(unsafe_code)]

mod reader;
mod sink;
mod types;

pub mod emf;

pub use emf::parse_emf;
pub use sink::{Feature, GraphicsState, MetafileHeader, MetafileSink, Primitive, Recording};
pub use types::{Bitmap, BitmapFormat, Brush, Color, Font, Pen, PenStyle, Point, Rect};

/// An error decoding a metafile stream.
///
/// Every variant but [`NotAMetafile`](Error::NotAMetafile) carries the byte `offset` it failed at, and
/// where a record was being read, that record's type. For a stream of vector commands "which record,
/// at which offset" *is* the diagnosis — a bare "unexpected end of stream" says only that something
/// went wrong somewhere. The type stays `Copy` (offsets and `&'static str`s only), so this costs
/// nothing and keeps the crate dependency-free.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The stream is not a metafile of the expected format (wrong record type or missing signature).
    NotAMetafile,
    /// A record or field extends past the end of the buffer (truncated stream).
    UnexpectedEof {
        /// Byte offset the read ran past the end at.
        offset: usize,
    },
    /// The stream is structurally invalid (e.g. a record smaller than its own header, or degenerate
    /// bounds).
    Malformed {
        /// The specific violation.
        why: &'static str,
        /// Byte offset it was detected at.
        offset: usize,
        /// The EMF record type being read, when one was.
        record: Option<u32>,
    },
    /// A record or feature the parser does not yet support in a position where it cannot be skipped.
    ///
    /// Distinct from [`Malformed`](Error::Malformed) because it is a gap in *this* parser, not damage
    /// to the file — a bug report rather than a broken input.
    Unsupported {
        /// The unsupported feature.
        what: &'static str,
        /// Byte offset it was met at.
        offset: usize,
        /// The EMF record type being read, when one was.
        record: Option<u32>,
    },
}

impl Error {
    /// A truncation at `offset`.
    pub(crate) fn eof(offset: usize) -> Error {
        Error::UnexpectedEof { offset }
    }

    /// A structural violation at `offset`, outside any record.
    pub(crate) fn malformed(why: &'static str, offset: usize) -> Error {
        Error::Malformed {
            why,
            offset,
            record: None,
        }
    }

    /// Note the EMF record type that was being read.
    #[must_use]
    pub fn in_record(mut self, rtype: u32) -> Error {
        match &mut self {
            Error::Malformed { record, .. } | Error::Unsupported { record, .. } => {
                *record = Some(rtype);
            }
            _ => {}
        }
        self
    }

    /// The byte offset this failed at, when the variant carries one.
    pub fn offset(&self) -> Option<usize> {
        match self {
            Error::NotAMetafile => None,
            Error::UnexpectedEof { offset }
            | Error::Malformed { offset, .. }
            | Error::Unsupported { offset, .. } => Some(*offset),
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        /// ` in record 0x0000002b`, or nothing.
        fn at_record(record: &Option<u32>) -> impl core::fmt::Display + '_ {
            struct D<'a>(&'a Option<u32>);
            impl core::fmt::Display for D<'_> {
                fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                    match self.0 {
                        Some(r) => write!(f, " in record {r:#010x}"),
                        None => Ok(()),
                    }
                }
            }
            D(record)
        }
        match self {
            Error::NotAMetafile => write!(f, "not a metafile of the expected format"),
            Error::UnexpectedEof { offset } => {
                write!(f, "unexpected end of stream at byte {offset}")
            }
            Error::Malformed {
                why,
                offset,
                record,
            } => write!(
                f,
                "malformed metafile: {why} at byte {offset}{}",
                at_record(record)
            ),
            Error::Unsupported {
                what,
                offset,
                record,
            } => write!(
                f,
                "unsupported metafile feature: {what} at byte {offset}{}",
                at_record(record)
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Parse an EMF stream and collect its primitives into a [`Recording`] — the convenience entry point
/// for consumers that want the decoded drawing as data.
///
/// # Errors
/// Propagates any [`Error`] from [`parse_emf`].
pub fn collect_emf(bytes: &[u8]) -> Result<Recording, Error> {
    let mut recording = Recording::default();
    parse_emf(bytes, &mut recording)?;
    Ok(recording)
}
