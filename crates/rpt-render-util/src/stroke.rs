//! Stroke dash-pattern math, shared by the vector and raster backends.

use rpt_pages::LineStyle;

/// The two-segment dash pattern `[on, off]` for a line `style` at stroke width `w`, or `None` for a
/// solid line. `Dashed` is `[4w, 4w]` and `Dotted` is `[w, 2w]`; `Single`/`Double` render solid.
///
/// Generic over the width type so each backend keeps its own unit: the SVG backend passes an `i32`
/// twip width (formatted into `stroke-dasharray`), the raster backend an `f32` device-pixel width
/// (fed to `tiny_skia::StrokeDash`). The multipliers are exact in both, so output is unchanged.
pub fn dash_pattern<T>(style: LineStyle, w: T) -> Option<[T; 2]>
where
    T: Copy + core::ops::Mul<Output = T> + From<u8>,
{
    match style {
        LineStyle::Single | LineStyle::Double => None,
        LineStyle::Dashed => Some([w * T::from(4u8), w * T::from(4u8)]),
        LineStyle::Dotted => Some([w * T::from(1u8), w * T::from(2u8)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_styles_have_no_dash() {
        assert_eq!(dash_pattern::<i32>(LineStyle::Single, 3), None);
        assert_eq!(dash_pattern::<i32>(LineStyle::Double, 3), None);
    }

    #[test]
    fn dashed_and_dotted_ratios() {
        assert_eq!(dash_pattern(LineStyle::Dashed, 3_i32), Some([12, 12]));
        assert_eq!(dash_pattern(LineStyle::Dotted, 3_i32), Some([3, 6]));
        assert_eq!(dash_pattern(LineStyle::Dashed, 2.0_f32), Some([8.0, 8.0]));
        assert_eq!(dash_pattern(LineStyle::Dotted, 2.0_f32), Some([2.0, 4.0]));
    }
}
