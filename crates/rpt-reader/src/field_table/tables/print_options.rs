//! Print options: the page setup, the paper, the columns, and the printer the report was saved
//! for.

use super::*;

/// `0x0066 PageSetup` — the four page margins, in twips.
///
/// It opens with the report kind (a narrowing enum), then two more enums — the second of which is
/// the report's canned formatting style — then the margins as
/// one `PageMargins`. All three enums are narrowing, so the style is the third *field* and never a
/// fixed byte offset. A margin stored as `i32::MIN` is the engine's
/// "use the default" sentinel rather than a distance — and a sentinel a narrowing twip could not
/// produce, which is what shows the four are fixed-width. Their order is left, right, top, bottom:
/// the engine's own margin accessor reads those four words in that order, at byte offsets 0, 4, 8
/// and 12 despite the conventional left-top-right-bottom naming of a `RECT`.
///
/// Everything past the margins is an opaque run: a long sequence of words, most of them
/// individually guarded on bytes remaining, around two nested records and two strings.
pub(crate) const PAGE_SETUP: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0066,
    name: "PageSetup",
    fields: &[
        Field::new("report_kind", Kind::VarU16),
        Field::new("_u0", Kind::VarU16),
        Field::new("report_style", Kind::VarU16),
        Field::new("margin_left", Kind::I32Be),
        Field::new("margin_right", Kind::I32Be),
        Field::new("margin_top", Kind::I32Be),
        Field::new("margin_bottom", Kind::I32Be),
        Field::new("_u2", Kind::Skip(52)),
        Field::new("xml_definition", Kind::Child(0x0151)),
        Field::new("_u3", Kind::Skip(10)),
        Field::new("object_marker", Kind::Child(0x0165)),
        Field::new("_u4", Kind::Skip(27)),
        Field::new("_u5", Kind::Skip(2)),
    ],
};

/// One entry of a run of field references.
const FIELD_REF: &[Field] = &[Field::new("field_ref", Kind::FieldRef)];

/// `0x018e PaperRect` — the sheet's width and height in twips, then four field references and a
/// trailing flag.
///
/// The 32 bytes after the dimensions are **four empty field references**, not four `(1, 0xffff)`
/// scalar pairs: that is what an empty reference looks like through a `u32` lens, and a report that
/// binds any of the four would break a fixed skip. The engine defaults the trailing flag to `1`
/// when the record ends before it — the same value stored records carry, so an explicit `1` cannot
/// be told apart from the default. Past the flag the engine reads nothing and simply skips to the
/// end of the record; writers leave nought,
/// two or four bytes there, so the run is declared as two words rather than one, and a record that
/// ends inside it still accounts for itself exactly.
pub(crate) const PAPER_RECT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x018e,
    name: "PaperRect",
    fields: &[
        Field::new("paper_width", Kind::I32Be),
        Field::new("paper_height", Kind::I32Be),
        Field::new(
            "field_refs",
            Kind::Repeat {
                count: Count::Fixed(4),
                body: FIELD_REF,
            },
        ),
        Field::new("_flag", Kind::I16Be),
        Field::new("_u0", Kind::Skip(2)),
        Field::new("_u1", Kind::Skip(2)),
    ],
};

/// `0x006c MultiColumn` — the "Format with Multiple Columns" label grid, a report-level singleton.
///
/// Two length-prefixed strings open the record, then the four detail dimensions in the designer's
/// own order — width, height, horizontal gap, vertical gap — the flow direction, and a trailing
/// word. The column width is `0` unless multi-column is enabled, so it doubles as the on/off
/// signal; the engine stores no column count and fits as many as span the printable width.
///
/// `direction` is `0` across-then-down and `1` down-then-across. It is the flow the engine
/// enforces, and it is **not** the off-state default: a report with multi-column disabled stores
/// `1` here, so reading it as the flow of a report that has no columns would report every ordinary
/// report as down-then-across.
///
/// The two strings and `detail_height` remain unconfirmed.
pub(crate) const MULTI_COLUMN: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x006c,
    name: "MultiColumn",
    fields: &[
        Field::new("_u0", Kind::Str),
        Field::new("_u1", Kind::Str),
        Field::new("column_width", Kind::I32Be),
        Field::new("detail_height", Kind::I32Be),
        Field::new("gap_h", Kind::I32Be),
        Field::new("gap_v", Kind::I32Be),
        Field::new("direction", Kind::VarU16),
        Field::new("_u2", Kind::I16Be),
    ],
};

/// Whether the page DEVMODE's `dmFields` mask has `bit` set — the gate on every member that
/// follows the fixed header.
fn dm_field(c: &Ctx<'_>, bit: u32) -> bool {
    c.row.u("dm_fields") & bit != 0
}

/// `0x0007 PageDevmode` (schema `0x0700`) — a big-endian, compacted form of the Win32 `DEVMODEW`
/// printer struct.
///
/// The leading `u32` is `dmFields` itself, and every member after `dmPaperSize` is present exactly
/// when its `DM_*` bit is set — so a member's position follows from which earlier ones the mask
/// selects, not from a fixed offset. Whether the `DM_ORIENTATION`/`DM_PAPERSIZE` slots are gated
/// the same way as the rest or simply fixed rests on the record's own reader rather than on a
/// confirming example; they are read as fixed. `dmFormName` is the trailing length-prefixed string
/// `DM_FORMNAME` (`0x00010000`, the mask's high word) selects.
pub(crate) const PAGE_DEVMODE: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0007,
    name: "PageDevmode",
    fields: &[
        Field::new("dm_fields", Kind::U32Be),
        Field::new("orientation", Kind::U16Be),
        Field::new("paper_size", Kind::U16Be),
        Field::when("paper_length", Kind::U16Be, |c| dm_field(c, 0x0000_0004)),
        Field::when("paper_width", Kind::U16Be, |c| dm_field(c, 0x0000_0008)),
        Field::when("scale", Kind::U16Be, |c| dm_field(c, 0x0000_0010)),
        Field::when("copies", Kind::U16Be, |c| dm_field(c, 0x0000_0100)),
        Field::when("default_source", Kind::U16Be, |c| dm_field(c, 0x0000_0200)),
        Field::when("print_quality", Kind::U16Be, |c| dm_field(c, 0x0000_0400)),
        Field::when("color", Kind::U16Be, |c| dm_field(c, 0x0000_0800)),
        Field::when("duplex", Kind::U16Be, |c| dm_field(c, 0x0000_1000)),
        Field::when("y_resolution", Kind::U16Be, |c| dm_field(c, 0x0000_2000)),
        Field::when("tt_option", Kind::U16Be, |c| dm_field(c, 0x0000_4000)),
        Field::when("collate", Kind::U16Be, |c| dm_field(c, 0x0000_8000)),
        Field::when("form_name", Kind::Str, |c| dm_field(c, 0x0001_0000)),
    ],
};

/// `0x0003 Printer` (schema `0x0700`) — the printer the report was last saved against: the driver
/// (`winspool`), the device name, and the port.
///
/// A report saved with no printer stores a different record under the same type number, at schema
/// `0x0701`; this table describes the `0x0700` form alone.
pub(crate) const PRINTER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0003,
    name: "Printer",
    fields: &[
        Field::new("_u0", Kind::U16Be),
        Field::new("driver_name", Kind::Str),
        Field::new("saved_printer_name", Kind::Str),
        Field::new("port_name", Kind::Str),
    ],
};
