//! Structured leveled logger for the `rpt-render` CLI.
//!
//! Every line is `LEVEL  component  message` on **stderr** (stdout stays reserved for the
//! rendered bytes when `-o -`). The level and component are fixed-width columns so a run reads
//! as an aligned table you can scan by column:
//!
//! ```text
//! INFO   render  rendering "Face_Sheet.rpt" → stdout (HTML)
//! INFO   decode  report has 6 subreport(s)
//! WARN   layout  table "Command" binds a raw SQL command …
//! INFO   data    datasource: none — no database contacted, no SQL sent; static bands only
//! DEBUG  data    SELECT … (push-down detail, -v only)
//! ```
//!
//! - **Levels** map to visibility: `ERROR` always; `WARN`/`INFO` at NORMAL+; `DEBUG` (the
//!   mechanical detail — SQL, timings, push-down) only at VERBOSE (`-v`). `-q` prints errors
//!   only (warnings are still counted for the summary).
//! - **Components** name the pipeline stage the line comes from ([`Comp`]).
//! - Colour is applied only when stderr is a TTY and `NO_COLOR` is unset.
//!
//! Warnings feed the **fidelity channel**: each [`warn`](Log::warn) is recorded so the run can
//! end with a one-line summary count ("rendered with N warnings").

use std::cell::RefCell;
use std::io::IsTerminal;

/// How much the CLI prints to stderr.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    /// `-q`: errors only (warnings are still counted for the summary, just not printed).
    Quiet,
    /// default: the operational narrative (report, params, datasource, healthcheck, summary).
    Normal,
    /// `-v`: everything, plus the SQL sent, per-stage timings, and push-down decisions.
    Verbose,
}

/// The pipeline stage a log line belongs to — the `component` column. Assigning every line a
/// stage makes it clear *which part of the app* is speaking (and, for warnings, where to look).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comp {
    /// CLI entrypoint: argument parsing, locale/parameter resolution, orchestration, fatal errors.
    Entry,
    /// Reading & decoding the `.rpt` container (streams, records, subreports).
    Decode,
    /// Data sourcing: datasource selection, SQL generation/push-down, row fetch.
    Data,
    /// Layout & pagination, including pipeline fidelity diagnostics (unsupported objects,
    /// raw-SQL `Command` tables).
    Layout,
    /// Rendering to a backend and writing the output.
    Render,
}

impl Comp {
    /// The lowercase tag printed in the component column.
    fn tag(self) -> &'static str {
        match self {
            Comp::Entry => "entry",
            Comp::Decode => "decode",
            Comp::Data => "data",
            Comp::Layout => "layout",
            Comp::Render => "render",
        }
    }
}

/// One line's severity — drives the `LEVEL` column and its colour.
#[derive(Clone, Copy)]
enum Sev {
    Error,
    Warn,
    Info,
    Debug,
}

impl Sev {
    fn label(self) -> &'static str {
        match self {
            Sev::Error => "ERROR",
            Sev::Warn => "WARN",
            Sev::Info => "INFO",
            Sev::Debug => "DEBUG",
        }
    }

    /// SGR colour code for the level label (used only when colour is enabled).
    fn color(self) -> &'static str {
        match self {
            Sev::Error => "1;31", // bold red
            Sev::Warn => "33",    // yellow
            Sev::Info => "32",    // green
            Sev::Debug => "90",   // bright black (dim)
        }
    }
}

/// A stderr logger with a level, a component-tagged aligned format, and a running warning tally.
pub struct Log {
    level: Level,
    color: bool,
    warnings: RefCell<Vec<String>>,
}

impl Log {
    pub fn new(level: Level) -> Log {
        // Colour only when stderr is a real terminal and the user hasn't opted out via NO_COLOR.
        let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        Log {
            level,
            color,
            warnings: RefCell::new(Vec::new()),
        }
    }

    /// Is verbose (`-v`) detail being printed? Guard expensive-to-format `DEBUG` lines with this.
    // Only used on the live-DB path (verbose SQL logging); dead in a DB-free build.
    #[cfg_attr(not(feature = "db-postgres"), allow(dead_code))]
    pub fn is_verbose(&self) -> bool {
        self.level >= Level::Verbose
    }

    /// Operational narrative — shown at NORMAL and VERBOSE.
    pub fn info(&self, comp: Comp, msg: impl AsRef<str>) {
        if self.level >= Level::Normal {
            self.emit(Sev::Info, comp, msg.as_ref());
        }
    }

    /// Mechanical detail (SQL, timings, push-down) — shown only at VERBOSE, as a `DEBUG` line.
    pub fn detail(&self, comp: Comp, msg: impl AsRef<str>) {
        if self.level >= Level::Verbose {
            self.emit(Sev::Debug, comp, msg.as_ref());
        }
    }

    /// A non-fatal fidelity/degradation warning: printed at NORMAL+ and always recorded for the
    /// end-of-run summary count.
    pub fn warn(&self, comp: Comp, msg: impl Into<String>) {
        let m = msg.into();
        if self.level >= Level::Normal {
            self.emit(Sev::Warn, comp, &m);
        }
        self.warnings.borrow_mut().push(m);
    }

    /// A fatal error — always printed, regardless of level. Attributed to the entrypoint.
    pub fn error(&self, msg: impl AsRef<str>) {
        self.emit(Sev::Error, Comp::Entry, msg.as_ref());
    }

    /// How many warnings were emitted (for the summary line).
    pub fn warning_count(&self) -> usize {
        self.warnings.borrow().len()
    }

    /// Format one aligned `LEVEL  component  message` line to stderr. A message with embedded
    /// newlines has its continuation lines indented under the message column so the table holds.
    fn emit(&self, sev: Sev, comp: Comp, msg: &str) {
        // Column widths: level 5 (DEBUG/ERROR), component 6 (decode/layout/render).
        const MSG_COL: usize = 5 + 2 + 6 + 2; // level + gap + comp + gap
        let (lvl, cmp) = if self.color {
            (
                format!("\x1b[{}m{:<5}\x1b[0m", sev.color(), sev.label()),
                format!("\x1b[2m{:<6}\x1b[0m", comp.tag()),
            )
        } else {
            (format!("{:<5}", sev.label()), format!("{:<6}", comp.tag()))
        };
        let msg = msg.replace('\n', &format!("\n{:width$}", "", width = MSG_COL));
        eprintln!("{lvl}  {cmp}  {msg}");
    }
}
