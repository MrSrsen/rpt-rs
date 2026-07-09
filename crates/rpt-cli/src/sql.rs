//! `sql` — every SQL statement a report can issue against the database, with provenance.
//!
//! A report talks to the database in three ways, and this command surfaces all of them, tagged with
//! where in the report each came from:
//!
//! * the **generated join query** the engine builds from the table/link graph (report-aware: only
//!   the referenced tables/columns, with the translatable part of the record-selection formula
//!   pushed into `WHERE`) — via `rpt-query`;
//! * each **stored SQL Command** (`Table.CommandText`), the author's hand-written query, emitted
//!   verbatim; and
//! * each **SQL Expression field**, a raw fragment the database evaluates.
//!
//! Subreports are walked recursively — each issues its own queries. It also summarises the
//! connections (server / database / driver) and the table list for a quick "what does this touch"
//! view. Read-only; no database connection is made — this is the SQL the report *would* run.

use rpt::model::{ConnectionInfo, Report, Table};
use rpt::Rpt;
use rpt_query::{build_query_for_report, Dialect};
use serde::Serialize;

use crate::util::{paint, print_json, BOLD, BOLD_GREEN, CYAN, DIM, YELLOW};

pub(crate) const HELP: &str = "\
rpt sql — the SQL a report can run against its database

Lists every statement the report would issue — the generated join SELECT (built from the table/link
graph, pruned to referenced tables and with the selection formula pushed into WHERE), each stored
SQL Command, and each SQL Expression field — tagged with where it came from. Subreports are included
recursively. Also summarises the connections and tables. No database connection is made.

Section headers, table names, and each statement's source are highlighted; scaffolding (indices,
separators, field labels) is dimmed. Color is on by default at a terminal and off when piped;
--color / --no-color (and NO_COLOR / CLICOLOR_FORCE) override that.

USAGE:
    rpt sql <file.rpt> [--dialect <d>] [--json] [--color | --no-color]

OPTIONS:
    --dialect <d>   SQL dialect for the generated query: postgres (default), sqlite, mysql
    --json          emit as JSON
    --color         force coloring on even when piped (e.g. `rpt sql f.rpt --color | less -R`)
    --no-color      force coloring off
";

/// The kind of SQL a query entry represents (its provenance category).
#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
enum QueryKind {
    /// The engine-generated join `SELECT` (from tables + links + selection push-down).
    Generated,
    /// A stored SQL Command (`Table.CommandText`), the author's hand-written query.
    Command,
    /// A SQL Expression field — a raw fragment the database evaluates.
    SqlExpression,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryJson {
    /// Where this SQL comes from (e.g. `"Main data query"`, `"Command table: sales"`,
    /// `"Subreport 'Details' › SQL Expression field: TaxRate"`).
    source: String,
    kind: QueryKind,
    /// The dialect the SQL was rendered for — only meaningful for a generated query.
    #[serde(skip_serializing_if = "Option::is_none")]
    dialect: Option<String>,
    /// The tables the generated query reads (its projected aliases); empty for the other kinds.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tables: Vec<String>,
    sql: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionJson {
    #[serde(skip_serializing_if = "Option::is_none")]
    server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    database: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    database_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    driver: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    kind: String,
    /// The tables read through this connection, as `alias` (main) or `alias @ Subreport`.
    tables: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TableJson {
    alias: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    qualified_name: Option<String>,
    /// `"command"` for a stored SQL Command table, else `"table"`.
    kind: &'static str,
    /// The subreport this table lives in, or `None` for the main report.
    #[serde(skip_serializing_if = "Option::is_none")]
    subreport: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SqlReport<'a> {
    file: &'a str,
    connections: Vec<ConnectionJson>,
    tables: Vec<TableJson>,
    queries: Vec<QueryJson>,
}

/// A connection attribute value, or `None` when absent/empty. The connection's server/database/type
/// are carried in the `Attributes` property bag (`QE_*` / `Database_DLL` keys).
fn attr<'a>(conn: &'a ConnectionInfo, key: &str) -> Option<&'a str> {
    conn.attributes
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .filter(|s| !s.is_empty())
}

/// Whether a table is a stored SQL Command (author-written raw SQL) rather than a plain table.
fn is_command(t: &Table) -> bool {
    matches!(&t.command_text, Some(c) if !c.trim().is_empty())
}

/// A short label for a table within the report tree: `alias` in the main report, `alias @ <sub>` in
/// a subreport.
fn table_label(alias: &str, subreport: Option<&str>) -> String {
    match subreport {
        Some(s) => format!("{alias} @ {s}"),
        None => alias.to_string(),
    }
}

/// Provenance prefix for a subreport's entries (`"Subreport 'Name' › "`), empty for the main report.
fn source_prefix(subreport: Option<&str>) -> String {
    match subreport {
        Some(s) => format!("Subreport '{s}' › "),
        None => String::new(),
    }
}

/// The distinct table aliases a generated query projects, in first-seen order — the tables it reads.
fn query_tables(q: &rpt_query::SqlQuery) -> Vec<String> {
    let mut seen = Vec::new();
    for c in &q.columns {
        if !c.alias.is_empty() && !seen.contains(&c.alias) {
            seen.push(c.alias.clone());
        }
    }
    seen
}

/// Collect the connection + table summary and every SQL statement for one report level, recursing
/// into subreports. `subreport` is the current level's name (`None` at the main report).
fn collect(
    report: &Report,
    subreport: Option<&str>,
    dialect: Dialect,
    conns: &mut Vec<ConnectionJson>,
    tables: &mut Vec<TableJson>,
    queries: &mut Vec<QueryJson>,
) {
    let prefix = source_prefix(subreport);

    // Tables + connection membership.
    for t in &report.database.tables {
        tables.push(TableJson {
            alias: t.alias.clone(),
            name: t.name.clone(),
            qualified_name: t.qualified_name.clone(),
            kind: if is_command(t) { "command" } else { "table" },
            subreport: subreport.map(str::to_string),
        });
        record_connection(&t.connection, &table_label(&t.alias, subreport), conns);
    }

    // The engine-generated join query (report-aware / pruned, with selection push-down). SQL
    // Expression fields are projected into it; the record-selection formula drives its WHERE.
    let sql_exprs: Vec<(String, String)> = report
        .data_definition
        .sql_expression_fields()
        .map(|(fd, x)| (fd.name.clone(), x.text.clone()))
        .collect();
    let selection = report
        .data_definition
        .record_selection
        .as_ref()
        .map(|f| f.0.as_str());
    if let Some(q) = build_query_for_report(report, &sql_exprs, selection, &[], dialect) {
        queries.push(QueryJson {
            source: format!("{prefix}Main data query"),
            kind: QueryKind::Generated,
            dialect: Some(dialect_name(dialect).to_string()),
            tables: query_tables(&q),
            sql: q.sql,
        });
    }

    // Stored SQL Commands — the author's verbatim queries. (Each is also embedded inside the
    // generated query above as `FROM (<sql>) AS alias`; surfaced separately here as the raw source.)
    for t in &report.database.tables {
        if let Some(cmd) = &t.command_text {
            if !cmd.trim().is_empty() {
                queries.push(QueryJson {
                    source: format!("{prefix}Command table: {}", t.alias),
                    kind: QueryKind::Command,
                    dialect: None,
                    tables: Vec::new(),
                    sql: cmd.clone(),
                });
            }
        }
    }

    // SQL Expression fields — raw fragments the database evaluates.
    for (fd, x) in report.data_definition.sql_expression_fields() {
        queries.push(QueryJson {
            source: format!("{prefix}SQL Expression field: {}", fd.name),
            kind: QueryKind::SqlExpression,
            dialect: None,
            tables: Vec::new(),
            sql: x.text.clone(),
        });
    }

    for sub in &report.subreports {
        collect(
            &sub.report,
            Some(&sub.name),
            dialect,
            conns,
            tables,
            queries,
        );
    }
}

/// Merge a table's connection into `conns`, deduping identical connections and appending the table
/// label to the matching entry's table list.
fn record_connection(conn: &ConnectionInfo, table_label: &str, conns: &mut Vec<ConnectionJson>) {
    let server = attr(conn, "QE_ServerDescription").map(str::to_string);
    let database = attr(conn, "QE_DatabaseName").map(str::to_string);
    let database_type = attr(conn, "QE_DatabaseType").map(str::to_string);
    let driver = attr(conn, "Database_DLL").map(str::to_string);
    let user = conn.user_name.clone().filter(|s| !s.is_empty());
    let kind = format!("{:?}", conn.kind);

    if let Some(existing) = conns.iter_mut().find(|c| {
        c.server == server
            && c.database == database
            && c.database_type == database_type
            && c.driver == driver
            && c.user == user
            && c.kind == kind
    }) {
        if !existing.tables.iter().any(|t| t == table_label) {
            existing.tables.push(table_label.to_string());
        }
        return;
    }
    conns.push(ConnectionJson {
        server,
        database,
        database_type,
        driver,
        user,
        kind,
        tables: vec![table_label.to_string()],
    });
}

fn dialect_name(d: Dialect) -> &'static str {
    match d {
        Dialect::Postgres => "postgres",
        Dialect::Sqlite => "sqlite",
        Dialect::Mysql => "mysql",
    }
}

/// Parse the `--dialect` value; `None`/unset defaults to Postgres. Returns an error for an
/// unrecognised name.
pub(crate) fn parse_dialect(name: Option<&str>) -> rpt::Result<Dialect> {
    match name.map(str::to_ascii_lowercase).as_deref() {
        None | Some("postgres") | Some("postgresql") | Some("pg") => Ok(Dialect::Postgres),
        Some("sqlite") => Ok(Dialect::Sqlite),
        Some("mysql") | Some("mariadb") => Ok(Dialect::Mysql),
        Some(other) => Err(rpt::Error::from(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("unknown --dialect '{other}' (expected postgres, sqlite, or mysql)"),
        ))),
    }
}

pub(crate) fn sql(file: &str, json: bool, dialect: Dialect, color: bool) -> rpt::Result<()> {
    let rpt = Rpt::open(file)?;
    let report = rpt.report();

    let mut conns = Vec::new();
    let mut tables = Vec::new();
    let mut queries = Vec::new();
    collect(report, None, dialect, &mut conns, &mut tables, &mut queries);

    if json {
        print_json(&SqlReport {
            file,
            connections: conns,
            tables,
            queries,
        });
        return Ok(());
    }

    print_text(file, &conns, &tables, &queries, color);
    Ok(())
}

/// The human-readable rendering: connections, then tables, then each SQL statement with its source.
///
/// Coloring follows the `tree` palette — bold section headers, prominent provenance/table names, and
/// dimmed scaffolding (indices, separators, `key =` labels) so the eye lands on the structure. The
/// SQL bodies themselves stay in the default color: there is a lot of it, and it is the most
/// readable left plain.
fn print_text(
    file: &str,
    conns: &[ConnectionJson],
    tables: &[TableJson],
    queries: &[QueryJson],
    color: bool,
) {
    let dim = |s: &str| paint(color, DIM, s);
    println!("{}\n", paint(color, BOLD, file));

    println!(
        "{}",
        paint(color, BOLD, &format!("Connections ({})", conns.len()))
    );
    if conns.is_empty() {
        println!(
            "  {}",
            dim("(none — the report references no database tables)")
        );
    }
    for c in conns {
        let driver = c.driver.as_deref().unwrap_or("?");
        println!(
            "  {} {} {} {}",
            paint(color, CYAN, "●"),
            paint(color, CYAN, &c.kind),
            dim("·"),
            paint(color, CYAN, driver),
        );
        let field = |label: &str, value: &str| {
            println!(
                "      {} {}",
                dim(&format!("{label:<9}=")),
                paint(color, YELLOW, value)
            );
        };
        if let Some(s) = &c.server {
            field("server", s);
        }
        if let Some(d) = &c.database {
            field("database", d);
        }
        if let Some(t) = &c.database_type {
            field("type", t);
        }
        if let Some(u) = &c.user {
            field("user", u);
        }
        field("tables", &c.tables.join(", "));
    }
    println!();

    println!(
        "{}",
        paint(color, BOLD, &format!("Tables ({})", tables.len()))
    );
    for t in tables {
        let name = if t.name == t.alias {
            paint(color, BOLD_GREEN, &t.alias)
        } else {
            format!(
                "{} {} {}",
                paint(color, BOLD_GREEN, &t.alias),
                dim("→"),
                t.name
            )
        };
        let kind = if t.kind == "command" {
            format!(" {}", paint(color, CYAN, "(SQL command)"))
        } else {
            String::new()
        };
        let qualified = match &t.qualified_name {
            Some(q) if *q != t.name => format!("  {}", dim(&format!("({q})"))),
            _ => String::new(),
        };
        let src = match &t.subreport {
            Some(s) => format!("  {}", dim(&format!("[sub: {s}]"))),
            None => String::new(),
        };
        println!("  {name}{kind}{qualified}{src}");
    }
    println!();

    println!(
        "{}",
        paint(color, BOLD, &format!("SQL ({})", queries.len()))
    );
    for (i, q) in queries.iter().enumerate() {
        // The dim descriptor after the source: the generated dialect, or the stored-SQL kind. Built
        // plain and dimmed once (a nested inner reset would end the dim run early).
        let mut descriptor = match &q.dialect {
            Some(d) => format!(" · generated ({d})"),
            None => match q.kind {
                QueryKind::Command => " · stored SQL command".to_string(),
                QueryKind::SqlExpression => " · SQL expression".to_string(),
                QueryKind::Generated => String::new(),
            },
        };
        if !q.tables.is_empty() {
            descriptor.push_str(&format!(" · tables: {}", q.tables.join(", ")));
        }
        println!(
            "\n  {} {}{}",
            dim(&format!("[{}]", i + 1)),
            paint(color, BOLD_GREEN, &q.source),
            dim(&descriptor),
        );
        for line in q.sql.lines() {
            println!("      {line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt::model::{
        ConnectionInfo, Database, DbFieldDef, FieldDef, FieldKindData, FieldValueType, Report,
        SqlExpressionField, Subreport, Table, TableJoinType, TableLink,
    };

    fn table(name: &str, fields: &[&str]) -> Table {
        Table {
            name: name.into(),
            alias: name.into(),
            data_fields: fields
                .iter()
                .map(|n| DbFieldDef {
                    name: (*n).into(),
                    value_type: FieldValueType::String,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    /// Collect the query entries for a report, using the default (Postgres) dialect.
    fn queries_of(report: &Report) -> Vec<QueryJson> {
        let mut conns = Vec::new();
        let mut tables = Vec::new();
        let mut queries = Vec::new();
        collect(
            report,
            None,
            Dialect::Postgres,
            &mut conns,
            &mut tables,
            &mut queries,
        );
        queries
    }

    #[test]
    fn dialect_names_resolve_and_reject() {
        assert_eq!(parse_dialect(None).unwrap(), Dialect::Postgres);
        assert_eq!(parse_dialect(Some("Postgres")).unwrap(), Dialect::Postgres);
        assert_eq!(parse_dialect(Some("sqlite")).unwrap(), Dialect::Sqlite);
        assert_eq!(parse_dialect(Some("MySQL")).unwrap(), Dialect::Mysql);
        assert!(parse_dialect(Some("oracle")).is_err());
    }

    #[test]
    fn generated_join_query_lists_its_tables() {
        let report = Report {
            database: Database {
                tables: vec![table("orders", &["cust"]), table("customers", &["id"])],
                links: vec![TableLink {
                    join_type: TableJoinType::Equal,
                    source_table_alias: "orders".into(),
                    target_table_alias: "customers".into(),
                    source_fields: vec!["cust".into()],
                    target_fields: vec!["id".into()],
                }],
            },
            ..Default::default()
        };
        let queries = queries_of(&report);
        // A report with no referenced fields falls back to the full database → both tables joined.
        let gen = &queries[0];
        assert_eq!(gen.source, "Main data query");
        assert!(matches!(gen.kind, QueryKind::Generated));
        assert_eq!(gen.tables, vec!["orders", "customers"]);
        assert!(gen.sql.contains("JOIN \"customers\""), "{}", gen.sql);
    }

    #[test]
    fn command_table_surfaced_verbatim() {
        let mut t = table("cmd", &["v"]);
        t.command_text = Some("SELECT v FROM raw_source".into());
        let report = Report {
            database: Database {
                tables: vec![t],
                ..Default::default()
            },
            ..Default::default()
        };
        let queries = queries_of(&report);
        let cmd = queries
            .iter()
            .find(|q| matches!(q.kind, QueryKind::Command))
            .expect("a command entry");
        assert_eq!(cmd.source, "Command table: cmd");
        assert_eq!(cmd.sql, "SELECT v FROM raw_source");
    }

    #[test]
    fn sql_expression_field_surfaced() {
        let mut report = Report {
            database: Database {
                tables: vec![table("t", &["price"])],
                ..Default::default()
            },
            ..Default::default()
        };
        report.data_definition.field_definitions.push(FieldDef {
            name: "TaxRate".into(),
            kind: FieldKindData::SqlExpression(SqlExpressionField {
                text: "price * 0.2".into(),
            }),
            ..Default::default()
        });
        let queries = queries_of(&report);
        let expr = queries
            .iter()
            .find(|q| matches!(q.kind, QueryKind::SqlExpression))
            .expect("a SQL expression entry");
        assert_eq!(expr.source, "SQL Expression field: TaxRate");
        assert_eq!(expr.sql, "price * 0.2");
    }

    #[test]
    fn subreport_queries_are_prefixed() {
        let sub = Subreport {
            name: "Details".into(),
            report: Box::new(Report {
                database: Database {
                    tables: vec![table("lines", &["qty"])],
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        let report = Report {
            database: Database {
                tables: vec![table("head", &["id"])],
                ..Default::default()
            },
            subreports: vec![sub],
            ..Default::default()
        };
        let queries = queries_of(&report);
        assert!(queries.iter().any(|q| q.source == "Main data query"));
        assert!(
            queries
                .iter()
                .any(|q| q.source == "Subreport 'Details' › Main data query"),
            "subreport query missing; got {:?}",
            queries.iter().map(|q| &q.source).collect::<Vec<_>>()
        );
    }

    #[test]
    fn identical_connections_are_deduped_with_merged_tables() {
        let conn = ConnectionInfo {
            attributes: vec![("QE_ServerDescription".into(), "srv".into())],
            ..Default::default()
        };
        let mut conns = Vec::new();
        record_connection(&conn, "a", &mut conns);
        record_connection(&conn, "b", &mut conns);
        assert_eq!(conns.len(), 1);
        assert_eq!(conns[0].server.as_deref(), Some("srv"));
        assert_eq!(conns[0].tables, vec!["a", "b"]);
    }
}
