//! A record type number means something different in each stream: these describe the query-engine
//! session's records, whose schema words carry the `0x09` dialect marker.
//!
//! The query engine's wire vocabulary differs from the report definition's in three ways that
//! every table below depends on:
//!
//! - A **boolean is two bytes**, not one: the archive routes it through its 16-bit reader.
//! - A **string's count includes its terminating NUL**, so an empty string is five bytes
//!   (`00 00 00 01 00`) and a null one is four (`00 00 00 00`). A **blob's** count is its bytes,
//!   with no terminator — which is why the two are separate kinds even though they frame alike.
//! - A **collection** is a `u32` item count in the parent's own byte run followed by that many
//!   nested records, so it appears here as a count field and a repeat driven by it. An empty
//!   collection is four zero bytes, indistinguishable from a null string — which is exactly how a
//!   collection came to be read as one.
//!
//! Every one of these record types is versioned, and its reader adds fields at named versions. The
//! gates match the engine's own versioning rather than what any committed fixture contains, so a
//! record written by an older version still decodes correctly.

use super::*;

/// `0x0004 QeField` — one column of a table: its id (the key a table link resolves against), name,
/// description, value type and stored byte length.
///
/// `length` is the column's byte count, with `0xffffffff` / `0x7fffffff` / `0x7ffffffe` standing
/// for an unlimited column.
///
/// Five fields were added one version at a time, and the last four are what a fixed run of eleven
/// bytes stood in for: the identifier is a string, so a column that carries one moves the two
/// fields after it.
pub(crate) const QE_FIELD: Table = Table {
    dialect: Dialect::QeSession,
    rtype: 0x0004,
    name: "QeField",
    fields: &[
        Field::new("field_id", Kind::U32Be),
        Field::new("name", Kind::Str),
        Field::new("description", Kind::Str),
        Field::new("value_type", Kind::U32Be),
        Field::new("length", Kind::U32Be),
        Field::from_schema("attributes", Kind::U32Be, 0x0901),
        Field::from_schema("precision", Kind::U32Be, 0x0902),
        Field::from_schema("id", Kind::Str, 0x0903),
        Field::from_schema("can_be_processed_on_server", Kind::Bool, 0x0904),
        Field::from_schema("field_lineage", Kind::U32Be, 0x0905),
    ],
};

/// One part of a table's qualified name — the catalog and schema a provider prefixes it with.
const QE_TABLE_QUALIFIER: &[Field] = &[Field::new("part", Kind::Str)];

/// One nested field record of a `0x0003 QeTable`.
const QE_TABLE_FIELD: &[Field] = &[Field::new("field", Kind::Child(0x0004))];

/// One nested bind-parameter record of a `0x0003 QeTable`.
const QE_TABLE_PARAMETER: &[Field] = &[Field::new("parameter", Kind::Child(0x0007))];

/// One nested index record of a `0x0003 QeTable`.
const QE_TABLE_INDEX: &[Field] = &[Field::new("index", Kind::Child(0x0008))];

/// One column of a `0x0008 QeIndex`, by the field id the column's own record carries.
const QE_INDEX_FIELD: &[Field] = &[Field::new("field_id", Kind::U32Be)];

/// `0x0008 QeIndex` — one index the provider reports on a table: its name, whether it is the
/// primary key and whether its values are unique, then the columns it covers.
///
/// The record has never been revised, so its version is its stream's default and its header states
/// no schema word — the short header form.
pub(crate) const QE_INDEX: Table = Table {
    dialect: Dialect::QeSession,
    rtype: 0x0008,
    name: "QeIndex",
    fields: &[
        Field::new("index_id", Kind::U32Be),
        Field::new("name", Kind::Str),
        Field::new("is_primary_key", Kind::Bool),
        Field::new("has_unique_values", Kind::Bool),
        Field::new("field_count", Kind::U32Be),
        Field::new(
            "fields",
            Kind::Repeat {
                count: Count::FromField("field_count"),
                body: QE_INDEX_FIELD,
            },
        ),
    ],
};

/// `0x0003 QeTable` — one table the report reads: its identity strings, its columns, a SQL
/// command's bind parameters, its indexes, and the command text.
///
/// Every child run is counted: the count precedes the run, so the record states how many columns,
/// bind parameters and indexes follow rather than leaving them to be found. The command text is a
/// stated field, empty for a plain database table — so it needs no SQL sniff and no "longest
/// string wins" search.
///
/// Six fields were added one version at a time after the command text, and the last of them is a
/// collection rather than a string — four zero bytes either way, which is why it read as one.
pub(crate) const QE_TABLE: Table = Table {
    dialect: Dialect::QeSession,
    rtype: 0x0003,
    name: "QeTable",
    fields: &[
        Field::new("table_id", Kind::U32Be),
        Field::new("name", Kind::Str),
        Field::new("description", Kind::Str),
        Field::new("qualified_name", Kind::Str),
        Field::new("qualifier_count", Kind::U32Be),
        Field::new(
            "qualifiers",
            Kind::Repeat {
                count: Count::FromField("qualifier_count"),
                body: QE_TABLE_QUALIFIER,
            },
        ),
        Field::new("table_type", Kind::U32Be),
        Field::new("alias", Kind::Str),
        Field::new("is_flat", Kind::Bool),
        Field::new("is_linkable", Kind::Bool),
        Field::new("field_count", Kind::U32Be),
        Field::new(
            "fields",
            Kind::Repeat {
                count: Count::FromField("field_count"),
                body: QE_TABLE_FIELD,
            },
        ),
        Field::new("parameter_count", Kind::U32Be),
        Field::new(
            "parameters",
            Kind::Repeat {
                count: Count::FromField("parameter_count"),
                body: QE_TABLE_PARAMETER,
            },
        ),
        Field::new("index_count", Kind::U32Be),
        Field::new(
            "indexes",
            Kind::Repeat {
                count: Count::FromField("index_count"),
                body: QE_TABLE_INDEX,
            },
        ),
        Field::new("command_text", Kind::Str),
        Field::from_schema("external_indexes", Kind::Str, 0x0901),
        Field::from_schema("overridden_qualified_name", Kind::Str, 0x0902),
        Field::from_schema("file_name", Kind::Str, 0x0903),
        Field::from_schema("binary_data", Kind::Blob, 0x0903),
        Field::from_schema("id", Kind::Str, 0x0904),
        // The properties collection. Its items are `0x0009` records; the count is ordinarily zero,
        // so the run itself is left unstated rather than declared unseen — a non-zero count leaves
        // the record unaccounted for, which is the loud answer.
        Field::from_schema("property_count", Kind::U32Be, 0x0905),
    ],
};

/// One logon-property record of a `0x0002 QeConnection`.
const QE_CONNECTION_PROPERTY: &[Field] = &[Field::new("property", Kind::Child(0x0009))];

/// One table record of a `0x0002 QeConnection`.
const QE_CONNECTION_TABLE: &[Field] = &[Field::new("table", Kind::Child(0x0003))];

/// `0x0002 QeConnection` — one connection: the driver DLL, the database-type display name and the
/// server description, then the logon-property bag, the properties, and the tables served through
/// it.
///
/// The three strings are the first three fields of the record, so the driver DLL is read at its
/// stated position rather than found by looking for a slot whose text ends `.dll`. Each child run
/// is preceded by its own count.
pub(crate) const QE_CONNECTION: Table = Table {
    dialect: Dialect::QeSession,
    rtype: 0x0002,
    name: "QeConnection",
    fields: &[
        Field::new("connection_id", Kind::U32Be),
        Field::new("driver_dll", Kind::Str),
        Field::new("database_type", Kind::Str),
        Field::new("server", Kind::Str),
        Field::new("logon_property_count", Kind::U32Be),
        Field::new(
            "logon_properties",
            Kind::Repeat {
                count: Count::FromField("logon_property_count"),
                body: QE_CONNECTION_PROPERTY,
            },
        ),
        Field::new("property_count", Kind::U32Be),
        Field::new(
            "properties",
            Kind::Repeat {
                count: Count::FromField("property_count"),
                body: QE_CONNECTION_PROPERTY,
            },
        ),
        Field::new("table_count", Kind::U32Be),
        Field::new(
            "tables",
            Kind::Repeat {
                count: Count::FromField("table_count"),
                body: QE_CONNECTION_TABLE,
            },
        ),
        // The SQL-expression fields the connection serves; their items are `0x0005` records.
        // Ordinarily zero, so the run is left unstated and a non-zero count is loud.
        Field::new("expression_field_count", Kind::U32Be),
        // The logon data the driver stores as bytes rather than as properties: a blob, so a
        // connection that carries one moves everything after it.
        Field::from_schema("binary_logon_data", Kind::Blob, 0x0901),
        // The connection-parameter collection. Its item record type is not established, and the
        // count is ordinarily zero.
        Field::from_schema("connection_parameter_count", Kind::U32Be, 0x0902),
    ],
};

/// `0x0009 QeLogonProperty` — one entry of a connection's property bag: its key, the designer's
/// display name and help text, and the value.
///
/// The value is a nested `0x000b` record the tree does not split out, so it is read here as the
/// two header bytes and the counted block that follows them. The decoder reads that block only for
/// the handful of keys that name the database, the server and the user; nothing else in the bag is
/// surfaced, and no credential-carrying key is read or emitted.
///
/// One version of the record reads three fields where every other reads a single attribute word —
/// an alternative layout rather than an addition, so a reader that treats the word as always
/// present is eight bytes out on the older form.
pub(crate) const QE_LOGON_PROPERTY: Table = Table {
    dialect: Dialect::QeSession,
    rtype: 0x0009,
    name: "QeLogonProperty",
    fields: &[
        Field::new("property_id", Kind::U32Be),
        Field::new("key", Kind::Str),
        Field::new("display_name", Kind::Str),
        Field::new("description", Kind::Str),
        Field::new("_marker", Kind::Skip(2)),
        Field::new("_value", Kind::Str),
        Field::new("data_type", Kind::U32Be),
        Field::only_at_schema("is_optional", Kind::Bool, 0x0900),
        Field::only_at_schema("is_advanced", Kind::Bool, 0x0900),
        Field::only_at_schema("options", Kind::U32Be, 0x0900),
        Field::when("attributes", Kind::U32Be, |c| c.schema != 0x0900),
        // The nested property collection; ordinarily zero.
        Field::new("property_count", Kind::U32Be),
    ],
};

/// `0x0007 QeCommandParameter` — one bind parameter of a SQL command or stored procedure.
///
/// Every width here rests on the engine reader rather than on a byte diff, and the two nested value
/// records are read as the two header bytes and the counted block that follows them, as elsewhere
/// in this stream.
///
/// Two of its fields are variants — a type word and a payload the type word chooses. Only the type
/// word is stated, so a parameter that stores a bound minimum or maximum leaves the record
/// unaccounted for rather than being read at shifted offsets.
pub(crate) const QE_COMMAND_PARAMETER: Table = Table {
    dialect: Dialect::QeSession,
    rtype: 0x0007,
    name: "QeCommandParameter",
    fields: &[
        Field::new("parameter_id", Kind::U32Be),
        Field::new("name", Kind::Str),
        Field::new("description", Kind::Str),
        Field::new("direction", Kind::U32Be),
        Field::new("value_type", Kind::U32Be),
        Field::new("length", Kind::U32Be),
        Field::new("is_nullable", Kind::Bool),
        Field::new("allows_multiple_values", Kind::Bool),
        Field::new("allows_ranges", Kind::Bool),
        Field::new("_default_marker", Kind::Skip(2)),
        Field::new("_default_value", Kind::Str),
        Field::from_schema("attributes", Kind::U32Be, 0x0901),
        Field::from_schema("precision", Kind::U32Be, 0x0902),
        Field::from_schema("allow_discrete_values", Kind::Bool, 0x0903),
        Field::from_schema("is_part_of_group", Kind::Bool, 0x0903),
        Field::from_schema("group_number", Kind::U16Be, 0x0903),
        Field::from_schema("is_mutually_exclusive_group", Kind::Bool, 0x0903),
        Field::from_schema("is_radio_button", Kind::Bool, 0x0903),
        Field::from_schema("is_check_box", Kind::Bool, 0x0903),
        Field::from_schema("id", Kind::Str, 0x0904),
        Field::from_schema("_description_marker", Kind::Skip(2), 0x0905),
        Field::from_schema("_description_value", Kind::Str, 0x0905),
        Field::from_schema("prompting_time", Kind::U32Be, 0x0905),
        Field::from_schema("max_value_type", Kind::U16Be, 0x0905),
        Field::from_schema("min_value_type", Kind::U16Be, 0x0905),
        Field::from_schema("edit_mask", Kind::Str, 0x0905),
        Field::from_schema("allow_custom_values", Kind::Bool, 0x0905),
        Field::from_schema("default_sort_order", Kind::U32Be, 0x0905),
        Field::from_schema("default_sort_method", Kind::U32Be, 0x0905),
        Field::from_schema("default_display_type", Kind::U32Be, 0x0905),
    ],
};

/// `0x000a QeTableLink` — one field pair of a join. A compound key is stored as one record per
/// pair, so a join over two columns is two of these.
///
/// The operator and the join kind are independent one-hot codes: the designer sets outer-ness and
/// the comparison separately, and the file mirrors that split.
pub(crate) const QE_TABLE_LINK: Table = Table {
    dialect: Dialect::QeSession,
    rtype: 0x000a,
    name: "QeTableLink",
    fields: &[
        Field::new("link_id", Kind::U32Be),
        Field::new("source_field_id", Kind::U32Be),
        Field::new("target_field_id", Kind::U32Be),
        Field::new("operator", Kind::U32Be),
        Field::new("join_kind", Kind::U32Be),
        Field::from_schema("table_join_enforced", Kind::U32Be, 0x0901),
    ],
};
