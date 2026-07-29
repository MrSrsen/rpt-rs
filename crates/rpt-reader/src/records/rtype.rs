//! The record types the decoders match on, each named after the field table that declares it.
//!
//! A number is written once, where the record's content is declared: every constant here is its
//! table's own `rtype`, so the two catalogs cannot come to disagree about which number a name
//! stands for. The name is the table's too — `numbers!` builds each constant from a table of the
//! same name, so a name no table carries does not compile.
//!
//! Taking the number from a table also settles the dialect, which a bare constant cannot state: a
//! type number is per stream, and `0x0007` is the report definition's page devmode, the query
//! engine's command parameter and the saved-data catalog's field container. Each constant below is
//! the number in the vocabulary its table declares, and never stands for the record another stream
//! writes under it.
//!
//! The constants exist at all because a decoder matches on them, and a `match` arm needs a path to
//! a constant — which a table's `rtype` field is not. The [`super::tag`] name table is the
//! complementary big lookup (type → symbolic name, for display).

/// Name a record type after the field table that declares it, taking the number from the table.
///
/// The indirection is the point, and writing these out as plain constants gives it up: each name
/// here has to be a table's name, because that is the path the number is read through, so a name no
/// table carries fails to compile and one that names the wrong table reads that table's number
/// under a name the reader will not recognise. Spelling the numbers out instead risks a silent
/// divergence between the two catalogs.
macro_rules! numbers {
    ($( $(#[$doc:meta])* $name:ident ),+ $(,)?) => {
        $(
            $(#[$doc])*
            pub(crate) const $name: u16 = crate::field_table::tables::$name.rtype;
        )+
    };
}

numbers! {
    // ---- `Contents` stream record types ----

    /// A referenced field definition (name + value-type + length).
    FIELD_DEFINITION,
    /// A formula body (field refs + the formula body text).
    FORMULA,
    /// A named value; immediately follows a formula body to name it.
    NAMED_VALUE,
    /// Page setup: the four page margins (BE u32 twips).
    PAGE_SETUP,
    /// Page-setup DEVMODE: orientation / paper size / source.
    PAGE_DEVMODE,
    /// One save-time environment key/value pair.
    SAVE_METADATA,
    /// An area, named e.g. "DetailArea1".
    AREA,
    /// A section: Height (u32 BE twips) + Name.
    SECTION,

    // Band markers: each parents its `0x008c` section and its record type is the authoritative band
    // kind (the area/section name is user-renameable — a group band is often named after its group
    // field, e.g. `nameHeader`/`customeridHeader`, so the name cannot be trusted for classification).
    REPORT_HEADER_BAND,
    REPORT_FOOTER_BAND,
    PAGE_HEADER_BAND,
    PAGE_FOOTER_BAND,
    DETAIL_BAND,
    GROUP_HEADER_BAND,
    GROUP_FOOTER_BAND,

    /// The per-area section-code wrapper that directly parents one `SectionCodeAreaType` (`0x9b`)
    /// record. Its byte 0 is the area-type (01=Page, 02=Report, 03=Group, 04=Detail) and — for a
    /// group area (03) — byte 2 is the 0-based group nesting level. This is the authoritative
    /// source of an area's group level (the area *name* is user-renameable and its binary storage
    /// order need not match the group sequence).
    SECTION_CODE_HEADER_FOOTER,

    // Each report object is a flat run of records: an *opener* (text / field / shape / picture)
    // followed by the *attribute* records that decorate it (name+size, position, format, border,
    // font color, font, and — for text objects — the literal text) until the next opener.
    /// Opens a text object; parents the text runs that carry its content.
    TEXT_OBJECT_CONTAINER,
    /// A text/heading object's paragraph format (alignment in byte 12).
    TEXT_OBJECT_FORMAT,
    /// One literal-text run of a text object's paragraph.
    TEXT_OBJECT,
    /// An embedded field/formula/parameter reference in a text object.
    TEXT_EMBEDDED_FIELD,
    /// Names the field object a text object is the heading for.
    FIELD_HEADING_LINK,
    /// Opens a field object (its data-source reference).
    FIELD_OBJECT,
    /// Opens a line/box drawing object (geometry distinguishes them).
    DRAWING_OBJECT,
    /// Opens a picture/OLE object.
    PICTURE_OBJECT,
    /// Wraps a picture opener: the blob field it shows, and where its picture is cached.
    BLOB_FIELD_WRAPPER,
    /// Decorates a static/OLE picture; bytes [0..4] BE = 1-based Embedding N ordinal.
    OLE_OBJECT_ITEM,
    /// Opens a subreport placeholder object.
    SUBREPORT_OBJECT,
    /// A subreport link record (follows the subreport object).
    SUBREPORT_LINK,
    /// Opens a cross-tab object (wrapped by the cross-tab wrapper; parents the object name).
    CROSSTAB_OBJECT,
    /// Wraps the cross-tab opener; starts the cross-tab binding block.
    CROSSTAB_WRAPPER,
    /// Opens a cross-tab's custom-group-members collection, stating how many are in it.
    CROSSTAB_CUSTOM_MEMBERS,
    /// Opens a chart object; its binding block follows (the object name is a descendant, not a
    /// child).
    CHART_OBJECT,
    /// Chart analytic header; byte 2 = ChartLayoutType (0 Detail/1 Group/2 CrossTab).
    CHART_ANALYTIC,
    /// Labeled-value analytic record ("Count of Command.some_field").
    CHART_DATA_VALUE,
    /// v2 chart-definition/styling record (type + titles).
    CHART_DEFINITION2,
    /// An object's Name + Width/Height.
    OBJECT_NAME,
    /// An object's Left/Top (u16 twips).
    OBJECT_POSITION,
    /// An object's format flags (horizontal alignment in byte 2).
    OBJECT_FORMAT,
    /// An object's conditional-format formula slot array.
    OBJECT_FORMAT_WRAPPER,
    /// An object's border styles + border/background colors.
    BORDER,
    /// Wrapper parenting the border; carries border color cond slots.
    BORDER_WRAPPER,
    /// An area's or section's format flags (52-byte block).
    AREA_SECTION_FORMAT,
    /// A section's conditional-format formula slot array.
    SECTION_FORMAT_WRAPPER,
    /// An object's font color (COLORREF 0x00BBGGRR).
    FONT_COLOR,
    /// An object's font conditional-format formula slot array.
    FONT_CONDITION_FORMAT,
    /// An object's font (name + size + weight).
    FONT,

    // Typed field-format family: each wrapper (odd) carries conditioned-value slots and parents its
    // value child (even). The block streams after every field opener, in the fixed order
    // f1 f9 f9 ef f3 f7 f5 fb. Only Common/Numeric/Boolean/String are byte-derived; the Date/Time
    // sub-formats are runtime-resolved (their stored values are the uniform default for every field).
    /// Wraps the common field format.
    COMMON_FIELD_FORMAT_WRAPPER,
    /// Wraps the numeric field format (streamed twice; the second is authoritative).
    NUMERIC_FIELD_FORMAT_WRAPPER,
    /// Wraps the Boolean field format.
    BOOLEAN_FIELD_FORMAT_WRAPPER,
    /// Wraps the date field format (classified for coverage; runtime-resolved).
    DATE_FIELD_FORMAT_WRAPPER,
    /// Wraps the time field format (runtime-resolved).
    TIME_FIELD_FORMAT_WRAPPER,
    /// Wraps the date-time field format (runtime-resolved).
    DATE_TIME_FIELD_FORMAT_WRAPPER,
    /// Wraps the string field format.
    STRING_FIELD_FORMAT_WRAPPER,

    // The value child parented by each wrapper above (wrapper − 1). Only the byte-derived ones are
    // matched by the field-format block's decode arm.
    /// The format members every field carries.
    COMMON_FIELD_FORMAT,
    /// Decimal places, negatives, currency, rounding, separators.
    NUMERIC_FIELD_FORMAT,
    /// The Boolean outputs (True/False, Yes/No, …).
    BOOLEAN_FIELD_FORMAT,
    /// Stored day/month/year enums.
    DATE_FIELD_FORMAT,
    /// Stored hour/minute/second enums.
    TIME_FIELD_FORMAT,
    /// Order + date/time separator.
    DATE_TIME_FIELD_FORMAT,
    /// Text format / word wrap / reading order.
    STRING_FIELD_FORMAT,

    /// A group: its condition field (+ "@Group #N Order").
    GROUP,
    /// One value of a group's specified order.
    HIERARCHICAL_GROUP_VALUE,
    /// The area-pair options of the group whose `0xe5` immediately follows it.
    GROUP_AREA_FORMAT,
    /// Field-pool census (20B: db-field count + formula count …).
    FIELD_MANAGER_ENTRY,
    /// A cross-tab dimension level (header + LP {table.field} ref).
    CROSSTAB_DIM_FIELD,
    /// Opens a column-axis level ("Column #N").
    CROSSTAB_COLUMN_AXIS,
    /// Opens a row-axis level ("Row #N").
    CROSSTAB_ROW_AXIS,
    /// One grid cell, nested under a `0xd7` summary block.
    CROSSTAB_GRID_CELL,
    /// Grid-level format word (u16 BE); opens the cell-format run.
    CROSSTAB_GRID_FORMAT,
    /// One grid-region cell format (11B: flags + BGR bg + flag).
    CROSSTAB_GRID_CELL_FORMAT,
    /// A record-level sort: field ref + direction (last byte).
    RECORD_SORT_FIELD,
    /// A summary/running-total definition (operation byte + summarized field).
    SUMMARY_FIELD_DEFINITION,
    /// Wraps a summary definition and states which kind of owner it belongs to — a chart's "show
    /// value" among them.
    SUMMARY_FIELD_WRAPPER,
    /// A running total: its conditions, around the summary definition it contains.
    RUNNING_TOTAL_FIELD,
    /// A SQL Expression field: SQL text (LP) + a named-value child.
    SQL_EXPRESSION_FIELD,
    /// One persisted Global/Shared formula variable (name+type+scope). The `0x0116` table header
    /// before it just holds the count — redundant, so not parsed.
    FORMULA_VARIABLE,
    /// Saved-data block descriptor (present ⟺ ReportDocument.HasSavedData).
    SAVED_DATA,

    // ---- `DataSourceManager` record types — the saved-data catalog and batch directory ----

    /// Saved-records structure record (parents the batch directory).
    SAVED_RECORDS_STRUCTURE,
    /// One batch directory entry: count + item_size (+ packed column table).
    SAVED_BATCH_ENTRY,
    /// A stored field's header (record-layout index + byte offset).
    SAVED_FIELD_HEADER,
    /// A stored field's descriptor (name + variable-length marker).
    SAVED_FIELD_DESCRIPTOR,
}

/// `DataSourceManager` `0x0007` — the container of stored database-field descriptors.
///
/// The one number here written out directly, because no field table declares this type: there is
/// no `tables::SAVED_FIELD_CONTAINER` to read it from. It is a known remainder, not an exception to
/// the rule above — tabling the type would replace this with a `numbers!` entry like every other.
///
/// Being written out, it also has to say its own dialect: this is the saved-data catalog's `0x0007`,
/// and the report definition writes its page devmode under the same number.
pub(crate) const SAVED_FIELD_CONTAINER: u16 = 0x0007;
