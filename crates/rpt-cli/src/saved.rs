//! `saved` — the report's decoded saved-data rows (the cached rowset a report carries when saved
//! with data).
//!
//! Unlike `dump` (raw record bytes), this decodes the saved-data batch into typed **rows** against
//! the report's stored schema — the "decoded rows" counterpart to `dump`'s "raw bytes". These are
//! the stored records as they sit in the bytes, not the engine's result rowset (which
//! projects/reorders/groups/formats them).

use std::fmt::Write as _;

use rpt::Rpt;
use serde::Serialize;

use crate::util::{print_json, truncate};

pub(crate) const HELP: &str = "\
rpt saved — the report's decoded saved-data rows

Decodes the saved-data batch (the DataSourceManager / SavedRecordsStream / MemoValuesStream streams)
into typed rows against the report's stored schema, and prints the columns + rows. These are the
stored records in record order — not the engine's result rowset. For the RAW record bytes of the
saved-data block instead, use `rpt dump`.

USAGE:
    rpt saved <file.rpt> [--schema] [--limit N] [--json]

OPTIONS:
    --schema      print only the column schema (names + types) and record count, no rows
    --limit N     show at most N rows (default 20); `all` shows every row
    --json        emit the schema + rows as JSON

EXAMPLES:
    rpt saved report.rpt                # schema + the first 20 rows
    rpt saved report.rpt --schema       # just the columns and record count
    rpt saved report.rpt --limit all --json   # every row, machine-readable
";

/// A cell truncated for display, with null shown as `∅`.
const CELL_MAX: usize = 40;

/// Resolve the row cap: default 20, `all` = every row, a number = that many.
fn resolve_limit(opt: Option<&str>) -> usize {
    match opt {
        None => 20,
        Some(s) if s.eq_ignore_ascii_case("all") => usize::MAX,
        Some(s) => s.parse().unwrap_or(20),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ColumnJson {
    name: String,
    value_type: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedJson<'a> {
    file: &'a str,
    record_count: u32,
    columns: Vec<ColumnJson>,
    shown_rows: usize,
    total_rows: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    rows: Option<Vec<Vec<Option<String>>>>,
}

pub(crate) fn saved(
    file: &str,
    json: bool,
    schema_only: bool,
    limit: Option<&str>,
) -> rpt::Result<()> {
    let rpt = Rpt::open(file)?;
    let Some(data) = rpt.saved_data() else {
        // Distinguish "no saved data at all" from "a descriptor is present but the rows did not
        // decode" — the latter is a real RE signal (an undecoded batch class).
        let has_descriptor =
            rpt.report().has_saved_data || rpt.saved_record_count().is_some_and(|n| n > 0);
        if json {
            print_json(&SavedJson {
                file,
                record_count: rpt.saved_record_count().unwrap_or(0),
                columns: Vec::new(),
                shown_rows: 0,
                total_rows: 0,
                rows: Some(Vec::new()),
            });
        } else if has_descriptor {
            println!(
                "{file}: a saved-data descriptor is present ({} records) but the rows did not \
                 decode (undecoded batch class — inspect the raw bytes with `rpt dump`)",
                rpt.saved_record_count().unwrap_or(0)
            );
        } else {
            println!("{file}: no saved data");
        }
        return Ok(());
    };

    let total = data.rows.len();
    let cap = resolve_limit(limit);
    let shown = if schema_only { 0 } else { cap.min(total) };

    if json {
        print_json(&SavedJson {
            file,
            record_count: data.record_count,
            columns: data
                .columns
                .iter()
                .map(|c| ColumnJson {
                    name: c.name.clone(),
                    value_type: format!("{:?}", c.value_type),
                })
                .collect(),
            shown_rows: shown,
            total_rows: total,
            rows: if schema_only {
                None
            } else {
                Some(data.rows.iter().take(shown).cloned().collect())
            },
        });
        return Ok(());
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{file}: saved data — {} records · {} columns",
        data.record_count,
        data.columns.len()
    );
    let _ = writeln!(out, "columns (positional):");
    for (i, c) in data.columns.iter().enumerate() {
        let _ = writeln!(out, "   [{i}] {:<34} {:?}", c.name, c.value_type);
    }
    if !schema_only {
        let _ = writeln!(out, "rows (showing {shown} of {total}):");
        for (i, row) in data.rows.iter().take(shown).enumerate() {
            let cells: Vec<String> = row
                .iter()
                .map(|cell| match cell {
                    None => "∅".to_string(),
                    Some(s) => truncate(s, CELL_MAX),
                })
                .collect();
            let _ = writeln!(out, "   #{i:<4} {}", cells.join(" | "));
        }
        if total > shown {
            let _ = writeln!(out, "   … {} more row(s) — use --limit all", total - shown);
        }
    }
    print!("{out}");
    Ok(())
}
