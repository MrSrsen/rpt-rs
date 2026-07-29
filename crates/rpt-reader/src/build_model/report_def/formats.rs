//! Per-object/area/section attribute and format record decoders.

use crate::build_model::record_values::colorref;
use crate::build_model::row_of;
use crate::codec::RecordNode;
use crate::field_table::table::{Cell, Row, Table};
use crate::field_table::tables as ft;
use crate::model::{
    Alignment, Font, FontColor, Hyperlink, HyperlinkType, LineStyle, ReportObject, ReportObjectKind,
};
use crate::records::rtype::*;

/// One member of the field-format family: a typed wrapper record, the one value record its type
/// names, and the field table that value record is read through.
pub(super) struct FieldFormatRecord {
    /// The wrapper record type, which is what the record grammar classifies.
    pub wrapper: u16,
    /// The value record type the wrapper parents.
    pub value: u16,
    /// The value record's field table.
    pub table: &'static Table,
}

/// The field-format family, stated once. A field's format is stored as seven independent typed
/// blocks — one per value domain — and every reading of them (the record grammar, the decode below)
/// takes the wrapper/value/table association from here.
pub(super) const FIELD_FORMAT_RECORDS: &[FieldFormatRecord] = &[
    FieldFormatRecord {
        wrapper: COMMON_FIELD_FORMAT_WRAPPER,
        value: COMMON_FIELD_FORMAT,
        table: &ft::COMMON_FIELD_FORMAT,
    },
    FieldFormatRecord {
        wrapper: NUMERIC_FIELD_FORMAT_WRAPPER,
        value: NUMERIC_FIELD_FORMAT,
        table: &ft::NUMERIC_FIELD_FORMAT,
    },
    FieldFormatRecord {
        wrapper: BOOLEAN_FIELD_FORMAT_WRAPPER,
        value: BOOLEAN_FIELD_FORMAT,
        table: &ft::BOOLEAN_FIELD_FORMAT,
    },
    FieldFormatRecord {
        wrapper: STRING_FIELD_FORMAT_WRAPPER,
        value: STRING_FIELD_FORMAT,
        table: &ft::STRING_FIELD_FORMAT,
    },
    FieldFormatRecord {
        wrapper: DATE_FIELD_FORMAT_WRAPPER,
        value: DATE_FIELD_FORMAT,
        table: &ft::DATE_FIELD_FORMAT,
    },
    FieldFormatRecord {
        wrapper: TIME_FIELD_FORMAT_WRAPPER,
        value: TIME_FIELD_FORMAT,
        table: &ft::TIME_FIELD_FORMAT,
    },
    FieldFormatRecord {
        wrapper: DATE_TIME_FIELD_FORMAT_WRAPPER,
        value: DATE_TIME_FIELD_FORMAT,
        table: &ft::DATE_TIME_FIELD_FORMAT,
    },
];

/// Whether a record type is a field-format wrapper — the block the grammar routes to the decode.
pub(super) fn is_field_format_wrapper(rtype: u16) -> bool {
    FIELD_FORMAT_RECORDS.iter().any(|r| r.wrapper == rtype)
}

/// The field table a field-format value record is read through, or `None` for anything else.
pub(crate) fn field_format_table(rtype: u16) -> Option<&'static Table> {
    FIELD_FORMAT_RECORDS
        .iter()
        .find(|r| r.value == rtype)
        .map(|r| r.table)
}

/// The `0x00fe` block's own statement of what it formats: `is_section` is `1` for a section's block
/// and `0` for the enclosing area's. An area and each of its sections store one block each, with the
/// same layout, and the two levels are independent — a section flag never writes the area's block.
pub(super) fn is_section_format(row: &Row) -> bool {
    row.i("is_section") == 1
}

/// Decode a section's `0x00fe` format block.
///
/// `visible` is the stored sense of `EnableSuppress`: `1` is shown, so suppression is its negation.
///
/// The engine seeds an area's block from its kind — a report-header area carries NewPageBefore, and
/// report-footer and page-footer areas carry NewPageAfter — so those read dense across any corpus,
/// while the author-set page breaks are sparse and live on sections. Both levels are reported as
/// stored; which one drives pagination is the layout engine's call, not the decoder's.
pub(super) fn decode_section_format(row: &Row) -> crate::model::SectionFormat {
    crate::model::SectionFormat {
        base: crate::model::SectionAreaFormatBase {
            suppress: !visible(row),
            new_page_before: row.i("new_page_before") != 0,
            new_page_after: row.i("new_page_after") != 0,
            keep_together: row.i("keep_together") != 0,
            print_at_bottom_of_page: row.i("print_at_bottom_of_page") != 0,
            reset_page_number_after: row.i("reset_page_number_after") != 0,
        },
        suppress_if_blank: row.i("suppress_blank_section") != 0,
        underlay_section: row.i("underlay_section") != 0,
        // The same `COLORREF` word as the object border, sentinel and all: no fill is the
        // overwhelmingly common case, since a section defaults to it.
        background_color: row
            .get("background_color")
            .and_then(Cell::u)
            .filter(|&word| word != u32::MAX)
            .map(colorref),
        ..Default::default()
    }
}

/// Decode an area's `0x00fe` format block — the same layout as [`decode_section_format`].
///
/// `show_area` is the stored sense of `EnableHideForDrillDown`, so hiding is its negation, and
/// suppression comes from the same `visible` word a section's does: an area has no format of its
/// own for the underlay flag beside it.
///
/// The record-per-page limit is a whole word near the end of the record and is only ever non-zero on
/// a Detail area.
pub(super) fn decode_area_format(row: &Row) -> crate::model::AreaFormat {
    crate::model::AreaFormat {
        base: crate::model::SectionAreaFormatBase {
            suppress: !visible(row),
            new_page_before: row.i("new_page_before") != 0,
            new_page_after: row.i("new_page_after") != 0,
            keep_together: row.i("keep_together") != 0,
            print_at_bottom_of_page: row.i("print_at_bottom_of_page") != 0,
            reset_page_number_after: row.i("reset_page_number_after") != 0,
        },
        hide_for_drill_down: row.get("show_area").and_then(Cell::i).unwrap_or(1) == 0,
        visible_records_per_page: row.i("visible_records_per_page"),
        ..Default::default()
    }
}

/// Whether a `0x00fe` block states its area or section is shown. A block too short to carry the word
/// states shown, which is the engine's own default.
fn visible(row: &Row) -> bool {
    row.get("visible").and_then(Cell::i).unwrap_or(1) != 0
}

/// Decode a `0x00f0` **CommonFieldFormat** record.
pub(crate) fn decode_common_format(row: &Row) -> crate::model::CommonFieldFormat {
    crate::model::CommonFieldFormat {
        suppress_if_duplicated: row.i("suppress_if_duplicated") != 0,
        use_system_defaults: row.i("use_system_defaults") != 0,
    }
}

/// Decode a `0x00f8` **NumericFieldFormat** record. Each field emits *two* `0x00f9`/`0x00f8` pairs —
/// a currency-format slot (first) and a number-format slot (second) — and the engine surfaces one
/// based on the field's value type (currency slot for a Currency-typed field, number slot
/// otherwise). This decodes one record; the caller selects which slot to keep.
///
/// The tail past the zero-value string (a flag and two more strings) is invariant across the corpus
/// and is not carried.
pub(crate) fn decode_numeric_format(row: &Row) -> crate::model::NumericFieldFormat {
    use crate::model::{CurrencyPosition, CurrencySymbolFormat, NegativeFormat, RoundingFormat};
    let code = |name: &str| row.u(name) as i32;
    crate::model::NumericFieldFormat {
        // A record that never reached the places defaults to two, the engine's own default.
        decimal_places: row.get("decimal_places").and_then(Cell::u).unwrap_or(2) as i32,
        rounding: RoundingFormat::from_code(code("rounding_type")),
        negative: NegativeFormat::from_code(code("negative_type")),
        currency_symbol: CurrencySymbolFormat::from_code(code("currency_symbol_type")),
        currency_position: CurrencyPosition::from_code(code("currency_position_type")),
        thousands_separator: row.i("thousands_separator") != 0,
        suppress_if_zero: row.i("suppress_if_zero") != 0,
        use_lead_zero: row.i("leading_zero") != 0,
        display_reverse_sign: row.i("reverse_sign") != 0,
        one_currency_symbol_per_page: row.i("one_currency_symbol_per_page") != 0,
        zero_value_string: row.text("zero_value_string").to_owned(),
        decimal_symbol: row.text("decimal_symbol").to_owned(),
        thousand_symbol: row.text("thousand_symbol").to_owned(),
        currency_symbol_text: row.text("currency_symbol").to_owned(),
    }
}

/// Decode a `0x00f4` **DateTimeFieldFormat** run. Byte 0 is the `DateTimeOrder` enum (which of the
/// date/time parts show, and in what order); the length-prefixed `DateTimeSeparator` string (the text
/// placed between the date and time parts, e.g. `"  "`) begins at offset 1.
pub(crate) fn decode_datetime_format(row: &Row) -> crate::model::DateTimeFieldFormat {
    crate::model::DateTimeFieldFormat {
        order: crate::model::DateTimeOrder::from_code(row.u("order") as i32),
        separator: row.text("separator").to_owned(),
    }
}

/// Decode a `0x00f6` **TimeFieldFormat** record. The whole SDK time surface is stored here: the five
/// element enums, then the AM and PM designators and the hour-minute and minute-second separators.
///
/// The clock base and the separators are genuine per-field facts, not host-locale defaults: a single
/// report stores 24-hour on one field and 12-hour on another, and a field whose minute-second
/// separator is the empty string renders its minute and second butted together (`0:0000`).
pub(crate) fn decode_time_format(row: &Row) -> crate::model::TimeFieldFormat {
    use crate::model::{AMPMFormat, HourFormat, MinuteFormat, SecondFormat, TimeBase};
    let code = |name: &str| row.u(name) as i32;
    crate::model::TimeFieldFormat {
        time_base: TimeBase::from_code(code("time_base")),
        am_pm_format: AMPMFormat::from_code(code("am_pm_type")),
        hour: HourFormat::from_code(code("hour_type")),
        minute: MinuteFormat::from_code(code("minute_type")),
        second: SecondFormat::from_code(code("second_type")),
        am_string: row.text("am_string").to_owned(),
        pm_string: row.text("pm_string").to_owned(),
        hour_minute_separator: row.text("hour_minute_separator").to_owned(),
        minute_second_separator: row.text("minute_second_separator").to_owned(),
    }
}

/// Decode a `0x00fa` **StringFieldFormat** record: how a string-valued field wraps, indents and
/// spaces its text.
///
/// `RightIndent` is the only indent that varies (72/144 twips); the first-line and left indents are
/// always `0`, so their assignment to the other two slots rests on the format's own declared field
/// order, not on an observed value. `TextFormat`, `EnableWordWrap` and `ReadingOrder` normally read
/// their zero/default values, `TextFormat` = `2` (HTMLText) being the exception.
///
/// The line spacing is invariant (single) but decoded for completeness. The record's one remaining
/// unnamed value sits between the spacing multiplier and the reading order — the structural home of
/// the SDK's `CharacterSpacing`, but that is unproven and not carried: it reads zero on every
/// report, and the authoring surface drops `StringFormat.CharacterSpacing` on save, so no minimal
/// pair can settle it.
pub(crate) fn decode_string_format(row: &Row) -> crate::model::StringFieldFormat {
    use crate::model::{IndentAndSpacingFormat, ReadingOrder, TextFormat, Twips};
    crate::model::StringFieldFormat {
        text_format: TextFormat::from_code(row.u("text_interpretation") as i32),
        enable_word_wrap: row.u("word_wrap") != 0,
        max_number_of_lines: row.u("max_lines") as u16,
        reading_order: ReadingOrder::from_code(row.u("reading_order") as i32),
        indent: IndentAndSpacingFormat {
            first_line_indent: Twips(row.i("first_line_indent")),
            left_indent: Twips(row.i("left_indent")),
            right_indent: Twips(row.i("right_indent")),
            line_spacing: crate::model::LineSpacing {
                spacing_type: match row.u("line_spacing_type") {
                    1 => crate::model::LineSpacingType::Exact,
                    _ => crate::model::LineSpacingType::Multiple,
                },
                raw: row.u("line_spacing"),
            },
        },
    }
}

/// Decode an object's `0x00fc` **ObjectFormat** record into the object's format.
///
/// The vertical alignment uses the shared [`Alignment`](crate::model::Alignment) ordinals (`6` =
/// top, `7` = vertical centre, `8` = bottom), so a record too short to carry it states top. The
/// rotation is the angle in degrees (`0` upright, `90` / `270` quarter turns).
///
/// A cross-tab ignores the stored keep-together flag, which it carries set like every other object,
/// so it keeps the default instead.
pub(super) fn apply_object_format(row: &Row, obj: &mut ReportObject) {
    obj.format.horizontal_alignment = Alignment::from_code(row.u("horizontal_alignment") as i32);
    obj.format.vertical_alignment = crate::model::VerticalAlignment::from_code(
        row.get("vertical_alignment").and_then(Cell::u).unwrap_or(6) as i32,
    );
    obj.format.text_rotation = crate::model::TextRotationAngle::from_code(row.u("rotation") as i32);
    obj.format.suppress.value = row.i("visible") == 0;
    if !matches!(obj.kind, ReportObjectKind::CrossTab(_)) {
        obj.format.keep_together = row
            .get("keep_object_together")
            .and_then(Cell::i)
            .is_none_or(|v| v != 0);
    }
    obj.format.can_grow = row.i("can_grow") != 0;
    obj.format.hyperlink = decode_hyperlink(row);
}

/// Decode a `0x00f2` **DateFieldFormat** record: the eight element enums, the five separator
/// strings, then where the weekday sits and what encloses it.
///
/// The separators are stored in the format's own order — the model names them prefix, first, second,
/// suffix, day-of-week — which is **not** the order the vendor's persisted model lists them in (that
/// one leads with the day-of-week separator), so taking the order from there mis-assigns three of
/// the five.
///
/// `dayOfWeekEnclosure` is an ordinal into a five-entry table of bracket pairs (none, `()`,
/// full-width `（）`, `[]`, full-width `［］`), which is why the SDK types the property as the
/// bracket *string* rather than an enum.
pub(crate) fn decode_date_format(row: &Row) -> crate::model::DateFieldFormat {
    use crate::model::{
        CalendarType, DateOrder, DateSystemDefaultType, DayFormat, DayOfWeekEnclosure,
        DayOfWeekFormat, DayOfWeekPosition, EraFormat, MonthFormat, YearFormat,
    };
    let code = |name: &str| row.u(name) as i32;
    crate::model::DateFieldFormat {
        date_order: DateOrder::from_code(code("date_order")),
        year: YearFormat::from_code(code("year_type")),
        month: MonthFormat::from_code(code("month_type")),
        day: DayFormat::from_code(code("day_type")),
        day_of_week: DayOfWeekFormat::from_code(code("day_of_week_type")),
        system_default: DateSystemDefaultType::from_code(code("system_default_type")),
        era: EraFormat::from_code(code("era_type")),
        calendar: CalendarType::from_code(code("calendar_type")),
        day_of_week_position: DayOfWeekPosition::from_code(code("day_of_week_position")),
        day_of_week_enclosure: DayOfWeekEnclosure::from_code(code("day_of_week_enclosure")),
        prefix_separator: row.text("zero_separator").to_owned(),
        first_separator: row.text("first_separator").to_owned(),
        second_separator: row.text("second_separator").to_owned(),
        suffix_separator: row.text("third_separator").to_owned(),
        day_of_week_separator: row.text("day_of_week_separator").to_owned(),
    }
}

/// Decode a `0x00ee` **BooleanFieldFormat** run: a one-byte enum OutputType at byte 0.
pub(crate) fn decode_boolean_format(row: &Row) -> crate::model::BooleanFieldFormat {
    crate::model::BooleanFieldFormat {
        output_type: crate::model::BooleanOutputType::from_code(row.u("output_type") as i32),
    }
}

/// Decode a field-format wrapper's value child into the field's `FieldFormat`.
///
/// Each typed wrapper parents the one value record its own type names, so the child's record type
/// selects both the table it is read through — through [`FIELD_FORMAT_RECORDS`] — and the slot it
/// fills. The numeric child streams twice
/// per field — the currency-format slot first, then the number-format slot — and
/// `currency_slot_pending` is what tells the two apart; the number slot is the reported value for a
/// non-currency field, so it also overwrites the first.
pub(super) fn apply_field_format_child(
    child: &RecordNode,
    logical: &[u8],
    ff: &mut crate::model::FieldFormat,
    currency_slot_pending: &mut bool,
) {
    let Some(table) = field_format_table(child.rtype) else {
        return;
    };
    let row = row_of(child, logical, table);
    match child.rtype {
        COMMON_FIELD_FORMAT => ff.common = decode_common_format(&row),
        NUMERIC_FIELD_FORMAT => {
            let nf = decode_numeric_format(&row);
            if *currency_slot_pending {
                ff.currency_numeric = nf.clone();
                *currency_slot_pending = false;
            }
            ff.numeric = nf;
        }
        BOOLEAN_FIELD_FORMAT => ff.boolean = decode_boolean_format(&row),
        DATE_FIELD_FORMAT => ff.date = decode_date_format(&row),
        TIME_FIELD_FORMAT => ff.time = decode_time_format(&row),
        STRING_FIELD_FORMAT => ff.string = decode_string_format(&row),
        DATE_TIME_FIELD_FORMAT => ff.date_time = decode_datetime_format(&row),
        _ => {}
    }
}

/// The font color of a text/field/heading object (drawing and picture objects have none).
pub(super) fn font_color_mut(obj: &mut ReportObject) -> Option<&mut FontColor> {
    match &mut obj.kind {
        ReportObjectKind::Text(t) => Some(&mut t.font_color),
        ReportObjectKind::Field(f) => Some(&mut f.font_color),
        ReportObjectKind::FieldHeading(h) => Some(&mut h.font_color),
        _ => None,
    }
}

/// Read an `ObjectName` record (`0x9e`) through its field table: the object's width and height,
/// then its name — which follows a variable-width `TwipRect` and so is not at a fixed offset.
pub(super) fn build_object_name(node: &RecordNode, logical: &[u8]) -> (String, i32, i32) {
    let row = row_of(node, logical, &ft::OBJECT_NAME);
    (row.text("name").to_owned(), row.i("width"), row.i("height"))
}

/// Decode an object border record (`0xec`): the four line styles in the order Left, Right, Top,
/// Bottom, then the drop-shadow flag and the two colors.
///
/// Each color is a big-endian `COLORREF` quad — `[type, Blue, Green, Red]` — so the whole word's top
/// byte is the type. The background carries the no-fill sentinel (all `0xff`); it is the default and
/// therefore dense, while an authored fill — including an explicit white `00 ff ff ff` — is sparse.
/// The border color never stores the sentinel: an unpainted edge is expressed by its line style, so
/// it is read unconditionally.
pub(super) fn build_border(row: &Row) -> crate::model::Border {
    let style = |name: &str| LineStyle::from_code(row.u(name) as i32);
    crate::model::Border {
        left: style("left_line_style"),
        right: style("right_line_style"),
        top: style("top_line_style"),
        bottom: style("bottom_line_style"),
        has_drop_shadow: row.i("drop_shadow") != 0,
        // A record too short to reach a colour states none, which is not the same as storing the
        // sentinel; both read as no colour, and neither is a black fill.
        border_color: row.get("border_color").and_then(Cell::u).map(colorref),
        background_color: row
            .get("background_color")
            .and_then(Cell::u)
            .filter(|&word| word != u32::MAX)
            .map(colorref),
        ..Default::default()
    }
}

/// An object-position record (`0xbe`): Left then Top, both narrowing twips — so where Top sits
/// follows how large Left is.
pub(super) fn build_object_pos(row: &Row) -> (i32, i32) {
    (row.u("left") as i32, row.u("top") as i32)
}

/// The font weight the host font system calls normal, and the one at and above which it calls a
/// font bold — the scale the record stores its weight on, so boldness is a threshold on it rather
/// than a flag of its own.
const NORMAL_FONT_WEIGHT: i32 = 400;
const BOLD_FONT_WEIGHT: i32 = 700;

/// Twips per point, the unit the record states a font's size in when it states a fraction of one.
const TWIPS_PER_POINT: f32 = 20.0;

/// Decode a `0x0008` **Font** record. A record that never reaches its face name names no font.
///
/// The record's own strikeout flag has no member on the model's font.
///
/// The size comes from the trailing twips, the only one of the record's two size fields that can
/// express a fraction of a point.
pub(super) fn build_font(row: &Row) -> Option<Font> {
    let name = row.get("face_name").and_then(Cell::text)?.to_owned();
    // A record too short to reach the weight states the normal one, which is the engine's default.
    let weight = row
        .get("weight")
        .and_then(Cell::u)
        .map_or(NORMAL_FONT_WEIGHT, |w| w as i32);
    // A record that ends before the twips states its size in whole points alone.
    let size_pt = row.get("size_twips").and_then(Cell::i).map_or_else(
        || row.u("size_points") as f32,
        |twips| twips as f32 / TWIPS_PER_POINT,
    );
    Some(Font {
        name,
        size_pt,
        bold: weight >= BOLD_FONT_WEIGHT,
        italic: row.i("italic") != 0,
        underline: row.i("underline") != 0,
        weight,
        ..Default::default()
    })
}

/// Decode an object's hyperlink from its `0x00fc` **ObjectFormat** record.
///
/// The type selector holds the [`HyperlinkType`] ordinal, and its `Undefined` code is the "no
/// hyperlink" state → `None`. Presence is decided by that selector alone, never by whether the
/// target text is empty: several real kinds (a field-value website, a report-part drill-down) carry
/// an empty target. A record that ends before the selector has no hyperlink either.
pub(super) fn decode_hyperlink(row: &Row) -> Option<Hyperlink> {
    let kind = HyperlinkType::from_code(row.get("hyperlink_type").and_then(Cell::u)? as i32);
    if kind == HyperlinkType::NoHyperlink {
        return None;
    }
    Some(Hyperlink {
        text: row.text("hyperlink_text").to_owned(),
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field_table::cursor::{Piece, RecordContent, StringFormat};
    use crate::field_table::table::{read_strings, Table};
    use crate::field_table::tables as ft;
    use crate::model::Color;
    use crate::model::{
        CurrencyPosition, CurrencySymbolFormat, DayOfWeekFormat, NegativeFormat, VerticalAlignment,
    };

    /// A numeric format read through its record's field table.
    fn numeric_fmt(run: &[u8]) -> crate::model::NumericFieldFormat {
        decode_numeric_format(&row_of_run(&ft::NUMERIC_FIELD_FORMAT, run))
    }

    /// A date format read through its record's field table.
    fn date_fmt(run: &[u8]) -> crate::model::DateFieldFormat {
        decode_date_format(&row_of_run(&ft::DATE_FIELD_FORMAT, run))
    }

    /// A time format read through its record's field table.
    fn time_fmt(run: &[u8]) -> crate::model::TimeFieldFormat {
        decode_time_format(&row_of_run(&ft::TIME_FIELD_FORMAT, run))
    }

    /// A record's reading under its own field table.
    ///
    /// A record built here carries no header to declare a string form, so every reading in this
    /// module names the enhanced form — the one the record-tree reader admits — rather than leaving
    /// it assumed.
    fn row_of_run(table: &'static Table, run: &[u8]) -> Row {
        read_strings(
            table,
            &RecordContent {
                rtype: table.rtype,
                schema: 0x0700,
                pieces: vec![Piece::Run(run.to_vec())],
            },
            StringFormat::Enhanced,
        )
        .row
    }

    /// Build a numeric-format run: a 14-byte scalar header (with `decimal_places`, `rounding`,
    /// `currency_symbol` at their known offsets) followed by the three length-prefixed symbol strings
    /// in stored order — thousand, decimal, currency.
    fn numeric_run(thousand: &str, decimal: &str, currency: &str) -> Vec<u8> {
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
        let run = numeric_run(",", ".", "kr ");
        let f = numeric_fmt(&run);
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
        let mut run = numeric_run(",", ".", "$");
        run[1] = 1; // EnableSuppressIfZero
        run[2] = 3; // Bracketed
        run[4] = 1; // ThousandsSeparator on
        run[10] = 2; // FloatingSymbol
        run[13] = 1; // LeadingCurrencyOutsideNegative
        let f = numeric_fmt(&run);
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
        let mut run = numeric_run("", "", "");
        run[4] = 0; // ThousandsSeparator off
        let f = numeric_fmt(&run);
        assert!(!f.thousands_separator);
        assert!(!f.suppress_if_zero);
        assert_eq!(
            f.currency_position,
            CurrencyPosition::LeadingCurrencyInsideNegative
        );
    }

    #[test]
    fn numeric_empty_currency_symbol() {
        let run = numeric_run(".", ",", "");
        let f = numeric_fmt(&run);
        assert_eq!(f.thousand_symbol, ".");
        assert_eq!(f.decimal_symbol, ",");
        assert_eq!(f.currency_symbol_text, "");
    }

    #[test]
    fn numeric_truncated_run_yields_empty_symbols() {
        // Only the scalar header, no string block — must not panic, symbols stay empty.
        let run = vec![0u8; 14];
        let f = numeric_fmt(&run);
        assert_eq!(f.decimal_symbol, "");
        assert_eq!(f.thousand_symbol, "");
        assert_eq!(f.currency_symbol_text, "");
    }

    /// A real `0x00f8` run from a Number field left at its defaults.
    const NUMERIC_RUN_BASE: [u8; 71] = [
        0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x02, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x02, 0x2e, 0x00, 0x00, 0x00, 0x00, 0x02,
        0x24, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x3c, 0x44, 0x65,
        0x66, 0x61, 0x75, 0x6c, 0x74, 0x20, 0x46, 0x6f, 0x72, 0x6d, 0x61, 0x74, 0x3e, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
    ];

    /// The flags that follow the symbol strings are not at a fixed run offset — they start where
    /// the currency symbol ends — and `DisplayReverseSign` is the third of them, so it is the sixth
    /// byte after the strings. Authored one property at a time off the run above.
    #[test]
    fn numeric_lead_zero_reverse_sign_and_per_page_symbol() {
        let base = numeric_fmt(&NUMERIC_RUN_BASE);
        assert!(base.use_lead_zero);
        assert!(!base.display_reverse_sign);
        assert!(!base.one_currency_symbol_per_page);

        // UseLeadZero off: the low byte of the two-byte flag at bytes 5-6.
        let mut lead_zero = NUMERIC_RUN_BASE;
        lead_zero[6] = 0;
        let f = numeric_fmt(&lead_zero);
        assert!(!f.use_lead_zero);
        assert!(!f.display_reverse_sign);
        assert!(!f.one_currency_symbol_per_page);

        // OneCurrencySymbolPerPage on: bytes 11-12, between the symbol format and the position.
        let mut per_page = NUMERIC_RUN_BASE;
        per_page[12] = 1;
        let f = numeric_fmt(&per_page);
        assert!(f.one_currency_symbol_per_page);
        assert!(f.use_lead_zero);
        assert_eq!(f.currency_position, CurrencyPosition::default());

        // DisplayReverseSign on: byte 37 here, six past the `"$"` symbol that ends at 32.
        let mut reverse = NUMERIC_RUN_BASE;
        reverse[37] = 1;
        let f = numeric_fmt(&reverse);
        assert!(f.display_reverse_sign);
        assert!(f.use_lead_zero);
        assert_eq!(f.currency_symbol_text, "$");

        // Widening the currency symbol moves the flag with it; a fixed offset 37 would then read a
        // byte of the symbol string instead.
        let mut wide = NUMERIC_RUN_BASE[..26].to_vec();
        wide.extend_from_slice(&[0, 0, 0, 11]);
        wide.extend_from_slice(b"US Dollars\0");
        wide.extend_from_slice(&NUMERIC_RUN_BASE[32..]);
        let f = numeric_fmt(&wide);
        assert_eq!(f.currency_symbol_text, "US Dollars");
        assert!(!f.display_reverse_sign);
        // Byte 37 is now inside the symbol text, so a fixed offset would read a letter as the flag.
        assert_eq!(wide[37], b'a');
        let mut wide_reverse = wide.clone();
        wide_reverse[46] = 1; // the same flag, now nine bytes later
        assert!(numeric_fmt(&wide_reverse).display_reverse_sign);
    }

    /// `ZeroValueString` is a stored literal that follows the three post-symbol flags. The engine's
    /// own "unset" marker is the literal `<Default Format>`, not an empty string.
    #[test]
    fn numeric_zero_value_string_is_a_stored_literal() {
        assert_eq!(
            numeric_fmt(&NUMERIC_RUN_BASE).zero_value_string,
            "<Default Format>"
        );
        // The same run authored with a real zero string, which shortens the run.
        let mut custom = NUMERIC_RUN_BASE[..38].to_vec();
        custom.extend_from_slice(&[0, 0, 0, 8]);
        custom.extend_from_slice(b"ZZZZZZZ\0");
        custom.extend_from_slice(&NUMERIC_RUN_BASE[59..]);
        let f = numeric_fmt(&custom);
        assert_eq!(f.zero_value_string, "ZZZZZZZ");
        // Nothing before it moves.
        assert_eq!(f.currency_symbol_text, "$");
        assert!(f.use_lead_zero);
        assert!(!f.display_reverse_sign);
        // A run that stops before the zero string reports none rather than panicking.
        assert_eq!(numeric_fmt(&NUMERIC_RUN_BASE[..38]).zero_value_string, "");
    }

    /// The numeric record does not end at its zero-value literal: a flag and two more strings
    /// follow, and the record accounts for every byte of them.
    ///
    /// Walking the run string by string reaches the literal and stops, so the ten bytes after it
    /// were never explained — which is exactly the shape of read a wrong length hides in.
    #[test]
    fn the_numeric_record_accounts_for_its_whole_tail() {
        let content = RecordContent {
            rtype: ft::NUMERIC_FIELD_FORMAT.rtype,
            schema: 0x0700,
            pieces: vec![Piece::Run(NUMERIC_RUN_BASE.to_vec())],
        };
        let r = read_strings(&ft::NUMERIC_FIELD_FORMAT, &content, StringFormat::Enhanced);
        assert!(r.exact() && r.complete, "{r:?}");
        assert_eq!(r.row.text("zero_value_string"), "<Default Format>");
        // The trailing group: present, and both strings genuinely empty rather than absent.
        assert_eq!(r.row.i("_u1"), 0);
        assert_eq!(r.row.text("_u2"), "");
        assert!(r.row.get("_u3").is_some());
    }

    /// A real `0x00f2` run from an explicit (non-system-default) date field left at its defaults:
    /// `/` for both element separators, a space before the (suppressed) weekday.
    const DATE_RUN_BASE: [u8; 38] = [
        0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x02, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x00, 0x02, 0x2f, 0x00, 0x00, 0x00, 0x00, 0x02, 0x2f, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
        0x00, 0x00, 0x00, 0x02, 0x20, 0x00, 0x00, 0x00,
    ];

    /// The same field with all five separators authored to distinct, different-length literals —
    /// the only way to tell the five slots apart, since a real field leaves three of them empty.
    const DATE_RUN_SEPARATORS: [u8; 55] = [
        0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x02, 0x01, 0x00, 0x00, 0x00, 0x05, 0x43, 0x43, 0x43,
        0x43, 0x00, 0x00, 0x00, 0x00, 0x03, 0x41, 0x41, 0x00, 0x00, 0x00, 0x00, 0x04, 0x42, 0x42,
        0x42, 0x00, 0x00, 0x00, 0x00, 0x06, 0x44, 0x44, 0x44, 0x44, 0x44, 0x00, 0x00, 0x00, 0x00,
        0x07, 0x45, 0x45, 0x45, 0x45, 0x45, 0x45, 0x00, 0x00, 0x00,
    ];

    /// The five separators are stored prefix, first, second, suffix, day-of-week.
    ///
    /// That is the Engine SDK's declaration order, but **not** the order the vendor's persisted
    /// model lists them in — that one leads with the day-of-week separator, so taking the order
    /// from it would mis-assign three of the five slots.
    #[test]
    fn date_separators_decode_in_stored_order() {
        let f = date_fmt(&DATE_RUN_SEPARATORS);
        assert_eq!(f.prefix_separator, "CCCC");
        assert_eq!(f.first_separator, "AA");
        assert_eq!(f.second_separator, "BBB");
        assert_eq!(f.suffix_separator, "DDDDD");
        assert_eq!(f.day_of_week_separator, "EEEEEE");

        // The default run: only the two element separators are set.
        let d = date_fmt(&DATE_RUN_BASE);
        assert_eq!(d.first_separator, "/");
        assert_eq!(d.second_separator, "/");
        assert_eq!(d.prefix_separator, "");
        assert_eq!(d.suffix_separator, "");
        assert_eq!(d.day_of_week_separator, " ");

        // Truncated run: no panic, no separators.
        let t = date_fmt(&DATE_RUN_BASE[..8]);
        assert_eq!(t.first_separator, "");
        assert_eq!(t.day_of_week_separator, "");
    }

    /// Era (byte 6), calendar (byte 7) and the day-of-week position (the byte after the separator
    /// strings) each read their own slot. Authored one property at a time off the default run.
    #[test]
    fn date_era_calendar_and_day_of_week_position() {
        use crate::model::{CalendarType, DayOfWeekPosition, EraFormat};
        let base = date_fmt(&DATE_RUN_BASE);
        assert_eq!(base.era, EraFormat::NoEra);
        assert_eq!(base.calendar, CalendarType::GregorianCalendar);
        assert_eq!(
            base.day_of_week_position,
            DayOfWeekPosition::LeadingPosition
        );

        let mut era = DATE_RUN_BASE;
        era[6] = 0;
        let f = date_fmt(&era);
        assert_eq!(f.era, EraFormat::ShortEra);
        assert_eq!(f.calendar, CalendarType::GregorianCalendar);

        // The calendar codes are the 1-based Win32 `CAL_*` identifiers, so `6` is Hijri.
        let mut calendar = DATE_RUN_BASE;
        calendar[7] = 6;
        let f = date_fmt(&calendar);
        assert_eq!(f.calendar, CalendarType::HijriCalendar);
        assert_eq!(f.era, EraFormat::NoEra);

        // The position sits past the separators, so its offset tracks their length rather than
        // being fixed: byte 36 on the default run, byte 53 on the authored-separator one.
        let mut position = DATE_RUN_BASE;
        position[36] = 1;
        assert_eq!(
            date_fmt(&position).day_of_week_position,
            DayOfWeekPosition::TrailingPosition
        );
        let mut long_position = DATE_RUN_SEPARATORS;
        long_position[53] = 1;
        assert_eq!(
            date_fmt(&long_position).day_of_week_position,
            DayOfWeekPosition::TrailingPosition
        );
        // Byte 36 of the longer run is separator text, not the position.
        let mut wrong = DATE_RUN_SEPARATORS;
        wrong[36] = 1;
        assert_eq!(
            date_fmt(&wrong).day_of_week_position,
            DayOfWeekPosition::LeadingPosition
        );
    }

    #[test]
    fn datetime_separator_decodes_lp_string_at_offset_1() {
        // Field bytes: byte0 DateTimeOrder, then LP string (BE u32 len incl NUL) "  ".
        let run = [0x00, 0x00, 0x00, 0x00, 0x03, 0x20, 0x20, 0x00];
        let f = decode_datetime_format(&row_of_run(&ft::DATE_TIME_FIELD_FORMAT, &run));
        assert_eq!(f.separator, "  ");
        // Truncated run: no panic, empty separator.
        let short = decode_datetime_format(&row_of_run(&ft::DATE_TIME_FIELD_FORMAT, &[0x00]));
        assert_eq!(short.separator, "");
    }

    #[test]
    fn datetime_order_from_byte0() {
        use crate::model::DateTimeOrder;
        // byte0 = DateTimeOrder: 0=DateThenTime, 2=DateOnly.
        let then_time = [0x00, 0x00, 0x00, 0x00, 0x03, 0x20, 0x20, 0x00];
        assert_eq!(
            decode_datetime_format(&row_of_run(&ft::DATE_TIME_FIELD_FORMAT, &then_time)).order,
            DateTimeOrder::DateThenTime
        );
        let date_only = [0x02, 0x00, 0x00, 0x00, 0x01, 0x00];
        assert_eq!(
            decode_datetime_format(&row_of_run(&ft::DATE_TIME_FIELD_FORMAT, &date_only)).order,
            DateTimeOrder::DateOnly
        );
    }

    /// The order is a narrowing enum: written in its two-byte form it still decodes to the same
    /// value, and the separator after it moves with it. Read as one byte the enum is wrong and the
    /// string is read out of the middle of its own length prefix.
    #[test]
    fn a_wide_order_enum_moves_the_separator_after_it() {
        use crate::model::DateTimeOrder;
        let wide = [0x80, 0x02, 0x00, 0x00, 0x00, 0x02, 0x2f, 0x00];
        let f = decode_datetime_format(&row_of_run(&ft::DATE_TIME_FIELD_FORMAT, &wide));
        assert_eq!(f.order, DateTimeOrder::DateOnly);
        assert_eq!(f.separator, "/");
    }

    #[test]
    fn date_order_from_byte0() {
        use crate::model::DateOrder;
        // 8-enum date header; byte0 = dateOrder (1 = DayMonthYear, 2 = MonthDayYear).
        let dmy = [1u8, 1, 1, 1, 2, 1, 0, 0];
        assert_eq!(date_fmt(&dmy).date_order, DateOrder::DayMonthYear);
        let mdy = [2u8, 0, 0, 0, 2, 1, 0, 0];
        assert_eq!(date_fmt(&mdy).date_order, DateOrder::MonthDayYear);
    }

    /// A real `0x00f6` run from an explicit datetime field: 24-hour clock, no-leading-zero hour,
    /// numeric minute, no second, lowercase designators, and an EMPTY minute-second separator.
    /// The four length-prefixed strings span offset 5 to the end.
    const TIME_RUN_EXPLICIT: [u8; 32] = [
        0x01, 0x01, 0x01, 0x00, 0x02, // timeBase, amPm, hour, minute, second
        0x00, 0x00, 0x00, 0x04, 0x20, 0x61, 0x6d, 0x00, // " am"
        0x00, 0x00, 0x00, 0x04, 0x20, 0x70, 0x6d, 0x00, // " pm"
        0x00, 0x00, 0x00, 0x02, 0x3a, 0x00, // ":"
        0x00, 0x00, 0x00, 0x01, 0x00, // ""
    ];

    /// A real `0x00f6` run from a system-default field in the same report: 12-hour clock, uppercase
    /// designators with a leading space, and both separators present.
    const TIME_RUN_SYSTEM_DEFAULT: [u8; 33] = [
        0x00, 0x01, 0x00, 0x00, 0x00, //
        0x00, 0x00, 0x00, 0x04, 0x20, 0x41, 0x4d, 0x00, // " AM"
        0x00, 0x00, 0x00, 0x04, 0x20, 0x50, 0x4d, 0x00, // " PM"
        0x00, 0x00, 0x00, 0x02, 0x3a, 0x00, // ":"
        0x00, 0x00, 0x00, 0x02, 0x3a, 0x00, // ":"
    ];

    #[test]
    fn time_format_elements_from_bytes_2_3_4() {
        use crate::model::{HourFormat, MinuteFormat, SecondFormat};
        let f = time_fmt(&TIME_RUN_EXPLICIT);
        assert_eq!(f.hour, HourFormat::NoLeadingZeroNumericHour);
        assert_eq!(f.minute, MinuteFormat::NumericMinute);
        assert_eq!(f.second, SecondFormat::NoSecond);
        // The same field with the second element turned on, and with the hour or the minute
        // suppressed — three authored variants of the run above, differing only at bytes 2/3/4.
        let mut seconds = TIME_RUN_EXPLICIT;
        seconds[4] = 0;
        assert_eq!(time_fmt(&seconds).second, SecondFormat::NumericSecond);
        let mut no_hour = TIME_RUN_EXPLICIT;
        no_hour[2] = 2;
        assert_eq!(time_fmt(&no_hour).hour, HourFormat::NoHour);
        let mut no_minute = TIME_RUN_EXPLICIT;
        no_minute[3] = 2;
        assert_eq!(time_fmt(&no_minute).minute, MinuteFormat::NoMinute);
    }

    /// `TimeBase` is byte 0 and `AMPMFormat` byte 1 — not the other way round. Both runs store
    /// `AMPMFormat` = 1, so an implementation that swapped them would read every field as 24-hour
    /// and never fail on the explicit run alone; the system-default run is what separates them.
    #[test]
    fn time_format_clock_base_and_am_pm_position_from_bytes_0_1() {
        use crate::model::{AMPMFormat, TimeBase};
        let explicit = time_fmt(&TIME_RUN_EXPLICIT);
        assert_eq!(explicit.time_base, TimeBase::TwentyFourHour);
        assert_eq!(explicit.am_pm_format, AMPMFormat::AMPMAfter);
        let sysdef = time_fmt(&TIME_RUN_SYSTEM_DEFAULT);
        assert_eq!(sysdef.time_base, TimeBase::TwelveHour);
        assert_eq!(sysdef.am_pm_format, AMPMFormat::AMPMAfter);
        // Byte 1 = 0 puts the designator in front of the time.
        let mut before = TIME_RUN_EXPLICIT;
        before[1] = 0;
        assert_eq!(time_fmt(&before).am_pm_format, AMPMFormat::AMPMBefore);
    }

    /// The four strings start at offset 5, in order: AM, PM, hour-minute separator, minute-second
    /// separator. Reading them from any other offset yields nothing that matches.
    #[test]
    fn time_format_designators_and_separators_from_offset_5() {
        let f = time_fmt(&TIME_RUN_EXPLICIT);
        assert_eq!(f.am_string, " am");
        assert_eq!(f.pm_string, " pm");
        assert_eq!(f.hour_minute_separator, ":");
        // Genuinely empty, which is why this field renders its minute and second butted together.
        assert_eq!(f.minute_second_separator, "");

        let g = time_fmt(&TIME_RUN_SYSTEM_DEFAULT);
        assert_eq!(g.am_string, " AM");
        assert_eq!(g.pm_string, " PM");
        assert_eq!(g.hour_minute_separator, ":");
        assert_eq!(g.minute_second_separator, ":");

        // A run whose designators are empty still lands the separators on the right slots.
        let empty_designators: [u8; 27] = [
            0x01, 0x01, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x01, 0x00, // ""
            0x00, 0x00, 0x00, 0x01, 0x00, // ""
            0x00, 0x00, 0x00, 0x02, 0x3a, 0x00, // ":"
            0x00, 0x00, 0x00, 0x02, 0x3a, 0x00, // ":"
        ];
        let h = time_fmt(&empty_designators);
        assert_eq!(h.am_string, "");
        assert_eq!(h.pm_string, "");
        assert_eq!(h.hour_minute_separator, ":");
        assert_eq!(h.minute_second_separator, ":");

        // Truncated run: no panic, empty strings.
        let t = time_fmt(&TIME_RUN_EXPLICIT[..5]);
        assert_eq!(t.am_string, "");
        assert_eq!(t.minute_second_separator, "");
    }

    /// A real Detail-area `0x00fe` run from a report authored with a record-per-page limit of 5.
    /// Byte 0x32 is the only one that differs from the same report authored without the limit.
    const AREA_FORMAT_RUN: [u8; 53] = [
        0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x00, 0x01, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00,
    ];

    /// An area format read through its record's field table.
    fn area_fmt(run: &[u8]) -> crate::model::AreaFormat {
        decode_area_format(&row_of_run(&ft::AREA_SECTION_FORMAT, run))
    }

    /// A section format read through its record's field table.
    fn section_fmt(run: &[u8]) -> crate::model::SectionFormat {
        decode_section_format(&row_of_run(&ft::AREA_SECTION_FORMAT, run))
    }

    /// The record-per-page limit is a **whole** big-endian word, four bytes from the record's end —
    /// not the single byte its low half occupies.
    ///
    /// A limit below 256 reads the same whether taken as the low byte or the whole word; a limit
    /// above 255 is what separates the two readings, and it reads as its remainder.
    #[test]
    fn area_format_reads_the_visible_record_limit() {
        assert_eq!(area_fmt(&AREA_FORMAT_RUN).visible_records_per_page, 5);

        let mut wide = AREA_FORMAT_RUN;
        wide[0x2f..0x33].copy_from_slice(&300i32.to_be_bytes());
        assert_eq!(area_fmt(&wide).visible_records_per_page, 300);
        assert_eq!(wide[0x32], 44, "the low byte alone is not the limit");

        // Zero is "no limit", which is what every area that does not set one stores.
        let mut none = AREA_FORMAT_RUN;
        none[0x32] = 0;
        assert_eq!(area_fmt(&none).visible_records_per_page, 0);
        // Short run (a block that stops well before the limit): no panic, no limit.
        assert_eq!(area_fmt(&AREA_FORMAT_RUN[..24]).visible_records_per_page, 0);
    }

    /// An area and a section state suppression in the **same** word — the one the wrapper's first
    /// conditional formula belongs to, stored as "visible" so `0` means suppressed.
    ///
    /// The word eleven values later is the underlay flag, which only a section ever sets: read as an
    /// area's suppression it would report every area shown regardless of what the area stores.
    #[test]
    fn an_area_and_a_section_suppress_in_the_same_word() {
        let mut suppressed = AREA_FORMAT_RUN;
        suppressed[5..7].copy_from_slice(&0i16.to_be_bytes());
        assert!(area_fmt(&suppressed).base.suppress);
        assert!(section_fmt(&suppressed).base.suppress);
        assert!(!area_fmt(&AREA_FORMAT_RUN).base.suppress);

        // The underlay word, which the old area reading took for suppression, leaves it alone.
        let mut underlaid = AREA_FORMAT_RUN;
        underlaid[21..23].copy_from_slice(&1i16.to_be_bytes());
        assert!(!area_fmt(&underlaid).base.suppress);
        assert!(section_fmt(&underlaid).underlay_section);
    }

    #[test]
    fn string_format_members_from_run() {
        use crate::model::{ReadingOrder, TextFormat};
        let string = |run: &[u8]| decode_string_format(&row_of_run(&ft::STRING_FIELD_FORMAT, run));
        let mut run = vec![0u8; 26];
        run[0] = 1; // word wrap on
        let f = string(&run);
        assert!(f.enable_word_wrap);
        assert_eq!(f.text_format, TextFormat::StandardText);
        assert_eq!(f.max_number_of_lines, 0);
        assert_eq!(f.reading_order, ReadingOrder::LeftToRight);
        // The interpretation is the sixth value, the line count the fifth (a whole word).
        run[15] = 2;
        run[13] = 0;
        run[14] = 5;
        let g = string(&run);
        assert_eq!(g.text_format, TextFormat::HTMLText);
        assert_eq!(g.max_number_of_lines, 5);
    }

    /// The line-spacing type comes **before** the spacing value, and the reading order is the
    /// record's last value — not the byte after the interpretation.
    ///
    /// Both are always zero on a real field, so this pins them to the order the format states
    /// rather than an observed value. Swapped, this run reports exact spacing as multiple and a
    /// left-to-right field as right-to-left.
    #[test]
    fn string_format_reading_order_is_the_records_last_value() {
        use crate::model::{LineSpacingType, ReadingOrder};
        let mut run = vec![0u8; 26];
        run[16] = 1; // line spacing: exact
        run[17..21].copy_from_slice(&360u32.to_be_bytes());
        run[25] = 0; // reading order: left to right
        let f = decode_string_format(&row_of_run(&ft::STRING_FIELD_FORMAT, &run));
        assert_eq!(f.indent.line_spacing.spacing_type, LineSpacingType::Exact);
        assert_eq!(f.indent.line_spacing.exact_twips(), Some(360));
        assert_eq!(f.reading_order, ReadingOrder::LeftToRight);

        // And the other way round: a right-to-left field spaced by a multiplier.
        let mut run = vec![0u8; 26];
        run[17..21].copy_from_slice(&0x0001_8000u32.to_be_bytes());
        run[25] = 1;
        let f = decode_string_format(&row_of_run(&ft::STRING_FIELD_FORMAT, &run));
        assert_eq!(f.indent.line_spacing.multiple(), Some(1.5));
        assert_eq!(f.reading_order, ReadingOrder::RightToLeft);
    }

    /// Decode a 34-byte `0xec` object-border run.
    fn decode_border_run(run: &[u8]) -> crate::model::Border {
        build_border(&row_of_run(&ft::BORDER, run))
    }

    /// A real `0xec` run: an object with a single-line top border colored RGB(153,140,44) and the
    /// all-`0xff` no-color sentinel in the background quad at bytes 14-17.
    const BORDER_RUN_NO_FILL: [u8; 34] = [
        0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0x2c, 0x8c, 0x99, 0xff,
        0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x1e, 0x01, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn border_background_no_fill_sentinel() {
        let no_fill = decode_border_run(&BORDER_RUN_NO_FILL);
        assert_eq!(no_fill.background_color, None);
        // The sentinel is confined to the background quad — the border color beside it is read
        // unconditionally, so the same `0xff` lead byte there is still a real color.
        assert_eq!(
            no_fill.border_color,
            Some(Color {
                a: 255,
                r: 153,
                g: 140,
                b: 44
            })
        );
        assert_eq!(no_fill.top, LineStyle::SingleLine);

        // Perturbation: flip only the sentinel's type byte to `0x00` and the very same quad becomes
        // an explicitly authored opaque white — the distinction the render depends on.
        let mut explicit_white = BORDER_RUN_NO_FILL;
        explicit_white[14] = 0x00;
        assert_eq!(
            decode_border_run(&explicit_white).background_color,
            Some(Color::WHITE)
        );

        // Perturbation: any other authored fill survives unchanged. RGB(95,58,31) stored BGR.
        let mut rust = BORDER_RUN_NO_FILL;
        rust[14..18].copy_from_slice(&[0x00, 0x1f, 0x3a, 0x5f]);
        assert_eq!(
            decode_border_run(&rust).background_color,
            Some(Color {
                a: 255,
                r: 95,
                g: 58,
                b: 31
            })
        );

        // Perturbation: a truncated run yields no color rather than reading past the end.
        assert_eq!(
            decode_border_run(&BORDER_RUN_NO_FILL[..16]).background_color,
            None
        );
    }

    /// The border's line width is a whole word, not the byte its low half looks like.
    ///
    /// A rule below 256 twips reads the same whether taken as the low byte or the whole word; a
    /// rule thicker than 255 twips is where they part, and the byte reading reports it as the
    /// remainder.
    #[test]
    fn border_line_width_is_a_whole_word() {
        let mut run = BORDER_RUN_NO_FILL;
        run[18..22].copy_from_slice(&300i32.to_be_bytes());
        let row = row_of_run(&ft::BORDER, &run);
        assert_eq!(row.i("line_width"), 300);
        assert_eq!(run[21], 44, "the low byte alone is not the width");
        // Nothing either side of it moves: the shape enum and the corner ellipse still land.
        assert_eq!(row.u("shape_kind"), 2);
        assert_eq!(
            (
                row.u("corner_ellipse_width"),
                row.u("corner_ellipse_height")
            ),
            (0, 0)
        );
    }

    /// A real `0x00fe` section-format run whose background quad at bytes 23-26 is the no-color
    /// sentinel.
    const SECTION_FORMAT_RUN: [u8; 53] = [
        0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x00, 0x01, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn section_background_no_fill_sentinel() {
        let plain = section_fmt(&SECTION_FORMAT_RUN);
        assert_eq!(plain.background_color, None);
        // The neighbouring flags stay put, so a shifted read of the quad would be caught here.
        assert!(!plain.base.suppress);
        assert!(plain.base.keep_together);
        // Perturbation: a real fill at the same offset is decoded BGR. RGB(192,192,192).
        let mut grey = SECTION_FORMAT_RUN;
        grey[23..27].copy_from_slice(&[0x00, 0xc0, 0xc0, 0xc0]);
        assert_eq!(
            section_fmt(&grey).background_color,
            Some(Color {
                a: 255,
                r: 192,
                g: 192,
                b: 192
            })
        );
        // Perturbation: the sentinel is a whole-quad match, so an explicit white is kept.
        let mut white = SECTION_FORMAT_RUN;
        white[23] = 0x00;
        assert_eq!(section_fmt(&white).background_color, Some(Color::WHITE));
    }

    /// A `0x00fc` ObjectFormat run: the six scalars, the tool-tip text, the hyperlink target, the
    /// rotation, the CSS class, two more words, one further string, and the hyperlink type.
    fn object_format_run(
        tool_tip: &str,
        target: &str,
        rotation: u16,
        css: &str,
        type_code: u8,
    ) -> Vec<u8> {
        fn push(v: &mut Vec<u8>, s: &str) {
            v.extend_from_slice(&((s.len() + 1) as u32).to_be_bytes()); // count incl NUL
            v.extend_from_slice(s.as_bytes());
            v.push(0);
        }
        let mut v = vec![0u8; 10];
        push(&mut v, tool_tip);
        push(&mut v, target);
        v.extend_from_slice(&rotation.to_be_bytes());
        push(&mut v, css);
        v.extend_from_slice(&[0, 0, 0, 0]);
        push(&mut v, "");
        v.push(type_code);
        v
    }

    /// An object format read through its record's field table.
    fn object_fmt(run: &[u8]) -> Row {
        row_of_run(&ft::OBJECT_FORMAT, run)
    }

    /// The alignment a `0x00fc` run states, as the model reports it.
    fn object_valign(run: &[u8]) -> VerticalAlignment {
        let mut obj = ReportObject::default();
        apply_object_format(&object_fmt(run), &mut obj);
        obj.format.vertical_alignment
    }

    #[test]
    fn object_vertical_alignment_is_the_third_value() {
        assert_eq!(object_valign(&[0, 1, 0, 6]), VerticalAlignment::Top);
        assert_eq!(
            object_valign(&[0, 1, 2, 7]),
            VerticalAlignment::VerticalCenter
        );
        assert_eq!(object_valign(&[0, 1, 0, 8]), VerticalAlignment::Bottom);
        // A record too short to reach it states top.
        assert_eq!(object_valign(&[0, 1]), VerticalAlignment::Top);
    }

    /// The vertical alignment is byte 3 of the real record, not one of its neighbours.
    ///
    /// Byte 2 beside it is the horizontal alignment and byte 4 is always zero, so both would decode
    /// as unmapped codes if the vertical alignment were read from either. Every other object in this
    /// report is top-aligned, so the assertion is a partition, not a spot check.
    #[test]
    fn object_vertical_alignment_matches_the_corpus() {
        let path = rpt_test_support::fixture("tests/fixtures/reports")
            .join("worrall/AlphaISOsByCountry.rpt");
        let rpt = crate::Rpt::open(&path).unwrap_or_else(|e| panic!("open: {e}"));
        let centered: Vec<&str> = rpt
            .report()
            .objects()
            .filter(|o| o.format.vertical_alignment == VerticalAlignment::VerticalCenter)
            .map(|o| o.name.as_str())
            .collect();
        assert_eq!(
            centered,
            [
                "Text7",
                "id1",
                "name1",
                "alpha2code1",
                "alpha3code1",
                "numericcode1",
                "CCTLDformatted1",
                "PageNofM1",
            ]
        );
        assert!(rpt.report().objects().all(|o| matches!(
            o.format.vertical_alignment,
            VerticalAlignment::Top | VerticalAlignment::VerticalCenter
        )));
    }

    /// The rotation angle follows the hyperlink target, whose length shifts it.
    #[test]
    fn text_rotation_follows_the_hyperlink_target() {
        use crate::model::TextRotationAngle;
        let rotation = |target: &str, angle: u16| {
            let mut obj = ReportObject::default();
            apply_object_format(
                &object_fmt(&object_format_run("", target, angle, "", 6)),
                &mut obj,
            );
            obj.format.text_rotation
        };
        for target in ["", "https://a.example", "https://example.com/a/longer/path"] {
            assert_eq!(
                rotation(target, 0),
                TextRotationAngle::Rotate0,
                "{target:?}"
            );
            assert_eq!(
                rotation(target, 90),
                TextRotationAngle::Rotate90,
                "{target:?}"
            );
            assert_eq!(
                rotation(target, 270),
                TextRotationAngle::Rotate270,
                "{target:?}"
            );
        }
        // With an empty target the angle lands at run offset 20; with a real one that offset falls
        // inside the URL text, where `"tt"` of `https://…` reads as the angle 0x7474.
        assert_eq!(
            crate::bytes::u16_be(&object_format_run("", "", 90, "", 6), 20),
            Some(90)
        );
        let url = object_format_run("", "https://google.com", 90, "", 6);
        assert_eq!(crate::bytes::u16_be(&url, 20), Some(0x7474));
        assert_eq!(
            rotation("https://google.com", 90),
            TextRotationAngle::Rotate90
        );
        // A record too short to reach it states upright.
        let mut obj = ReportObject::default();
        apply_object_format(&object_fmt(&[0, 1]), &mut obj);
        assert_eq!(obj.format.text_rotation, TextRotationAngle::Rotate0);
    }

    /// The tool-tip text is a **string** among the record's opening values, so everything after it —
    /// the hyperlink target, the rotation, the remaining strings and the hyperlink type — moves with
    /// its length.
    ///
    /// An empty tool-tip puts the target at run offset 15, where a fixed-offset read would happen
    /// to agree; a non-empty one reads the target's own count as text and the type from inside the
    /// target.
    #[test]
    fn a_tool_tip_moves_every_value_after_it() {
        use crate::model::HyperlinkType;
        let run = object_format_run("a tool tip", "mailto:a@b", 90, "", 1);
        let row = object_fmt(&run);
        assert_eq!(row.text("tool_tip_text"), "a tool tip");
        assert_eq!(row.text("hyperlink_text"), "mailto:a@b");
        assert_eq!(row.u("rotation"), 90);
        assert_eq!(row.u("hyperlink_type"), 1);
        assert_eq!(
            decode_hyperlink(&row).map(|h| h.kind),
            Some(HyperlinkType::AnEMailAddress)
        );
        // Offset 15 — where the target sits when the tool-tip is empty — is inside the tool-tip.
        assert_eq!(&run[15..25], b" tool tip\0");
    }

    /// A stored line spacing is a type and a 4-byte value: a 16.16 multiplier
    /// (`0x0001_0000` = `1.0`) when the type is multiple, a twip pitch when it is exact. The values
    /// are the ones the paragraph-typography fixture stores: single, 1.5, double, exact-360-twips.
    #[test]
    fn line_spacing_reads_its_value_by_its_type() {
        use crate::model::{LineSpacing, LineSpacingType};
        let multiple = |raw| LineSpacing {
            spacing_type: LineSpacingType::Multiple,
            raw,
        };
        assert_eq!(multiple(0x0001_0000).multiple(), Some(1.0));
        assert_eq!(multiple(0x0001_8000).multiple(), Some(1.5));
        assert_eq!(multiple(0x0002_0000).multiple(), Some(2.0));
        let exact = LineSpacing {
            spacing_type: LineSpacingType::Exact,
            raw: 360,
        };
        assert_eq!(exact.exact_twips(), Some(360));
        // A record that never reached the spacing reads as single.
        assert_eq!(LineSpacing::default().multiple(), Some(1.0));
    }

    #[test]
    fn date_day_of_week_type_from_byte4() {
        // 8 one-byte enums: date-order, year, month, day, day-of-week, windows-default, ...
        let run = [0u8, 0, 1, 1, 2, 1, 0, 0]; // dayOfWeekType (byte4) = 2 = NoDayOfWeek
        let f = date_fmt(&run);
        assert_eq!(f.day_of_week, DayOfWeekFormat::NoDayOfWeek);
        let run0 = [0u8, 0, 1, 1, 0, 1, 0, 0]; // byte4 = 0 = ShortDayOfWeek
        assert_eq!(date_fmt(&run0).day_of_week, DayOfWeekFormat::ShortDayOfWeek);
    }

    #[test]
    fn hyperlink_type_decoded_from_its_stored_selector() {
        use crate::model::HyperlinkType::*;
        let link = |text: &str, code: u8| {
            decode_hyperlink(&object_fmt(&object_format_run("", text, 0, "", code)))
        };
        // Undefined (6) is the "no hyperlink" state.
        assert!(link("", 6).is_none());
        // Each stored code maps to its variant; the target text is carried through verbatim.
        let cases = [
            ("https://example.com", 0u8, Website),
            ("someone@example.com", 1, AnEMailAddress),
            ("", 2, Html), // distinct from Website
            ("", 4, CurrentWebsiteField),
            ("", 5, CurrentWebsiteField), // the e-mail field value, grouped with it
            ("", 7, ReportPartDrilldown),
            ("Text2", 8, AnotherReportObject),
            ("", 3, Other(3)), // no variant of its own — preserved as its code
            ("", 99, Other(99)),
        ];
        for (text, code, want) in cases {
            let h = link(text, code).unwrap_or_else(|| panic!("code {code} is a hyperlink"));
            assert_eq!(h.kind, want, "code {code}");
            assert_eq!(h.text, text, "code {code} text");
        }
    }

    #[test]
    fn hyperlink_type_survives_a_nonempty_css_class() {
        let row = object_fmt(&object_format_run("", "mailto:a@b", 0, "myclass", 1));
        assert_eq!(row.text("css_class"), "myclass");
        let h = decode_hyperlink(&row).expect("hyperlink");
        assert_eq!(h.kind, crate::model::HyperlinkType::AnEMailAddress);
        assert_eq!(h.text, "mailto:a@b");
    }
}
