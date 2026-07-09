//! Datasource handling for the CLI: enumerate the connections a report uses, validate that we can
//! supply credentials for all of them, and drive the `--db` live-fetch behind a driver abstraction.
//!
//! ## Multiple connections
//! A report is not single-connection: every [`Table`](rpt::model::Table) carries its own [`ConnectionInfo`], and
//! subreports are full nested reports with their own tables. Each report *scope* (main + each
//! subreport) uses one server, so connections are keyed by SERVER: one connection URL per distinct
//! server, in `RPT_DB_URL_<SERVER>` (or the generic `RPT_DB_URL`/`DATABASE_URL` for a single-server
//! report). [`Resolver::build`] resolves and validates every server up front — naming the exact
//! variables to set if any are missing — before a render begins. At render time the main scope is
//! fetched directly and each subreport scope through [`LiveScopeData`].
//!
//! ## Driver abstraction
//! [`Driver`] recognizes all the intended backends (postgres, mysql, mariadb, sqlite, mssql), chosen
//! by the connection URL's scheme. Postgres and SQLite are implemented; the rest return a clear
//! "recognized but not available in this build" error.

use crate::applog::Comp;
use rpt::model::{ConnectionInfo, Report};
#[cfg(feature = "db")]
use rpt_render::RenderError;

/// A distinct data source (connection) a report reads from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataSource {
    pub server: Option<String>,
    pub database: Option<String>,
    /// `QE_DatabaseType` display string (e.g. "PostgreSQL", "ODBC (RDO)", "Field Definitions Only").
    pub db_type: Option<String>,
    pub user: Option<String>,
    /// How many tables (across main + subreports) draw from this source.
    pub table_count: usize,
}

impl DataSource {
    /// Does this source need live credentials? A real server/database source does; a
    /// field-definitions-only or empty descriptor (saved-data / no live DB) does not.
    pub fn needs_credentials(&self) -> bool {
        let is_field_defs = self
            .db_type
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case("Field Definitions Only"));
        !is_field_defs && (self.server.is_some() || self.database.is_some())
    }

    /// A one-line human description for logs/errors.
    pub fn describe(&self) -> String {
        let server = self.server.as_deref().unwrap_or("?");
        let db = self.database.as_deref().unwrap_or("?");
        let ty = self.db_type.as_deref().unwrap_or("?");
        format!(
            "{server}/{db} [{ty}] ({} table{})",
            self.table_count,
            if self.table_count == 1 { "" } else { "s" }
        )
    }

    /// The environment variable that supplies THIS source's connection URL. Keyed by the source's
    /// SERVER, so a report maps to one variable per distinct server — stable, discoverable, and
    /// printed by the CLI (no guessing). E.g. server `Sales DB` → `RPT_DB_URL_SALES_DB`.
    pub fn env_var(&self) -> String {
        format!("RPT_DB_URL_{}", self.env_key())
    }

    /// The server-based grouping identity: the server description, or the database name when there is
    /// no server. Sources sharing a `group_id` are one connection: a report's main and subreports
    /// typically hit the same server, with the subreports omitting the database name.
    fn group_id(&self) -> String {
        self.server
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| self.database.clone().filter(|s| !s.is_empty()))
            .unwrap_or_default()
    }

    fn env_key(&self) -> String {
        let key = sanitize_env_key(&self.group_id());
        if key.is_empty() {
            "DEFAULT".to_string()
        } else {
            key
        }
    }
}

/// Uppercase, keep `[A-Z0-9]`, collapse every other run into a single `_`, and trim edge `_` —
/// producing a valid, stable environment-variable-name fragment.
fn sanitize_env_key(s: &str) -> String {
    let mut out = String::new();
    let mut pending_underscore = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_underscore && !out.is_empty() {
                out.push('_');
            }
            pending_underscore = false;
            out.push(ch.to_ascii_uppercase());
        } else {
            pending_underscore = true;
        }
    }
    out
}

/// The report's sources that need live credentials (a real server/database, not field-definitions).
pub fn credential_sources(sources: &[DataSource]) -> Vec<DataSource> {
    sources
        .iter()
        .filter(|s| s.needs_credentials())
        .cloned()
        .collect()
}

/// Read a non-empty connection attribute by key.
fn attr<'a>(conn: &'a ConnectionInfo, key: &str) -> Option<&'a str> {
    conn.attributes
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .filter(|s| !s.is_empty())
}

/// The (server, database, type, user) identity of a connection.
fn identity(conn: &ConnectionInfo) -> DataSource {
    DataSource {
        server: attr(conn, "QE_ServerDescription").map(str::to_string),
        database: attr(conn, "QE_DatabaseName").map(str::to_string),
        db_type: attr(conn, "QE_DatabaseType").map(str::to_string),
        user: conn.user_name.clone(),
        table_count: 0,
    }
}

/// Enumerate the distinct data sources a report uses, across the main report and all subreports.
pub fn enumerate(report: &Report) -> Vec<DataSource> {
    let mut acc: Vec<DataSource> = Vec::new();
    collect(report, &mut acc);
    acc
}

fn collect(report: &Report, acc: &mut Vec<DataSource>) {
    for t in &report.database.tables {
        let id = identity(&t.connection);
        let gid = id.group_id();
        match acc.iter_mut().find(|d| d.group_id() == gid) {
            Some(existing) => {
                existing.table_count += 1;
                // Keep the most informative database label if the first table's was blank (a
                // subreport connection often omits the database name the main scope carries).
                if existing.database.as_deref().unwrap_or("").is_empty() && id.database.is_some() {
                    existing.database = id.database;
                }
            }
            None => acc.push(DataSource {
                table_count: 1,
                ..id
            }),
        }
    }
    for sr in &report.subreports {
        collect(&sr.report, acc);
    }
}

/// The server key of a report scope (its first credential-needing table's server), or `None` when
/// the scope has no live tables (nothing to fetch — falls back to saved data).
#[cfg_attr(not(feature = "db"), allow(dead_code))]
pub fn scope_server_key(database: &rpt::model::Database) -> Option<String> {
    database.tables.iter().find_map(|t| {
        let id = identity(&t.connection);
        id.needs_credentials().then(|| id.group_id())
    })
}

/// A live-database backend, selected by the connection URL's scheme. DB-path only, so it is compiled
/// out when no driver feature is on.
#[cfg(feature = "db")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    Postgres,
    MySql,
    MariaDb,
    Sqlite,
    MsSql,
}

#[cfg(feature = "db")]
impl Driver {
    /// Select the backend from a connection URL's scheme (the universal `DATABASE_URL` convention:
    /// `postgres://…`, `mysql://…`, `sqlite://…`). Recognizes every intended backend even though
    /// only [`Postgres`](Driver::Postgres) is implemented.
    pub fn from_url(url: &str) -> Result<Driver, RenderError> {
        let scheme = match url.split_once("://") {
            Some((s, _)) if !s.is_empty() => s.to_ascii_lowercase(),
            _ => {
                return Err(RenderError::Datasource(format!(
                    "database URL must be scheme://… (e.g. postgres://user:pass@host:5432/db), got {url:?}"
                )))
            }
        };
        match scheme.as_str() {
            "postgres" | "postgresql" => Ok(Driver::Postgres),
            "mysql" => Ok(Driver::MySql),
            "mariadb" => Ok(Driver::MariaDb),
            "sqlite" | "sqlite3" => Ok(Driver::Sqlite),
            "mssql" | "sqlserver" => Ok(Driver::MsSql),
            other => Err(RenderError::Datasource(format!(
                "unknown database URL scheme {other:?} (expected: postgres, mysql, mariadb, sqlite, mssql)"
            ))),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Driver::Postgres => "postgres",
            Driver::MySql => "mysql",
            Driver::MariaDb => "mariadb",
            Driver::Sqlite => "sqlite",
            Driver::MsSql => "mssql",
        }
    }

    /// Whether the driver is recognized (its scheme is understood) but not yet implemented, as
    /// opposed to implemented-but-not-compiled-in for this build.
    fn recognized_unimplemented(self) -> bool {
        match self {
            // Postgres + SQLite are implemented (behind the db-postgres / db-sqlite features).
            Driver::Postgres | Driver::Sqlite => false,
            Driver::MySql | Driver::MariaDb | Driver::MsSql => true,
        }
    }
}

/// The "driver not implemented yet" message.
#[cfg(feature = "db")]
pub fn not_implemented(driver: Driver) -> String {
    if driver.recognized_unimplemented() {
        format!(
            "the {} driver is recognized but not available in this build; \
             postgres and sqlite are implemented",
            driver.name()
        )
    } else {
        format!(
            "the {} driver is not available in this build",
            driver.name()
        )
    }
}

/// Redact the password from a connection string for logging. Handles both forms the postgres client
/// accepts: the libpq `key=value` form (drops any `password=…` token) and the URL form
/// `scheme://user:password@host/db` (masks the userinfo password).
#[cfg(feature = "db")]
fn redacted_summary(conn: &str) -> String {
    conn.split_whitespace()
        .filter_map(redact_token)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Redact one whitespace-separated connection token: `None` drops it, `Some` keeps it (masked).
#[cfg(feature = "db")]
fn redact_token(tok: &str) -> Option<String> {
    // libpq `password=secret` → dropped entirely.
    if tok.to_ascii_lowercase().starts_with("password=") {
        return None;
    }
    // URL `scheme://user:secret@host/db` → mask the userinfo password (between the first ':' after
    // `://` and the first '@'). No `:` in the userinfo means no password to hide.
    if let Some(sep) = tok.find("://") {
        let (prefix, after) = tok.split_at(sep + 3);
        if let Some(at) = after.find('@') {
            let (userinfo, rest) = after.split_at(at); // rest starts with '@'
            if let Some((user, _password)) = userinfo.split_once(':') {
                return Some(format!("{prefix}{user}:***{rest}"));
            }
        }
    }
    Some(tok.to_string())
}

/// The universal DB-connection env vars, in precedence order. Each carries a full connection URL
/// whose scheme selects the backend (`postgres://…`, `mysql://…`, `sqlite://…`). Keeping the URL
/// (and any embedded password) in the environment rather than argv is the secure-by-default channel;
/// securing the environment itself is the user's responsibility.
#[cfg(feature = "db")]
const DB_URL_VARS: [&str; 2] = ["RPT_DB_URL", "DATABASE_URL"];

/// A resolved set of live connections: server `group_id` → (driver, connection URL). Built once from
/// the report + environment and validated up front, so a render never starts with a missing
/// connection. One entry per distinct server the report reads from (main report + all subreports).
#[cfg(feature = "db")]
pub struct Resolver {
    by_server: std::collections::HashMap<String, (Driver, String)>,
}

#[cfg(feature = "db")]
impl Resolver {
    /// Resolve every credential-needing server the report uses to a connection URL from the
    /// environment (`RPT_DB_URL_<SERVER>`; the generic `RPT_DB_URL`/`DATABASE_URL` is also accepted
    /// when the report uses exactly one server). Errors — before any render — naming the exact
    /// variables to set when any are missing.
    pub fn build(report: &Report) -> Result<Resolver, RenderError> {
        let needing = credential_sources(&enumerate(report));
        let allow_global = needing.len() == 1;
        let mut by_server = std::collections::HashMap::new();
        let mut missing = Vec::new();
        for s in &needing {
            match env_url_for(s, allow_global)? {
                Some(entry) => {
                    by_server.insert(s.group_id(), entry);
                }
                None => missing.push(s.clone()),
            }
        }
        if !missing.is_empty() {
            return Err(RenderError::Datasource(missing_urls_error(
                &missing,
                allow_global,
            )));
        }
        Ok(Resolver { by_server })
    }

    fn get(&self, server_key: &str) -> Option<&(Driver, String)> {
        self.by_server.get(server_key)
    }
}

/// Look up one source's connection URL: its server-keyed variable, or (single-server reports only)
/// the generic `RPT_DB_URL`/`DATABASE_URL`. Returns the parsed driver + URL, `None` if unset. Errors
/// only on a malformed URL/scheme.
#[cfg(feature = "db")]
fn env_url_for(
    source: &DataSource,
    allow_global: bool,
) -> Result<Option<(Driver, String)>, RenderError> {
    let mut keys = vec![source.env_var()];
    if allow_global {
        keys.extend(DB_URL_VARS.iter().map(|s| s.to_string()));
    }
    for key in keys {
        if let Ok(url) = std::env::var(&key) {
            let url = url.trim();
            if !url.is_empty() {
                let driver = Driver::from_url(url)?;
                return Ok(Some((driver, url.to_string())));
            }
        }
    }
    Ok(None)
}

/// The up-front, pre-render error naming the exact environment variable to set for each unresolved
/// server — so the required setup is unambiguous, never guessed.
#[cfg(feature = "db")]
fn missing_urls_error(missing: &[DataSource], allow_global: bool) -> String {
    let mut msg = format!(
        "missing a database connection URL for {} data source(s). Set each in the environment \
         (the URL scheme selects the backend):\n",
        missing.len()
    );
    for s in missing {
        msg.push_str(&format!(
            "  {}\n    export {}='postgres://user:pass@host:5432/dbname'\n",
            s.describe(),
            s.env_var()
        ));
    }
    if allow_global {
        msg.push_str(
            "(this single-source report also accepts the generic RPT_DB_URL / DATABASE_URL.)\n",
        );
    }
    msg.push_str("Run `rpt-render <file> --list-sources` to see this list.");
    msg
}

/// Fetch one report scope's rows from its live connection (resolved by the scope's server), logging
/// the connection (redacted), healthcheck, the SQL sent (verbose), and the row count/timing.
#[cfg(feature = "db")]
pub fn fetch_scope(
    report: &Report,
    selection: Option<&str>,
    sql_exprs: &[(String, String)],
    params: &rpt_data::Parameters,
    resolver: &Resolver,
    log: &crate::applog::Log,
) -> Result<Box<dyn rpt_data::RowSource>, RenderError> {
    // Prune to only the tables/columns the report references, so declared-but-unused tables are not
    // pulled into the FROM (and cross-joined into a cartesian). The engine fetches only used fields.
    let database =
        rpt_query::prune_database(&report.database, &rpt_query::used_database_fields(report));
    let database = &database;
    // A scope with no live table (e.g. a report bound only to saved data / field-definitions, or an
    // empty base report) has nothing to query — render its static bands from an empty dataset rather
    // than failing, matching the engine (which lays out a near-empty page).
    let Some(server_key) = scope_server_key(database) else {
        log.warn(
            Comp::Data,
            "no live datasource in this scope; rendering static bands from an empty dataset",
        );
        return Ok(Box::new(EmptyRowSource));
    };
    let (driver, url) = resolver.get(&server_key).ok_or_else(|| {
        RenderError::Datasource(format!(
            "internal: no resolved connection for server {server_key:?}"
        ))
    })?;

    log.info(
        Comp::Data,
        format!("datasource: {} ({})", driver.name(), redacted_summary(url)),
    );
    // `selection` (the record-selection formula) and `params` are only consumed by the Postgres
    // WHERE push-down; the SQLite/other paths fetch the full table and filter in-pipeline.
    #[cfg(not(feature = "db-postgres"))]
    let _ = (selection, params);
    // `sql_exprs` is consumed by every concrete driver — unused only when none is compiled in.
    #[cfg(not(any(feature = "db-postgres", feature = "db-sqlite")))]
    let _ = sql_exprs;

    match driver {
        #[cfg(feature = "db-postgres")]
        Driver::Postgres => fetch_scope_postgres(database, selection, sql_exprs, params, url, log),
        #[cfg(feature = "db-sqlite")]
        Driver::Sqlite => fetch_scope_sqlite(database, sql_exprs, url, log),
        // A driver whose backend feature is not compiled in (or has no implementation yet).
        other => Err(RenderError::Datasource(not_implemented(*other))),
    }
}

/// A row source with no columns and no rows: a scope with no live datasource still renders its static
/// bands (a report bound only to saved data / field-definitions, or an empty base report).
#[cfg(feature = "db")]
struct EmptyRowSource;

#[cfg(feature = "db")]
impl rpt_data::RowSource for EmptyRowSource {
    fn columns(&self) -> &[rpt_data::Column] {
        &[]
    }
    fn rows(&self) -> Vec<rpt_data::Row> {
        Vec::new()
    }
}

/// Verbose-log a generated query (shared by the driver backends).
#[cfg(feature = "db")]
fn log_query(query: &rpt_query::SqlQuery, selection: Option<&str>, log: &crate::applog::Log) {
    if log.is_verbose() {
        log.detail(Comp::Data, format!("SQL: {}", query.sql));
        log.detail(Comp::Data, format!("columns: {}", query.columns.len()));
        if selection.is_some() {
            log.detail(
                Comp::Data,
                "record-selection formula present: translatable part pushed to SQL WHERE \
                 (see SQL), remainder applied per-row in-engine",
            );
        }
    }
}

#[cfg(feature = "db-postgres")]
fn fetch_scope_postgres(
    database: &rpt::model::Database,
    selection: Option<&str>,
    sql_exprs: &[(String, String)],
    params: &rpt_data::Parameters,
    url: &str,
    log: &crate::applog::Log,
) -> Result<Box<dyn rpt_data::RowSource>, RenderError> {
    use rpt_data::RowSource;
    use rpt_db_postgres::PostgresConn;
    use rpt_query::{build_query_full, Dialect};
    use std::time::Instant;

    // The record-selection push-down binds `{?Name}` to each parameter's current value.
    let param_pairs: Vec<(String, rpt_query::Value)> =
        params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let query = build_query_full(
        database,
        sql_exprs,
        selection,
        &param_pairs,
        Dialect::Postgres,
    )
    .ok_or_else(|| RenderError::Datasource("report has no database table to query".to_string()))?;
    let t0 = Instant::now();
    log.info(
        Comp::Data,
        "connecting to PostgreSQL (blocks until the server responds)…",
    );
    let mut conn = PostgresConn::connect(url)?;
    conn.ping()
        .map_err(|e| RenderError::Db(format!("healthcheck (SELECT 1) failed: {e}")))?;
    let ping_ms = t0.elapsed().as_millis();
    let version = conn
        .server_version()
        .unwrap_or_else(|| "unknown".to_string());
    log.info(
        Comp::Data,
        format!("healthcheck OK: PostgreSQL {version} ({ping_ms} ms)"),
    );
    log_query(&query, selection, log);
    log.info(
        Comp::Data,
        format!(
            "executing SQL query ({} column(s) across {} table(s)) — waiting for the server; \
             this blocks until every row is returned",
            query.columns.len(),
            database.tables.len(),
        ),
    );

    let t1 = Instant::now();
    let source = conn.run(&query)?;
    log.info(
        Comp::Data,
        format!(
            "fetched {} row(s) in {} ms",
            source.rows().len(),
            t1.elapsed().as_millis()
        ),
    );
    Ok(Box::new(source))
}

/// Fetch a scope's rows from an in-process SQLite database (`sqlite://…`). The SQLite dialect fetches
/// the full table (no WHERE push-down); the pipeline applies any record-selection formula per row.
#[cfg(feature = "db-sqlite")]
fn fetch_scope_sqlite(
    database: &rpt::model::Database,
    sql_exprs: &[(String, String)],
    url: &str,
    log: &crate::applog::Log,
) -> Result<Box<dyn rpt_data::RowSource>, RenderError> {
    use rpt_data::RowSource;
    use rpt_db_sqlite::SqliteSource;
    use rpt_query::{build_query_full, Dialect};
    use std::time::Instant;

    if let Some(query) = build_query_full(database, sql_exprs, None, &[], Dialect::Sqlite) {
        log_query(&query, None, log);
        log.info(
            Comp::Data,
            format!(
                "executing SQL query ({} column(s) across {} table(s)) — reading rows",
                query.columns.len(),
                database.tables.len(),
            ),
        );
    }
    let t1 = Instant::now();
    let source = SqliteSource::fetch(url, database, sql_exprs)?;
    log.info(
        Comp::Data,
        format!(
            "fetched {} row(s) in {} ms",
            source.rows().len(),
            t1.elapsed().as_millis()
        ),
    );
    Ok(Box::new(source))
}

/// A [`ScopeData`](rpt_data::ScopeData) that fetches each subreport scope's rows from its own live
/// connection, so subreports render live like the main report. A fetch failure warns and falls back
/// to the subreport's saved data rather than aborting the whole render.
#[cfg(feature = "db")]
pub struct LiveScopeData<'a> {
    pub resolver: &'a Resolver,
    pub log: &'a crate::applog::Log,
    /// Parameter current-values, bound into each subreport's pushed-down `WHERE`.
    pub params: &'a rpt_data::Parameters,
}

#[cfg(feature = "db")]
impl rpt_data::ScopeData for LiveScopeData<'_> {
    fn rows_for(&self, report: &Report) -> Option<Box<dyn rpt_data::RowSource>> {
        // Only fetch scopes that actually have live tables; others keep their saved data.
        scope_server_key(&report.database)?;
        let selection = report
            .data_definition
            .record_selection
            .as_ref()
            .map(|f| f.0.as_str());
        // The subreport's own SQL Expression fields.
        let sql_exprs: Vec<(String, String)> = report
            .data_definition
            .sql_expression_fields()
            .map(|(f, x)| (f.name.clone(), x.text.clone()))
            .collect();
        match fetch_scope(
            report,
            selection,
            &sql_exprs,
            self.params,
            self.resolver,
            self.log,
        ) {
            Ok(src) => Some(src),
            Err(e) => {
                self.log.warn(
                    Comp::Data,
                    format!("subreport datasource unavailable ({e}); using saved data"),
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(server: &str, db: &str, ty: &str) -> ConnectionInfo {
        ConnectionInfo {
            attributes: vec![
                ("QE_ServerDescription".into(), server.into()),
                ("QE_DatabaseName".into(), db.into()),
                ("QE_DatabaseType".into(), ty.into()),
            ],
            ..Default::default()
        }
    }

    fn report_with(conns: &[ConnectionInfo]) -> Report {
        Report {
            database: rpt::model::Database {
                tables: conns
                    .iter()
                    .enumerate()
                    .map(|(i, c)| rpt::model::Table {
                        name: format!("t{i}"),
                        alias: format!("t{i}"),
                        connection: c.clone(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn sources_grouped_by_server_across_scopes() {
        // Same server "db1" in the main scope (with a db name) and a subreport (blank db name) is
        // ONE source; "db2" is a second.
        let mut report = report_with(&[
            conn("db1", "app", "PostgreSQL"),
            conn("db2", "app", "PostgreSQL"),
        ]);
        report.subreports = vec![rpt::model::Subreport {
            name: "s".into(),
            report: Box::new(report_with(&[conn("db1", "", "PostgreSQL")])),
            ..Default::default()
        }];

        let sources = enumerate(&report);
        assert_eq!(sources.len(), 2, "grouped by server, not server+db");
        let db1 = sources
            .iter()
            .find(|s| s.server.as_deref() == Some("db1"))
            .unwrap();
        assert_eq!(db1.table_count, 2, "main + subreport table on db1");
    }

    #[test]
    fn env_var_is_server_keyed() {
        let sources = enumerate(&report_with(&[conn("Sales DB", "sales", "ODBC (RDO)")]));
        assert_eq!(sources[0].env_var(), "RPT_DB_URL_SALES_DB");
    }

    #[test]
    fn credential_sources_excludes_field_definitions() {
        let sources = enumerate(&report_with(&[conn("", "", "Field Definitions Only")]));
        assert!(credential_sources(&sources).is_empty());
    }

    #[test]
    fn scope_server_key_reads_first_live_table() {
        let report = report_with(&[conn("Sales DB", "sales", "ODBC (RDO)")]);
        assert_eq!(
            scope_server_key(&report.database).as_deref(),
            Some("Sales DB")
        );
    }

    #[cfg(feature = "db")]
    #[test]
    fn driver_from_url_scheme() {
        assert_eq!(
            Driver::from_url("postgres://u:p@h:5432/db").unwrap(),
            Driver::Postgres
        );
        assert_eq!(
            Driver::from_url("postgresql://h/db").unwrap(),
            Driver::Postgres
        );
        assert_eq!(Driver::from_url("mysql://h/db").unwrap(), Driver::MySql);
        assert_eq!(
            Driver::from_url("sqlite:///tmp/x.db").unwrap(),
            Driver::Sqlite
        );
        assert_eq!(Driver::from_url("sqlserver://h/db").unwrap(), Driver::MsSql);
        // unknown scheme and non-URL both error.
        assert!(Driver::from_url("oracle://h/db").is_err());
        assert!(Driver::from_url("host=h dbname=db").is_err());
    }

    #[cfg(feature = "db")]
    #[test]
    fn unimplemented_drivers_are_recognized_but_unavailable() {
        // Postgres + SQLite are implemented; the rest are recognized but not yet available.
        assert!(not_implemented(Driver::MySql).contains("recognized but not available"));
        assert!(not_implemented(Driver::MsSql).contains("recognized but not available"));
    }

    #[cfg(feature = "db")]
    #[test]
    fn redaction_hides_password_in_both_forms() {
        // libpq key=value form: the password token is dropped entirely.
        let libpq = super::redacted_summary("host=db user=rpt password=secret dbname=sales");
        assert_eq!(libpq, "host=db user=rpt dbname=sales");
        assert!(!libpq.contains("secret"));

        // URL form: the userinfo password is masked but the rest is preserved.
        let url = super::redacted_summary("postgres://rpt:secret@db.internal:5432/sales");
        assert_eq!(url, "postgres://rpt:***@db.internal:5432/sales");
        assert!(!url.contains("secret"));

        // URL with no password is left intact.
        assert_eq!(
            super::redacted_summary("postgres://rpt@db/sales"),
            "postgres://rpt@db/sales"
        );
    }
}
