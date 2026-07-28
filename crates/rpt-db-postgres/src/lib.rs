//! # rpt-db-postgres — a live PostgreSQL [`RowSource`]
//!
//! The native-side live-data path: given a report's decoded database
//! schema and a Postgres connection, it generates SQL via [`rpt_query`], executes it, and returns a
//! [`RowSource`] the [`rpt_data`] pipeline consumes exactly like the offline
//! [`SavedDataSource`](rpt_data::SavedDataSource). So the same report renders from a live DB with no
//! change to the pipeline or layout engine — the seam the plan designed for.
//!
//! SQL generation (the join graph, column list, and optional `WHERE` push-down) lives in the pure,
//! WASM-safe [`rpt_query`] crate; this crate only executes the string and re-types the result.
//! Multi-table reports are fetched with their links as `JOIN`s, and the translatable
//! part of the record-selection formula is pushed into `WHERE` to fetch fewer rows.
//! Every text-castable column is fetched as text (`::text`) and re-typed via
//! [`rpt_data::cell_to_value`] against the report's declared
//! [`FieldValueType`](rpt_model::FieldValueType); a binary (blob/bytea) column is fetched raw as
//! bytes so its data survives intact. So there is no per-Postgres-type extraction code.

use postgres::fallible_iterator::FallibleIterator;
use postgres::types::ToSql;
use rpt_data::{Cell, Column, Row, RowData, RowSource};
use rpt_model::{Database, Report};
use rpt_query::{build_query_for_report, build_query_full, Dialect, QueryColumn, SqlQuery, Value};
use std::collections::{HashMap, HashSet};

/// A failure of the live PostgreSQL path — the shared [`rpt_data::DbError`] aliased to this driver's
/// error type, so a caller can tell a report with no bound table apart from a genuine connection or
/// query failure. Constructed through its `no_query` / `connect` / `query` helpers.
pub type DbError = rpt_data::DbError<postgres::Error>;

/// A [`RowSource`] backed by a live Postgres query over a report's linked tables.
#[derive(Debug, Clone)]
pub struct PostgresSource(RowData);

impl PostgresSource {
    /// Connect to `conn_str` (libpq/URL form, e.g.
    /// `host=localhost port=55432 user=rpt password=rpt dbname=rptdemo`) and fetch the report's
    /// tables, joined per the link graph. Errors on connection/query failure or a report with no
    /// table.
    ///
    /// The full driver constructor, shared with the SQLite path: `sql_exprs` are the report's SQL
    /// Expression fields (`(name, text)`), `selection` is the translatable part of the record-
    /// selection formula pushed into `WHERE`, `params` bind `{?Name}` current-values into that
    /// push-down, and `comment`, when set, is prepended as a `/* … */` tracking comment for the DB's
    /// query logs.
    ///
    /// # Errors
    ///
    /// - [`DbError::NoQuery`] — no query could be built for the report (it binds no table).
    /// - [`DbError::Connect`] — the database could not be opened or reached.
    /// - [`DbError::Query`] — the statement failed; the error carries the SQL and, for a missing
    ///   table or column, a pointer at `rpt sql` (see [`DbError::hint`]).
    pub fn fetch(
        conn_str: &str,
        database: &Database,
        sql_exprs: &[(String, String)],
        selection: Option<&str>,
        params: &[(String, Value)],
        comment: Option<&str>,
    ) -> Result<PostgresSource, DbError> {
        let mut query = build_query_full(database, sql_exprs, selection, params, Dialect::Postgres)
            .map_err(|e| DbError::no_query(e.to_string()))?;
        if let Some(c) = comment {
            query = query.with_comment(c);
        }
        let mut client =
            postgres::Client::connect(conn_str, postgres::NoTls).map_err(DbError::connect)?;
        Self::run_query_typed(&mut client, &query, database)
    }

    /// Like [`fetch`](Self::fetch) but also pushes the translatable part of `selection` (the report's
    /// Crystal `RecordSelectionFormula`) into the SQL `WHERE`, so the server returns fewer rows. The
    /// pipeline still applies the full formula, so the result set is unchanged.
    ///
    /// `sql_exprs` are the report's SQL Expression fields (`(name, text)`), each selected as
    /// `(<text>) AS "<name>"` so a `{%name}` reference resolves against the fetched column.
    /// `params` are parameter current-values (`{?Name}`) bound into the pushed-down `WHERE`.
    ///
    /// # Errors
    ///
    /// - [`DbError::NoQuery`] — no query could be built for the report (it binds no table).
    /// - [`DbError::Connect`] — the database could not be opened or reached.
    /// - [`DbError::Query`] — the statement failed; the error carries the SQL and, for a missing
    ///   table or column, a pointer at `rpt sql` (see [`DbError::hint`]).
    pub fn fetch_with_selection(
        conn_str: &str,
        database: &Database,
        selection: Option<&str>,
        sql_exprs: &[(String, String)],
        params: &[(String, Value)],
    ) -> Result<PostgresSource, DbError> {
        let query = build_query_full(database, sql_exprs, selection, params, Dialect::Postgres)
            .map_err(|e| DbError::no_query(e.to_string()))?;
        let mut client =
            postgres::Client::connect(conn_str, postgres::NoTls).map_err(DbError::connect)?;
        Self::run_query_typed(&mut client, &query, database)
    }

    /// Like [`fetch_with_selection`](Self::fetch_with_selection) but prunes the query to the tables
    /// and columns the `report` actually references ([`build_query_for_report`]), so declared-but-
    /// unused tables are not pulled into the `FROM` (and cross-joined into a cartesian). This is what
    /// the engine does; prefer it whenever the full [`Report`] is available.
    ///
    /// # Errors
    ///
    /// - [`DbError::NoQuery`] — no query could be built for the report (it binds no table).
    /// - [`DbError::Connect`] — the database could not be opened or reached.
    /// - [`DbError::Query`] — the statement failed; the error carries the SQL and, for a missing
    ///   table or column, a pointer at `rpt sql` (see [`DbError::hint`]).
    pub fn fetch_for_report(
        conn_str: &str,
        report: &Report,
        selection: Option<&str>,
        sql_exprs: &[(String, String)],
        params: &[(String, Value)],
    ) -> Result<PostgresSource, DbError> {
        let query = build_query_for_report(report, sql_exprs, selection, params, Dialect::Postgres)
            .map_err(|e| DbError::no_query(e.to_string()))?;
        let mut client =
            postgres::Client::connect(conn_str, postgres::NoTls).map_err(DbError::connect)?;
        Self::run_query_typed(&mut client, &query, &report.database)
    }

    /// Execute an already-generated [`SqlQuery`] on an existing client (exposed for tests / custom
    /// callers). Rows are keyed by each column's `alias.field` name (how formulas reference them).
    ///
    /// This variant does not know the report schema, so a Postgres `boolean` column bound to a
    /// String-declared field is fetched as Postgres's `true`/`false`. Prefer
    /// [`run_query_typed`](Self::run_query_typed) when the [`Database`] is available so booleans match
    /// the psqlODBC representation Crystal authored against.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the statement fails. The error carries the SQL and, for a missing table
    /// or column, a pointer at `rpt sql` (see [`DbError::hint`]).
    pub fn run_query(
        client: &mut postgres::Client,
        query: &SqlQuery,
    ) -> Result<PostgresSource, DbError> {
        Self::run_query_masked(client, query, &[])
    }

    /// Like [`run_query`](Self::run_query) but probes the DB catalog for which selected columns are a
    /// Postgres `boolean`, so each such cell is represented as psqlODBC's `BoolsAsChar` does — `1`/`0`
    /// — rather than `true`/`false`. Crystal authored these reports through psqlODBC, so a boolean
    /// column typically binds to a **String** field whose formulas compare against `"1"`/`"0"`; a
    /// native `true`/`false` would break those comparisons. General across every boolean schema column,
    /// not report-specific.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the statement fails. The error carries the SQL and, for a missing table
    /// or column, a pointer at `rpt sql` (see [`DbError::hint`]).
    pub fn run_query_typed(
        client: &mut postgres::Client,
        query: &SqlQuery,
        database: &Database,
    ) -> Result<PostgresSource, DbError> {
        let bools = probe_bool_columns(client, database, &query.columns);
        Self::run_query_masked(client, query, &bools)
    }

    /// Shared executor. `bool_mask[i] == true` marks result column `i` as a Postgres boolean whose
    /// text (`true`/`false`) is remapped to `1`/`0` (NULL stays NULL). An empty mask disables the
    /// remap for every column.
    fn run_query_masked(
        client: &mut postgres::Client,
        query: &SqlQuery,
        bool_mask: &[bool],
    ) -> Result<PostgresSource, DbError> {
        let columns: Vec<Column> = query.result_columns();
        let n = columns.len();
        // A binary (bytea) column is selected raw and read as bytes; every other column is selected
        // as text. Snapshot the per-column choice before `columns` is moved into `from_cells`.
        let binary: Vec<bool> = columns.iter().map(|c| c.value_type.is_binary()).collect();
        let is_bool = |i: usize| bool_mask.get(i).copied().unwrap_or(false);
        // Stream the result set with a server-side portal (`query_raw`) instead of buffering the whole
        // result into a `Vec<Row>` up front (`query`). A large joined result set (tens of millions of
        // rows) then costs O(1) driver memory — each row is re-typed into the `RowData` and dropped as
        // it arrives, rather than the entire raw result and the typed copy being resident at once.
        let no_params: Vec<&(dyn ToSql + Sync)> = Vec::new();
        let mut pg_iter = client
            .query_raw(&query.sql, no_params)
            .map_err(DbError::query)?;
        // `RowData::from_cells` keys and re-types every cell — this closure supplies only the driver's
        // cell accessor. `try_get` surfaces a type-conversion failure or out-of-range index as
        // `DbError::Query` instead of panicking the process (which `Row::get` would).
        let data = RowData::from_cells(columns, || {
            let Some(pg) = pg_iter.next().map_err(DbError::query)? else {
                return Ok(None);
            };
            let mut cells = Vec::with_capacity(n);
            for (i, &is_binary) in binary.iter().enumerate() {
                let cell = if is_binary {
                    pg.try_get::<_, Option<Vec<u8>>>(i)
                        .map_err(DbError::query)?
                        .map(Cell::Bytes)
                } else {
                    pg.try_get::<_, Option<String>>(i)
                        .map_err(DbError::query)?
                        .map(|s| if is_bool(i) { bool_to_char(s) } else { s })
                        .map(Cell::Text)
                };
                cells.push(cell);
            }
            Ok::<_, DbError>(Some(cells))
        })?;
        Ok(PostgresSource(data))
    }
}

/// Represent a Postgres boolean's text the way psqlODBC's `BoolsAsChar` does: `true`→`1`, `false`→`0`.
/// Any other text (already NULL-filtered upstream) is returned unchanged.
fn bool_to_char(s: String) -> String {
    match s.as_str() {
        "true" => "1".to_string(),
        "false" => "0".to_string(),
        _ => s,
    }
}

/// Probe the DB catalog for which of `columns` are backed by a Postgres `boolean`, returning a
/// per-column mask in result order. Only plain table columns are probed (a SQL-expression or
/// command-table column has no `information_schema` row to consult and is left as text). A catalog
/// query failure is non-fatal: the mask is all-`false`, so the boolean fix is simply skipped rather
/// than failing the whole fetch.
fn probe_bool_columns(
    client: &mut postgres::Client,
    database: &Database,
    columns: &[QueryColumn],
) -> Vec<bool> {
    // Table alias (how a column is selected) → real table name (what the catalog is keyed by).
    let alias_to_table: HashMap<&str, &str> = database
        .tables
        .iter()
        .map(|t| (t.alias.as_str(), t.name.as_str()))
        .collect();
    // The distinct real tables the plain columns are drawn from — the catalog scope.
    let tables: Vec<String> = columns
        .iter()
        .filter(|c| c.expr.is_none())
        .filter_map(|c| alias_to_table.get(c.alias.as_str()).copied())
        .collect::<HashSet<_>>()
        .into_iter()
        .map(str::to_string)
        .collect();
    if tables.is_empty() {
        return vec![false; columns.len()];
    }
    let bool_cols: HashSet<(String, String)> = match client.query(
        "SELECT table_name::text, column_name::text FROM information_schema.columns \
         WHERE table_name::text = ANY($1) AND data_type = 'boolean'",
        &[&tables],
    ) {
        Ok(rows) => rows
            .iter()
            .filter_map(|r| Some((r.try_get(0).ok()?, r.try_get(1).ok()?)))
            .collect(),
        Err(_) => return vec![false; columns.len()],
    };
    columns
        .iter()
        .map(|c| {
            c.expr.is_none()
                && alias_to_table
                    .get(c.alias.as_str())
                    .is_some_and(|t| bool_cols.contains(&((*t).to_string(), c.field.clone())))
        })
        .collect()
}

impl RowSource for PostgresSource {
    fn columns(&self) -> &[Column] {
        self.0.columns()
    }
    fn rows(&self) -> Vec<Row> {
        self.0.rows()
    }
}

/// Connect to `conn_str` and run a batch of SQL (schema + seed data) as one script. A convenience for
/// tests and fixture builders so callers don't depend on the `postgres` crate directly. The batch is
/// expected to be idempotent (`DROP TABLE IF EXISTS` → `CREATE` → `INSERT`) so a fixture can re-seed a
/// shared server between runs.
///
/// # Errors
///
/// [`DbError::Connect`] if the database cannot be opened or reached.
///
/// [`DbError::Query`] if a statement in `sql` fails.
pub fn seed(conn_str: &str, sql: &str) -> Result<(), DbError> {
    let mut client =
        postgres::Client::connect(conn_str, postgres::NoTls).map_err(DbError::connect)?;
    client.batch_execute(sql).map_err(DbError::query)?;
    Ok(())
}

/// A live Postgres connection, split from the fetch so a caller (e.g. the render CLI) can order the
/// steps itself: connect → [`ping`](Self::ping) healthcheck → build+log the SQL → [`run`](Self::run).
/// Keeps every `postgres`-crate specific inside this crate; callers deal only in [`SqlQuery`] /
/// [`PostgresSource`].
pub struct PostgresConn {
    client: postgres::Client,
}

impl std::fmt::Debug for PostgresConn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `postgres::Client` is not Debug; nothing here is safe/useful to print.
        f.debug_struct("PostgresConn").finish_non_exhaustive()
    }
}

impl PostgresConn {
    /// Open a connection (no TLS). `conn_str` is libpq/URL form
    /// (`host=… port=… user=… password=… dbname=…`).
    ///
    /// # Errors
    ///
    /// [`DbError::Connect`] if the database cannot be opened or reached.
    pub fn connect(conn_str: &str) -> Result<PostgresConn, DbError> {
        Ok(PostgresConn {
            client: postgres::Client::connect(conn_str, postgres::NoTls)
                .map_err(DbError::connect)?,
        })
    }

    /// Cheap liveness probe (`SELECT 1`) — the pre-fetch healthcheck. Errors if the server is
    /// unreachable or rejects the round-trip, so a bad connection fails fast.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the statement fails. The error carries the SQL and, for a missing table
    /// or column, a pointer at `rpt sql` (see [`DbError::hint`]).
    pub fn ping(&mut self) -> Result<(), DbError> {
        self.client
            .simple_query("SELECT 1")
            .map_err(DbError::query)?;
        Ok(())
    }

    /// The server's reported version string (`SHOW server_version`), for the healthcheck log. `None`
    /// if the probe fails (non-fatal — only informational).
    pub fn server_version(&mut self) -> Option<String> {
        self.client
            .query_one("SHOW server_version", &[])
            .ok()
            .and_then(|r| r.try_get::<_, String>(0).ok())
    }

    /// Execute an already-built [`SqlQuery`] and materialize the [`PostgresSource`]. Booleans are
    /// fetched as Postgres's `true`/`false`; prefer [`run_typed`](Self::run_typed) when the
    /// [`Database`] is available.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the statement fails. The error carries the SQL and, for a missing table
    /// or column, a pointer at `rpt sql` (see [`DbError::hint`]).
    pub fn run(&mut self, query: &SqlQuery) -> Result<PostgresSource, DbError> {
        PostgresSource::run_query(&mut self.client, query)
    }

    /// Like [`run`](Self::run) but resolves Postgres `boolean` columns to the psqlODBC `1`/`0`
    /// representation the report was authored against (see
    /// [`PostgresSource::run_query_typed`]).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the statement fails. The error carries the SQL and, for a missing table
    /// or column, a pointer at `rpt sql` (see [`DbError::hint`]).
    pub fn run_typed(
        &mut self,
        query: &SqlQuery,
        database: &Database,
    ) -> Result<PostgresSource, DbError> {
        PostgresSource::run_query_typed(&mut self.client, query, database)
    }
}

#[cfg(test)]
mod tests {
    use rpt_model::{
        Database, DbFieldDef, FieldValueType, Table, TableJoinKind, TableLink, TableLinkOperator,
    };
    use rpt_query::build_query;

    fn table(name: &str, fields: &[(&str, FieldValueType)]) -> Table {
        Table {
            name: name.into(),
            alias: name.into(),
            data_fields: fields
                .iter()
                .map(|(n, vt)| DbFieldDef {
                    name: (*n).into(),
                    value_type: *vt,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn bool_text_maps_to_psqlodbc_chars() {
        // A Postgres boolean column bound to a String field must read as psqlODBC's `1`/`0`, not the
        // native `true`/`false`, so a `{field} <> "1"` comparison behaves as Crystal authored it.
        assert_eq!(super::bool_to_char("true".to_string()), "1");
        assert_eq!(super::bool_to_char("false".to_string()), "0");
        // Anything else (a genuine text value) is untouched.
        assert_eq!(super::bool_to_char("truthy".to_string()), "truthy");
    }

    #[test]
    fn single_table_select_shape() {
        let db = Database {
            tables: vec![table(
                "countries_all_iso",
                &[
                    ("id", FieldValueType::Int32s),
                    ("name", FieldValueType::String),
                ],
            )],
            ..Default::default()
        };
        let q = build_query(&db).unwrap();
        assert_eq!(
            q.sql,
            r#"SELECT "countries_all_iso"."id"::text, "countries_all_iso"."name"::text FROM "countries_all_iso" AS "countries_all_iso""#
        );
    }

    #[test]
    fn multi_table_join_fetches_all_columns() {
        // Two linked tables → the generated query fetches both tables' columns, keyed alias.field,
        // so the pipeline sees the joined row.
        let mut db = Database {
            tables: vec![
                table(
                    "orders",
                    &[
                        ("id", FieldValueType::Int32s),
                        ("cust", FieldValueType::Int32s),
                    ],
                ),
                table(
                    "customers",
                    &[
                        ("id", FieldValueType::Int32s),
                        ("name", FieldValueType::String),
                    ],
                ),
            ],
            ..Default::default()
        };
        db.links = vec![TableLink {
            join_kind: TableJoinKind::Inner,
            operator: TableLinkOperator::Equal,
            source_table_alias: "orders".into(),
            target_table_alias: "customers".into(),
            source_fields: vec!["cust".into()],
            target_fields: vec!["id".into()],
        }];

        let q = build_query(&db).unwrap();
        assert!(q.sql.contains("JOIN \"customers\""), "{}", q.sql);
        assert_eq!(
            q.columns.iter().map(|c| c.key()).collect::<Vec<_>>(),
            vec!["orders.id", "orders.cust", "customers.id", "customers.name"]
        );
    }
}
