//! Per-object/area/section attribute and format record decoders.

use super::*;

/// Decode a section's `0x00fe` format block (byte 4 == 0x01). Flags are one byte each at fixed even
/// offsets (00/01). `EnableSuppress` is stored inverted (01 = shown). Bytes 18/20 are
/// ResetPageNumberAfter / PrintAtBottomOfPage, matching the area block's ordering.
pub(super) fn decode_section_format(lb: &[u8]) -> crate::model::SectionFormat {
    let flag = |b: usize| lb.get(b).copied().unwrap_or(0) != 0;
    // Background colour: byte 23 is the "default background" flag (`0xff` = the engine's default
    // white); otherwise bytes 24-26 are the colour as `BGR` (same convention as the object border).
    let bgr = |i: usize| -> Option<Color> {
        Some(Color {
            a: 255,
            r: *lb.get(i + 2)?,
            g: *lb.get(i + 1)?,
            b: *lb.get(i)?,
        })
    };
    let background_color = match lb.get(23).copied() {
        Some(0xff) => Some(Color::WHITE),
        _ => bgr(24),
    };
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
pub(super) fn decode_common_format(lb: &[u8]) -> crate::model::CommonFieldFormat {
    crate::model::CommonFieldFormat {
        suppress_if_duplicated: u16_be(lb, 0).unwrap_or(0) != 0,
        use_system_defaults: u16_be(lb, 2).unwrap_or(0) != 0,
    }
}

/// Decode a `0x00f8` **NumericFieldFormat** leaf (the second `0x00f9` record — the one the engine
/// reports). Fields are a flat concatenation in writer order; the relevant scalars sit at:
/// byte 2 = NegativeFormat, bytes 7..9 = DecimalPlaces (BE u16), byte 9 = RoundingFormat code,
/// byte 10 = CurrencySymbolFormat. (`EnableUseLeadingZero` is not in the leaf — the engine derives
/// it from value type, so the exporter resolves it.)
pub(super) fn decode_numeric_format(lb: &[u8]) -> crate::model::NumericFieldFormat {
    use crate::model::{CurrencySymbolFormat, NegativeFormat, RoundingFormat};
    let byte = |i: usize| lb.get(i).copied().unwrap_or(0);
    // The scalar header is a fixed 14 bytes (short + enum + 2×short + ushort + 2×enum + short +
    // enum), after which the length-prefixed symbol strings begin. Symbol-string order:
    // DecimalSymbol, ThousandSymbol, CurrencySymbol.
    let (decimal_symbol, p1) = read_fmt_string(lb, 14).unwrap_or((String::new(), 14));
    let (thousand_symbol, p2) = read_fmt_string(lb, p1).unwrap_or((String::new(), p1));
    let (currency_symbol_text, _) = read_fmt_string(lb, p2).unwrap_or((String::new(), p2));
    crate::model::NumericFieldFormat {
        decimal_places: u16_be(lb, 7).unwrap_or(2) as i32,
        rounding: RoundingFormat::from_code(i32::from(byte(9))),
        negative: NegativeFormat::from_code(i32::from(byte(2))),
        currency_symbol: CurrencySymbolFormat::from_code(i32::from(byte(10))),
        decimal_symbol,
        thousand_symbol,
        currency_symbol_text,
    }
}

/// Read a length-prefixed format string at `pos` in a field-format leaf: a `u32`-BE byte length
/// (including the trailing NUL), then that many bytes, decoded as ASCII up to the first NUL. Returns
/// the string and the offset just past it, or `None` if the length runs past the leaf. An empty
/// string is stored as length 1 (a lone NUL) and decodes to `""`.
///
/// A deliberate strict variant of the shared LP-string readers: it accepts an empty (lone-NUL)
/// string — which `read_be_lp_string_lossy` also does, but here the `1000`-byte cap and absolute
/// `pos` semantics are tuned for the short symbol strings, so it is kept separate.
fn read_fmt_string(lb: &[u8], pos: usize) -> Option<(String, usize)> {
    let len = u32_be(lb, pos)? as usize;
    // Guard against a mis-parse walking off into the record: symbol strings are short.
    if len > 0x1000 {
        return None;
    }
    let raw = lb.get(pos + 4..pos + 4 + len)?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    Some((
        String::from_utf8_lossy(&raw[..end]).into_owned(),
        pos + 4 + len,
    ))
}

/// Decode a `0x00f2` **DateFieldFormat** leaf into the stored day/month/year format enums. The leaf
/// is a flat run of one-byte enums in the order: date-order, year, month, day, day-of-week,
/// windows-default, era, calendar — so byte 1 = year, byte 2 = month, byte 3 = day. Only these three
/// are exposed by the SDK/RptToXml (`DateFieldFormat.{Day,Month,Year}Format`).
pub(super) fn decode_date_format(lb: &[u8]) -> crate::model::DateFieldFormat {
    use crate::model::{
        DateSystemDefaultType, DayFormat, DayOfWeekFormat, MonthFormat, YearFormat,
    };
    let byte = |i: usize| i32::from(lb.get(i).copied().unwrap_or(0));
    crate::model::DateFieldFormat {
        year: YearFormat::from_code(byte(1)),
        month: MonthFormat::from_code(byte(2)),
        day: DayFormat::from_code(byte(3)),
        // byte 4 = `dayOfWeekType` (native writer order); not exposed by RptToXml, decoded for
        // record completeness only.
        day_of_week: DayOfWeekFormat::from_code(byte(4)),
        system_default: DateSystemDefaultType::from_code(byte(5)),
    }
}

/// Decode a `0x00ee` **BooleanFieldFormat** leaf: a one-byte enum OutputType at byte 0.
pub(super) fn decode_boolean_format(lb: &[u8]) -> crate::model::BooleanFieldFormat {
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
/// Left, Right, Top, Bottom; byte 9 is the `HasDropShadow` flag (non-zero = on); bytes 11-13 the
/// border colour (`BGR`), byte 14 a "default background" flag (`0xff`) and bytes 15-17 the
/// background colour (`BGR`).
pub(super) fn raise_border(node: &RecordNode, logical: &[u8]) -> crate::model::Border {
    let b = node.leaf_bytes(logical);
    let style = |i: usize| LineStyle::from_code(i32::from(b.get(i).copied().unwrap_or(0)));
    let bgr = |i: usize| -> Option<Color> {
        let (r, g, bl) = (*b.get(i + 2)?, *b.get(i + 1)?, *b.get(i)?);
        Some(Color {
            a: 255,
            r,
            g,
            b: bl,
        })
    };
    let background = match b.get(14).copied() {
        Some(0xff) => Some(Color::WHITE),
        _ => bgr(15),
    };
    crate::model::Border {
        left: style(0),
        right: style(1),
        top: style(2),
        bottom: style(3),
        has_drop_shadow: b.get(9).copied().unwrap_or(0) != 0,
        border_color: bgr(11),
        background_color: background,
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

/// Decode an object's hyperlink from its `0x00fc ObjectFormat` leaf. The `HyperlinkText` is the
/// first field of the format record's CSArchive tail — a big-endian `u32` byte count (including the
/// trailing NUL) at a fixed offset (leaf `15`, just past the flags header), then the text. An empty
/// target (count `1` = lone NUL) means no hyperlink → `None`.
///
/// The `HyperlinkType` enum is not isolated as a distinct stored byte from the available (website)
/// fixture — the engine reports it from the target — so it is inferred from the target here: a
/// `mailto:` target is an e-mail address, any other present target a file/website. STRUCTURAL:
/// RptToXml omits hyperlinks; verified against RAS `Format.HyperlinkText`/`HyperlinkType`.
pub(super) fn decode_hyperlink(leaf: &[u8]) -> Option<Hyperlink> {
    const HYPERLINK_TEXT_OFF: usize = 15;
    let (text, _) = read_lp_string(leaf.get(HYPERLINK_TEXT_OFF..)?)?;
    let kind = if text.starts_with("mailto:") {
        HyperlinkType::AnEMailAddress
    } else {
        HyperlinkType::AFileOrWebSite
    };
    Some(Hyperlink { text, kind })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CurrencySymbolFormat, DayOfWeekFormat};

    /// Build a numeric-format leaf: a 14-byte scalar header (with `decimal_places`, `rounding`,
    /// `currency_symbol` at their known offsets) followed by three length-prefixed symbol strings.
    fn numeric_leaf(decimal: &str, thousand: &str, currency: &str) -> Vec<u8> {
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
        push(&mut v, decimal);
        push(&mut v, thousand);
        push(&mut v, currency);
        v
    }

    #[test]
    fn numeric_symbols_decode_in_writer_order() {
        let leaf = numeric_leaf(".", ",", "kr ");
        let f = decode_numeric_format(&leaf);
        assert_eq!(f.decimal_places, 2);
        assert_eq!(f.currency_symbol, CurrencySymbolFormat::FixedSymbol);
        assert_eq!(f.decimal_symbol, ".");
        assert_eq!(f.thousand_symbol, ",");
        assert_eq!(f.currency_symbol_text, "kr ");
    }

    #[test]
    fn numeric_empty_currency_symbol() {
        let leaf = numeric_leaf(",", ".", "");
        let f = decode_numeric_format(&leaf);
        assert_eq!(f.decimal_symbol, ",");
        assert_eq!(f.thousand_symbol, ".");
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
}
