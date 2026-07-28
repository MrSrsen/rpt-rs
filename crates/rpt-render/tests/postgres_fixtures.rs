//! Data-driven HTML baseline regression over PostgreSQL fixtures.
//!
//! PostgreSQL is the single DB technology for the render-parity corpus: the SAME synthetic database is
//! read by (a) our renderer here and (b) the native Crystal engine out-of-band, so the only
//! variable under test is rendering, not the
//! datasource. For each fixture whose report is available, seed a committed `.sql` migration into a
//! PostgreSQL server, render the whole pipeline (decode → data → layout → Page IR → HTML) with the
//! **deterministic** [`ApproxLayout`](rpt_render::ApproxLayout) — no system fonts, so the committed
//! baseline is host-independent — and compare the HTML against the committed baseline.
//!
//! Because our datasource re-types every column against the report's declared field types (not the
//! DB's), the render is backend-independent: a given set of rows produces the same HTML regardless of
//! which engine served them.
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
//!   baseline is `tests/meridian/baselines/html/<relpath>.html`, where `<relpath>` is the report's path
//!   relative to `reports/` with `.rpt`→`.html`. The reports dirs may be empty while corpus authoring
//!   is in progress; an empty Meridian corpus contributes no fixtures and never panics.
//! - **Legacy** (`tests/fixtures/`) — the older group-shared corpus (parking/worrall + gitignored
//!   private samples), deprecated in favor of `tests/meridian/`. Its discovery (below) is kept working
//!   for as long as this corpus exists alongside Meridian.
//!
//! Legacy structure — fixtures are grouped one directory deep by report set (no filename prefix), and
//! each baseline mirrors that `<group>/<name>` path (like the model baseline harness):
//!   tests/fixtures/sql/<group>/<name>.sql                     — schema + SYNTHETIC seed (committed)
//!   tests/fixtures/baselines/html/<group>/<name>.html         — committed HTML baseline (blessed)
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

use rpt_test_support::workspace_root as repo_root;

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

/// Seed the `.sql` migration into PostgreSQL, render the report from it, and return the HTML. Uses the
/// dependency-free [`ApproxLayout`] so the render is byte-deterministic (host-independent).
fn render_from_sql(conn: &str, rpt_path: &Path, sql: &str, stem: &str) -> String {
    // The migration is idempotent (DROP/CREATE/INSERT), so re-seeding right before the render leaves
    // exactly this report's tables current, whatever other fixtures did to the shared server.
    rpt_db_postgres::seed(conn, sql).expect("seed postgres");

    let rpt = rpt::Rpt::open(rpt_path).expect("open report");
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
    let dataset = match rpt_db_postgres::PostgresSource::fetch_for_report(
        conn,
        report,
        None,
        &sql_exprs,
        &[],
    ) {
        Ok(src) => rpt_data::build_dataset(&src, &report.data_definition),
        Err(e) if e.to_string().contains("no database table") => {
            rpt_data::build_dataset(&EmptySource, &report.data_definition)
        }
        Err(e) => panic!("{stem}: fetch rows: {e}"),
    };
    let doc = rpt_render::render_dataset_with(
        report,
        &dataset,
        Box::new(rpt_render::ApproxLayout),
        rpt_render::Locale::default(),
        None,
        None,
    );
    // Inline the report's embedded images (the doc carries them), so the baseline is faithful.
    let html = rpt_render_html::render_pages_with_assets(&doc.pages, &doc.assets);

    assert!(!doc.pages.is_empty(), "{stem}: produced at least one page");
    html.replace("\r\n", "\n")
}

/// A git-style unified diff between the baseline and the current render.
fn unified_diff(name: &str, baseline: &str, current: &str) -> String {
    let body = similar::TextDiff::from_lines(baseline, current)
        .unified_diff()
        .context_radius(3)
        .header(&format!("{name} (baseline)"), &format!("{name} (current)"))
        .to_string();
    format!("{name}: render differs from baseline\n{body}")
}

/// One `(report stem, migration SQL, baseline path)` to check.
struct Fixture {
    stem: String,
    rpt: PathBuf,
    sql: String,
    baseline: PathBuf,
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
/// `tests/fixtures/baselines/html/<group>/<name>.html`. A group-shared seed (`<group>/<group>.sql`)
/// fans out to one fixture per report under `reports/<group>/`.
fn collect_fixtures(skipped: &mut usize) -> Vec<Fixture> {
    let root = repo_root();
    let baseline_root = root.join("tests/fixtures/baselines/html");
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
                    baseline: baseline_root.join(format!("{stem}.html")),
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
                baseline: baseline_root.join(format!("{rel}.html")),
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
/// `tests/meridian/baselines/html/<relpath>.html`. Contributes nothing when the seed or the reports are
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
    let baseline_root = root.join("baselines/html");
    let mut out = Vec::new();
    for (rel, rpt) in reports {
        let baseline = baseline_root.join(format!("{rel}.html"));
        if !bless && !baseline.exists() {
            eprintln!(
                "SKIP meridian/{rel}: baseline not blessed yet ({})",
                baseline.display()
            );
            *skipped += 1;
            continue;
        }
        out.push(Fixture {
            sql: sql.clone(),
            baseline,
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

    let mut failures = Vec::new();
    for f in &fixtures {
        let html = render_from_sql(&conn, &f.rpt, &f.sql, &f.stem);
        if bless {
            if let Some(dir) = f.baseline.parent() {
                std::fs::create_dir_all(dir).expect("create baselines dir");
            }
            std::fs::write(&f.baseline, &html).expect("write baseline");
            continue;
        }
        match std::fs::read_to_string(&f.baseline) {
            Ok(expected) => {
                let expected = expected.replace("\r\n", "\n");
                if expected != html {
                    failures.push(unified_diff(&f.stem, &expected, &html));
                }
            }
            Err(_) => failures.push(format!(
                "{}: missing baseline {} (run with RPT_BLESS=1)",
                f.stem,
                f.baseline.display()
            )),
        }
    }

    eprintln!(
        "postgres fixtures: {} {}, {skipped} skipped",
        fixtures.len(),
        if bless { "blessed" } else { "checked" }
    );
    if bless {
        return;
    }
    assert!(
        !fixtures.is_empty(),
        "no postgres fixtures ran (expected the public set to be present)"
    );
    assert!(
        failures.is_empty(),
        "{} baseline mismatch(es):\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
