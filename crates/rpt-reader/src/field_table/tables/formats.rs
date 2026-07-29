//! A placed field carries one format record per value type, each wrapped in a record of the next type
//! number up. These are the value records: what the field's own format states, as opposed to the
//! conditional formulas its wrapper carries.

use super::*;

/// `0x00ee BooleanFieldFormat` — which pair of words a boolean value is spelled with.
pub(crate) const BOOLEAN_FIELD_FORMAT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00ee,
    name: "BooleanFieldFormat",
    fields: &[Field::new("output_type", Kind::VarU16)],
};

/// `0x00f0 CommonFieldFormat` — the two flags a field carries whatever its value type. Both are
/// whole words, not the flag bytes their low halves look like.
pub(crate) const COMMON_FIELD_FORMAT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00f0,
    name: "CommonFieldFormat",
    fields: &[
        Field::new("suppress_if_duplicated", Kind::I16Be),
        Field::new("use_system_defaults", Kind::I16Be),
    ],
};

/// `0x00f4 DateTimeFieldFormat` — which of the date and time parts show and in what order, then the
/// text written between them.
pub(crate) const DATE_TIME_FIELD_FORMAT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00f4,
    name: "DateTimeFieldFormat",
    fields: &[
        Field::new("order", Kind::VarU16),
        Field::new("separator", Kind::Str),
    ],
};

/// `0x00ec Border` — an object's four edge styles, its two colours and the box it draws.
///
/// The four line styles open the record as narrowing enums (`NoLine ..= DotLine`), then the two
/// tight-fit flags and the drop shadow as whole words, the two colours as `COLORREF` longs, the line
/// width, the fill style, and the enum that tells a box from a line. The corner ellipse the box is
/// rounded by, and the word after it, are written only while the record still has content.
///
/// Both colours are stored as a big-endian quad `[type, blue, green, red]`, so the whole `u32`'s top
/// byte is the type: `0xff` is the no-colour sentinel, which only the background ever carries — an
/// unpainted edge is expressed by its line style, not by its colour.
pub(crate) const BORDER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00ec,
    name: "Border",
    fields: &[
        Field::new("left_line_style", Kind::VarU16),
        Field::new("right_line_style", Kind::VarU16),
        Field::new("top_line_style", Kind::VarU16),
        Field::new("bottom_line_style", Kind::VarU16),
        Field::new("tight_horizontal", Kind::I16Be),
        Field::new("tight_vertical", Kind::I16Be),
        Field::new("drop_shadow", Kind::I16Be),
        Field::new("border_color", Kind::U32Be),
        Field::new("background_color", Kind::U32Be),
        Field::new("line_width", Kind::I32Be),
        Field::new("fill_style", Kind::VarU16),
        Field::new("_u0", Kind::I16Be),
        Field::new("shape_kind", Kind::VarU16),
        Field::optional("corner_ellipse_width", Kind::VarU32),
        Field::optional("corner_ellipse_height", Kind::VarU32),
        Field::optional("_u1", Kind::U32Be),
    ],
};

/// `0x0008 Font` — the description a font is looked up by: its face name, the family and pitch that
/// substitute for it when the face is missing, its size, its three style flags and its weight.
///
/// The size is stored **twice, in two units**: as whole points, and again as twips in the trailing
/// word. The two are the same quantity — the writer emits `twips / 20` for the first and the twips
/// for the second — and only the trailing word can express a fractional point size, so it is the
/// authoritative one. It is also the only field a record need not carry: a record that stops after
/// the weight states its size in points alone, and the size in twips is then twenty times it.
///
/// The three flags are whole signed shorts rather than byte flags, and the third enum is written as
/// the constant `1` and read back into nothing.
pub(crate) const FONT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0008,
    name: "Font",
    fields: &[
        Field::new("face_name", Kind::Str),
        Field::new("family", Kind::VarU16),
        Field::new("pitch", Kind::VarU16),
        Field::new("_u0", Kind::VarU16),
        Field::new("size_points", Kind::U16Be),
        Field::new("italic", Kind::I16Be),
        Field::new("underline", Kind::I16Be),
        Field::new("strikeout", Kind::I16Be),
        Field::new("weight", Kind::U16Be),
        Field::optional("size_twips", Kind::I32Be),
    ],
};

/// `0x0100 FontColor` — the colour an object's text is drawn in.
///
/// The whole record is one colour and nothing else. It is the same big-endian quad as [`BORDER`]'s
/// two colours — `[type, blue, green, red]` — so the value's low byte is red and its top byte is
/// the colour type, `0x00` for a plain colour and `0xff` on the sentinel `0xffffffff`.
///
/// An object stores one of these per text run and the runs are not distinguished here; which run's
/// colour an object is reported as is the caller's question, not the record's.
pub(crate) const FONT_COLOR: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0100,
    name: "FontColor",
    fields: &[Field::new("color", Kind::U32Be)],
};

/// `0x00f8 NumericFieldFormat` — how a number- or currency-valued field is spelled.
///
/// Nine scalars, the three symbol strings, then the clipping flag. Everything after that is written
/// only while the record still has content: the reverse-sign pair and the zero-value literal, then a
/// flag and two more strings.
///
/// A field stores this record **twice** — a currency slot then a number slot — and which one the
/// engine surfaces follows the field's value type, so the pair is told apart by its position in the
/// stream rather than by anything in the record.
pub(crate) const NUMERIC_FIELD_FORMAT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00f8,
    name: "NumericFieldFormat",
    fields: &[
        Field::new("suppress_if_zero", Kind::I16Be),
        Field::new("negative_type", Kind::VarU16),
        Field::new("thousands_separator", Kind::I16Be),
        Field::new("leading_zero", Kind::I16Be),
        Field::new("decimal_places", Kind::U16Be),
        Field::new("rounding_type", Kind::VarU16),
        Field::new("currency_symbol_type", Kind::VarU16),
        Field::new("one_currency_symbol_per_page", Kind::I16Be),
        Field::new("currency_position_type", Kind::VarU16),
        Field::new("thousand_symbol", Kind::Str),
        Field::new("decimal_symbol", Kind::Str),
        Field::new("currency_symbol", Kind::Str),
        Field::new("allow_field_clipping", Kind::I16Be),
        Field::optional("_u0", Kind::I16Be),
        Field::optional("reverse_sign", Kind::I16Be),
        Field::optional("zero_value_string", Kind::Str),
        Field::optional("_u1", Kind::I16Be),
        Field::optional("_u2", Kind::Str),
        Field::optional("_u3", Kind::Str),
    ],
};

/// `0x00f2 DateFieldFormat` — how a date-valued field spells its parts.
///
/// Eight narrowing enums, the five separator strings, then where the weekday sits and what encloses
/// it. The enclosure is written only while the record still has content.
///
/// The separators are stored in the format's own order — zeroth, first, second, third, then the
/// day-of-week one — which is **not** the order the vendor's persisted model lists them in (that one
/// leads with the day-of-week separator), so taking the order from there mis-assigns three of five.
pub(crate) const DATE_FIELD_FORMAT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00f2,
    name: "DateFieldFormat",
    fields: &[
        Field::new("date_order", Kind::VarU16),
        Field::new("year_type", Kind::VarU16),
        Field::new("month_type", Kind::VarU16),
        Field::new("day_type", Kind::VarU16),
        Field::new("day_of_week_type", Kind::VarU16),
        Field::new("system_default_type", Kind::VarU16),
        Field::new("era_type", Kind::VarU16),
        Field::new("calendar_type", Kind::VarU16),
        Field::new("zero_separator", Kind::Str),
        Field::new("first_separator", Kind::Str),
        Field::new("second_separator", Kind::Str),
        Field::new("third_separator", Kind::Str),
        Field::new("day_of_week_separator", Kind::Str),
        Field::new("day_of_week_position", Kind::VarU16),
        Field::optional("day_of_week_enclosure", Kind::VarU16),
    ],
};

/// `0x00f6 TimeFieldFormat` — how a time-valued field spells its parts.
///
/// The five element enums, then the two designators and the two separators. The string order is the
/// format's, not the SDK's declaration order: the designators come first.
pub(crate) const TIME_FIELD_FORMAT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00f6,
    name: "TimeFieldFormat",
    fields: &[
        Field::new("time_base", Kind::VarU16),
        Field::new("am_pm_type", Kind::VarU16),
        Field::new("hour_type", Kind::VarU16),
        Field::new("minute_type", Kind::VarU16),
        Field::new("second_type", Kind::VarU16),
        Field::new("am_string", Kind::Str),
        Field::new("pm_string", Kind::Str),
        Field::new("hour_minute_separator", Kind::Str),
        Field::new("minute_second_separator", Kind::Str),
    ],
};

/// `0x00fa StringFieldFormat` — how a string-valued field wraps, indents and spaces its text.
///
/// The record is a straight run of ten values with no strings in it, so its length varies only by
/// which of the trailing fields the record still carries. Three of the last four are written only
/// while the record still has content: the interpretation, then the line-spacing trio, then the
/// reading order — each a later addition to the layout.
///
/// The reading order is the record's **last** value, not the byte after the interpretation, and the
/// line-spacing *type* precedes the multiplier rather than trailing the run. Both are always zero,
/// so the pair's order rests on the format's statement rather than on either taking a distinguishing
/// value.
pub(crate) const STRING_FIELD_FORMAT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00fa,
    name: "StringFieldFormat",
    fields: &[
        Field::new("word_wrap", Kind::VarU16),
        Field::new("first_line_indent", Kind::I32Be),
        Field::new("left_indent", Kind::I32Be),
        Field::new("right_indent", Kind::I32Be),
        Field::new("max_lines", Kind::U16Be),
        Field::optional("text_interpretation", Kind::VarU16),
        Field::optional("line_spacing_type", Kind::VarU16),
        Field::optional("line_spacing", Kind::U32Be),
        Field::optional("_u0", Kind::I32Be),
        Field::optional("reading_order", Kind::VarU16),
    ],
};

/// `0x00fc ObjectFormat` — the format an object carries whatever it draws: whether it shows, how it
/// aligns, whether it grows, and the hyperlink and HTML-export properties.
///
/// The six scalars that open the record are followed by a **string**, so nothing after the sixth
/// scalar is at a fixed offset: a non-empty tool-tip moves the hyperlink target, the rotation, the
/// remaining strings and the hyperlink type together.
///
/// The order of the two later strings is the format's own: the CSS class follows the rotation, and
/// the string just before the hyperlink type is a different one this reader does not name — both
/// empty, and told apart only by the order the format states.
///
/// The hyperlink type is the selector that decides whether the object has a hyperlink at all — its
/// `Undefined` code is the "no hyperlink" state — and never the emptiness of the target text, which
/// several real hyperlink kinds leave empty.
///
/// The record's last value is a mode name, spelled `Fit to box` wherever it is stored; a record that
/// stops before it takes one of two defaults chosen by `can_grow`.
pub(crate) const OBJECT_FORMAT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00fc,
    name: "ObjectFormat",
    fields: &[
        Field::new("visible", Kind::I16Be),
        Field::new("horizontal_alignment", Kind::VarU16),
        Field::new("vertical_alignment", Kind::VarU16),
        Field::new("keep_object_together", Kind::I16Be),
        Field::new("split_adornment", Kind::I16Be),
        Field::new("can_grow", Kind::I16Be),
        Field::new("tool_tip_text", Kind::Str),
        Field::optional("hyperlink_text", Kind::Str),
        Field::optional("rotation", Kind::U16Be),
        Field::optional("css_class", Kind::Str),
        Field::optional("_u0", Kind::I16Be),
        Field::optional("_u1", Kind::I16Be),
        Field::optional("_u2", Kind::Str),
        Field::optional("hyperlink_type", Kind::VarU16),
        Field::optional("_u3", Kind::U32Be),
        Field::optional("_u4", Kind::I16Be),
        Field::optional("_u5", Kind::I16Be),
        Field::optional("_u6", Kind::I16Be),
        Field::optional("_u7", Kind::I16Be),
        Field::optional("_u8", Kind::I16Be),
        Field::optional("_u9", Kind::Str),
    ],
};

/// `0x00fe AreaSectionFormat` — the format block an area and each of its sections both store.
///
/// The record opens with three structural values that say what it formats — the area kind, which
/// half of the header/footer pair, and whether this block is a section's or the enclosing area's —
/// and everything after them is the formattable properties, in the order the wrapper one type number
/// up names its conditional formulas.
///
/// The record-per-page limit is a whole big-endian long near the end, not the byte its low half
/// occupies: read as a byte-wide value, a limit above 255 reads as its remainder.
///
/// The background colour is a big-endian `COLORREF` quad `[type, blue, green, red]`, so the word's
/// top byte is the type and all-`0xff` is the no-fill sentinel.
pub(crate) const AREA_SECTION_FORMAT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00fe,
    name: "AreaSectionFormat",
    fields: &[
        Field::new("area_kind", Kind::VarU16),
        Field::new("is_header", Kind::I16Be),
        Field::new("is_section", Kind::I16Be),
        Field::new("visible", Kind::I16Be),
        Field::new("show_area", Kind::I16Be),
        Field::new("new_page_before", Kind::I16Be),
        Field::new("new_page_after", Kind::I16Be),
        Field::new("keep_together", Kind::I16Be),
        Field::new("suppress_blank_section", Kind::I16Be),
        Field::new("reset_page_number_after", Kind::I16Be),
        Field::new("print_at_bottom_of_page", Kind::I16Be),
        Field::new("underlay_section", Kind::I16Be),
        Field::new("background_color", Kind::U32Be),
        Field::new("_u0", Kind::I16Be),
        Field::optional("_u1", Kind::I16Be),
        Field::optional("_u2", Kind::I32Be),
        Field::optional("css_class", Kind::Str),
        Field::optional("_u3", Kind::I16Be),
        Field::optional("_u4", Kind::I16Be),
        Field::optional("_u5", Kind::I16Be),
        Field::optional("_u6", Kind::U8),
        Field::optional("visible_records_per_page", Kind::I32Be),
        Field::optional("_u7", Kind::I16Be),
    ],
};
