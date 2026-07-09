//! `rpt-render` — the unified command-line renderer for Crystal Reports (`.rpt`).
//!
//! One entrypoint for the five inputs a render needs — the report, a datasource (embedded saved data
//! or a live database via `--db`), a locale, report parameters, and an output format/destination
//! (file or stdout) — with NORMAL/VERBOSE logging and a pre-fetch DB healthcheck.
//! Companion to `rpt xml-dump` (XML export) and `rpt` (inspection).
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
mod locale;
mod params;

use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use rpt_data::{EmptySource, RowSource, SavedDataSource};
use rpt_render::RenderError;

use applog::{Comp, Level, Log};

const USAGE: &str = "\
rpt-render — render a Crystal Reports (.rpt) file to HTML, PDF, SVG, or PNG

Opens the report, runs the data pipeline + layout engine, and writes the paginated result through
the chosen backend. Rows come from the report's embedded saved data (default) or a live database
(--db); text is laid out with real system-font metrics. No Crystal Reports runtime required.

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
#[derive(Clone, Copy, PartialEq, Eq)]
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
            log.error(err.to_string());
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

    // `--list-sources`: print the report's live sources + the exact env var for each, then exit
    // (before any render-prep logging, so the listing stands alone).
    if cli.list_sources {
        return list_sources(&cli.input, report);
    }

    // Resolve output format + destination first, so we can report them up front.
    let dest = match cli.output.as_deref() {
        None | Some("-") => Dest::Stdout,
        Some(path) => Dest::File(path.to_string()),
    };
    let format = cli.format.unwrap_or_else(|| match &dest {
        Dest::File(p) => infer_format(p),
        Dest::Stdout => Format::Html,
    });

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

    // Locale: resolve the tag, then map it to a built-in render locale (separators + month/day
    // names + AM/PM), merged with each field's stored format at render time.
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

    // Parameters (coerce + report; warns on undeclared names).
    let (parameters, resolved) = params::build(report, &cli.params, log)?;
    if resolved.is_empty() {
        let declared = params::declared(report);
        if declared.is_empty() {
            log.detail(Comp::Entry, "report declares no parameters");
        } else {
            // The report expects inputs but none were given — warn and list what's expected
            // (rendered with each parameter's default). One multi-line warning: the logger indents
            // the continuation lines under the message column.
            let mut msg = format!(
                "no parameters supplied; the report declares {} — rendering with defaults \
                 (set with -p Name=Value):",
                declared.len()
            );
            for d in &declared {
                let mut flags = Vec::new();
                if d.optional {
                    flags.push("optional");
                }
                if d.multi {
                    flags.push("multi-valued");
                }
                let suffix = if flags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", flags.join(", "))
                };
                msg.push_str(&format!("\n  - {} : {}{suffix}", d.name, d.type_name));
            }
            log.warn(Comp::Entry, msg);
        }
    } else {
        for p in &resolved {
            log.info(
                Comp::Entry,
                format!("param {}: {} = {}", p.name, p.type_name, p.display),
            );
        }
    }

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

    // Resolve rows. Bindings are pre-declared so the `&dyn RowSource` can borrow whichever holder.
    let saved_holder: SavedDataSource;
    let empty = EmptySource;
    #[cfg(feature = "db")]
    let db_holder: Box<dyn RowSource>;
    // For --db, the resolver (validated up front) also feeds live rows to each subreport scope.
    #[cfg(feature = "db")]
    let mut resolver_opt: Option<datasource::Resolver> = None;

    let source: &dyn RowSource = match &cli.mode {
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
                db_holder = datasource::fetch_scope(
                    report,
                    selection,
                    &sql_exprs,
                    &parameters,
                    &resolver,
                    log,
                )?;
                resolver_opt = Some(resolver);
                db_holder.as_ref()
            }
            #[cfg(not(feature = "db"))]
            {
                return Err(RenderError::Datasource(
                    "--db requested, but this build has no database drivers compiled in \
                            (rebuild with --features db-postgres and/or db-sqlite)"
                        .to_string(),
                ));
            }
        }
        DataMode::Saved | DataMode::Auto => match &report.saved_data {
            Some(sd) => {
                log.info(Comp::Data, "datasource: embedded saved data");
                saved_holder = SavedDataSource::from_report(sd, report);
                &saved_holder
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
                &empty
            }
        },
    };

    // Build the dataset (applies record selection, grouping) + attach parameters.
    let mut dataset = rpt_data::build_dataset(source, &report.data_definition);
    dataset.params = parameters;
    let selected_rows = dataset.iter_detail_rows().len();

    // With --db, subreports fetch their rows live too; otherwise they use saved data.
    #[cfg(feature = "db")]
    let live_scope;
    #[cfg(feature = "db")]
    let scope_data: Option<&dyn rpt_render::ScopeData> = match &resolver_opt {
        Some(r) => {
            live_scope = datasource::LiveScopeData {
                resolver: r,
                log,
                params: &dataset.params,
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
            ..Default::default()
        },
    )?;

    // Surface the render-side fidelity diagnostics (unsupported objects, formula errors) collected
    // deep in the pipeline into the CLI's warning channel + summary count.
    for d in &doc.diagnostics {
        match &d.source {
            Some(s) => log.warn(Comp::Layout, format!("{} ({s})", d.message)),
            None => log.warn(Comp::Layout, d.message.clone()),
        }
    }

    write_output(&dest, format, &doc, cli.force, log)?;

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

/// Write the paginated document to the destination in the chosen format.
fn write_output(
    dest: &Dest,
    format: Format,
    doc: &rpt_pages::PagedDocument,
    force: bool,
    log: &Log,
) -> Result<(), RenderError> {
    match format {
        Format::Html => {
            let html =
                rpt_render::render_backend(doc, &rpt_render::HtmlBackend, &rpt_render::HtmlOptions);
            write_bytes(dest, html.as_bytes(), format, log)
        }
        Format::Pdf => {
            let pdf = rpt_render::render_backend(
                doc,
                &rpt_render::PdfBackend,
                &rpt_render::PdfOptions::default(),
            );
            write_bytes(dest, &pdf, format, log)
        }
        Format::Svg => write_svg(dest, doc, force, log),
        Format::Png => write_png(dest, doc, force, log),
    }
}

/// Write bytes to a file or stdout, guarding against dumping binary to a terminal.
fn write_bytes(dest: &Dest, bytes: &[u8], format: Format, log: &Log) -> Result<(), RenderError> {
    match dest {
        Dest::File(path) => {
            std::fs::write(path, bytes)
                .map_err(|e| RenderError::Io(format!("cannot write {path:?}: {e}")))?;
            log.info(
                Comp::Render,
                format!("wrote {path} ({}, {} bytes)", format.name(), bytes.len()),
            );
            Ok(())
        }
        Dest::Stdout => {
            if format == Format::Pdf && std::io::stdout().is_terminal() {
                return Err(RenderError::Io(
                    "refusing to write binary PDF to a terminal; redirect to a file or use -o <path>"
                        .to_string(),
                ));
            }
            std::io::stdout()
                .write_all(bytes)
                .map_err(|e| RenderError::Io(format!("cannot write to stdout: {e}")))
        }
    }
}

/// SVG is one file per page — a multi-file output. To stdout it only works for a single-page report
/// (you cannot pipe multiple files); otherwise it needs a base name via `-o`. Because a shorter
/// render would otherwise leave the previous run's higher-numbered pages behind, this refuses to
/// write over a base that already has `<base>-N.svg` pages unless `--force`, and with `--force` it
/// deletes the stale siblings first so the directory reflects exactly this render.
fn write_svg(
    dest: &Dest,
    doc: &rpt_pages::PagedDocument,
    force: bool,
    log: &Log,
) -> Result<(), RenderError> {
    match dest {
        Dest::Stdout => match doc.pages.as_slice() {
            [page] => std::io::stdout()
                .write_all(rpt_render_svg::render_page(page).as_bytes())
                .map_err(|e| RenderError::Io(format!("cannot write to stdout: {e}"))),
            pages => Err(RenderError::Io(format!(
                "SVG is one file per page ({} pages) and multiple files cannot be piped; \
                 specify -o <base> to write <base>-N.svg",
                pages.len()
            ))),
        },
        Dest::File(path) => {
            let base = path.strip_suffix(".svg").unwrap_or(path);
            let stale = existing_svg_pages(base);
            if !stale.is_empty() && !force {
                return Err(RenderError::Io(format!(
                    "{} existing {base}-N.svg page(s) would be overwritten; pass --force to \
                     replace them (stale higher-numbered pages are removed first)",
                    stale.len()
                )));
            }
            for name in &stale {
                let _ = std::fs::remove_file(name);
            }
            for (i, page) in doc.pages.iter().enumerate() {
                let name = format!("{base}-{}.svg", i + 1);
                std::fs::write(&name, rpt_render_svg::render_page(page))
                    .map_err(|e| RenderError::Io(format!("cannot write {name:?}: {e}")))?;
            }
            log.info(
                Comp::Render,
                format!(
                    "wrote {} SVG page(s) as {base}-N.svg{}",
                    doc.pages.len(),
                    if stale.is_empty() {
                        String::new()
                    } else {
                        format!(" (replaced {} stale page file(s))", stale.len())
                    }
                ),
            );
            Ok(())
        }
    }
}

/// SVG's `<base>-N.svg` sibling files (thin wrapper over [`existing_numbered_pages`]).
fn existing_svg_pages(base: &str) -> Vec<String> {
    existing_numbered_pages(base, "svg")
}

/// PNG's `<base>-N.png` sibling files (raster preview is one file per page, like SVG).
fn write_png(
    dest: &Dest,
    doc: &rpt_pages::PagedDocument,
    force: bool,
    log: &Log,
) -> Result<(), RenderError> {
    match dest {
        Dest::Stdout => match doc.pages.as_slice() {
            [page] => {
                if std::io::stdout().is_terminal() {
                    return Err(RenderError::Io(
                        "refusing to write binary PNG to a terminal; redirect to a file \
                                or use -o <path>"
                            .to_string(),
                    ));
                }
                std::io::stdout()
                    .write_all(&rpt_render_raster::render_page(page))
                    .map_err(|e| RenderError::Io(format!("cannot write to stdout: {e}")))
            }
            pages => Err(RenderError::Io(format!(
                "PNG is one file per page ({} pages) and multiple files cannot be piped; \
                 specify -o <base> to write <base>-N.png",
                pages.len()
            ))),
        },
        Dest::File(path) => {
            let base = path.strip_suffix(".png").unwrap_or(path);
            let stale = existing_numbered_pages(base, "png");
            if !stale.is_empty() && !force {
                return Err(RenderError::Io(format!(
                    "{} existing {base}-N.png page(s) would be overwritten; pass --force to \
                     replace them (stale higher-numbered pages are removed first)",
                    stale.len()
                )));
            }
            for name in &stale {
                let _ = std::fs::remove_file(name);
            }
            for (i, page) in doc.pages.iter().enumerate() {
                let name = format!("{base}-{}.png", i + 1);
                std::fs::write(&name, rpt_render_raster::render_page(page))
                    .map_err(|e| RenderError::Io(format!("cannot write {name:?}: {e}")))?;
            }
            log.info(
                Comp::Render,
                format!(
                    "wrote {} PNG page(s) as {base}-N.png{}",
                    doc.pages.len(),
                    if stale.is_empty() {
                        String::new()
                    } else {
                        format!(" (replaced {} stale page file(s))", stale.len())
                    }
                ),
            );
            Ok(())
        }
    }
}

/// The existing `<base>-N.<ext>` sibling files for a given base name (page number `N` ≥ 1), so a
/// re-render can detect and clean a prior run's pages. Returns full paths; empty if none/unreadable.
fn existing_numbered_pages(base: &str, ext: &str) -> Vec<String> {
    let path = Path::new(base);
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let prefix = format!(
        "{}-",
        path.file_name().and_then(|n| n.to_str()).unwrap_or(base)
    );
    let suffix = format!(".{ext}");
    let read = match std::fs::read_dir(dir.unwrap_or_else(|| Path::new("."))) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let fname = entry.file_name();
        let Some(name) = fname.to_str() else { continue };
        if let Some(mid) = name
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix(&suffix))
        {
            if !mid.is_empty() && mid.bytes().all(|b| b.is_ascii_digit()) {
                out.push(entry.path().to_string_lossy().into_owned());
            }
        }
    }
    out
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
    use std::sync::atomic::{AtomicU32, Ordering};

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

    #[test]
    fn existing_svg_pages_matches_only_numbered_siblings() {
        // Unique scratch dir per test run (no time/rand needed).
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rpt-render-svgtest-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("pg");
        let base = base.to_str().unwrap();

        // Matches: pg-1.svg, pg-12.svg. Non-matches: different stem, non-numeric, wrong ext.
        for f in [
            "pg-1.svg",
            "pg-12.svg",
            "pg-x.svg",
            "pgg-3.svg",
            "pg-2.txt",
            "pg.svg",
        ] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let mut found: Vec<String> = existing_svg_pages(base)
            .into_iter()
            .map(|p| {
                Path::new(&p)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        found.sort();
        assert_eq!(found, vec!["pg-1.svg".to_string(), "pg-12.svg".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn existing_svg_pages_empty_when_none() {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rpt-render-svgtest-empty-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("pg");
        assert!(existing_svg_pages(base.to_str().unwrap()).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
