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
