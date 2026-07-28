//! `rpt-render` — the unified command-line renderer for Crystal Reports (`.rpt`).
//!
//! One entrypoint for the five inputs a render needs — the report, a datasource (embedded saved data
//! or a live database via `--db`), a locale, report parameters, and an output format/destination
//! (file or stdout) — with NORMAL/VERBOSE logging and a pre-fetch DB healthcheck.
//! Companion to the `rpt` inspection/export CLI.
//!
//! This module doc is the authoritative usage contract; the [`USAGE`] string mirrors it for `--help`,
//! and the per-module docs ([`datasource`], [`params`], [`locale`]) document the internals.
//!
//! ## Invocation
//! ```text
//! rpt-render <file.rpt> [OPTIONS]
//! ```
//!
//! ## Flags
//! - `--saved` / `--db` (mutually exclusive) — datasource selection; default (neither) is the
//!   report's saved data if present, else empty (only static bands render).
//! - `--list-sources` — print the report's live data sources and the exact env var to set for each,
//!   then exit without rendering.
//! - `-p`, `--param Name=Value` — a report parameter; repeatable, and repeating one name builds a
//!   multi-value parameter. Values are coerced to the declared type (see [`params`]).
//! - `--locale <tag>` — locale for date/number formatting (e.g. `en-US`, `de-DE`).
//! - `-f`, `--format <html|pdf|svg|png>` — output format; defaults to the `-o` extension, else HTML.
//! - `-o`, `--output <path>` — output file (`-` or omitted = stdout). SVG/PNG are one file per page,
//!   written as `<base>-N.svg`/`<base>-N.png`, so they need a real `-o` base (a single page may pipe).
//! - `--force` — for the multi-file SVG/PNG output, overwrite existing `<base>-N` pages and remove
//!   stale higher-numbered pages first. A single self-contained HTML/PDF file always overwrites.
//! - `-v`/`--verbose`, `-q`/`--quiet` (mutually exclusive), `-h`/`--help`.
//!
//! ## Datasource (`--db`) URL schemes
//! `--db` takes the connection URL(s) from the environment (never the command line, so no password
//! leaks into `ps`/history). The URL scheme selects the backend:
//!
//! - `postgres://` (or `postgresql://`) — implemented.
//! - `sqlite:///path/to/file.db` (or `sqlite::memory:`) — implemented.
//! - `mysql://`, `mariadb://`, `mssql://`/`sqlserver://` — recognized but not yet implemented.
//!
//! A single-server report reads `RPT_DB_URL`, falling back to `DATABASE_URL`. A report that spans
//! multiple servers (via subreports) needs one URL per distinct server in `RPT_DB_URL_<SERVER>`,
//! where `<SERVER>` is the server name upper-cased with non-alphanumerics turned to `_`. Run
//! `--list-sources` to print the exact variable names for a report. (See [`datasource`].)
//!
//! ## Locale resolution
//! Precedence: an explicit `--locale` overrides the host OS locale (`LC_ALL`/`LC_NUMERIC`/`LANG`),
//! which overrides the `en-US` fallback. (See [`locale`].)
//!
//! ## Output format selection
//! `-f`/`--format` wins; otherwise the format is inferred from the `-o` extension (`.pdf`/`.svg`/
//! `.png`, else HTML). HTML and PDF are a single self-contained document (safe to pipe to stdout);
//! SVG and PNG emit one file per page. PNG is a raster preview at 96 DPI.

mod applog;
mod datasource;
mod error;
mod locale;
mod output;
mod params;

use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use rpt_data::{EmptySource, RowSource, SavedDataSource};

use error::RenderError;

use applog::{Comp, Level, Log};

const USAGE: &str = "\
rpt-render — render a Crystal Reports (.rpt) file to HTML, PDF, SVG, or PNG

Opens the report, runs the data pipeline + layout engine, and writes the paginated result through
the chosen backend. Rows come from the report's embedded saved data (default) or a live database
(--db); text is laid out with real system-font metrics.

USAGE:
    rpt-render <file.rpt> [OPTIONS]

ARGS:
    <file.rpt>              the report to render

DATASOURCE (default: the report's saved data if present, else empty):
        --saved            use the report's embedded saved data
        --db               fetch rows live (main report + subreports) from the database URL(s) in
                           the environment (see DATABASE CONFIGURATION). The URL scheme selects the
                           backend.
        --list-sources     print the report's live data sources and the exact env var to set for
                           each, then exit (no render). Use this to discover what `--db` needs.

PARAMETERS:
    -p, --param Name=Value report parameter (see `rpt inputs <file>`). Repeatable; repeat the same
                           name for a multi-value parameter. Values are coerced to the declared type.

LOCALE:
        --locale <tag>     locale for date/number formatting (e.g. en-US, de-DE). Default: the host
                           locale (LC_ALL/LC_NUMERIC/LANG), else en-US.

OUTPUT:
    -f, --format <F>       html | pdf | svg | png. Default: inferred from -o's extension, else html.
                           png is a raster preview: one PNG per page (tiny-skia), 96 DPI.
    -o, --output <path>    output file; '-' or omitted writes to stdout. HTML and PDF are one
                           self-contained file (safe to pipe). For SVG and PNG (one file per page)
                           this is the base name: pages are written as <base>-1.svg / <base>-1.png,
                           <base>-2.svg / … — a multi-file output, so it needs a real -o path (a
                           single page may pipe).
        --force            for multi-file output (SVG/PNG pages), overwrite existing <base>-N.svg /
                           <base>-N.png pages, removing any stale higher-numbered pages from a
                           previous render first. A single self-contained file (HTML/PDF) always
                           overwrites and ignores this.

LOGGING:
    -v, --verbose          verbose: also log the SQL sent, timings, and push-down decisions
    -q, --quiet            quiet: errors only
    -h, --help             show this help and exit

EXAMPLES:
    # Render the report's saved data (format inferred from -o's extension)
    rpt-render report.rpt -o out.pdf

    # Pass parameters (repeat a name for a multi-value parameter); pipe HTML to stdout
    rpt-render report.rpt -p AsOfDate=2026-01-31 -p Region=West -p Region=East -f html > out.html

    # Render from a live database with -v to see the SQL sent and timings
    RPT_DB_URL='postgres://rpt:secret@db.internal:5432/sales' \\
        rpt-render report.rpt --db -o out.html -v

DATABASE CONFIGURATION (--db):
    The connection is a single URL taken from the environment — RPT_DB_URL (or DATABASE_URL). It is
    read from the environment, never the command line, so the password is not visible in `ps` or
    shell history; securing the environment itself is up to you. The URL SCHEME selects the backend:

        postgres://user:password@host:port/dbname     (or postgresql://)   [implemented]
        mysql://user:password@host:port/dbname                             [not yet]
        mariadb://user:password@host:port/dbname                           [not yet]
        sqlite:///path/to/file.db  (or sqlite::memory:)                    [implemented]
        mssql://user:password@host:port/dbname        (or sqlserver://)    [not yet]

    Examples (RPT_DB_URL takes precedence; DATABASE_URL is the fallback — use either):

        # inline for one run
        RPT_DB_URL='postgres://rpt:secret@db.internal:5432/sales' \\
            rpt-render report.rpt --db -o out.pdf

        # exported once (e.g. the 12-factor DATABASE_URL), reused across commands
        export DATABASE_URL='postgres://rpt:secret@db.internal:5432/sales'
        rpt-render report.rpt --db -o out.pdf

    postgres and sqlite are implemented; mysql, mariadb, and mssql are recognized (the scheme is
    understood) but not yet implemented.

    MULTIPLE CONNECTIONS:
        A report (with its subreports) can read from more than one server. Each distinct SERVER gets
        its own variable, RPT_DB_URL_<SERVER>, where <SERVER> is the server name upper-cased with
        non-alphanumerics turned to '_'. Run `--list-sources` to print the exact names for a report:

            $ rpt-render report.rpt --list-sources
            report.rpt: reads from 2 data source(s). Set a connection URL for each:
              sales-db/sales [ODBC (RDO)] (12 tables)
                export RPT_DB_URL_SALES_DB='postgres://user:pass@host:5432/dbname'
              hr-db/hr [ODBC (RDO)] (3 tables)
                export RPT_DB_URL_HR_DB='postgres://user:pass@host:5432/dbname'

        A single-server report also accepts the generic RPT_DB_URL / DATABASE_URL shown above.

    SECURITY:
        The connection URL — including any password — is read ONLY from the environment, never from
        a command-line flag, so it does not appear in `ps` output or shell history. rpt-render also
        redacts the password from all of its own log lines. Beyond that, protecting the environment
        is your responsibility: prefer a secrets manager or a root-owned env file over a plaintext
        `export` in a shared shell, and avoid printing the environment in CI logs.

ABOUT:
    Part of the rpt-rs project — a pure-Rust reader/renderer for the Crystal Reports (.rpt) format.
    Homepage:     https://github.com/MrSrsen/rpt-rs
    Report bugs:  https://github.com/MrSrsen/rpt-rs/issues
";

/// Output format.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Format {
    Html,
    Pdf,
    Svg,
    /// Raster preview: one PNG per page (rpt-render-raster / tiny-skia).
    Png,
}

impl Format {
    fn parse(s: &str) -> Result<Format, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "html" => Ok(Format::Html),
            "pdf" => Ok(Format::Pdf),
            "svg" => Ok(Format::Svg),
            "png" => Ok(Format::Png),
            other => Err(format!(
                "unknown --format {other:?} (expected html, pdf, svg, or png)"
            )),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Format::Html => "HTML",
            Format::Pdf => "PDF",
            Format::Svg => "SVG",
            Format::Png => "PNG",
        }
    }
}

/// Where output goes.
enum Dest {
    File(String),
    Stdout,
}

/// How to source rows.
enum DataMode {
    /// Neither flag: saved data if present, else empty.
    Auto,
    /// `--saved`.
    Saved,
    /// `--db`: live fetch from the database URL in the environment (scheme selects the driver).
    Db,
}

/// The parsed command line.
struct Cli {
    input: String,
    mode: DataMode,
    params: Vec<(String, String)>,
    locale: Option<String>,
    format: Option<Format>,
    output: Option<String>,
    level: Level,
    /// `--list-sources`: print the report's live data sources + the env var to set for each, then exit.
    list_sources: bool,
    /// `--force`: for multi-file output (SVG pages), overwrite a non-empty target and clean stale
    /// sibling pages first. A single self-contained file always overwrites silently regardless.
    force: bool,
}

fn main() -> ExitCode {
    rpt::install_panic_hook();
    let cli = match parse_args(std::env::args().skip(1)) {
        Ok(Some(cli)) => cli,
        Ok(None) => return ExitCode::SUCCESS, // --help
        Err(msg) => {
            eprintln!("rpt-render: {msg}\n");
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    let log = Log::new(cli.level);
    match run(&cli, &log) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log.error(error::full_chain(&err));
            // The actionable detail the driver offered — the failing statement, the `rpt sql` pointer —
            // printed under the message rather than crammed into it.
            if let Some(hint) = err.hint() {
                log.error(hint);
            }
            ExitCode::from(1)
        }
    }
}

/// Parse argv into a [`Cli`]. `Ok(None)` means `--help` was shown.
fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Cli>, String> {
    let mut input: Option<String> = None;
    let mut mode_saved = false;
    let mut mode_db = false;
    let mut list_sources = false;
    let mut params: Vec<(String, String)> = Vec::new();
    let mut locale: Option<String> = None;
    let mut format: Option<Format> = None;
    let mut output: Option<String> = None;
    let mut verbose = false;
    let mut quiet = false;
    let mut force = false;

    let mut args = args;
    while let Some(arg) = args.next() {
        let mut take = |flag: &str| -> Result<String, String> {
            args.next().ok_or_else(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "--saved" => mode_saved = true,
            "--db" => mode_db = true,
            "--list-sources" => list_sources = true,
            "-p" | "--param" => {
                let p = take("--param")?;
                match p.split_once('=') {
                    Some((n, v)) => params.push((n.to_string(), v.to_string())),
                    None => return Err(format!("--param must be Name=Value, got {p:?}")),
                }
            }
            "--locale" => locale = Some(take("--locale")?),
            "-f" | "--format" => format = Some(Format::parse(&take("--format")?)?),
            "-o" | "--output" => output = Some(take("--output")?),
            "-v" | "--verbose" => verbose = true,
            "-q" | "--quiet" => quiet = true,
            "--force" | "--overwrite" => force = true,
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option {other:?}"));
            }
            _ => {
                if input.replace(arg).is_some() {
                    return Err("expected exactly one <file.rpt>".to_string());
                }
            }
        }
    }

    let input = input.ok_or("missing <file.rpt>")?;
    if mode_saved && mode_db {
        return Err("--saved and --db are mutually exclusive".to_string());
    }
    if verbose && quiet {
        return Err("--verbose and --quiet are mutually exclusive".to_string());
    }
    let mode = match (mode_db, mode_saved) {
        (true, _) => DataMode::Db,
        (false, true) => DataMode::Saved,
        (false, false) => DataMode::Auto,
    };
    let level = if quiet {
        Level::Quiet
    } else if verbose {
        Level::Verbose
    } else {
        Level::Normal
    };

    Ok(Some(Cli {
        input,
        mode,
        params,
        locale,
        format,
        output,
        level,
        list_sources,
        force,
    }))
}

fn run(cli: &Cli, log: &Log) -> Result<(), RenderError> {
    let started = Instant::now();

    // Open + decode.
    let rpt = rpt::Rpt::open(&cli.input)?;
    let report = rpt.report();

    // An unrecognized record becomes a default rather than an error, so an incomplete decode would
    // otherwise render as a confident-looking page with content silently missing.
    if let Some(w) = rpt.decode_coverage().warning() {
        log.warn(Comp::Decode, w);
    }

    // `--list-sources`: print the report's live sources + the exact env var for each, then exit
    // (before any render-prep logging, so the listing stands alone).
    if cli.list_sources {
        return list_sources(&cli.input, report);
    }

    // Resolve output destination + format first, so we can report them up front.
    let (dest, format) = resolve_output(cli);
    log.info(
        Comp::Render,
        format!(
            "rendering {:?} → {} ({})",
            cli.input,
            match &dest {
                Dest::File(p) => p.as_str(),
                Dest::Stdout => "stdout",
            },
            format.name()
        ),
    );
    if !report.subreports.is_empty() {
        log.detail(
            Comp::Decode,
            format!("report has {} subreport(s)", report.subreports.len()),
        );
    }

    let render_locale = resolve_locale(cli, log);
    let parameters = report_parameters(report, cli, log)?;

    // Enumerate + log the report's data sources, then bind the main-scope row source (validating and
    // fetching live for `--db`). Bindings are owned by `rows` so `&dyn RowSource` can borrow from it.
    let rows = resolve_rows(cli, report, &parameters, log)?;
    let source: &dyn RowSource = rows.source.as_ref();

    // Capture the render's as-of instant once, so the record-selection pass and the layout pass share
    // a single fixed value for the date/time specials (`CurrentDate`/…) — the whole render is
    // deterministic for one invocation.
    let as_of = rpt_render::default_as_of();

    // Build the dataset (applies record selection, grouping) + attach parameters. Selection and
    // grouping formulas resolve `{?Param}` against the same values, so a parameter-filtered report
    // keeps the rows the parameters select.
    // The pipeline fails open: a selection formula that errors drops the row, a {@formula} that errors
    // resolves to Null. The sink is what makes those failures reachable — without it this command can
    // render zero rows from non-empty saved data and report success.
    let data_sink = rpt_data::CollectingSink::new();
    let mut dataset = rpt_data::build_dataset_opts(
        source,
        &report.data_definition,
        rpt_data::DatasetOptions {
            params: Some(&parameters),
            sink: Some(&data_sink),
            datetime: Some(as_of),
            ..Default::default()
        },
    );
    dataset.params = parameters;
    let selected_rows = dataset.iter_detail_rows().len();
    let data_diagnostics = rpt_render::data_diagnostics::from_evals(&data_sink.into_diagnostics());

    // With --db, subreports fetch their rows live too; otherwise they use saved data.
    #[cfg(feature = "db")]
    let live_scope;
    #[cfg(feature = "db")]
    let scope_data: Option<&dyn rpt_render::ScopeData> = match &rows.resolver {
        Some(r) => {
            live_scope = datasource::LiveScopeData {
                resolver: r,
                log,
                params: &dataset.params,
                report_label: &rows.report_label,
                cache: Default::default(),
            };
            Some(&live_scope)
        }
        None => None,
    };
    #[cfg(not(feature = "db"))]
    let scope_data: Option<&dyn rpt_render::ScopeData> = None;

    // The dataset is already built (its rows were fetched/selected above and its params attached), so
    // hand it to the one render entry point as a pre-built source; scope + locale ride along in the
    // options. The datasource itself already succeeded, so this render cannot fail.
    let doc = rpt_render::render_with(
        report,
        rpt_render::RenderOptions {
            datasource: rpt_render::RenderSource::Dataset(&dataset),
            locale: render_locale,
            scope: scope_data,
            as_of: Some(as_of),
            ..Default::default()
        },
    );

    // Surface every pipeline diagnostic — the data pipeline's (built above, so they are not in
    // `doc.diagnostics`) followed by layout/render's — into the CLI's channels and summary count.
    report_diagnostics(data_diagnostics.iter().chain(&doc.diagnostics), log);

    output::write_output(&dest, format, &doc, cli.force, log)?;

    // End-of-run summary (always, unless quiet): selected rows → pages, wall-clock, warning count.
    let warns = log.warning_count();
    log.info(
        Comp::Render,
        format!(
            "done: {selected_rows} row(s) → {} page(s) in {} ms{}",
            doc.pages.len(),
            started.elapsed().as_millis(),
            match warns {
                0 => String::new(),
                n => format!(" — {n} warning(s)"),
            }
        ),
    );
    Ok(())
}

/// Print pipeline diagnostics, collapsing repeats.
///
/// The same failure recurs per row — one broken selection formula over 600 rows is 600 identical
/// diagnostics — so identical ones are reported once with a count and the first occurrence's location.
/// Collapsing belongs here, in the presentation layer: the sink keeps every occurrence for a caller
/// that wants them, and a screen of identical lines would bury the summary that explains the problem.
fn report_diagnostics<'a>(diagnostics: impl Iterator<Item = &'a rpt_pages::Diagnostic>, log: &Log) {
    use std::collections::HashMap;

    type Key = (
        rpt_pages::Severity,
        rpt_pages::DiagnosticKind,
        String,
        Option<String>,
    );

    // Keyed on everything but the location, which is what varies between repeats. Insertion order is
    // tracked separately so output order still follows the pipeline.
    let mut order: Vec<Key> = Vec::new();
    let mut seen: HashMap<Key, (usize, String)> = HashMap::new();
    for d in diagnostics {
        let key = (d.severity, d.kind, d.message.clone(), d.source.clone());
        match seen.get_mut(&key) {
            Some((count, _)) => *count += 1,
            None => {
                seen.insert(key.clone(), (1, d.describe()));
                order.push(key);
            }
        }
    }
    for key in order {
        let (count, first) = &seen[&key];
        let msg = match count {
            1 => first.clone(),
            n => format!("{first} — and {} more like it", n - 1),
        };
        let comp = match key.1 {
            rpt_pages::DiagnosticKind::RecordSelection
            | rpt_pages::DiagnosticKind::GroupSelection
            | rpt_pages::DiagnosticKind::UnsupportedGroupCondition
            | rpt_pages::DiagnosticKind::TypeCoercion => Comp::Data,
            _ => Comp::Layout,
        };
        match key.0 {
            rpt_pages::Severity::Error => log.error_at(comp, msg),
            rpt_pages::Severity::Warning => log.warn(comp, msg),
        }
    }
}

/// Resolve the output destination and format. `-o` selects the destination — a path, or stdout for
/// `-`/omitted. `-f`/`--format` wins for the format; otherwise it is inferred from the destination
/// path's extension (`.pdf`/`.svg`/`.png`, else HTML), and stdout defaults to HTML.
fn resolve_output(cli: &Cli) -> (Dest, Format) {
    let dest = match cli.output.as_deref() {
        None | Some("-") => Dest::Stdout,
        Some(path) => Dest::File(path.to_string()),
    };
    let format = cli.format.unwrap_or_else(|| match &dest {
        Dest::File(p) => infer_format(p),
        Dest::Stdout => Format::Html,
    });
    (dest, format)
}

/// Resolve the effective render locale from `--locale` / the host environment (see [`locale`]),
/// logging the chosen tag and its source, and warning when the tag falls outside the built-in table
/// (so the render silently uses the en-US fallback for separators/month names).
fn resolve_locale(cli: &Cli, log: &Log) -> rpt_render::Locale {
    // Resolve the tag, then map it to a built-in render locale (separators + month/day names + AM/PM),
    // merged with each field's stored format at render time.
    let (loc_tag, loc_src) = locale::resolve(cli.locale.as_deref());
    let render_locale = rpt_render::Locale::from_tag(&loc_tag);
    log.info(
        Comp::Entry,
        format!(
            "locale: {loc_tag} (from {}) → formatting as {}",
            loc_src.label(),
            render_locale.tag
        ),
    );
    if rpt_render::Locale::lookup(&loc_tag).is_none() {
        log.warn(
            Comp::Entry,
            format!(
            "locale {loc_tag:?} is not in the built-in table (en-US, en-GB, de-DE, fr-FR, es-ES, \
             it-IT); formatting with the en-US fallback"
        ),
        );
    }
    render_locale
}

/// Coerce the `--param` pairs to the report's declared parameter types (see [`params`]), logging each
/// effective value. When the report declares parameters but none were supplied, warn with the list of
/// expected inputs (the render then uses each parameter's default).
fn report_parameters(
    report: &rpt::model::Report,
    cli: &Cli,
    log: &Log,
) -> Result<rpt_data::Parameters, RenderError> {
    let (parameters, resolved) = params::build(report, &cli.params, log)?;
    if resolved.is_empty() {
        let declared = params::declared(report);
        if declared.is_empty() {
            log.detail(Comp::Entry, "report declares no parameters");
        } else {
            log.warn(Comp::Entry, params::missing_values_warning(&declared));
        }
    } else {
        for p in &resolved {
            log.info(
                Comp::Entry,
                format!("param {}: {} = {}", p.name, p.type_name, p.display),
            );
        }
    }
    Ok(parameters)
}

/// The resolved main-scope row source, plus (on the `--db` path) the connection resolver and report
/// label that also feed each subreport scope its live rows via [`datasource::LiveScopeData`].
struct ResolvedRows {
    source: Box<dyn RowSource>,
    /// The live-connection resolver, `Some` only on the `--db` path; drives the subreport live fetches.
    #[cfg(feature = "db")]
    resolver: Option<datasource::Resolver>,
    /// The report's file name, tagged into every SQL sent to the DB (query-log provenance).
    #[cfg(feature = "db")]
    report_label: String,
}

/// Enumerate and log the report's data sources, then bind the main-scope row source. `--db` validates
/// every connection URL up front and fetches live rows (also returning the resolver so subreports
/// fetch live); `--saved`/auto use the report's embedded saved data, or an empty source when absent.
fn resolve_rows(
    cli: &Cli,
    report: &rpt::model::Report,
    parameters: &rpt_data::Parameters,
    log: &Log,
) -> Result<ResolvedRows, RenderError> {
    // `parameters` binds only into the `--db` WHERE push-down; unused on a DB-free build.
    #[cfg(not(feature = "db"))]
    let _ = parameters;

    // Always enumerate the report's data sources so the user sees what it reads from.
    let sources = datasource::enumerate(report);
    if sources.iter().any(|s| s.needs_credentials()) {
        log.info(
            Comp::Data,
            format!(
                "report uses {} data source(s):",
                sources.iter().filter(|s| s.needs_credentials()).count()
            ),
        );
        for s in sources.iter().filter(|s| s.needs_credentials()) {
            log.info(Comp::Data, format!("  - {}", s.describe()));
        }
    }

    // The report's file name, tagged into every SQL sent to the DB (query-log provenance).
    #[cfg(feature = "db")]
    let report_label: String = std::path::Path::new(&cli.input)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| cli.input.clone());

    match &cli.mode {
        DataMode::Db => {
            #[cfg(feature = "db")]
            {
                // Validate that every source has a connection URL BEFORE fetching or rendering.
                let resolver = datasource::Resolver::build(report)?;
                let selection = report
                    .data_definition
                    .record_selection
                    .as_ref()
                    .map(|f| f.0.as_str());
                // SQL Expression fields selected into the query; parameter values
                // bound into the pushed-down WHERE.
                let sql_exprs: Vec<(String, String)> = report
                    .data_definition
                    .sql_expression_fields()
                    .map(|(f, x)| (f.name.clone(), x.text.clone()))
                    .collect();
                let inputs = datasource::FetchInputs {
                    selection,
                    sql_exprs: &sql_exprs,
                    params: parameters,
                };
                let source = datasource::fetch_scope(
                    report,
                    &inputs,
                    &resolver,
                    log,
                    Some(&datasource::scope_comment(&report_label, "main")),
                )?;
                Ok(ResolvedRows {
                    source,
                    resolver: Some(resolver),
                    report_label,
                })
            }
            #[cfg(not(feature = "db"))]
            {
                Err(RenderError::Datasource(
                    "--db requested, but this build has no database drivers compiled in \
                            (rebuild with --features db-postgres and/or db-sqlite)"
                        .to_string(),
                ))
            }
        }
        DataMode::Saved | DataMode::Auto => match &report.saved_data {
            Some(sd) => {
                log.info(Comp::Data, "datasource: embedded saved data");
                Ok(ResolvedRows {
                    source: Box::new(SavedDataSource::from_report(sd, report)),
                    #[cfg(feature = "db")]
                    resolver: None,
                    #[cfg(feature = "db")]
                    report_label,
                })
            }
            None => {
                if matches!(cli.mode, DataMode::Saved) {
                    log.warn(
                        Comp::Data,
                        "--saved given, but the report has no saved data — rendering static \
                         bands only (no rows)",
                    );
                } else {
                    log.info(
                        Comp::Data,
                        "datasource: none — no database contacted and no SQL sent; the report \
                         has no saved data, so only static bands render. Pass --db to fetch \
                         live rows.",
                    );
                }
                Ok(ResolvedRows {
                    source: Box::new(EmptySource),
                    #[cfg(feature = "db")]
                    resolver: None,
                    #[cfg(feature = "db")]
                    report_label,
                })
            }
        },
    }
}

/// Print the report's live data sources and the exact environment variable to set for each (keyed by
/// server), so running `--db` is unambiguous. Writes to stdout (this is the command's output).
fn list_sources(input: &str, report: &rpt::model::Report) -> Result<(), RenderError> {
    let sources = datasource::credential_sources(&datasource::enumerate(report));
    if sources.is_empty() {
        println!("{input}: no live database sources (saved-data / field-definitions only).");
        return Ok(());
    }
    println!(
        "{input}: reads from {} data source(s). Set a connection URL for each in the environment:\n",
        sources.len()
    );
    for s in &sources {
        println!("  {}", s.describe());
        println!(
            "    export {}='postgres://user:pass@host:5432/dbname'",
            s.env_var()
        );
    }
    if sources.len() == 1 {
        println!("\n(a single-source report also accepts the generic RPT_DB_URL / DATABASE_URL.)");
    }
    println!("\nThen render with:  rpt-render {input} --db -o out.pdf");
    Ok(())
}

/// Infer the output format from a file extension, defaulting to HTML.
fn infer_format(output: &str) -> Format {
    match Path::new(output)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("pdf") => Format::Pdf,
        Some("svg") => Format::Svg,
        Some("png") => Format::Png,
        _ => Format::Html,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Cli {
        parse_args(args.iter().map(|s| s.to_string()))
            .expect("parse ok")
            .expect("not --help")
    }

    #[test]
    fn force_flag_parses_and_defaults_off() {
        assert!(!cli(&["r.rpt"]).force);
        assert!(cli(&["r.rpt", "--force"]).force);
        assert!(cli(&["r.rpt", "--overwrite"]).force);
    }

    /// `-o` selects the destination: a path is a file, `-`/omitted is stdout.
    #[test]
    fn resolve_output_destination() {
        assert!(matches!(resolve_output(&cli(&["r.rpt"])).0, Dest::Stdout));
        assert!(matches!(
            resolve_output(&cli(&["r.rpt", "-o", "-"])).0,
            Dest::Stdout
        ));
        match resolve_output(&cli(&["r.rpt", "-o", "out.pdf"])).0 {
            Dest::File(p) => assert_eq!(p, "out.pdf"),
            Dest::Stdout => panic!("expected a file destination"),
        }
    }

    /// Format inference: `-f` wins; otherwise the `-o` extension decides; a bare stdout is HTML.
    #[test]
    fn resolve_output_format_inference() {
        // No -f, no -o: stdout defaults to HTML.
        assert_eq!(resolve_output(&cli(&["r.rpt"])).1, Format::Html);
        // Inferred from the -o extension (case-insensitive), unknown/none → HTML.
        assert_eq!(
            resolve_output(&cli(&["r.rpt", "-o", "out.pdf"])).1,
            Format::Pdf
        );
        assert_eq!(
            resolve_output(&cli(&["r.rpt", "-o", "out.SVG"])).1,
            Format::Svg
        );
        assert_eq!(
            resolve_output(&cli(&["r.rpt", "-o", "out.png"])).1,
            Format::Png
        );
        assert_eq!(
            resolve_output(&cli(&["r.rpt", "-o", "out.txt"])).1,
            Format::Html
        );
        assert_eq!(
            resolve_output(&cli(&["r.rpt", "-o", "noext"])).1,
            Format::Html
        );
        // -f wins over the -o extension.
        assert_eq!(
            resolve_output(&cli(&["r.rpt", "-f", "png", "-o", "out.pdf"])).1,
            Format::Png
        );
        // -f wins even for a stdout destination.
        assert_eq!(resolve_output(&cli(&["r.rpt", "-f", "pdf"])).1, Format::Pdf);
    }

    /// Locale precedence through the extracted stage: an explicit `--locale` wins over the host
    /// environment, and its tag is normalized + mapped to the built-in render locale.
    #[test]
    fn resolve_locale_flag_wins_and_normalizes() {
        let log = Log::new(Level::Quiet);
        // `de_DE.UTF-8` normalizes to `de-DE` and maps to the de-DE built-in.
        assert_eq!(
            resolve_locale(&cli(&["r.rpt", "--locale", "de_DE.UTF-8"]), &log).tag,
            "de-DE"
        );
        // A tag outside the built-in table falls back to en-US formatting.
        assert_eq!(
            resolve_locale(&cli(&["r.rpt", "--locale", "xx-XX"]), &log).tag,
            "en-US"
        );
    }
}
