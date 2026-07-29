//! The small stored-token conventions the decoders share: colours, field references, and the
//! abbreviated operator names.
//!
//! Every value a decoder in this layer reads out of a record comes from that record's field table,
//! so what is left here is the interpretation of those values rather than any byte addressing.
//! Locating the records themselves is [`super::tree_search`].

use crate::model::{Color, SummaryOperation};

/// The three operations a stored display reference abbreviates. Their expansions are the model's
/// own spelling, so this states only which tokens are abbreviated, not what they expand to.
const ABBREVIATED_SUMMARY_OPS: [(&str, SummaryOperation); 3] = [
    ("Max", SummaryOperation::Maximum),
    ("Min", SummaryOperation::Minimum),
    ("Avg", SummaryOperation::Average),
];

/// Expand an abbreviated summary-operation token from a stored display reference (`Sum of {…}`) to
/// the operator name a rendered summary expression spells out; any other token is returned
/// unchanged. Only the tokens in [`ABBREVIATED_SUMMARY_OPS`] differ between the two forms.
pub(super) fn summary_op_full(token: &str) -> &str {
    ABBREVIATED_SUMMARY_OPS
        .iter()
        .find(|(abbrev, _)| *abbrev == token)
        .map_or(token, |(_, op)| op.full_name())
}

/// Whether a string is an engine field reference: a database field (`Table.field`) or a formula
/// (`@name`). Excludes literals like `Others` and localized order/name marker strings.
pub(super) fn is_field_ref(s: &str) -> bool {
    s.starts_with('@') || s.contains('.')
}

/// Decode a `COLORREF` value (`0x00BBGGRR`) into a [`Color`]: red in the low byte, then green, then
/// blue. The caller reads the `u32` in the record's own endianness and applies its own sentinel.
pub(super) fn colorref(v: u32) -> Color {
    Color {
        a: 255,
        r: (v & 0xff) as u8,
        g: ((v >> 8) & 0xff) as u8,
        b: ((v >> 16) & 0xff) as u8,
    }
}

/// The `COLORREF` a record stores for "default / no colour".
const NO_COLOR: u32 = 0xffff_ffff;

/// The same, with [`NO_COLOR`] reported as White.
pub(super) fn colorref_or_white(v: u32) -> Color {
    if v == NO_COLOR {
        Color::WHITE
    } else {
        colorref(v)
    }
}

#[cfg(test)]
mod tests {
    use super::summary_op_full;

    #[test]
    fn summary_op_full_expands_only_min_max() {
        assert_eq!(summary_op_full("Max"), "Maximum");
        assert_eq!(summary_op_full("Min"), "Minimum");
        assert_eq!(summary_op_full("Avg"), "Average");
        // Any other token passes through unchanged.
        assert_eq!(summary_op_full("Sum"), "Sum");
        assert_eq!(summary_op_full("DistinctCount"), "DistinctCount");
    }
}
