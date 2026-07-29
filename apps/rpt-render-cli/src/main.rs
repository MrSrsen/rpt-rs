//! `rpt-render` — the unified command-line renderer for Crystal Reports (`.rpt`).
//!
//! One entrypoint for the five inputs a render needs — the report, a datasource (embedded saved data
//! or a live database via `--db`), a locale, report parameters, and an output destination (file or
//! stdout) — with NORMAL/VERBOSE logging and a pre-fetch DB healthcheck.
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
//! - `--list-fonts` — print the font library a render would use (face count, origins, searched
//!   directories, generic mappings) and exit; needs no report. `-v` lists every face with its path.
//! - `--list-sources` — print the report's live data sources and the exact env var to set for each,
//!   then exit without rendering.
//! - `-p`, `--param Name=Value` — a report parameter; repeatable, and repeating one name builds a
//!   multi-value parameter. Values are coerced to the declared type (see [`params`]).
//! - `--locale <tag>` — locale for date/number formatting (e.g. `en-US`, `de-DE`).
//! - `--system-fonts` — measure and embed the host's installed faces instead of the bundled ones (see
//!   [Fonts](#fonts)).
//! - `--pdfa <1b|2b|3b|1a|2a|3a>` — export against a PDF/A archival standard (see
//!   [Archival](#archival-pdfa)); the `a` levels additionally require tagging.
//! - `--pdfua` — export against PDF/UA-1, the accessibility standard (see [Tagging](#tagging)).
//! - `--tagged` — emit a structure tree without claiming any standard.
//! - `--lang <tag>` — the document's natural language (`en-US`); no report stores one.
//! - `--title <text>` — the document's title, overriding the report's own summary title.
//! - `--alt <Object>=<text>` — alternate text for a picture or chart; repeatable, and an empty text
//!   marks the graphic decorative.
//! - `-o`, `--output <path>` — output file (`-` or omitted = stdout).
//! - `-v`/`--verbose`, `-q`/`--quiet` (mutually exclusive), `-h`/`--help`, `-V`/`--version`.
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
//! ## Fonts
//! By default both halves of the font stack — the metrics layout measures with and the faces the PDF
//! embeds — come from the **bundled** Liberation/DejaVu set, so the same report and rows render to the
//! same bytes on every machine. `--system-fonts` switches both halves to the host's installed library,
//! which is what you want when the report's real faces are installed and fidelity matters more than
//! reproducibility. Only Arial, Times New Roman and Courier New are metric-compatible with the bundled
//! set, so for any other family the two modes lay text out differently.
//!
//! ## Archival (PDF/A)
//! `--pdfa 1b|2b|3b` exports against the corresponding ISO 19005 level-B standard. It is a **checked
//! claim**: the render fails, naming every unmet requirement, rather than writing a file that claims
//! conformance it does not honour — so a `--pdfa` run either produces a conforming document or produces
//! nothing and exits non-zero. `--pdfa 1a|2a|3a` adds the level-A accessibility requirements, which is
//! the same claim plus the tagging one below.
//!
//! The levels are not "newer is better". `1b` is PDF 1.4 and forbids transparency and 16-bit images
//! outright, so a report that legally uses either fails `--pdfa 1b` and passes `--pdfa 2b` — that is
//! the standards differing, not a defect. Conforming output also differs from ordinary output by
//! design: rasters embed uninterpolated (PDF/A forbids `/Interpolate true`), and the document carries
//! an output intent, XMP metadata and a creation date. The date is the Unix epoch, not the host clock,
//! so the same report still renders to the same bytes on every machine and every run.
//!
//! ## Tagging
//! `--tagged` adds a **structure tree** — what each mark means and in what order it is read — and
//! claims nothing. `--pdfua` (ISO 14289-1) and `--pdfa 1a|2a|3a` are the standards that require one,
//! and they are checked claims like the archival levels.
//!
//! A conforming tree needs three things a `.rpt` does not fully carry, so the render is **refused**,
//! naming each, rather than granting an accessibility claim it cannot support:
//!
//! - the document's **natural language** (`--lang en-US`) — no report stores one, and the render
//!   locale states number and date conventions rather than language;
//! - a **title** (PDF/UA-1 only) — taken from the report's summary information when the author filled
//!   it in, overridable with `--title`;
//! - **alternate text** for every picture and chart (`--alt <Object>=<text>`) — taken from the
//!   object's stored `ToolTipText` when it has one. `--alt <Object>=` with an empty text is the HTML
//!   `alt=""` convention: the graphic is decorative and is emitted as an artifact. The refusal names
//!   each undescribed figure, so a first run tells you exactly which `--alt` flags to add.
//!
//! ## Output
//! PDF is the only output format, so there is nothing to select and no `--format` flag: `-o` names a
//! path, `-` or omitting it writes to stdout. The document is a single self-contained file, safe to
//! pipe, and always overwrites. A path ending `.html`, `.svg` or `.png` is refused — those formats
//! existed in 0.3.0, so such a command was written against the old CLI and its extension says what the
//! caller expected.

// `db` is an umbrella over the driver-agnostic live-DB machinery, turned on BY each driver feature
// and never selectable alone: without a driver every `--db` render resolves its connections and then
// fails with "not available in this build", so the whole path is reachable only through a driver.
// Rejecting the combination at compile time is what lets the code gate that machinery on plain
// `feature = "db"` — under any buildable configuration it implies at least one driver.
#[cfg(all(
    feature = "db",
    not(any(feature = "db-postgres", feature = "db-sqlite"))
))]
compile_error!(
    "the `db` feature is the shared live-DB machinery, not a build on its own: enable a driver \
     (`db-postgres` and/or `db-sqlite`), which turns `db` on, or none of them for a DB-free build"
);

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

/// The `-V`/`--version` line. The version comes from `[workspace.package] version` in the root
/// manifest, which every crate inherits and Cargo exposes to the build as `CARGO_PKG_VERSION`, so
/// the binary reports the version it was compiled from and nothing has to be kept in step by hand.
const VERSION: &str = concat!("rpt-render ", env!("CARGO_PKG_VERSION"));

const USAGE: &str = concat!(
    "rpt-render ",
    env!("CARGO_PKG_VERSION"),
    " — render a Crystal Reports (.rpt) file to PDF

Opens the report, runs the data pipeline + layout engine, and writes the paginated result through
the chosen backend. Rows come from the report's embedded saved data (default) or a live database
(--db); text is laid out and embedded from the bundled faces, so the output is the same on every
machine (--system-fonts uses the host's installed faces instead).

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
        --list-fonts       print the font library a render would use — how many faces, where they
                           came from, which directories were searched, and what the sans-serif /
                           serif / monospace generics resolve to — then exit. Needs no <file.rpt>.
                           Add -v for a line per face with its path; add --system-fonts to list the
                           host's library instead of the bundled one.

PARAMETERS:
    -p, --param Name=Value report parameter (see `rpt inputs <file>`). Repeatable; repeat the same
                           name for a multi-value parameter. Values are coerced to the declared type.

LOCALE:
        --locale <tag>     locale for date/number formatting (e.g. en-US, de-DE). Default: the host
                           locale (LC_ALL/LC_NUMERIC/LANG), else en-US.

FONTS (default: the bundled Liberation/DejaVu faces, so the render is reproducible on any machine):
        --system-fonts     lay out and embed the host's installed faces instead. Use it when the
                           report's real fonts are installed and fidelity matters more than
                           reproducibility; the output then depends on this machine's font library.

ARCHIVAL (default: an ordinary PDF with no conformance claim):
        --pdfa <LEVEL>     export against the PDF/A standard of that part+level (ISO 19005-1/-2/-3).
                           The claim is CHECKED: if the document does not meet the level the render
                           fails, listing every unmet requirement, and writes nothing — a file
                           claiming conformance it does not honour is worse than no file.

                           The levels differ in what they FORBID, so newer is not simply better:
                             1b   PDF 1.4 — no transparency, no 16-bit images. Most portable, most
                                  restrictive; a report using either legitimately will fail here.
                             2b   PDF 1.7 — transparency and JPEG2000 allowed. The usual choice.
                             3b   as 2b, plus arbitrary file attachments (which this renderer never
                                  emits); pick it only if your archive mandates the -3 family.
                             1a / 2a / 3a
                                  the same, plus the level-A accessibility requirements — a tagged
                                  structure tree and the semantics under ACCESSIBILITY below.

                           So `--pdfa 1b` failing on a report that `--pdfa 2b` renders is the standards
                           differing, not a bug.

                           Conforming output differs from ordinary output by design — rasters embed
                           uninterpolated (PDF/A forbids interpolation), and the file carries an output
                           intent, XMP metadata and a creation date. That date is the Unix epoch rather
                           than the host clock, so the render stays byte-reproducible.

ACCESSIBILITY (default: no structure tree):
        --tagged           emit a structure tree — what each mark means and in what order it is read —
                           without claiming any standard. Makes the report readable by assistive
                           technology and copy-pasteable in reading order.
        --pdfua            export against PDF/UA-1 (ISO 14289-1), the accessibility standard. A
                           CHECKED claim, like --pdfa, and it implies --tagged.

                           A conforming tree needs three things the .rpt does not fully carry, so the
                           render is REFUSED — naming each — rather than claiming an accessibility it
                           cannot deliver:
        --lang <tag>       the document's natural language, e.g. en-US. REQUIRED by --pdfua and
                           --pdfa 1a|2a|3a: no report stores one, and --locale states number/date
                           conventions rather than language, so it cannot stand in.
        --title <text>     the document's title. PDF/UA-1 requires one and makes the viewer show it.
                           Defaults to the report's own summary title when the author filled it in
                           (many leave it empty); this flag overrides it.
        --alt <Obj>=<text> alternate text describing the picture or chart named <Obj>. Repeatable.
                           Defaults to the object's stored ToolTipText when it has one. An EMPTY text
                           (--alt Picture1=) is the HTML alt=\"\" convention: the graphic carries no
                           information and is emitted as an artifact instead of a figure.
                           The refusal names every undescribed figure, so one failed run tells you
                           exactly which flags to add.

OUTPUT:
    -o, --output <path>    output file; '-' or omitted writes to stdout. PDF is the only output
                           format, so there is no --format flag. The PDF is one self-contained file,
                           safe to pipe, and always overwrites. A .html/.svg/.png path is refused:
                           those formats were removed in 0.4.0.

LOGGING:
    -v, --verbose          verbose: also log the SQL sent, timings, and push-down decisions
    -q, --quiet            quiet: errors only
    -h, --help             show this help and exit
    -V, --version          show the version and exit

EXAMPLES:
    # Render the report's saved data to a PDF
    rpt-render report.rpt -o out.pdf

    # Pass parameters (repeat a name for a multi-value parameter); pipe the PDF to stdout
    rpt-render report.rpt -p AsOfDate=2026-01-31 -p Region=West -p Region=East > out.pdf

    # Render from a live database with -v to see the SQL sent and timings
    RPT_DB_URL='postgres://rpt:secret@db.internal:5432/sales' \\
        rpt-render report.rpt --db -o out.pdf -v

    # Use the host's installed faces (this machine's Arial, Calibri, …) instead of the bundled ones
    rpt-render report.rpt --system-fonts -o out.pdf

    # Export an archival PDF/A-2b; a document that does not meet the level fails instead of writing
    rpt-render report.rpt --pdfa 2b -o archive.pdf

    # Export an accessible PDF/UA-1; the language is not in the report, so it has to be stated
    rpt-render report.rpt --pdfua --lang en-US -o accessible.pdf

    # ...and a report with a chart needs that chart described (an empty text = decorative)
    rpt-render report.rpt --pdfua --lang en-US --alt 'Chart1=Sales by region, 2019-2024' -o out.pdf

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
"
);

/// Where output goes.
#[derive(Debug)]
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
    output: Option<String>,
    level: Level,

    /// `--list-sources`: print the report's live data sources + the env var to set for each, then exit.
    list_sources: bool,
    /// `--system-fonts`: take both halves of the font stack (layout metrics + embedded faces) from the
    /// host's installed library instead of the bundled faces.
    system_fonts: bool,
    /// `--pdfa <level>` / `--pdfua`: the archival or accessibility standard to export against,
    /// checked at serialization time. Default [`rpt_render::Conformance::None`] — an ordinary PDF.
    conformance: rpt_render::Conformance,
    /// `--tagged`: emit a structure tree while claiming no standard.
    tagged: bool,
    /// `--lang <tag>`: the document's natural language. Nothing else can supply it — no report
    /// stores one.
    language: Option<String>,
    /// `--title <text>`: the document's title, overriding the report's own summary title.
    title: Option<String>,
    /// `--alt <Object>=<text>`: alternate text per figure, in flag order. An empty text marks the
    /// graphic decorative.
    alt_text: Vec<(String, String)>,
}

impl Cli {
    /// The face library this run renders from — one value for both halves, so the metrics text is laid
    /// out to and the faces the PDF embeds can never disagree.
    fn fonts(&self) -> rpt_render::FontSource {
        if self.system_fonts {
            rpt_render::FontSource::System
        } else {
            rpt_render::FontSource::Bundled
        }
    }

    /// Whether this run builds a structure tree: asked for outright, or implied by a conformance
    /// level that cannot be claimed without one.
    fn tagged(&self) -> bool {
        self.tagged || self.conformance.requires_tagging()
    }

    /// The document semantics for this run: what the report itself states
    /// ([`rpt_render::semantics_of`]), with each explicitly passed flag layered on top.
    ///
    /// The report's own facts are read **only for a tagged run**. They are metadata the backend
    /// writes whenever they are set, so deriving a title for every render would move the bytes of
    /// every existing invocation; a flag the user typed is a different matter and always applies.
    fn semantics(&self, report: &rpt_reader::model::Report) -> rpt_render::Semantics {
        let mut semantics = match self.tagged() {
            true => rpt_render::semantics_of(report),
            false => rpt_render::Semantics::default(),
        };
        if let Some(title) = &self.title {
            semantics.title = Some(title.clone());
        }
        if let Some(language) = &self.language {
            semantics.language = Some(language.clone());
        }
        for (object, text) in &self.alt_text {
            semantics.alt_text.insert(object.clone(), text.clone());
        }
        semantics
    }

    /// The backend options this run writes its PDF with. A run that passes none of the output flags
    /// yields exactly [`rpt_render::PdfOptions::default()`], so the ordinary render's bytes are
    /// untouched by any of these features existing.
    ///
    /// `created` is left unset: the backend then writes no date for an ordinary render and the epoch
    /// for a conforming one (PDF/A requires a date), so neither reads the host clock and both stay
    /// reproducible.
    fn pdf_options(&self, semantics: rpt_render::Semantics) -> rpt_render::PdfOptions {
        rpt_render::PdfOptions {
            fonts: self.fonts(),
            conformance: self.conformance,
            created: None,
            producer: rpt_render::Producer::default(),
            tagged: self.tagged,
            semantics,
        }
    }
}

/// Parse a `--pdfa` level. The spelling is the PDF/A part+level (`1b`, `2a`, …), case-insensitive
/// and tolerant of the `pdf/a-` prefix people write out of habit.
fn parse_pdfa(value: &str) -> Result<rpt_render::Conformance, String> {
    let v = value.to_ascii_lowercase();
    match v.trim_start_matches("pdf/a-").trim_start_matches("pdfa-") {
        "1b" => Ok(rpt_render::Conformance::PdfA1b),
        "2b" => Ok(rpt_render::Conformance::PdfA2b),
        "3b" => Ok(rpt_render::Conformance::PdfA3b),
        "1a" => Ok(rpt_render::Conformance::PdfA1a),
        "2a" => Ok(rpt_render::Conformance::PdfA2a),
        "3a" => Ok(rpt_render::Conformance::PdfA3a),
        _ => Err(format!(
            "--pdfa takes 1b, 2b, 3b, 1a, 2a or 3a (the PDF/A part and level to export against), \
             got {value:?}. The 'a' levels additionally require a tagged structure tree, the \
             document's language (--lang) and alternate text for every figure (--alt). For PDF/UA-1, \
             pass --pdfua."
        )),
    }
}

/// What a tagged run is committed to, as one line: each semantic fact the structure tree carries and
/// where it came from. A refusal a moment later then reads against what was actually supplied.
fn describe_semantics(cli: &Cli, semantics: &rpt_render::Semantics) -> String {
    let title = match (&semantics.title, &cli.title) {
        (Some(t), Some(_)) => format!("title {t:?} (--title)"),
        (Some(t), None) => format!("title {t:?} (from the report)"),
        (None, _) => "no title (pass --title)".to_string(),
    };
    let language = match &semantics.language {
        Some(l) => format!("language {l:?} (--lang)"),
        None => "no language (pass --lang)".to_string(),
    };
    let from_flags: std::collections::BTreeSet<&str> =
        cli.alt_text.iter().map(|(o, _)| o.as_str()).collect();
    format!(
        "tagging: emitting a structure tree — {title}, {language}, {} figure description(s) ({} \
         from --alt, the rest from stored ToolTipText)",
        semantics.alt_text.len(),
        from_flags.len()
    )
}

/// Print the font library a render would actually use: how many faces, where they came from, which
/// directories were searched, and what the three generics resolve to.
///
/// Built from the same [`rpt_render::FontSource`] a render would build, so the listing cannot claim
/// something the renderer does not see. `verbose` adds a line per face with its path.
fn list_fonts_report(system_fonts: bool, verbose: bool) {
    let source = if system_fonts {
        rpt_render::FontSource::System
    } else {
        rpt_render::FontSource::Bundled
    };
    let inv = source.load().inventory();
    let (bundled, system) = (inv.bundled_count(), inv.system_count());

    println!(
        "fonts: {} available ({bundled} bundled, {system} system) — source: {}{}",
        inv.faces.len(),
        if system_fonts { "system" } else { "bundled" },
        if system_fonts {
            ""
        } else {
            " (default; pass --system-fonts for the host's own faces)"
        }
    );
    println!(
        "generics: sans-serif = {}, serif = {}, monospace = {}",
        inv.sans_serif, inv.serif, inv.monospace
    );

    // Printed whether or not a scan happened, and whether or not each exists: a directory that was
    // searched and found empty explains an absent font far better than its silent absence from a list.
    println!("\nsearched for system fonts:");
    if system_fonts {
        for dir in rpt_render::system_font_dirs() {
            let mark = if dir.is_dir() { "found " } else { "missing" };
            println!("  [{mark}] {}", dir.display());
        }
    } else {
        println!("  (none — the bundled faces are compiled in, so no directory is read)");
    }

    if !verbose {
        println!("\nrun with -v for the per-face listing");
        return;
    }
    println!("\nfaces:");
    for f in &inv.faces {
        let origin = match &f.path {
            Some(p) if f.index == 0 => p.display().to_string(),
            Some(p) => format!("{}#{}", p.display(), f.index),
            None => "<compiled in>".to_string(),
        };
        println!(
            "  {:<28} {:<8} w{:<4} {:<10}{} {}",
            f.family,
            f.style,
            f.weight,
            f.stretch,
            if f.monospaced { " mono" } else { "     " },
            origin
        );
    }
}

fn main() -> ExitCode {
    error::install_panic_hook();
    let cli = match parse_args(std::env::args().skip(1)) {
        Ok(Some(cli)) => cli,
        Ok(None) => return ExitCode::SUCCESS, // --help / --version
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

/// Parse argv into a [`Cli`]. `Ok(None)` means `--help` or `--version` was shown.
fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Cli>, String> {
    let mut input: Option<String> = None;
    let mut mode_saved = false;
    let mut mode_db = false;
    let mut list_sources = false;
    let mut list_fonts = false;
    let mut params: Vec<(String, String)> = Vec::new();
    let mut locale: Option<String> = None;
    let mut output: Option<String> = None;
    let mut verbose = false;
    let mut quiet = false;
    let mut system_fonts = false;
    let mut pdfa: Option<rpt_render::Conformance> = None;
    let mut pdfua = false;
    let mut tagged = false;
    let mut language: Option<String> = None;
    let mut title: Option<String> = None;
    let mut alt_text: Vec<(String, String)> = Vec::new();

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
            "-V" | "--version" => {
                println!("{VERSION}");
                return Ok(None);
            }
            "--saved" => mode_saved = true,
            "--db" => mode_db = true,
            "--list-sources" => list_sources = true,
            "--list-fonts" => list_fonts = true,
            "-p" | "--param" => {
                let p = take("--param")?;
                match p.split_once('=') {
                    Some((n, v)) => params.push((n.to_string(), v.to_string())),
                    None => return Err(format!("--param must be Name=Value, got {p:?}")),
                }
            }
            "--locale" => locale = Some(take("--locale")?),
            "--system-fonts" => system_fonts = true,
            "--pdfa" => pdfa = Some(parse_pdfa(&take("--pdfa")?)?),
            "--pdfua" => pdfua = true,
            "--tagged" => tagged = true,
            "--lang" => language = Some(take("--lang")?),
            "--title" => title = Some(take("--title")?),
            "--alt" => {
                let a = take("--alt")?;
                match a.split_once('=') {
                    // An empty text is meaningful — the alt="" convention — so it is kept, not dropped.
                    Some((object, text)) => alt_text.push((object.to_string(), text.to_string())),
                    None => {
                        return Err(format!(
                        "--alt must be Object=Text (an empty Text marks the graphic decorative), \
                             got {a:?}"
                    ))
                    }
                }
            }
            "-o" | "--output" => output = Some(take("--output")?),
            "-v" | "--verbose" => verbose = true,
            "-q" | "--quiet" => quiet = true,
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

    // `--list-fonts` answers "what faces does this machine offer", which is not a question about a
    // report — so it prints and exits before the input is required, unlike `--list-sources`.
    if list_fonts {
        list_fonts_report(system_fonts, verbose);
        return Ok(None);
    }

    let input = input.ok_or("missing <file.rpt>")?;
    if mode_saved && mode_db {
        return Err("--saved and --db are mutually exclusive".to_string());
    }
    if verbose && quiet {
        return Err("--verbose and --quiet are mutually exclusive".to_string());
    }
    // One document, one conformance claim: a file cannot be exported against two standards at once,
    // and silently honouring whichever was parsed last would write a claim the caller did not choose.
    let conformance = match (pdfua, pdfa) {
        (true, Some(level)) => {
            return Err(format!(
                "--pdfua and --pdfa {level} are mutually exclusive — a document is exported against \
                 one standard. PDF/A-1a/2a/3a already carry the level-A accessibility requirements."
            ))
        }
        (true, None) => rpt_render::Conformance::PdfUa1,
        (false, Some(level)) => level,
        (false, None) => rpt_render::Conformance::None,
    };
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
        output,
        level,
        list_sources,
        system_fonts,
        conformance,
        tagged,
        language,
        title,
        alt_text,
    }))
}

fn run(cli: &Cli, log: &Log) -> Result<(), RenderError> {
    let started = Instant::now();

    // Open + decode.
    let rpt = rpt_reader::Rpt::open(&cli.input)?;
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
    let dest = resolve_output(cli)?;
    log.info(
        Comp::Render,
        format!(
            "rendering {:?} → {} (PDF)",
            cli.input,
            match &dest {
                Dest::File(p) => p.as_str(),
                Dest::Stdout => "stdout",
            }
        ),
    );
    if !report.subreports.is_empty() {
        log.detail(
            Comp::Decode,
            format!("report has {} subreport(s)", report.subreports.len()),
        );
    }

    let render_locale = resolve_locale(cli, log);
    let fonts = cli.fonts();
    log.detail(
        Comp::Render,
        match fonts {
            rpt_render::FontSource::Bundled => {
                "fonts: the bundled faces — reproducible on any machine (--system-fonts uses the \
                 host's installed library)"
            }
            rpt_render::FontSource::System => {
                "fonts: the host's installed library (--system-fonts) — the output depends on this \
                 machine's faces"
            }
        },
    );
    // The archival claim is checked at serialization, so say up front what the render is committed to
    // — a later conformance failure then reads as this flag's consequence rather than a render bug.
    if cli.conformance != rpt_render::Conformance::None {
        log.info(
            Comp::Render,
            format!(
                "conformance: exporting against {} — the render fails if the document does not meet it",
                cli.conformance
            ),
        );
    }
    // The semantics are resolved here, from the report, rather than at write time: what the report
    // states and what the caller had to state are worth logging before the pipeline runs, so a later
    // refusal reads against a listing of what was actually supplied.
    let semantics = cli.semantics(report);
    if cli.tagged() {
        log.info(Comp::Render, describe_semantics(cli, &semantics));
    }
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
            fonts,
            ..Default::default()
        },
    );

    // Surface every pipeline diagnostic — the data pipeline's (built above, so they are not in
    // `doc.diagnostics`) followed by layout/render's — into the CLI's channels and summary count.
    report_diagnostics(data_diagnostics.iter().chain(&doc.diagnostics), log);

    output::write_output(&dest, &doc, &cli.pdf_options(semantics), log)?;

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

/// Resolve the output destination: a path, or stdout for `-`/omitted. There is no format to choose —
/// PDF is the only output — so the path is taken as given rather than inferred from.
///
/// One exception, and it is about migration rather than format selection: a `.html`, `.svg` or `.png`
/// path is refused. Those formats existed in 0.3.0, so such a path is a command written against the
/// old CLI, and writing a PDF into it would satisfy the letter of the request while ignoring what was
/// asked for.
fn resolve_output(cli: &Cli) -> Result<Dest, RenderError> {
    match cli.output.as_deref() {
        None | Some("-") => Ok(Dest::Stdout),
        Some(path) => {
            if let Some(ext) = Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .filter(|e| matches!(e.as_str(), "html" | "svg" | "png"))
            {
                return Err(RenderError::Output(format!(
                    "refusing to write {path:?}: the {} output format was removed in 0.4.0 and PDF is \
                     the only one left. Write to a .pdf path (or `-` for stdout).",
                    ext.to_ascii_uppercase()
                )));
            }
            Ok(Dest::File(path.to_string()))
        }
    }
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
    report: &rpt_reader::model::Report,
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
    report: &rpt_reader::model::Report,
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
fn list_sources(input: &str, report: &rpt_reader::model::Report) -> Result<(), RenderError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Cli {
        parse_args(args.iter().map(|s| s.to_string()))
            .expect("parse ok")
            .expect("not --help")
    }

    /// `--version` reports the version the binary was compiled from, and the `--help` header carries
    /// the same string — one source (`[workspace.package] version`), so a release tag can be checked
    /// against the manifest instead of against hand-written text.
    #[test]
    fn version_comes_from_the_manifest() {
        assert_eq!(VERSION, format!("rpt-render {}", env!("CARGO_PKG_VERSION")));
        assert!(
            USAGE.starts_with(VERSION),
            "{:?}",
            USAGE.lines().next().unwrap_or_default()
        );
        for flag in ["-V", "--version"] {
            let parsed = parse_args([flag.to_string()].into_iter()).expect("parse ok");
            assert!(parsed.is_none(), "{flag} must not produce a render");
        }
    }

    /// `-o` selects the destination: a path is a file, `-`/omitted is stdout. There is no format to
    /// resolve — PDF is the only output — so any path the user names is taken as given.
    #[test]
    fn resolve_output_destination() {
        let out = |args: &[&str]| resolve_output(&cli(args)).expect("resolves");
        assert!(matches!(out(&["r.rpt"]), Dest::Stdout));
        assert!(matches!(out(&["r.rpt", "-o", "-"]), Dest::Stdout));
        for path in ["out.pdf", "out.PDF", "noext", "report.dat"] {
            match out(&["r.rpt", "-o", path]) {
                Dest::File(p) => assert_eq!(p, path),
                Dest::Stdout => panic!("expected a file destination for {path}"),
            }
        }
    }

    /// A path named for one of the formats 0.3.0 could write is refused rather than quietly receiving
    /// PDF bytes: such a command was written against the old CLI, and the extension says what the
    /// caller expected.
    #[test]
    fn resolve_output_rejects_a_removed_format_extension() {
        for (path, named) in [("out.svg", "SVG"), ("out.png", "PNG"), ("out.html", "HTML")] {
            let err = resolve_output(&cli(&["r.rpt", "-o", path]))
                .expect_err("a removed format's extension must not silently receive PDF");
            let msg = err.to_string();
            assert!(
                msg.contains(path) && msg.contains(named) && msg.contains(".pdf"),
                "{msg}"
            );
        }
    }

    /// `--format` is gone with the choice it expressed; the parser must reject it rather than ignore it.
    #[test]
    fn the_format_flag_is_gone() {
        for args in [vec!["r.rpt", "-f", "pdf"], vec!["r.rpt", "--format", "pdf"]] {
            match parse_args(args.iter().map(|s| s.to_string())) {
                Err(err) => assert!(err.contains("-f") || err.contains("format"), "{err}"),
                Ok(_) => panic!("--format is no longer an option: {args:?} must not parse"),
            }
        }
    }

    /// The bundled faces are the default and `--system-fonts` is the opt-in, so a plain invocation
    /// renders reproducibly and the host library is reachable but never implicit.
    #[test]
    fn system_fonts_flag_is_the_opt_in() {
        assert_eq!(
            cli(&["r.rpt"]).fonts(),
            rpt_render::FontSource::Bundled,
            "the default must not read the host's font library"
        );
        assert_eq!(
            cli(&["r.rpt", "--system-fonts"]).fonts(),
            rpt_render::FontSource::System
        );
    }

    /// `--pdfa` takes the PDF/A part+level, case-insensitively and with the prefix people write out of
    /// habit, and maps it to the backend's conformance level.
    #[test]
    fn pdfa_flag_selects_the_level() {
        use rpt_render::Conformance;
        for (arg, level) in [
            ("1b", Conformance::PdfA1b),
            ("2b", Conformance::PdfA2b),
            ("3b", Conformance::PdfA3b),
            ("2B", Conformance::PdfA2b),
            ("PDF/A-3b", Conformance::PdfA3b),
        ] {
            assert_eq!(cli(&["r.rpt", "--pdfa", arg]).conformance, level, "{arg}");
        }
    }

    /// The accessible levels are reachable too, and each one turns tagging on — so `--pdfa 2a` is
    /// `2b` plus a structure tree rather than a differently-spelled `2b`.
    #[test]
    fn pdfa_selects_the_accessible_levels_and_they_imply_tagging() {
        use rpt_render::Conformance;
        for (arg, level) in [
            ("1a", Conformance::PdfA1a),
            ("2a", Conformance::PdfA2a),
            ("3a", Conformance::PdfA3a),
            ("PDF/A-2A", Conformance::PdfA2a),
        ] {
            let parsed = cli(&["r.rpt", "--pdfa", arg]);
            assert_eq!(parsed.conformance, level, "{arg}");
            assert!(parsed.tagged(), "{arg} cannot be claimed untagged");
        }
    }

    /// `--pdfua` is the accessibility standard proper, and it implies a structure tree.
    #[test]
    fn pdfua_selects_ua1_and_implies_tagging() {
        let parsed = cli(&["r.rpt", "--pdfua"]);
        assert_eq!(parsed.conformance, rpt_render::Conformance::PdfUa1);
        assert!(parsed.tagged());
        assert!(
            parsed.conformance.requires_title(),
            "PDF/UA-1 is the level that needs a title"
        );
    }

    /// A document is exported against one standard. Asking for two is rejected rather than resolved
    /// by argument order, which would write a claim the caller did not choose.
    #[test]
    fn pdfua_and_pdfa_are_mutually_exclusive() {
        for args in [
            vec!["r.rpt", "--pdfua", "--pdfa", "2b"],
            vec!["r.rpt", "--pdfa", "1a", "--pdfua"],
        ] {
            match parse_args(args.iter().map(|s| s.to_string())) {
                Err(err) => assert!(err.contains("--pdfua") && err.contains("--pdfa"), "{err}"),
                Ok(_) => panic!("two conformance claims must not parse: {args:?}"),
            }
        }
    }

    /// A level that is not a PDF/A part+level is rejected at parse time rather than accepted and
    /// quietly downgraded, and the message names the levels that do work.
    #[test]
    fn pdfa_rejects_a_level_it_cannot_honour() {
        for arg in ["4b", "yes", "ua1", ""] {
            let err = parse_pdfa(arg).expect_err("only the PDF/A levels are offered");
            assert!(
                ["1b", "2b", "3b", "1a", "2a", "3a"]
                    .iter()
                    .all(|l| err.contains(l)),
                "{err}"
            );
            assert!(
                err.contains("--pdfua"),
                "and PDF/UA has its own flag: {err}"
            );
        }
        // A missing value is an error, not a silently-defaulted level.
        match parse_args(["r.rpt", "--pdfa"].iter().map(|s| s.to_string())) {
            Err(err) => assert!(err.contains("--pdfa"), "{err}"),
            Ok(_) => panic!("--pdfa needs a value"),
        }
    }

    /// The ordinary render is untouched by conformance or tagging existing: with none of the output
    /// flags, the options handed to the backend are exactly its defaults, so the bytes are the ones
    /// the PDF baselines were blessed against.
    #[test]
    fn default_run_renders_with_the_backend_defaults() {
        let parsed = cli(&["r.rpt"]);
        assert_eq!(
            parsed.pdf_options(parsed.semantics(&rpt_reader::model::Report::default())),
            rpt_render::PdfOptions::default()
        );
        assert_eq!(parsed.conformance, rpt_render::Conformance::None);
        assert!(!parsed.tagged());
    }

    /// The flags compose, and neither run reads the host clock: `created` stays unset, so the backend
    /// writes no date for an ordinary render and the epoch for a conforming one.
    #[test]
    fn pdfa_composes_with_the_font_source_and_never_dates_from_the_clock() {
        let opts = cli(&["r.rpt", "--pdfa", "2b", "--system-fonts"])
            .pdf_options(rpt_render::Semantics::default());
        assert_eq!(opts.conformance, rpt_render::Conformance::PdfA2b);
        assert_eq!(opts.fonts, rpt_render::FontSource::System);
        assert_eq!(
            opts.created, None,
            "a render must not date itself from the host clock"
        );
    }

    /// `--tagged` is the opt-in for a structure tree with no conformance claim attached.
    #[test]
    fn tagged_claims_no_standard() {
        let parsed = cli(&["r.rpt", "--tagged"]);
        assert!(parsed.tagged());
        assert_eq!(parsed.conformance, rpt_render::Conformance::None);
        assert!(
            parsed.pdf_options(rpt_render::Semantics::default()).tagged,
            "the flag has to reach the backend on its own — no level implies it here"
        );
    }

    /// The report's own facts are read only for a tagged run, so an ordinary render's metadata (and
    /// therefore its bytes) does not move because a report happens to carry a summary title.
    #[test]
    fn report_semantics_are_read_only_for_a_tagged_run() {
        let report = titled_report();
        assert_eq!(
            cli(&["r.rpt"]).semantics(&report),
            rpt_render::Semantics::default(),
            "an untagged run must derive nothing"
        );
        let tagged = cli(&["r.rpt", "--tagged"]).semantics(&report);
        assert_eq!(tagged.title.as_deref(), Some("Quarterly Review"));
        assert_eq!(
            tagged.alt_text.get("Chart1").map(String::as_str),
            Some("Revenue by quarter")
        );
        assert_eq!(
            tagged.language, None,
            "no report states its language, so it is never derived"
        );
        assert_eq!(
            tagged.artifact_sections, None,
            "band classification comes from the document, not from here"
        );
    }

    /// What the caller typed wins over what the report states, and applies whether or not the run is
    /// tagged — an explicit flag is a request, not an inference.
    #[test]
    fn explicit_flags_override_the_report() {
        let report = titled_report();
        let parsed = cli(&[
            "r.rpt",
            "--pdfua",
            "--title",
            "Board pack",
            "--lang",
            "en-GB",
            "--alt",
            "Chart1=Revenue, four quarters",
        ]);
        let semantics = parsed.semantics(&report);
        assert_eq!(semantics.title.as_deref(), Some("Board pack"));
        assert_eq!(semantics.language.as_deref(), Some("en-GB"));
        assert_eq!(
            semantics.alt_text.get("Chart1").map(String::as_str),
            Some("Revenue, four quarters")
        );

        // Untagged, the same flags still apply — and nothing else appears beside them.
        let plain = cli(&["r.rpt", "--lang", "en-GB"]).semantics(&report);
        assert_eq!(plain.language.as_deref(), Some("en-GB"));
        assert_eq!(plain.title, None);
    }

    /// The CLI is a caller of the facade and nothing more: what it hands the backend is
    /// `semantics_of` plus the flags, so the command line and a library call agree by construction.
    #[test]
    fn cli_semantics_are_the_facade_plus_the_flags() {
        let report = titled_report();
        let expected = rpt_render::Semantics {
            language: Some("en-US".to_string()),
            ..rpt_render::semantics_of(&report)
        };
        assert_eq!(
            cli(&["r.rpt", "--pdfua", "--lang", "en-US"]).semantics(&report),
            expected
        );
    }

    /// `--alt Object=` with an empty text is the `alt=""` convention and must survive parsing: it is
    /// the caller saying the graphic is decorative, which is a different answer from saying nothing.
    #[test]
    fn alt_keeps_an_empty_description() {
        let parsed = cli(&["r.rpt", "--tagged", "--alt", "Picture1="]);
        assert_eq!(parsed.alt_text, vec![("Picture1".into(), String::new())]);
        assert_eq!(
            parsed
                .semantics(&titled_report())
                .alt_text
                .get("Picture1")
                .map(String::as_str),
            Some("")
        );
        // A value with no `=` is a mistake, not an empty description.
        match parse_args(["r.rpt", "--alt", "Picture1"].iter().map(|s| s.to_string())) {
            Err(err) => assert!(err.contains("--alt"), "{err}"),
            Ok(_) => panic!("--alt must be Object=Text"),
        }
    }

    /// A report stating a summary title and describing its chart through the stored `ToolTipText` —
    /// the two facts the facade derives.
    fn titled_report() -> rpt_reader::model::Report {
        use rpt_reader::model::{Area, ReportObject, ReportObjectKind, Section, SummaryInfo};
        let chart = ReportObject {
            name: "Chart1".to_string(),
            kind: ReportObjectKind::Chart(Box::default()),
            format: rpt_reader::model::ObjectFormat {
                tooltip_text: Some("Revenue by quarter".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut report = rpt_reader::model::Report {
            summary_info: SummaryInfo {
                title: "Quarterly Review".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        report.report_definition.areas.push(Area {
            sections: vec![Section {
                objects: vec![chart],
                ..Default::default()
            }],
            ..Default::default()
        });
        report
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
