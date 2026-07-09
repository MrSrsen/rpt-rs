//! # rpt-query — SQL generation for the live-DB path
//!
//! The native engine builds a join graph from a report's decoded link metadata and emits joined
//! SQL to the query engine — a multi-table report needs every
//! referenced table joined into one query, not just the primary table. This crate is the
//! SQL-generation layer that produces that joined SQL: given a report's [`Database`] (tables +
//! links) it produces a single `SELECT … FROM … JOIN …` and the ordered [`QueryColumn`] list a
//! fetcher uses to key and re-type each returned column. It is pure (no I/O) and WASM-safe — the
//! structured SQL is rendered by [`sea_query`] (core only, no driver/binder features) so quoting,
//! text-casts, and joins are dialect-correct; the DB driver (native-only) consumes the string it
//! returns.
//!
//! Dialects: Postgres / SQLite / MySQL, via sea-query's per-dialect builders. Author-written
//! command tables (Crystal SQL Commands) are passed through **verbatim** and cannot be expressed as
//! a structured sea-query source, so a report containing one is rendered by the hand-rolled string
//! path instead; fully-structured reports use sea-query.
//!
//! Scope: `INNER`/`LEFT`/`RIGHT` joins over the link graph, multi-field link conditions, self-joins
//! (each table is aliased), command tables (`FROM (<sql>) AS alias`), and — via
//! [`build_query_with_selection`] — pushing the translatable subset of the record-selection formula
//! into `WHERE`. Non-equijoin link operators (`>`/`<`/`<>`) render as inner joins with
//! that operator. Full outer joins are not a decoded variant.

mod selection;
mod used_fields;

pub use selection::{push_down_selection, push_down_selection_with_params, PushDown};
pub use used_fields::{prune_database, used_database_fields};

/// Re-exported so callers can name the parameter-value type used by [`build_query_full`] /
/// [`push_down_selection_with_params`] without depending on `crystal-formula` directly.
pub use crystal_formula::eval::Value;

use rpt_model::{Database, FieldValueType, Report, Table, TableJoinType};
use sea_query::{
    Alias, Asterisk, BinOper, Condition, Expr, JoinType, MysqlQueryBuilder, PostgresQueryBuilder,
    Query, SimpleExpr, SqliteQueryBuilder,
};
use std::collections::HashSet;

/// One column of the generated query: how to key it back onto a [`Row`](rpt_data) (`alias.field`,
/// the name formulas use) and the report's declared type for re-typing the text-cast cell.
///
/// Most columns are a plain table field (`alias` + `field`). A **SQL Expression field**
/// (`ISQLExpressionField`) is instead a raw SQL fragment the server computes: it sets
/// `expr` to that fragment, leaves `alias` empty, and is keyed by its bare `field` name — the name a
/// `{%name}` reference resolves against (the render side looks it up via `row.get(name)`).
#[derive(Debug, Clone, PartialEq)]
pub struct QueryColumn {
    /// The table alias the column is selected from (empty for an expression column).
    pub alias: String,
    /// The field name, i.e. the last segment of the `{alias}.{field}` reference formulas use.
    pub field: String,
    /// The report's declared value type, used to re-type the text-cast cell fetched from the DB.
    pub value_type: FieldValueType,
    /// A raw SQL expression (a SQL Expression field's `text`) selected as `(<expr>) AS "<field>"`
    /// instead of a table column. `None` for a plain table field.
    pub expr: Option<String>,
}

impl QueryColumn {
    /// The key a [`Row`](rpt_data) is indexed by (how formulas reference the field). A plain field is
    /// `alias.field`; an expression column (empty `alias`) is keyed by its bare `field` name, which is
    /// how a `{%name}` SQL-expression reference resolves.
    pub fn key(&self) -> String {
        if self.alias.is_empty() {
            self.field.clone()
        } else {
            format!("{}.{}", self.alias, self.field)
        }
    }
}

/// The pipeline column projection for a query column: keyed by [`QueryColumn::key`] (how formulas
/// reference the value) and carrying the declared value type used to re-type the fetched text cell.
/// Defining it here means every DB backend keys and types its rows the same way.
impl From<&QueryColumn> for rpt_data::Column {
    fn from(c: &QueryColumn) -> Self {
        rpt_data::Column {
            name: c.key(),
            value_type: c.value_type,
        }
    }
}

/// A generated query: the SQL text and the ordered columns it selects (position `i` in the result
/// is `columns[i]`).
#[derive(Debug, Clone, PartialEq)]
pub struct SqlQuery {
    /// The generated `SELECT` statement, ready to execute against the target dialect.
    pub sql: String,
    /// The selected columns in result order (result position `i` is `columns[i]`).
    pub columns: Vec<QueryColumn>,
}

impl SqlQuery {
    /// The pipeline column projection: one [`rpt_data::Column`] per selected column, in result order.
    /// A DB backend builds its [`Row`](rpt_data::Row)s against this (via
    /// [`rows_from_cells`](rpt_data::rows_from_cells)) so the column keying and declared types live in
    /// one place rather than being re-derived per driver.
    pub fn result_columns(&self) -> Vec<rpt_data::Column> {
        self.columns.iter().map(rpt_data::Column::from).collect()
    }
}

/// The SQL dialect a generated query targets. The cast-to-text syntax differs across the backends
/// (Postgres `x::text`, SQLite `CAST(x AS TEXT)`, MySQL `CAST(x AS CHAR)` — MySQL has no `AS TEXT`),
/// as does identifier quoting (`"…"` vs MySQL's `` `…` ``). Structured queries are rendered by
/// sea-query's per-dialect builder so all of this is dialect-correct by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dialect {
    /// PostgreSQL (`x::text` casts, `"…"` quoting). The default dialect.
    #[default]
    Postgres,
    /// SQLite (`CAST(x AS TEXT)` casts, `"…"` quoting).
    Sqlite,
    /// MySQL/MariaDB (`CAST(x AS CHAR)` casts, `` `…` `` quoting).
    Mysql,
}

impl Dialect {
    /// Cast a (raw, already-quoted) column reference to text so every value is fetched uniformly as a
    /// string and re-typed against the report's declared field type by the executor. Used only by the
    /// hand-rolled command-table path; the structured (sea-query) path builds the cast via [`cast_expr`].
    fn cast_text(self, expr: &str) -> String {
        match self {
            Dialect::Postgres => format!("{expr}::text"),
            Dialect::Sqlite => format!("CAST({expr} AS TEXT)"),
            Dialect::Mysql => format!("CAST({expr} AS CHAR)"),
        }
    }
}

/// The dialect-correct "cast this column to text" sea-query expression for the structured path.
/// Postgres keeps the `x::text` operator form (byte-identical to the legacy builder and to the live
/// `rpt-db-postgres` path); SQLite/MySQL use `CAST(x AS TEXT|CHAR)`. sea-query renders the qualified
/// `"alias"."field"` reference (quoted per dialect) via the `$1` placeholder / column ref.
// The per-dialect cast forms here, in `cast_raw_expr`, and in `Dialect::cast_text` duplicate the
// same per-dialect logic; consolidating them into one dialect-cast abstraction is out of scope for
// the join-mapping dedup this function is part of.
fn cast_expr(dialect: Dialect, alias: &str, field: &str) -> SimpleExpr {
    let colref = Expr::col((Alias::new(alias), Alias::new(field)));
    match dialect {
        Dialect::Postgres => Expr::cust_with_expr("$1::text", colref),
        Dialect::Sqlite => colref.cast_as(Alias::new("TEXT")),
        Dialect::Mysql => colref.cast_as(Alias::new("CHAR")),
    }
}

/// Cast a raw SQL Expression field fragment to text (structured path). The author's SQL is emitted
/// verbatim (wrapped in parens, like a command table's SQL) and cast the same way as a column
/// reference — so a SQL Expression's value arrives as a uniformly-text cell to re-type.
fn cast_raw_expr(dialect: Dialect, raw: &str) -> SimpleExpr {
    let inner = Expr::cust(format!("({raw})"));
    match dialect {
        Dialect::Postgres => Expr::cust_with_expr("$1::text", inner),
        Dialect::Sqlite => inner.cast_as(Alias::new("TEXT")),
        Dialect::Mysql => inner.cast_as(Alias::new("CHAR")),
    }
}

/// Build the joined `SELECT` for a report's whole table graph. `None` when the database has no
/// tables. Equivalent to [`build_query_with_selection`] with no selection formula.
pub fn build_query(database: &Database) -> Option<SqlQuery> {
    build_query_with_selection(database, None)
}

/// Like [`build_query`] but for an explicit SQL [`Dialect`] (no selection push-down).
pub fn build_query_in(database: &Database, dialect: Dialect) -> Option<SqlQuery> {
    build_query_with_selection_in(database, None, dialect)
}

/// Like [`build_query`] but also pushes the translatable part of `selection` (a Crystal
/// record-selection formula) into a `WHERE` clause. The untranslatable remainder is
/// still applied per-row by the pipeline, so the result set is identical — just fewer rows fetched.
pub fn build_query_with_selection(
    database: &Database,
    selection: Option<&str>,
) -> Option<SqlQuery> {
    build_query_with_selection_in(database, selection, Dialect::Postgres)
}

/// [`build_query_with_selection`] targeting an explicit SQL [`Dialect`]. The `WHERE` push-down is
/// only emitted for the Postgres dialect (its predicate SQL is Postgres-flavoured); other dialects
/// fetch the full table and rely on the pipeline applying the selection formula per row.
pub fn build_query_with_selection_in(
    database: &Database,
    selection: Option<&str>,
    dialect: Dialect,
) -> Option<SqlQuery> {
    build_query_full(database, &[], selection, &[], dialect)
}

/// The full builder: joins + optional SQL Expression fields + optional selection
/// push-down bound with parameter current-values. All the narrower public
/// builders delegate here.
///
/// - `sql_exprs` — `(field_name, sql_text)` pairs from `IDataDefinition.SQLExpressionFields`; each is
///   appended to the `SELECT` as `(<sql_text>) AS "<field_name>"` (cast to text) and keyed by its
///   bare name so `{%name}` resolves against it.
/// - `params` — parameter current-values (`{?Name}` → literal); only consulted by the Postgres
///   `WHERE` push-down, so a selection formula comparing a field to a parameter binds the literal.
pub fn build_query_full(
    database: &Database,
    sql_exprs: &[(String, String)],
    selection: Option<&str>,
    params: &[(String, Value)],
    dialect: Dialect,
) -> Option<SqlQuery> {
    if database.tables.is_empty() {
        return None;
    }
    let order = join_order(database);

    // Columns in placement order: every placed table's fields, keyed by its alias.
    let mut columns = Vec::new();
    for ti in &order.placed {
        let t = &database.tables[*ti];
        for f in &t.data_fields {
            columns.push(QueryColumn {
                alias: t.alias.clone(),
                field: f.name.clone(),
                value_type: f.value_type,
                expr: None,
            });
        }
    }

    // SQL Expression fields (server-computed raw SQL), appended after the table columns and keyed by
    // their bare name (empty alias) so a `{%name}` reference resolves to `row.get("name")`.
    for (name, text) in sql_exprs {
        columns.push(QueryColumn {
            alias: String::new(),
            field: name.clone(),
            value_type: FieldValueType::String,
            expr: Some(text.clone()),
        });
    }

    // The push-down predicate SQL is Postgres-flavoured; other dialects fetch the full table and let
    // the pipeline apply the selection formula per row (identical result set, just more rows fetched).
    let where_sql: Option<String> = if dialect == Dialect::Postgres {
        selection.and_then(|formula| {
            push_down_selection_with_params(formula, &columns, params).where_sql
        })
    } else {
        None
    };

    // Dual-path rendering. A report table's `command_text` is the report author's raw
    // SQL command; it is passed to the database verbatim as `(<command_text>) AS "alias"` and must
    // never be translated or restructured. sea-query has no raw-SQL `TableRef`, so any report that
    // contains a command table falls back to the hand-rolled string builder (which already emits the
    // command SQL verbatim + does the joins). Fully-structured reports are built with sea-query so
    // quoting / casts / joins are dialect-correct by construction.
    let has_command = order
        .placed
        .iter()
        .any(|ti| is_command_table(&database.tables[*ti]));

    let sql = if has_command {
        build_sql_raw(database, &order, &columns, dialect, where_sql.as_deref())
    } else {
        build_sql_seaquery(database, &order, &columns, dialect, where_sql.as_deref())
    };

    Some(SqlQuery { sql, columns })
}

/// The report-aware builder: like [`build_query_full`] but first prunes the database to the tables
/// and columns the report actually references ([`used_database_fields`] + [`prune_database`]), so the
/// generated `SELECT` projects only used columns and omits declared-but-unused tables. This is what
/// the native engine does; without it, a report's stray unused tables are cross-joined into a
/// cartesian. A report that references no attributable field falls back to the full database.
///
/// `sql_exprs` / `selection` / `params` behave exactly as in [`build_query_full`] — SQL Expression
/// fields are always projected, and the selection push-down (Postgres) sees the pruned columns.
pub fn build_query_for_report(
    report: &Report,
    sql_exprs: &[(String, String)],
    selection: Option<&str>,
    params: &[(String, Value)],
    dialect: Dialect,
) -> Option<SqlQuery> {
    let used = used_database_fields(report);
    let pruned = prune_database(&report.database, &used);
    build_query_full(&pruned, sql_exprs, selection, params, dialect)
}

/// Whether a table is a Crystal SQL Command (author-written raw SQL), which is passed through
/// verbatim and therefore forces the hand-rolled (non-sea-query) build path.
fn is_command_table(table: &Table) -> bool {
    matches!(&table.command_text, Some(cmd) if !cmd.trim().is_empty())
}

/// Build the `SELECT` for a fully-structured report (no command tables) with sea-query, so quoting,
/// text-casts, and joins are rendered correctly for `dialect`. Rendered with `to_string` (not
/// `build`) so no values are parameterized — the returned SQL is self-contained/executable (all
/// values are inlined via the text casts and the raw `WHERE`).
fn build_sql_seaquery(
    database: &Database,
    order: &JoinOrder,
    columns: &[QueryColumn],
    dialect: Dialect,
    where_sql: Option<&str>,
) -> String {
    let tables = &database.tables;
    let mut q = Query::select();

    if columns.is_empty() {
        q.column(Asterisk);
    } else {
        for c in columns {
            match &c.expr {
                // A SQL Expression field: the raw fragment cast to text, aliased by its field name.
                Some(e) => q.expr_as(cast_raw_expr(dialect, e), Alias::new(&c.field)),
                None => q.expr(cast_expr(dialect, &c.alias, &c.field)),
            };
        }
    }

    let base = &tables[order.placed[0]];
    q.from_as(Alias::new(&base.name), Alias::new(&base.alias));

    for step in &order.steps {
        let t = &tables[step.table];
        match step.link {
            Some(li) => {
                let link = &database.links[li];
                let (kind, cond) = join_spec(link, step.reversed);
                q.join_as(kind, Alias::new(&t.name), Alias::new(&t.alias), cond);
            }
            // Linkless table: a cartesian product. `JOIN … ON TRUE` is exactly a CROSS JOIN.
            None => {
                q.join_as(
                    JoinType::Join,
                    Alias::new(&t.name),
                    Alias::new(&t.alias),
                    Expr::cust("TRUE"),
                );
            }
        }
    }

    if let Some(w) = where_sql {
        q.cond_where(Expr::cust(w));
    }

    match dialect {
        Dialect::Postgres => q.to_string(PostgresQueryBuilder),
        Dialect::Sqlite => q.to_string(SqliteQueryBuilder),
        Dialect::Mysql => q.to_string(MysqlQueryBuilder),
    }
}

/// Neutral join kind — the LEFT/RIGHT/INNER choice, decoupled from any SQL representation. Both the
/// structured (sea-query) and hand-rolled string paths derive their own keyword/type from this one
/// resolution ([`resolve_join_kind`]) so the two can't drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JoinKind {
    Inner,
    Left,
    Right,
}

/// Neutral link comparison operator for the ON condition, shared by both render paths (mapped from
/// [`resolve_link_op`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkOp {
    Eq,
    Gt,
    Lt,
    Ne,
}

/// Resolve a link's stored join type + placement reversal to the neutral [`JoinKind`]. `reversed`
/// (the link's *source* is the newly-reached table) flips `LEFT`↔`RIGHT` so the intended table stays
/// the preserved side of an outer join. Inner and non-outer variants are `Inner`.
fn resolve_join_kind(join_type: &TableJoinType, reversed: bool) -> JoinKind {
    match (join_type, reversed) {
        (TableJoinType::LeftOuter, false) | (TableJoinType::RightOuter, true) => JoinKind::Left,
        (TableJoinType::RightOuter, false) | (TableJoinType::LeftOuter, true) => JoinKind::Right,
        _ => JoinKind::Inner,
    }
}

/// Map a link's stored join type to the neutral comparison [`LinkOp`] for its ON condition. Only the
/// inequality variants carry a distinct operator; everything else is an equijoin (`=`).
fn resolve_link_op(join_type: &TableJoinType) -> LinkOp {
    match join_type {
        TableJoinType::GreaterThan => LinkOp::Gt,
        TableJoinType::LessThan => LinkOp::Lt,
        TableJoinType::NotEqual => LinkOp::Ne,
        _ => LinkOp::Eq,
    }
}

/// The sea-query `JoinType` + ON `Condition` for `link`. Mirrors [`join_clause`]: both derive the
/// LEFT/RIGHT/INNER choice and the comparison operator from the shared [`resolve_join_kind`] /
/// [`resolve_link_op`]. No field pairs → `ON TRUE` (an empty `Condition::all()`).
fn join_spec(link: &rpt_model::TableLink, reversed: bool) -> (JoinType, Condition) {
    let kind = match resolve_join_kind(&link.join_type, reversed) {
        JoinKind::Inner => JoinType::Join,
        JoinKind::Left => JoinType::LeftJoin,
        JoinKind::Right => JoinType::RightJoin,
    };
    let op = match resolve_link_op(&link.join_type) {
        LinkOp::Eq => BinOper::Equal,
        LinkOp::Gt => BinOper::GreaterThan,
        LinkOp::Lt => BinOper::SmallerThan,
        LinkOp::Ne => BinOper::NotEqual,
    };
    let mut cond = Condition::all();
    for (sf, tf) in link.source_fields.iter().zip(link.target_fields.iter()) {
        let left = Expr::col((Alias::new(&link.source_table_alias), Alias::new(sf)));
        let right = Expr::col((Alias::new(&link.target_table_alias), Alias::new(tf)));
        cond = cond.add(left.binary(op, right));
    }
    (kind, cond)
}

/// Build the `SELECT` for a report that contains at least one command table, by hand (sea-query
/// cannot wrap the author's raw SQL as a structured source). The command SQL is emitted verbatim as
/// `(<command_text>) AS "alias"` by [`from_source`]; the joins/casts use ANSI double-quote quoting.
fn build_sql_raw(
    database: &Database,
    order: &JoinOrder,
    columns: &[QueryColumn],
    dialect: Dialect,
    where_sql: Option<&str>,
) -> String {
    let tables = &database.tables;
    let select = if columns.is_empty() {
        "*".to_string()
    } else {
        columns
            .iter()
            .map(|c| match &c.expr {
                // A SQL Expression field: the raw fragment (verbatim, in parens) cast to text and
                // aliased by its field name.
                Some(e) => format!(
                    "{} AS {}",
                    dialect.cast_text(&format!("({e})")),
                    quote_ident(&c.field)
                ),
                None => dialect.cast_text(&qualify(&c.alias, &c.field)),
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    let mut sql = format!(
        "SELECT {select} FROM {}",
        from_source(&tables[order.placed[0]])
    );
    for step in &order.steps {
        sql.push(' ');
        match step.link {
            Some(li) => sql.push_str(&join_clause(
                &tables[step.table],
                &database.links[li],
                step.reversed,
            )),
            None => sql.push_str(&format!("CROSS JOIN {}", from_source(&tables[step.table]))),
        }
    }

    if let Some(w) = where_sql {
        sql.push_str(" WHERE ");
        sql.push_str(w);
    }
    sql
}

/// The tables placed (in join order) plus the ordered steps connecting them.
struct JoinOrder {
    /// Indices into `database.tables`, primary first, then each table as it is joined in.
    placed: Vec<usize>,
    /// One step per non-primary placed table (aligned with `placed[1..]`).
    steps: Vec<JoinStep>,
}

/// How one non-primary table is attached to the already-placed set.
struct JoinStep {
    /// Index into `database.tables` of the newly-attached table.
    table: usize,
    /// Index into `database.links` of the connecting link, or `None` for a linkless `CROSS JOIN`.
    link: Option<usize>,
    /// The link's *source* is the new table (reached from the target side) — flips `LEFT`↔`RIGHT`.
    reversed: bool,
}

/// Order the tables by walking the link graph out from the primary (`tables[0]`), emitting a join
/// step for each newly-reached table. Tables not reachable through any link are appended as a
/// `CROSS JOIN` (a cartesian product — a linkless report, rare; the pipeline still filters).
fn join_order(database: &Database) -> JoinOrder {
    let tables = &database.tables;
    let index_of = |alias: &str| tables.iter().position(|t| t.alias == alias);

    let mut placed: Vec<usize> = vec![0];
    let mut placed_aliases: HashSet<String> = HashSet::new();
    placed_aliases.insert(tables[0].alias.clone());
    let mut steps: Vec<JoinStep> = Vec::new();

    // The link graph may have several connected components (plus fully isolated tables). Process one
    // component at a time: grow the current frontier through its links, then — if tables remain —
    // bridge to the next unplaced table with a CROSS JOIN and grow *its* component. Every table
    // linked to an already-placed one is attached via that link, regardless of which component the
    // primary (tables[0]) fell in; only genuine cross-component bridges become a cartesian.
    loop {
        // Repeatedly attach a link with exactly one endpoint already placed, until a pass adds none.
        loop {
            let mut progressed = false;
            for (li, link) in database.links.iter().enumerate() {
                let src_placed = placed_aliases.contains(&link.source_table_alias);
                let dst_placed = placed_aliases.contains(&link.target_table_alias);
                let (new_alias, reversed) = match (src_placed, dst_placed) {
                    (true, false) => (&link.target_table_alias, false),
                    (false, true) => (&link.source_table_alias, true),
                    _ => continue, // both placed (redundant link) or neither (not yet reachable)
                };
                let Some(ni) = index_of(new_alias) else {
                    continue;
                };
                steps.push(JoinStep {
                    table: ni,
                    link: Some(li),
                    reversed,
                });
                placed.push(ni);
                placed_aliases.insert(new_alias.clone());
                progressed = true;
            }
            if !progressed {
                break;
            }
        }

        // Bridge to the next unplaced table (a separate component or an isolated table) with a
        // CROSS JOIN, then loop to grow that component through its own links.
        match tables
            .iter()
            .position(|t| !placed_aliases.contains(&t.alias))
        {
            Some(i) => {
                steps.push(JoinStep {
                    table: i,
                    link: None,
                    reversed: false,
                });
                placed.push(i);
                placed_aliases.insert(tables[i].alias.clone());
            }
            None => break,
        }
    }

    JoinOrder { placed, steps }
}

/// The `[LEFT|RIGHT] JOIN <source> ON …` clause attaching `new_table` to already-placed tables via
/// `link`. `reversed` = the link's *source* is the new table (we reached it from the target side),
/// which flips `LEFT`↔`RIGHT` so the intended table stays the preserved side of an outer join. Shares
/// [`resolve_join_kind`] / [`resolve_link_op`] with the structured [`join_spec`] path.
fn join_clause(new_table: &Table, link: &rpt_model::TableLink, reversed: bool) -> String {
    let kind = match resolve_join_kind(&link.join_type, reversed) {
        JoinKind::Inner => "JOIN",
        JoinKind::Left => "LEFT JOIN",
        JoinKind::Right => "RIGHT JOIN",
    };
    let op = match resolve_link_op(&link.join_type) {
        LinkOp::Eq => "=",
        LinkOp::Gt => ">",
        LinkOp::Lt => "<",
        LinkOp::Ne => "<>",
    };
    // The ON condition references each side by its fixed alias.field, independent of placement order.
    let conds: Vec<String> = link
        .source_fields
        .iter()
        .zip(link.target_fields.iter())
        .map(|(sf, tf)| {
            format!(
                "{} {op} {}",
                qualify(&link.source_table_alias, sf),
                qualify(&link.target_table_alias, tf)
            )
        })
        .collect();
    let on = if conds.is_empty() {
        // A link with no field pairs can't constrain — emit a TRUE so the SQL stays valid.
        "TRUE".to_string()
    } else {
        conds.join(" AND ")
    };
    format!("{kind} {} ON {on}", from_source(new_table))
}

/// The `FROM`/`JOIN` source for a table: `"name" AS "alias"`, or `(<command sql>) AS "alias"` for a
/// command table.
fn from_source(table: &Table) -> String {
    match &table.command_text {
        Some(cmd) if !cmd.trim().is_empty() => format!("({cmd}) AS {}", quote_ident(&table.alias)),
        _ => format!(
            "{} AS {}",
            quote_ident(&table.name),
            quote_ident(&table.alias)
        ),
    }
}

/// `"alias"."field"` — a column reference with both identifiers quoted.
fn qualify(alias: &str, field: &str) -> String {
    format!("{}.{}", quote_ident(alias), quote_ident(field))
}

/// Double-quote a SQL identifier, escaping embedded quotes (Postgres-safe).
pub(crate) fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_model::{Database, DbFieldDef, Table, TableLink};

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

    fn link(
        join_type: TableJoinType,
        src: &str,
        tgt: &str,
        src_fields: &[&str],
        tgt_fields: &[&str],
    ) -> TableLink {
        TableLink {
            join_type,
            source_table_alias: src.into(),
            target_table_alias: tgt.into(),
            source_fields: src_fields.iter().map(|s| s.to_string()).collect(),
            target_fields: tgt_fields.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn single_table_select() {
        let db = Database {
            tables: vec![table(
                "countries",
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
            r#"SELECT "countries"."id"::text, "countries"."name"::text FROM "countries" AS "countries""#
        );
        assert_eq!(q.columns.len(), 2);
        assert_eq!(q.columns[0].key(), "countries.id");
    }

    #[test]
    fn unlinked_primary_still_joins_the_linked_component() {
        // The primary (tables[0]) is an isolated table in its own component; the linked pair
        // fact→dim lives in a *separate* component. The isolated table must not suppress the
        // pair's equijoin — regression for the cartesian-explosion bug (join_order walked out
        // from tables[0] only, so a linked core reachable from no primary got all CROSS JOINs).
        let db = Database {
            tables: vec![
                table("iso", &[("code", FieldValueType::String)]), // isolated, sorts first
                table(
                    "fact",
                    &[
                        ("id", FieldValueType::Int32s),
                        ("dim_id", FieldValueType::Int32s),
                    ],
                ),
                table("dim", &[("id", FieldValueType::Int32s)]),
            ],
            links: vec![link(
                TableJoinType::Equal,
                "fact",
                "dim",
                &["dim_id"],
                &["id"],
            )],
        };
        let q = build_query(&db).unwrap();
        // The fact→dim link is applied (an equijoin, not ON TRUE)…
        assert!(
            q.sql
                .contains(r#"JOIN "dim" AS "dim" ON "fact"."dim_id" = "dim"."id""#),
            "linked component must equijoin; sql was: {}",
            q.sql
        );
        // …and exactly one CROSS JOIN bridges the isolated table to the linked component.
        assert_eq!(
            q.sql.matches("ON TRUE").count(),
            1,
            "only the cross-component bridge is a cartesian; sql was: {}",
            q.sql
        );
    }

    #[test]
    fn inner_join_two_tables() {
        let db = Database {
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
            links: vec![link(
                TableJoinType::Equal,
                "orders",
                "customers",
                &["cust"],
                &["id"],
            )],
        };
        let q = build_query(&db).unwrap();
        assert!(
            q.sql.contains(r#"FROM "orders" AS "orders" JOIN "customers" AS "customers" ON "orders"."cust" = "customers"."id""#),
            "sql was: {}",
            q.sql
        );
        // All four columns fetched, orders first then customers.
        assert_eq!(
            q.columns.iter().map(|c| c.key()).collect::<Vec<_>>(),
            vec!["orders.id", "orders.cust", "customers.id", "customers.name"]
        );
    }

    #[test]
    fn left_outer_join_keyword() {
        let db = Database {
            tables: vec![
                table("a", &[("k", FieldValueType::Int32s)]),
                table("b", &[("k", FieldValueType::Int32s)]),
            ],
            links: vec![link(TableJoinType::LeftOuter, "a", "b", &["k"], &["k"])],
        };
        let q = build_query(&db).unwrap();
        assert!(
            q.sql
                .contains(r#"LEFT JOIN "b" AS "b" ON "a"."k" = "b"."k""#),
            "{}",
            q.sql
        );
    }

    #[test]
    fn reversed_left_outer_becomes_right() {
        // Link source = b (unplaced), target = a (the primary, placed first). Reaching b from a
        // reverses the link, so a LeftOuter (preserve b) must render as RIGHT JOIN.
        let db = Database {
            tables: vec![
                table("a", &[("k", FieldValueType::Int32s)]),
                table("b", &[("k", FieldValueType::Int32s)]),
            ],
            links: vec![link(TableJoinType::LeftOuter, "b", "a", &["k"], &["k"])],
        };
        let q = build_query(&db).unwrap();
        assert!(
            q.sql
                .contains(r#"RIGHT JOIN "b" AS "b" ON "b"."k" = "a"."k""#),
            "{}",
            q.sql
        );
    }

    #[test]
    fn multi_field_link_conditions() {
        let db = Database {
            tables: vec![
                table(
                    "a",
                    &[("x", FieldValueType::Int32s), ("y", FieldValueType::Int32s)],
                ),
                table(
                    "b",
                    &[("x", FieldValueType::Int32s), ("y", FieldValueType::Int32s)],
                ),
            ],
            links: vec![link(
                TableJoinType::Equal,
                "a",
                "b",
                &["x", "y"],
                &["x", "y"],
            )],
        };
        let q = build_query(&db).unwrap();
        assert!(
            q.sql
                .contains(r#"ON "a"."x" = "b"."x" AND "a"."y" = "b"."y""#),
            "{}",
            q.sql
        );
    }

    #[test]
    fn no_tables_is_none() {
        assert!(build_query(&Database::default()).is_none());
    }

    #[test]
    fn command_table_from_subquery() {
        let mut db = Database::default();
        let mut t = table("cmd", &[("v", FieldValueType::Int32s)]);
        t.command_text = Some("SELECT 1 AS v".into());
        db.tables = vec![t];
        let q = build_query(&db).unwrap();
        assert!(
            q.sql.contains(r#"FROM (SELECT 1 AS v) AS "cmd""#),
            "{}",
            q.sql
        );
    }

    // --- Per-dialect snapshots (sea-query structured path) -------------------------------------

    fn one_table_db() -> Database {
        Database {
            tables: vec![table(
                "countries",
                &[
                    ("id", FieldValueType::Int32s),
                    ("name", FieldValueType::String),
                ],
            )],
            ..Default::default()
        }
    }

    #[test]
    fn postgres_snapshot_uses_double_colon_cast() {
        let q = build_query_in(&one_table_db(), Dialect::Postgres).unwrap();
        assert_eq!(
            q.sql,
            r#"SELECT "countries"."id"::text, "countries"."name"::text FROM "countries" AS "countries""#
        );
    }

    #[test]
    fn sqlite_snapshot_uses_cast_as_text() {
        let q = build_query_in(&one_table_db(), Dialect::Sqlite).unwrap();
        assert_eq!(
            q.sql,
            r#"SELECT CAST("countries"."id" AS TEXT), CAST("countries"."name" AS TEXT) FROM "countries" AS "countries""#
        );
    }

    #[test]
    fn mysql_snapshot_uses_backticks_and_cast_as_char() {
        let q = build_query_in(&one_table_db(), Dialect::Mysql).unwrap();
        assert_eq!(
            q.sql,
            "SELECT CAST(`countries`.`id` AS CHAR), CAST(`countries`.`name` AS CHAR) FROM `countries` AS `countries`"
        );
    }

    #[test]
    fn sqlite_join_snapshot() {
        let db = Database {
            tables: vec![
                table("orders", &[("cust", FieldValueType::Int32s)]),
                table("customers", &[("id", FieldValueType::Int32s)]),
            ],
            links: vec![link(
                TableJoinType::Equal,
                "orders",
                "customers",
                &["cust"],
                &["id"],
            )],
        };
        let q = build_query_in(&db, Dialect::Sqlite).unwrap();
        assert_eq!(
            q.sql,
            r#"SELECT CAST("orders"."cust" AS TEXT), CAST("customers"."id" AS TEXT) FROM "orders" AS "orders" JOIN "customers" AS "customers" ON "orders"."cust" = "customers"."id""#
        );
    }

    #[test]
    fn sql_expression_field_emitted_and_keyed_by_name() {
        // A SQL Expression field is appended to the SELECT as `(<expr>) AS "name"` (cast to text) and
        // keyed by its bare name so a `{%name}` reference resolves via `row.get("name")`.
        let db = one_table_db();
        let exprs = vec![("TaxRate".to_string(), "unit_price * 0.2".to_string())];
        // Postgres (structured sea-query path).
        let q = build_query_full(&db, &exprs, None, &[], Dialect::Postgres).unwrap();
        assert!(
            q.sql.contains(r#"(unit_price * 0.2)::text AS "TaxRate""#),
            "{}",
            q.sql
        );
        let last = q.columns.last().unwrap();
        assert_eq!(last.expr.as_deref(), Some("unit_price * 0.2"));
        assert_eq!(last.alias, "");
        // Keyed by the bare name (no leading dot), so `row.get("TaxRate")` finds it.
        assert_eq!(last.key(), "TaxRate");

        // SQLite (hand-rolled raw path via a command table forcing it) still emits the aliased expr.
        let mut db2 = one_table_db();
        db2.tables[0].command_text = Some("SELECT 1 AS id, 'x' AS name".into());
        let q2 = build_query_full(&db2, &exprs, None, &[], Dialect::Sqlite).unwrap();
        assert!(
            q2.sql
                .contains(r#"CAST((unit_price * 0.2) AS TEXT) AS "TaxRate""#),
            "{}",
            q2.sql
        );
    }

    #[test]
    fn command_report_uses_raw_path_in_every_dialect() {
        // A command table forces the hand-rolled path in all dialects; the author SQL is verbatim.
        let mut db = Database::default();
        let mut t = table("cmd", &[("v", FieldValueType::Int32s)]);
        t.command_text = Some("SELECT foo FROM bar".into());
        db.tables = vec![t];
        for d in [Dialect::Postgres, Dialect::Sqlite, Dialect::Mysql] {
            let q = build_query_in(&db, d).unwrap();
            assert!(
                q.sql.contains(r#"FROM (SELECT foo FROM bar) AS "cmd""#),
                "{d:?}: {}",
                q.sql
            );
        }
    }

    #[test]
    fn join_kind_resolution_flips_with_reversal() {
        use JoinKind::*;
        // Non-reversed: the stored side is preserved as-is.
        assert_eq!(resolve_join_kind(&TableJoinType::LeftOuter, false), Left);
        assert_eq!(resolve_join_kind(&TableJoinType::RightOuter, false), Right);
        assert_eq!(resolve_join_kind(&TableJoinType::Equal, false), Inner);
        // Reversed (reached from the target side): LEFT↔RIGHT swap; inner is unaffected.
        assert_eq!(resolve_join_kind(&TableJoinType::LeftOuter, true), Right);
        assert_eq!(resolve_join_kind(&TableJoinType::RightOuter, true), Left);
        assert_eq!(resolve_join_kind(&TableJoinType::Equal, true), Inner);
    }

    #[test]
    fn link_op_resolution_maps_only_inequalities() {
        use LinkOp::*;
        assert_eq!(resolve_link_op(&TableJoinType::GreaterThan), Gt);
        assert_eq!(resolve_link_op(&TableJoinType::LessThan), Lt);
        assert_eq!(resolve_link_op(&TableJoinType::NotEqual), Ne);
        assert_eq!(resolve_link_op(&TableJoinType::Equal), Eq);
        // An outer-join link with no explicit operator is still an equijoin.
        assert_eq!(resolve_link_op(&TableJoinType::LeftOuter), Eq);
    }
}
