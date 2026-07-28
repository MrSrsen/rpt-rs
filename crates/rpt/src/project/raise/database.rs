//! Database — the `QESession` (Query Engine) stream: connections, tables, fields, links.

use super::*;

/// Project the `QESession` record tree into the [`Database`] model: tables (with their SQL
/// `Command` text, field schema, and each table's own connection info), connections, and links.
pub(super) fn raise_database(qe: &RecordStream) -> Database {
    let logical = qe.logical_bytes();
    let tree = qe.qe_record_tree();
    let mut db = Database::default();

    // The connection container (0x02) holds the driver/type/server (its own strings) plus the
    // logon-property child records, and it is the PARENT of the table records (0x03) it serves. A
    // report may have several connections (e.g. two Command tables under two distinct connections
    // with different databases), so each table takes the connection of its owning 0x02 — not one
    // shared connection.
    let conn_nodes = nodes_where(&tree, |n| n.rtype == QE_CONNECTION);
    let mut tables_with_conn: Vec<(&RecordNode, ConnectionInfo)> = Vec::new();
    for cn in &conn_nodes {
        let conn = raise_connection(cn, logical);
        cn.walk(&mut |t| {
            if t.rtype == QE_TABLE {
                tables_with_conn.push((t, conn.clone()));
            }
        });
    }

    // Each table record (0x03): its own strings are [name, alias, qualified, command-text…];
    // its 0x04 children are the data fields. While walking, index every field by its global id
    // (the leading u32 of a field record) so table links can resolve their endpoints.
    let mut field_index: BTreeMap<i32, (String, DbFieldDef)> = BTreeMap::new();
    // Tables paired with their stored order id (the `0x03` leaf's leading big-endian u32). Tables are
    // listed by that id, which is not always the stream's physical order, so collect then sort so the
    // emitted order matches the engine.
    let mut table_list: Vec<(u32, Table)> = Vec::new();
    for (n, connection) in &tables_with_conn {
        let order_id = u32_be(&n.leaf_bytes(logical), 0).unwrap_or(0);
        let strings = own_lp_strings(n, logical);
        let name = strings.first().cloned().unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        // The alias is a "decorated" form of the table name: the name itself or the name plus
        // an instance suffix of digits/underscores (e.g. `clients1`, `Command_2`). Among the
        // table's own strings, the last such match is the stored alias (a suffixed instance
        // wins over the bare name at index 0); everything else — the qualified name, server,
        // table-type word — is anchored elsewhere. The table and every field are qualified by
        // this alias, not the raw name.
        let alias = table_alias(&strings, &name);
        // The provider's qualified name is the table's *second* own string: the bare table name for
        // an unqualified table (`Customer`), the literal `Command` for a SQL-command table, or the
        // full `catalog.schema.table` when the provider qualified it (`rptfixtures.public.orders`).
        // It sits at index 1 because the name is repeated at index 0 and the qualified form follows
        // (the intervening NUL slots are dropped by `own_lp_strings`).
        let qualified_name = strings.get(1).filter(|s| !s.is_empty()).cloned();
        // Every SQL-command table is named `Command` / `Command_N`; any other name is a real
        // database table or view. The class is keyed on the name, not on detecting a SQL string.
        let is_command = name == "Command" || name.starts_with("Command_");
        // The command text is, structurally, the table's last own string (`[name, name, alias,
        // SQL]`) and by far the longest — so a command table's SQL is selected by length, with no
        // assumptions about its content (it may be a CTE, begin with a comment, call a stored
        // procedure, …). Non-command tables have no command text.
        let command_text = is_command.then(|| command_sql(n, logical)).flatten();
        let class_name = Some(if is_command {
            "CrystalReports.CommandTable".to_string()
        } else {
            "CrystalReports.Table".to_string()
        });
        let mut data_fields = Vec::new();
        for c in n.children.iter().filter(|c| c.rtype == QE_FIELD) {
            if let Some((id, mut field)) = raise_db_field(c, logical) {
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
            .filter(|c| c.rtype == QE_COMMAND_PARAM)
            .filter_map(|c| raise_command_param(c, logical))
            .collect();
        table_list.push((
            order_id,
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

    // Table links (0x0a) — root-level records of six big-endian u32s:
    // `[link_id][src_field_id][dst_field_id][operator][join_kind][w5]`. Resolve the field ids against
    // the index to recover the linked tables and fields. The stream sometimes stores links out of
    // order; they are emitted by ascending link_id, so collect then sort.
    //
    // The two predicate words are one-hot bit codes and are INDEPENDENT — the designer's Link Options
    // dialog sets outer-ness and comparison operator separately, and the file mirrors that split even
    // though the SDK's single `TableJoinType` cannot express both at once. Word 5 stays undecoded: it
    // is 1 on every link ever observed, including under a lookup-type change and a compound key (a
    // two-term join is stored as two `0x0a` records, so it is not a field-term count either).
    let mut raw_links: Vec<(i32, TableLink)> = Vec::new();
    for root in &tree {
        root.walk(&mut |n| {
            if n.rtype != QE_TABLE_LINK {
                return;
            }
            let b = n.leaf_bytes(logical);
            let be = |i: usize| i32_be(&b, i * 4);
            let (Some(link_id), Some(src_id), Some(dst_id), Some(operator), Some(join)) =
                (be(0), be(1), be(2), be(3), be(4))
            else {
                return;
            };
            let (Some((src_table, src_field)), Some((dst_table, dst_field))) =
                (field_index.get(&src_id), field_index.get(&dst_id))
            else {
                return;
            };
            raw_links.push((
                link_id,
                TableLink {
                    join_kind: TableJoinKind::from_code(join),
                    operator: TableLinkOperator::from_code(operator),
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

/// The table's stored alias among its own strings: the last string that is an instance-suffix
/// variant of the table name — one of the two is the other followed by a run of digits/underscores.
/// A base table's alias decorates the name (`clients` -> `clients1`); a SQL command table's name
/// decorates the alias (`Command_1` with alias `Command`). Anchored on the name so the qualified
/// name / server / type word never match. Falls back to `name`.
pub(super) fn table_alias(strings: &[String], name: &str) -> String {
    // The alias is the table name — or its space-sanitized form (Crystal replaces spaces with
    // underscores for a valid identifier: `Orders Detail` -> `Orders_Detail`) — optionally with an
    // instance suffix of digits/underscores (`clients` -> `clients1`).
    let sanitized = name.replace(' ', "_");
    let is_variant = |s: &str| {
        [name, sanitized.as_str()].iter().any(|base| {
            let (short, long) = if base.len() <= s.len() {
                (*base, s)
            } else {
                (s, *base)
            };
            long.strip_prefix(short)
                .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit() || c == '_'))
        })
    };
    // A *command table* stores its strings as `[name, name, alias, command-text…]`: the name (always
    // `Command` / `Command_N`) is repeated, then the user-facing alias (index 2), then the command
    // text. For a default (un-aliased) command the alias equals the name; for a renamed one it is a
    // user-assigned identifier not derivable from the name. The alias is taken straight from index 2
    // — the command text at index 3 is NOT gated through a SQL sniff, because stored procs / T-SQL /
    // brace-wrapped commands (`:DECLARE…`, `BDECLARE…`, `{SELECT…}`) do not read as plain `SELECT`
    // and would otherwise drop the alias back to the literal `Command`.
    let is_command = name == "Command" || name.starts_with("Command_");
    if is_command && strings.len() >= 3 && strings[0] == name && strings[1] == name {
        let alias = &strings[2];
        if !alias.is_empty() && alias.len() < 64 && !alias.chars().any(char::is_whitespace) {
            return alias.clone();
        }
    }
    // A fully-qualified base table stores its report *instance* name — the alias — as the last element
    // of a trailing `[database, schema, instance]` triple, alongside the fully-qualified
    // `database.schema.name` string (`strings[1]`). When a report self-joins one table under several
    // aliases, the bare `name` repeats across the instances, so only this instance string tells them
    // apart. When the qualified name is reconstructable from a consecutive `[db, schema, X]` triple
    // (`db.schema.name` == the qualified string), `X` is the alias — for an un-aliased table it simply
    // equals the name.
    if let Some(qualified) = strings.get(1) {
        if let Some(w) = strings
            .windows(3)
            .find(|w| &format!("{}.{}.{}", w[0], w[1], name) == qualified)
        {
            return w[2].clone();
        }
    }
    // Otherwise the alias is the bare name or a suffixed instance variant (`clients1`, `Command_2`).
    strings
        .iter()
        .rev()
        .find(|s| is_variant(s))
        .cloned()
        .unwrap_or_else(|| name.to_string())
}

/// Decode the obfuscated string value of a QE logon-property child: string variants store their
/// bytes XOR'd with `0x07`, preceded by an XOR'd copy of the property key. The actual value is the
/// last printable-after-XOR run in the blob (the key copy comes first, the value last).
pub(super) fn xor7_value(blob: &[u8]) -> String {
    // A run byte is text after de-XOR: not a control char and not DEL. High bytes (>= 0x80) are
    // kept so a localized (UTF-8) value isn't split — the run is decoded as UTF-8 below.
    let printable = |b: u8| {
        let c = b ^ 0x07;
        c >= 0x20 && c != 0x7f
    };
    let (mut best, mut i) = (None, 0);
    while i < blob.len() {
        let mut j = i;
        while j < blob.len() && printable(blob[j]) {
            j += 1;
        }
        if j - i >= 2 {
            best = Some((i, j));
        }
        i = if j > i { j } else { i + 1 };
    }
    best.map(|(s, e)| {
        let xored: Vec<u8> = blob[s..e].iter().map(|&b| b ^ 0x07).collect();
        String::from_utf8_lossy(&xored).into_owned()
    })
    .unwrap_or_default()
}

/// The SQL command text of a command table: the longest length-prefixed string in the table's own
/// leaf. Structurally the command is the table's last own string (`[name, name, alias, SQL]`) and is
/// far longer than the name/alias/qualified-name, so selecting by length is **content-agnostic** —
/// any SQL works (CTEs that start with `WITH`, bodies that begin with a `--`/`/* */` comment, stored
/// procedure calls). [`longest_lp`]'s sliding scan matters here: the command's framing can be
/// shadowed by a one-byte-early false match (when the command length's high bytes are zero) or
/// enveloped by an earlier mis-framed run — either of which would otherwise hide it.
pub(super) fn command_sql(node: &RecordNode, logical: &[u8]) -> Option<String> {
    longest_lp(&node.leaf_bytes(logical))
}

/// The three positional own-slots of a `0x02` connection leaf: `(Database_DLL, QE_DatabaseType,
/// QE_ServerDescription)`. The leaf is `[marker][DLL][type][server]`; the marker width varies, so
/// anchor on the driver DLL (the first slot whose text ends `.dll`) and read the next two slots
/// from there, keeping empties (the type slot is commonly empty — the engine then derives it).
///
/// Slots are read as length-prefixed lossy strings (an empty length-1 NUL slot is a valid value
/// here, not a skip), capped wide enough for a long server description.
fn qe_connection_slots(b: &[u8]) -> (String, String, String) {
    let slot = |off: usize| read_be_lp_string_lossy_at(b, off, 0x4000);
    let mut i = 0;
    while i + 4 <= b.len() {
        if let Some((s, next)) = slot(i) {
            if s.to_ascii_lowercase().ends_with(".dll") {
                let (db_type, n2) = slot(next).unwrap_or_default();
                let (server, _) = slot(n2).unwrap_or_default();
                return (s, db_type, server);
            }
        }
        i += 1;
    }
    (String::new(), String::new(), String::new())
}

/// A Crystal database-driver provider, identified by its `crdb_*.dll`. `QE_DatabaseType` is stored in
/// the connection record and used verbatim; this enum supplies the display name **only as a fallback**
/// for the rare empty-type slot, so an unknown driver still gets a sensible value.
///
/// The full provider set Crystal ships is:
/// `crdb_ado, crdb_adoplus, crdb_businessview, crdb_bwmdx, crdb_bwquery, crdb_dao, crdb_dataset,
/// crdb_dictionary, crdb_fielddef, crdb_infoset, crdb_jdbc, crdb_odbc, crdb_ods, crdb_olap,
/// crdb_opensql, crdb_orrapps, crdb_p2bbtrv, crdb_p2sdb2, crdb_p2sifmx, crdb_p2sodbc, crdb_p2sora,
/// crdb_p2soledb, crdb_p2ssql, crdb_p2ssyb, crdb_psenterprise, crdb_pseone, crdb_psqry, crdb_query,
/// crdb_siebel, crdb_universe, crdb_xml`. ODBC/ADO use known display names; JDBC/XML/field-definitions
/// use Crystal's documented `QE_DatabaseType` strings; any other DLL falls to
/// [`DatabaseDriver::Other`], whose display name is derived from the DLL stem.
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

/// Build the [`ConnectionInfo`] for one `QESession` connection container (0x02): its own strings
/// give the driver DLL / type / server, and its child records carry the logon properties.
pub(super) fn raise_connection(n: &RecordNode, logical: &[u8]) -> ConnectionInfo {
    let mut connection = ConnectionInfo::default();
    // The connection leaf is `[marker][Database_DLL, QE_DatabaseType, server]` — three positional
    // length-prefixed slots, any of which may be empty (a length-1 NUL). The QE_DatabaseType display
    // name ("ODBC (RDO)", …) is stored and used verbatim; only when that slot is empty does the
    // engine derive it from the driver DLL. Read positionally keeping empties (the general
    // `own_lp_strings` drops empty slots, which would collapse the indices).
    let (dll, stored_type, own_server) = qe_connection_slots(&n.leaf_bytes(logical));
    let db_type = if !stored_type.is_empty() {
        stored_type
    } else {
        DatabaseDriver::from_dll(&dll).display_name(&dll)
    };
    // The database name, server, and user come from the logon-property child records (one per
    // connection property): each is `[u32 index][LP key][LP display][variant value]`, and string
    // values are obfuscated by XOR with 0x07. Surfaced: `Database`/`Initial Catalog`
    // (→ QE_DatabaseName), `Server` (→ QE_ServerDescription) and `User ID` (→ the top-level
    // UserName); the rest form the COM logon bag (QE_LogonProperties).
    let (mut db_name, mut user) = (String::new(), String::new());
    let (mut initial_catalog, mut server_prop) = (String::new(), String::new());
    for child in &n.children {
        let cb = child.leaf_bytes(logical);
        let mut c = Cursor::at(&cb, 4);
        if let Some(key) = c.lp_string() {
            let val = &cb[c.pos()..];
            match key.as_str() {
                "Database" => db_name = xor7_value(val),
                "Initial Catalog" => initial_catalog = xor7_value(val),
                "Server" => server_prop = xor7_value(val),
                "User ID" => user = xor7_value(val),
                _ => {}
            }
        }
    }
    // ODBC connections store the database under `Database`; OLE DB (ADO) providers (e.g. SQLOLEDB)
    // store it under `Initial Catalog` instead. Prefer the explicit `Database` when present.
    if db_name.is_empty() {
        db_name = initial_catalog;
    }
    // QE_ServerDescription is the clean host (or `host:port`). The discrete `Server` logon property
    // carries exactly that; the connection's own last string is the raw connection string for HANA,
    // so prefer the `Server` property and fall back to the own string (`.` for local OLE DB).
    let server = if server_prop.is_empty() {
        own_server
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

/// A `QESession` command-parameter record (0x07), a child of a command/stored-proc table (0x03):
/// `[u32-BE id][lp-string name][lp-string heading][u32-BE flags=1][u32-BE value-type]…`. The name
/// is the bind's stored spelling (`cardCode`, `$[BOY_AB_FROMDATE]`); the value-type is the
/// `CrFieldValueTypeEnum` code (`6`=Number, `9`=Date, `11`=String, …). The heading/prompt slot is a
/// length-prefixed string — a 5-byte null placeholder (`00 00 00 01 00`) when the parameter has no
/// heading, exactly as the field record's description slot — and is skipped over to reach the type.
/// The trailing default/current-value payload (report-instance data) is intentionally not decoded.
pub(super) fn raise_command_param(node: &RecordNode, logical: &[u8]) -> Option<CommandParameter> {
    let bytes = node.leaf_bytes(logical);
    let mut c = Cursor::new(&bytes);
    c.u32_be()?; // parameter id (unused)
    let name = c.lp_string()?;
    // The heading/prompt slot: a real lp-string advances the cursor; an empty placeholder
    // (`00 00 00 01 00`) fails `lp_string` validation, so skip its fixed 5 bytes.
    if c.lp_string().is_none() {
        c.skip(5);
    }
    c.u32_be()?; // flags (== 1)
    let value_type = c
        .u32_be()
        .map(|v| FieldValueType::from_code(v as i32))
        .unwrap_or_default();
    Some(CommandParameter { name, value_type })
}

/// A `QESession` field record (0x04): `[u32 id][lp-string name][u32 flags][u32][u32-le
/// value-type][u32-le length]`. The id (big-endian, the table-link reference key) and name
/// length are big-endian; the trailing value-type and length scalars are little-endian. Returns
/// the field's global id alongside the field.
pub(super) fn raise_db_field(node: &RecordNode, logical: &[u8]) -> Option<(i32, DbFieldDef)> {
    let bytes = node.leaf_bytes(logical);
    let mut c = Cursor::new(&bytes);
    let id = c.u32_be()? as i32;
    let name = c.lp_string()?;
    // After the name comes a description slot (a length-prefixed string), then 3 zero padding
    // bytes, then the 1-byte value-type code and the field's byte length as a big-endian u32.
    // The slot is *always present*: when the field has no description it is a 5-byte null
    // placeholder (`00 00 00 01 00`) whose single NUL content byte fails `lp_string`'s
    // validation — so the fallback skips those fixed 5 bytes. A real description is a normal
    // printable lp-string and advances the cursor accordingly.
    let description = c.lp_string();
    if description.is_none() {
        c.skip(5);
    }
    c.skip(3);
    let value_type = c
        .u8()
        .map(|v| FieldValueType::from_code(i32::from(v)))
        .unwrap_or_default();
    // The field's column size follows the value-type code. Normally it is a single big-endian u32.
    // A MAX/unlimited column (a `(N)VARCHAR(MAX)` / large-object mapping) instead prefixes it with a
    // 4-byte "unlimited" descriptor whose high word is 0xFFFF (observed `FF FF 00 00`, the SQL MAX
    // convention), and the engine's fallback capacity (65536 wide chars) follows. A high word of
    // 0xFFFF can never be a valid byte count, so it unambiguously flags the variant. This fires on
    // `nvarchar(max)` String columns and on wide `Memo` columns; for Memo the length is overridden
    // to -1 below, so the skip only affects the String case.
    if u16_be(&bytes, c.pos()) == Some(0xffff) {
        c.skip(4);
    }
    let stored_length = c.u32_be().map(|v| v as i32).unwrap_or_default();
    // The empty `0x0000` child record marks a **wide** (2-byte-per-char, `nvarchar`/Unicode) column.
    // Its stored value is the character capacity plus one (a null-terminator slot), so the byte
    // length is `(stored - 1) * 2`. A single-byte (`varchar`) column has no such child and stores
    // its byte length directly. (This distinction, not the value-type, is what separates wide from
    // narrow String columns; Memo columns also carry the marker but take their fixed -1 length.)
    let stored_length = if value_type == FieldValueType::String
        && node.children.iter().any(|c| c.rtype == QE_EMPTY_MARKER)
    {
        stored_length.saturating_sub(1).saturating_mul(2)
    } else {
        stored_length
    };
    let length = value_type.byte_length().unwrap_or(stored_length);
    Some((
        id,
        DbFieldDef {
            long_name: Some(name.clone()),
            short_name: Some(name.clone()),
            name,
            value_type,
            length,
            description,
        },
    ))
}
