# Block catalog

This is the reference for the record (block) types `rpt-rs` decodes: what each one is, what it means, and how its bytes
are laid out. It assumes you have read [The record tree](04-record-tree.md) for how records nest and mask.

## Conventions

- **Header.** Every record header is 8 bytes: a flag byte (`0xF8`/`0xF9`), the type, a `0x07` subtype high byte, and a
  4-byte big-endian length. Headers are read at the parent's content mask.
- **Content mask.** A record's content is read under the XOR of the low bytes of the record types on the parse stack
  (see [The record tree](04-record-tree.md)).
- **lp-string.** A length-prefixed string: a 4-byte big-endian length, then the bytes (NUL-terminated).
- **Twips.** Geometry unit: 1/1440 inch.
- **Endianness.** Mixed; framing tends big-endian, value codes/flags little-endian. See
  [Endianness](appendix-endianness.md).
- Byte offsets below are offsets into a record's (un-masked) content leaf.
- Examples use generic placeholders: `{Table.Field}`, `@Formula`, `?Parameter`.

Two record types are **overloaded** by stream: type `0x03` is a printer-info record in the `Contents` stream but a table
record in the `QESession` stream. The decoder resolves them by context.

## Stream and report structure

### `0xFFFF` — StreamHeader

The first record of every report stream, stored in plaintext. Its body carries `isEncrypted`, `version`, `useFixedKey`,
the 16-byte decryption `IV`, and a trailer. See [Stream decoding](03-stream-decoding.md).

### `0x0064` — ReportRoot

The report root record; appears once, first, inside a `Contents` stream. Carries report-level metadata and option flags.
The report name is an lp-string whose length is a big-endian `u32` at offset 7. Byte 24 holds option bits (bit 0 = save
data with the report). The "save preview picture" flag is a single byte stored in the record's trailer, immediately
before a fixed marker sequence (`10 01 00 00 00`); the marker's position floats, so it is located by scanning for the
marker.

## Data source (the `QESession` stream and field definitions)

### `0x02` — QE_CONNECTION

A data-source connection container: the database driver (DLL), the connection type, and the database name. Logon
properties are stored as keyed strings; the database name appears under keys such as `Database` / `Initial Catalog`, and
the server under the server property (not the full connection string). Passwords are never surfaced. A connection owns
the table records that follow it.

### `0x03` — QE_TABLE (in `QESession`)

A table: its name and optional alias, the SQL command text (for command-based tables), and its fields. Layout is
positional: `[name][name][alias][sql]`, with each string an lp-string whose length (including the NUL) is a 4-byte
big-endian value.

### `0x04` — QE_FIELD

A table data field: its name, a value-type code, and a length. The value-type code is a little-endian `u16`; the length
is the field's byte width.

### `0x0a` — QE_TABLE_LINK

A table-to-table join: the source and destination field identifiers and the join type. One record is emitted per linked
field pair; pairs that share the same table pair and join are folded into a single logical link.

### `0x0073` — FieldDef

A referenced database field definition. The library stores only the fields the report actually references, not the full
table schema. Layout: an lp-string `name`, then `value_type` (a little-endian `u16`), then the field byte `length` (a
big-endian `u16`). Recognized value-type codes map to a typed enum (e.g. integer, date, string); unrecognized codes are
preserved as-is.

### `0x0071` — NamedValue

A length-prefixed named value that immediately follows a formula body to name it. Also used to carry a formula's stored
result width (a big-endian `i32`).

## Printing and page setup

### `0x03` — PrinterInfo (in `Contents`)

Printer information: the print driver, printer name, and port (e.g. driver/`winspool`/port strings).

### `0x0007` — PaperSize / DEVMODE

Page-setup information derived from a Windows `DEVMODE` structure: orientation, paper-size code, and paper source. These
fields follow Windows conventions and are little-endian.

### `0x0066` — PageSetup

Page setup: the four page margins, each a big-endian `u32` in twips.

### `0x018e` — PaperRect

The page rectangle: paper width and height, each a big-endian `u32` in twips.

## Data definition (formulas, parameters, groups, sorting, summaries)

### `0x0076` — Formula

A formula field's body: the referenced fields plus the formula text (for example `{Table.Field}` references and
expressions). The following `0x0071` NamedValue names the formula.

### `0x007a` — ParamRecord

A parameter field's detail record. Its content is obfuscated with an additional XOR by `0x7A`. It carries the
parameter's prompt text (UTF-8), its type (anchored after a `0xFF` block), value lists (default values), and a global
parameter index used to join current values from the report's parameter stream. Number/currency values are stored as a
big-endian `f64` divided by 100; dates as a big-endian Julian day number; strings verbatim.

### `0x00e5` — Group

A report group: its grouping condition field and order. Carries the group's keep-together / repeat-header /
visible-per-page options and, for date groups, a granularity token.

### `0x0029` — RecordSortField

A record-level sort: a field reference plus the sort direction (in the last byte).

### `0x007e` — SummaryDef

A summary or running-total definition: an operation byte (sum, count, average, …) and the summarized field. A standalone
run of these defines the report's summary fields.

### `0x0080` — RunningTotalReset

A running total's reset condition. It immediately precedes the `0x007e` it applies to.

## Layout: areas, sections, and objects

The page layout is a flat, ordered run of records: an area marker, then its sections, then the objects inside each
section. Order is significant — objects belong to the most recent area/section.

### `0x008a` — Area

An area marker, named by role and index (e.g. `DetailArea1`, `PageHeaderArea1`). Areas are delimited in document order;
the sections and objects that follow belong to the current area.

### `0x008c` — Section

A section within an area: its height (a big-endian `u32` in twips) and name (e.g. `ReportHeaderSection1`).

### `0x009e` — ObjectName

An object's name plus its width and height. Attaches to the object record it follows.

### `0x00be` — ObjectPosition

An object's position: left and top, in twips (`u16`).

### `0x009f` — FieldObject

Opens a field object — a placed field bound to a data source. Its leaf carries the data-source reference (e.g.
`{Table.Field}`).

### `0x00a5` — TextObject

Opens a text object. Byte 15 set to 1 marks the object as a _field heading_ (a label attached to a field).

### `0x00c2` — TextContent

A text object's literal text content.

### `0x00c4` — TextEmbeddedField

An embedded field, formula, or parameter reference inside a text object's flowing text.

### `0x00a9` — LineObject / Box

Opens a line or box drawing object; geometry distinguishes the two. Coordinates use a variable-width encoding
(`read_coord`): 2 bytes normally, 4 bytes when the value exceeds `0x7FFF`. A byte flags "extend to bottom of section". A
related border record (`0x00ec`) classifies the shape (box vs. line) and supplies styling.

### `0x00ae` — PictureObject

Opens a picture or OLE object. A bare `0x00ae` is a static picture or chart; when wrapped by a `0x00b1` record (whose
leaf names a database field), it is a blob/image field bound to that field.

### `0x00a3` — SubreportObject

Opens a subreport placeholder object. A big-endian `u32` at offset 0 is the subdocument index — the `Subdocument N`
storage that holds the subreport's streams.

### `0x0166` — FieldHeadingLink

Names the field object that a text object is the heading for.

### `0x0106` — SubReportLink

A subreport link: how a value passes from the main report into a subreport. The leading `u16` is the subreport parameter
index (the pairing key). The main-report field name is stored as a string; the subreport field is stored as a
`(kind, index)` handle in the trailing descriptor (`kind` 0 = the Nth database field, `kind` 1 = the Nth formula),
resolved against the subreport's per-kind field pool.

## Formatting

Most format records attach to the object or section that precedes them. Conditional-format records hold an array of
formula slots: a property is either a fixed value or driven by a formula.

### `0x0008` — Font

An object's font: name, size, weight, and style. Size in twips is a big-endian `u32` at offset 13; the weight is a
big-endian `u16` at offset 11 (`0x0190` = 400 normal, `0x02BC` = 700 bold); italic and underline are flag bytes. A
multi-run text object uses the first run's font.

### `0x0100` — FontColor

An object's font color, as a `COLORREF` (`0x00BBGGRR`).

### `0x00ec` — ObjectBorder

An object's border styles and its border and background colors. Byte 25 is the shape type for box objects (1 = box, 2 =
line). Byte 9 flags a drop shadow.

### `0x00fc` — ObjectFormat

An object's format flags, including horizontal alignment (in byte 2).

### `0x00fd` — ObjectConditionFormulas

An object's conditional-format formula slot array (the formulas driving its conditioned properties, such as suppression
and display string).

### `0x00c0` — TextObjectFormat

A text or heading object's paragraph format, including alignment (in byte 12).

### `0x00fe` — AreaSectionFormat

An area's or section's format flags — a 52-byte block of options (suppress, keep-together, new-page-before/after, and
similar).

### `0x00ff` — SectionConditionFormulas

A section's conditional-format formula slot array.

### `0x0101` — FontConditionFormulas

An object's font conditional-format formula slot array.

## Recognized but not decoded

Many record types appear in real reports but are not yet interpreted into the model. They are preserved verbatim in the
[record substrate](04-record-tree.md) as `Unknown` nodes (so the round-trip stays lossless) and are still counted in the
report's record inventory. They include chart and cross-tab data blocks, OLAP grids, and a long tail of less common
types. The [support matrix](09-support-matrix.md) tracks which are decoded.

## Feature areas not covered

Beyond individual record types, whole Crystal feature areas are not modelled because they introduce their own families
of records: deep chart/graph models (axes, series, styling — only the placeholder object is modelled), cross-tabs and
OLAP grids, maps, alerts, hierarchical grouping, typed field sub-formats (number/date/currency masks and rounding), and
the full range of conditional formatting (rotation, tooltips, hyperlinks). See the
[support matrix](09-support-matrix.md).
