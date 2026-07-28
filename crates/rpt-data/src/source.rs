//! Row sources: the typed rows a report's pipeline consumes.
//!
//! [`RowSource`] is the seam the native query-engine / saved-data feed sits behind.
//! [`SavedDataSource`] reads the stored rows `rpt` already decodes; a live SQL source is
//! a future native-only impl behind the same trait.

use crystal_formula::eval::{Date, Time, Value};
use rpt_model::{FieldValueType, Report, SavedData};
use std::collections::BTreeMap;
use std::sync::Arc;

/// One raw cell fetched from a DB backend, before re-typing to a [`Value`]. Every text-castable
/// column arrives as [`Cell::Text`] (the backend casts it to text and re-types here); a **binary**
/// (blob/bytea) column arrives as [`Cell::Bytes`] so its bytes survive intact for a blob-bound
/// picture object.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    /// A text cell — the form every non-binary column is fetched in (cast to text).
    Text(String),
    /// Raw bytes from a binary column (a blob/bytea), fetched without a text cast.
    Bytes(Vec<u8>),
}

/// One column's name and stored value type.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// The column name (how formulas reference the value).
    pub name: String,
    /// The column's stored value type.
    pub value_type: FieldValueType,
}

/// A materialized data row: field values keyed by the source column name. Both the full
/// `table.field` name and the bare `field` short name resolve (formulas use either).
///
/// The value map is held behind an [`Arc`] so cloning a row is a refcount bump, not a deep copy:
/// the grouping pipeline nests each row into a bucket at every level and shares the one allocation
/// instead of duplicating the map per level. Mutators copy-on-write via [`Arc::make_mut`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    values: Arc<BTreeMap<String, Value>>,
    /// The row's 0-based position in **read order** (source order after record selection, before
    /// sort/group), stamped by the pipeline. Lets the render pass map a printed record back to its
    /// read-order slot for evaluation-time scheduling. `None` before it is stamped.
    read_index: Option<u64>,
}

impl Row {
    /// Look up a field value by name (case-insensitive), trying the given name then its short form.
    pub fn get(&self, name: &str) -> Option<&Value> {
        // Fast path: an already-lower-case ASCII name keys the map directly, and the short form is a
        // borrowed slice — no allocation. Only a name that `to_lowercase` would actually change (an
        // ASCII-uppercase or non-ASCII name) takes the allocating path.
        if name.is_ascii() && !name.bytes().any(|b| b.is_ascii_uppercase()) {
            return self
                .values
                .get(name)
                .or_else(|| self.values.get(name.rsplit('.').next().unwrap_or(name)));
        }
        let lname = name.to_lowercase();
        self.values
            .get(&lname)
            .or_else(|| self.values.get(lname.rsplit('.').next().unwrap_or(&lname)))
    }

    /// This row's read-order index, if stamped by the pipeline.
    pub fn read_index(&self) -> Option<u64> {
        self.read_index
    }

    /// Stamp this row's read-order index.
    pub fn set_read_index(&mut self, idx: u64) {
        self.read_index = Some(idx);
    }

    /// Insert a value under both its full and short (post-last-`.`) names.
    pub fn insert(&mut self, name: &str, value: Value) {
        let lname = name.to_lowercase();
        let short = short_name(&lname);
        let values = Arc::make_mut(&mut self.values);
        if short != lname {
            values.entry(short).or_insert_with(|| value.clone());
        }
        values.insert(lname, value);
    }
}

/// The bare field name after the last `.` (`countries.id` → `id`).
fn short_name(name: &str) -> String {
    name.rsplit('.').next().unwrap_or(name).to_string()
}

/// A source of typed rows for a report's data pipeline — the seam any datasource (a report's saved
/// data, a live database, or a custom in-memory feed) sits behind.
///
/// # Contract
/// - [`columns`](RowSource::columns) returns the **row schema**: one [`Column`] per field, in the
///   order rows are keyed. Each column's `name` is how a formula references the value (see below).
/// - [`rows`](RowSource::rows) **materializes every row eagerly**, in source order (before the
///   pipeline's record selection / sort / grouping). It returns owned [`Row`]s and **may be called
///   more than once** — the pipeline calls it once per [`build_dataset`](crate::build_dataset), and a
///   report with subreports builds a dataset per scope — so an implementation should be cheap to call
///   repeatedly (materialize or clone from a cache rather than re-fetching each time).
///
/// # Column names
/// A [`Row`] resolves a value by either its full `table.field` (or `alias.field`) name or its bare
/// `field` short name (see [`Row::get`] / [`Row::insert`]), so formulas can use either form. Insert a
/// value under its full name and the short name resolves automatically.
///
/// # Who calls it
/// [`build_dataset`](crate::build_dataset) drives a `RowSource` through the pipeline; the render
/// facade and layout engine build on that. [`SavedDataSource`] and [`EmptySource`] are the built-in
/// implementations, and the live-DB backends implement it too.
///
/// # Implementing a custom source
/// ```
/// use rpt_data::{build_dataset, Column, Row, RowSource};
/// use rpt_model::{DataDefinition, FieldValueType};
/// use crystal_formula::eval::Value;
///
/// struct InMemory {
///     columns: Vec<Column>,
///     rows: Vec<Row>,
/// }
///
/// impl RowSource for InMemory {
///     fn columns(&self) -> &[Column] {
///         &self.columns
///     }
///     fn rows(&self) -> Vec<Row> {
///         self.rows.clone()
///     }
/// }
///
/// // Two columns, two rows. Insert under the full `table.field` name; the short name resolves too.
/// let columns = vec![
///     Column { name: "customers.name".into(), value_type: FieldValueType::String },
///     Column { name: "customers.balance".into(), value_type: FieldValueType::Number },
/// ];
/// let mut rows = Vec::new();
/// for (name, balance) in [("Acme", 120.0), ("Globex", 340.0)] {
///     let mut row = Row::default();
///     row.insert("customers.name", Value::Str(name.into()));
///     row.insert("customers.balance", Value::Number(balance));
///     rows.push(row);
/// }
/// let source = InMemory { columns, rows };
///
/// assert_eq!(source.columns().len(), 2);
/// assert_eq!(source.rows()[0].get("name"), Some(&Value::Str("Acme".into())));
///
/// // Feed it through the pipeline (an empty data definition selects/groups nothing → both rows pass).
/// let dataset = build_dataset(&source, &DataDefinition::default());
/// assert_eq!(dataset.iter_detail_rows().len(), 2);
/// ```
pub trait RowSource {
    /// The row schema: the columns every returned [`Row`] is keyed by.
    fn columns(&self) -> &[Column];
    /// Materialize every row, in source order (before selection/sort/grouping). Eager and owned; the
    /// pipeline may call it more than once.
    fn rows(&self) -> Vec<Row>;

    /// Columns whose cells would not parse as their declared type, so a different type was
    /// substituted.
    ///
    /// The substitution is deliberate (one bad cell must not abort a render) but it silently changes
    /// sorting, grouping, and summaries — so the pipeline reports these to its
    /// [`DiagnosticSink`](crate::DiagnosticSink) when one is attached. Defaults to none, so a source
    /// that does no conversion (or does not track it) needs no change.
    fn coercions(&self) -> Vec<ColumnCoercion> {
        Vec::new()
    }
}

/// A [`RowSource`] with no columns and no rows: the no-data path, where only a report's static bands
/// (page/report headers and footers) format. The zero-config default when a report has neither saved
/// data nor a live datasource.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptySource;

impl RowSource for EmptySource {
    fn columns(&self) -> &[Column] {
        &[]
    }
    fn rows(&self) -> Vec<Row> {
        Vec::new()
    }
}

/// Supplies live rows for a (sub)report scope, so the layout engine can render subreports from a
/// live datasource instead of only their saved data. A native caller implements this with a DB
/// fetch keyed by the scope's connection; offline/WASM callers pass `None` and subreports fall back
/// to their saved data. Kept dependency-free (returns a boxed [`RowSource`]) so `rpt-layout` — which
/// calls it while rendering a subreport — stays WASM-safe.
///
/// # End-to-end threading
/// The render caller supplies a provider through `rpt-render`'s `RenderOptions::scope` (the render
/// CLI's `--db` path builds one). While formatting, the layout engine calls
/// [`rows_for`](ScopeData::rows_for) once per subreport, passing that subreport's [`Report`]: a `Some`
/// result feeds the subreport's own pipeline from the returned live rows, and `None` falls back to
/// that subreport's saved data. With no provider, every subreport renders from saved data.
///
/// ```
/// use rpt_data::{Column, Row, RowSource, ScopeData};
/// use rpt_model::Report;
///
/// struct LiveScope;
///
/// impl ScopeData for LiveScope {
///     fn rows_for(&self, report: &Report) -> Option<Box<dyn RowSource>> {
///         // Key a fetch off the scope's tables/connection; `None` falls back to saved data.
///         if report.database.tables.is_empty() {
///             return None;
///         }
///         struct Fetched;
///         impl RowSource for Fetched {
///             fn columns(&self) -> &[Column] { &[] }
///             fn rows(&self) -> Vec<Row> { Vec::new() }
///         }
///         Some(Box::new(Fetched))
///     }
/// }
/// ```
pub trait ScopeData {
    /// Rows for this report scope's tables, or `None` to fall back to the scope's saved data (e.g.
    /// the scope has no live tables, or a fetch failed non-fatally).
    fn rows_for(&self, report: &Report) -> Option<Box<dyn RowSource>>;
}

/// A [`RowSource`] over a report's stored saved data (the offline, no-DB path).
#[derive(Debug, Clone)]
pub struct SavedDataSource {
    columns: Vec<Column>,
    rows: Vec<Row>,
    /// Columns whose stored cells would not parse as their declared type, collected while re-typing.
    coercions: Vec<ColumnCoercion>,
}

impl SavedDataSource {
    /// Build from decoded [`SavedData`] alone, typing each column from the saved batch's own schema.
    /// String/memo cells that decode as absent are treated as the empty string (an empty
    /// persistent-memo); numeric absents are `Null`.
    ///
    /// Real offline renders should prefer [`from_report`](Self::from_report): a saved batch stores
    /// Date/DateTime fields as integer serials *typed as integers*, so the batch schema alone
    /// mistypes them and dates never group/sort/format correctly.
    pub fn new(saved: &SavedData) -> SavedDataSource {
        Self::build(saved, &DeclaredTypes::default())
    }

    /// Build reconciling the saved batch's physical column types against the report's declared field
    /// types. A saved batch stores a Date/DateTime field as an integer Julian-day serial typed as
    /// `Int32s` (`orders.created_at` → serial `2_460_312`); only the report's field definitions know
    /// it is temporal. Re-typing here lets offline renders group/sort/format dates just like the
    /// live-DB path, which types columns from the same declared source.
    ///
    /// This is the right default for an offline render — prefer it over [`new`](Self::new), which
    /// types columns from the batch schema alone and so leaves date fields as bare integers:
    ///
    /// ```no_run
    /// # use rpt_data::SavedDataSource;
    /// # fn demo(saved: &rpt_model::SavedData, report: &rpt_model::Report) {
    /// // Dates re-typed from the report's field definitions → they group/sort/format correctly.
    /// let source = SavedDataSource::from_report(saved, report);
    /// // `SavedDataSource::new(saved)` would type a date-serial column as an integer instead.
    /// # let _ = source;
    /// # }
    /// ```
    pub fn from_report(saved: &SavedData, report: &Report) -> SavedDataSource {
        Self::build(saved, &DeclaredTypes::from_report(report))
    }

    fn build(saved: &SavedData, declared: &DeclaredTypes) -> SavedDataSource {
        let columns: Vec<Column> = saved
            .columns
            .iter()
            .map(|c| Column {
                name: c.name.clone(),
                value_type: declared.get(&c.name).unwrap_or(c.value_type),
            })
            .collect();
        // The saved-data path re-types exactly as the live-DB one does, so it can substitute a type
        // just as silently — it tracks coercions the same way.
        let mut log = CoercionLog::new(&columns);
        let rows = saved
            .rows
            .iter()
            .map(|stored| {
                let mut row = Row::default();
                for (i, col) in columns.iter().enumerate() {
                    // Saved-data cells are stored as text; wrap each in a text [`Cell`] to re-type.
                    let cell = stored
                        .get(i)
                        .and_then(|c| c.as_ref())
                        .map(|s| Cell::Text(s.clone()));
                    let (value, coerced) = cell_to_value_reporting(col.value_type, cell.as_ref());
                    log.note(i, coerced, cell.as_ref());
                    row.insert(&col.name, value);
                }
                row
            })
            .collect();
        let coercions = log.finish();
        SavedDataSource {
            columns,
            rows,
            coercions,
        }
    }
}

/// A report's declared field types, keyed by field name (bare `field` and every `qualifier.field`
/// form, lowercased — matching [`Row::get`]'s resolution). Used to re-type a saved batch's columns,
/// whose stored types are physical (a date serial is stored as an integer). An empty map (the
/// [`Default`], used by [`SavedDataSource::new`]) overrides nothing.
#[derive(Default)]
struct DeclaredTypes(BTreeMap<String, FieldValueType>);

impl DeclaredTypes {
    fn from_report(report: &Report) -> DeclaredTypes {
        let mut map = BTreeMap::new();
        for table in &report.database.tables {
            for field in &table.data_fields {
                map.entry(field.name.to_lowercase())
                    .or_insert(field.value_type);
                for qualifier in [&table.name, &table.alias] {
                    if !qualifier.is_empty() {
                        map.insert(
                            format!("{qualifier}.{}", field.name).to_lowercase(),
                            field.value_type,
                        );
                    }
                }
            }
        }
        DeclaredTypes(map)
    }

    fn get(&self, name: &str) -> Option<FieldValueType> {
        let lname = name.to_lowercase();
        self.0
            .get(&lname)
            .or_else(|| self.0.get(&short_name(&lname)))
            .copied()
    }
}

impl RowSource for SavedDataSource {
    fn columns(&self) -> &[Column] {
        &self.columns
    }
    fn rows(&self) -> Vec<Row> {
        self.rows.clone()
    }
    fn coercions(&self) -> Vec<ColumnCoercion> {
        self.coercions.clone()
    }
}

/// Convert a raw fetched cell (a [`Cell`] + its declared type) to a runtime [`Value`].
pub fn cell_to_value(value_type: FieldValueType, cell: Option<&Cell>) -> Value {
    cell_to_value_reporting(value_type, cell).0
}

/// What [`cell_to_value_reporting`] had to do because a cell would not parse as its declared type.
///
/// The substitution itself is deliberate — one unparseable cell must not abort a render — but it
/// silently changes behaviour: a date column that falls back to text sorts and groups as text, so
/// group headers and date ranges come out wrong, and a summary over it returns 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coercion {
    /// The value could not be parsed as its declared type and was kept as text.
    ToText,
    /// The value could not be parsed and became null, so it is indistinguishable from a real null.
    ToNull,
}

/// [`cell_to_value`], also reporting whether the declared type had to be abandoned.
///
/// `None` means the cell was either absent (a genuine null) or converted cleanly.
pub fn cell_to_value_reporting(
    value_type: FieldValueType,
    cell: Option<&Cell>,
) -> (Value, Option<Coercion>) {
    let value = convert(value_type, cell);
    // A cell that was not there cannot have been mis-converted; only a *present* cell that came out
    // as a different type than declared is a coercion worth reporting.
    let coercion = match (cell, &value) {
        (None, _) | (Some(_), Value::Null) if cell.is_none() => None,
        (Some(_), Value::Null) => Some(Coercion::ToNull),
        (Some(_), Value::Str(_))
            if !matches!(value_type, FieldValueType::String) && !is_texty(value_type) =>
        {
            Some(Coercion::ToText)
        }
        _ => None,
    };
    (value, coercion)
}

/// Whether a declared type is legitimately represented as a [`Value::Str`].
fn is_texty(value_type: FieldValueType) -> bool {
    use FieldValueType as T;
    !matches!(
        value_type,
        T::Int8s
            | T::Int16s
            | T::Int32s
            | T::Int32u
            | T::Number
            | T::Currency
            | T::Boolean
            | T::Date
            | T::Time
            | T::DateTime
    )
}

fn convert(value_type: FieldValueType, cell: Option<&Cell>) -> Value {
    use FieldValueType as T;
    // A binary column keeps its raw bytes so a blob-bound picture receives the real image data. A
    // backend that could only return it as text (no bytes path) keeps the string as a fallback.
    if value_type.is_binary() {
        return match cell {
            Some(Cell::Bytes(b)) => Value::Bytes(b.clone()),
            Some(Cell::Text(t)) if !t.is_empty() => Value::Str(t.clone()),
            _ => Value::Null,
        };
    }
    // Every other type is fetched as a text cell; a stray bytes cell has no text form (→ null).
    let cell: Option<&String> = match cell {
        Some(Cell::Text(t)) => Some(t),
        _ => None,
    };
    match value_type {
        T::Int8s | T::Int16s | T::Int32s | T::Int32u | T::Number => cell
            .and_then(|t| t.trim().parse::<f64>().ok())
            .map(Value::Number)
            .unwrap_or(Value::Null),
        T::Currency => cell
            .and_then(|t| t.trim().parse::<f64>().ok())
            .map(Value::Currency)
            .unwrap_or(Value::Null),
        T::Boolean => match cell {
            Some(t) => Value::Bool(t.trim().eq_ignore_ascii_case("true")),
            None => Value::Null,
        },
        // Date/time fields arrive one of two ways: ISO text (`2024-01-03`, `09:12:00`,
        // `2024-01-03 09:12:00`) from the live-DB `::text` cast, or an integer Julian-day serial
        // (`2460312`) from a saved-data batch, which stores dates as integers. Type them either way
        // so the pipeline can group/sort/format them as dates (date-group bucketing, comparison, and
        // locale display all depend on this). An unparseable value falls back to a plain string.
        T::Date => cell
            .and_then(|t| parse_date_cell(t))
            .map(Value::Date)
            .unwrap_or_else(|| str_or_null(cell)),
        T::Time => cell
            .and_then(|t| parse_iso_time(t))
            .map(Value::Time)
            .unwrap_or_else(|| str_or_null(cell)),
        T::DateTime => cell
            .and_then(|t| parse_datetime_cell(t))
            .map(|(d, t)| Value::DateTime(d, t))
            .unwrap_or_else(|| str_or_null(cell)),
        // String / memo / blob stored as text: absent = empty string.
        _ => Value::Str(cell.cloned().unwrap_or_default()),
    }
}

/// A failure of a live-database [`RowSource`] fetch, generic over the driver's own error type `E`
/// (a `postgres::Error`, a `rusqlite::Error`, …) so both native backends share one error shape while
/// this crate stays free of any DB-driver dependency (and WASM-safe). Each driver aliases it to its
/// own `E`, which keeps the two aliases distinct types — so a downstream `From<DbError<..>>` per
/// driver does not collide.
///
/// It is `#[non_exhaustive]`, so construct it through the [`no_query`](DbError::no_query) /
/// [`connect`](DbError::connect) / [`query`](DbError::query) constructors rather than the variants
/// directly.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DbError<E: std::error::Error + 'static> {
    /// No query could be built for the report, with the query builder's own reason. Previously each
    /// caller synthesized "report has no database table", which was a guess.
    #[error("cannot build a query for this report: {0}")]
    NoQuery(String),
    /// Opening or connecting to the database failed. The driver's error is the
    /// [`source`](std::error::Error::source) and is deliberately *not* interpolated here, so a
    /// chain-printing reporter shows it exactly once.
    #[error("connection failed{}", label_suffix(.label))]
    Connect {
        /// The data source / server the connection was for. With several `RPT_DB_URL_<SERVER>`
        /// variables configured, this is the only thing that says which one is wrong.
        label: Option<String>,
        /// The driver's error.
        #[source]
        source: E,
    },
    /// Preparing or executing a query failed. As [`Connect`](DbError::Connect), the driver's error
    /// is the source rather than part of this message.
    #[error("query failed{}", label_suffix(.label))]
    Query {
        /// The data source / server the query ran against.
        label: Option<String>,
        /// The statement that failed. Not interpolated into the message — a `SELECT` across a dozen
        /// joined tables does not belong on an error line — but available via [`DbError::hint`].
        sql: Option<String>,
        /// The driver's error.
        #[source]
        source: E,
    },
}

/// ` against \`label\``, or nothing when the source is unknown.
fn label_suffix(label: &Option<String>) -> String {
    match label {
        Some(l) => format!(" against `{l}`"),
        None => String::new(),
    }
}

impl<E: std::error::Error + 'static> DbError<E> {
    /// No query could be built, for `reason` (the query builder's own).
    pub fn no_query(reason: impl Into<String>) -> DbError<E> {
        DbError::NoQuery(reason.into())
    }

    /// A connection/open failure carrying the driver's error.
    pub fn connect(err: E) -> DbError<E> {
        DbError::Connect {
            label: None,
            source: err,
        }
    }

    /// A query prepare/execute failure carrying the driver's error.
    pub fn query(err: E) -> DbError<E> {
        DbError::Query {
            label: None,
            sql: None,
            source: err,
        }
    }

    /// Attach the data source this failed against, and the SQL that failed.
    ///
    /// Applied by the driver once, at the top of the fetch, so the many inner `map_err(DbError::query)`
    /// sites stay terse and no site has to remember the context.
    #[must_use]
    pub fn in_context(mut self, source_label: Option<&str>, statement: Option<&str>) -> DbError<E> {
        match &mut self {
            DbError::Connect { label, .. } => *label = source_label.map(str::to_string),
            DbError::Query { label, sql, .. } => {
                *label = source_label.map(str::to_string);
                *sql = statement.map(str::to_string);
            }
            DbError::NoQuery(_) => {}
        }
        self
    }

    /// Extra lines a reporter should print beneath the message: the failing statement, and — when the
    /// driver's complaint looks like a missing table or column — a pointer at `rpt sql`, which prints
    /// every SQL the report can issue with table provenance and needs no database.
    ///
    /// Kept out of `Display` so the error stays one line while the detail is still reachable.
    pub fn hint(&self) -> Option<String> {
        let DbError::Query { sql, source, .. } = self else {
            return None;
        };
        let mut out = Vec::new();
        if let Some(sql) = sql {
            out.push(format!("statement: {}", one_line(sql, 300)));
        }
        let complaint = source.to_string().to_ascii_lowercase();
        if [
            "no such table",
            "does not exist",
            "unknown column",
            "no such column",
        ]
        .iter()
        .any(|m| complaint.contains(m))
        {
            out.push(
                "the report expects a table or column the database does not have; \
                 run `rpt sql <file>` to see every query the report can issue, with the table each \
                 column comes from, and compare that against the schema"
                    .to_string(),
            );
        }
        (!out.is_empty()).then(|| out.join("\n"))
    }
}

/// Collapse whitespace and cap length, so a multi-line `SELECT` reads on one line.
fn one_line(sql: &str, max: usize) -> String {
    let flat = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(max) {
        Some((cut, _)) => format!("{}… (run with -v for the full SQL)", &flat[..cut]),
        None => flat,
    }
}

/// Build the pipeline [`Row`]s from a live-DB driver's result set, applying the shared re-typing
/// rules exactly once.
///
/// `columns` is the query's column projection (each [`Column`]'s `name` is how formulas key the
/// value). `next_cells` is the driver's cursor advance: each call returns the next row's cells as raw
/// [`Cell`]s (`Vec<Option<Cell>>`, positionally aligned with `columns` — text for a cast column, bytes
/// for a binary one), or `None` at end of results. Every cell is re-typed against its column's
/// [`FieldValueType`] via [`cell_to_value`], so the cell→[`Value`] rules live here and cannot drift
/// between DB backends — each backend supplies only its own cell accessor.
///
/// # Errors
///
/// Whatever `next_cells` returns — a driver read failure. This function adds no failures of its own.
pub fn rows_from_cells<E>(
    columns: &[Column],
    next_cells: impl FnMut() -> Result<Option<Vec<Option<Cell>>>, E>,
) -> Result<Vec<Row>, E> {
    rows_from_cells_reporting(columns, next_cells).map(|(rows, _)| rows)
}

/// [`rows_from_cells`], also returning the per-column coercions it had to make.
///
/// Accumulated per column rather than per cell: one column arriving in an unexpected format produces
/// one finding with a row count and an example, not one per row.
///
/// # Errors
///
/// Whatever `next_cells` returns — this function adds no failures of its own.
pub fn rows_from_cells_reporting<E>(
    columns: &[Column],
    mut next_cells: impl FnMut() -> Result<Option<Vec<Option<Cell>>>, E>,
) -> Result<(Vec<Row>, Vec<ColumnCoercion>), E> {
    let mut rows = Vec::new();
    let mut coercions = CoercionLog::new(columns);
    while let Some(cells) = next_cells()? {
        let mut row = Row::default();
        for (i, (col, cell)) in columns.iter().zip(cells).enumerate() {
            let (value, coerced) = cell_to_value_reporting(col.value_type, cell.as_ref());
            coercions.note(i, coerced, cell.as_ref());
            row.insert(&col.name, value);
        }
        rows.push(row);
    }
    Ok((rows, coercions.finish()))
}

/// A column whose cells would not parse as their declared type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnCoercion {
    /// The column's name.
    pub column: String,
    /// The type the report says it is.
    pub declared: FieldValueType,
    /// What the values became instead.
    pub fallback: Coercion,
    /// How many rows were affected.
    pub rows: u64,
    /// One offending raw value, so the format mismatch is visible rather than merely counted.
    pub example: Option<String>,
}

impl ColumnCoercion {
    /// A one-line description, ready for a diagnostic.
    pub fn describe(&self) -> String {
        let became = match self.fallback {
            Coercion::ToText => "kept as text",
            Coercion::ToNull => "treated as null",
        };
        let example = match &self.example {
            Some(v) => format!(" (e.g. {v:?})"),
            None => String::new(),
        };
        let consequence = match self.fallback {
            Coercion::ToText => {
                "so they sort, group, and summarize as text rather than as their declared type"
            }
            Coercion::ToNull => "so they are indistinguishable from genuinely missing values",
        };
        format!(
            "column `{}` is declared {:?} but {} value(s) would not parse and were {became}{example} \
             — {consequence}",
            self.column, self.declared, self.rows
        )
    }
}

/// Accumulates coercions per column while rows are built.
struct CoercionLog<'a> {
    columns: &'a [Column],
    seen: Vec<Option<(Coercion, u64, Option<String>)>>,
}

impl<'a> CoercionLog<'a> {
    fn new(columns: &'a [Column]) -> CoercionLog<'a> {
        CoercionLog {
            columns,
            seen: vec![None; columns.len()],
        }
    }

    fn note(&mut self, index: usize, coercion: Option<Coercion>, cell: Option<&Cell>) {
        let Some(coercion) = coercion else { return };
        let Some(slot) = self.seen.get_mut(index) else {
            return;
        };
        match slot {
            Some((_, count, _)) => *count += 1,
            None => {
                // Keep the first offending value as the example: it shows the format actually
                // arriving, which is what tells the user whether the report or the data is wrong.
                let example = match cell {
                    Some(Cell::Text(t)) => Some(t.clone()),
                    Some(Cell::Bytes(b)) => Some(format!("<{} bytes>", b.len())),
                    None => None,
                };
                *slot = Some((coercion, 1, example));
            }
        }
    }

    fn finish(self) -> Vec<ColumnCoercion> {
        self.seen
            .into_iter()
            .enumerate()
            .filter_map(|(i, slot)| {
                let (fallback, rows, example) = slot?;
                Some(ColumnCoercion {
                    column: self.columns[i].name.clone(),
                    declared: self.columns[i].value_type,
                    fallback,
                    rows,
                    example,
                })
            })
            .collect()
    }
}

/// A materialized query result — the projected [`Column`]s plus every re-typed [`Row`] — with a
/// ready [`RowSource`] impl. The live-DB backends (`rpt-db-postgres`, `rpt-db-sqlite`) build one from
/// their result set and wrap it, so the `{columns, rows}` carrier and its `RowSource` impl live here
/// once instead of being copy-pasted per driver.
#[derive(Debug, Clone, PartialEq)]
pub struct RowData {
    columns: Vec<Column>,
    rows: Vec<Row>,
}

impl RowData {
    /// Wrap an already-built column projection and its rows.
    pub fn new(columns: Vec<Column>, rows: Vec<Row>) -> RowData {
        RowData { columns, rows }
    }

    /// Build from a column projection and a driver's raw text-cell cursor, re-typing every cell
    /// against its column's [`FieldValueType`] via [`cell_to_value`] (through [`rows_from_cells`]).
    /// `next_cells` returns the next row's cells as raw text, or `None` at end of results; a driver
    /// read failure surfaces as `E` rather than panicking.
    ///
    /// # Errors
    ///
    /// Whatever `next_cells` returns — a driver read failure.
    pub fn from_cells<E>(
        columns: Vec<Column>,
        next_cells: impl FnMut() -> Result<Option<Vec<Option<Cell>>>, E>,
    ) -> Result<RowData, E> {
        let rows = rows_from_cells(&columns, next_cells)?;
        Ok(RowData { columns, rows })
    }
}

impl RowSource for RowData {
    fn columns(&self) -> &[Column] {
        &self.columns
    }
    fn rows(&self) -> Vec<Row> {
        self.rows.clone()
    }
}

/// A present-but-unparseable date/time cell keeps its text; an absent one is null.
fn str_or_null(cell: Option<&String>) -> Value {
    match cell {
        Some(s) => Value::Str(s.clone()),
        None => Value::Null,
    }
}

/// A date cell is either ISO text (live-DB cast) or an integer Julian-day serial (saved batch).
fn parse_date_cell(s: &str) -> Option<Date> {
    parse_iso_date(s).or_else(|| parse_serial(s).map(Date::from_julian_serial))
}

/// A datetime cell is either ISO text or an integer Julian-day serial. A saved-batch serial carries
/// only the date part (an `i32` day serial), so its time defaults to midnight.
fn parse_datetime_cell(s: &str) -> Option<(Date, Time)> {
    parse_iso_datetime(s)
        .or_else(|| parse_serial(s).map(|n| (Date::from_julian_serial(n), Time::new(0, 0, 0))))
}

/// Parse an integer date serial, tolerating a trailing `.0` fraction from a numeric text cast.
fn parse_serial(s: &str) -> Option<i64> {
    let t = s.trim();
    let int = t.split_once('.').map_or(t, |(i, _)| i);
    int.parse::<i64>().ok()
}

/// Parse an ISO date `YYYY-MM-DD` (a leading date; any trailing time is ignored by the caller).
fn parse_iso_date(s: &str) -> Option<Date> {
    let mut it = s.trim().splitn(3, '-');
    let year: i32 = it.next()?.trim().parse().ok()?;
    let month: u8 = it.next()?.trim().parse().ok()?;
    let day: u8 = it.next()?.trim().parse().ok()?;
    (1..=12).contains(&month).then_some(())?;
    (1..=31).contains(&day).then_some(())?;
    Some(Date::new(year, month, day))
}

/// Parse an ISO time `HH:MM[:SS]`, ignoring any fractional seconds or trailing timezone.
fn parse_iso_time(s: &str) -> Option<Time> {
    let mut it = s.trim().split(':');
    let hour: u8 = it.next()?.trim().parse().ok()?;
    let minute: u8 = it.next()?.trim().parse().ok()?;
    // Seconds may carry a fraction (`09.5`) or timezone (`09+02`); keep the leading integer part.
    let second: u8 = match it.next() {
        Some(sec) => sec
            .trim()
            .split(['.', '+', '-', 'Z'])
            .next()?
            .parse()
            .ok()?,
        None => 0,
    };
    (hour <= 23 && minute <= 59 && second <= 60).then_some(())?;
    Some(Time::new(hour, minute, second))
}

/// Parse an ISO datetime `YYYY-MM-DD[ T]HH:MM:SS`. A missing time part defaults to midnight.
fn parse_iso_datetime(s: &str) -> Option<(Date, Time)> {
    let s = s.trim();
    let (date_part, time_part) = s.split_once([' ', 'T']).unwrap_or((s, ""));
    let date = parse_iso_date(date_part)?;
    let time = if time_part.trim().is_empty() {
        Time::new(0, 0, 0)
    } else {
        parse_iso_time(time_part).unwrap_or(Time::new(0, 0, 0))
    };
    Some((date, time))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_model::{DbFieldDef, SavedColumn, Table};

    /// A stand-in driver error, so these tests need no DB crate.
    #[derive(Debug)]
    struct Driver(&'static str);

    impl std::fmt::Display for Driver {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for Driver {}

    /// `Connect` / `Query` keep the driver's error as their `source` and must not interpolate it —
    /// a chain-printing reporter would otherwise emit the driver's message twice.
    #[test]
    fn db_error_does_not_interpolate_its_source() {
        for err in [
            DbError::connect(Driver("connection refused")),
            DbError::query(Driver("no such table: orders")),
        ] {
            let top = err.to_string();
            let cause = std::error::Error::source(&err).expect("the driver error is the source");
            assert!(
                !top.contains(&cause.to_string()),
                "`{top}` already carries its cause `{cause}`, so a chain walk would repeat it"
            );
        }
    }

    #[test]
    fn an_unparseable_date_is_reported_once_per_column_with_a_count_and_an_example() {
        use FieldValueType as T;
        let columns = [Column {
            name: "orders.shipped".to_string(),
            value_type: T::Date,
        }];
        // Three rows: two unparseable dates and one good one.
        let mut feed = [Some("03/04/2024"), Some("2024-01-03"), Some("not-a-date")].into_iter();
        let (rows, coercions) =
            rows_from_cells_reporting::<std::convert::Infallible>(&columns, || {
                Ok(feed
                    .next()
                    .map(|v| vec![v.map(|s| Cell::Text(s.to_string()))]))
            })
            .expect("the feed cannot fail");
        assert_eq!(rows.len(), 3);

        // One finding for the column, not one per row.
        assert_eq!(coercions.len(), 1, "{coercions:?}");
        let c = &coercions[0];
        assert_eq!(c.column, "orders.shipped");
        assert_eq!(c.declared, T::Date);
        assert_eq!(c.fallback, Coercion::ToText);
        assert_eq!(c.rows, 2, "only the two unparseable rows count");
        assert_eq!(c.example.as_deref(), Some("03/04/2024"));

        // And the description says what it means, not just what happened.
        let d = c.describe();
        assert!(d.contains("orders.shipped"), "{d}");
        assert!(d.contains("2 value(s)"), "{d}");
        assert!(d.contains("sort, group"), "{d}");
    }

    #[test]
    fn a_column_that_converts_cleanly_reports_nothing() {
        use FieldValueType as T;
        let columns = [
            Column {
                name: "t.n".to_string(),
                value_type: T::Number,
            },
            Column {
                name: "t.s".to_string(),
                value_type: T::String,
            },
        ];
        // Includes a genuine null, which is not a coercion — the cell was simply absent.
        let mut feed = [vec![Some("12.5"), Some("hello")], vec![None, None]].into_iter();
        let (_, coercions) =
            rows_from_cells_reporting::<std::convert::Infallible>(&columns, || {
                Ok(feed.next().map(|r| {
                    r.into_iter()
                        .map(|c| c.map(|s: &str| Cell::Text(s.to_string())))
                        .collect()
                }))
            })
            .expect("the feed cannot fail");
        assert!(coercions.is_empty(), "{coercions:?}");
    }

    #[test]
    fn typed_temporal_cells_parse_iso_text_and_julian_serials() {
        use FieldValueType as T;
        let text = |s: &str| Cell::Text(s.to_string());
        // Live-DB path: ISO text.
        assert_eq!(
            cell_to_value(T::Date, Some(&text("2024-01-03"))),
            Value::Date(Date::new(2024, 1, 3))
        );
        assert_eq!(
            cell_to_value(T::DateTime, Some(&text("2024-01-03 09:12:00"))),
            Value::DateTime(Date::new(2024, 1, 3), Time::new(9, 12, 0))
        );
        // Saved-batch path: integer Julian-day serial (2460312 == 2024-01-03), date only.
        assert_eq!(
            cell_to_value(T::Date, Some(&text("2460312"))),
            Value::Date(Date::new(2024, 1, 3))
        );
        assert_eq!(
            cell_to_value(T::DateTime, Some(&text("2460312"))),
            Value::DateTime(Date::new(2024, 1, 3), Time::new(0, 0, 0))
        );
        // An unparseable temporal cell keeps its text rather than dropping to null.
        assert_eq!(
            cell_to_value(T::Date, Some(&text("not-a-date"))),
            Value::Str("not-a-date".to_string())
        );
        // A binary column keeps its raw bytes as `Value::Bytes`.
        assert_eq!(
            cell_to_value(T::Blob, Some(&Cell::Bytes(vec![1, 2, 3]))),
            Value::Bytes(vec![1, 2, 3])
        );
    }

    #[test]
    fn get_is_case_insensitive_via_both_lookup_paths() {
        let mut row = Row::default();
        row.insert("Orders.Amount", Value::Number(42.0));
        // Fast path (already-lower-case ASCII): full and short name both resolve.
        assert_eq!(row.get("orders.amount"), Some(&Value::Number(42.0)));
        assert_eq!(row.get("amount"), Some(&Value::Number(42.0)));
        // Slow path (mixed-case name → `to_lowercase`): same result, full and short.
        assert_eq!(row.get("Orders.Amount"), Some(&Value::Number(42.0)));
        assert_eq!(row.get("AMOUNT"), Some(&Value::Number(42.0)));
        // A genuinely absent name is still `None` on both paths.
        assert_eq!(row.get("orders.missing"), None);
        assert_eq!(row.get("MISSING"), None);
    }

    #[test]
    fn row_data_from_cells_retypes_and_exposes_as_row_source() {
        // The shared carrier the DB drivers wrap: it re-types raw text cells against their columns
        // and serves them through `RowSource`, so a driver only supplies its own cell cursor.
        let columns = vec![
            Column {
                name: "t.name".into(),
                value_type: FieldValueType::String,
            },
            Column {
                name: "t.pop".into(),
                value_type: FieldValueType::Int32s,
            },
        ];
        let mut cells = vec![
            vec![
                Some(Cell::Text("Toronto".into())),
                Some(Cell::Text("100".into())),
            ],
            vec![Some(Cell::Text("Ottawa".into())), None],
        ]
        .into_iter();
        let data: RowData =
            RowData::from_cells(columns, || Ok::<_, std::convert::Infallible>(cells.next()))
                .unwrap();

        assert_eq!(
            data.columns()
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["t.name", "t.pop"]
        );
        let rows = data.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("name"), Some(&Value::Str("Toronto".into())));
        assert_eq!(rows[0].get("pop"), Some(&Value::Number(100.0)));
        // A null numeric cell re-types to Null.
        assert_eq!(rows[1].get("pop"), Some(&Value::Null));
    }

    #[test]
    fn from_report_retypes_a_serial_column_declared_datetime() {
        // A saved batch that stored `orders.created_at` (a DateTime field) as an Int32s serial.
        let saved = SavedData {
            record_count: 2,
            columns: vec![
                SavedColumn {
                    name: "orders.id".to_string(),
                    value_type: FieldValueType::Int32s,
                },
                SavedColumn {
                    name: "orders.created_at".to_string(),
                    value_type: FieldValueType::Int32s,
                },
            ],
            rows: vec![
                vec![Some("1".to_string()), Some("2460312".to_string())],
                vec![Some("2".to_string()), Some("2460314".to_string())],
            ],
        };
        // A report whose database declares created_at as DateTime.
        let field = |name: &str, vt: FieldValueType| DbFieldDef {
            name: name.to_string(),
            value_type: vt,
            ..Default::default()
        };
        let table = Table {
            name: "orders".to_string(),
            data_fields: vec![
                field("id", FieldValueType::Int32s),
                field("created_at", FieldValueType::DateTime),
            ],
            ..Default::default()
        };
        let report = Report {
            database: rpt_model::Database {
                tables: vec![table],
                ..Default::default()
            },
            ..Default::default()
        };

        // `new` alone types from the batch schema → serial surfaces as a bare number.
        let plain = SavedDataSource::new(&saved);
        assert_eq!(
            plain.rows()[0].get("orders.created_at"),
            Some(&Value::Number(2460312.0))
        );

        // `from_report` reconciles against the declared DateTime type → typed date.
        let typed = SavedDataSource::from_report(&saved, &report);
        assert_eq!(
            typed.columns()[1].value_type,
            FieldValueType::DateTime,
            "column re-typed to the declared field type"
        );
        assert_eq!(
            typed.rows()[0].get("orders.created_at"),
            Some(&Value::DateTime(Date::new(2024, 1, 3), Time::new(0, 0, 0)))
        );
        // The short name resolves too, and the second row converts independently.
        assert_eq!(
            typed.rows()[1].get("created_at"),
            Some(&Value::DateTime(Date::new(2024, 1, 5), Time::new(0, 0, 0)))
        );
    }
}
