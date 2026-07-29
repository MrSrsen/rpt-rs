# Block catalog

This is the reference for the record (block) types `rpt-rs` decodes: what each one is, what it means, and how its bytes
are laid out. It assumes you have read [The record tree](04-record-tree.md) for how records nest and mask.

## Conventions

- **Header.** A nested `Contents` record header is a flag byte (`0xF8` for record types `0x0000`–`0x00FF`, `0xF9` for
  `0x0100`–`0x01FF` — the type's high byte rides in the flag byte's low bits), the type's low byte, a big-endian schema
  word (the record type's version — one opaque number, not a dialect byte plus a version byte), and a 4-byte big-endian
  length — 8 bytes in all. An empty record omits the length field and is 4 bytes. Headers are read at the parent's
  content mask.
- **Content mask.** A record's content is read under the XOR of the low bytes of the record types on the parse stack
  (see [The record tree](04-record-tree.md)).
- **lp-string.** A length-prefixed string: a 4-byte big-endian byte count (the NUL included), then that many bytes; a
  count of `0` is the null string. This is the *enhanced* of the format's two string wire forms, and the one every
  engine-written record declares — see
  [strings come in two wire forms](04-record-tree.md#strings-come-in-two-wire-forms).
- **Twips.** Geometry unit: 1/1440 inch.
- **Endianness.** Mixed, and a per-field fact rather than a per-layer one; framing is big-endian. See
  [Endianness](07-endianness.md).
- Byte offsets below are offsets into a record's own (un-masked) field bytes — its **runs**, joined in order with any
  nested records spliced out, which is what `rpt dump` prints.
- Examples use generic placeholders: `{Table.Field}`, `@Formula`, `?Parameter`.

A record type number is **per stream**, so several numbers name unrelated records depending on where they occur, and the
decoder resolves them by the stream's own vocabulary (`RecordTag::name(dialect)`). `rpt dump --type` takes a name from
any vocabulary and reduces it to a type *number*; `--stream` then decides which record you get:

- `0x03` — printer info in `Contents`, a table in `QESession`.
- `0x07` — the page-setup `DEVMODE` in `Contents`, a command/stored-procedure bind parameter in `QESession`, and the
  stored-field container in `DataSourceManager` (see [Saved data](05-saved-data.md)).
- `0x08` — an object's font in `Contents`, a table index in `QESession`.
- `0x09` — the report's document bounds in `Contents`, a connection's logon property in `QESession`.

## Stream and report structure

### `0xFFFF` — StreamHeader

The first record of every report stream, stored in plaintext. Its body carries `isEncrypted`, `version`, `useFixedKey`
(inert), the 16-byte decryption `IV`, and a trailer. See [Stream decoding](03-stream-decoding.md).

### `0x0064` — ReportRoot

The report root record; appears once, first, inside a `Contents` stream. Carries report-level metadata and option flags,
as a sequence rather than at fixed offsets: the authoring version (major, minor, a letter), the document name as an
lp-string, a save timestamp as a Julian day and a second of the day, an option word (bit 0 = save data with the report),
a saved-data flag, and the nested `0x0000` document record. A main report leaves the document name empty and a subreport
carries its own name there, so every field after the name moves by the name's length.

Past the document record the record is a run of trailing fields, each present only while the record still has content:
the saved-data handle (when the saved-data flag is set), two GUIDs, a list of the streams stored beside the report, the
"save preview picture" / "verify on every print" word, a saved-data version, the time zone the report was saved in, and
the locale.

## Data source (the `QESession` stream and field definitions)

### `0x02` — QE_CONNECTION

A data-source connection container: a connection id, the driver DLL, the database-type display name, and the **server**
description; then counted runs of nested logon-property (`0x09`) and table (`0x03`) children. The database name is not
one of the record's own strings — it appears among the logon properties, under keys such as `Database` /
`Initial Catalog`. Passwords are never surfaced.

### `0x03` — QE_TABLE (in `QESession`)

A table the report reads: `table_id` (a big-endian `u32` — the key a table link resolves against), the table name, a
description, the qualified name and a **counted** run of qualifier parts (the catalog and schema a provider prefixes it
with), a table type, the alias, the `is_flat` and `is_linkable` flags, then three counted runs of nested children —
`0x04` fields, `0x07` bind parameters and `0x08` indexes. The command text follows as the **first** trailing value, so
it needs no "longest string wins" search; after it come the external-index list, an overridden qualified name, a file
name, a binary blob, an id and a property count. Every child run states its count ahead of the run, so the record says
how many follow rather than leaving them to be discovered.

### `0x04` — QE_FIELD

One column of a table: `field_id` (big-endian `u32` — what a table link resolves against), the name, a description, the
`value_type` (a big-endian `u32`, **not** a little-endian `u16` like the `Contents` field definitions), and the stored
byte `length`. A `length` of `0xffffffff`, `0x7fffffff` or `0x7ffffffe` stands for an unlimited column, which the model
reports as the `-1` the record stores rather than as a fabricated width. Five more fields — the attribute mask, the
precision, the provider's own identifier string, a can-be-processed-on-server flag and the field lineage — were each
added at a later schema version and are read only from that version on.

### `0x0008` — QE_INDEX (in `QESession`)

One index the provider reports on a table, nested in the `0x0003` table's counted index run: an index id, the name,
whether it is the primary key, whether its values are unique, then a counted run of the field ids the index covers. The
record type has never been revised, so its header states **no schema word** and it takes its stream's default version —
the short header form (see [The record tree](04-record-tree.md#the-header-is-variable-width)). Nothing about an index
reaches the model.

### `0x0a` — QE_TABLE_LINK

A table-to-table join: the source and destination field identifiers and the join predicate. One record is emitted per
linked field pair; pairs that share the same table pair and predicate are folded into one logical link where they are
consecutive in `link_id` order. The record's own bytes are five big-endian `u32` — `link_id`, `source_field_id`,
`target_field_id`, `operator`, `join_kind` — plus a sixth, `table_join_enforced`, added at schema `0x0901`. `operator`
and `join_kind` are independent one-hot bit codes — `operator` `0x04` `=`, `0x08` `<>`, `0x10` `<`, `0x20` `<=`, `0x40`
`>`, `0x80` `>=`;
`join_kind` `0x1` inner, `0x2` left outer, `0x4` right outer, `0x8` full outer. `table_join_enforced` is `1` on every
corpus record.

### `0x0073` — FieldDef

A referenced database field definition. The library stores only the fields the report actually references, not the full
table schema. It is a wrapper: a nested `0x0072` (itself wrapping the `0x0071` base that carries the name, value type
and length) comes **first**, then the record's own `field_id` — the big-endian `u32` handle resolving the `QESession`
column it reads. Recognized value-type codes map to a typed enum (e.g. integer, date, string); unrecognized codes are
preserved as-is.

### `0x0071` — NamedValue

The field-definition base every kind of field is built on: a name, the value type it produces, and how long that value
is. It has **one shape, not two** — a definition with no name of its own (a summary) stores an *empty* string rather
than omitting the field, so its type byte sits where a named definition's does.

The length is stored **twice**. The narrow form is a big-endian `u16` counting *characters* for a string-typed value and
bytes for everything else, and it saturates at 255; a second, signed big-endian `i32` past the record's second string is
the byte count outright and supersedes it. Only the wide form is a byte count for every type, so it is the one a
consumer wants — and it is signed, so a value of no fixed width (a blob) stores `-1`.

### `0x0072` — NamedValueWrapper

A wrapper carrying one `0x0071` and nothing of its own — the form a database-field definition takes. Every
field-definition wrapper (`0x0072`, `0x0073`, `0x0077`, `0x0078`, `0x0079`, `0x007f`) writes the definition it wraps
**first**, so the wrapper's own fields begin where its child ends.

### `0x0081` — SqlExpressionField

A **SQL Expression** field: SQL text pushed down to the server. Its `0x0071` NamedValue comes **first**, giving the
expression's name, value type and length; the text follows it, then a word, then the fields the text names — a count and
one field reference each. The text is a stored value even when empty, so an unbound expression still carries its
framing. Listed by `rpt sql` alongside the generated query and the stored SQL Commands.

### `0x0160` — ReportOptions

The document's report-wide option bag, one per `Contents` stream — so one per report and one per subreport. It sits in
the report-definition tail beside the paper rectangle and the saved-data selection formula, nests no record and stores
no string: a word, a narrowing enum, then a run of option **words**, each a whole `i16` rather than a bit, a boolean one
carrying its truth in the word's low half. Two of them are `ConvertNullFieldToDefault` and `ConvertOtherNullsToDefault`
(`ReportOptions`); the rest are named by position alone.

One value is written into **two consecutive words** rather than one, so that pair always carries the same number.
Everything from `ConvertOtherNullsToDefault` on is written one word at a time and only read while the record still has
content, so a record can stop at any of its trailing fields. The corpus carries two lengths — 43 bytes and 45 — with a
little over a quarter of records stopping before the last word.

## Printing and page setup

### `0x03` — PrinterInfo (in `Contents`)

Printer information: a word, then the print driver (`winspool`), the saved device name, and the port (`Ne00:`). A report
saved with no printer writes an unrelated record under the same type number at a different schema; the layout above is
the one with a printer.

### `0x0007` — PaperSize / DEVMODE (in `Contents`)

Page-setup information from a Crystal-compacted Windows `DEVMODEW` structure. The record opens with the **whole 32-bit
`dmFields` mask** as a big-endian `u32` at `[0..4]` — not a sub-type plus the mask's low half — followed by the
orientation and paper size as fixed slots (`DM_ORIENTATION` and `DM_PAPERSIZE` are set on every corpus record, so a gate
could not be told from a slot), then one big-endian `u16` per **set** bit for the rest, in `DEVMODE` member order
(PaperLength, PaperWidth, Scale, Copies, DefaultSource, PrintQuality, Color, Duplex, YResolution, TTOption, Collate).
Because the mask is read whole, the saved form name that `DM_FORMNAME` (`0x00010000`) selects is decoded too, as a
trailing lp-string. Scalars here are big-endian, unlike a raw Win32 `DEVMODE`.

### `0x0066` — PageSetup

Page setup: three narrowing enums (1 byte each, or 2 with the `0x80` escape), then the four page margins as big-endian
`i32` twips — left, right, top, bottom. The first enum is the report-level `ReportKind` — Columnar, Label, or
MultiColumn; the third is the report's canned formatting **style**. Because all three are narrowing, the style is the
third *field*, never a fixed byte offset. The second is not identified. A margin stored as `i32::MIN` is the engine's
"use the default" sentinel rather than a distance.

### `0x018e` — PaperRect

The page rectangle: paper width and height, each a big-endian `i32` in twips, then four **field references** (a name
plus its `(pool, index)` handle) — stored empty in every report seen, but variable-length by construction, so they are
read rather than skipped over as a fixed run.

### `0x006c` — MultiColumnFormat

The "Format with Multiple Columns" detail layout, a report-level singleton. Two lp-strings, then four big-endian
`i32` dimensions in the designer's own order — column width, detail height, horizontal gap, vertical gap — then a
narrowing flow-direction enum and a trailing word. There are no margins in this record.

The flow enum is **not** a plain "down then across" boolean: a report with multi-column *disabled* stores `1`, so
reading it as the flow reports every ordinary report as down-then-across. The engine also stores **no** column count: it
fits as many column-width columns as span the printable width, so the count is derived as
`(content_width + horizontal_gap) / (column_width + horizontal_gap)`. `column_width` is `0` unless multi-column is
enabled, which is how the reader detects that it is on.

## Data definition (formulas, parameters, groups, sorting, summaries)

### `0x0076` — Formula

A formula field's body. Its `0x0071` NamedValue is nested **first**, at content offset zero, not a sibling that follows
it. After it: a reference count and one field reference per field the body names, the body text, which kind of formula
it is and which format property it conditions, seven words, then three trailing fields a record need not carry.

A report-level **custom function** is stored in this same record shape, told apart by its body, which opens with the
reserved `Function (…)` header a report formula body never can. Its `0x0071` does carry a distinct value type, but the
body is the discriminator the reader uses. Such records are modelled separately as custom functions rather than formula
fields.

### `0x007a` — ParameterRecord

A parameter field's detail record, opened by the `0x0071` NamedValue naming it. Its content carries no obfuscation of
its own: like every record it is read under the stack mask of §[03](03-stream-decoding.md), which for a top-level record
of this type is `0x7A` because that is its own type number.

After the definition come the global parameter index — the key the report's parameter stream joins its current values
by — the prompt text, the field the parameter is bound to, and the value list: the value type once, a count, and one
entry per value. An entry states a byte count and then a payload of the width its type has, so a count of zero is the
null value; the count is a word up to schema `0x0700` and a long from `0x0701`. Number and currency values are a
big-endian `f64` scaled by 100, dates a big-endian Julian day number, strings verbatim.

The bounds follow, per value family rather than per parameter: a flag, two doubles, then a long each for a date and a
time bound and two for a date-and-time one. An unset numeric bound is `±f32::MAX` — an empty interval — and the rest are
`-1`. Everything after that is written only while the record has content left: the flags governing what the prompt
accepts, the parameter's name, how the default list is displayed and sorted, one description per stored value, the
`crobj://{…}` identity that joins the record to its `PromptManager` entry, and the optional-prompt flag that closes it.

### `0x00e5` — Group

A report group: the condition field (its name plus a `(pool, index)` handle), the grouping condition ordinal — for a
date group, its granularity — and the sort direction. Then the Top N / Bottom N settings, the group-name and
group-name-formula field references, the hierarchy fields, and the specified-order pair array. The keep-together,
repeat-header and visible-per-page options are **not** here: they live on the `0x0088` GroupAreaFormat below.

Two consecutive enums follow the condition ordinal: the **first** is the sort direction, and the second is one the
engine loads and immediately throws away, so a reader that takes the second reads a value nothing uses.

### `0x0088` — GroupAreaFormat

The area-pair options record that immediately *precedes* the `0x00e5` Group it describes — including the outermost
group, whose `0x0088` sits before the first `0x00e5`. Layout: the repeat-group-header and keep-group-together flags, the
group indent (a whole `i32` the engine clamps to zero when negative, not the `u16` its low half looks like), a nested
`0x0151` record, the visible-groups-per-page count (also a whole `i32`), and the formula that can override the
new-page-after behaviour, with its `(pool, index)` handle. The trailing lp-string makes the record variable-length.

### `0x0029` — RecordSortField

A record-level sort: the field name, its `(pool, index)` handle — `pool` `0` a database field, `1` a formula, `2` a
summary, which is what distinguishes a plain field sort from a group summary sort — and then the sort direction.

### `0x007e` — SummaryFieldDefinition

A summary or running-total definition; a standalone run of these defines the report's summary fields. A nested
`0x0071` NamedValue carrying the result type comes **first**, ahead of the record's own fields. Those are: the operation
(sum, count, average, …), a narrowing enum the engine reads and discards, the OperationParameter (the N of an
Nth-largest / percentile op), then two **field references** — the summarized field and a second operand, each a name
plus its `(pool, index)` handle. The second is stored empty unless the definition was built through the controller API,
but it is variable-length either way, so everything after it sits at an offset both names' lengths decide.

The tail is guarded rather than fixed: the percentage base group is read **only when the IsPercentageSummary flag is
set**. Read unconditionally it lands on the two trailing enums instead — which is a number, just not that one.
OperationParameter and the secondary field are 0 / empty across the corpus.

### `0x0080` — RunningTotalField

A running total. It **contains** the `0x007e` that carries its operation and summarized field — the summary definition
is the record's first content, exactly as a formula's `0x0071` is — and adds the two conditions that drive it: when the
accumulator resets, and when a record is included.

Each condition is a kind (`0` none, `1` on change of field, `2` on change of group, `3` on a formula) followed by what
that kind names: a field reference for the field and formula kinds, a bare word for the group, nothing for none. So the
evaluate condition sits wherever the reset condition ends, and a trailing word closes the record when it has the bytes
for one. A `0x007e` no `0x0080` contains is a plain summary.

### `0x00e9` — HierarchicalGroupingOptions

A specified-order (hierarchical) group value: an lp-string group-value name followed by an lp-string defining
condition-formula. One record per named value.

### `0x0118` — FormulaVariable

One persisted Global/Shared formula variable: its name, result type, and scope. The preceding `0x0116` table header just
holds the variable count and is not parsed.

### `0x006e` — FieldManagerEntry

The field-pool census: a leading word, then **nine** pool sizes as big-endian `u16`, each of which the engine hands
straight to one field array before reading that pool's records; the ninth is read only if the record has bytes left. The
first two pools are the database fields and the formula bodies (the latter short of the three built-in formulas). The
leading word is *not* part of the first count — it is zero on every record seen, which is the only reason reading the
two together as one `u32` ever gave the right number. A single record, parented by the `0x006f` FieldManager collection;
drives the field-kind partitioning used elsewhere (notably resolving a subreport link's `(pool, index)`
handle).

## Layout: areas, sections, and objects

The page layout is a flat, ordered run of records: an area marker, then its sections, then the objects inside each
section. Order is significant — objects belong to the most recent area/section.

### `0x008a` — Area

An area marker, named by role and index (e.g. `DetailArea1`, `PageHeaderArea1`). It states its own **`section_count`**
as a big-endian `u16` ahead of the name, so the sections that follow are counted rather than found by scanning to the
next area; a `0x008b` end record closes it. The objects inside each section belong to the current area.

### `0x008d`–`0x0099` — Band markers

The record that brackets each area declares the area's **kind**, one code per band: `0x8d` report header, `0x8f` report
footer, `0x91` page header, `0x93` page footer, `0x95` detail, `0x97` group header, `0x99` group footer (the even code
above each closes it). The kind comes from this marker, *not* from the area's name — a group area a report tool named
after its group field still lays out as a group band. A band's whole content is the `0x008c` **Section** it brackets —
nested **first**, at content offset zero, with no field bytes of its own — so the seven are one shape, distinguished
only by their number and by the code that closes each.

Alongside them, `0x9c` wraps a `0x9b` whose content is two fields: the **area type** as a narrowing enum (`01` page,
`02` report, `03` group, `04` detail) and, for a group area, the 0-based group nesting level as a whole big-endian
`u16` after it — the authoritative source of that level, since an area's *name* is renameable and its storage order need
not match the group sequence. `0x9d` is named but not decoded.

### `0x008c` — Section

A section within an area: its height (a big-endian `i32` in twips), its **`object_count`** as a big-endian `u16` — which
the engine sizes its object array from — and its name (e.g. `ReportHeaderSection1`), followed by a marker and a nested
`0x0151` record.

### `0x009e` — ObjectName

An object's size, bounds and name — and it is **nested inside** the object record, coming *first* in that record's
content, ahead of every field of its own. Layout: `width` and `height` as big-endian `i32`, a four-edge `TwipRect` of
narrowing twips (so the rectangle's own width follows its magnitude), the name, a marker, two nested records, a
repository URI, and a trailing block. Because both the rectangle's width and the name's length are variable, everything
after them sits at an offset neither is fixed at.

### `0x00be` — ObjectPosition

An object's position: left and top, each a **narrowing** twip — 2 bytes below `0x8000`, 4 above — so the record is 4 or
8 bytes rather than a fixed 4.

### `0x009f` — FieldObject

Opens a field object — a placed field bound to a data source. Its nested `0x009e` ObjectName comes **first**, ahead of
the record's own fields, which are: the data-source reference as one composite (its display text, e.g. `Table.Field`,
then the pool the field lives in as a narrowing enum and the index within that pool), two big-endian `u16` counts of the
object's highlighting rules (the second sizes the list and is never the smaller; a record that stops after the first
states its count once for both), and a signed-word marker followed by the reference's handle again — index first, then
the pool. A zero marker, which is what every known file stores, means the object takes its field from the reference; a
non-zero one makes the trailing handle authoritative and the field's own definition then runs to the end of the record.
Because the reference's text is variable, nothing after it sits at a fixed offset.

### `0x00a5` — TextObjectContainer

Opens a text object. Its nested `0x009e` ObjectName comes **first**, ahead of the record's own fields, which are: four
words of their own, the **paragraph count** as a big-endian `u16` — how many `0x00c0` paragraphs the object holds, which
the engine sizes the object from rather than counting the records that follow — then a trailing pair written only while
the record still has content: a big-endian `u32` and a signed word flagging a _field heading_ (a label attached to a
field). Where that flag is set, a `0x0166` FieldHeadingLink naming the field object follows; the two agree per stream.
Because the pair is a trailing cascade, a record may end before either, so the flag is the end of a sequence rather than
a byte at a fixed distance from the record's start.

### `0x00c2` — TextObject

One literal-text run of a paragraph: the run's text as a length-prefixed string, then a signed 4-byte character
spacing — the rigid extra advance added after each character, in twips. The spacing is written only while the record
still has content, so a run may end after its text. Nothing else belongs to the run: its font is the `0x08` record after
it, and a `0x00c3` closes it.

### `0x00c4` — TextEmbeddedField

One field run of a paragraph: an embedded field, formula, or parameter reference inside a text object's flowing text. It
opens with the same composite `0x009f` does — the reference's display text, then the pool the field lives in as a
narrowing enum and the index within that pool — followed by a signed 4-byte word of no known meaning. Then, written only
while the record still has content: the run's character spacing, the same signed 4-byte twip advance a `0x00c2` literal
run stores, and a signed-word marker followed by the reference's handle again, index first and then the pool, on the
same terms as `0x009f`. A `0x00c5` closes the run. Because the reference's text is variable, nothing after it sits at a
fixed offset — including the index a special field's own kind is read from, which is a whole big-endian `u16` and not
the byte beside the pool.

### `0x00a9` — DrawingObject (line / box)

Opens a line or box drawing object; geometry distinguishes the two. Its nested `0x009e` ObjectName comes **first**,
ahead of the record's own fields, which are: a signed word naming the section the shape's second corner lies in (its
index in layout order — so a shape spanning sections states where it ends rather than having it rediscovered from
section heights), the bottom-right corner as two narrowing twips (2 bytes normally, 4 when the value exceeds `0x7FFF`),
and a signed word flagging "extend to bottom of section". A related border record (`0x00ec`) classifies the shape (box
vs. line) and supplies styling.

### `0x00ae` — PictureObject

Opens a picture or OLE object. A bare `0x00ae` is a static picture or chart; wrapped by a `0x00b1` record, it is a
blob/image field bound to a database field. Its nested `0x009e` ObjectName comes **first**, ahead of the record's own
content, which is a single big-endian `u32` — written by every writer and read only while the record still has bytes
left, so a record that stops after the name is complete rather than truncated. The image itself is elsewhere: a static
picture's bytes live in the `Embedding N` storage the accompanying `0x00bd` record names, and a blob field's come from
the database, so this record is the same four bytes whichever kind of object it opens.

### `0x00b1` — BlobFieldWrapper

Wraps a `0x00ae` picture opener to make it a blob-field object, and is what tells one from a static picture — the opener
itself is identical either way. The wrapped `0x00ae` is the **first** thing in the content, ahead of every field of the
wrapper's own; a `0x00b2` record closes the run. The fields, in order:

| Field                              | Wire form                                                                | Meaning                                                                                                                                                                                                                                    |
|------------------------------------|--------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `data_source`                      | field reference (length-prefixed name, narrowing pool enum, `u16` index) | the database field the picture comes from; the index selects it from the report's own field definitions                                                                                                                                    |
| `natural_width` / `natural_height` | two narrowing twips                                                      | the picture's size **unscaled** — what the object's cropping and scaling are computed against, not its size on the page. `1440`×`1440` until a picture has been read from the field, since nothing about the image is known at design time |
| `blob_stream`                      | `u32` big-endian                                                         | the number of the `BLOB <n>l` stream caching the last picture read from the field                                                                                                                                                          |
| `blob_stream_is_zlib`              | signed word, optional                                                    | non-zero: the cache is the `zlibBLOB` stream instead. Every writer emits it set                                                                                                                                                            |
| `zlib_blob_stream`                 | `u32` big-endian, optional                                               | that stream's number. A record ending after `blob_stream` predates the zlib form, and its one number stands for both                                                                                                                       |

### `0x00a3` — SubreportObject

Opens a subreport placeholder object. A big-endian `u32` at offset 0 is the subdocument index — the `Subdocument N`
storage that holds the subreport's streams.

### `0x0166` — FieldHeadingLink

Names the field object that a text object is the heading for. The whole record is that one length-prefixed object name —
the reference is a name, not a handle into a pool and not an index.

### `0x0106` — SubreportLink

A subreport link: how a value passes from the main report into a subreport. The leading `u16` is the subreport parameter
index (the pairing key). The main-report field name is stored as a string, followed by a `(kind, index)` handle
re-stating it against the **main** report's own pools; a link flag then gates a second handle, read only when the flag
is zero, naming the **subreport** field the link feeds. In both handles `kind` selects the pool (0 = database field, 1 =
formula, 5 = parameter) and `index` the entry within it.

### `0x00bd` — OleObjectItem

Decorates a static picture / OLE object; its content opens with a big-endian 1-based ordinal naming the `Embedding N`
storage that holds the object's payload, followed by eight bytes of no known meaning that a record may end before.

### `0x00b4` — ChartObject

Opens a chart's binding block. Its content is one child — the `0x00b3` ChartAnalyticObject — followed by the chart's
render extent as a `TwipSize` (width then height, each a narrowing twip). Unlike the other object openers it does not
nest the `0x009e` ObjectName itself: the name is two levels down, in the `0x00ae` graphic base the `0x00b3` nests.

The chart's "show value" data field is carried by a `0x007f` SummaryFieldWrapper (around a `0x007e` child); the `0x011c`
ChartAnalyticHeader opens the analytic block (a word, then the layout type as a narrowing enum: `0` Detail / `1` Group /
`2` CrossTab, and a trailing word the record need not carry) and labeled analytic values arrive as `0x011f`
ChartDataValue records. `0x0128` is a pure container that carries no content of its own; `0x0121` ChartDefinition2 is
its style member rather than a second version of it, and its content carries the v2 chart type and subtype, the titles
and axis titles, the legend flags, position and layout, the four gridline modes (group, series, value, value-2), the
data-label mode, the shape and size enums (bar/pie/marker size, pie slice detachment, marker shape, colour mode, data
point, data-value number format), a per-axis run for each of the value, secondary-value and series axes (bounds, number
format, auto-range, auto-scale, division method and count), ten text-element font faces, and per-element flags and
weight/slant styles — and, for the pie/doughnut and 3-D families, the conditional fields only they carry (the viewing
angle among them). A per-element *point size* is not in this record, or anywhere else in `Contents`.

### `0x00b8` — CrossTabObject

Opens a cross-tab object, wrapped by a `0x00b9` record that starts the cross-tab binding block. The `0x009e` name is
nested in **this** record, not in the wrapper, and comes first in its content. The record then carries the grid's
display options — `show_grid`, its pen and colour, position, `keep_columns_together`, `repeat_row_labels`,
`suppress_empty_columns`, `suppress_empty_rows`, and the counts of grid columns, rows and cells. Each display option is
a whole signed word, not the low byte of one, and there is **no cell-margin flag here**: the stored twip margins are on
the `0x00d6` grid cells. The wrapper carries `suppress_column_grand_totals` and `suppress_row_grand_totals`.

The grid's dimensions and formats follow:

- `0x00cb` CrossTabDimensionField — a dimension level: a `TwipSize`, a `TwipPoint`, five scalars and an lp-string
  `{table.field}` reference.
- `0x00ce` CrossTabDimension — a column-axis level (`Column #N`); `0x00d2` CrossTabRecord — a row-axis level (`Row #N`).
  Each closes with a level **count**, not an option mask.
- `0x00d6` CrossTabGridCell — one grid cell: its horizontal and vertical margins as big-endian `i32` twips, its
  rectangle as narrowing twips, and its row/column indices.
- `0x0143` — the **count** of `0x0145` cell formats that follow; a record storing no word means sixteen.
- `0x0145` — one grid-region cell format: flags, a BGR background colour, and an enabled word.

## Formatting

Most format records attach to the object or section that precedes them. Conditional-format records hold an array of
formula slots: a property is either a fixed value or driven by a formula.

### `0x0008` — Font

An object's font, as the description it is looked up by: the length-prefixed face name, the family and pitch that
substitute for a missing face (both narrowing), a narrowing marker written as the constant `1`, the size in whole
points, the italic, underline and strikeout flags as whole signed words, and the weight as a big-endian `u16`
(`0x0190` = 400 normal, `0x02BC` = 700 bold). A trailing signed `i32` repeats the size in **twips** — the same quantity
in the only unit that can express a fractional point size — and is the one field a record need not carry. A multi-run
text object uses the first run's font.

Read the size from the twips and divide by 20; the whole-point field is that value rounded, and reading it instead loses
a half-point size. A record that ends before the twips states its size in whole points alone.

Note the type number is per stream: `0x0008` in `QESession` is the unrelated [QE_INDEX](#0x0008--qe_index-in-qesession)
record.

### `0x0100` — FontColor

An object's font color, as a `COLORREF` (`0x00BBGGRR`).

### `0x00ec` — ObjectBorder

An object's border styles and its border and background colors. Byte 25 is the shape type for box objects (1 = box, 2 =
line). Byte 9 flags a drop shadow.

### `0x00fc` — ObjectFormat

An object's format flags, including horizontal alignment (in byte 2). The object's hyperlink target text and type are
also carried in this record; the type byte alone decides whether there is a hyperlink (code `6`, `Undefined`, means
none), never whether the target text is empty.

### `0x00fd` — ObjectConditionFormat

An object's conditional-format formula slot array (the formulas driving its conditioned properties, such as suppression
and display string).

### `0x00c0` — TextObjectFormat

A text or heading object's paragraph format, including alignment (in byte 12).

### `0x00fe` — AreaSectionFormat

An area's or section's format flags — a 52-byte block of options (suppress, keep-together, new-page-before/after, and
similar).

### `0x00ff` — SectionConditionFormat

A section's conditional-format formula slot array.

### `0x0101` — FontConditionFormat

An object's font conditional-format formula slot array.

### `0x00ed` — ObjectAdornment

The conditional-format wrapper that parents a `0x00ec` ObjectBorder; it carries the border-color condition-formula
slots.

### `0x00ee`–`0x00fb` — Typed field sub-formats

A field object's typed display format streams after its `0x009f` opener as a fixed run of wrapper/value record pairs —
each odd wrapper carries conditioned-value slots and parents its even value child: Common (`0x00f0`/`0x00f1`), Numeric
(`0x00f8`/`0x00f9`), Boolean (`0x00ee`/`0x00ef`), Date (`0x00f2`/`0x00f3`), Time (`0x00f6`/`0x00f7`), DateTime
(`0x00f4`/`0x00f5`), and String (`0x00fa`/`0x00fb`). The Common/Numeric/Boolean/String value bytes are decoded into the
model, as are the Date, Time and DateTime leaves. What the file stores and what the engine *displays* still differ: for
a field using system defaults the engine resolves the effective date/time form at runtime from the value type and host
locale, so — like a formula's `NumberOfBytes` — the displayed format is left to the consumer that needs it.

Two notes on the layout. **Numeric** emits *two* consecutive `0x00f9`/`0x00f8` pairs — a currency-format slot then a
number-format slot; the engine reports the currency slot for a Currency-valued field and the number slot otherwise. Its
content is a sequence, not a fixed header: each enum is a **narrowing** integer, one byte only while its value stays
below `0x80`, so the offsets a corpus record happens to show are the common case rather than the layout. In order:
SuppressIfZero, NegativeFormat, ThousandsSeparator, UseLeadZero, DecimalPlaces (a whole `u16`), RoundingFormat,
CurrencySymbolFormat, OneCurrencySymbolPerPage, and **CurrencyPosition** (leading/trailing × inside/outside the negative
sign). Then the thousands, decimal and currency-symbol strings; AllowFieldClipping, a word, and DisplayReverseSign; the
ZeroValueString shown in place of a zero (`<Default Format>` is the engine's own
"unset" marker, not an empty string); and a further word and two strings the record may end before. **Date** (`0x00f2`)
is eight narrowing enums (order, year, month, day, day-of-week, system default, era, calendar), then the five literal
separators — named zero, first, second, third and day-of-week — then the day-of-week position and, as a trailing field
the record need not carry, the bracket pair the weekday element is wrapped in. **Time** (`0x00f6`) is five narrowing
enums — clock base, AM/PM placement, then the hour, minute and second element styles — followed by four lp-strings in
the writer's order: AM designator, PM designator, hour-minute separator, minute-second separator. The record ends there,
so nothing else about a time format is stored. **DateTime** (`0x00f4`) is the order enum selecting which of the two
parts show and in which sequence, then the literal written between them — the string begins after the enum's own width,
which is narrowing, not at a fixed offset. **String** (`0x00fa`) opens with word-wrap, then three **signed** `i32`
indent longs (first-line / left / right, in twips) and a maximum line count; the rest is read while the record still has
content — text interpretation (Standard / HTML), the line-spacing type and pitch, a long, and the reading order last.

## Authoring, saved data, and designer state

These records carry provenance and editor state.

### `0x0061` — SavedData

The saved-data block descriptor. Its presence marks `ReportDocument.HasSavedData`; the cached rows themselves live in
separate streams (see [Saved data](05-saved-data.md)). The record is two big-endian words: a constant, then the id of
the stream holding the saved instance state — the compound file names each stream `<name> <id>l`, and this id is the
suffix of its `AnalysisGridsStream`. It is a different id from the report root's saved-data handle, which names the
`DataSourceManager` stream, though both are drawn from one document-wide sequence. An empty `0x0062` closes the block.

### `0x0031` — CurrentValueRecord (in `ReportParametersStream`)

One parameter's saved **current value**: a global parameter index plus per-type value entries, joined against the
`0x007a` ParamRecord definitions by that index. `ReportParametersStream` is a TSLV record stream decoded through the
same pipeline as `Contents`.

### `0x0178` — SaveMetadata

One save-time environment key/value pair, one record per save event, kept in stream order.

### `0x0142` — SubreportReimportInfo

A subreport re-import descriptor, one per report whether or not it holds a subreport: the source path the
report/subreport was imported from as an lp-string, then the import timestamp as a Julian day and a same-day time
fraction, the "re-import when opening" enum, and the source's own save timestamp in the same two parts. A report that
imported nothing stores the empty path and a zero source timestamp, which is what makes a stored path the evidence that
a subreport was imported at all. The path being the first field, everything after it moves by the path's length.

### `0x010c` — GuidelineEntry

A designer snap guideline: a big-endian signed `i32` position in twips, then a `u16` count of the object connections
attached to the guide — the `0x0112` collections that follow this record inside the same guideline list.

### `0x0111` — ObjectConnection

A designer object-connection edge. The record does not state an object twice: an object kind and index name the one end,
then two longs and two narrowing words, then a content-guarded group of four words naming a sub-object (`-1` when none).
It is 22 bytes only where every narrowing field is at its narrow width.

## Record-type coverage

Practically every record type the library encounters is **identified and named** in the registry
(`RecordTag::name(dialect)` — a type number is per stream, so the lookup takes the stream's vocabulary), so
`rpt tree` renders each node with its type name rather than raw hex, and `rpt streams` names its per-stream "top types"
the same way, each in the vocabulary its stream is written in. Naming reaches down to codes `0x017e`/`0x017f`
(`CrossTabCustomMembersBegin`/`CrossTabCustomMembersEnd`, a bracket pair around a cross-tab's custom-group-members
collection — the opener's whole content is a big-endian `u32` stating how many members are in it, and the reader takes
exactly that many `0x0180`…`0x0181` member pairs before the closing `0x017f`. A cross-tab writes the bracket whether or
not it has custom members, so every collection in the corpus states none and the member records are absent). A record
type the registry has not seen is preserved verbatim as an `Unknown` node so the round-trip stays lossless. The
[support matrix](../reader/02-support-matrix.md) tracks which types are fully **decoded into the model** vs **named for
recognition** (structural bracket/wrapper records and opaque render state carry nothing a reader needs), and names the
one record type still `Unknown` corpus-wide — `0x32` in the `ReportParametersStream`.

## Feature areas: modelling depth

A record type being *named* does not mean its whole feature is *modelled*. Full OLAP-grid / map / alert / Flash /
XML-export structure is named at the family level but not decoded into the model, and the effective *runtime* display
format of typed field sub-formats remains outside the decode by design (the stored format is decoded; the
locale-resolved display value is not a stored fact, like a formula's `NumberOfBytes`). Not all object-level sub-format
condition formulas are decoded yet. See the [support matrix](../reader/02-support-matrix.md).

## See it yourself

The `rpt dump` command is the byte-layout workbench for a record type: an annotated hex dump of the record's own
demasked field bytes, and then the reading itself. What follows the hex depends on the type — one decoded from a field
table shows that table's own reading, every field's name, value and byte range, plus whether the table consumed the
record exactly; a type with no table shows a scalar-probe grid instead. So an entry on this page can be checked against
the decoder in one command:

```console
$ rpt dump report.rpt --type 0x0064
```

---

← [Saved data](05-saved-data.md) · [Index](README.md) · **Next:** [Endianness](07-endianness.md) →
