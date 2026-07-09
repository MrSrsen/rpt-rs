//! # rpt-db-sqlite — an in-process SQLite [`RowSource`]
//!
//! The zero-process live-data path: given a report's decoded database schema and a SQLite database
//! (a file or `:memory:`), it generates SQL via [`rpt_query`] (the SQLite dialect), executes it with
//! a **bundled** SQLite (no system library, no server), and returns a [`RowSource`] the [`rpt_data`]
//! pipeline consumes exactly like the offline [`SavedDataSource`](rpt_data::SavedDataSource) or the
//! live `rpt-db-postgres` path. So the same report renders from a real database with no change to
//! the pipeline or layout engine.
//!
//! Because SQLite runs in-process, this is the datasource for **full-stack render tests in CI and on
//! localhost** with no external processes to spin up. SQL generation lives in the pure,
//! WASM-safe [`rpt_query`] crate; this crate only executes the string and re-types the result. Every
//! column is fetched as text (`CAST(x AS TEXT)`) and re-typed via [`rpt_data::cell_to_value`] against
//! the report's declared [`FieldValueType`](rpt_model::FieldValueType), so there is no
//! per-SQLite-type extraction code.

use rpt_data::{rows_from_cells, Column, Row, RowSource};
use rpt_model::Database;
use rpt_query::{build_query_full, Dialect, SqlQuery};
use rusqlite::Connection;

/// A failure of the SQLite path, typed so a caller can tell a report with no bound table apart from a
/// genuine open or query failure (the previous `Box<dyn Error>` erased that).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DbError {
    /// The report has no bound database table, so no query can be built.
    #[error("report has no database table to query")]
    NoTable,
    /// Opening the database file/connection failed.
    #[error("open failed: {0}")]
    Open(#[source] rusqlite::Error),
    /// Preparing or executing a query failed.
    #[error("query failed: {0}")]
    Query(#[source] rusqlite::Error),
}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> DbError {
        DbError::Query(e)
    }
}

/// A [`RowSource`] backed by a SQLite query over a report's linked tables.
#[derive(Debug, Clone)]
pub struct SqliteSource {
    columns: Vec<Column>,
    rows: Vec<Row>,
}

impl SqliteSource {
    /// Open the database at `url` and fetch the report's tables (joined per the link graph),
    /// re-typing every value against the report's declared field types. `url` accepts
    /// `sqlite:///abs/path.db`, `sqlite://rel/path.db`, `sqlite::memory:`, or a bare filesystem path.
    /// `sql_exprs` are the report's SQL Expression fields (`(name, text)`), each selected as
    /// `(<text>) AS "<name>"` so a `{%name}` reference resolves against the fetched column.
    /// Errors on open/query failure or a report with no table.
    pub fn fetch(
        url: &str,
        database: &Database,
        sql_exprs: &[(String, String)],
    ) -> Result<SqliteSource, DbError> {
        let conn = open(url).map_err(DbError::Open)?;
        // SQLite has no WHERE push-down (Postgres-only), so no selection/params are passed.
        let query = build_query_full(database, sql_exprs, None, &[], Dialect::Sqlite)
            .ok_or(DbError::NoTable)?;
        Self::run_query(&conn, &query)
    }

    /// Execute an already-generated [`SqlQuery`] on an existing connection (exposed for tests / custom
    /// callers). Rows are keyed by each column's `alias.field` name (how formulas reference them).
    pub fn run_query(conn: &Connection, query: &SqlQuery) -> Result<SqliteSource, DbError> {
        let columns: Vec<Column> = query.result_columns();
        let n = columns.len();
        let mut stmt = conn.prepare(&query.sql)?;
        let mut sqlite_rows = stmt.query([])?;
        // Each column was CAST to text (or is NULL); read them positionally. The shared
        // `rows_from_cells` keys and re-types every cell — this closure supplies only the driver's
        // text-cell accessor (advancing the forward-only cursor one row at a time).
        let rows = rows_from_cells(&columns, || -> Result<_, DbError> {
            match sqlite_rows.next()? {
                Some(sr) => {
                    let mut cells = Vec::with_capacity(n);
                    for i in 0..n {
                        cells.push(sr.get::<_, Option<String>>(i)?);
                    }
                    Ok(Some(cells))
                }
                None => Ok(None),
            }
        })?;
        Ok(SqliteSource { columns, rows })
    }
}

impl RowSource for SqliteSource {
    fn columns(&self) -> &[Column] {
        &self.columns
    }
    fn rows(&self) -> Vec<Row> {
        self.rows.clone()
    }
}

/// Create/open the database at `url` and run a batch of SQL (schema + seed data). A convenience for
/// tests and fixture builders so callers don't depend on `rusqlite` directly.
pub fn seed(url: &str, sql: &str) -> Result<(), DbError> {
    let conn = open(url).map_err(DbError::Open)?;
    conn.execute_batch(sql)?;
    Ok(())
}

/// Resolve a SQLite URL/path to a connection. Accepts `sqlite:` URLs (`sqlite:///abs`,
/// `sqlite://rel`, `sqlite::memory:`) and bare paths / `:memory:`.
fn open(url: &str) -> rusqlite::Result<Connection> {
    let rest = url.strip_prefix("sqlite:").unwrap_or(url);
    if rest == ":memory:" || rest == "//:memory:" || rest.is_empty() {
        return Connection::open_in_memory();
    }
    // Strip the authority-less `//` prefix: `sqlite:///abs` → `/abs`, `sqlite://./rel` → `./rel`.
    let path = rest.strip_prefix("//").unwrap_or(rest);
    Connection::open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crystal_formula::eval::Value;
    use rpt_model::{Database, DbFieldDef, FieldValueType, Table};
    use rpt_query::build_query_in;

    fn one_table_db() -> Database {
        let mut t = Table {
            name: "cities".into(),
            alias: "cities".into(),
            ..Default::default()
        };
        for (n, vt) in [
            ("name", FieldValueType::String),
            ("pop", FieldValueType::Int32s),
        ] {
            t.data_fields.push(DbFieldDef {
                name: n.into(),
                value_type: vt,
                ..Default::default()
            });
        }
        Database {
            tables: vec![t],
            ..Default::default()
        }
    }

    #[test]
    fn fetches_and_retypes_rows_from_memory_db() {
        // Note: an in-memory DB is per-connection, so seed + fetch must share one connection.
        let conn = open("sqlite::memory:").unwrap();
        conn.execute_batch(
            "CREATE TABLE cities(name TEXT, pop INTEGER);
             INSERT INTO cities VALUES ('Toronto', 2794356), ('Ottawa', 1017449);",
        )
        .unwrap();
        let db = one_table_db();
        let q = build_query_in(&db, Dialect::Sqlite).unwrap();
        assert!(
            q.sql.contains("CAST("),
            "SQLite dialect casts to text: {}",
            q.sql
        );
        let src = SqliteSource::run_query(&conn, &q).unwrap();

        assert_eq!(
            src.columns()
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["cities.name", "cities.pop"]
        );
        let rows = src.rows();
        assert_eq!(rows.len(), 2);
        // Values are re-typed: name → String, pop → Number.
        assert_eq!(
            rows[0].get("cities.name"),
            Some(&Value::Str("Toronto".into()))
        );
        assert_eq!(rows[0].get("cities.pop"), Some(&Value::Number(2794356.0)));
    }

    #[test]
    fn sql_expression_field_is_fetched_and_keyed_by_name() {
        // A SQL Expression field selected as `(<text>) AS "name"` resolves under its bare name:
        // `row.get("tax")` finds the server-computed value.
        let conn = open("sqlite::memory:").unwrap();
        conn.execute_batch(
            "CREATE TABLE cities(name TEXT, pop INTEGER);
             INSERT INTO cities VALUES ('Toronto', 100);",
        )
        .unwrap();
        let db = one_table_db();
        let exprs = vec![("tax".to_string(), "pop * 2".to_string())];
        let q = build_query_full(&db, &exprs, None, &[], Dialect::Sqlite).unwrap();
        assert!(q.sql.contains(r#"AS "tax""#), "{}", q.sql);
        let src = SqliteSource::run_query(&conn, &q).unwrap();
        let rows = src.rows();
        assert_eq!(rows.len(), 1);
        // Keyed by the bare SQL-expression name.
        assert_eq!(rows[0].get("tax"), Some(&Value::Str("200".into())));
    }

    #[test]
    fn url_forms_open() {
        assert!(open("sqlite::memory:").is_ok());
        assert!(open(":memory:").is_ok());
    }
}
