//! Datasource handling for the CLI: enumerate the connections a report uses, validate that we can
//! supply credentials for all of them, and drive the `--db` live-fetch behind a driver abstraction.
//!
//! ## Multiple connections
//! A report is not single-connection: every [`Table`](rpt_reader::model::Table) carries its own [`ConnectionInfo`], and
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

#[cfg(feature = "db")]
use crate::applog::Comp;
#[cfg(feature = "db")]
use crate::error::RenderError;
use rpt_reader::model::{ConnectionInfo, Report};

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
#[cfg(feature = "db")]
pub fn scope_server_key(database: &rpt_reader::model::Database) -> Option<String> {
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

/// Build the SQL tracking comment for a report scope: names the originating report file and scope so
/// a query surfacing in the database's own logs (e.g. slow-query log) traces back to the report that
/// issued it. `report_label` is the report's file name; `scope` is `"main"` or a subreport tag.
#[cfg(feature = "db")]
pub fn scope_comment(report_label: &str, scope: &str) -> String {
    format!("rpt-rs report={report_label:?} scope={scope}")
}

/// The report-derived inputs that shape a scope's fetch query: the record-selection formula (pushed
/// to the `WHERE` where translatable), the scope's SQL Expression fields (selected into the query),
/// and the parameter values (bound into the pushed-down `WHERE`). Bundled so the fetch functions take
/// one query-inputs argument instead of threading the three separately. Every driver hands the whole
/// bundle to the query builder and the dialect decides how much of it it can use, so a driver's SQL
/// differs from another's only by its dialect.
#[cfg(feature = "db")]
pub struct FetchInputs<'a> {
    pub selection: Option<&'a str>,
    pub sql_exprs: &'a [(String, String)],
    pub params: &'a rpt_data::Parameters,
}

/// Fetch one report scope's rows from its live connection (resolved by the scope's server), logging
/// the connection (redacted), healthcheck, the SQL sent (verbose), and the row count/timing.
#[cfg(feature = "db")]
pub fn fetch_scope(
    report: &Report,
    inputs: &FetchInputs,
    resolver: &Resolver,
    log: &crate::applog::Log,
    comment: Option<&str>,
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
        return Ok(Box::new(rpt_data::EmptySource));
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
    let fetched = match driver {
        #[cfg(feature = "db-postgres")]
        Driver::Postgres => fetch_scope_postgres(database, inputs, url, log, comment),
        #[cfg(feature = "db-sqlite")]
        Driver::Sqlite => fetch_scope_sqlite(database, inputs, url, log, comment),
        // A driver whose backend feature is not compiled in (or has no implementation yet).
        other => Err(RenderError::Datasource(not_implemented(*other))),
    };
    // The server key is known here and nowhere below, so a multi-source report's failure says which
    // of its configured connections it came from.
    fetched.map_err(|e| e.against_source(&server_key))
}

/// Verbose-log a generated query (shared by the driver backends). `qid` is the shared query id so
/// these lines pair with the matching `executing`/`fetched` lines.
#[cfg(feature = "db")]
fn log_query(
    qid: usize,
    query: &rpt_query::SqlQuery,
    selection: Option<&str>,
    log: &crate::applog::Log,
) {
    if log.is_verbose() {
        log.detail(Comp::Data, format!("[query {qid}] SQL: {}", query.sql));
        log.detail(
            Comp::Data,
            format!("[query {qid}] columns: {}", query.columns.len()),
        );
        if selection.is_some() {
            log.detail(
                Comp::Data,
                format!(
                    "[query {qid}] record-selection formula present: whatever the dialect can \
                     translate is in the SQL WHERE (see SQL), the rest is applied per-row in-engine"
                ),
            );
        }
    }
    // A predicate the database is NOT applying means the query fetches more rows than the report
    // shows. That is correct but is the usual answer to "why is this slow", so it is a warning at
    // NORMAL rather than a detail only `-v` reveals.
    if !query.not_pushed.is_empty() {
        log.warn(
            Comp::Data,
            format!(
                "[query {qid}] {} of the record selection's condition(s) could not be pushed to SQL \
                 and are applied locally after the fetch, so the query reads more rows than the \
                 report shows: {}",
                query.not_pushed.len(),
                query.not_pushed.join("; ")
            ),
        );
    }
}

#[cfg(feature = "db-postgres")]
fn fetch_scope_postgres(
    database: &rpt_reader::model::Database,
    inputs: &FetchInputs,
    url: &str,
    log: &crate::applog::Log,
    comment: Option<&str>,
) -> Result<Box<dyn rpt_data::RowSource>, RenderError> {
    use rpt_data::RowSource;
    use rpt_db_postgres::PostgresConn;
    use rpt_query::{build_query_full, Dialect};
    use std::time::Instant;

    let selection = inputs.selection;
    // The record-selection push-down binds `{?Name}` to each parameter's current value.
    let param_pairs: Vec<(String, rpt_query::Value)> = inputs
        .params
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut query = build_query_full(
        database,
        inputs.sql_exprs,
        selection,
        &param_pairs,
        Dialect::Postgres,
    )
    // The builder's own reason, not a synthesized one — "no database table" was only ever a guess
    // that happened to match the single case that existed.
    .map_err(|e| RenderError::Datasource(e.to_string()))?;
    if let Some(c) = comment {
        query = query.with_comment(c);
    }
    let qid = log.next_query_id();
    let t0 = Instant::now();
    log.info(
        Comp::Data,
        "connecting to PostgreSQL (blocks until the server responds)…",
    );
    let mut conn = PostgresConn::connect(url)?;
    conn.ping().map_err(|e| RenderError::Db {
        context: "healthcheck (SELECT 1) failed".to_string(),
        hint: e.hint(),
        source: Box::new(e),
    })?;
    let ping_ms = t0.elapsed().as_millis();
    let version = conn
        .server_version()
        .unwrap_or_else(|| "unknown".to_string());
    log.info(
        Comp::Data,
        format!("healthcheck OK: PostgreSQL {version} ({ping_ms} ms)"),
    );
    log_query(qid, &query, selection, log);
    log.info(
        Comp::Data,
        format!(
            "[query {qid}] executing SQL query ({} column(s) across {} table(s)) — waiting for the \
             server; this blocks until every row is returned",
            query.columns.len(),
            database.tables.len(),
        ),
    );

    let t1 = Instant::now();
    let source = conn.run_typed(&query, database)?;
    log.info(
        Comp::Data,
        format!(
            "[query {qid}] fetched {} row(s) in {} ms",
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
    database: &rpt_reader::model::Database,
    inputs: &FetchInputs,
    url: &str,
    log: &crate::applog::Log,
    comment: Option<&str>,
) -> Result<Box<dyn rpt_data::RowSource>, RenderError> {
    use rpt_data::RowSource;
    use rpt_db_sqlite::{DbError, SqliteConn};
    use rpt_query::{build_query_full, Dialect};
    use std::time::Instant;

    let param_pairs: Vec<(String, rpt_query::Value)> = inputs
        .params
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    // Build the query once, then run *that* query — the logged SQL is by construction the executed
    // SQL. The full input set goes to the builder here exactly as it does on the Postgres path; only
    // the dialect differs. SQLite translates no predicate, so the builder returns the whole record
    // selection as not-pushed and the log says the query reads more rows than the report shows.
    let conn = SqliteConn::open(url)?;
    let mut query = build_query_full(
        database,
        inputs.sql_exprs,
        inputs.selection,
        &param_pairs,
        Dialect::Sqlite,
    )
    .map_err(|e| DbError::no_query(e.to_string()))?;
    if let Some(c) = comment {
        query = query.with_comment(c);
    }
    let qid = log.next_query_id();
    log_query(qid, &query, inputs.selection, log);
    log.info(
        Comp::Data,
        format!(
            "[query {qid}] executing SQL query ({} column(s) across {} table(s)) — reading rows",
            query.columns.len(),
            database.tables.len(),
        ),
    );
    let t1 = Instant::now();
    let source = conn.run(&query)?;
    log.info(
        Comp::Data,
        format!(
            "[query {qid}] fetched {} row(s) in {} ms",
            source.rows().len(),
            t1.elapsed().as_millis()
        ),
    );
    Ok(Box::new(source))
}

/// An in-memory [`RowSource`](rpt_data::RowSource) backing the subreport fetch cache: it owns one
/// fetched scope's schema and rows behind `Arc`s, so replaying a cached fetch to another subreport
/// instance is a refcount bump plus a shallow row-vector clone (each `Row`'s values are themselves
/// `Arc`-shared) rather than a second database round-trip.
#[cfg(feature = "db")]
#[derive(Clone)]
pub struct CachedRows {
    columns: std::sync::Arc<Vec<rpt_data::Column>>,
    rows: std::sync::Arc<Vec<rpt_data::Row>>,
}

#[cfg(feature = "db")]
impl CachedRows {
    /// Materialize a fetched source's schema + rows once, so later replays clone from memory.
    fn capture(src: &dyn rpt_data::RowSource) -> CachedRows {
        CachedRows {
            columns: std::sync::Arc::new(src.columns().to_vec()),
            rows: std::sync::Arc::new(src.rows()),
        }
    }
}

#[cfg(feature = "db")]
impl rpt_data::RowSource for CachedRows {
    fn columns(&self) -> &[rpt_data::Column] {
        &self.columns
    }
    fn rows(&self) -> Vec<rpt_data::Row> {
        (*self.rows).clone()
    }
}

/// A [`ScopeData`](rpt_data::ScopeData) that fetches each subreport scope's rows from its own live
/// connection, so subreports render live like the main report. A fetch failure warns and falls back
/// to the subreport's saved data rather than aborting the whole render.
///
/// The layout engine calls [`rows_for`](rpt_data::ScopeData::rows_for) once per subreport *instance*
/// (an inline subreport in a group repeats for every group row), and each call would otherwise issue
/// its own full-table query. But the parent link is applied in-memory after the fetch, never in the
/// `WHERE`, so every instance of one subreport issues the identical query — an O(instances × rows)
/// blowup on a report with many instances. [`cache`](LiveScopeData::cache) memoizes each distinct
/// fetch (keyed by the query-determining inputs) so those identical fetches collapse to one.
#[cfg(feature = "db")]
pub struct LiveScopeData<'a> {
    pub resolver: &'a Resolver,
    pub log: &'a crate::applog::Log,
    /// Parameter current-values, bound into each subreport's pushed-down `WHERE`.
    pub params: &'a rpt_data::Parameters,
    /// The report's file name, for the SQL tracking comment (subreport scopes tag `report=<label>`).
    pub report_label: &'a str,
    /// Per-render fetch memo: query-key → the fetched rows, so repeated subreport instances that
    /// issue the identical query share one round-trip. Interior-mutable because `rows_for` takes
    /// `&self`; single-threaded with the render, so a `RefCell` suffices.
    pub cache: std::cell::RefCell<std::collections::HashMap<u64, CachedRows>>,
}

#[cfg(feature = "db")]
impl rpt_data::ScopeData for LiveScopeData<'_> {
    fn rows_for(&self, report: &Report) -> Option<Box<dyn rpt_data::RowSource>> {
        // Only fetch scopes that actually have live tables; others keep their saved data.
        let server_key = scope_server_key(&report.database)?;
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
        // Cache key: the fetch is fully determined by the scope's server, record-selection, SQL
        // Expression fields, and table graph — none of which vary across the instances of one
        // subreport (the parent link is applied in-memory after the fetch, never in the WHERE), so
        // every instance issues the identical query. Params are render-fixed and so constant here.
        // Over-keying (e.g. on an unused declared table) only ever causes a fresh fetch, never wrong
        // rows, so a coarse structural fingerprint of the database is a safe key.
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            server_key.hash(&mut h);
            selection.hash(&mut h);
            sql_exprs.hash(&mut h);
            format!("{:?}", report.database).hash(&mut h);
            h.finish()
        };
        if let Some(hit) = self.cache.borrow().get(&key) {
            self.log.detail(
                Comp::Data,
                "subreport datasource: reusing a cached fetch (identical query)",
            );
            return Some(Box::new(hit.clone()));
        }
        // Tag the subreport scope with its report title when it has one, else a generic marker.
        let title = report.summary_info.title.trim();
        let scope = if title.is_empty() {
            "subreport".to_string()
        } else {
            format!("subreport:{title}")
        };
        let comment = scope_comment(self.report_label, &scope);
        let inputs = FetchInputs {
            selection,
            sql_exprs: &sql_exprs,
            params: self.params,
        };
        match fetch_scope(report, &inputs, self.resolver, self.log, Some(&comment)) {
            Ok(src) => {
                // Materialize once and memoize, so the next instance issuing this same query replays
                // it from memory instead of re-querying the database.
                let cached = CachedRows::capture(src.as_ref());
                self.cache.borrow_mut().insert(key, cached.clone());
                Some(Box::new(cached))
            }
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
            database: rpt_reader::model::Database {
                tables: conns
                    .iter()
                    .enumerate()
                    .map(|(i, c)| rpt_reader::model::Table {
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
        report.subreports = vec![rpt_reader::model::Subreport {
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

    #[cfg(feature = "db")]
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

    /// Every driver hands the query builder the whole [`FetchInputs`] bundle and lets the dialect
    /// decide what it can use. For a dialect with no predicate translator that must change only the
    /// not-pushed report, never the SQL — otherwise passing the selection would silently alter the
    /// rows a non-Postgres fetch returns.
    #[cfg(feature = "db")]
    #[test]
    fn a_dialect_without_push_down_reports_the_selection_without_changing_the_sql() {
        use rpt_query::{build_query_full, Dialect};
        let database = rpt_reader::model::Database {
            tables: vec![rpt_reader::model::Table {
                name: "countries".into(),
                alias: "countries".into(),
                data_fields: vec![rpt_reader::model::DbFieldDef {
                    name: "id".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let without = build_query_full(&database, &[], None, &[], Dialect::Sqlite).unwrap();
        let with = build_query_full(
            &database,
            &[],
            Some("{countries.id} > 1"),
            &[],
            Dialect::Sqlite,
        )
        .unwrap();

        assert_eq!(
            with.sql, without.sql,
            "the selection must not reach the SQL"
        );
        assert!(without.not_pushed.is_empty());
        assert_eq!(
            with.not_pushed,
            vec!["{countries.id} > 1".to_string()],
            "the whole selection is applied locally, and the log must be able to say so"
        );
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
