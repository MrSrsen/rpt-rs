//! Formula-evaluation gate: evaluate each report's stored formulas over its saved-data
//! rows and compare against the engine oracle's processed result rowset (the formula columns in
//! the oracle's `*.saved.xml` are the real engine's evaluated values).
//!
//! Env-gated: skips (passing) when `samples/` or the oracle dir is absent, so CI without the
//! private corpus stays green. Never embeds fixture bytes.

use crystal_formula::eval::{Evaluator, MapContext, Value};
use crystal_formula::{parse, EvalError, RefKind, Syntax};
use quick_xml::events::Event;
use quick_xml::Reader;
use rpt::model::{FieldKindData, FieldValueType, Report};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The oracle's processed rowset: short column names + row-major cells (None = null cell).
struct OracleRows {
    columns: Vec<String>,
    rows: Vec<Vec<Option<String>>>,
}

fn parse_oracle_saved(path: &Path) -> Option<OracleRows> {
    let xml = std::fs::read_to_string(path).ok()?;
    let mut reader = Reader::from_str(&xml);
    let mut columns = Vec::new();
    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    let mut cur_row: Option<Vec<Option<String>>> = None;
    let mut cur_cell: Option<String> = None;
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == b"Column" => {
                for attr in e.attributes().flatten() {
                    if attr.key.as_ref() == b"Name" {
                        columns.push(String::from_utf8_lossy(&attr.value).into_owned());
                    }
                }
            }
            Event::Start(e) if e.name().as_ref() == b"Row" => cur_row = Some(Vec::new()),
            Event::End(e) if e.name().as_ref() == b"Row" => {
                if let Some(r) = cur_row.take() {
                    rows.push(r);
                }
            }
            Event::Start(e) if cur_row.is_some() && e.name().as_ref() == b"F" => {
                cur_cell = Some(String::new());
            }
            Event::Empty(e) if cur_row.is_some() && e.name().as_ref() == b"F" => {
                cur_row.as_mut().unwrap().push(None);
            }
            Event::Text(t) => {
                if let Some(c) = cur_cell.as_mut() {
                    c.push_str(&t.unescape().ok()?);
                }
            }
            Event::End(e) if e.name().as_ref() == b"F" => {
                if let (Some(r), Some(c)) = (cur_row.as_mut(), cur_cell.take()) {
                    r.push(Some(c));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Some(OracleRows { columns, rows })
}

/// Convert a stored saved-data cell (string form) to a runtime [`Value`] by its column type.
///
/// String/memo columns: a `None` cell is an **empty** value (`""`), not null — the `rpt`
/// saved-data decoder collapses an empty persistent-memo to `None`, and an empty
/// persistent-memo is semantically the empty string. Numeric columns keep `None` → `Null`.
fn cell_value(value_type: FieldValueType, cell: Option<&String>) -> Value {
    match value_type {
        FieldValueType::Int8s
        | FieldValueType::Int16s
        | FieldValueType::Int32s
        | FieldValueType::Int32u
        | FieldValueType::Number => cell
            .and_then(|t| t.trim().parse::<f64>().ok())
            .map(Value::Number)
            .unwrap_or(Value::Null),
        FieldValueType::Currency => cell
            .and_then(|t| t.trim().parse::<f64>().ok())
            .map(Value::Currency)
            .unwrap_or(Value::Null),
        FieldValueType::Boolean => match cell {
            Some(t) => Value::Bool(t.trim().eq_ignore_ascii_case("true")),
            None => Value::Null,
        },
        // Strings, memos, and anything not yet convertible flow through as text; absent = "".
        _ => Value::Str(cell.cloned().unwrap_or_default()),
    }
}

/// Lenient our-value vs oracle-cell comparison: numeric when both sides parse, else string.
fn matches_oracle(ours: &Value, oracle: Option<&String>) -> bool {
    match (ours, oracle) {
        (Value::Null, None) => true,
        (Value::Null, Some(s)) => s.is_empty(),
        (v, Some(s)) => match v {
            Value::Number(n) | Value::Currency(n) => s
                .trim()
                .parse::<f64>()
                .map(|o| (o - n).abs() < 1e-9)
                .unwrap_or(false),
            Value::Str(t) => t == s,
            Value::Bool(b) => s.eq_ignore_ascii_case(if *b { "true" } else { "false" }),
            _ => false,
        },
        (_, None) => false,
    }
}

/// Per-formula tally over one report's rows.
#[derive(Debug, Default)]
struct Tally {
    matched: usize,
    mismatched: usize,
    errored: usize,
    first_error: Option<EvalError>,
}

fn check_report(report: &Report, oracle: &OracleRows, out: &mut Vec<(String, Tally)>) {
    let Some(saved) = &report.saved_data else {
        return;
    };
    // Stored columns keyed by short name (the oracle uses `field`, storage `table.field`).
    let stored_short: Vec<String> = saved
        .columns
        .iter()
        .map(|c| c.name.rsplit('.').next().unwrap_or(&c.name).to_string())
        .collect();
    let formulas: HashMap<String, (&str, Syntax)> = report
        .data_definition
        .field_definitions
        .iter()
        .filter_map(|f| match &f.kind {
            FieldKindData::Formula(ff) => Some((
                f.name.clone(),
                (
                    ff.text.0.as_str(),
                    match ff.syntax {
                        rpt::model::FormulaSyntax::Basic => Syntax::Basic,
                        _ => Syntax::Crystal,
                    },
                ),
            )),
            _ => None,
        })
        .collect();
    for (col_idx, col_name) in oracle.columns.iter().enumerate() {
        let Some((body, syntax)) = formulas.get(col_name) else {
            continue;
        };
        let (ast, _diags) = parse(body, *syntax);
        let mut tally = Tally::default();
        for (row_idx, oracle_row) in oracle.rows.iter().enumerate() {
            let Some(stored_row) = saved.rows.get(row_idx) else {
                break;
            };
            let mut ctx = MapContext::default();
            for (i, col) in saved.columns.iter().enumerate() {
                ctx = ctx.with_field(
                    RefKind::Field,
                    &col.name,
                    cell_value(col.value_type, stored_row.get(i).and_then(|c| c.as_ref())),
                );
                // Some formulas reference the short form too.
                ctx = ctx.with_field(
                    RefKind::Field,
                    &stored_short[i],
                    cell_value(col.value_type, stored_row.get(i).and_then(|c| c.as_ref())),
                );
            }
            match Evaluator::new(&ctx).eval(&ast) {
                Ok(v) => {
                    if matches_oracle(&v, oracle_row.get(col_idx).and_then(|c| c.as_ref())) {
                        tally.matched += 1;
                    } else {
                        tally.mismatched += 1;
                    }
                }
                Err(e) => {
                    tally.errored += 1;
                    tally.first_error.get_or_insert(e);
                }
            }
        }
        out.push((col_name.clone(), tally));
    }
}

#[test]
fn saved_data_formula_columns_match_oracle() {
    let root = workspace_root();
    let samples = root.join("samples");
    let oracle_dir = root.join("research/oracles/saved");
    if !samples.is_dir() || !oracle_dir.is_dir() {
        eprintln!("SKIP: samples/ or research/oracles/saved/ not present");
        return;
    }
    let mut all: Vec<(String, String, Tally)> = Vec::new();
    for entry in std::fs::read_dir(&samples).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().map(|e| e != "rpt").unwrap_or(true) {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let oracle_path = oracle_dir.join(format!("{stem}.saved.xml"));
        if !oracle_path.is_file() {
            continue;
        }
        let Ok(rpt) = rpt::Rpt::open(&path) else {
            continue;
        };
        let Some(oracle) = parse_oracle_saved(&oracle_path) else {
            continue;
        };
        if oracle.rows.is_empty() {
            continue;
        }
        let mut per_formula = Vec::new();
        check_report(rpt.report(), &oracle, &mut per_formula);
        for (formula, tally) in per_formula {
            all.push((stem.clone(), formula, tally));
        }
    }
    let mut evaluated_rows = 0usize;
    for (report, formula, t) in &all {
        eprintln!(
            "{report} @{formula}: {} ok, {} mismatch, {} error{}",
            t.matched,
            t.mismatched,
            t.errored,
            t.first_error
                .as_ref()
                .map(|e| format!(" (first: {e})"))
                .unwrap_or_default()
        );
        evaluated_rows += t.matched + t.mismatched;
    }
    // Hard gate: the known-good target formula must match every row the decoder provides with no
    // mismatch or error. The `rpt` saved-data decoder now decodes all `MemoValuesStream` batches,
    // so all 249 rows of this report are reconstructed and evaluated.
    let target = all
        .iter()
        .find(|(r, f, _)| r == "worrall_AlphaISOsByCountry" && f == "CCTLD_formatted")
        .expect("worrall_AlphaISOsByCountry @CCTLD_formatted must be present");
    assert!(
        target.2.matched >= 249,
        "gate formula matched too few rows: {:?}",
        target.2
    );
    assert_eq!(
        target.2.mismatched + target.2.errored,
        0,
        "gate formula must have zero mismatch/error: {:?}",
        target.2
    );
    assert!(evaluated_rows > 0, "no formula rows evaluated at all");
}
