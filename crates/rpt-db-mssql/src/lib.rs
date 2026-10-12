//! # rpt-db-mssql — a live SQL Server [`RowSource`]
//!
//! Given a report's decoded database schema and a `mssql://` URL, it generates T-SQL via
//! [`rpt_query`] (the [`Dialect::MsSql`] dialect), executes it over TDS with the pure-Rust
//! [`tiberius`] client, and returns a [`RowSource`] the [`rpt_data`] pipeline consumes exactly like the
//! saved-data, PostgreSQL and SQLite paths.
//!
//! As in the other drivers, every column is fetched as **text** and re-typed by
//! [`rpt_data::cell_to_value`] against the report's declared field type; the dialect already converts
//! each type to the text form `rpt-data` parses (see `Dialect::MsSql`), so there is no per-SQL-Server-type
//! extraction code here. A binary column is the exception and arrives as bytes.
//!
//! `tiberius` is async. The driver owns a private single-threaded runtime and blocks on it, so the public
//! API stays synchronous like its siblings and callers need no async runtime.
//!
//! ## URL
//!
//! `mssql://user:password@host[:port]/database[?encrypt=…&trust_cert=…]` (or `sqlserver://`). User and
//! password are percent-decoded. SQL Server logins only (no integrated authentication). The port
//! defaults to 1433; named instances are reached by port. Options:
//!
//! - `encrypt` — `required` (default; TLS for everything), `on`, `off` (TLS for the login only, if the
//!   server offers it) or `none` (no TLS at all).
//! - `trust_cert` — `true` accepts any server certificate (a self-signed test server); the default
//!   validates it against the system roots.
//! - `app_name` — the application name shown in the server's session list.

use rpt_data::{Cell, Column, Row, RowData, RowSource};
use rpt_model::{Database, Report};
use rpt_query::{build_query_for_report, build_query_full, Dialect, SqlQuery, Value};
use std::time::Duration;
use tiberius::{AuthMethod, Client, ColumnData, Config, EncryptionLevel};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

/// How long to wait for the TCP connection before giving up, so an unreachable host fails instead of
/// hanging a render.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// What went wrong below the shared [`DbError`] wrapper.
#[derive(Debug, thiserror::Error)]
pub enum MssqlError {
    /// The URL could not be understood. The message never repeats the URL, which carries the password.
    #[error("invalid SQL Server URL: {0}")]
    Url(String),
    /// The TCP connection failed or timed out.
    #[error("network error: {0}")]
    Io(#[from] std::io::Error),
    /// The server or the TDS protocol reported an error (login, TLS, syntax, a missing table, …).
    #[error("{0}")]
    Tds(Box<tiberius::error::Error>),
    /// A result column came back in a type the driver does not read. Every column the query builder
    /// emits is converted to text (or is binary), so this means hand-written SQL selected something else.
    #[error("unsupported result column type: {0}")]
    Unsupported(&'static str),
}

// Boxed: tiberius' error is large enough to make every `Result` in the crate trip `clippy::result_large_err`.
impl From<tiberius::error::Error> for MssqlError {
    fn from(e: tiberius::error::Error) -> MssqlError {
        MssqlError::Tds(Box::new(e))
    }
}

/// A failure of the SQL Server path — the shared [`rpt_data::DbError`] aliased to this driver's error
/// type. Constructed through its `no_query` / `connect` / `query` helpers.
pub type DbError = rpt_data::DbError<MssqlError>;

/// A [`RowSource`] backed by a T-SQL query over a report's linked tables.
#[derive(Debug, Clone)]
pub struct MssqlSource(RowData);

impl MssqlSource {
    /// Connect to the server at `url` and fetch the report's tables (joined per the link graph),
    /// re-typing every value against the report's declared field types.
    ///
    /// `sql_exprs` are the report's SQL Expression fields (`(name, text)`), `selection`/`params` the
    /// record-selection push-down (the translatable part lands in the `WHERE`, the rest is applied per
    /// row by the pipeline), and `comment`, when set, is prepended as a `/* … */` tracking comment.
    ///
    /// # Errors
    ///
    /// - [`DbError::NoQuery`] — no query could be built for the report (it binds no table).
    /// - [`DbError::Connect`] — the URL is invalid or the server could not be reached or refused the login.
    /// - [`DbError::Query`] — the statement failed; the error carries the SQL (see [`DbError::hint`]).
    pub fn fetch(
        url: &str,
        database: &Database,
        sql_exprs: &[(String, String)],
        selection: Option<&str>,
        params: &[(String, Value)],
        comment: Option<&str>,
    ) -> Result<MssqlSource, DbError> {
        let mut query = build_query_full(database, sql_exprs, selection, params, Dialect::MsSql)
            .map_err(|e| DbError::no_query(e.to_string()))?;
        if let Some(c) = comment {
            query = query.with_comment(c);
        }
        MssqlConn::open(url)?.run(&query)
    }

    /// Like [`fetch`](Self::fetch) but prunes the query to the tables and columns the `report` actually
    /// references, so declared-but-unused tables are not cross-joined into the `FROM`.
    ///
    /// # Errors
    ///
    /// As [`fetch`](Self::fetch).
    pub fn fetch_for_report(
        url: &str,
        report: &Report,
        selection: Option<&str>,
        sql_exprs: &[(String, String)],
        params: &[(String, Value)],
    ) -> Result<MssqlSource, DbError> {
        let query = build_query_for_report(report, sql_exprs, selection, params, Dialect::MsSql)
            .map_err(|e| DbError::no_query(e.to_string()))?;
        MssqlConn::open(url)?.run(&query)
    }
}

impl RowSource for MssqlSource {
    fn columns(&self) -> &[Column] {
        self.0.columns()
    }
    fn rows(&self) -> Vec<Row> {
        self.0.rows()
    }
}

/// An open SQL Server connection, split from the fetch so a caller (the render CLI) can order the steps
/// itself: [`open`](Self::open) → healthcheck → build and log the SQL → [`run`](Self::run). This mirrors
/// `PostgresConn` / `SqliteConn`, so the CLI runs the very query it logged.
pub struct MssqlConn {
    rt: Runtime,
    client: Client<Compat<TcpStream>>,
}

impl std::fmt::Debug for MssqlConn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MssqlConn").finish_non_exhaustive()
    }
}

impl MssqlConn {
    /// Connect to the server named by `url` (see the [crate docs](crate) for the form).
    ///
    /// # Errors
    ///
    /// [`DbError::Connect`] if the URL is invalid, the server cannot be reached within 15 seconds, TLS
    /// negotiation fails, or the login is refused.
    pub fn open(url: &str) -> Result<MssqlConn, DbError> {
        let config = parse_url(url).map_err(DbError::connect)?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| DbError::connect(e.into()))?;
        let client = rt.block_on(connect(config)).map_err(DbError::connect)?;
        Ok(MssqlConn { rt, client })
    }

    /// Run `SELECT 1`, to prove the connection answers before the real query is sent.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the server does not answer.
    pub fn ping(&mut self) -> Result<(), DbError> {
        self.scalar("SELECT 1").map(|_| ())
    }

    /// The server's version banner (the first line of `@@VERSION`), or `None` if it cannot be read.
    pub fn server_version(&mut self) -> Option<String> {
        let banner = self.scalar("SELECT @@VERSION").ok()??;
        banner.lines().next().map(|l| l.trim().to_string())
    }

    /// Execute an already-built [`SqlQuery`] and materialize the [`MssqlSource`]. Rows are keyed by
    /// each column's `alias.field` name (how formulas reference them).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] if the statement fails or a column has an unsupported type. The error
    /// carries the SQL (see [`DbError::hint`]).
    pub fn run(&mut self, query: &SqlQuery) -> Result<MssqlSource, DbError> {
        self.run_inner(query)
            .map_err(|e| e.in_context(None, Some(&query.sql)))
    }

    fn run_inner(&mut self, query: &SqlQuery) -> Result<MssqlSource, DbError> {
        let columns = query.result_columns();
        let rows = self
            .rt
            .block_on(async {
                self.client
                    .simple_query(query.sql.as_str())
                    .await?
                    .into_first_result()
                    .await
            })
            .map_err(|e| DbError::query(e.into()))?;
        let mut pending = rows.into_iter();
        let data = RowData::from_cells(columns, || -> Result<_, DbError> {
            let Some(row) = pending.next() else {
                return Ok(None);
            };
            row.into_iter()
                .map(|d| cell(d).map_err(DbError::query))
                .collect::<Result<Vec<_>, _>>()
                .map(Some)
        })?;
        Ok(MssqlSource(data))
    }

    /// The first column of the first row of `sql` as text.
    fn scalar(&mut self, sql: &str) -> Result<Option<String>, DbError> {
        let rows = self
            .rt
            .block_on(async {
                self.client
                    .simple_query(sql)
                    .await?
                    .into_first_result()
                    .await
            })
            .map_err(|e| DbError::query(e.into()).in_context(None, Some(sql)))?;
        let first = rows
            .into_iter()
            .next()
            .and_then(|r| r.into_iter().next())
            .map(cell)
            .transpose()
            .map_err(DbError::query)?;
        Ok(match first {
            Some(Some(Cell::Text(t))) => Some(t),
            _ => None,
        })
    }
}

/// Open the TCP connection (with a timeout) and run the TDS login.
async fn connect(config: Config) -> Result<Client<Compat<TcpStream>>, MssqlError> {
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(config.get_addr()))
        .await
        .map_err(|_| {
            MssqlError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "timed out connecting to the server",
            ))
        })??;
    tcp.set_nodelay(true)?;
    Ok(Client::connect(config, tcp.compat_write()).await?)
}

/// Convert one result value to the cell the pipeline re-types. Text and binary are the two forms the
/// query builder produces; plain numbers are accepted too, for hand-written SQL Expression fields.
fn cell(data: ColumnData<'static>) -> Result<Option<Cell>, MssqlError> {
    let text = |s: String| Some(Cell::Text(s));
    Ok(match data {
        ColumnData::String(s) => s.map(|s| Cell::Text(s.into_owned())),
        ColumnData::Binary(b) => b.map(|b| Cell::Bytes(b.into_owned())),
        ColumnData::Guid(g) => g.map(|g| Cell::Text(g.to_string())),
        ColumnData::Bit(b) => b.and_then(|b| text(b.to_string())),
        ColumnData::U8(v) => v.and_then(|v| text(v.to_string())),
        ColumnData::I16(v) => v.and_then(|v| text(v.to_string())),
        ColumnData::I32(v) => v.and_then(|v| text(v.to_string())),
        ColumnData::I64(v) => v.and_then(|v| text(v.to_string())),
        ColumnData::F32(v) => v.and_then(|v| text(v.to_string())),
        ColumnData::F64(v) => v.and_then(|v| text(v.to_string())),
        ColumnData::Numeric(v) => v.and_then(|v| text(v.to_string())),
        ColumnData::Xml(_) => return Err(MssqlError::Unsupported("xml")),
        _ => {
            return Err(MssqlError::Unsupported(
                "date/time (select it converted to text)",
            ))
        }
    })
}

/// Parse a `mssql://` URL into a client configuration. Errors never echo the URL (it holds the password).
fn parse_url(url: &str) -> Result<Config, MssqlError> {
    let bad = |what: &str| MssqlError::Url(what.to_string());
    let rest = url
        .split_once("://")
        .filter(|(scheme, _)| {
            scheme.eq_ignore_ascii_case("mssql") || scheme.eq_ignore_ascii_case("sqlserver")
        })
        .map(|(_, rest)| rest)
        .ok_or_else(|| bad("expected mssql://user:password@host[:port]/database"))?;
    let (authority_path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (authority, path) = authority_path
        .split_once('/')
        .unwrap_or((authority_path, ""));
    // The password may itself contain `@`, so the host part starts after the *last* one.
    let (userinfo, hostport) = authority
        .rsplit_once('@')
        .ok_or_else(|| bad("a SQL Server login is required (user:password@host)"))?;
    let (user, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
    if user.is_empty() {
        return Err(bad("the user name is empty"));
    }
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => (
            h,
            p.parse::<u16>()
                .map_err(|_| bad("the port is out of range"))?,
        ),
        _ => (hostport, 1433),
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return Err(bad("the host is empty"));
    }

    let mut config = Config::new();
    config.host(host);
    config.port(port);
    config.authentication(AuthMethod::sql_server(
        percent_decode(user),
        percent_decode(password),
    ));
    let database = percent_decode(path);
    if !database.is_empty() {
        config.database(database);
    }
    config.encryption(EncryptionLevel::Required);
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode(value);
        match key.to_ascii_lowercase().as_str() {
            "encrypt" => {
                config.encryption(match value.to_ascii_lowercase().as_str() {
                    "required" | "true" | "yes" => EncryptionLevel::Required,
                    "on" => EncryptionLevel::On,
                    "off" => EncryptionLevel::Off,
                    "none" | "false" | "no" => EncryptionLevel::NotSupported,
                    _ => return Err(bad("encrypt must be required, on, off or none")),
                });
            }
            "trust_cert" => {
                if matches!(value.to_ascii_lowercase().as_str(), "true" | "yes" | "1") {
                    config.trust_cert();
                }
            }
            "app_name" => config.application_name(value),
            other => return Err(bad(&format!("unknown option {other:?}"))),
        }
    }
    Ok(config)
}

/// Decode `%XX` escapes (a password with `@`, `:` or `/` must be written that way); everything else,
/// including a stray `%`, is kept as is.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_url_parses() {
        let c =
            parse_url("mssql://sa:p%40ss@db.example.com:14330/Shop?encrypt=off&trust_cert=true")
                .unwrap();
        assert_eq!(c.get_addr(), "db.example.com:14330");
    }

    #[test]
    fn port_defaults_to_1433_and_scheme_alias_works() {
        let c = parse_url("sqlserver://sa:x@localhost/db").unwrap();
        assert_eq!(c.get_addr(), "localhost:1433");
    }

    #[test]
    fn password_may_contain_at_sign() {
        let c = parse_url("mssql://sa:p@ss@host/db").unwrap();
        assert_eq!(c.get_addr(), "host:1433");
    }

    #[test]
    fn invalid_urls_are_rejected_without_echoing_the_password() {
        for url in [
            "postgres://sa:secret-pw@h/db",
            "mssql://host/db",
            "mssql://:secret-pw@host/db",
            "mssql://sa:secret-pw@host:99999/db",
            "mssql://sa:secret-pw@host/db?encrypt=maybe",
            "mssql://sa:secret-pw@host/db?bogus=1",
        ] {
            let err = parse_url(url).unwrap_err().to_string();
            assert!(!err.contains("secret-pw"), "{err}");
        }
    }

    #[test]
    fn percent_decoding_handles_escapes_and_strays() {
        assert_eq!(percent_decode("a%40b%3Ac"), "a@b:c");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%C3%86"), "Æ");
    }

    #[test]
    fn cells_map_text_binary_and_null() {
        assert_eq!(
            cell(ColumnData::String(Some("x".into()))).unwrap(),
            Some(Cell::Text("x".into()))
        );
        assert_eq!(
            cell(ColumnData::Binary(Some(vec![1u8, 2].into()))).unwrap(),
            Some(Cell::Bytes(vec![1, 2]))
        );
        assert_eq!(cell(ColumnData::String(None)).unwrap(), None);
        assert_eq!(
            cell(ColumnData::I32(Some(7))).unwrap(),
            Some(Cell::Text("7".into()))
        );
    }
}
