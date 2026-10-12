//! Live test against a real SQL Server. Skipped (passes trivially) unless `RPT_MSSQL_TEST_URL` is set to
//! a `mssql://…` URL, so the ordinary test run needs no server. It only issues `SELECT`s over literals.

use rpt_data::RowSource;
use rpt_db_mssql::MssqlConn;
use rpt_formula::eval::Value;
use rpt_model::FieldValueType;
use rpt_query::{QueryColumn, SqlQuery};

fn url() -> Option<String> {
    std::env::var("RPT_MSSQL_TEST_URL")
        .ok()
        .filter(|u| !u.is_empty())
}

fn column(alias: &str, field: &str, value_type: FieldValueType) -> QueryColumn {
    QueryColumn {
        alias: alias.into(),
        field: field.into(),
        value_type,
        expr: None,
    }
}

#[test]
fn ping_and_version() {
    let Some(url) = url() else { return };
    let mut conn = MssqlConn::open(&url).expect("connect");
    conn.ping().expect("SELECT 1");
    assert!(conn
        .server_version()
        .is_some_and(|v| v.contains("Microsoft SQL Server")));
}

#[test]
fn typed_text_conversions_come_back_re_typed() {
    let Some(url) = url() else { return };
    let mut conn = MssqlConn::open(&url).expect("connect");
    // The conversions the T-SQL dialect emits, applied to literals.
    let query = SqlQuery {
        sql: "SELECT CONVERT(NVARCHAR(64), CAST(661.5 AS real), 3), \
              CONVERT(NVARCHAR(64), CAST(1323.4567 AS money), 2), \
              CONVERT(CHAR(10), CAST('2026-08-31' AS date), 23), \
              CONVERT(VARCHAR(19), CAST('2026-08-31T12:34:56' AS datetime), 120), \
              CASE WHEN CAST(1 AS bit) <> 0 THEN 'true' ELSE 'false' END, \
              CAST(N'Ærlig' AS NVARCHAR(MAX)), \
              CAST(NULL AS NVARCHAR(MAX)), \
              CAST(0x0102 AS varbinary(max))"
            .into(),
        columns: vec![
            column("t", "real", FieldValueType::Number),
            column("t", "money", FieldValueType::Currency),
            column("t", "date", FieldValueType::Date),
            column("t", "stamp", FieldValueType::DateTime),
            column("t", "flag", FieldValueType::Boolean),
            column("t", "text", FieldValueType::String),
            column("t", "nothing", FieldValueType::String),
            column("t", "bytes", FieldValueType::Blob),
        ],
        not_pushed: Vec::new(),
    };
    let source = conn.run(&query).expect("query");
    let rows = source.rows();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.get("t.real"), Some(&Value::Number(661.5)));
    assert_eq!(row.get("t.money"), Some(&Value::Currency(1323.4567)));
    assert_eq!(row.get("t.flag"), Some(&Value::Bool(true)));
    assert_eq!(row.get("t.text"), Some(&Value::Str("Ærlig".into())));
    assert!(
        matches!(row.get("t.date"), Some(Value::Date(_))),
        "{:?}",
        row.get("t.date")
    );
    assert!(matches!(row.get("t.stamp"), Some(Value::DateTime(..))));
}

#[test]
fn a_failing_statement_reports_the_sql() {
    let Some(url) = url() else { return };
    let mut conn = MssqlConn::open(&url).expect("connect");
    let query = SqlQuery {
        sql: "SELECT x FROM [no_such_table_rpt]".into(),
        columns: vec![column("t", "x", FieldValueType::String)],
        not_pushed: Vec::new(),
    };
    let err = conn.run(&query).expect_err("must fail");
    assert!(err.hint().is_some_and(|h| h.contains("no_such_table_rpt")));
}
