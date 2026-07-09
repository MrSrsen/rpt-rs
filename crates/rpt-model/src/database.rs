//! Database model (SDK: `IDatabase`, `ITable`, `ITableLink`, `IConnectionInfo`).

use super::enums::{ConnectionInfoKind, FieldValueType, TableJoinType};

/// SDK: `IDatabase`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Database {
    /// The report's data tables (SDK `Database.Tables`).
    pub tables: Vec<Table>,
    /// The join links between those tables (SDK `Database.Links`).
    pub links: Vec<TableLink>,
}

/// SDK: `ITable` / `ICommandTable` (XML `<Table>`).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Table {
    /// The table's original database name (SDK `Table.Name`).
    pub name: String,
    /// The alias the report refers to the table by (SDK `Table.Alias`).
    pub alias: String,
    /// The provider class name of the table (SDK `Table.ClassName`).
    pub class_name: Option<String>,
    /// The fully-qualified `catalog.schema.table` name, when the provider supplies one.
    pub qualified_name: Option<String>,
    /// The connection this table is read through (SDK `Table.ConnectionInfo`).
    pub connection: ConnectionInfo,
    /// The table's columns (SDK `Table.Fields`).
    pub data_fields: Vec<DbFieldDef>,
    /// SDK `ICommandTable.CommandText` (XML `<Command>`).
    pub command_text: Option<String>,
}

/// A table's data field (the `<Field>` rows under `<Fields>`) — a thin DB-field descriptor.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DbFieldDef {
    /// The field's database column name (SDK `DatabaseFieldDefinition.Name`).
    pub name: String,
    /// The field's declared value type.
    pub value_type: FieldValueType,
    /// The field's storage length in bytes, as reported by the provider.
    pub length: i32,
    /// The provider's short (unqualified) field name, when distinct from `name`.
    pub short_name: Option<String>,
    /// The provider's fully-qualified field name, when distinct from `name`.
    pub long_name: Option<String>,
    /// The field's description/heading text (SDK `Field.Description`), when the QE field record
    /// carries one (fields without a description store a null placeholder in its place).
    pub description: Option<String>,
}

/// SDK: `IConnectionInfo` (XML `<ConnectionInfo>`).
///
/// **The password is intentionally not retained** (SDK `Password`) — the records may carry it,
/// but it is never surfaced in the model.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ConnectionInfo {
    /// The login user name (SDK `ConnectionInfo.UserID`); the password is never retained.
    pub user_name: Option<String>,
    /// How the connection is established (native / ODBC / OLE DB / …).
    pub kind: ConnectionInfoKind,
    /// SDK `Attributes` (PropertyBag) — the `QE_*` / `Database_DLL` / `SSO_Enabled` keys.
    pub attributes: Vec<(String, String)>,
}

/// SDK: `ITableLink` (XML `<TableLink>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TableLink {
    /// The SQL join type used to link the two tables (inner / left / …).
    pub join_type: TableJoinType,
    /// Alias of the table on the "from" side of the link.
    pub source_table_alias: String,
    /// Alias of the table on the "to" side of the link.
    pub target_table_alias: String,
    /// The source table's join fields, paired positionally with `target_fields`.
    pub source_fields: Vec<String>,
    /// The target table's join fields, paired positionally with `source_fields`.
    pub target_fields: Vec<String>,
}
