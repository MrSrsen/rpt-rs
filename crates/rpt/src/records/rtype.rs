//! The one place each record-type `u16` is named. Both the `project::raise` projection and the `io`
//! orchestration facade classify records by these types, so the constants live crate-wide rather
//! than being re-declared per module. The [`super::tag`] name table is the complementary big lookup
//! (type → symbolic name); this module is the by-name catalog the decoders match against.

// ---- `Contents` stream record types ----

pub(crate) const FIELD_DEF: u16 = 0x73; // a referenced field definition (name + value-type + length)
pub(crate) const FORMULA: u16 = 0x76; // a formula body (field refs + the formula body text)
pub(crate) const NAMED_VALUE: u16 = 0x71; // a named value; immediately follows a formula body to name it
pub(crate) const PRINTER: u16 = 0x03; // printer info (driver / name / port)
pub(crate) const PAGE_SETUP: u16 = 0x66; // page setup: the four page margins (BE u32 twips)
pub(crate) const PAPER_RECT: u16 = 0x018e; // the page rectangle: paper width + height (BE u32 twips)
pub(crate) const PAPER_DEVMODE: u16 = 0x0007; // page-setup DEVMODE: orientation / paper size / source
pub(crate) const SAVE_METADATA: u16 = 0x0178; // one save-time environment key/value pair
pub(crate) const DATA_SOURCE_OPTIONS: u16 = 0x0160; // report-level data-source options (ConvertNull/ConvertOther flags)
pub(crate) const AREA_MARKER: u16 = 0x8a; // an area, named e.g. "DetailArea1"
pub(crate) const SECTION_MARKER: u16 = 0x8c; // a section: Height (u32 BE twips) + Name

// Band markers: each parents its `0x008c` section and its record type is the authoritative band
// kind (the area/section name is user-renameable — a group band is often named after its group
// field, e.g. `nameHeader`/`customeridHeader`, so the name cannot be trusted for classification).
pub(crate) const REPORT_HEADER_BAND: u16 = 0x8d;
pub(crate) const REPORT_FOOTER_BAND: u16 = 0x8f;
pub(crate) const PAGE_HEADER_BAND: u16 = 0x91;
pub(crate) const PAGE_FOOTER_BAND: u16 = 0x93;
pub(crate) const DETAIL_BAND: u16 = 0x95;
pub(crate) const GROUP_HEADER_BAND: u16 = 0x97;
pub(crate) const GROUP_FOOTER_BAND: u16 = 0x99;

// `SectionCodeHeaderFooter` (0x9c): the per-area section-code wrapper that directly parents one
// `SectionCodeAreaType` (0x9b) leaf. That leaf's byte 0 is the area-type (01=Page, 02=Report,
// 03=Group, 04=Detail) and — for a group area (03) — byte 2 is the 0-based group nesting level.
// This is the authoritative source of an area's group level (the area *name* is user-renameable and
// its binary storage order need not match the group sequence).
pub(crate) const SECTION_CODE_HEADER_FOOTER: u16 = 0x9c;

// Each report object is a flat run of records: an *opener* (text / field / shape / picture)
// followed by the *attribute* records that decorate it (name+size, position, format, border,
// font colour, font, and — for text objects — the literal text) until the next opener.
pub(crate) const TEXT_OBJECT: u16 = 0xa5; // opens a text object; byte 15 == 1 marks a field heading
pub(crate) const TEXT_OBJECT_FORMAT: u16 = 0xc0; // a text/heading object's paragraph format (alignment in byte 12)
pub(crate) const TEXT_CONTENT: u16 = 0xc2; // a text object's literal text content
pub(crate) const TEXT_EMBEDDED_FIELD: u16 = 0xc4; // an embedded field/formula/parameter reference in a text object
pub(crate) const FIELD_HEADING_LINK: u16 = 0x0166; // names the FieldObject a text object is the heading for
pub(crate) const FIELD_OBJECT: u16 = 0x9f; // opens a field object (its data-source reference)
pub(crate) const LINE_OBJECT: u16 = 0xa9; // opens a line/box drawing object (geometry distinguishes them)
pub(crate) const PICTURE_OBJECT: u16 = 0xae; // opens a picture/OLE object
pub(crate) const BLOB_FIELD_REF: u16 = 0xb1; // wraps a picture opener; its leaf holds the bound blob field ref
pub(crate) const OLE_OBJECT_ITEM: u16 = 0xbd; // decorates a static/OLE picture; leaf [0..4] BE = 1-based Embedding N ordinal
pub(crate) const SUBREPORT_OBJECT: u16 = 0xa3; // opens a subreport placeholder object
pub(crate) const SUBREPORT_LINK: u16 = 0x0106; // a subreport link record (follows the 0xa3 object)
pub(crate) const CROSSTAB_OBJECT: u16 = 0xb8; // opens a cross-tab object (wrapped by 0xb9; parents the 0x9e name)
pub(crate) const CROSSTAB_WRAPPER: u16 = 0xb9; // wraps the 0xb8 cross-tab opener; starts the cross-tab binding block
pub(crate) const CROSSTAB_CUSTOM_MEMBERS_BEGIN: u16 = 0x017e; // opens a cross-tab's custom-group-members collection; 4B leaf = u32 count
pub(crate) const CHART_BINDING: u16 = 0xb4; // starts a chart's binding block (nests the chart's ObjectName)
pub(crate) const CHART_DATA: u16 = 0x7f; // wraps a chart's data ("show value") field ref (0x7e child)
pub(crate) const CHART_ANALYTIC: u16 = 0x011c; // chart analytic header; leaf byte 2 = ChartLayoutType (0 Detail/1 Group/2 CrossTab)
pub(crate) const CHART_DATA_VALUE: u16 = 0x011f; // labeled-value analytic record ("Count of Command.some_field")
pub(crate) const CHART_DEFINITION2: u16 = 0x0121; // v2 chart-definition/styling leaf (type + titles)
pub(crate) const OBJECT_NAME: u16 = 0x9e; // an object's Name + Width/Height
pub(crate) const OBJECT_POS: u16 = 0xbe; // an object's Left/Top (u16 twips)
pub(crate) const OBJECT_FORMAT: u16 = 0xfc; // an object's format flags (horizontal alignment in byte 2)
pub(crate) const OBJECT_COND: u16 = 0xfd; // an object's conditional-format formula slot array
pub(crate) const OBJECT_BORDER: u16 = 0xec; // an object's border styles + border/background colours
pub(crate) const OBJECT_BORDER_COND: u16 = 0xed; // wrapper parenting `0xec`; carries border colour cond slots
pub(crate) const AREA_SECTION_FORMAT: u16 = 0xfe; // an area's or section's format flags (52-byte block)
pub(crate) const PARAM_RECORD: u16 = 0x007a; // a parameter field's detail record (XOR-0x7a obfuscated)
pub(crate) const SECTION_COND: u16 = 0xff; // a section's conditional-format formula slot array
pub(crate) const FONT_COLOR: u16 = 0x0100; // an object's font colour (COLORREF 0x00BBGGRR)
pub(crate) const FONT_COND: u16 = 0x0101; // an object's font conditional-format formula slot array
pub(crate) const FONT: u16 = 0x08; // an object's font (name + size + weight)

// Typed field-format family: each wrapper (odd) carries conditioned-value slots and parents its
// value child (even). The block streams after every `0x9f` field opener, in the fixed order
// f1 f9 f9 ef f3 f7 f5 fb. Only Common/Numeric/Boolean/String are byte-derived; the Date/Time
// sub-formats are runtime-resolved (their leaves are the uniform default for every field).
pub(crate) const FF_COMMON_WRAPPER: u16 = 0xf1; // wraps 0xf0 CommonFieldFormat
pub(crate) const FF_NUMERIC_WRAPPER: u16 = 0xf9; // wraps 0xf8 NumericFieldFormat (streamed twice; 2nd is authoritative)
pub(crate) const FF_BOOLEAN_WRAPPER: u16 = 0xef; // wraps 0xee BooleanFieldFormat
pub(crate) const FF_DATE_WRAPPER: u16 = 0xf3; // wraps 0xf2 DateFieldFormat (classified for coverage; runtime-resolved)
pub(crate) const FF_TIME_WRAPPER: u16 = 0xf7; // wraps 0xf6 TimeFieldFormat (runtime-resolved)
pub(crate) const FF_DATETIME_WRAPPER: u16 = 0xf5; // wraps 0xf4 DateTimeFieldFormat (runtime-resolved)
pub(crate) const FF_STRING_WRAPPER: u16 = 0xfb; // wraps 0xfa StringFieldFormat

// The value child parented by each wrapper above (wrapper − 1). Only the byte-derived ones are
// matched by the `FieldFormatBlock` decode arm.
pub(crate) const FF_COMMON_VALUE: u16 = 0xf0; // CommonFieldFormat
pub(crate) const FF_NUMERIC_VALUE: u16 = 0xf8; // NumericFieldFormat
pub(crate) const FF_BOOLEAN_VALUE: u16 = 0xee; // BooleanFieldFormat
pub(crate) const FF_DATE_VALUE: u16 = 0xf2; // DateFieldFormat (stored day/month/year enums)
pub(crate) const FF_TIME_VALUE: u16 = 0xf6; // TimeFieldFormat (stored hour/minute/second enums)
pub(crate) const FF_DATETIME_VALUE: u16 = 0xf4; // DateTimeFieldFormat (order + date/time separator)
pub(crate) const FF_STRING_VALUE: u16 = 0xfa; // StringFieldFormat (text-format/word-wrap/reading-order)

pub(crate) const GROUP: u16 = 0xe5; // a group: its condition field (+ "@Group #N Order")
pub(crate) const HIER_GROUP: u16 = 0xe9; // a specified-order group value: [LP name][LP condition]
pub(crate) const GROUP_OPTIONS: u16 = 0x88; // GroupAreaFormat of the group whose 0xe5 immediately follows it
pub(crate) const FIELD_MANAGER_ENTRY: u16 = 0x6e; // field-pool census (20B: db-field count + formula count …)
pub(crate) const CROSSTAB_DIM_FIELD: u16 = 0xcb; // a cross-tab dimension level (header + LP {table.field} ref)
pub(crate) const CROSSTAB_COLUMN_AXIS: u16 = 0xce; // CrossTabDimension: opens a column-axis level ("Column #N")
pub(crate) const CROSSTAB_ROW_AXIS: u16 = 0xd2; // CrossTabRecord: opens a row-axis level ("Row #N")
pub(crate) const CROSSTAB_GRID_FORMAT: u16 = 0x0143; // grid-level format word (u16 BE); opens the cell-format run
pub(crate) const CROSSTAB_GRID_CELL_FORMAT: u16 = 0x0145; // one grid-region cell format (11B: flags + BGR bg + flag)
pub(crate) const REIMPORT_INFO: u16 = 0x0142; // subreport re-import descriptor (source path + import timestamps)
pub(crate) const GUIDELINE_ENTRY: u16 = 0x010c; // a designer snap guideline ([u32 BE pos-twips][u16 flags])
pub(crate) const OBJECT_CONNECTION: u16 = 0x0111; // a designer object-connection edge (22B: src/dst/kind)
pub(crate) const RECORD_SORT_FIELD: u16 = 0x29; // a record-level sort: field ref + direction (last byte)
pub(crate) const SUMMARY_DEF: u16 = 0x7e; // a summary/running-total def (operation byte + summarized field)
pub(crate) const RT_RESET: u16 = 0x80; // a running total's reset condition (precedes its 0x7e)
pub(crate) const SQL_EXPRESSION: u16 = 0x81; // a SQL Expression field: SQL text (LP) + 0x71 name/type/length child
pub(crate) const FORMULA_VARIABLE: u16 = 0x0118; // one persisted Global/Shared formula variable (name+type+scope)
                                                 // (the preceding `0x0116` table header just holds the count — redundant, so not parsed)
pub(crate) const REPORT_HEADER: u16 = 0x0064; // top-level report header (option bits: byte 24 bit 0 = save-data)
pub(crate) const SAVED_DATA: u16 = 0x0061; // saved-data block descriptor (present ⟺ ReportDocument.HasSavedData)
pub(crate) const MULTI_COLUMN: u16 = 0x6c; // multi-column detail layout ("Format with Multiple Columns"); report-level singleton

// ---- `ReportParametersStream` record types ----

pub(crate) const CURRENT_VALUE_RECORD: u16 = 0x0031; // a parameter's current-value record (index + per-type entries)

// ---- `QESession` (Query Engine) record types — the database/connection metadata ----

pub(crate) const QE_CONNECTION: u16 = 0x02; // connection container (Database_DLL / type / database name)
pub(crate) const QE_TABLE: u16 = 0x03; // a table: name (+ alias), the SQL command text, and its fields
pub(crate) const QE_FIELD: u16 = 0x04; // a table data field: name + value-type code + length
pub(crate) const QE_COMMAND_PARAM: u16 = 0x07; // a command/stored-proc bind parameter: name + value type
pub(crate) const QE_TABLE_LINK: u16 = 0x0a; // a table link: src/dst field ids + join type
pub(crate) const QE_EMPTY_MARKER: u16 = 0x00; // empty child record; on a String field marks an `nvarchar` column

// ---- `DataSourceManager` record types — the saved-data catalog and batch directory ----

pub(crate) const DSM_STRUCTURE: u16 = 0x2d; // saved-records structure record (parents the batch directory)
pub(crate) const DSM_BATCH_ENTRY: u16 = 0x6d; // one batch directory entry: count + item_size (+ packed column table)
pub(crate) const DSM_FIELD_CONTAINER: u16 = 0x07; // container of stored database-field descriptors
pub(crate) const DSM_FIELD_HEADER: u16 = 0x41; // a stored field's header (record-layout index + byte offset)
pub(crate) const DSM_FIELD_DESC: u16 = 0x40; // a stored field's descriptor (name + variable-length marker)
