//! Primitives shared across the [`rpt_pages`] render backends (SVG / HTML / PDF / raster) and the
//! layout engine: the twip↔unit conversion constants, XML/HTML text escaping, and the stroke dash
//! pattern math. These are backend-serialization concerns kept out of the frozen Page IR itself.
//!
//! The crate is dependency-minimal and WASM-safe: it depends only on [`rpt_pages`] and pulls in no
//! filesystem, font, or platform code.
//!
//! # Cross-backend coordinate model
//! The [`rpt_pages`] Page IR is in **twips** (1/1440"); draw-op coordinates are **printable-relative**
//! (0-based, page margin removed). Each backend re-applies [`Page::origin`](rpt_pages::Page::origin)
//! (the margin) exactly once and converts twips to its own output unit. The four backends differ only
//! in that unit and in how they apply the origin:
//!
//! | Backend | Output unit | Twips → unit | Origin (margin) | Notes |
//! | ------- | ----------- | ------------ | --------------- | ----- |
//! | `rpt-render-html` | CSS px @ 96 dpi | `px = round(twips / 15)`, half away from zero ([`TWIPS_PER_PX`]) | the RAS host wraps each page in a fixed 24 px (360 twip) container margin | positions are page-relative |
//! | `rpt-render-pdf` | typographic point | `pt = twips / 20` ([`TWIPS_PER_POINT`]) | `Page::origin` added to place content on the sheet | the basic writer flips y (raw PDF is y-up); the krilla writer does not (y-down, matching the IR) |
//! | `rpt-render-svg` | user unit | `1 user unit = 1 twip` (the `viewBox` is the page's twip extent; font points convert via [`TWIPS_PER_POINT`]) | `<g transform="translate(origin)">` | a consumer scales via `width`/`height` or CSS |
//! | `rpt-render-raster` | device px | `px = (twip + origin) * DPI / 1440` ([`TWIPS_PER_INCH`]) | folded into the same twip→px scale | default 96 DPI (= `TWIPS_PER_PX` 15) |

mod stroke;
mod units;
mod xml;

pub use stroke::dash_pattern;
pub use units::{POINTS_PER_INCH, TWIPS_PER_INCH, TWIPS_PER_POINT, TWIPS_PER_PX};
pub use xml::{escape_xml_attr, escape_xml_text};
