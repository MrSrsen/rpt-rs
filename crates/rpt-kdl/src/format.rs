//! Formatting sub-trees: font, border, field display format, and condition-formula lists.
//!
//! Each helper returns `None`/an empty `Vec` when the formatting is entirely default, so a plainly
//! formatted object emits no formatting nodes at all (sparse emission). Every struct is destructured
//! without `..` so a new model field fails to compile until it is emitted or skipped.

use rpt_model::{
    BooleanFieldFormat, Border, DateFieldFormat, DateTimeFieldFormat, FieldFormat, Font, FontColor,
    NumericFieldFormat, StringFieldFormat, TimeFieldFormat,
};

use crate::build::{color, int, Node};
use crate::enums;

/// The object/run font as a `font` child node, or `None` when it is the default font.
pub(crate) fn font_node(fc: &FontColor) -> Option<Node> {
    if fc == &FontColor::default() {
        return None;
    }
    let FontColor {
        color: fc_color,
        font,
        condition_formulas,
    } = fc;
    let Font {
        name,
        size_pt,
        bold,
        italic,
        underline,
        strikethrough,
        weight,
        charset,
    } = font;
    let mut n = Node::new("font")
        .arg_if(!name.is_empty(), name.as_str())
        .prop_if(*size_pt != 0.0, "size", *size_pt as f64)
        .flag("bold", *bold)
        .flag("italic", *italic)
        .flag("underline", *underline)
        .flag("strikethrough", *strikethrough)
        .prop_if(*weight != 0, "weight", int(*weight))
        .prop_if(*charset != 0, "charset", int(*charset));
    if *fc_color != rpt_model::Color::default() {
        n = n.prop("color", color(*fc_color));
    }
    n = n.children(condition_formula_nodes(condition_formulas));
    Some(n)
}

/// The object border as a `border` child node, or `None` when it is the default (no lines, no
/// colors, no shadow).
pub(crate) fn border_node(b: &Border) -> Option<Node> {
    if b == &Border::default() {
        return None;
    }
    let Border {
        top,
        bottom,
        left,
        right,
        has_drop_shadow,
        border_color,
        background_color,
        tight_horizontal,
        condition_formulas,
    } = b;
    let mut n = Node::new("border")
        .prop_if(
            *top != rpt_model::LineStyle::default(),
            "top",
            enums::line_style(*top),
        )
        .prop_if(
            *bottom != rpt_model::LineStyle::default(),
            "bottom",
            enums::line_style(*bottom),
        )
        .prop_if(
            *left != rpt_model::LineStyle::default(),
            "left",
            enums::line_style(*left),
        )
        .prop_if(
            *right != rpt_model::LineStyle::default(),
            "right",
            enums::line_style(*right),
        )
        .flag("drop-shadow", *has_drop_shadow)
        .flag("tight-horizontal", *tight_horizontal);
    if let Some(c) = border_color {
        n = n.prop("color", color(*c));
    }
    if let Some(c) = background_color {
        n = n.prop("background", color(*c));
    }
    n = n.children(condition_formula_nodes(condition_formulas));
    Some(n)
}

/// The field's type-specific display format as `numeric`/`date`/`boolean` child nodes, each emitted
/// only when it deviates from the value-type defaults.
pub(crate) fn field_format_nodes(f: &FieldFormat) -> Vec<Node> {
    let FieldFormat {
        // The common format (suppress-if-duplicated / use-system-defaults) is emitted on the field node.
        common: _,
        numeric,
        currency_numeric,
        boolean,
        string,
        date,
        time,
        date_time,
    } = f;
    let mut out = Vec::new();
    // Both stored numeric slots: the number-format slot the engine applies to a non-currency field,
    // and the currency-format slot it applies to a Currency-valued one.
    if let Some(n) = numeric_node("numeric", numeric) {
        out.push(n);
    }
    if let Some(n) = numeric_node("currency-numeric", currency_numeric) {
        out.push(n);
    }
    if let Some(n) = date_node(date) {
        out.push(n);
    }
    if let Some(n) = time_node(time) {
        out.push(n);
    }
    if let Some(n) = date_time_node(date_time) {
        out.push(n);
    }
    if let Some(n) = string_node(string) {
        out.push(n);
    }
    if let Some(n) = boolean_node(boolean) {
        out.push(n);
    }
    out
}

fn time_node(tf: &TimeFieldFormat) -> Option<Node> {
    if tf == &TimeFieldFormat::default() {
        return None;
    }
    let d = TimeFieldFormat::default();
    let TimeFieldFormat {
        time_base,
        am_pm_format,
        hour,
        minute,
        second,
        am_string,
        pm_string,
        hour_minute_separator,
        minute_second_separator,
    } = tf;
    Some(
        Node::new("time")
            .prop_if(
                *time_base != d.time_base,
                "base",
                enums::time_base(*time_base),
            )
            .prop_if(
                *am_pm_format != d.am_pm_format,
                "am-pm",
                enums::am_pm_format(*am_pm_format),
            )
            .prop_if(*hour != d.hour, "hour", enums::hour_format(*hour))
            .prop_if(*minute != d.minute, "minute", enums::minute_format(*minute))
            .prop_if(*second != d.second, "second", enums::second_format(*second))
            .str_if("am", am_string)
            .str_if("pm", pm_string)
            .str_if("hour-minute-separator", hour_minute_separator)
            .str_if("minute-second-separator", minute_second_separator),
    )
}

fn date_time_node(dtf: &DateTimeFieldFormat) -> Option<Node> {
    if dtf == &DateTimeFieldFormat::default() {
        return None;
    }
    let d = DateTimeFieldFormat::default();
    Some(
        Node::new("date-time")
            .prop_if(
                dtf.order != d.order,
                "order",
                enums::date_time_order(dtf.order),
            )
            .str_if("separator", &dtf.separator),
    )
}

fn string_node(sf: &StringFieldFormat) -> Option<Node> {
    if sf == &StringFieldFormat::default() {
        return None;
    }
    let d = StringFieldFormat::default();
    let StringFieldFormat {
        text_format,
        enable_word_wrap,
        max_number_of_lines,
        reading_order,
        indent,
    } = sf;
    Some(
        Node::new("string")
            .prop_if(
                *text_format != d.text_format,
                "text-format",
                enums::text_format(*text_format),
            )
            .flag("no-word-wrap", !*enable_word_wrap)
            .prop_if(
                *max_number_of_lines != d.max_number_of_lines,
                "max-lines",
                int(i32::from(*max_number_of_lines)),
            )
            .prop_if(
                *reading_order != d.reading_order,
                "reading-order",
                enums::reading_order(*reading_order),
            )
            .prop_if(
                indent.first_line_indent.0 != 0,
                "first-line-indent",
                int(indent.first_line_indent.0),
            )
            .prop_if(
                indent.left_indent.0 != 0,
                "left-indent",
                int(indent.left_indent.0),
            )
            .prop_if(
                indent.right_indent.0 != 0,
                "right-indent",
                int(indent.right_indent.0),
            ),
    )
}

fn numeric_node(name: &str, nf: &NumericFieldFormat) -> Option<Node> {
    if nf == &NumericFieldFormat::default() {
        return None;
    }
    let d = NumericFieldFormat::default();
    let NumericFieldFormat {
        decimal_places,
        rounding,
        negative,
        currency_symbol,
        currency_position,
        thousands_separator,
        suppress_if_zero,
        use_lead_zero,
        display_reverse_sign,
        one_currency_symbol_per_page,
        zero_value_string,
        decimal_symbol,
        thousand_symbol,
        currency_symbol_text,
    } = nf;
    Some(
        Node::new(name)
            .prop_if(
                *decimal_places != d.decimal_places,
                "decimals",
                int(*decimal_places),
            )
            .prop_if(
                *rounding != d.rounding,
                "rounding",
                enums::rounding_format(*rounding),
            )
            .prop_if(
                *negative != d.negative,
                "negative",
                enums::negative_format(*negative),
            )
            .prop_if(
                *currency_symbol != d.currency_symbol,
                "currency-symbol",
                enums::currency_symbol(*currency_symbol),
            )
            .prop_if(
                *currency_position != d.currency_position,
                "currency-position",
                enums::currency_position(*currency_position),
            )
            // Thousands separator defaults on, so record only when it is turned off.
            .flag("no-thousands-separator", !*thousands_separator)
            .flag("suppress-if-zero", *suppress_if_zero)
            // The leading zero defaults on, so record only when it is turned off.
            .flag("no-lead-zero", !*use_lead_zero)
            .flag("reverse-sign", *display_reverse_sign)
            .flag(
                "one-currency-symbol-per-page",
                *one_currency_symbol_per_page,
            )
            .str_if("zero-value", zero_value_string)
            .str_if("decimal-symbol", decimal_symbol)
            .str_if("thousand-symbol", thousand_symbol)
            .str_if("currency-text", currency_symbol_text),
    )
}

fn date_node(df: &DateFieldFormat) -> Option<Node> {
    if df == &DateFieldFormat::default() {
        return None;
    }
    let d = DateFieldFormat::default();
    let DateFieldFormat {
        date_order,
        day,
        month,
        year,
        system_default,
        day_of_week,
        era,
        calendar,
        day_of_week_position,
        day_of_week_enclosure,
        prefix_separator,
        first_separator,
        second_separator,
        suffix_separator,
        day_of_week_separator,
    } = df;
    Some(
        Node::new("date")
            .prop_if(
                *date_order != d.date_order,
                "order",
                enums::date_order(*date_order),
            )
            .prop_if(*day != d.day, "day", enums::day_format(*day))
            .prop_if(*month != d.month, "month", enums::month_format(*month))
            .prop_if(*year != d.year, "year", enums::year_format(*year))
            .prop_if(
                *system_default != d.system_default,
                "system-default",
                enums::date_system_default(*system_default),
            )
            .prop_if(
                *day_of_week != d.day_of_week,
                "day-of-week",
                enums::day_of_week_format(*day_of_week),
            )
            .prop_if(*era != d.era, "era", enums::era_format(*era))
            .prop_if(
                *calendar != d.calendar,
                "calendar",
                enums::calendar_type(*calendar),
            )
            .prop_if(
                *day_of_week_position != d.day_of_week_position,
                "day-of-week-position",
                enums::day_of_week_position(*day_of_week_position),
            )
            .prop_if(
                *day_of_week_enclosure != d.day_of_week_enclosure,
                "day-of-week-enclosure",
                enums::day_of_week_enclosure(*day_of_week_enclosure),
            )
            .str_if("prefix-separator", prefix_separator)
            .str_if("first-separator", first_separator)
            .str_if("second-separator", second_separator)
            .str_if("suffix-separator", suffix_separator)
            .str_if("day-of-week-separator", day_of_week_separator),
    )
}

fn boolean_node(bf: &BooleanFieldFormat) -> Option<Node> {
    if bf == &BooleanFieldFormat::default() {
        return None;
    }
    let BooleanFieldFormat { output_type } = bf;
    Some(Node::new("boolean").prop("output", enums::boolean_output(*output_type)))
}

/// A list of `(reserved-name, body)` conditional formulas as `when "<name>" "<body>"` child nodes.
/// The body is a formula string (rendered multi-line when it spans lines).
pub(crate) fn condition_formula_nodes(formulas: &[(String, String)]) -> Vec<Node> {
    formulas
        .iter()
        .map(|(name, body)| Node::new("when").arg(name.as_str()).arg(body.as_str()))
        .collect()
}
