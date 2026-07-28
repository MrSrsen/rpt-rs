//! The portable SQL model: column types, tables, cell values, and the emitter.
//!
//! One schema drives both engines. The *only* per-dialect differences are the
//! physical column type of the blob/bool/timestamp/date families and the
//! literal syntax for blobs and booleans; everything else — identifiers,
//! numbers, strings, dates-as-text — is emitted identically.

use crate::calendar::{fmt_date, fmt_timestamp};
use std::fmt::Write as _;

/// Target SQL dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    /// PostgreSQL (perf / oracle path).
    Postgres,
    /// SQLite (zero-process CI / localhost path).
    Sqlite,
}

impl Dialect {
    /// Parse the `--dialect` argument.
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "postgres" => Some(Self::Postgres),
            "sqlite" => Some(Self::Sqlite),
            _ => None,
        }
    }
}

/// A portable column type.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Ty {
    /// 32-bit integer surrogate keys / small counts.
    Int,
    /// 64-bit integer.
    BigInt,
    /// `VARCHAR(n)`.
    Varchar(u32),
    /// `NUMERIC(precision, scale)`.
    Numeric(u8, u8),
    /// Calendar date.
    Date,
    /// Date + time of day.
    Timestamp,
    /// Boolean.
    Bool,
    /// Binary blob.
    Blob,
}

impl Ty {
    /// The physical type keyword for a dialect.
    fn keyword(self, d: Dialect) -> String {
        use Dialect::{Postgres, Sqlite};
        match (self, d) {
            (Ty::Int, _) => "INTEGER".into(),
            (Ty::BigInt, _) => "BIGINT".into(),
            (Ty::Varchar(n), _) => format!("VARCHAR({n})"),
            (Ty::Numeric(p, s), _) => format!("NUMERIC({p},{s})"),
            (Ty::Date, Postgres) => "DATE".into(),
            (Ty::Date, Sqlite) => "TEXT".into(),
            (Ty::Timestamp, Postgres) => "TIMESTAMP".into(),
            (Ty::Timestamp, Sqlite) => "TEXT".into(),
            (Ty::Bool, Postgres) => "BOOLEAN".into(),
            (Ty::Bool, Sqlite) => "INTEGER".into(),
            (Ty::Blob, Postgres) => "BYTEA".into(),
            (Ty::Blob, Sqlite) => "BLOB".into(),
        }
    }
}

/// A column definition.
#[derive(Debug, Clone)]
pub(crate) struct Column {
    pub(crate) name: &'static str,
    pub(crate) ty: Ty,
    pub(crate) nullable: bool,
    /// `Some((table, column))` if this column references another.
    pub(crate) fk: Option<(&'static str, &'static str)>,
}

/// Column builders — concise, so a table spec reads like the DDL.
pub(crate) fn col(name: &'static str, ty: Ty) -> Column {
    Column {
        name,
        ty,
        nullable: false,
        fk: None,
    }
}

/// A nullable column.
pub(crate) fn nul(name: &'static str, ty: Ty) -> Column {
    Column {
        nullable: true,
        ..col(name, ty)
    }
}

/// A non-null foreign-key column.
pub(crate) fn fk(name: &'static str, ref_table: &'static str, ref_col: &'static str) -> Column {
    Column {
        fk: Some((ref_table, ref_col)),
        ..col(name, Ty::Int)
    }
}

/// A nullable foreign-key column.
pub(crate) fn fk_nul(name: &'static str, ref_table: &'static str, ref_col: &'static str) -> Column {
    Column {
        nullable: true,
        ..fk(name, ref_table, ref_col)
    }
}

/// A non-null foreign-key column referencing a `VARCHAR` key (e.g. a currency
/// code). The column type must match the referenced key's type, or PostgreSQL
/// rejects the constraint.
pub(crate) fn fk_text(
    name: &'static str,
    ref_table: &'static str,
    ref_col: &'static str,
    len: u32,
) -> Column {
    Column {
        name,
        ty: Ty::Varchar(len),
        nullable: false,
        fk: Some((ref_table, ref_col)),
    }
}

/// A cell value, dialect-independent apart from rendering.
#[derive(Debug, Clone)]
pub(crate) enum Val {
    /// `NULL`.
    Null,
    /// Integer.
    Int(i64),
    /// Fixed-point decimal `raw / 10^scale`.
    Dec(i64, u8),
    /// String (single quotes escaped on emit).
    Text(String),
    /// Boolean.
    Bool(bool),
    /// Date as a day offset from 1970-01-01.
    Date(i32),
    /// Timestamp as `(day offset, seconds within day)`.
    Ts(i32, u32),
    /// Binary blob.
    Blob(Vec<u8>),
}

impl Val {
    /// Render the value as a SQL literal for the dialect.
    fn render(&self, d: Dialect, out: &mut String) {
        match self {
            Val::Null => out.push_str("NULL"),
            Val::Int(n) => {
                let _ = write!(out, "{n}");
            }
            Val::Dec(raw, scale) => render_decimal(*raw, *scale, out),
            Val::Text(s) => render_text(s, out),
            Val::Bool(b) => match (d, b) {
                (Dialect::Postgres, true) => out.push_str("TRUE"),
                (Dialect::Postgres, false) => out.push_str("FALSE"),
                (Dialect::Sqlite, true) => out.push('1'),
                (Dialect::Sqlite, false) => out.push('0'),
            },
            Val::Date(day) => {
                let _ = write!(out, "'{}'", fmt_date(*day));
            }
            Val::Ts(day, secs) => {
                let _ = write!(out, "'{}'", fmt_timestamp(*day, *secs));
            }
            Val::Blob(bytes) => render_blob(bytes, d, out),
        }
    }
}

fn render_decimal(raw: i64, scale: u8, out: &mut String) {
    if scale == 0 {
        let _ = write!(out, "{raw}");
        return;
    }
    let neg = raw < 0;
    let mag = (raw as i128).unsigned_abs();
    let div = 10u128.pow(u32::from(scale));
    let int = mag / div;
    let frac = mag % div;
    if neg {
        out.push('-');
    }
    let _ = write!(out, "{int}.{frac:0width$}", width = scale as usize);
}

fn render_text(s: &str, out: &mut String) {
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push('\'');
        }
        out.push(ch);
    }
    out.push('\'');
}

fn render_blob(bytes: &[u8], d: Dialect, out: &mut String) {
    match d {
        Dialect::Postgres => {
            out.push_str("'\\x");
            for b in bytes {
                let _ = write!(out, "{b:02x}");
            }
            out.push('\'');
        }
        Dialect::Sqlite => {
            out.push_str("X'");
            for b in bytes {
                let _ = write!(out, "{b:02X}");
            }
            out.push('\'');
        }
    }
}

/// A fully-populated table: schema plus generated rows in emit order.
#[derive(Debug)]
pub(crate) struct Table {
    pub(crate) name: &'static str,
    pub(crate) columns: Vec<Column>,
    /// Extra table-level constraint clauses (e.g. a composite `PRIMARY KEY`).
    pub(crate) primary_key: Option<Vec<&'static str>>,
    pub(crate) rows: Vec<Vec<Val>>,
}

impl Table {
    /// A single-column integer primary key on `columns[0]`.
    pub(crate) fn new(name: &'static str, columns: Vec<Column>) -> Self {
        Self {
            name,
            columns,
            primary_key: None,
            rows: Vec::new(),
        }
    }

    /// Declare a composite primary key (otherwise the first column is the PK).
    pub(crate) fn with_pk(mut self, cols: Vec<&'static str>) -> Self {
        self.primary_key = Some(cols);
        self
    }

    /// Push a row; the column count is checked in debug builds.
    pub(crate) fn push(&mut self, row: Vec<Val>) {
        debug_assert_eq!(
            row.len(),
            self.columns.len(),
            "row width mismatch for table {}",
            self.name
        );
        self.rows.push(row);
    }

    /// Emit the `CREATE TABLE` for a dialect.
    fn write_ddl(&self, d: Dialect, out: &mut String) {
        let _ = writeln!(out, "CREATE TABLE {} (", self.name);
        let single_pk = self.primary_key.is_none();
        let mut lines: Vec<String> = Vec::new();
        for (i, c) in self.columns.iter().enumerate() {
            let mut line = format!("  {} {}", c.name, c.ty.keyword(d));
            if single_pk && i == 0 {
                line.push_str(" PRIMARY KEY");
            } else if !c.nullable {
                line.push_str(" NOT NULL");
            }
            lines.push(line);
        }
        if let Some(pk) = &self.primary_key {
            lines.push(format!("  PRIMARY KEY ({})", pk.join(", ")));
        }
        for c in &self.columns {
            if let Some((t, rc)) = c.fk {
                lines.push(format!(
                    "  FOREIGN KEY ({}) REFERENCES {} ({})",
                    c.name, t, rc
                ));
            }
        }
        out.push_str(&lines.join(",\n"));
        out.push_str("\n);\n");
    }

    /// Emit the batched multi-row `INSERT`s for a dialect.
    fn write_inserts(&self, d: Dialect, out: &mut String) {
        if self.rows.is_empty() {
            return;
        }
        let cols = self
            .columns
            .iter()
            .map(|c| c.name)
            .collect::<Vec<_>>()
            .join(", ");
        const BATCH: usize = 500;
        for batch in self.rows.chunks(BATCH) {
            let _ = writeln!(out, "INSERT INTO {} ({cols}) VALUES", self.name);
            for (i, row) in batch.iter().enumerate() {
                out.push(' ');
                out.push('(');
                for (j, v) in row.iter().enumerate() {
                    if j > 0 {
                        out.push_str(", ");
                    }
                    v.render(d, out);
                }
                out.push(')');
                out.push_str(if i + 1 == batch.len() { ";\n" } else { ",\n" });
            }
        }
    }
}

/// Emit the whole schema DDL (no rows) for a dialect.
pub(crate) fn emit_schema(tables: &[Table], d: Dialect, out: &mut String) {
    out.push_str("-- Meridian Global Logistics — synthetic corpus schema.\n");
    out.push_str("-- Generated by `meridian-seed`; do not edit by hand.\n\n");
    for t in tables.iter().rev() {
        let _ = writeln!(out, "DROP TABLE IF EXISTS {};", t.name);
    }
    out.push('\n');
    for t in tables {
        t.write_ddl(d, out);
        out.push('\n');
    }
}

/// Emit the whole seed: DDL followed by FK-safe `INSERT`s.
pub(crate) fn emit_seed(tables: &[Table], d: Dialect, out: &mut String) {
    emit_schema(tables, d, out);
    out.push_str("-- Data ---------------------------------------------------------------\n\n");
    for t in tables {
        t.write_inserts(d, out);
        if !t.rows.is_empty() {
            out.push('\n');
        }
    }
}
