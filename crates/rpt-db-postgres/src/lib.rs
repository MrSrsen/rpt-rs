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
//! Every column is fetched as text (`::text`) and re-typed via [`rpt_data::cell_to_value`] against
//! the report's declared [`FieldValueType`](rpt_model::FieldValueType), so no per-Postgres-type extraction code is needed.

use rpt_data::{rows_from_cells, Column, Row, RowSource};
use rpt_model::{Database, Report};
use rpt_query::{build_query_for_report, build_query_full, Dialect, SqlQuery, Value};

/// A failure of the live PostgreSQL path, typed so a caller can tell a report with no bound table
/// apart from a genuine connection or query failure (the previous `Box<dyn Error>` erased that).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DbError {
    /// The report has no bound database table, so no query can be built.
    #[error("report has no database table to query")]
    NoTable,
    /// Opening the connection to the server failed.
    #[error("connection failed: {0}")]
    Connect(#[source] postgres::Error),
    /// Executing a query (or the `SELECT 1` healthcheck) failed.
    #[error("query failed: {0}")]
    Query(#[source] postgres::Error),
}

/// A [`RowSource`] backed by a live Postgres query over a report's linked tables.
#[derive(Debug, Clone)]
pub struct PostgresSource {
    columns: Vec<Column>,
    rows: Vec<Row>,
}

impl PostgresSource {
    /// Connect to `conn_str` (libpq/URL form, e.g.
    /// `host=localhost port=55432 user=rpt password=rpt dbname=rptdemo`) and fetch the report's
    /// tables, joined per the link graph. Errors on connection/query failure or a report with no
    /// table.
    pub fn fetch(conn_str: &str, database: &Database) -> Result<PostgresSource, DbError> {
        Self::fetch_with_selection(conn_str, database, None, &[], &[])
    }

    /// Like [`fetch`](Self::fetch) but also pushes the translatable part of `selection` (the report's
    /// Crystal `RecordSelectionFormula`) into the SQL `WHERE`, so the server returns fewer rows. The
    /// pipeline still applies the full formula, so the result set is unchanged.
    ///
    /// `sql_exprs` are the report's SQL Expression fields (`(name, text)`), each selected as
    /// `(<text>) AS "<name>"` so a `{%name}` reference resolves against the fetched column.
    /// `params` are parameter current-values (`{?Name}`) bound into the pushed-down `WHERE`.
    pub fn fetch_with_selection(
        conn_str: &str,
        database: &Database,
        selection: Option<&str>,
        sql_exprs: &[(String, String)],
        params: &[(String, Value)],
    ) -> Result<PostgresSource, DbError> {
        let query = build_query_full(database, sql_exprs, selection, params, Dialect::Postgres)
            .ok_or(DbError::NoTable)?;
        let mut client =
            postgres::Client::connect(conn_str, postgres::NoTls).map_err(DbError::Connect)?;
        Self::run_query(&mut client, &query)
    }

    /// Like [`fetch_with_selection`](Self::fetch_with_selection) but prunes the query to the tables
    /// and columns the `report` actually references ([`build_query_for_report`]), so declared-but-
    /// unused tables are not pulled into the `FROM` (and cross-joined into a cartesian). This is what
    /// the engine does; prefer it whenever the full [`Report`] is available.
    pub fn fetch_for_report(
        conn_str: &str,
        report: &Report,
        selection: Option<&str>,
        sql_exprs: &[(String, String)],
        params: &[(String, Value)],
    ) -> Result<PostgresSource, DbError> {
        let query = build_query_for_report(report, sql_exprs, selection, params, Dialect::Postgres)
            .ok_or(DbError::NoTable)?;
        let mut client =
            postgres::Client::connect(conn_str, postgres::NoTls).map_err(DbError::Connect)?;
        Self::run_query(&mut client, &query)
    }

    /// Execute an already-generated [`SqlQuery`] on an existing client (exposed for tests / custom
    /// callers). Rows are keyed by each column's `alias.field` name (how formulas reference them).
    pub fn run_query(
        client: &mut postgres::Client,
        query: &SqlQuery,
    ) -> Result<PostgresSource, DbError> {
        let columns: Vec<Column> = query.result_columns();
        let n = columns.len();
        let pg_rows = client.query(&query.sql, &[]).map_err(DbError::Query)?;
        // Each column was selected as text (or NULL); read them positionally. The shared
        // `rows_from_cells` keys and re-types every cell — this closure supplies only the driver's
        // text-cell accessor.
        let mut pg_iter = pg_rows.iter();
        let rows = rows_from_cells(&columns, || {
            Ok::<_, DbError>(
                pg_iter
                    .next()
                    .map(|pg| (0..n).map(|i| pg.get::<_, Option<String>>(i)).collect()),
            )
        })?;
        Ok(PostgresSource { columns, rows })
    }
}

impl RowSource for PostgresSource {
    fn columns(&self) -> &[Column] {
        &self.columns
    }
    fn rows(&self) -> Vec<Row> {
        self.rows.clone()
    }
}

/// Connect to `conn_str` and run a batch of SQL (schema + seed data) as one script. A convenience for
/// tests and fixture builders so callers don't depend on the `postgres` crate directly. The batch is
/// expected to be idempotent (`DROP TABLE IF EXISTS` → `CREATE` → `INSERT`) so a fixture can re-seed a
/// shared server between runs.
pub fn seed(conn_str: &str, sql: &str) -> Result<(), DbError> {
    let mut client =
        postgres::Client::connect(conn_str, postgres::NoTls).map_err(DbError::Connect)?;
    client.batch_execute(sql).map_err(DbError::Query)?;
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
    pub fn connect(conn_str: &str) -> Result<PostgresConn, DbError> {
        Ok(PostgresConn {
            client: postgres::Client::connect(conn_str, postgres::NoTls)
                .map_err(DbError::Connect)?,
        })
    }

    /// Cheap liveness probe (`SELECT 1`) — the pre-fetch healthcheck. Errors if the server is
    /// unreachable or rejects the round-trip, so a bad connection fails fast.
    pub fn ping(&mut self) -> Result<(), DbError> {
        self.client
            .simple_query("SELECT 1")
            .map_err(DbError::Query)?;
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

    /// Execute an already-built [`SqlQuery`] and materialize the [`PostgresSource`].
    pub fn run(&mut self, query: &SqlQuery) -> Result<PostgresSource, DbError> {
        PostgresSource::run_query(&mut self.client, query)
    }
}

#[cfg(test)]
mod tests {
    use rpt_model::{Database, DbFieldDef, FieldValueType, Table, TableJoinType, TableLink};
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
            join_type: TableJoinType::Equal,
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
