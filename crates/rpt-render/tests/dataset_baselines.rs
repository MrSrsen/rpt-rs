//! L2 — committed **`Dataset` snapshot** baselines: the regression surface for `rpt-data`, the
//! record pipeline.
//!
//! Where this sits. L1 (`apps/rpt-cli/tests/json_baseline.rs`) pins the decode and L3
//! (`postgres_fixtures.rs`) pins the Page IR. Between them sits everything `rpt-data` does — record
//! selection, sort, grouping, summaries, running totals and formula evaluation — producing the
//! [`Dataset`] the layout engine pulls from. Without a baseline here a grouping bug and a pagination
//! bug are the same symptom: a moved Page IR. A diff in this file with L1 green means the *pipeline*
//! changed.
//!
//! One baseline per fixture:
//!   `tests/fixtures/baselines/dataset/<group>/<name>.txt`
//!
//! **Hermetic by construction.** Every fixture reads its own embedded saved data, so this harness
//! needs no database and runs on every checkout under a plain `cargo test`. Formula evaluation can
//! read date specials, so the as-of instant is frozen ([`AS_OF_UNIX`]) — otherwise a
//! `CurrentDate`-using formula would re-bless itself every midnight.
//!
//! **Why a hand-written formatter and not serde.** `rpt-data` has no serde dependency (it takes
//! `rpt-model` *without* the serde feature) and neither does `rpt-formula`, whose [`Value`] is
//! meant to stay reusable from an LSP or a WASM sandbox on minimal deps. Deriving `Serialize` to get
//! a test surface would change two shipping crates' dependency shape, and it would drag
//! `Value::Number`/`Value::Currency` — both `f64` — into a text format on the platform's terms. The
//! formatter below lives entirely in the harness instead: it decides float formatting explicitly
//! (see [`num`]), emits each column exactly once (see [`detail_row`]), and is shaped so a diff is
//! readable — which is the whole point of testing a stage at its own boundary.
//!
//! **What the snapshot deliberately leaves out.**
//! - *Formatted strings.* Values are emitted typed (`Cur 1234.5`, not `$1,234.50`). Display
//!   formatting is resolved by `rpt-layout` against a locale, so formatting here would make the host
//!   locale part of the baseline and duplicate L3.
//! - *`Global`/`Shared` variable state.* The record pipeline never touches
//!   [`SharedState`](rpt_data::SharedState) — those variables accumulate in the layout engine's print
//!   pass, in print order. There is no end-of-pipeline variable state to serialize; it belongs to L3.
//! - *Print-order running totals.* The group-level running total (the value a group header/footer
//!   shows) *is* in the `Dataset`, as a summary keyed `#name`, and is covered. The per-record value a
//!   detail band shows is accumulated by [`RunningTotals`](rpt_data::RunningTotals) as the layout
//!   engine prints records, so it is L3's.
//! - *Diagnostic message text.* The `diagnostics` section pins which fail-open site fired, on which
//!   record — the behavioural fact. The prose is presentation: rewording a message must not turn
//!   eight baselines red.
//! - *The stored definition.* Selection/sort/group specs are L1's surface; this layer records what
//!   the pipeline *did* with them — which rows survived, in which order, nested how.
//!
//! Regenerate after an intentional pipeline change with:
//!
//! ```sh
//! RPT_BLESS=1 cargo test -p rpt-render --test dataset_baselines
//! ```

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rpt_data::{
    build_dataset_opts, classify_eval_time, compile_formulas_at, CollectingSink, Column,
    DataContext, Dataset, DatasetOptions, DateTimeSpecials, EvalDiagnostic, EvalTime,
    FormulaRegistry, GroupInstance, Parameters, Row, SavedDataSource, Summary,
};
use rpt_formula::eval::{Date, EvalContext, Time, Value};
use rpt_formula::RefKind;
use rpt_reader::model::{DataDefinition, FieldKindData};
use rpt_test_support::workspace_root;

/// The frozen "now" every fixture evaluates against, so a formula reading a date special yields the
/// same value today and next year. 2023-11-14T22:13:20Z — the same instant the L3/L4a harnesses use.
const AS_OF_UNIX: i64 = 1_700_000_000;

/// The fixtures this layer covers, and the pipeline path each one buys.
///
/// Deliberately a handful, and deliberately **small**: a snapshot lists every detail row, so a
/// 2,600-row report would cost more baseline than the rest of the corpus and pin nothing the 9-row one
/// does not. Each entry earns its place with a stage the others do not reach.
///
/// Nested grouping is covered by `synthetic/nested_group3`, authored for it: the reports that do nest
/// three levels are the DB-backed Meridian corpus, too large for a hand-diffable snapshot, so the
/// coverage instead comes from a report built small enough to afford: 34 rows, three levels, a Sum at
/// each.
const FIXTURES: &[&str] = &[
    // The smallest end-to-end pipeline: a selection formula, a record sort, one group level, a Sum
    // summary at that level, and the grand total — over six rows.
    "benbrahim777/Big Cells - Mexico",
    // A saved batch that does NOT satisfy its own record selection, which is why it is here: the
    // formula `'Pink' IN {web_colours.name}` is case-sensitive and matches 9 of the 52 stored rows,
    // yet the engine renders all 52 — the database matched them under its own collation at save time
    // and the engine never re-checks. All 52 detail lines are the assertion; dropping to 9 means the
    // selection is being re-applied to a batch that had already passed it.
    "worrall/PinkPaletteSampler",
    // The multi-key sort comparator: three record-sort fields (country, then region, then customer
    // name), no selection and no grouping, so nothing but the ordering is under test. The row count is
    // the point — a comparator that lost its second key permutes the middle of the list.
    "benbrahim777/Country_Region_CustName_sort",
    // Summaries plus per-record formula evaluation: one group level with two Sum summaries, and a
    // `Select … Case` formula returning a string per row.
    "benbrahim777/China Orders, Grouped with dsct",
    // A running-total field (`Sum` of order amount, `OnChangeOfField` evaluate, `NoCondition` reset),
    // which the pipeline surfaces as a summary keyed `#Order Total`.
    "benbrahim777/China Orders, with running totals",
    // Formula evaluation with no grouping: arithmetic (`{Product.Price (SRP)} * 0.75`) and a string
    // builtin (`UpperCase`), evaluated once per record.
    "benbrahim777/Formulas",
    // Date-condition grouping: the group breaks `Weekly` over a date field, so the group keys are
    // period buckets rather than raw values — the `date_bucket` path.
    "parking/orders_weekly",
    // Three group levels with a Sum at every one — the only fixture that nests groups (see above).
    // The tree branches at every level (3 regions, 7 teams, 16 reps, 2-3 rows each) so a degenerate
    // implementation cannot pass it, and the summaries are checked arithmetic: the subtotals and the
    // 4,221.60 grand total are the seeded rows' actual sums.
    "synthetic/nested_group3",
    // The second saved batch that outlives its own selection: `{Orders.Order Amount} =
    // {?Order_Amt_Range}` cannot be evaluated at all here, because the parameter is not supplied at
    // render time — and all 27 stored rows appear regardless, because a stored batch is not re-filtered.
    "benbrahim777/Orders10k",
];

/// The parameter values supplied to a fixture, keyed as [`rpt_data::normalize_param_name`] does.
///
/// `Orders10k`'s selection is `{Orders.Order Amount} = {?Order_Amt_Range}`, one of that fixture's own
/// saved order amounts. A saved batch has already passed record selection, so all 27 rows appear
/// whatever this resolves to — but the assertion still matters: the value reaches the dataset by
/// another route (parameter resolution), and removing it moves the baseline.
///
/// It is the amount as the report means it, not as the batch stores it: saved numeric cells are
/// written scaled by 100 and the decoder divides that out, so this is `10_259.10` rather than the
/// `1_025_910` the bytes hold. Get this wrong and the fixture keeps zero rows and asserts nothing.
fn params_for(rel: &str) -> Parameters {
    let mut params = Parameters::new();
    if rel == "benbrahim777/Orders10k" {
        params.insert("order_amt_range".to_string(), Value::Currency(10_259.10));
    }
    params
}

// ---------------------------------------------------------------------------------------------
// Value formatting
// ---------------------------------------------------------------------------------------------

/// Format an `f64` for the snapshot: **six fixed decimal places, then trailing zeros and a trailing
/// `.` trimmed**. So `42.0` prints `42`, `1.5` prints `1.5`, and `1.0/3.0` prints `0.333333`.
///
/// The rule is stated explicitly because the alternative — a shortest-round-trip `{}` — makes the
/// baseline sensitive to the last bit of a floating-point accumulation, and every summary is a fold
/// over `f64`. A sum that moves by less than 5e-7 is re-association noise, not a pipeline change; a
/// sum that moves for a real reason (a dropped row, a wrong accumulator) moves by orders of magnitude
/// more. Magnitudes below 5e-7 therefore collapse to `0`, and `-0` normalizes to `0` so the sign of a
/// zero cannot flip a baseline. Rust's float formatting is implemented in `core` rather than delegated
/// to the platform, so the same bits render identically on every host.
fn num(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    let mut s = format!("{v:.6}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".to_string();
    }
    s
}

/// A double-quoted, single-line rendering of a string cell. Every row is one line, so a value carrying
/// a newline must not be able to forge one.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:04x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// FNV-1a over a blob, so a binary cell is pinned by content without a hex dump of a JPEG in the
/// baseline — and without a hashing dependency.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn date(d: Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

fn time(t: Time) -> String {
    format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second)
}

/// A typed, single-line rendering of a [`Value`]. The type tag is part of the output so a cell that
/// changes type — the classic re-typing regression — is a visible diff rather than an identical
/// number.
fn value(v: &Value) -> String {
    match v {
        Value::Number(n) => format!("Num {}", num(*n)),
        Value::Currency(n) => format!("Cur {}", num(*n)),
        Value::Str(s) => format!("Str {}", quote(s)),
        Value::Bool(b) => format!("Bool {b}"),
        Value::Date(d) => format!("Date {}", date(*d)),
        Value::Time(t) => format!("Time {}", time(*t)),
        Value::DateTime(d, t) => format!("DateTime {} {}", date(*d), time(*t)),
        Value::Bytes(b) => format!("Bytes len={} fnv={:016x}", b.len(), fnv1a(b)),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(value).collect();
            format!("Array [{}]", inner.join(", "))
        }
        Value::Range {
            lo,
            hi,
            lo_incl,
            hi_incl,
        } => format!(
            "Range {}{} .. {}{}",
            if *lo_incl { '[' } else { '(' },
            value(lo),
            value(hi),
            if *hi_incl { ']' } else { ')' },
        ),
        Value::Null => "Null".to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// The snapshot
// ---------------------------------------------------------------------------------------------

/// One formula field, as the snapshot describes it.
struct FormulaEntry {
    /// The field's name, without the `@`.
    name: String,
    /// When `rpt-data` says it evaluates — a derivation of the body, so a classifier change shows up.
    when: EvalTime,
}

impl FormulaEntry {
    /// Whether the pipeline can evaluate this formula on a bare record. A `WhilePrintingRecords`
    /// formula reads print state (page/record position, `Previous`/`Next`) that exists only once the
    /// layout engine is walking pages, so evaluating it here would invent a value this layer does not
    /// own.
    fn evaluable(&self) -> bool {
        self.when != EvalTime::WhilePrintingRecords
    }
}

/// The report's formula fields, in definition order — a stored fact, so it is stable.
fn formula_entries(data_def: &DataDefinition) -> Vec<FormulaEntry> {
    data_def
        .field_definitions
        .iter()
        .filter_map(|f| match &f.kind {
            FieldKindData::Formula(formula) => Some(FormulaEntry {
                name: f.name.clone(),
                when: classify_eval_time(&formula.text.0),
            }),
            _ => None,
        })
        .collect()
}

/// Everything a detail row needs: the canonical column list, the compiled formulas plus their
/// eval-time classification, and the parameter values a `{?Param}` resolves against.
struct RowFmt<'a> {
    columns: &'a [Column],
    formulas: &'a FormulaRegistry,
    entries: &'a [FormulaEntry],
    params: &'a Parameters,
}

/// One detail row: `ri=<read index>`, then every column's value **in `Dataset::columns` order**, then
/// the evaluable formulas' values after `||`.
///
/// Driving the cells off the column list rather than iterating the row's map is what makes the row's
/// double-keying (every column is stored under both `table.field` and its bare short name) invisible:
/// each column is asked for exactly once, by its canonical name. A column the row does not carry
/// prints `<absent>`, which is distinct from a present-but-null `Null`.
fn detail_row(row: &Row, fmt: &RowFmt<'_>) -> String {
    let mut line = match row.read_index() {
        Some(i) => format!("ri={i}"),
        None => "ri=-".to_string(),
    };
    for column in fmt.columns {
        let cell = match row.get(&column.name) {
            Some(v) => value(v),
            None => "<absent>".to_string(),
        };
        let _ = write!(line, " | {cell}");
    }
    // A fresh context per row: no `SharedState`, no `RunningTotals`, no `SummaryScope`. Those are the
    // layout engine's print-pass injections, so a formula reaching for one resolves `Null` here — the
    // honest answer at this boundary, and stable because nothing accumulates across rows.
    let ctx = DataContext::new(row, fmt.formulas).with_params(fmt.params);
    for entry in fmt.entries.iter().filter(|e| e.evaluable()) {
        let resolved = ctx
            .resolve(RefKind::Formula, &entry.name)
            .map(|v| value(&v))
            .unwrap_or_else(|| "<unresolved>".to_string());
        let _ = write!(line, " || @{}={resolved}", entry.name);
    }
    line
}

/// Render a summary list under `label`: `Sum(Orders.Order Amount) = Cur 1234.5` per entry.
fn write_summaries(out: &mut String, indent: usize, label: &str, summaries: &[Summary]) {
    let pad = " ".repeat(indent);
    let _ = writeln!(out, "{pad}{label} ({}):", summaries.len());
    for s in summaries {
        let _ = writeln!(
            out,
            "{pad}  {:?}({}) = {}",
            s.operation,
            s.field,
            value(&s.value)
        );
    }
}

/// Render one group instance and, recursively, its subgroups. Each level indents by two, so nesting is
/// visible at a glance and a group inserted at depth 2 stays a localized diff.
fn write_group(out: &mut String, indent: usize, group: &GroupInstance, fmt: &RowFmt<'_>) {
    let pad = " ".repeat(indent);
    let _ = writeln!(
        out,
        "{pad}level {} field={} key={}",
        group.level,
        group.condition_field,
        value(&group.key)
    );
    write_summaries(out, indent + 2, "summaries", &group.summaries);
    if group.subgroups.is_empty() {
        // Only the deepest level carries details, so a leaf always states its count — including zero,
        // which is itself a fact worth pinning (a group selection that kept an empty instance).
        let _ = writeln!(out, "{pad}  details ({}):", group.details.len());
        for row in &group.details {
            let _ = writeln!(out, "{pad}    {}", detail_row(row, fmt));
        }
    } else {
        let _ = writeln!(out, "{pad}  subgroups ({}):", group.subgroups.len());
        for sub in &group.subgroups {
            write_group(out, indent + 4, sub, fmt);
        }
    }
    // Hierarchical grouping only: instances of this same level nested under this one. Written only
    // when present, so an ordinary group's snapshot is unchanged.
    if !group.hierarchy_children.is_empty() {
        let _ = writeln!(
            out,
            "{pad}  hierarchy_children ({}):",
            group.hierarchy_children.len()
        );
        for child in &group.hierarchy_children {
            write_group(out, indent + 4, child, fmt);
        }
    }
}

/// Project a built [`Dataset`] into its snapshot text.
///
/// The formula registry is rebuilt here with the same as-of instant the pipeline used, so a per-row
/// formula value is what the pipeline's own contexts would have produced.
fn snapshot(
    label: &str,
    dataset: &Dataset,
    data_def: &DataDefinition,
    diagnostics: &[EvalDiagnostic],
    as_of: DateTimeSpecials,
) -> String {
    let entries = formula_entries(data_def);
    let formulas = compile_formulas_at(data_def, as_of);
    let fmt = RowFmt {
        columns: &dataset.columns,
        formulas: &formulas,
        entries: &entries,
        params: &dataset.params,
    };

    let mut out = String::new();
    let _ = writeln!(out, "# L2 dataset snapshot: {label}");
    let _ = writeln!(out, "row_count: {}", dataset.row_count);
    let _ = writeln!(out);

    let _ = writeln!(out, "columns ({}):", dataset.columns.len());
    for (i, c) in dataset.columns.iter().enumerate() {
        let _ = writeln!(out, "  {i:>3} {} : {:?}", c.name, c.value_type);
    }
    let _ = writeln!(out);

    // `Parameters` is a `HashMap`, so the snapshot sorts it or it is nondeterministic.
    let mut params: Vec<(&String, &Value)> = dataset.params.iter().collect();
    params.sort_by(|a, b| a.0.cmp(b.0));
    let _ = writeln!(out, "params ({}):", params.len());
    for (name, v) in params {
        let _ = writeln!(out, "  {name} = {}", value(v));
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "formulas ({}):", entries.len());
    for e in &entries {
        let _ = writeln!(
            out,
            "  @{} {:?}{}",
            e.name,
            e.when,
            if e.evaluable() {
                ""
            } else {
                " (print pass — not evaluated here)"
            }
        );
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "diagnostics ({}):", diagnostics.len());
    for d in diagnostics {
        let _ = writeln!(
            out,
            "  {:?} source={} record={} span={}",
            d.kind,
            d.source.as_deref().unwrap_or("-"),
            d.record_index
                .map(|i| i.to_string())
                .unwrap_or_else(|| "-".to_string()),
            d.span
                .as_ref()
                .map(|s| format!("{}..{}", s.start, s.end))
                .unwrap_or_else(|| "-".to_string()),
        );
    }
    let _ = writeln!(out);

    write_summaries(&mut out, 0, "grand_total", &dataset.grand_total);
    let _ = writeln!(out);

    if dataset.groups.is_empty() {
        let _ = writeln!(out, "details ({}):", dataset.details.len());
        for row in &dataset.details {
            let _ = writeln!(out, "  {}", detail_row(row, &fmt));
        }
    } else {
        let _ = writeln!(out, "groups ({}):", dataset.groups.len());
        for g in &dataset.groups {
            write_group(&mut out, 2, g, &fmt);
        }
    }
    out
}

/// Build the fixture's dataset from its own saved data and project it into the snapshot text.
///
/// The build mirrors the render facade's own (`rpt_render::render` → `build_and_lay_out`): same
/// source, same [`DatasetOptions`], same as-of instant, same `dataset.params` assignment — so this
/// snapshot describes the very dataset a render of this fixture lays out, not a second,
/// differently-built one.
fn snapshot_of(path: &Path, label: &str) -> String {
    let rpt = rpt_reader::Rpt::open(path).expect("open fixture report");
    let report = rpt.report();
    let saved = report.saved_data.as_ref().unwrap_or_else(|| {
        panic!("{label}: no saved data — this layer's fixtures must be hermetic")
    });
    let source = SavedDataSource::from_report(saved, report);
    let as_of = DateTimeSpecials::from_unix_seconds(AS_OF_UNIX);
    let params = params_for(label);
    let sink = CollectingSink::new();
    let mut dataset = build_dataset_opts(
        &source,
        &report.data_definition,
        DatasetOptions {
            params: Some(&params),
            sink: Some(&sink),
            datetime: Some(as_of),
            ..Default::default()
        },
    );
    dataset.params = params;
    snapshot(
        label,
        &dataset,
        &report.data_definition,
        &sink.into_diagnostics(),
        as_of,
    )
}

// ---------------------------------------------------------------------------------------------
// Bless / check mechanics — the same shape as the other layers' harnesses
// ---------------------------------------------------------------------------------------------

/// A git-style unified diff, matching the other baseline harnesses' reporting.
fn unified_diff(name: &str, baseline: &str, current: &str) -> String {
    let body = similar::TextDiff::from_lines(baseline, current)
        .unified_diff()
        .context_radius(3)
        .header(&format!("{name} (baseline)"), &format!("{name} (current)"))
        .to_string();
    format!("{name}: dataset differs from baseline\n{body}")
}

/// Compare `actual` against the committed baseline at `path`, or write it when blessing. Returns a
/// diff to report on mismatch.
fn check(label: &str, path: &Path, actual: &str, bless: bool) -> Option<String> {
    if bless {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("create baselines dir");
        }
        std::fs::write(path, actual).expect("write baseline");
        return None;
    }
    match std::fs::read_to_string(path) {
        Ok(expected) => {
            let expected = expected.replace("\r\n", "\n");
            (expected != actual).then(|| unified_diff(label, &expected, actual))
        }
        Err(_) => Some(format!(
            "{label}: missing baseline {} (run with RPT_BLESS=1)",
            path.display()
        )),
    }
}

/// Resolve a fixture's `<group>/<name>` key to the committed report, or `None` when this checkout does
/// not carry it.
fn report_path(root: &Path, rel: &str) -> Option<PathBuf> {
    let path = root
        .join("tests/fixtures/reports")
        .join(format!("{rel}.rpt"));
    path.is_file().then_some(path)
}

#[test]
fn datasets_match_baselines() {
    let root = workspace_root();
    let bless = std::env::var_os("RPT_BLESS").is_some();
    let baselines = root.join("tests/fixtures/baselines/dataset");

    let mut ran = 0usize;
    let mut skipped = 0usize;
    let mut lines = 0usize;
    let mut failures = Vec::new();
    for rel in FIXTURES {
        let Some(path) = report_path(&root, rel) else {
            eprintln!("SKIP {rel}: not present on this checkout");
            skipped += 1;
            continue;
        };
        let text = snapshot_of(&path, rel);
        ran += 1;
        lines += text.lines().count();
        if let Some(d) = check(rel, &baselines.join(format!("{rel}.txt")), &text, bless) {
            failures.push(d);
        }
    }

    eprintln!(
        "dataset baselines: {ran} fixture(s) / {lines} line(s) {}, {skipped} skipped",
        if bless { "blessed" } else { "checked" }
    );
    // Asserted in BOTH modes, deliberately. A bless that matched no fixture writes no baseline and
    // would otherwise exit green, leaving an empty baseline tree that later reads as "covered".
    assert_eq!(
        ran,
        FIXTURES.len(),
        "{skipped} of the {} committed reports listed in FIXTURES are missing, so their baselines \
         were neither checked nor blessed",
        FIXTURES.len()
    );
    if bless {
        return;
    }
    assert!(
        failures.is_empty(),
        "{} dataset baseline mismatch(es):\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_reader::model::{FieldValueType, SummaryOperation};

    fn leaf(level: usize, key: &str, rows: &[(&str, f64)]) -> GroupInstance {
        GroupInstance {
            level,
            condition_field: format!("t.g{level}"),
            key: Value::Str(key.to_string()),
            date_condition: None,
            summaries: vec![Summary {
                operation: SummaryOperation::Sum,
                field: "t.amount".to_string(),
                value: Value::Currency(rows.iter().map(|(_, v)| v).sum()),
            }],
            subgroups: Vec::new(),
            hierarchy_children: Vec::new(),
            details: rows
                .iter()
                .enumerate()
                .map(|(i, (name, amount))| {
                    let mut row = Row::default();
                    row.insert("t.name", Value::Str((*name).to_string()));
                    row.insert("t.amount", Value::Currency(*amount));
                    row.set_read_index(i as u64);
                    row
                })
                .collect(),
        }
    }

    fn wrap(level: usize, key: &str, child: GroupInstance) -> GroupInstance {
        GroupInstance {
            level,
            condition_field: format!("t.g{level}"),
            key: Value::Str(key.to_string()),
            date_condition: None,
            summaries: Vec::new(),
            subgroups: vec![child],
            hierarchy_children: Vec::new(),
            details: Vec::new(),
        }
    }

    /// The `Dataset` group tree is recursive and no hermetic fixture nests (see [`FIXTURES`]), so the
    /// formatter's recursion is pinned here, over a hand-built three-level tree.
    #[test]
    fn nested_group_tree_indents_by_level() {
        let columns = vec![
            Column {
                name: "t.name".to_string(),
                value_type: FieldValueType::String,
            },
            Column {
                name: "t.amount".to_string(),
                value_type: FieldValueType::Currency,
            },
        ];
        let formulas = FormulaRegistry::new();
        let params = Parameters::new();
        let fmt = RowFmt {
            columns: &columns,
            formulas: &formulas,
            entries: &[],
            params: &params,
        };
        let top = wrap(
            0,
            "top",
            wrap(1, "mid", leaf(2, "leaf", &[("a", 1.5), ("b", 2.0)])),
        );
        let mut out = String::new();
        write_group(&mut out, 2, &top, &fmt);
        assert_eq!(
            out,
            concat!(
                "  level 0 field=t.g0 key=Str \"top\"\n",
                "    summaries (0):\n",
                "    subgroups (1):\n",
                "      level 1 field=t.g1 key=Str \"mid\"\n",
                "        summaries (0):\n",
                "        subgroups (1):\n",
                "          level 2 field=t.g2 key=Str \"leaf\"\n",
                "            summaries (1):\n",
                "              Sum(t.amount) = Cur 3.5\n",
                "            details (2):\n",
                "              ri=0 | Str \"a\" | Cur 1.5\n",
                "              ri=1 | Str \"b\" | Cur 2\n",
            )
        );
    }

    /// The documented float rule, at its edges.
    #[test]
    fn float_rule_holds_at_its_edges() {
        assert_eq!(num(42.0), "42");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(1.0 / 3.0), "0.333333");
        assert_eq!(num(1e-9), "0");
        assert_eq!(num(f64::NAN), "NaN");
        assert_eq!(num(f64::NEG_INFINITY), "-inf");
    }

    /// A cell may not forge a line break or a column separator.
    #[test]
    fn strings_stay_on_one_line() {
        assert_eq!(quote("a\nb"), "\"a\\nb\"");
        assert_eq!(quote("q\"\\"), "\"q\\\"\\\\\"");
    }
}
