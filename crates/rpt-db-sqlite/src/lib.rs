//! # rpt-db-sqlite — an in-process SQLite [`RowSource`]
//!
//! The zero-process live-data path: given a report's decoded database schema and a SQLite database
//! (a file or `:memory:`), it generates SQL via [`rpt_query`] (the SQLite dialect), executes it with
//! a **bundled** SQLite (no system library, no server), and returns a [`RowSource`] the [`rpt_data`]
//! pipeline consumes exactly like the offline [`SavedDataSource`](rpt_data::SavedDataSource) or the
//! live `rpt-db-postgres` path. So the same report renders from a real database with no change to
//! the pipeline or layout engine.
//!
//! SQLite runs in-process, so this path needs no server to provision — a file or `:memory:` is the
//! whole datasource. SQL generation lives in the pure,
//! WASM-safe [`rpt_query`] crate; this crate only executes the string and re-types the result. Every
//! column is fetched as text (`CAST(x AS TEXT)`) and re-typed via [`rpt_data::cell_to_value`] against
//! the report's declared [`FieldValueType`](rpt_model::FieldValueType), so there is no
//! per-SQLite-type extraction code.

use rpt_data::{Cell, Column, Row, RowData, RowSource};
use rpt_model::{Database, Report};
use rpt_query::{build_query_for_report, build_query_full, Dialect, SqlQuery, Value};
use rusqlite::Connection;

/// A failure of the SQLite path — the shared [`rpt_data::DbError`] aliased to this driver's error
/// type, so a caller can tell a report with no bound table apart from a genuine open or query
/// failure. Constructed through its `no_query` / `connect` / `query` helpers.
pub type DbError = rpt_data::DbError<rusqlite::Error>;

/// A [`RowSource`] backed by a SQLite query over a report's linked tables.
#[derive(Debug, Clone)]
pub struct SqliteSource(RowData);

impl SqliteSource {
    /// Open the database at `url` and fetch the report's tables (joined per the link graph),
    /// re-typing every value against the report's declared field types. `url` accepts
    /// `sqlite:///abs/path.db`, `sqlite://rel/path.db`, `sqlite::memory:`, or a bare filesystem path.
    ///
    /// The full driver constructor, shared with the Postgres path: `sql_exprs` are the report's SQL
    /// Expression fields (`(name, text)`), each selected as `(<text>) AS "<name>"` so a `{%name}`
    /// reference resolves against the fetched column; `selection`/`params` are the record-selection
    /// push-down (the SQLite dialect emits no `WHERE`, so it fetches the full table and the pipeline
    /// applies the formula per row — the result set is identical either way); and `comment`, when
    /// set, is prepended as a `/* … */` tracking comment for the DB's query logs. Errors on
    /// open/query failure or a report with no table.
    ///
    /// # Errors
    ///
    /// - [`DbError::NoQuery`] — no query could be built for the report (it binds no table).
    /// - [`DbError::Connect`] — the database could not be opened or reached.
    /// - [`DbError::Query`] — the statement failed; the error carries the SQL and, for a missing
    ///   table or column, a pointer at `rpt sql` (see [`DbError::hint`]).
    pub fn fetch(
        url: &str,
        database: &Database,
        sql_exprs: &[(String, String)],
        selection: Option<&str>,
        params: &[(String, Value)],
        comment: Option<&str>,
    ) -> Result<SqliteSource, DbError> {
        let query = build_query_full(database, sql_exprs, selection, params, Dialect::Sqlite)
            .map_err(|e| DbError::no_query(e.to_string()))?;
        Self::open_and_run(url, query, comment)
    }

    /// Like [`fetch`](Self::fetch) but prunes the query to the tables and columns the `report`
    /// actually references ([`build_query_for_report`]), so declared-but-unused tables are not
    /// cross-joined into the `FROM`. Prefer it whenever the full [`Report`] is available.
    ///
    /// # Errors
    ///
    /// - [`DbError::NoQuery`] — no query could be built for the report (it binds no table).
    /// - [`DbError::Connect`] — the database could not be opened or reached.
    /// - [`DbError::Query`] — the statement failed; the error carries the SQL and, for a missing
    ///   table or column, a pointer at `rpt sql` (see [`DbError::hint`]).
    pub fn fetch_for_report(
        url: &str,
        report: &Report,
        selection: Option<&str>,
        sql_exprs: &[(String, String)],
        params: &[(String, Value)],
    ) -> Result<SqliteSource, DbError> {
        let query = build_query_for_report(report, sql_exprs, selection, params, Dialect::Sqlite)
            .map_err(|e| DbError::no_query(e.to_string()))?;
        Self::open_and_run(url, query, None)
    }

    /// Open `url`, optionally stamp `comment` onto `query`, and run it.
    fn open_and_run(
        url: &str,
        mut query: SqlQuery,
        comment: Option<&str>,
    ) -> Result<SqliteSource, DbError> {
        if let Some(c) = comment {
            query = query.with_comment(c);
        }
        let conn = open(url).map_err(DbError::connect)?;
        Self::run_query(&conn, &query)
    }

    /// Execute an already-generated [`SqlQuery`] on an existing connection, for a caller that manages
    /// the connection itself. Rows are keyed by each column's `alias.field` name (how formulas
    /// reference them).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the statement fails. The error carries the SQL and, for a missing table
    /// or column, a pointer at `rpt sql` (see [`DbError::hint`]).
    pub fn run_query(conn: &Connection, query: &SqlQuery) -> Result<SqliteSource, DbError> {
        // Attach the failing statement once here rather than at each `map_err(DbError::query)` below:
        // without it the driver's "no such table: x" arrives with no way to see what was actually sent.
        Self::run_query_inner(conn, query).map_err(|e| e.in_context(None, Some(&query.sql)))
    }

    fn run_query_inner(conn: &Connection, query: &SqlQuery) -> Result<SqliteSource, DbError> {
        let columns: Vec<Column> = query.result_columns();
        let n = columns.len();
        // A binary column is selected raw and read as bytes; every other column is CAST to text.
        // Snapshot the per-column choice before `columns` is moved into `from_cells`.
        let binary: Vec<bool> = columns.iter().map(|c| c.value_type.is_binary()).collect();
        let mut stmt = conn.prepare(&query.sql).map_err(DbError::query)?;
        let mut sqlite_rows = stmt.query([]).map_err(DbError::query)?;
        // `RowData::from_cells` keys and re-types every cell — this closure supplies only the driver's
        // cell accessor (advancing the forward-only cursor one row at a time). Every read failure is
        // classified explicitly as `DbError::Query`, so open and query failures stay distinguishable.
        let data = RowData::from_cells(columns, || -> Result<_, DbError> {
            match sqlite_rows.next().map_err(DbError::query)? {
                Some(sr) => {
                    let mut cells = Vec::with_capacity(n);
                    for (i, &is_binary) in binary.iter().enumerate() {
                        let cell = if is_binary {
                            sr.get::<_, Option<Vec<u8>>>(i)
                                .map_err(DbError::query)?
                                .map(Cell::Bytes)
                        } else {
                            sr.get::<_, Option<String>>(i)
                                .map_err(DbError::query)?
                                .map(Cell::Text)
                        };
                        cells.push(cell);
                    }
                    Ok(Some(cells))
                }
                None => Ok(None),
            }
        })?;
        Ok(SqliteSource(data))
    }
}

impl RowSource for SqliteSource {
    fn columns(&self) -> &[Column] {
        self.0.columns()
    }
    fn rows(&self) -> Vec<Row> {
        self.0.rows()
    }
}

/// An open SQLite connection, split from the fetch so a caller (e.g. the render CLI) can order the
/// steps itself: [`open`](Self::open) → build+log the SQL → [`run`](Self::run). Keeps every
/// `rusqlite` specific inside this crate; callers deal only in [`SqlQuery`] / [`SqliteSource`]. This
/// mirrors `rpt-db-postgres`'s `PostgresConn`, so the CLI runs the *same* query it logged instead of
/// building the SQL string a second time.
pub struct SqliteConn {
    conn: Connection,
}

impl std::fmt::Debug for SqliteConn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `rusqlite::Connection` is not Debug; nothing here is useful to print.
        f.debug_struct("SqliteConn").finish_non_exhaustive()
    }
}

impl SqliteConn {
    /// Open the database at `url` (`sqlite:///abs`, `sqlite://rel`, `sqlite::memory:`, or a bare
    /// path). Errors on open failure.
    ///
    /// # Errors
    ///
    /// [`DbError::Connect`] if the database cannot be opened or reached.
    pub fn open(url: &str) -> Result<SqliteConn, DbError> {
        Ok(SqliteConn {
            conn: open(url).map_err(DbError::connect)?,
        })
    }

    /// Execute an already-built [`SqlQuery`] and materialize the [`SqliteSource`].
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the statement fails. The error carries the SQL and, for a missing table
    /// or column, a pointer at `rpt sql` (see [`DbError::hint`]).
    pub fn run(&self, query: &SqlQuery) -> Result<SqliteSource, DbError> {
        SqliteSource::run_query(&self.conn, query)
    }
}

/// Create/open the database at `url` and run a batch of SQL (schema + seed data), so a caller can
/// populate a database without depending on `rusqlite` directly.
///
/// # Errors
///
/// [`DbError::Connect`] if the database cannot be opened or reached.
///
/// [`DbError::Query`] if a statement in `sql` fails.
pub fn seed(url: &str, sql: &str) -> Result<(), DbError> {
    let conn = open(url).map_err(DbError::connect)?;
    conn.execute_batch(sql).map_err(DbError::query)?;
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
    fn leading_tracking_comment_does_not_break_execution() {
        // A `/* … */` tracking comment prepended to the SQL is valid and returns the same rows.
        let conn = open("sqlite::memory:").unwrap();
        conn.execute_batch(
            "CREATE TABLE cities(name TEXT, pop INTEGER);
             INSERT INTO cities VALUES ('Toronto', 5);",
        )
        .unwrap();
        let db = one_table_db();
        let q = build_query_in(&db, Dialect::Sqlite)
            .unwrap()
            .with_comment(r#"rpt-rs report="x.rpt" scope=main"#);
        assert!(q.sql.starts_with("/* rpt-rs"), "{}", q.sql);
        let src = SqliteSource::run_query(&conn, &q).unwrap();
        assert_eq!(src.rows().len(), 1);
    }

    #[test]
    fn url_forms_open() {
        assert!(open("sqlite::memory:").is_ok());
        assert!(open(":memory:").is_ok());
    }
}
