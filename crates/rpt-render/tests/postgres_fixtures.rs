//! Data-driven render baseline regression over PostgreSQL fixtures.
//!
//! Each fixture is rendered once and compared against its committed **Page IR JSON** baseline — the
//! structural contract between layout and its consumers, and the render-regression surface. The Page
//! IR sees every twip, so it catches movement no serialized-output baseline can (a backend that
//! rounds twips to its own unit cannot report a sub-unit shift at all).
//!
//! PostgreSQL is the single DB technology for the render-test corpus, so the only variable under
//! test is rendering, not the datasource. For each fixture whose report is available, seed a committed `.sql` migration into a
//! PostgreSQL server, render the whole pipeline (decode → data → layout → Page IR) with the
//! **deterministic** [`ApproxLayout`](rpt_render::ApproxLayout) — no system fonts, so the committed
//! baselines are host-independent — and compare the serialized Page IR against its baseline.
//!
//! Because our datasource re-types every column against the report's declared field types (not the
//! DB's), the render is datasource-independent: a given set of rows produces the same pages
//! regardless of which engine served them.
//!
//! **Subreports are fed too.** Every report scope — the main report and each subreport — gets its own
//! rows from the same server ([`PgScopeData`]), because a subreport with no rows formats as nothing
//! and its baseline would then cover none of subreport flow, per-instance link filtering, or
//! subreport pagination while still reading as a healthy fixture.
//!
//! **Parameters are supplied too**, per fixture, by [`fixture_params`]. A report whose record
//! selection filters on `{?Param}` keeps no row at all while the parameter is unbound — the formula
//! fails on every record and the pipeline drops it fail-open — so such a fixture renders as its
//! static headers and its baseline covers nothing, while still looking healthy.
//!
//! **Connection.** The server is taken from `RPT_DB_URL` (else `DATABASE_URL`), in libpq/URL form
//! (`postgres://user:pass@host:port/db` or `host=… port=… user=… dbname=…`). When neither is set the
//! whole test skips, so a DB-less `cargo test` stays green; CI provides a `postgres` service so the
//! corpus actually runs there.
//!
//! Regenerate the baselines after an intentional render change with:
//!
//! ```sh
//! RPT_DB_URL=postgres://postgres:postgres@localhost:5432/postgres \
//!   RPT_BLESS=1 cargo test -p rpt-render --test postgres_fixtures
//! ```
//!
//! Two corpora feed this harness:
//!
//! - **Meridian** (`tests/meridian/`) — the self-contained synthetic universe and the corpus going
//!   forward. The one seed `tests/meridian/sql/meridian.sql` feeds *every* `.rpt` found **recursively**
//!   under `tests/meridian/reports/**` (organized by division: `sales/`, `freight/`, …); each report's
//!   baseline is `tests/meridian/baselines/page-ir/<relpath>.json`, where `<relpath>` is the report's
//!   path relative to `reports/`. The reports dirs may be empty while authoring is in progress; an
//!   empty Meridian corpus contributes no fixtures and never panics.
//! - **Legacy** (`tests/fixtures/`) — the older group-shared corpus (parking/worrall), deprecated in
//!   favor of `tests/meridian/`. Its discovery (below) is kept working for as long as this corpus
//!   exists alongside Meridian.
//!
//! Legacy structure — fixtures are grouped one directory deep by report set (no filename prefix), and
//! each baseline mirrors that `<group>/<name>` path (like the model baseline harness):
//!   tests/fixtures/sql/<group>/<name>.sql                     — schema + SYNTHETIC seed (committed)
//!   tests/fixtures/baselines/page-ir/<group>/<name>.json      — committed Page IR baseline (blessed)
//!
//! A legacy seed drives its reports in one of two cardinalities:
//!   - **Per-report (1:1)** — `sql/<group>/<name>.sql` seeds the single report `<group>/<name>.rpt`.
//!   - **Group-shared (1:N)** — `sql/<group>/<group>.sql` (the seed named after its own group dir) is
//!     the ONE database read identically by *every* report under `reports/<group>/`. This is the
//!     render-parity corpus model: one synthetic DB (e.g. `parking/parking.sql`), many reports authored
//!     against it. A per-report seed for a given report still wins over the group-shared one. A group
//!     with no reports yet contributes no fixtures.
//!
//! Each seed migration is idempotent (`DROP TABLE IF EXISTS` + `CREATE` + `INSERT`) and is re-applied
//! immediately before the report it feeds, so fixtures never observe each other's leftover state. A
//! fixture whose report is absent on this checkout is skipped, so a clean public checkout stays green.

use std::path::{Path, PathBuf};

use rpt_data::{normalize_param_name, Parameters};
use rpt_formula::eval::{Date, Time, Value};
use rpt_test_support::workspace_root as repo_root;

/// The frozen "now" every fixture renders against, so a report using a date special (`CurrentDate`,
/// `Now`) yields the same page today and next year. 2023-11-14T22:13:20Z.
const AS_OF_UNIX: i64 = 1_700_000_000;

/// The fixtures [`fixture_params`] supplies parameter values for. Asserted to be present in the
/// collected corpus, so renaming a report out from under its values fails by name rather than by
/// quietly rendering it down to its static headers again.
const PARAMETERIZED: &[&str] = &["meridian/sales/01_customer_statement"];

/// The report parameter values a fixture renders with — the harness's stand-in for what an operator
/// would be prompted for. A fixture not listed here renders with none, which is correct for a report
/// that declares none.
///
/// The values are chosen to keep the fixture *small*: a parameterized report run wide open is not a
/// better test, it is a multi-megabyte baseline nobody reads.
fn fixture_params(stem: &str) -> Parameters {
    let pairs: Vec<(&str, Value)> = match stem {
        // A per-customer statement, one group (and one subreport instance) per customer. The three
        // customers give the subreport 3, 0 and 10 linked invoices, so its per-instance link filter
        // is shown selecting a different set each time — and correctly selecting nothing for the
        // customer whose invoices are all paid. Statement date and minimum balance are the report's
        // aging pivot and selection floor: the statement date is the render's own frozen instant, so
        // the invoices fall across every aging bucket, and a floor of 0 keeps every invoice the
        // chosen customers have.
        "meridian/sales/01_customer_statement" => vec![
            (
                "StatementDate",
                Value::DateTime(Date::new(2023, 11, 14), Time::new(0, 0, 0)),
            ),
            ("ReportingCurrency", Value::Currency(0.0)),
            (
                "Customer",
                Value::Array(vec![
                    Value::Number(15.0),
                    Value::Number(370.0),
                    Value::Number(393.0),
                ]),
            ),
        ],
        _ => Vec::new(),
    };
    pairs
        .into_iter()
        .map(|(name, value)| (normalize_param_name(name), value))
        .collect()
}

/// The PostgreSQL connection string, from `RPT_DB_URL` (else `DATABASE_URL`). `None` → skip the test.
fn conn_str() -> Option<String> {
    std::env::var("RPT_DB_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// Resolve a fixture's `<group>/<name>` key to the committed fixture at
/// `reports/<group>/<name>.rpt`. `None` = no such report → skip.
fn report_path(rel: &str) -> Option<PathBuf> {
    let path = repo_root()
        .join("tests/fixtures/reports")
        .join(format!("{rel}.rpt"));
    path.is_file().then_some(path)
}

/// An empty row source for a report with no bound database table (renders only its static bands).
struct EmptySource;
impl rpt_data::RowSource for EmptySource {
    fn columns(&self) -> &[rpt_data::Column] {
        &[]
    }
    fn rows(&self) -> Vec<rpt_data::Row> {
        Vec::new()
    }
}

/// One fetched scope's schema and rows behind `Arc`s, so replaying a cached fetch to another
/// subreport instance is a refcount bump plus a shallow row-vector clone rather than a second
/// database round-trip.
#[derive(Clone)]
struct CachedRows {
    columns: std::sync::Arc<Vec<rpt_data::Column>>,
    rows: std::sync::Arc<Vec<rpt_data::Row>>,
}

impl rpt_data::RowSource for CachedRows {
    fn columns(&self) -> &[rpt_data::Column] {
        &self.columns
    }
    fn rows(&self) -> Vec<rpt_data::Row> {
        (*self.rows).clone()
    }
}

/// Feeds every **subreport** scope its own rows from the same server, so a fixture's subreports
/// render their data instead of nothing.
///
/// Without this the layout engine finds no scope provider, falls back to the subreport's saved data
/// (none, in this corpus), and formats an empty subreport — a baseline that looks healthy while
/// covering no subreport behaviour at all.
///
/// The fetch mirrors the main scope's exactly: [`fetch_for_report`](rpt_db_postgres::PostgresSource::fetch_for_report)
/// against the *subreport's own* [`Report`](rpt_reader::model::Report) (its tables, links and SQL Expression
/// fields), with no `WHERE` push-down. The per-instance link filter is applied by the layout engine
/// after the fetch, from the enclosing row's link values — so one unfiltered fetch serves every
/// instance of a subreport, and [`cache`](Self::cache) collapses them to a single round-trip.
struct PgScopeData<'a> {
    conn: &'a str,
    /// Query-key → the fetched rows. Interior-mutable because `rows_for` takes `&self`; the render
    /// is single-threaded, so a `RefCell` suffices.
    cache: std::cell::RefCell<std::collections::HashMap<u64, CachedRows>>,
}

impl<'a> PgScopeData<'a> {
    fn new(conn: &'a str) -> PgScopeData<'a> {
        PgScopeData {
            conn,
            cache: std::cell::RefCell::new(std::collections::HashMap::new()),
        }
    }
}

impl rpt_data::ScopeData for PgScopeData<'_> {
    fn rows_for(&self, report: &rpt_reader::model::Report) -> Option<Box<dyn rpt_data::RowSource>> {
        let sql_exprs: Vec<(String, String)> = report
            .data_definition
            .sql_expression_fields()
            .map(|(f, x)| (f.name.clone(), x.text.clone()))
            .collect();
        // The fetch is fully determined by the scope's table graph and its SQL Expression fields —
        // neither varies across the instances of one subreport, since the parent link is applied
        // after the fetch. Over-keying only ever costs an extra fetch, never wrong rows.
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            format!("{:?}", report.database).hash(&mut h);
            sql_exprs.hash(&mut h);
            h.finish()
        };
        if let Some(hit) = self.cache.borrow().get(&key) {
            return Some(Box::new(hit.clone()));
        }
        match rpt_db_postgres::PostgresSource::fetch_for_report(
            self.conn,
            report,
            None,
            &sql_exprs,
            &[],
        ) {
            Ok(src) => {
                use rpt_data::RowSource as _;
                let cached = CachedRows {
                    columns: std::sync::Arc::new(src.columns().to_vec()),
                    rows: std::sync::Arc::new(src.rows()),
                };
                self.cache.borrow_mut().insert(key, cached.clone());
                Some(Box::new(cached))
            }
            // A scope binding no live table has nothing to query: fall back to its saved data, the
            // same as an offline render.
            Err(e) if e.to_string().contains("no database table") => None,
            // Anything else is a real failure. Falling back here would restore exactly the silence
            // this provider exists to remove — an unfetchable subreport would render empty and the
            // baseline would still pass.
            Err(e) => panic!("subreport fetch rows: {e}"),
        }
    }
}

/// Seed the `.sql` migration into PostgreSQL, render the report from it, and serialize its Page IR.
/// Uses the dependency-free [`ApproxLayout`] so the render is byte-deterministic (host-independent):
/// it measures text from built-in heuristics rather than from any installed face, so no baseline here
/// can encode the blessing machine's font set.
fn render_from_sql(conn: &str, rpt_path: &Path, sql: &str, stem: &str) -> String {
    // The migration is idempotent (DROP/CREATE/INSERT), so re-seeding right before the render leaves
    // exactly this report's tables current, whatever other fixtures did to the shared server.
    rpt_db_postgres::seed(conn, sql).expect("seed postgres");

    let rpt = rpt_reader::Rpt::open(rpt_path).expect("open report");
    let report = rpt.report();
    // Pass the report's SQL Expression fields so `(<text>) AS "<name>"` columns are fetched too.
    let sql_exprs: Vec<(String, String)> = report
        .data_definition
        .sql_expression_fields()
        .map(|(f, x)| (f.name.clone(), x.text.clone()))
        .collect();
    // No WHERE push-down: fetch the seeded rows and let the pipeline apply the record-selection
    // formula, matching how the baselines were blessed (backend-independent output). A report with no
    // bound database table (e.g. an empty base report authored incrementally) renders its static
    // bands from an empty dataset — the same as the offline path — rather than being a hard error.
    // The fixture's parameter values reach both the record selection (which otherwise fails on every
    // row) and, through the built `Dataset`, the layout engine's own formula evaluation.
    let params = fixture_params(stem);
    let as_of = rpt_render::DateTimeSpecials::from_unix_seconds(AS_OF_UNIX);
    let dataset = match rpt_db_postgres::PostgresSource::fetch_for_report(
        conn,
        report,
        None,
        &sql_exprs,
        &[],
    ) {
        Ok(src) => {
            rpt_data::build_dataset_with_params_at(&src, &report.data_definition, &params, as_of)
        }
        Err(e) if e.to_string().contains("no database table") => {
            rpt_data::build_dataset_with_params_at(
                &EmptySource,
                &report.data_definition,
                &params,
                as_of,
            )
        }
        Err(e) => panic!("{stem}: fetch rows: {e}"),
    };
    // Feed every subreport scope from the same server, so a subreport renders its rows rather than
    // nothing. The layout engine applies each instance's link filter over these rows.
    let scope = PgScopeData::new(conn);
    let doc = rpt_render::render_dataset_with(
        report,
        &dataset,
        Box::new(rpt_render::ApproxLayout),
        rpt_render::Locale::default(),
        Some(&scope),
        // A frozen clock, the same instant the dataset was built against, so a fixture using a date
        // special renders a stable baseline instead of one that drifts with the wall clock.
        Some(as_of),
    );

    assert!(!doc.pages.is_empty(), "{stem}: produced at least one page");

    // The document-scoped block first, then one JSON document per page, concatenated with a page
    // marker so a page-count change is a visible diff rather than a silently truncated comparison.
    let mut ir = document_block(&doc);
    ir.push_str(
        &doc.pages
            .iter()
            .map(|p| p.to_normalized_json())
            .enumerate()
            .map(|(i, p)| format!("// page {}\n{p}\n", i + 1))
            .collect::<String>(),
    );
    ir
}

/// The document-scoped half of the Page IR, as a leading `// document` block. Pages alone would leave
/// [`PagedDocument::sections`] — produced by this very layer — invisible to its own baseline.
fn document_block(doc: &rpt_pages::PagedDocument) -> String {
    let value = serde_json::json!({ "sections": doc.sections });
    format!(
        "// document\n{}\n",
        serde_json::to_string_pretty(&value).expect("SectionInfo is always serializable")
    )
}

/// Compare one serialization against its committed baseline, or write it when blessing. Returns the
/// diff to report on mismatch, `None` when it matched (or was blessed).
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
            (expected != *actual).then(|| unified_diff(label, &expected, actual))
        }
        Err(_) => Some(format!(
            "{label}: missing baseline {} (run with RPT_BLESS=1)",
            path.display()
        )),
    }
}

/// Above this, a whole-file unified diff is not computed. The Page IR baseline for a long report runs
/// to tens of megabytes, and diffing two of those costs more memory than the test process has — a
/// change that moves every fixture (a one-twip layout shift does exactly that) would otherwise report
/// nothing at all, because the process is killed while building the diffs rather than printing them.
/// Such a baseline is instead compared block by block, which is bounded by the largest single page.
const FULL_DIFF_LIMIT: usize = 256 * 1024;

/// How many differing blocks a large baseline's report diffs in full. Enough to act on, few enough
/// that a report that moved every page still prints in seconds.
const SAMPLED_BLOCKS: usize = 3;

/// At most this many runs of differing blocks are listed by name before the rest are summarized.
const LISTED_RUNS: usize = 40;

/// A git-style unified diff between the baseline and the current render. A baseline too large to diff
/// whole is reported per block instead: how many of its blocks moved, which ones, and a full diff of
/// the first few — so "one value moved" and "everything moved" do not read alike.
fn unified_diff(name: &str, baseline: &str, current: &str) -> String {
    if baseline.len() <= FULL_DIFF_LIMIT && current.len() <= FULL_DIFF_LIMIT {
        return format!(
            "{name}: render differs from baseline\n{}",
            diff_body(name, baseline, current)
        );
    }
    blockwise_diff(name, baseline, current)
}

/// The unified-diff body for two texts small enough to diff whole.
fn diff_body(name: &str, baseline: &str, current: &str) -> String {
    similar::TextDiff::from_lines(baseline, current)
        .unified_diff()
        .context_radius(3)
        .header(&format!("{name} (baseline)"), &format!("{name} (current)"))
        .to_string()
}

/// One `// …`-headed block of a serialized Page IR: the leading `// document` block, then one per page.
struct Block<'a> {
    /// The marker line without its `// ` prefix (`document`, `page 7`).
    label: &'a str,
    body: &'a str,
}

/// Split a serialized Page IR at its `// …` marker lines. A text with no marker is one unnamed block,
/// so the report degrades to a whole-text comparison rather than losing the diff entirely.
fn blocks(text: &str) -> Vec<Block<'_>> {
    let mut heads: Vec<(usize, usize)> = Vec::new();
    let mut pos = 0usize;
    for line in text.split_inclusive('\n') {
        if line.starts_with("// ") {
            heads.push((pos, pos + line.len()));
        }
        pos += line.len();
    }
    if heads.is_empty() {
        return vec![Block {
            label: "whole file",
            body: text,
        }];
    }
    (0..heads.len())
        .map(|i| {
            let end = heads.get(i + 1).map_or(text.len(), |h| h.0);
            Block {
                label: text[heads[i].0 + 3..heads[i].1].trim_end(),
                body: &text[heads[i].1..end],
            }
        })
        .collect()
}

/// Report a large baseline's divergence block by block: a per-block count, the differing blocks by
/// name, and a full unified diff of the first [`SAMPLED_BLOCKS`] of them.
fn blockwise_diff(name: &str, baseline: &str, current: &str) -> String {
    let b = blocks(baseline);
    let c = blocks(current);
    let differing: Vec<usize> = (0..b.len().max(c.len()))
        .filter(|&i| match (b.get(i), c.get(i)) {
            (Some(x), Some(y)) => x.label != y.label || x.body != y.body,
            _ => true,
        })
        .collect();

    let label = |i: usize| {
        b.get(i)
            .or_else(|| c.get(i))
            .map_or_else(|| format!("block {i}"), |blk| blk.label.to_string())
    };
    let mut out = format!(
        "{name}: render differs from baseline (too large for a whole-file diff)\n  \
         baseline: {} bytes, {} block(s)\n  current:  {} bytes, {} block(s)\n  \
         {} of {} block(s) differ: {}\n",
        baseline.len(),
        b.len(),
        current.len(),
        c.len(),
        differing.len(),
        b.len().max(c.len()),
        runs(&differing, &label),
    );
    if b.len() != c.len() {
        out.push_str(&format!(
            "  page count changed: {} -> {}\n",
            b.len().saturating_sub(1),
            c.len().saturating_sub(1)
        ));
    }

    for &i in differing.iter().take(SAMPLED_BLOCKS) {
        let name = label(i);
        out.push('\n');
        match (b.get(i), c.get(i)) {
            (Some(x), Some(y))
                if x.body.len() <= FULL_DIFF_LIMIT && y.body.len() <= FULL_DIFF_LIMIT =>
            {
                out.push_str(&diff_body(&name, x.body, y.body));
            }
            (Some(x), Some(y)) => out.push_str(&first_divergence(&name, x.body, y.body)),
            (Some(_), None) => out.push_str(&format!("{name}: present in baseline only\n")),
            (None, Some(_)) => out.push_str(&format!("{name}: present in current only\n")),
            (None, None) => unreachable!("index came from the union of both block lists"),
        }
    }
    if differing.len() > SAMPLED_BLOCKS {
        out.push_str(&format!(
            "\n… and {} more differing block(s), named above. Re-bless and read `git diff` for the rest.\n",
            differing.len() - SAMPLED_BLOCKS
        ));
    }
    out
}

/// Render sorted indices as a compact run list (`document, page 4-9, page 30`), truncated after
/// [`LISTED_RUNS`] runs so a report that moved every other page still prints one readable line.
fn runs(indices: &[usize], label: &dyn Fn(usize) -> String) -> String {
    if indices.is_empty() {
        return "none".to_string();
    }
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for &i in indices {
        match runs.last_mut() {
            Some(last) if last.1 + 1 == i => last.1 = i,
            _ => runs.push((i, i)),
        }
    }
    let shown = runs
        .iter()
        .take(LISTED_RUNS)
        .map(|&(s, e)| {
            if s == e {
                label(s)
            } else {
                format!("{}-{}", label(s), label(e))
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    if runs.len() > LISTED_RUNS {
        format!("{shown}, … ({} more run(s))", runs.len() - LISTED_RUNS)
    } else {
        shown
    }
}

/// The first differing line between two texts, reported with its line number and both sides. Used for
/// a single block that is itself too large to diff.
fn first_divergence(name: &str, baseline: &str, current: &str) -> String {
    let mut b = baseline.lines();
    let mut c = current.lines();
    let mut line = 0usize;
    let detail = loop {
        line += 1;
        match (b.next(), c.next()) {
            (Some(x), Some(y)) if x == y => continue,
            (Some(x), Some(y)) => {
                break format!("line {line} differs\n  baseline: {x}\n  current:  {y}");
            }
            (Some(x), None) => break format!("current ends at line {line}; baseline has: {x}"),
            (None, Some(y)) => break format!("baseline ends at line {line}; current has: {y}"),
            (None, None) => {
                break "no differing line — the two differ only in trailing bytes".to_string();
            }
        }
    };
    format!(
        "{name}: too large for a full diff ({} vs {} bytes), so only its first divergence is shown\n{detail}\n",
        baseline.len(),
        current.len()
    )
}

/// One report to check, with the baseline its render is compared against.
struct Fixture {
    stem: String,
    rpt: PathBuf,
    sql: String,
    /// The Page IR JSON baseline — the layer-3 surface.
    baseline_ir: PathBuf,
}

/// Recursively collect `<group>/<name>.sql` migrations under `root` as `(rel-stem, path)`.
fn walk_sql(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_sql(root, &p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("sql") {
            let rel = p.strip_prefix(root).unwrap().with_extension("");
            out.push((rel.to_string_lossy().replace('\\', "/"), p));
        }
    }
}

/// A group-shared seed is `<group>/<group>.sql` — the seed named after its own top-level group
/// directory, read identically by every report in that group. Returns the group name (`None` for a
/// per-report seed or a deeper-nested path).
fn group_shared_of(rel: &str) -> Option<&str> {
    let (group, name) = rel.split_once('/')?;
    (name == group && !group.is_empty()).then_some(group)
}

/// The `.rpt` reports directly under `<report_base>/<group>/`, as `(file-stem, path)`, sorted.
fn reports_in_group(report_base: &Path, group: &str) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(report_base.join(group)) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("rpt") {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            out.push((name, p));
        }
    }
    out.sort();
    out
}

/// Collect the fixtures whose report is available, from the committed seeds under
/// `tests/fixtures/sql/`. Each maps to a baseline under
/// `tests/fixtures/baselines/page-ir/<group>/<name>.json`. A group-shared seed (`<group>/<group>.sql`)
/// fans out to one fixture per report under `reports/<group>/`.
fn collect_fixtures(skipped: &mut usize) -> Vec<Fixture> {
    let root = repo_root();
    let ir_root = root.join("tests/fixtures/baselines/page-ir");
    let report_base = root.join("tests/fixtures/reports");
    let sql_root = root.join("tests/fixtures/sql");
    let mut sqls = Vec::new();
    walk_sql(&sql_root, &sql_root, &mut sqls);
    // Per-report seeds take precedence over a group-shared seed for the same report.
    let per_report: std::collections::HashSet<String> = sqls
        .iter()
        .filter(|(rel, _)| group_shared_of(rel).is_none())
        .map(|(rel, _)| rel.clone())
        .collect();
    let mut out: Vec<Fixture> = Vec::new();
    for (rel, path) in &sqls {
        let sql = std::fs::read_to_string(path).expect("read .sql");
        if let Some(group) = group_shared_of(rel) {
            let reports = reports_in_group(&report_base, group);
            if reports.is_empty() {
                eprintln!("SKIP {rel}: group-shared seed, but no reports under {group}/ yet");
                *skipped += 1;
                continue;
            }
            for (name, rpt) in reports {
                let stem = format!("{group}/{name}");
                if per_report.contains(&stem) {
                    continue; // a dedicated per-report seed handles this report
                }
                out.push(Fixture {
                    sql: sql.clone(),
                    baseline_ir: ir_root.join(format!("{stem}.json")),
                    rpt,
                    stem,
                });
            }
        } else {
            let Some(rpt) = report_path(rel) else {
                eprintln!("SKIP {rel}: no committed report for this seed");
                *skipped += 1;
                continue;
            };
            out.push(Fixture {
                sql,
                baseline_ir: ir_root.join(format!("{rel}.json")),
                rpt,
                stem: rel.clone(),
            });
        }
    }
    out.sort_by(|a, b| a.stem.cmp(&b.stem));
    out.dedup_by(|a, b| a.stem == b.stem);
    out
}

/// Recursively collect `.rpt` reports under `dir` as `(rel-stem, path)`, where `rel-stem` is the path
/// relative to `base` with the extension dropped (forward-slashed). Non-`.rpt` entries (`.gitkeep`,
/// `README.md`) are ignored.
fn walk_rpt(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_rpt(base, &p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("rpt") {
            let rel = p.strip_prefix(base).unwrap().with_extension("");
            out.push((rel.to_string_lossy().replace('\\', "/"), p));
        }
    }
}

/// Collect the self-contained Meridian fixtures: the one seed `tests/meridian/sql/meridian.sql` fans out
/// to every `.rpt` found recursively under `tests/meridian/reports/`, each with a baseline at
/// `tests/meridian/baselines/page-ir/<relpath>.json`. Contributes nothing when the seed or the reports are
/// absent (corpus authoring is ongoing), so an empty corpus never panics. In check mode a report whose
/// baseline has not been blessed yet is counted as skipped rather than failed; bless mode still renders
/// it to create the baseline.
fn collect_meridian_fixtures(bless: bool, skipped: &mut usize) -> Vec<Fixture> {
    let root = repo_root().join("tests/meridian");
    let Ok(tables_sql) = std::fs::read_to_string(root.join("sql/meridian.sql")) else {
        return Vec::new();
    };
    // The Meridian schema has dependent views (pg-init/25-meridian-views.sql) that some reports'
    // SQL Expression fields query. `meridian.sql` only manages the base tables, so a self-contained
    // re-seed must first DROP the views (else `DROP TABLE` fails on the dependency) and then recreate
    // them afterwards — otherwise re-seeding a warm DB either aborts on the view dependency or leaves
    // a view-backed report unable to fetch.
    let views_sql =
        std::fs::read_to_string(root.join("sql/pg-init/25-meridian-views.sql")).unwrap_or_default();
    let sql = format!(
        "DROP VIEW IF EXISTS invoice_payment_totals, exchange_rate_latest CASCADE;\n{tables_sql}\n{views_sql}"
    );
    let reports_base = root.join("reports");
    let mut reports = Vec::new();
    walk_rpt(&reports_base, &reports_base, &mut reports);
    reports.sort();
    let ir_root = root.join("baselines/page-ir");
    let mut out = Vec::new();
    for (rel, rpt) in reports {
        let baseline_ir = ir_root.join(format!("{rel}.json"));
        if !bless && !baseline_ir.exists() {
            eprintln!(
                "SKIP meridian/{rel}: baseline not blessed yet ({})",
                baseline_ir.display()
            );
            *skipped += 1;
            continue;
        }
        out.push(Fixture {
            sql: sql.clone(),
            baseline_ir,
            rpt,
            stem: format!("meridian/{rel}"),
        });
    }
    out
}

#[test]
fn postgres_fixtures_match_baselines() {
    let Some(conn) = conn_str() else {
        eprintln!(
            "SKIP postgres_fixtures: set RPT_DB_URL (or DATABASE_URL) to a PostgreSQL server to run \
             the render-parity corpus"
        );
        return;
    };
    let bless = std::env::var_os("RPT_BLESS").is_some();
    let mut skipped = 0usize;
    let mut fixtures = collect_fixtures(&mut skipped);
    fixtures.extend(collect_meridian_fixtures(bless, &mut skipped));
    fixtures.sort_by(|a, b| a.stem.cmp(&b.stem));

    // A stem whose report was renamed keeps its baseline and silently loses its parameter values,
    // which renders it back down to its static headers — so say so by name. Skipped for a checkout
    // with no corpus at all, which the emptiness assertion below covers instead.
    for stem in PARAMETERIZED {
        assert!(
            fixtures.is_empty() || fixtures.iter().any(|f| f.stem == *stem),
            "{stem} has parameter values in this harness but is not in the corpus"
        );
    }

    let mut failures = Vec::new();
    for f in &fixtures {
        let ir = render_from_sql(&conn, &f.rpt, &f.sql, &f.stem);
        let label = format!("{} [page-ir]", f.stem);
        if let Some(d) = check(&label, &f.baseline_ir, &ir, bless) {
            failures.push(d);
        }
    }

    eprintln!(
        "postgres fixtures: {} Page IR baseline(s) {}, {skipped} skipped",
        fixtures.len(),
        if bless { "blessed" } else { "checked" }
    );
    // Asserted in BOTH modes, deliberately. A bless that matched no fixture writes no baseline and
    // would otherwise exit green, leaving an empty baseline tree that later reads as "covered".
    // A count, not a boolean: a corpus that shrank to a handful still renders "some" fixtures, and
    // the mismatch assertion below would then pass having compared almost nothing. The floor sits
    // under the committed baseline count, so only losing fixtures trips it.
    assert!(
        fixtures.len() >= 60,
        "only {} postgres fixture(s) ran, {skipped} skipped — the committed corpus is not being \
         found; is RPT_DB_URL pointing at a server with the seeded corpus?",
        fixtures.len()
    );
    if bless {
        return;
    }
    assert!(
        failures.is_empty(),
        "{} baseline mismatch(es):\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// The failure report itself is tested: a baseline too large to diff whole is the case where a reader
/// has nothing else to go on, so it must still distinguish one moved value from a moved document.
#[cfg(test)]
mod tests {
    use super::*;

    /// A serialization of `pages` pages, with `edit` applied to each page's body.
    fn doc(pages: usize, edit: impl Fn(usize, &mut String)) -> String {
        let mut out = String::from("// document\n{ \"sections\": {} }\n");
        for i in 1..=pages {
            let mut body =
                format!("{{\n  \"number\": {i},\n  \"ops\": [\"a\", \"b\", \"c\"]\n}}\n");
            edit(i, &mut body);
            out.push_str(&format!("// page {i}\n{body}"));
        }
        out
    }

    /// The stem list and the value table are two halves of one fact; a stem in neither or only one
    /// of them is a fixture rendering unparameterized without saying so.
    #[test]
    fn every_parameterized_stem_has_values() {
        for stem in PARAMETERIZED {
            assert!(!fixture_params(stem).is_empty(), "{stem}");
        }
        assert!(fixture_params("meridian/sales/02_product_catalog").is_empty());
    }

    #[test]
    fn blocks_split_at_markers() {
        let text = doc(2, |_, _| {});
        let blocks = blocks(&text);
        let labels: Vec<&str> = blocks.iter().map(|b| b.label).collect();
        assert_eq!(labels, ["document", "page 1", "page 2"]);
        assert!(blocks[1].body.contains("\"number\": 1"));
    }

    #[test]
    fn blocks_without_markers_stay_one_block() {
        let blocks = blocks("{ \"ops\": [] }\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].label, "whole file");
    }

    #[test]
    fn one_moved_value_names_its_page_and_diffs_it() {
        let base = doc(200, |_, _| {});
        let current = doc(200, |i, b| {
            if i == 7 {
                *b = b.replace("\"b\"", "\"B\"");
            }
        });
        let report = blockwise_diff("fixture", &base, &current);
        assert!(
            report.contains("1 of 201 block(s) differ: page 7"),
            "{report}"
        );
        // The one divergence is shown in full, not merely counted.
        assert!(
            report.contains("-  \"ops\": [\"a\", \"b\", \"c\"]"),
            "{report}"
        );
        assert!(
            report.contains("+  \"ops\": [\"a\", \"B\", \"c\"]"),
            "{report}"
        );
        assert!(!report.contains("more differing block(s)"), "{report}");
    }

    #[test]
    fn a_moved_document_reads_differently_from_a_moved_value() {
        let base = doc(200, |_, _| {});
        let current = doc(200, |_, b| *b = b.replace("\"b\"", "\"B\""));
        let report = blockwise_diff("fixture", &base, &current);
        assert!(
            report.contains("200 of 201 block(s) differ: page 1-page 200"),
            "{report}"
        );
        // Bounded: a sample is diffed, the rest is counted.
        assert!(
            report.contains("… and 197 more differing block(s)"),
            "{report}"
        );
    }

    #[test]
    fn a_page_count_change_is_called_out() {
        let report = blockwise_diff("fixture", &doc(5, |_, _| {}), &doc(3, |_, _| {}));
        assert!(report.contains("page count changed: 5 -> 3"), "{report}");
        assert!(report.contains("page 4-page 5"), "{report}");
        assert!(
            report.contains("page 4: present in baseline only"),
            "{report}"
        );
    }

    #[test]
    fn runs_compress_to_ranges() {
        let label = |i: usize| format!("page {i}");
        assert_eq!(runs(&[], &label), "none");
        assert_eq!(runs(&[3], &label), "page 3");
        assert_eq!(runs(&[1, 2, 3, 7], &label), "page 1-page 3, page 7");
    }

    #[test]
    fn run_list_is_truncated() {
        let every_other: Vec<usize> = (0..200).map(|i| i * 2).collect();
        let out = runs(&every_other, &|i| format!("page {i}"));
        assert!(out.ends_with("… (160 more run(s))"), "{out}");
    }
}
