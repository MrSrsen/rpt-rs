//! Primitives shared by the [`rpt_pages`] output backend and the layout engine: the twip→point
//! conversion constant, the text-placement math (alignment anchor, justification slack, baseline
//! fallback), image content hashing and BMP decoding. These are backend-serialization concerns kept
//! out of the frozen Page IR itself.
//!
//! The crate is dependency-minimal and WASM-safe: it depends only on [`rpt_pages`] and pulls in no
//! filesystem, font, or platform code.
//!
//! # Coordinate model
//! The [`rpt_pages`] Page IR is in **twips** (1/1440"); draw-op coordinates are **printable-relative**
//! (0-based, page margin removed). A backend re-applies [`Page::origin`](rpt_pages::Page::origin) (the
//! margin) exactly once and converts twips to its own output unit. `rpt-render-pdf` works in
//! typographic points (`pt = twips / 20`, [`TWIPS_PER_POINT`]) and adds `Page::origin` to place
//! content on the sheet, staying y-down like the IR.

mod bmp;
mod hash;
mod text;
mod units;

pub use bmp::decode_bmp_rgba;
pub use hash::content_hash;
pub use text::{aligned_x, baseline_offset_twips, justify_gap_extra, JustifyUnit};
pub use units::TWIPS_PER_POINT;
