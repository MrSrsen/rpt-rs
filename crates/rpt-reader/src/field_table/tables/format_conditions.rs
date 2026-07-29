//! Each value record above is wrapped in the record one type number up, and every one of those
//! wrappers has the same shape: the value record first, then one conditional-format formula
//! reference per property the value record stores, in the value record's own field order. A slot
//! that names no formula is the empty reference — eight bytes of `00`s and `ff`s — which is why a
//! wrapper's length is a multiple of eight until a report binds one.
//!
//! The trailing slots of the longer wrappers are written only while the record still has content,
//! so a wrapper from before a property existed simply ends early.

use super::*;

/// `0x00ef BooleanFieldFormatWrapper` — the boolean format and the formula behind its one property.
pub(crate) const BOOLEAN_FIELD_FORMAT_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00ef,
    name: "BooleanFieldFormatWrapper",
    fields: &[
        Field::new("format", Kind::Child(0x00ee)),
        Field::new("output_type_formula", Kind::FieldRef),
    ],
};

/// `0x00f1 CommonFieldFormatWrapper` — the common format and the formula behind its suppression
/// flag. The wrapper carries no slot for `use_system_defaults`, which is not a conditional property.
pub(crate) const COMMON_FIELD_FORMAT_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00f1,
    name: "CommonFieldFormatWrapper",
    fields: &[
        Field::new("format", Kind::Child(0x00f0)),
        Field::new("suppress_if_duplicate_formula", Kind::FieldRef),
    ],
};

/// `0x00f5 DateTimeFieldFormatWrapper` — the date-time format and the two formulas behind it.
pub(crate) const DATE_TIME_FIELD_FORMAT_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00f5,
    name: "DateTimeFieldFormatWrapper",
    fields: &[
        Field::new("format", Kind::Child(0x00f4)),
        Field::new("order_formula", Kind::FieldRef),
        Field::new("separator_formula", Kind::FieldRef),
    ],
};

/// `0x00f7 TimeFieldFormatWrapper` — the time format and the nine formulas behind it, in the value
/// record's order: the five element enums, then the two separators, then the two designators.
pub(crate) const TIME_FIELD_FORMAT_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00f7,
    name: "TimeFieldFormatWrapper",
    fields: &[
        Field::new("format", Kind::Child(0x00f6)),
        Field::new("time_base_formula", Kind::FieldRef),
        Field::new("am_pm_type_formula", Kind::FieldRef),
        Field::new("hour_type_formula", Kind::FieldRef),
        Field::new("minute_type_formula", Kind::FieldRef),
        Field::new("second_type_formula", Kind::FieldRef),
        Field::new("hour_minute_separator_formula", Kind::FieldRef),
        Field::new("minute_second_separator_formula", Kind::FieldRef),
        Field::new("am_string_formula", Kind::FieldRef),
        Field::new("pm_string_formula", Kind::FieldRef),
    ],
};

/// `0x00fb StringFieldFormatWrapper` — the string format and the seven formulas behind it. The last
/// two are the trailing slots the record need not carry.
pub(crate) const STRING_FIELD_FORMAT_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00fb,
    name: "StringFieldFormatWrapper",
    fields: &[
        Field::new("format", Kind::Child(0x00fa)),
        Field::new("word_wrap_formula", Kind::FieldRef),
        Field::new("first_line_indent_formula", Kind::FieldRef),
        Field::new("left_indent_formula", Kind::FieldRef),
        Field::new("right_indent_formula", Kind::FieldRef),
        Field::new("max_lines_formula", Kind::FieldRef),
        Field::optional("text_interpretation_formula", Kind::FieldRef),
        Field::optional("reading_order_formula", Kind::FieldRef),
    ],
};

/// `0x00f9 NumericFieldFormatWrapper` — the numeric format and the fourteen formulas behind it, in
/// the value record's order: the nine scalar properties, the three symbol strings, the clipping
/// flag, and the reverse-sign flag the record need not carry.
pub(crate) const NUMERIC_FIELD_FORMAT_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00f9,
    name: "NumericFieldFormatWrapper",
    fields: &[
        Field::new("format", Kind::Child(0x00f8)),
        Field::new("suppress_if_zero_formula", Kind::FieldRef),
        Field::new("negative_type_formula", Kind::FieldRef),
        Field::new("thousands_separator_formula", Kind::FieldRef),
        Field::new("leading_zero_formula", Kind::FieldRef),
        Field::new("decimal_places_formula", Kind::FieldRef),
        Field::new("rounding_type_formula", Kind::FieldRef),
        Field::new("currency_symbol_type_formula", Kind::FieldRef),
        Field::new("one_currency_symbol_per_page_formula", Kind::FieldRef),
        Field::new("currency_position_type_formula", Kind::FieldRef),
        Field::new("thousand_symbol_formula", Kind::FieldRef),
        Field::new("decimal_symbol_formula", Kind::FieldRef),
        Field::new("currency_symbol_formula", Kind::FieldRef),
        Field::new("allow_field_clipping_formula", Kind::FieldRef),
        Field::optional("reverse_sign_formula", Kind::FieldRef),
    ],
};

/// `0x00f3 DateFieldFormatWrapper` — the date format and the fifteen formulas behind it, in the
/// value record's order: the eight element enums, the five separators, the day-of-week position and
/// the enclosure the record need not carry.
///
/// The separators are named here as the format states them — zeroth, first, second, third, then the
/// day-of-week one — which is the order the value record stores them in.
pub(crate) const DATE_FIELD_FORMAT_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00f3,
    name: "DateFieldFormatWrapper",
    fields: &[
        Field::new("format", Kind::Child(0x00f2)),
        Field::new("date_order_formula", Kind::FieldRef),
        Field::new("year_type_formula", Kind::FieldRef),
        Field::new("month_type_formula", Kind::FieldRef),
        Field::new("day_type_formula", Kind::FieldRef),
        Field::new("day_of_week_type_formula", Kind::FieldRef),
        Field::new("system_default_type_formula", Kind::FieldRef),
        Field::new("era_type_formula", Kind::FieldRef),
        Field::new("calendar_type_formula", Kind::FieldRef),
        Field::new("zero_separator_formula", Kind::FieldRef),
        Field::new("first_separator_formula", Kind::FieldRef),
        Field::new("second_separator_formula", Kind::FieldRef),
        Field::new("third_separator_formula", Kind::FieldRef),
        Field::new("day_of_week_separator_formula", Kind::FieldRef),
        Field::new("day_of_week_position_formula", Kind::FieldRef),
        Field::optional("day_of_week_enclosure_formula", Kind::FieldRef),
    ],
};

/// `0x00ed BorderWrapper` — an object's border and the eleven formulas behind it, in the border
/// record's own order: the four line styles, the two tight-fit flags, the drop shadow, the two
/// colours, the line width and the fill style.
pub(crate) const BORDER_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00ed,
    name: "BorderWrapper",
    fields: &[
        Field::new("border", Kind::Child(0x00ec)),
        Field::new("left_line_style_formula", Kind::FieldRef),
        Field::new("right_line_style_formula", Kind::FieldRef),
        Field::new("top_line_style_formula", Kind::FieldRef),
        Field::new("bottom_line_style_formula", Kind::FieldRef),
        Field::new("tight_horizontal_formula", Kind::FieldRef),
        Field::new("tight_vertical_formula", Kind::FieldRef),
        Field::new("drop_shadow_formula", Kind::FieldRef),
        Field::new("border_color_formula", Kind::FieldRef),
        Field::new("background_color_formula", Kind::FieldRef),
        Field::new("line_width_formula", Kind::FieldRef),
        Field::new("fill_style_formula", Kind::FieldRef),
    ],
};

/// `0x00fd ObjectFormatWrapper` — an object's format record and the fourteen formulas behind it.
/// The seven from the hyperlink target on are the trailing slots the record need not carry.
pub(crate) const OBJECT_FORMAT_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00fd,
    name: "ObjectFormatWrapper",
    fields: &[
        Field::new("format", Kind::Child(0x00fc)),
        Field::new("visibility_formula", Kind::FieldRef),
        Field::new("horizontal_alignment_formula", Kind::FieldRef),
        Field::new("vertical_alignment_formula", Kind::FieldRef),
        Field::new("keep_object_together_formula", Kind::FieldRef),
        Field::new("split_adornment_formula", Kind::FieldRef),
        Field::new("can_grow_formula", Kind::FieldRef),
        Field::new("tool_tip_text_formula", Kind::FieldRef),
        Field::optional("hyperlink_text_formula", Kind::FieldRef),
        Field::optional("rotation_formula", Kind::FieldRef),
        Field::optional("css_class_formula", Kind::FieldRef),
        Field::optional("display_string_formula", Kind::FieldRef),
        Field::optional("delta_x_formula", Kind::FieldRef),
        Field::optional("delta_width_formula", Kind::FieldRef),
        Field::optional("graphic_location_formula", Kind::FieldRef),
    ],
};

/// `0x00ff SectionFormatWrapper` — an area's or section's format record and the fourteen formulas
/// behind it. The four from the eleventh on are the trailing slots the record need not carry; the
/// eleventh is the only slot the format states no name for.
pub(crate) const SECTION_FORMAT_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00ff,
    name: "SectionFormatWrapper",
    fields: &[
        Field::new("format", Kind::Child(0x00fe)),
        Field::new("visibility_formula", Kind::FieldRef),
        Field::new("show_area_formula", Kind::FieldRef),
        Field::new("new_page_before_formula", Kind::FieldRef),
        Field::new("new_page_after_formula", Kind::FieldRef),
        Field::new("keep_together_formula", Kind::FieldRef),
        Field::new("suppress_blank_section_formula", Kind::FieldRef),
        Field::new("reset_page_number_after_formula", Kind::FieldRef),
        Field::new("print_at_bottom_of_page_formula", Kind::FieldRef),
        Field::new("underlay_section_formula", Kind::FieldRef),
        Field::new("background_color_formula", Kind::FieldRef),
        Field::optional("_u0_formula", Kind::FieldRef),
        Field::optional("css_class_formula", Kind::FieldRef),
        Field::optional("new_page_after_n_records_formula", Kind::FieldRef),
        Field::optional("clamp_page_footer_formula", Kind::FieldRef),
    ],
};

/// Whether the record carries the block of five font slots that follows the colour formula.
///
/// The colour formula is written on its own; the five after it are one block behind a single check
/// that the record still has content, so the first of them decides for all five.
fn carries_the_font_slot_block(c: &Ctx<'_>) -> bool {
    c.row.get("height_formula").is_some()
}

/// `0x0101 FontConditionFormat` — an object's font colour record and the six formulas behind the
/// font's own properties, in the order the font states them: the colour, then the height, the
/// strikeout, the underline, the style and the face name.
///
/// It is the same shape as the format wrappers above — a nested value record, then one conditional-
/// format formula reference per property — and differs only in where the value record's own
/// properties live: the colour is the nested record's, and the five after it belong to the font
/// that follows this record rather than to anything nested inside it.
pub(crate) const FONT_CONDITION_FORMAT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0101,
    name: "FontConditionFormat",
    fields: &[
        Field::new("font_color", Kind::Child(0x0100)),
        Field::new("color_formula", Kind::FieldRef),
        Field::optional("height_formula", Kind::FieldRef),
        Field::when(
            "strikeout_formula",
            Kind::FieldRef,
            carries_the_font_slot_block,
        ),
        Field::when(
            "underline_formula",
            Kind::FieldRef,
            carries_the_font_slot_block,
        ),
        Field::when(
            "font_style_formula",
            Kind::FieldRef,
            carries_the_font_slot_block,
        ),
        Field::when(
            "font_name_formula",
            Kind::FieldRef,
            carries_the_font_slot_block,
        ),
    ],
};

/// `0x0111 ObjectConnection` — one edge of the report designer's connection graph: the layout
/// object a guideline is attached to, and the state of the attachment.
///
/// The record does not state an object twice. Its leading pair and its trailing four words are one
/// object identifier, split by the attachment's own state: a kind and an index — the shape every
/// handle in the format resolves by — then four qualifier words naming a sub-object within it,
/// which read `-1` when there is none. A leading pair of `-1, -1` is the identifier for no object
/// at all, and the record then attaches to nothing.
///
/// The qualifier words are one group guarded together, so a record that stops after the attachment
/// state carries none of them rather than some.
pub(crate) const OBJECT_CONNECTION: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0111,
    name: "ObjectConnection",
    fields: &[
        Field::new("object_kind", Kind::I16Be),
        Field::new("object_index", Kind::I16Be),
        Field::new("_u0", Kind::I32Be),
        Field::new("_u1", Kind::I32Be),
        Field::new("_u2", Kind::VarU16),
        Field::new("_u3", Kind::VarU16),
        Field::optional(
            "object_qualifier",
            Kind::Repeat {
                count: Count::Fixed(4),
                body: &[Field::new("word", Kind::I16Be)],
            },
        ),
    ],
};
