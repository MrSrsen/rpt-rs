//! Row sources: the typed rows a report's pipeline consumes.
//!
//! [`RowSource`] is the seam the native query-engine / saved-data feed sits behind.
//! [`SavedDataSource`] reads the stored rows `rpt` already decodes; a live SQL source is
//! a future native-only impl behind the same trait.

use crystal_formula::eval::{Date, Time, Value};
use rpt_model::{FieldValueType, Report, SavedData};
use std::collections::BTreeMap;
use std::sync::Arc;

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
        let lname = name.to_lowercase();
        self.values
            .get(&lname)
            .or_else(|| self.values.get(&short_name(&lname)))
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
        let rows = saved
            .rows
            .iter()
            .map(|stored| {
                let mut row = Row::default();
                for (i, col) in columns.iter().enumerate() {
                    let cell = stored.get(i).and_then(|c| c.as_ref());
                    row.insert(&col.name, cell_to_value(col.value_type, cell));
                }
                row
            })
            .collect();
        SavedDataSource { columns, rows }
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
}

/// Convert a stored cell (string form + declared type) to a runtime [`Value`].
pub fn cell_to_value(value_type: FieldValueType, cell: Option<&String>) -> Value {
    use FieldValueType as T;
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

/// Build the pipeline [`Row`]s from a live-DB driver's result set, applying the shared re-typing
/// rules exactly once.
///
/// `columns` is the query's column projection (each [`Column`]'s `name` is how formulas key the
/// value). `next_cells` is the driver's cursor advance: each call returns the next row's cells as raw
/// text (`Vec<Option<String>>`, positionally aligned with `columns`), or `None` at end of results.
/// Every cell is re-typed against its column's [`FieldValueType`] via [`cell_to_value`], so the
/// string→[`Value`] rules live here and cannot drift between DB backends — each backend supplies only
/// its own text-cell accessor.
pub fn rows_from_cells<E>(
    columns: &[Column],
    mut next_cells: impl FnMut() -> Result<Option<Vec<Option<String>>>, E>,
) -> Result<Vec<Row>, E> {
    let mut rows = Vec::new();
    while let Some(cells) = next_cells()? {
        let mut row = Row::default();
        for (col, cell) in columns.iter().zip(cells) {
            row.insert(&col.name, cell_to_value(col.value_type, cell.as_ref()));
        }
        rows.push(row);
    }
    Ok(rows)
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

    #[test]
    fn typed_temporal_cells_parse_iso_text_and_julian_serials() {
        use FieldValueType as T;
        // Live-DB path: ISO text.
        assert_eq!(
            cell_to_value(T::Date, Some(&"2024-01-03".to_string())),
            Value::Date(Date::new(2024, 1, 3))
        );
        assert_eq!(
            cell_to_value(T::DateTime, Some(&"2024-01-03 09:12:00".to_string())),
            Value::DateTime(Date::new(2024, 1, 3), Time::new(9, 12, 0))
        );
        // Saved-batch path: integer Julian-day serial (2460312 == 2024-01-03), date only.
        assert_eq!(
            cell_to_value(T::Date, Some(&"2460312".to_string())),
            Value::Date(Date::new(2024, 1, 3))
        );
        assert_eq!(
            cell_to_value(T::DateTime, Some(&"2460312".to_string())),
            Value::DateTime(Date::new(2024, 1, 3), Time::new(0, 0, 0))
        );
        // An unparseable temporal cell keeps its text rather than dropping to null.
        assert_eq!(
            cell_to_value(T::Date, Some(&"not-a-date".to_string())),
            Value::Str("not-a-date".to_string())
        );
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
