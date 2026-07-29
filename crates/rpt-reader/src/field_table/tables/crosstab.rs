//! Cross-tab: the object, its row and column axes, and the formats of its grid.

use super::*;

/// `0x00b9 CrossTabWrapper` — the block that opens a cross-tab, wrapping its `0x00b8` opener.
///
/// Like every object opener, the nested record comes **first**: the wrapper's own flags follow it.
pub(crate) const CROSSTAB_WRAPPER: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00b9,
    name: "CrossTabWrapper",
    fields: &[
        Field::new("crosstab_object", Kind::Child(0x00b8)),
        Field::new("_u0", Kind::Skip(1)),
        Field::new("suppress_column_grand_totals", Kind::U8),
        Field::new("_u1", Kind::Skip(1)),
        Field::new("suppress_row_grand_totals", Kind::U8),
        Field::new("_u2", Kind::Skip(27)),
    ],
};

/// `0x00b8 CrossTabObject` — the cross-tab opener, carrying the grid display options.
///
/// The four display booleans are whole `i16`s, not the low byte of one: their bytes are the second
/// of each pair. The grid pen opens the record — two words and a `COLORREF` that go `1`, `1`,
/// `0x00000000` with the grid shown and `0`, `0`, `0xffffffff` (`CLR_INVALID`) with it hidden, so
/// which of the two words the designer's single "show grid" checkbox is cannot be told apart here.
///
/// **There is no cell-margin flag in this record.** Turning cell margins off shrinks the grid, so
/// the third byte of the grid width happens to read `1` on a grid between 256 and 511 twips wide
/// and `0` on a narrower one; the stored margin is on the `0x00d6` grid cells
/// ([`CROSSTAB_GRID_CELL`]).
///
/// The three counts size the arrays the engine fills from the grid's own records: the number of
/// grid columns, of grid rows, and of cells. They are `3 · 2 · 6` on a cross-tab whose `0x00d6`
/// cells run `row` 0..1 × `column` 0..2, which is what separates the first two. Everything past
/// them is guarded on bytes remaining, and the whole tail is present on every corpus record.
pub(crate) const CROSSTAB_OBJECT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00b8,
    name: "CrossTabObject",
    fields: &[
        Field::new("object_name", Kind::Child(0x009e)),
        Field::new("show_grid", Kind::I16Be),
        Field::new("_grid_pen", Kind::I16Be),
        Field::new("grid_color", Kind::U32Be),
        Field::new("_u0", Kind::I32Be),
        Field::new("_u1", Kind::I32Be),
        Field::new("_pos_x", Kind::VarU32),
        Field::new("_pos_y", Kind::VarU32),
        Field::new("_u2", Kind::U8),
        Field::new("keep_columns_together", Kind::I16Be),
        Field::new("repeat_row_labels", Kind::I16Be),
        Field::new("suppress_empty_columns", Kind::I16Be),
        Field::new("suppress_empty_rows", Kind::I16Be),
        Field::new("grid_columns", Kind::U16Be),
        Field::new("grid_rows", Kind::U16Be),
        Field::new("grid_cells", Kind::U16Be),
        Field::new("_u3", Kind::I16Be),
        Field::new("_u4", Kind::I16Be),
        Field::new("_u5", Kind::I16Be),
        Field::new("_u6", Kind::I32Be),
        Field::new("_u7", Kind::I16Be),
        Field::new("_u8", Kind::I32Be),
        Field::new("_u9", Kind::I16Be),
    ],
};

/// `0x00d6 CrossTabGridCell` — one cell of the cross-tab grid: its margins, its position and size,
/// and its `(row, column)` index into the grid.
///
/// The two leading margins are the **stored** cell-margin setting: the designer's "show cell
/// margins" writes 100 twips into both and clearing it writes zero, shrinking the grid by twice the
/// margin in each direction. Every cell of a grid carries the same pair.
pub(crate) const CROSSTAB_GRID_CELL: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00d6,
    name: "CrossTabGridCell",
    fields: &[
        Field::new("margin_h", Kind::I32Be),
        Field::new("margin_v", Kind::I32Be),
        Field::new("left", Kind::VarU32),
        Field::new("top", Kind::VarU32),
        Field::new("width", Kind::VarU32),
        Field::new("height", Kind::VarU32),
        Field::new("row", Kind::U16Be),
        Field::new("column", Kind::U16Be),
    ],
};

/// `0x00cb CrossTabDimensionField` — one dimension level: its background color, its size and
/// position, and the bound field reference.
///
/// The size and position are a `TwipSize` and a `TwipPoint` — four narrowing twips, eight bytes
/// only while each is under `0x8000`, which is what fixes where the reference begins. The reference
/// itself is guarded on bytes remaining; a grand-total level stores it as an empty string (a lone
/// NUL), not as an absent field.
pub(crate) const CROSSTAB_DIM_FIELD: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00cb,
    name: "CrossTabDimensionField",
    fields: &[
        Field::new("background_color", Kind::U32Be),
        Field::new("width", Kind::VarU32),
        Field::new("height", Kind::VarU32),
        Field::new("left", Kind::VarU32),
        Field::new("top", Kind::VarU32),
        Field::new("_u0", Kind::I32Be),
        Field::new("_u1", Kind::I32Be),
        Field::new("_u2", Kind::I16Be),
        Field::new("_u3", Kind::U8),
        Field::new("_u4", Kind::I16Be),
        Field::new("_u5", Kind::I16Be),
        Field::new("field_ref", Kind::Str),
    ],
};

/// `0x00ce CrossTabDimension` — opens a column-axis level.
///
/// The word past the nested level record is a **count**: the engine appends that many empty slots
/// to the axis. It is read only if the record has bytes left, so a record may legitimately end
/// before it.
pub(crate) const CROSSTAB_COLUMN_AXIS: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00ce,
    name: "CrossTabColumnAxis",
    fields: &[
        Field::new("level", Kind::Child(0x00cc)),
        Field::new("level_count", Kind::U16Be),
    ],
};

/// `0x00d2 CrossTabRecord` — opens a row-axis level, with the same shape as the column axis.
pub(crate) const CROSSTAB_ROW_AXIS: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x00d2,
    name: "CrossTabRowAxis",
    fields: &[
        Field::new("level", Kind::Child(0x00cd)),
        Field::new("level_count", Kind::U16Be),
    ],
};

/// `0x017e CrossTabCustomMembersBegin` — opens a cross-tab's custom-group-members collection, and
/// states how many members are in it.
///
/// The count is the whole record. It drives the read outright: the reader takes the word and then
/// reads exactly that many members, each its own `0x0180`…`0x0181` record pair, before the `0x017f`
/// that closes the collection — so a member run is as long as the word says and is never counted
/// from the records themselves.
///
/// A cross-tab writes the collection whether or not it has custom members, so an empty one is the
/// ordinary case, not an omission.
pub(crate) const CROSSTAB_CUSTOM_MEMBERS: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x017e,
    name: "CrossTabCustomMembersBegin",
    fields: &[Field::new("member_count", Kind::U32Be)],
};

/// `0x0143 CrossTabGridFormat` — opens the cell-format run, and states how long it is.
///
/// The word is a **count**: that many `0x0145` cell formats follow, and then the `0x0144` end
/// record. It is the record's only field and the record need not carry it — an empty `0x0143`
/// stands for [`CROSSTAB_GRID_CELL_DEFAULT_COUNT`] cell formats rather than for none.
pub(crate) const CROSSTAB_GRID_FORMAT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0143,
    name: "CrossTabGridFormat",
    fields: &[Field::optional("cell_count", Kind::U16Be)],
};

/// How many `0x0145` cell formats follow a `0x0143` that stores no count.
pub(crate) const CROSSTAB_GRID_CELL_DEFAULT_COUNT: u32 = 16;

/// `0x0145 CrossTabGridCellFormat` — one grid region's format: a word, an enum, a colour and the
/// region's enabled flag.
///
/// Only `enabled` carries information; the other three fields are always `0`, `1` and `0`, so this
/// table's shape rests on the record's own reader rather than on a confirming example, and none of
/// the three has a meaning pinned here. In particular the colour is one whole `u32`, not three
/// channel bytes with padding either side.
pub(crate) const CROSSTAB_GRID_CELL_FORMAT: Table = Table {
    dialect: Dialect::Contents,
    rtype: 0x0145,
    name: "CrossTabGridCellFormat",
    fields: &[
        Field::new("flags", Kind::I32Be),
        Field::new("_u0", Kind::VarU16),
        Field::new("background_color", Kind::U32Be),
        Field::new("enabled", Kind::I16Be),
    ],
};
