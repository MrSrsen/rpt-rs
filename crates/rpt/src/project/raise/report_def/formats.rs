//! Per-object/area/section attribute and format record decoders.

use super::*;

/// Decode a section's `0x00fe` format block (byte 4 == 0x01). Flags are one byte each at fixed even
/// offsets (00/01). `EnableSuppress` is stored inverted (01 = shown). Bytes 18/20 are
/// ResetPageNumberAfter / PrintAtBottomOfPage, matching the area block's ordering.
pub(super) fn decode_section_format(lb: &[u8]) -> crate::model::SectionFormat {
    let flag = |b: usize| lb.get(b).copied().unwrap_or(0) != 0;
    // Background colour is an A-B-G-R quad (same convention as the object border): byte 23 = alpha,
    // bytes 24-26 the colour as `BGR`. The alpha is inert (the engine reports the colour opaque), so
    // byte 23 must NOT be treated as a "default white" sentinel — that would discard a real fill.
    let background_color = bgr(lb, 24);
    crate::model::SectionFormat {
        base: crate::model::SectionAreaFormatBase {
            suppress: lb.get(6).copied().unwrap_or(1) == 0, // inverted: 0 = suppressed
            new_page_before: flag(10),
            new_page_after: flag(12),
            keep_together: flag(14),
            print_at_bottom_of_page: flag(20),
            reset_page_number_after: flag(18),
        },
        suppress_if_blank: flag(16),
        underlay_section: flag(22),
        background_color,
        ..Default::default()
    }
}

/// Decode an area's `0x00fe` format block (byte 4 == 0x00). `EnableHideForDrillDown` is stored
/// inverted (01 = False). Area-level `EnableSuppress` (byte 22) is read non-inverted (defaults to
/// False).
pub(super) fn decode_area_format(lb: &[u8]) -> crate::model::AreaFormat {
    let flag = |b: usize| lb.get(b).copied().unwrap_or(0) != 0;
    crate::model::AreaFormat {
        base: crate::model::SectionAreaFormatBase {
            suppress: flag(22),
            new_page_before: flag(10),
            new_page_after: flag(12),
            keep_together: flag(14),
            print_at_bottom_of_page: flag(20),
            reset_page_number_after: flag(18),
        },
        hide_for_drill_down: lb.get(8).copied().unwrap_or(1) == 0, // inverted: 0 = True
        ..Default::default()
    }
}

/// Decode a `0x00f0` **CommonFieldFormat** leaf: `store(short)` EnableSuppressIfDuplicated at
/// bytes 0..2 (BE), `store(short)` EnableUseSystemDefaults at bytes 2..4 (BE).
pub(crate) fn decode_common_format(lb: &[u8]) -> crate::model::CommonFieldFormat {
    crate::model::CommonFieldFormat {
        suppress_if_duplicated: u16_be(lb, 0).unwrap_or(0) != 0,
        use_system_defaults: u16_be(lb, 2).unwrap_or(0) != 0,
    }
}

/// Decode a `0x00f8` **NumericFieldFormat** leaf. Each field emits *two* `0x00f9`/`0x00f8` pairs — a
/// currency-format slot (first) and a number-format slot (second) — and the engine surfaces one based
/// on the field's value type (currency slot for a Currency-typed field, number slot otherwise). This
/// decodes one leaf; the caller selects which slot to keep.
///
/// The 14-byte scalar header holds, at fixed offsets: byte 1 = EnableSuppressIfZero, byte 2 =
/// NegativeFormat, byte 4 = ThousandsSeparator, byte 6 = EnableUseLeadingZero (runtime-resolved, not
/// modelled), bytes 7..9 = DecimalPlaces (BE u16), byte 9 = RoundingFormat code, byte 10 =
/// CurrencySymbolFormat, byte 13 = CurrencyPosition. After the header the length-prefixed symbol
/// strings follow in the order ThousandSymbol, DecimalSymbol, CurrencySymbol.
pub(crate) fn decode_numeric_format(lb: &[u8]) -> crate::model::NumericFieldFormat {
    use crate::model::{CurrencyPosition, CurrencySymbolFormat, NegativeFormat, RoundingFormat};
    let byte = |i: usize| lb.get(i).copied().unwrap_or(0);
    let (thousand_symbol, p1) =
        read_be_lp_string_lossy_at(lb, 14, 4096).unwrap_or((String::new(), 14));
    let (decimal_symbol, p2) =
        read_be_lp_string_lossy_at(lb, p1, 4096).unwrap_or((String::new(), p1));
    let (currency_symbol_text, _) =
        read_be_lp_string_lossy_at(lb, p2, 4096).unwrap_or((String::new(), p2));
    crate::model::NumericFieldFormat {
        decimal_places: u16_be(lb, 7).unwrap_or(2) as i32,
        rounding: RoundingFormat::from_code(i32::from(byte(9))),
        negative: NegativeFormat::from_code(i32::from(byte(2))),
        currency_symbol: CurrencySymbolFormat::from_code(i32::from(byte(10))),
        currency_position: CurrencyPosition::from_code(i32::from(byte(13))),
        thousands_separator: byte(4) != 0,
        suppress_if_zero: byte(1) != 0,
        decimal_symbol,
        thousand_symbol,
        currency_symbol_text,
    }
}

/// Decode a `0x00f4` **DateTimeFieldFormat** leaf. Byte 0 is the `DateTimeOrder` enum (which of the
/// date/time parts show, and in what order); the length-prefixed `DateTimeSeparator` string (the text
/// placed between the date and time parts, e.g. `"  "`) begins at offset 1.
pub(crate) fn decode_datetime_format(lb: &[u8]) -> crate::model::DateTimeFieldFormat {
    let (separator, _) = read_be_lp_string_lossy_at(lb, 1, 4096).unwrap_or((String::new(), 1));
    crate::model::DateTimeFieldFormat {
        order: crate::model::DateTimeOrder::from_code(i32::from(lb.first().copied().unwrap_or(0))),
        separator,
    }
}

/// Decode a `0x00f6` **TimeFieldFormat** leaf. The 14-byte scalar header carries the element-display
/// enums at fixed offsets: byte 2 = `hourType`, byte 3 = `minuteType`, byte 4 = `secondType` (each a
/// one-byte enum). The remainder of the SDK time surface (`TimeBase`, the AM/PM strings, the
/// hour/minute/second separators) is resolved at runtime from the host locale and is not decoded.
pub(crate) fn decode_time_format(lb: &[u8]) -> crate::model::TimeFieldFormat {
    use crate::model::{HourFormat, MinuteFormat, SecondFormat};
    let byte = |i: usize| i32::from(lb.get(i).copied().unwrap_or(0));
    crate::model::TimeFieldFormat {
        hour: HourFormat::from_code(byte(2)),
        minute: MinuteFormat::from_code(byte(3)),
        second: SecondFormat::from_code(byte(4)),
    }
}

/// Decode a `0x00fa` **StringFieldFormat** leaf. The prefix stores: byte 0 =
/// `EnableWordWrap` (1-byte enum), then three `u32`-BE indent longs at bytes 1-4 / 5-8 / 9-12
/// (`IndentAndSpacingFormat`), bytes 13-14 = `MaxNumberOfLines` (`u16` BE), byte 15 = `TextFormat`,
/// byte 16 = `ReadingOrder`. `RightIndent` (bytes 9-12) is the only indent that ever varies (72/144
/// twips); the first-line and left indents are `0` on every placed field, so their assignment to the
/// other two slots is not empirically distinguishable. The trailing spacing members are bytes 17-20
/// (`LineSpacing`, 16.16 fixed; `0x00010000` = `1.0`), bytes 21-24 (`CharacterSpacing`), byte 25
/// (`LineSpacingType`, `0` = multiple); the line spacing is invariant (single) but decoded for
/// completeness. `TextFormat`, `EnableWordWrap`, and `ReadingOrder` normally read their zero/default
/// values, `TextFormat` = `2` (HTMLText) being the exception.
pub(crate) fn decode_string_format(lb: &[u8]) -> crate::model::StringFieldFormat {
    use crate::model::{IndentAndSpacingFormat, ReadingOrder, TextFormat, Twips};
    let byte = |i: usize| i32::from(lb.get(i).copied().unwrap_or(0));
    let long = |i: usize| Twips(u32_be(lb, i).unwrap_or(0) as i32);
    crate::model::StringFieldFormat {
        text_format: TextFormat::from_code(byte(15)),
        enable_word_wrap: lb.first().copied().unwrap_or(0) != 0,
        max_number_of_lines: u16_be(lb, 13).unwrap_or(0),
        reading_order: ReadingOrder::from_code(byte(16)),
        indent: IndentAndSpacingFormat {
            first_line_indent: long(1),
            left_indent: long(5),
            right_indent: long(9),
            line_spacing: decode_line_spacing(lb, 25, 17),
        },
    }
}

/// Decode a paragraph/field line spacing from a format leaf: the `LineSpacingType` byte at `type_off`
/// (`0` = multiple, `1` = exact) and the 4-byte big-endian value at `value_off` (a 16.16 multiplier
/// when multiple — `0x0001_0000` = `1.0` — or a twip pitch when exact). The offsets differ per leaf
/// (`0x00c0` paragraph vs `0x00fa` string field). Falls back to the single-spacing default when the
/// value bytes are absent.
pub(crate) fn decode_line_spacing(
    lb: &[u8],
    type_off: usize,
    value_off: usize,
) -> crate::model::LineSpacing {
    use crate::model::{LineSpacing, LineSpacingType};
    let Some(raw) = u32_be(lb, value_off) else {
        return LineSpacing::default();
    };
    let spacing_type = match lb.get(type_off).copied() {
        Some(1) => LineSpacingType::Exact,
        _ => LineSpacingType::Multiple,
    };
    LineSpacing { spacing_type, raw }
}

/// Decode the vertical text alignment from an object's `0x00fc` ObjectFormat leaf: byte 3 carries it
/// using the shared [`Alignment`](crate::model::Alignment) ordinals (`6` = top, `7` = vertical
/// centre, `8` = bottom). Absent/short leaf defaults to top.
pub(super) fn object_vertical_alignment(lb: &[u8]) -> crate::model::VerticalAlignment {
    crate::model::VerticalAlignment::from_code(i32::from(lb.get(3).copied().unwrap_or(6)))
}

/// Decode the text-rotation angle from an object's `0x00fc` ObjectFormat leaf: bytes 20-21 (`u16` BE)
/// store the angle in degrees directly (`0` upright, `90` / `270` quarter turns). Absent/short leaf
/// defaults to upright.
pub(super) fn object_text_rotation(lb: &[u8]) -> crate::model::TextRotationAngle {
    crate::model::TextRotationAngle::from_code(i32::from(u16_be(lb, 20).unwrap_or(0)))
}

/// Decode a `0x00f2` **DateFieldFormat** leaf into the stored day/month/year format enums. The leaf
/// is a flat run of one-byte enums in the order: date-order, year, month, day, day-of-week,
/// windows-default, era, calendar — so byte 1 = year, byte 2 = month, byte 3 = day. Only these three
/// are exposed by the SDK (`DateFieldFormat.{Day,Month,Year}Format`).
pub(crate) fn decode_date_format(lb: &[u8]) -> crate::model::DateFieldFormat {
    use crate::model::{
        DateOrder, DateSystemDefaultType, DayFormat, DayOfWeekFormat, MonthFormat, YearFormat,
    };
    let byte = |i: usize| i32::from(lb.get(i).copied().unwrap_or(0));
    crate::model::DateFieldFormat {
        date_order: DateOrder::from_code(byte(0)),
        year: YearFormat::from_code(byte(1)),
        month: MonthFormat::from_code(byte(2)),
        day: DayFormat::from_code(byte(3)),
        // byte 4 = `dayOfWeekType`; no SDK accessor exposes it, decoded for record completeness only.
        day_of_week: DayOfWeekFormat::from_code(byte(4)),
        system_default: DateSystemDefaultType::from_code(byte(5)),
    }
}

/// Decode a `0x00ee` **BooleanFieldFormat** leaf: a one-byte enum OutputType at byte 0.
pub(crate) fn decode_boolean_format(lb: &[u8]) -> crate::model::BooleanFieldFormat {
    crate::model::BooleanFieldFormat {
        output_type: crate::model::BooleanOutputType::from_code(i32::from(
            lb.first().copied().unwrap_or(0),
        )),
    }
}

/// The font colour of a text/field/heading object (drawing and picture objects have none).
pub(super) fn font_color_mut(obj: &mut ReportObject) -> Option<&mut FontColor> {
    match &mut obj.kind {
        ReportObjectKind::Text(t) => Some(&mut t.font_color),
        ReportObjectKind::Field(f) => Some(&mut f.font_color),
        ReportObjectKind::FieldHeading(h) => Some(&mut h.font_color),
        _ => None,
    }
}

/// Read an `ObjectName` record (`0x9e`): Width (u32 BE [0..4]), Height (u32 BE [4..8]), then the
/// length-prefixed object Name.
pub(super) fn raise_object_name(node: &RecordNode, logical: &[u8]) -> (String, i32, i32) {
    let b = node.leaf_bytes(logical);
    let name = b.get(8..).and_then(first_lp).unwrap_or_default();
    (name, i32_be(&b, 0).unwrap_or(0), i32_be(&b, 4).unwrap_or(0))
}

/// Decode an object border record (`0xec`): bytes 0-3 are the four line styles in the order
/// Left, Right, Top, Bottom; byte 9 is the `HasDropShadow` flag (non-zero = on). The border and
/// background colours are each stored as an A-B-G-R quad: byte 10 = border alpha, bytes 11-13 the
/// border colour (`BGR`); byte 14 = background alpha, bytes 15-17 the background colour (`BGR`). The
/// engine always reports the colours fully opaque, so the alpha bytes are inert here — byte 14 is
/// `0xff` on almost every record but a real non-white fill (e.g. `0x00` with a genuine colour) is
/// still surfaced opaque, so it must NOT be treated as a "default white" sentinel.
pub(super) fn raise_border(node: &RecordNode, logical: &[u8]) -> crate::model::Border {
    let b = node.leaf_bytes(logical);
    let style = |i: usize| LineStyle::from_code(i32::from(b.get(i).copied().unwrap_or(0)));
    crate::model::Border {
        left: style(0),
        right: style(1),
        top: style(2),
        bottom: style(3),
        has_drop_shadow: b.get(9).copied().unwrap_or(0) != 0,
        border_color: bgr(&b, 11),
        background_color: bgr(&b, 15),
        ..Default::default()
    }
}

/// An object-position record (`0xbe`): Left then Top (twips), each in the variable-width
/// [`read_coord`] encoding (2 bytes, or 4 with the high-bit escape).
pub(super) fn raise_object_pos(node: &RecordNode, logical: &[u8]) -> Option<(i32, i32)> {
    let b = node.leaf_bytes(logical);
    let (left, next) = read_coord(&b, 0)?;
    let (top, _) = read_coord(&b, next)?;
    Some((left, top))
}

/// Decode a font record (`0x08`): a length-prefixed name, then a fixed attribute block —
/// `Size` (point size) at byte 4, the `Italic` flag at byte 6, the `Underline` flag at byte 8,
/// and `Weight` as a big-endian `u16` at bytes 11-12 (700 = bold, 400 = normal).
pub(super) fn raise_font(node: &RecordNode, logical: &[u8]) -> Option<Font> {
    let bytes = node.leaf_bytes(logical);
    let (name, after) = read_lp_string(&bytes)?;
    let attr = &bytes[after..];
    let size = i32::from(*attr.get(4)?);
    let italic = attr.get(6).is_some_and(|&b| b != 0);
    let underline = attr.get(8).is_some_and(|&b| b != 0);
    let weight = u16_be(attr, 11).map_or(400, i32::from);
    Some(Font {
        name,
        size_pt: size as f32,
        bold: weight >= 700,
        italic,
        underline,
        weight,
        ..Default::default()
    })
}

/// Decode an object's hyperlink from its `0x00fc ObjectFormat` leaf.
///
/// The leaf is a 15-byte flags header followed by a CSArchive tail. `HyperlinkText` is the first
/// tail field — a big-endian `u32` byte count (including the trailing NUL) at leaf offset `15`, then
/// the text. The `HyperlinkType` selector is a single byte further along, reached by walking the two
/// intervening length-prefixed strings so a non-empty `ToolTipText`/`CssClass` cannot shift it:
/// `HyperlinkText`, 2 filler bytes, `ToolTipText`, 4 filler bytes, `CssClass`, then the type byte.
///
/// The byte holds the RAS `CrHyperlinkTypeEnum` ordinal (see [`HyperlinkType::from_code`]); code `6`
/// (`Undefined`) is the engine's "no hyperlink" sentinel → `None`. The presence of a hyperlink is
/// decided by this byte, never by whether the target text is empty (several real kinds carry an
/// empty target). Mirrors RAS `Format.HyperlinkText` / `HyperlinkType`.
pub(super) fn decode_hyperlink(leaf: &[u8]) -> Option<Hyperlink> {
    const HYPERLINK_TEXT_OFF: usize = 15;
    // Consumed span (4-byte big-endian count incl NUL, then bytes) of a length-prefixed string,
    // tolerating the empty `ToolTipText`/`CssClass` the strict text reader rejects; `None` if the
    // span is missing or the length is absurd.
    let lp_len = |at: usize| -> Option<usize> {
        let n = u32_be(leaf, at)? as usize;
        (n <= 0x40000 && leaf.len() >= at + 4 + n).then_some(4 + n)
    };
    // `HyperlinkText` is read leniently: several real kinds (a field-value website, a report-part
    // drill-down) carry an empty target, so an empty string is not "no hyperlink".
    let (text, used) = read_be_lp_string_lossy(leaf, HYPERLINK_TEXT_OFF)?;
    let mut cur = HYPERLINK_TEXT_OFF + used + 2;
    cur += lp_len(cur)?; // ToolTipText
    cur += 4;
    cur += lp_len(cur)?; // CssClass
    let kind = HyperlinkType::from_code(i32::from(*leaf.get(cur)?));
    if kind == HyperlinkType::NoHyperlink {
        return None;
    }
    Some(Hyperlink { text, kind })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CurrencyPosition, CurrencySymbolFormat, DayOfWeekFormat, NegativeFormat, VerticalAlignment,
    };

    /// Build a numeric-format leaf: a 14-byte scalar header (with `decimal_places`, `rounding`,
    /// `currency_symbol` at their known offsets) followed by the three length-prefixed symbol strings
    /// in stored order — thousand, decimal, currency.
    fn numeric_leaf(thousand: &str, decimal: &str, currency: &str) -> Vec<u8> {
        let mut v = vec![0u8; 14];
        v[8] = 0x02; // u16_be[7..9] decimal places = 2
        v[9] = 9; // rounding code (RoundToHundredth)
        v[10] = 1; // currency symbol format = FixedSymbol
        let push = |v: &mut Vec<u8>, s: &str| {
            let len = (s.len() + 1) as u32; // include NUL
            v.extend_from_slice(&len.to_be_bytes());
            v.extend_from_slice(s.as_bytes());
            v.push(0);
        };
        push(&mut v, thousand);
        push(&mut v, decimal);
        push(&mut v, currency);
        v
    }

    #[test]
    fn numeric_symbols_decode_in_stored_order() {
        // Stored order is thousand, decimal, currency (US locale currency: thousand ",", decimal ".").
        let leaf = numeric_leaf(",", ".", "kr ");
        let f = decode_numeric_format(&leaf);
        assert_eq!(f.decimal_places, 2);
        assert_eq!(f.currency_symbol, CurrencySymbolFormat::FixedSymbol);
        assert_eq!(f.thousand_symbol, ",");
        assert_eq!(f.decimal_symbol, ".");
        assert_eq!(f.currency_symbol_text, "kr ");
    }

    #[test]
    fn numeric_scalar_flags_decode() {
        // Currency-slot header: byte1=SuppressIfZero, byte2=Negative, byte4=ThousandsSeparator,
        // byte13=CurrencyPosition.
        let mut leaf = numeric_leaf(",", ".", "$");
        leaf[1] = 1; // EnableSuppressIfZero
        leaf[2] = 3; // Bracketed
        leaf[4] = 1; // ThousandsSeparator on
        leaf[10] = 2; // FloatingSymbol
        leaf[13] = 1; // LeadingCurrencyOutsideNegative
        let f = decode_numeric_format(&leaf);
        assert!(f.suppress_if_zero);
        assert_eq!(f.negative, NegativeFormat::Bracketed);
        assert!(f.thousands_separator);
        assert_eq!(f.currency_symbol, CurrencySymbolFormat::FloatingSymbol);
        assert_eq!(
            f.currency_position,
            CurrencyPosition::LeadingCurrencyOutsideNegative
        );
    }

    #[test]
    fn numeric_thousands_separator_off() {
        let mut leaf = numeric_leaf("", "", "");
        leaf[4] = 0; // ThousandsSeparator off
        let f = decode_numeric_format(&leaf);
        assert!(!f.thousands_separator);
        assert!(!f.suppress_if_zero);
        assert_eq!(
            f.currency_position,
            CurrencyPosition::LeadingCurrencyInsideNegative
        );
    }

    #[test]
    fn numeric_empty_currency_symbol() {
        let leaf = numeric_leaf(".", ",", "");
        let f = decode_numeric_format(&leaf);
        assert_eq!(f.thousand_symbol, ".");
        assert_eq!(f.decimal_symbol, ",");
        assert_eq!(f.currency_symbol_text, "");
    }

    #[test]
    fn numeric_truncated_leaf_yields_empty_symbols() {
        // Only the scalar header, no string block — must not panic, symbols stay empty.
        let leaf = vec![0u8; 14];
        let f = decode_numeric_format(&leaf);
        assert_eq!(f.decimal_symbol, "");
        assert_eq!(f.thousand_symbol, "");
        assert_eq!(f.currency_symbol_text, "");
    }

    #[test]
    fn datetime_separator_decodes_lp_string_at_offset_1() {
        // Leaf: byte0 DateTimeOrder, then LP string (BE u32 len incl NUL) "  ".
        let leaf = [0x00, 0x00, 0x00, 0x00, 0x03, 0x20, 0x20, 0x00];
        assert_eq!(decode_datetime_format(&leaf).separator, "  ");
        // Truncated leaf: no panic, empty separator.
        assert_eq!(decode_datetime_format(&[0x00]).separator, "");
    }

    #[test]
    fn datetime_order_from_byte0() {
        use crate::model::DateTimeOrder;
        // byte0 = DateTimeOrder: 0=DateThenTime, 2=DateOnly.
        let then_time = [0x00, 0x00, 0x00, 0x00, 0x03, 0x20, 0x20, 0x00];
        assert_eq!(
            decode_datetime_format(&then_time).order,
            DateTimeOrder::DateThenTime
        );
        let date_only = [0x02, 0x00, 0x00, 0x00, 0x01, 0x00];
        assert_eq!(
            decode_datetime_format(&date_only).order,
            DateTimeOrder::DateOnly
        );
    }

    #[test]
    fn date_order_from_byte0() {
        use crate::model::DateOrder;
        // 8-enum date header; byte0 = dateOrder (1 = DayMonthYear, 2 = MonthDayYear).
        let dmy = [1u8, 1, 1, 1, 2, 1, 0, 0];
        assert_eq!(decode_date_format(&dmy).date_order, DateOrder::DayMonthYear);
        let mdy = [2u8, 0, 0, 0, 2, 1, 0, 0];
        assert_eq!(decode_date_format(&mdy).date_order, DateOrder::MonthDayYear);
    }

    #[test]
    fn time_format_hour_minute_second_from_bytes_2_3_4() {
        use crate::model::{HourFormat, MinuteFormat, SecondFormat};
        // 14-byte scalar header: byte2=hour, byte3=minute, byte4=second.
        // Default numeric form (all 0): NumericHour/NumericMinute/NumericSecond.
        let numeric = vec![1u8, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
        let f = decode_time_format(&numeric);
        assert_eq!(f.hour, HourFormat::NumericHour);
        assert_eq!(f.minute, MinuteFormat::NumericMinute);
        assert_eq!(f.second, SecondFormat::NumericSecond);
        // All-suppressed form (2/2/2): NoHour/NoMinute/NoSecond.
        let none = vec![1u8, 1, 2, 2, 2, 0, 0, 0, 1, 0, 0, 0, 0, 1];
        let g = decode_time_format(&none);
        assert_eq!(g.hour, HourFormat::NoHour);
        assert_eq!(g.minute, MinuteFormat::NoMinute);
        assert_eq!(g.second, SecondFormat::NoSecond);
        // byte2 = 1 → NoLeadingZeroNumericHour (the disk-code swap vs NumericHour).
        let no_lz = vec![1u8, 1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
        assert_eq!(
            decode_time_format(&no_lz).hour,
            HourFormat::NoLeadingZeroNumericHour
        );
    }

    #[test]
    fn string_format_members_from_leaf() {
        use crate::model::{ReadingOrder, TextFormat};
        // 26-byte leaf: byte0=EnableWordWrap, bytes13-14=MaxNumberOfLines(u16 BE), byte15=TextFormat,
        // byte16=ReadingOrder. Standard text, word-wrap on, LTR.
        let mut leaf = vec![0u8; 26];
        leaf[0] = 1; // EnableWordWrap
        leaf[18] = 1; // an unrelated tail byte (ignored)
        let f = decode_string_format(&leaf);
        assert!(f.enable_word_wrap);
        assert_eq!(f.text_format, TextFormat::StandardText);
        assert_eq!(f.max_number_of_lines, 0);
        assert_eq!(f.reading_order, ReadingOrder::LeftToRight);
        // HTML text = byte15 code 2; a nonzero MaxNumberOfLines at bytes 13-14 (BE).
        leaf[15] = 2;
        leaf[13] = 0;
        leaf[14] = 5;
        let g = decode_string_format(&leaf);
        assert_eq!(g.text_format, TextFormat::HTMLText);
        assert_eq!(g.max_number_of_lines, 5);
    }

    /// Build a `0xec` object-border leaf. `bg` is the A-B-G-R background quad written at bytes
    /// 14-17 (byte 14 = alpha, 15-17 = BGR); `border` the A-B-G-R quad at bytes 10-13.
    fn border_leaf(border: [u8; 4], bg: [u8; 4]) -> Vec<u8> {
        let mut v = vec![0u8; 34];
        v[10..14].copy_from_slice(&border);
        v[14..18].copy_from_slice(&bg);
        v
    }

    #[test]
    fn border_background_fill_preserved_regardless_of_alpha_byte() {
        let decode = |leaf: Vec<u8>| {
            let node = RecordNode {
                rtype: 0x00ec,
                subtype: 0,
                offset: 0,
                content_start: 0,
                content_end: leaf.len(),
                mask: 0,
                children: Vec::new(),
            };
            raise_border(&node, &leaf)
        };
        // Opaque (alpha byte 0xff) real fill RGB(95,58,31) stored BGR = 1f 3a 5f: preserved, not
        // clobbered to white by the old byte-14 sentinel.
        let rust = decode(border_leaf([0xff, 0, 0, 0], [0xff, 0x1f, 0x3a, 0x5f]));
        assert_eq!(
            rust.background_color,
            Some(Color {
                a: 255,
                r: 95,
                g: 58,
                b: 31
            })
        );
        // Alpha byte 0x00 with a real fill is still surfaced opaque —
        // the engine reports A=255. Fill RGB(242,245,248) stored BGR = f8 f5 f2.
        let light = decode(border_leaf([0xff, 0, 0, 0], [0x00, 0xf8, 0xf5, 0xf2]));
        assert_eq!(
            light.background_color,
            Some(Color {
                a: 255,
                r: 242,
                g: 245,
                b: 248
            })
        );
        // All-white fill decodes to white (the common case).
        let white = decode(border_leaf([0xff, 0, 0, 0], [0xff, 0xff, 0xff, 0xff]));
        assert_eq!(white.background_color, Some(Color::WHITE));
    }

    #[test]
    fn object_vertical_alignment_from_byte3() {
        assert_eq!(
            object_vertical_alignment(&[0, 1, 0, 6]),
            VerticalAlignment::Top
        );
        assert_eq!(
            object_vertical_alignment(&[0, 1, 2, 7]),
            VerticalAlignment::VerticalCenter
        );
        assert_eq!(
            object_vertical_alignment(&[0, 1, 0, 8]),
            VerticalAlignment::Bottom
        );
        // Short leaf defaults to top.
        assert_eq!(object_vertical_alignment(&[0, 1]), VerticalAlignment::Top);
    }

    #[test]
    fn text_rotation_from_bytes_20_21() {
        use crate::model::TextRotationAngle;
        // Bytes 20-21 (u16 BE) hold the angle in degrees. Rotate0 leaf (fixture Txt1-15).
        let mut leaf = [0u8; 22];
        assert_eq!(object_text_rotation(&leaf), TextRotationAngle::Rotate0);
        // 0x005a = 90 (fixture Txt16).
        leaf[20] = 0x00;
        leaf[21] = 0x5a;
        assert_eq!(object_text_rotation(&leaf), TextRotationAngle::Rotate90);
        // 0x010e = 270 (fixture Txt17).
        leaf[20] = 0x01;
        leaf[21] = 0x0e;
        assert_eq!(object_text_rotation(&leaf), TextRotationAngle::Rotate270);
        // Short leaf defaults to upright.
        assert_eq!(object_text_rotation(&[0, 1]), TextRotationAngle::Rotate0);
    }

    #[test]
    fn line_spacing_from_paragraph_leaf() {
        use crate::model::LineSpacingType;
        // The `0x00c0` paragraph leaf: type at byte 17, value (u32 BE) at bytes 18-21. Values observed
        // in the paragraph_typography fixture: single / 1.5 / double / exact-360-twips.
        let leaf = |ty: u8, val: u32| {
            let mut v = [0u8; 22];
            v[17] = ty;
            v[18..22].copy_from_slice(&val.to_be_bytes());
            v
        };
        let single = decode_line_spacing(&leaf(0, 0x0001_0000), 17, 18);
        assert_eq!(single.spacing_type, LineSpacingType::Multiple);
        assert_eq!(single.multiple(), Some(1.0));
        assert_eq!(
            decode_line_spacing(&leaf(0, 0x0001_8000), 17, 18).multiple(),
            Some(1.5)
        );
        assert_eq!(
            decode_line_spacing(&leaf(0, 0x0002_0000), 17, 18).multiple(),
            Some(2.0)
        );
        let exact = decode_line_spacing(&leaf(1, 360), 17, 18);
        assert_eq!(exact.spacing_type, LineSpacingType::Exact);
        assert_eq!(exact.exact_twips(), Some(360));
        // A leaf too short for the value defaults to single spacing.
        assert_eq!(decode_line_spacing(&[0u8; 4], 17, 18).multiple(), Some(1.0));
    }

    #[test]
    fn date_day_of_week_type_from_byte4() {
        // 8 one-byte enums: date-order, year, month, day, day-of-week, windows-default, ...
        let leaf = [0u8, 0, 1, 1, 2, 1, 0, 0]; // dayOfWeekType (byte4) = 2 = NoDayOfWeek
        let f = decode_date_format(&leaf);
        assert_eq!(f.day_of_week, DayOfWeekFormat::NoDayOfWeek);
        let leaf0 = [0u8, 0, 1, 1, 0, 1, 0, 0]; // byte4 = 0 = ShortDayOfWeek
        assert_eq!(
            decode_date_format(&leaf0).day_of_week,
            DayOfWeekFormat::ShortDayOfWeek
        );
    }

    /// Build a `0x00fc` ObjectFormat leaf carrying a hyperlink: 15-byte header, `HyperlinkText`,
    /// 2 filler bytes, an empty `ToolTipText`, 4 filler bytes, an empty `CssClass`, then the
    /// `HyperlinkType` selector byte (the stored RAS `CrHyperlinkTypeEnum` ordinal).
    fn hyperlink_leaf(text: &str, type_code: u8) -> Vec<u8> {
        let mut v = vec![0u8; 15];
        let push = |v: &mut Vec<u8>, s: &str| {
            v.extend_from_slice(&((s.len() + 1) as u32).to_be_bytes()); // len incl NUL
            v.extend_from_slice(s.as_bytes());
            v.push(0);
        };
        push(&mut v, text); // HyperlinkText
        v.extend_from_slice(&[0, 0]); // 2 filler bytes
        push(&mut v, ""); // ToolTipText (empty)
        v.extend_from_slice(&[0, 0, 0, 0]); // 4 filler bytes
        push(&mut v, ""); // CssClass (empty)
        v.push(type_code); // HyperlinkType
        v
    }

    #[test]
    fn hyperlink_type_decoded_from_stored_byte() {
        use crate::model::HyperlinkType::*;
        // Undefined (6) is the engine's "no hyperlink" sentinel.
        assert!(decode_hyperlink(&hyperlink_leaf("", 6)).is_none());
        // Each stored RAS code maps to its variant; the target text is carried through verbatim.
        let cases = [
            ("https://example.com", 0u8, Website),      // Website
            ("someone@example.com", 1, AnEMailAddress), // Email
            ("", 2, Html),                              // Html (distinct from Website in RAS)
            ("", 4, CurrentWebsiteField),               // WebsiteFieldValue
            ("", 5, CurrentWebsiteField),               // EmailFieldValue (grouped)
            ("", 7, ReportPartDrilldown),               // Drilldown
            ("Text2", 8, AnotherReportObject),          // ReportObject
            ("", 3, Other(3)),                          // CrystalReport — preserved, no variant
            ("", 99, Other(99)),                        // unknown code
        ];
        for (text, code, want) in cases {
            let h = decode_hyperlink(&hyperlink_leaf(text, code))
                .unwrap_or_else(|| panic!("code {code} should decode to a hyperlink"));
            assert_eq!(h.kind, want, "code {code}");
            assert_eq!(h.text, text, "code {code} text");
        }
    }

    #[test]
    fn hyperlink_type_survives_nonempty_tooltip_and_css() {
        // A non-empty ToolTipText / CssClass must not shift the located type byte.
        let mut v = vec![0u8; 15];
        let push = |v: &mut Vec<u8>, s: &str| {
            v.extend_from_slice(&((s.len() + 1) as u32).to_be_bytes());
            v.extend_from_slice(s.as_bytes());
            v.push(0);
        };
        push(&mut v, "mailto:a@b"); // HyperlinkText
        v.extend_from_slice(&[0, 0]);
        push(&mut v, "a tooltip"); // ToolTipText (non-empty)
        v.extend_from_slice(&[0, 0, 0, 0]);
        push(&mut v, "myclass"); // CssClass (non-empty)
        v.push(1); // Email
        let h = decode_hyperlink(&v).expect("hyperlink");
        assert_eq!(h.kind, crate::model::HyperlinkType::AnEMailAddress);
        assert_eq!(h.text, "mailto:a@b");
    }
}
