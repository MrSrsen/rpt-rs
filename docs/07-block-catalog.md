# Block catalog

This is the reference for the record (block) types `rpt-rs` decodes: what each one is, what it means, and how its bytes
are laid out. It assumes you have read [The record tree](04-record-tree.md) for how records nest and mask.

## Conventions

- **Header.** A nested `Contents` record header is a flag byte (`0xF8` = type packed inline, `0xF9` = an extended
  little-endian type word follows), the type, a subtype word with a `0x07` high byte, and a 4-byte big-endian length —
  8 bytes with an inline type, 10 with an extended one. Headers are read at the parent's content mask.
- **Content mask.** A record's content is read under the XOR of the low bytes of the record types on the parse stack
  (see [The record tree](04-record-tree.md)).
- **lp-string.** A length-prefixed string: a 4-byte big-endian length, then the bytes (NUL-terminated).
- **Twips.** Geometry unit: 1/1440 inch.
- **Endianness.** Mixed; framing tends big-endian, value codes/flags little-endian. See
  [Endianness](09-endianness.md).
- Byte offsets below are offsets into a record's (un-masked) content leaf.
- Examples use generic placeholders: `{Table.Field}`, `@Formula`, `?Parameter`.

Two record types are **overloaded** by stream, and the decoder resolves them by context:

- `0x03` — printer info in `Contents`, a table in `QESession`.
- `0x07` — the page-setup `DEVMODE` in `Contents`, a command/stored-procedure bind parameter in `QESession`, and the
  stored-field container in `DataSourceManager` (see [Saved data](06-saved-data.md)).

## Stream and report structure

### `0xFFFF` — StreamHeader

The first record of every report stream, stored in plaintext. Its body carries `isEncrypted`, `version`, `useFixedKey` (inert — see `docs/03`),
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

A table-to-table join: the source and destination field identifiers and the join predicate. One record is emitted per
linked field pair; pairs that share the same table pair and predicate are folded into a single logical link. The leaf is
six big-endian `u32`: `[link_id, src_field, dst_field, operator, join_kind, word5]`. `operator` and `join_kind` are
independent one-hot bit codes — `operator` `0x04` `=`, `0x08` `<>`, `0x10` `<`, `0x20` `<=`, `0x40` `>`, `0x80` `>=`;
`join_kind` `0x1` inner, `0x2` left outer, `0x4` right outer, `0x8` full outer. `word5` (always 1) stays unmapped.

### `0x0073` — FieldDef

A referenced database field definition. The library stores only the fields the report actually references, not the full
table schema. Layout: an lp-string `name`, then `value_type` (a little-endian `u16`), then the field byte `length` (a
big-endian `u16`). Recognized value-type codes map to a typed enum (e.g. integer, date, string); unrecognized codes are
preserved as-is.

### `0x0071` — NamedValue

A length-prefixed named value that immediately follows a formula body to name it. Also used to carry a formula's stored
result width (a big-endian `i32`).

### `0x0081` — FieldDefinition3 (SQL Expression field)

A **SQL Expression** field: an lp-string of SQL text pushed down to the server, with a `0x0071` NamedValue child giving
the expression's name, value type, and length. Listed by `rpt sql` alongside the generated query and the stored SQL
Commands.

### `0x0160` — FieldDefinitionsHeader

Report-level data-source options: the `ConvertNullFieldToDefault` / `ConvertOtherNullsToDefault` flags (`ReportOptions`).

## Printing and page setup

### `0x03` — PrinterInfo (in `Contents`)

Printer information: the print driver, printer name, and port (e.g. driver/`winspool`/port strings).

### `0x0007` — PaperSize / DEVMODE (in `Contents`)

Page-setup information from a Crystal-compacted Windows `DEVMODEW` structure: orientation, paper-size code, paper
source, and duplex. The leaf is an 8-byte header
`[subtype u16][dmFields low word u16][dmOrientation u16][dmPaperSize u16]`
followed by one big-endian `u16` per further **set** `dmFields` bit, in DEVMODE member order (PaperLength, PaperWidth,
Scale, Copies, DefaultSource, PrintQuality, Color, Duplex, YResolution, TTOption, Collate). Orientation and paper-size
bits are always set, so those two occupy the fixed header. (Scalars here are big-endian, unlike a raw Win32 DEVMODE.)

### `0x0066` — PageSetup

Page setup: the four page margins, each a big-endian `u32` in twips.

### `0x018e` — PaperRect

The page rectangle: paper width and height, each a big-endian `u32` in twips.

### `0x006c` — MultiColumnFormat

The "Format with Multiple Columns" detail layout, a report-level singleton. Fields are big-endian `u16` twips —
leftMargin, topMargin, **labelWidth** (offset `0x0c`), labelHeight, horizontalGap (`0x10`), verticalGap (`0x12`) — then a
`u32` downThenAcross boolean at `0x14`. The engine stores **no** column count: it fits as many label-width columns as
span the printable width, so the count is derived as `content_width / (labelWidth + horizontalGap)`. `labelWidth` is `0`
unless multi-column is enabled, which is also how the reader detects that it is on.

## Data definition (formulas, parameters, groups, sorting, summaries)

### `0x0076` — Formula

A formula field's body: the referenced fields plus the formula text (for example `{Table.Field}` references and
expressions). The following `0x0071` NamedValue names the formula.

A report-level **custom function** is stored in this same record shape, distinguished by a `0xFF80` sentinel in the
`u16`
value-type field after the name in the `0x0071` child (its body opens `Function (args) …`). Such records are modelled
separately as custom functions rather than formula fields.

### `0x007a` — ParamRecord

A parameter field's detail record. Its content is obfuscated with an additional XOR by `0x7A`. It carries the
parameter's prompt text (UTF-8), its type (anchored after a `0xFF` block), value lists (default values), and a global
parameter index used to join current values from the report's parameter stream. Number/currency values are stored as a
big-endian `f64` divided by 100; dates as a big-endian Julian day number; strings verbatim.

### `0x00e5` — Group

A report group: its grouping condition field and order. Carries the group's keep-together / repeat-header /
visible-per-page options and, for date groups, a granularity token.

### `0x0088` — GroupAreaFormat

A 24-byte area-pair options record that immediately *precedes* the `0x00e5` Group it describes: repeat-group-header and
keep-group-together flags plus the visible-groups-per-page count. The outermost group has no preceding `0x0088`
and keeps defaults.

### `0x0029` — RecordSortField

A record-level sort: a field reference plus the sort direction (in the last byte).

### `0x007e` — SummaryDef

A summary or running-total definition: an operation byte (sum, count, average, …) and the summarized field. A standalone
run of these defines the report's summary fields. Leaf: `byte 0` = operation, `byte 1` = a constant-0 separator,
`bytes 2..4` = OperationParameter (a big-endian `u16`, the N of an Nth-largest / percentile op), `byte 4` = the primary
summarized field (lp-string) followed by an optional secondary summarized field (lp-string, empty for a single-field
op). An IsPercentageSummary flag sits at `4 + primary-field-consumed + 12`. OperationParameter and the secondary field
are 0 / empty across the corpus.

### `0x0080` — RunningTotalReset

A running total's reset condition. It immediately precedes the `0x007e` it applies to.

### `0x00e9` — HierGroup

A specified-order (hierarchical) group value: an lp-string group-value name followed by an lp-string defining
condition-formula. One record per named value.

### `0x0118` — FormulaVariable

One persisted Global/Shared formula variable: its name, result type, and scope. The preceding `0x0116` table header just
holds the variable count and is not parsed.

### `0x006e` — FieldManagerEntry

The field-pool census: a 20-byte block counting the report's database fields, formulas, and the other field-manager
pools. A single record, parented by the `0x006f` FieldManager collection; drives the field-kind partitioning used
elsewhere (notably resolving a subreport link's `(kind, index)` handle).

## Layout: areas, sections, and objects

The page layout is a flat, ordered run of records: an area marker, then its sections, then the objects inside each
section. Order is significant — objects belong to the most recent area/section.

### `0x008a` — Area

An area marker, named by role and index (e.g. `DetailArea1`, `PageHeaderArea1`). Areas are delimited in document order;
the sections and objects that follow belong to the current area.

### `0x008d`–`0x0099` — Band markers

The record that brackets each area declares the area's **kind**, one code per band: `0x8d` report header, `0x8f` report
footer, `0x91` page header, `0x93` page footer, `0x95` detail, `0x97` group header, `0x99` group footer (the odd
in-between codes close them). The kind comes from this marker, *not* from the area's name — a group area a report tool
named after its group field still lays out as a group band. Codes `0x9b`–`0x9d` carry the section/area-type codes.

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

### `0x00bd` — OleObjectItem

Decorates a static picture / OLE object; its leaf `[0..4]` is a big-endian 1-based ordinal naming the `Embedding N`
storage that holds the object's payload.

### `0x00b4` — ChartBinding

Opens a chart's binding block and nests the chart's `0x009e` ObjectName. The chart's "show value" data field is carried
by a `0x007f` ChartData wrapper (around a `0x007e` child); the `0x011c` ChartAnalyticHeader opens the analytic block
(leaf byte 2 = the layout type: `0` Detail / `1` Group / `2` CrossTab) and labeled analytic values arrive as `0x011f`
ChartDataValue records. The `0x0121` ChartDefinition2 leaf carries the v2 chart type, the titles, and — for the 3-D
riser/surface types only — the viewing angle; the rest of its styling blob (axes, series, colours) is named and left
opaque.

### `0x00b8` — CrossTabObject

Opens a cross-tab object, wrapped by a `0x00b9` record that starts the cross-tab binding block and parents the object's
`0x009e` name. The grid's dimensions and formats follow:

- `0x00cb` — a dimension level: a header plus an lp-string `{table.field}` reference.
- `0x00ce` — a column-axis level (`Column #N`); `0x00d2` — a row-axis level (`Row #N`).
- `0x0143` — a grid-level format word (a big-endian `u16`), which opens the cell-format run.
- `0x0145` — one grid-region cell format (an 11-byte block: flags + a BGR background colour + a flag).

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

An object's format flags, including horizontal alignment (in byte 2). The object's hyperlink target text and type are
also carried in this leaf (a lone-NUL target means no hyperlink).

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

### `0x00ed` — ObjectBorderCondition

The conditional-format wrapper that parents a `0x00ec` ObjectBorder; it carries the border-colour condition-formula
slots.

### `0x00ee`–`0x00fb` — Typed field sub-formats

A field object's typed display format streams after its `0x009f` opener as a fixed run of wrapper/value record pairs —
each odd wrapper carries conditioned-value slots and parents its even value child: Common (`0x00f0`/`0x00f1`), Numeric
(`0x00f8`/`0x00f9`), Boolean (`0x00ee`/`0x00ef`), Date (`0x00f2`/`0x00f3`), Time (`0x00f6`/`0x00f7`), DateTime
(`0x00f4`/`0x00f5`), and String (`0x00fa`/`0x00fb`). The Common/Numeric/Boolean/String value bytes are decoded into the
model, as are the stored ordering choices of the date/time leaves (Date's DateOrder, DateTime's DateTimeOrder, Time's
hour/minute/second formats); the remaining date/time display attributes resolve at runtime from the value type and host
locale, so — like a formula's `NumberOfBytes` — they are not stored facts and are left to the consumer that needs them.

Two notes on the layout. **Numeric** emits *two* consecutive `0x00f9`/`0x00f8` pairs — a currency-format slot then a
number-format slot; the engine reports the currency slot for a Currency-valued field and the number slot otherwise. Its
14-byte header carries NegativeFormat (`leaf[2]`), DecimalPlaces (`u16` at `[7..9]`), CurrencySymbolFormat (`leaf[10]`),
and **CurrencyPosition** (`leaf[13]`: leading/trailing × inside/outside the negative sign), then the thousands, decimal,
and currency-symbol strings. **String** (`0x00fa`) carries three big-endian `u32` indent longs (first-line / left /
right, in twips) at `bytes 1..12`, plus word-wrap, text-format (Standard / HTML), and reading-order bytes.

## Authoring, saved data, and designer state

These records carry provenance and editor state.

### `0x0061` — SavedData

The saved-data block descriptor. Its presence marks `ReportDocument.HasSavedData`; the cached rows themselves live in
separate streams (see [Saved data](06-saved-data.md)).

### `0x0031` — CurrentValueRecord (in `ReportParametersStream`)

One parameter's saved **current value**: a global parameter index plus per-type value entries, joined against the
`0x007a` ParamRecord definitions by that index. `ReportParametersStream` is a TSLV record stream decoded through the same
pipeline as `Contents`.

### `0x0178` — SaveMetadata

One save-time environment key/value pair, one record per save event, kept in stream order.

### `0x0142` — ReimportInfo

A subreport re-import descriptor: the source path the report/subreport was imported from plus its import timestamps.

### `0x010c` — GuidelineEntry

A designer snap guideline: a big-endian `u32` position in twips followed by a `u16` flags word.

### `0x0111` — ObjectConnection

A designer object-connection edge (a 22-byte block: source, destination, and kind).

## Record-type coverage

Every record type the library has encountered is **identified and named** in the registry (`RecordTag::name()`), so
`rpt tree` renders each node with its type name rather than raw hex (`rpt streams`, being a coverage meter, still lists
its per-stream "top types" by code). Naming reaches down to codes `0x017e`/`0x017f`
(`CrossTabCustomMembersBegin`/`CrossTabCustomMembersEnd`, a bracket pair around a cross-tab's custom-group-members
collection; the per-member detail records `0x0180`/`0x0181` are absent from the corpus). A record type the registry has
not seen is preserved verbatim as an `Unknown` node so the round-trip stays lossless. The
[support matrix](08-support-matrix.md) tracks which types are fully **decoded into the model** vs **named for
recognition** (structural bracket/wrapper records and opaque render state carry nothing a reader needs), and reports the
current corpus-wide Unknown count.

## Feature areas: modelling depth

A record type being *named* does not mean its whole feature is *modelled*. Deep chart/graph styling (axes, series,
colours — the chart object, analytic layout, and data-value labels are decoded, but the `0x0121` styling blob is named
and left opaque), full OLAP-grid / map / alert / Flash / XML-export structure (named at the family level but not decoded
into the model), and the effective *runtime* display format of typed field sub-formats (the stored format is decoded;
the locale-resolved display value is not a stored fact, like a formula's `NumberOfBytes`) remain partial. Not all object-level
sub-format condition formulas are decoded yet. See the [support matrix](08-support-matrix.md).

## See it yourself

The `rpt dump` command is the byte-layout workbench for a record type — an annotated hex dump of the demasked leaf bytes
a decoder reads:

```console
$ rpt dump report.rpt --type 0x0064
```

---

← [Saved data](06-saved-data.md) · [Index](README.md) · **Next:** [Support matrix](08-support-matrix.md) →
