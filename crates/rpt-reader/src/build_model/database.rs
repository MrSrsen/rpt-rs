//! Database — the `QESession` (Query Engine) stream: connections, tables, fields, links.

use super::row_of;
use super::tree_search::nodes_where;
use crate::codec::{Dialect, RecordNode};
use crate::field_table::table::{Cell, Row};
use crate::field_table::tables as ft;
use crate::model::{
    CommandParameter, ConnectionInfo, Database, DbFieldDef, FieldValueType, Table, TableJoinKind,
    TableLink, TableLinkOperator,
};
use crate::records::RecordStream;
use std::collections::BTreeMap;

/// Project the `QESession` record tree into the [`Database`] model: tables (with their SQL
/// `Command` text, field schema, and each table's own connection info), connections, and links.
///
/// The record numbers below are the query engine's own — `0x0003` is a table here and an unrelated
/// report-definition record in `Contents` — so the tree is taken only where the stream is written in
/// that vocabulary.
pub(super) fn build_database(qe: &RecordStream) -> Database {
    let logical = qe.logical_bytes();
    let tree = qe.record_tree_in(Dialect::QeSession);
    let mut db = Database::default();

    // The connection container (0x02) holds the driver/type/server (its own strings) plus the
    // logon-property child records, and it is the PARENT of the table records (0x03) it serves. A
    // report may have several connections (e.g. two Command tables under two distinct connections
    // with different databases), so each table takes the connection of its owning 0x02 — not one
    // shared connection.
    let conn_nodes = nodes_where(&tree, |n| n.rtype == ft::QE_CONNECTION.rtype);
    let mut tables_with_conn: Vec<(&RecordNode, ConnectionInfo)> = Vec::new();
    for cn in &conn_nodes {
        let conn = build_connection(cn, logical);
        cn.walk(&mut |t| {
            if t.rtype == ft::QE_TABLE.rtype {
                tables_with_conn.push((t, conn.clone()));
            }
        });
    }

    // Each table record (0x03) states its own identity strings, then its `0x04` field children.
    // While walking, index every field by its global id (the leading u32 of a field record) so
    // table links can resolve their endpoints.
    let mut field_index: BTreeMap<i32, (String, DbFieldDef)> = BTreeMap::new();
    // Tables paired with their stored order id. Tables are listed by that id, which is not always
    // the stream's physical order, so collect then sort so the emitted order matches the engine.
    let mut table_list: Vec<(u32, Table)> = Vec::new();
    for (n, connection) in &tables_with_conn {
        let row = row_of(n, logical, &ft::QE_TABLE);
        let name = row.text("name").to_owned();
        if name.is_empty() {
            continue;
        }
        // The table and every field are qualified by the stored alias, not the raw name: a report
        // that self-joins one table under several aliases repeats `name` across the instances, and
        // only the alias tells them apart.
        let alias = row.text("alias").to_owned();
        // The provider's qualified name: the bare table name for an unqualified table (`Customer`),
        // the literal `Command` for a SQL-command table, or the full `catalog.schema.table` when the
        // provider qualified it.
        let qualified_name = Some(row.text("qualified_name").to_owned()).filter(|s| !s.is_empty());
        // Every SQL-command table is named `Command` / `Command_N`; any other name is a real
        // database table or view. The class is keyed on the name, not on detecting a SQL string.
        let is_command = name == "Command" || name.starts_with("Command_");
        // The command text is a stated field, empty for a plain database table — so any SQL works
        // (a CTE, a body opening with a comment, a stored-procedure call) with no content sniff.
        let command_text = Some(row.text("command_text").to_owned()).filter(|s| !s.is_empty());
        let class_name = Some(if is_command {
            "CrystalReports.CommandTable".to_string()
        } else {
            "CrystalReports.Table".to_string()
        });
        let mut data_fields = Vec::new();
        for c in n.children.iter().filter(|c| c.rtype == ft::QE_FIELD.rtype) {
            if let Some((id, mut field)) = build_db_field(c, logical) {
                // The long/short names: `Alias.field` and `field`.
                field.long_name = Some(format!("{alias}.{}", field.name));
                field.short_name = Some(field.name.clone());
                field_index.insert(id, (alias.clone(), field.clone()));
                data_fields.push(field);
            }
        }
        // The command's bind parameters are the table's `0x07` children (a command/stored-proc
        // table only; a plain database table has none).
        let parameters = n
            .children
            .iter()
            .filter(|c| c.rtype == ft::QE_COMMAND_PARAMETER.rtype)
            .filter_map(|c| build_command_param(c, logical))
            .collect();
        table_list.push((
            row.u("table_id"),
            Table {
                alias,
                class_name,
                connection: connection.clone(),
                data_fields,
                command_text,
                parameters,
                name,
                qualified_name,
            },
        ));
    }
    // Emit tables in the engine's order (ascending stored order id), not the stream's physical order.
    table_list.sort_by_key(|(id, _)| *id);
    db.tables = table_list.into_iter().map(|(_, t)| t).collect();

    // Table links (0x0a): resolve the field ids against the index to recover the linked tables and
    // fields. The stream sometimes stores links out of order; they are emitted by ascending
    // link_id, so collect then sort.
    //
    // The two predicate words are one-hot bit codes and are INDEPENDENT — the designer's Link Options
    // dialog sets outer-ness and comparison operator separately, and the file mirrors that split even
    // though the SDK's single `TableJoinType` cannot express both at once.
    let mut raw_links: Vec<(i32, TableLink)> = Vec::new();
    for root in &tree {
        root.walk(&mut |n| {
            if n.rtype != ft::QE_TABLE_LINK.rtype {
                return;
            }
            let row = row_of(n, logical, &ft::QE_TABLE_LINK);
            let (Some((src_table, src_field)), Some((dst_table, dst_field))) = (
                field_index.get(&(row.u("source_field_id") as i32)),
                field_index.get(&(row.u("target_field_id") as i32)),
            ) else {
                return;
            };
            raw_links.push((
                row.u("link_id") as i32,
                TableLink {
                    join_kind: TableJoinKind::from_code(row.u("join_kind") as i32),
                    operator: TableLinkOperator::from_code(row.u("operator") as i32),
                    source_table_alias: src_table.clone(),
                    target_table_alias: dst_table.clone(),
                    source_fields: vec![src_field.name.clone()],
                    target_fields: vec![dst_field.name.clone()],
                },
            ));
        });
    }
    raw_links.sort_by_key(|(id, _)| *id);
    // A join between two tables is one <TableLink> carrying the full (possibly compound) key, whereas
    // the QE stream stores one `0x0a` record per field-pair. Fold consecutive records (in link_id /
    // emit order) that share the same source table, target table and predicate into a single link,
    // concatenating their fields.
    let mut links: Vec<TableLink> = Vec::new();
    for (_, link) in raw_links {
        match links.last_mut() {
            Some(last)
                if last.source_table_alias == link.source_table_alias
                    && last.target_table_alias == link.target_table_alias
                    && last.join_kind == link.join_kind
                    && last.operator == link.operator =>
            {
                last.source_fields.extend(link.source_fields);
                last.target_fields.extend(link.target_fields);
            }
            _ => links.push(link),
        }
    }
    db.links = links;

    db
}

/// Decode the obfuscated string value of a QE logon property: string variants store their bytes
/// XOR'd with `0x07`, preceded by an XOR'd copy of the property key. The actual value is the last
/// printable-after-XOR run in the block (the key copy comes first, the value last).
pub(super) fn xor7_value(blob: &[u8]) -> String {
    /// The byte a QE logon property's stored string bytes are XOR'd with.
    const XOR_KEY: u8 = 0x07;
    /// The shortest run read as a value: one printable byte on its own is as likely to be a stray
    /// byte of the block around it.
    const MIN_RUN: usize = 2;

    // A run byte is text after de-XOR: not a control char and not DEL. High bytes (>= 0x80) are
    // kept so a localized (UTF-8) value isn't split — the run is decoded as UTF-8 below.
    let printable = |b: u8| {
        let c = b ^ XOR_KEY;
        c >= 0x20 && c != 0x7f
    };
    let (mut best, mut i) = (None, 0);
    while i < blob.len() {
        let mut j = i;
        while j < blob.len() && printable(blob[j]) {
            j += 1;
        }
        if j - i >= MIN_RUN {
            best = Some((i, j));
        }
        i = if j > i { j } else { i + 1 };
    }
    best.map(|(s, e)| {
        let xored: Vec<u8> = blob[s..e].iter().map(|&b| b ^ XOR_KEY).collect();
        String::from_utf8_lossy(&xored).into_owned()
    })
    .unwrap_or_default()
}

/// A Crystal database-driver provider, identified by its `crdb_*.dll`. `QE_DatabaseType` is stored in
/// the connection record and used verbatim; this enum supplies the display name **only as a fallback**
/// for the rare empty-type slot, so an unknown driver still gets a sensible value. A handful of known
/// DLLs get their documented `QE_DatabaseType` name; any other DLL falls to [`DatabaseDriver::Other`],
/// whose display name is derived from the DLL stem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DatabaseDriver {
    Odbc,
    OleDbAdo,
    Jdbc,
    Xml,
    FieldDefinitions,
    Other,
}

impl DatabaseDriver {
    /// Classify a connection's `Database_DLL` (e.g. `crdb_odbc.dll`) by its stem.
    pub(super) fn from_dll(dll: &str) -> Self {
        let stem = dll
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(dll)
            .trim_end_matches(".dll")
            .trim_end_matches(".DLL")
            .to_ascii_lowercase();
        match stem.as_str() {
            "crdb_odbc" | "crdb_p2sodbc" => Self::Odbc,
            "crdb_ado" => Self::OleDbAdo,
            "crdb_jdbc" => Self::Jdbc,
            "crdb_xml" => Self::Xml,
            "crdb_fielddef" => Self::FieldDefinitions,
            _ => Self::Other,
        }
    }

    /// The `QE_DatabaseType` display name (fallback only — the stored value is authoritative).
    pub(super) fn display_name(self, dll: &str) -> String {
        match self {
            Self::Odbc => "ODBC (RDO)".to_string(),
            Self::OleDbAdo => "OLE DB (ADO)".to_string(),
            // Crystal's documented QE_DatabaseType names.
            Self::Jdbc => "JDBC (JNDI)".to_string(),
            Self::Xml => "XML and Web Services".to_string(),
            Self::FieldDefinitions => "Field Definitions Only".to_string(),
            // Unknown provider: a readable name derived from the DLL stem (`crdb_p2ssql.dll` →
            // `p2ssql`). Reached only when the stored type slot is empty *and* the driver is outside
            // the known set, so it never overrides a real stored value.
            Self::Other => dll
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(dll)
                .trim_end_matches(".dll")
                .trim_end_matches(".DLL")
                .trim_start_matches("crdb_")
                .to_string(),
        }
    }
}

/// The value block of a logon property, as stored (obfuscated).
fn property_value(row: &Row) -> &[u8] {
    match row.get("_value") {
        Some(Cell::Str { block, .. }) => block,
        _ => &[],
    }
}

/// Build the [`ConnectionInfo`] for one `QESession` connection container (0x02): its own strings
/// give the driver DLL / type / server, and its child records carry the logon properties.
pub(super) fn build_connection(n: &RecordNode, logical: &[u8]) -> ConnectionInfo {
    let mut connection = ConnectionInfo::default();
    // The connection's first three strings are the driver DLL, the QE_DatabaseType display name
    // ("ODBC (RDO)", …) and the server description. The type is stored and used verbatim; only when
    // that slot is empty does the engine derive it from the driver DLL.
    let row = row_of(n, logical, &ft::QE_CONNECTION);
    let dll = row.text("driver_dll").to_owned();
    let stored_type = row.text("database_type");
    let db_type = if stored_type.is_empty() {
        DatabaseDriver::from_dll(&dll).display_name(&dll)
    } else {
        stored_type.to_owned()
    };
    // The database name, server, and user come from the logon-property child records (0x09), one
    // per connection property; string values are obfuscated by XOR with 0x07. Surfaced:
    // `Database`/`Initial Catalog` (→ QE_DatabaseName), `Server` (→ QE_ServerDescription) and
    // `User ID` (→ the top-level UserName); the rest form the COM logon bag (QE_LogonProperties),
    // and no credential-carrying property is read.
    let (mut db_name, mut user) = (String::new(), String::new());
    let (mut initial_catalog, mut server_prop) = (String::new(), String::new());
    for child in n
        .children
        .iter()
        .filter(|c| c.rtype == ft::QE_LOGON_PROPERTY.rtype)
    {
        let property = row_of(child, logical, &ft::QE_LOGON_PROPERTY);
        match property.text("key") {
            "Database" => db_name = xor7_value(property_value(&property)),
            "Initial Catalog" => initial_catalog = xor7_value(property_value(&property)),
            "Server" => server_prop = xor7_value(property_value(&property)),
            "User ID" => user = xor7_value(property_value(&property)),
            _ => {}
        }
    }
    // ODBC connections store the database under `Database`; OLE DB (ADO) providers (e.g. SQLOLEDB)
    // store it under `Initial Catalog` instead. Prefer the explicit `Database` when present.
    if db_name.is_empty() {
        db_name = initial_catalog;
    }
    // QE_ServerDescription is the clean host (or `host:port`). The discrete `Server` logon property
    // carries exactly that; the connection's own string is the raw connection string for HANA, so
    // prefer the `Server` property and fall back to the stored one (`.` for local OLE DB).
    let server = if server_prop.is_empty() {
        row.text("server").to_owned()
    } else {
        server_prop
    };
    connection.user_name = (!user.is_empty()).then_some(user);
    // Attribute order; QE_LogonProperties is the (unserializable) COM object, and QE_SQLDB/SSO_Enabled
    // are constants. (UserName + Password are appended by the emitter from the top-level properties.)
    connection.attributes = vec![
        ("Database_DLL".into(), dll.clone()),
        ("QE_DatabaseName".into(), db_name),
        ("QE_DatabaseType".into(), db_type),
        ("QE_LogonProperties".into(), "System.__ComObject".into()),
        ("QE_ServerDescription".into(), server),
        ("QE_SQLDB".into(), "True".into()),
        ("SSO_Enabled".into(), "False".into()),
    ];
    connection
}

/// A `QESession` command-parameter record (0x07), a child of a command/stored-proc table (0x03).
/// The name is the bind's stored spelling (`cardCode`, `$[FromDate]`); the value type is the
/// `CrFieldValueTypeEnum` code (`6`=Number, `9`=Date, `11`=String, …). The default/current value
/// blocks are report-instance data and are intentionally not decoded.
pub(super) fn build_command_param(node: &RecordNode, logical: &[u8]) -> Option<CommandParameter> {
    let row = row_of(node, logical, &ft::QE_COMMAND_PARAMETER);
    let name = row.text("name").to_owned();
    if name.is_empty() {
        return None;
    }
    Some(CommandParameter {
        name,
        value_type: FieldValueType::from_code(row.u("value_type") as i32),
    })
}

/// A `QESession` field record (0x04): the column's id (the table-link reference key), name,
/// description, value type and stored byte length. Returns the field's global id alongside the
/// field.
///
/// The stored length is the column's **byte** count, wide columns included — an `nvarchar(20)`
/// stores 42, not 20 — so it needs no per-type arithmetic. An unlimited column
/// (`(n)varchar(max)`, a large object) stores a saturated length; for the value types that reach
/// this path the type's own fixed width takes precedence anyway.
pub(super) fn build_db_field(node: &RecordNode, logical: &[u8]) -> Option<(i32, DbFieldDef)> {
    let row = row_of(node, logical, &ft::QE_FIELD);
    let name = row.text("name").to_owned();
    if name.is_empty() {
        return None;
    }
    let value_type = FieldValueType::from_code(row.u("value_type") as i32);
    let stored_length = row.u("length") as i32;
    Some((
        row.u("field_id") as i32,
        DbFieldDef {
            long_name: Some(name.clone()),
            short_name: Some(name.clone()),
            name,
            value_type,
            length: value_type.byte_length().unwrap_or(stored_length),
            description: Some(row.text("description").to_owned()).filter(|s| !s.is_empty()),
        },
    ))
}
