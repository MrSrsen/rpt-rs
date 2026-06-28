//! Database model (SDK: `IDatabase`, `ITable`, `ITableLink`, `IConnectionInfo`).

use super::enums::{ConnectionInfoKind, FieldValueType, TableJoinType};

/// SDK: `IDatabase`.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Database {
    pub tables: Vec<Table>,
    pub links: Vec<TableLink>,
}

/// SDK: `ITable` / `ICommandTable` (XML `<Table>`).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Table {
    pub name: String,
    pub alias: String,
    pub class_name: Option<String>,
    pub qualified_name: Option<String>,
    pub connection: ConnectionInfo,
    pub data_fields: Vec<DbFieldDef>,
    /// SDK `ICommandTable.CommandText` (XML `<Command>`).
    pub command_text: Option<String>,
}

/// A table's data field (the `<Field>` rows under `<Fields>`) — a thin DB-field descriptor.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct DbFieldDef {
    pub name: String,
    pub value_type: FieldValueType,
    pub length: i32,
    pub short_name: Option<String>,
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
#[non_exhaustive]
pub struct ConnectionInfo {
    pub user_name: Option<String>,
    pub kind: ConnectionInfoKind,
    /// SDK `Attributes` (PropertyBag) — the `QE_*` / `Database_DLL` / `SSO_Enabled` keys.
    pub attributes: Vec<(String, String)>,
}

/// SDK: `ITableLink` (XML `<TableLink>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct TableLink {
    pub join_type: TableJoinType,
    pub source_table_alias: String,
    pub target_table_alias: String,
    pub source_fields: Vec<String>,
    pub target_fields: Vec<String>,
}
