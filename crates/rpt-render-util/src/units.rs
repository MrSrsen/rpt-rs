//! Twip conversion constants. A twip is 1/1440 inch (the Page IR unit); these relate it to the other
//! units the backends work in. All are exactly representable in `f32`, so a backend that works in
//! `f32` can cast without loss.

/// 1 typographic point = 20 twips.
pub const TWIPS_PER_POINT: f64 = 20.0;

/// 1 CSS pixel at 96 dpi = 15 twips (1440 twips/in ÷ 96 px/in).
pub const TWIPS_PER_PX: f64 = 15.0;

/// 1 inch = 1440 twips (the Page IR unit).
pub const TWIPS_PER_INCH: f64 = 1440.0;

/// 1 inch = 72 typographic points.
pub const POINTS_PER_INCH: f64 = 72.0;
