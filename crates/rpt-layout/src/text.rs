//! The [`TextLayout`] trait lives in the leaf crate [`rpt_pages`] (beside `FontSpec`), so the
//! layout engine and `rpt-text`'s real font stack share one definition without `rpt-text` depending
//! on all of `rpt-layout`. Re-exported here so `crate::text::…` paths keep resolving.

pub use rpt_pages::{ApproxLayout, TextLayout, TWIPS_PER_PT};
