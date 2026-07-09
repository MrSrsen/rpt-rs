//! Crystal `ToText`/`CStr` numeric **picture string** parser (`"#,##0.00"`, `"0.0%"`, …).
//!
//! Crystal's picture strings are a subset of the VB/.NET custom numeric format: `0` = a digit slot
//! that shows a zero if empty, `#` = a digit slot that shows nothing if empty, `.` = decimal point,
//! `,` (between digit placeholders) = enable thousands grouping. We map a picture to a
//! [`NumberFormat`], which the caller then applies with [`crate::format_number`]. This covers the
//! common report cases; exotic sections (`;` negative/zero sub-patterns, scientific `E`, embedded
//! literals) are reported unsupported so a caller can fall back rather than emit a wrong string.

use crate::{NegativeStyle, NumberFormat};

/// Parse a numeric picture into a [`NumberFormat`], or `None` if it uses a feature this parser does
/// not model (so the caller can decline rather than guess).
pub fn parse_number_picture(picture: &str) -> Option<NumberFormat> {
    // Unsupported advanced features → bail (caller falls back to default formatting).
    if picture.contains(';') || picture.contains('E') || picture.contains('e') {
        return None;
    }
    let has_percent = picture.contains('%');
    if has_percent {
        // Percent scaling changes the value, not just the format — out of scope for a spec-only map.
        return None;
    }

    let (int_part, frac_part) = match picture.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (picture, None),
    };

    // Grouping is on iff a comma appears among the integer digit placeholders.
    let use_thousands = int_part.contains(',');
    // Leading zero iff the integer part contains a `0` placeholder.
    let leading_zero = int_part.contains('0');

    // Decimal places = count of digit placeholders after the point.
    let decimals = frac_part
        .map(|f| f.chars().filter(|c| *c == '0' || *c == '#').count() as u32)
        .unwrap_or(0);

    // Validate the placeholders are only the ones we understand.
    let ok = |part: &str| part.chars().all(|c| matches!(c, '0' | '#' | ',' | ' '));
    if !ok(int_part) || !frac_part.is_none_or(|f| f.chars().all(|c| matches!(c, '0' | '#'))) {
        return None;
    }

    Some(NumberFormat {
        decimals,
        use_thousands,
        leading_zero,
        negative: NegativeStyle::LeadingMinus,
        ..NumberFormat::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format_number;

    fn fmt(value: f64, picture: &str) -> Option<String> {
        parse_number_picture(picture).map(|spec| format_number(value, &spec))
    }

    #[test]
    fn common_pictures() {
        assert_eq!(fmt(1234.5, "#,##0.00").as_deref(), Some("1,234.50"));
        assert_eq!(fmt(1234.5, "0").as_deref(), Some("1235"));
        assert_eq!(fmt(1234.567, "0.0").as_deref(), Some("1234.6"));
        // Decimals = placeholder count (fixed), mapping to the SDK's `DecimalPlaces`. Trailing-`#`
        // trimming (`.5` vs `.50`) is not modeled.
        assert_eq!(fmt(0.5, "#.##").as_deref(), Some(".50"));
        assert_eq!(fmt(0.5, "0.##").as_deref(), Some("0.50"));
    }

    #[test]
    fn grouping_and_leading_zero() {
        let spec = parse_number_picture("#,##0.00").unwrap();
        assert!(spec.use_thousands);
        assert!(spec.leading_zero);
        let bare = parse_number_picture("#.##").unwrap();
        assert!(!bare.use_thousands);
        assert!(!bare.leading_zero);
    }

    #[test]
    fn unsupported_features_decline() {
        assert!(parse_number_picture("0.0%").is_none());
        assert!(parse_number_picture("0;(0)").is_none());
        assert!(parse_number_picture("0.00E+00").is_none());
    }
}
