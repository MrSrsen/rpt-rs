//! Structured leveled logger for the `rpt-render` CLI.
//!
//! Every line is `LEVEL  component  message` on **stderr** (stdout stays reserved for the
//! rendered bytes when `-o -`). The level and component are fixed-width columns so a run reads
//! as an aligned table you can scan by column:
//!
//! ```text
//! INFO   render  rendering "invoice.rpt" → stdout (PDF)
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
//! - Color is applied only when stderr is a TTY and `NO_COLOR` is unset.
//!
//! Warnings feed the **fidelity channel**: each [`warn`](Log::warn) is recorded so the run can
//! end with a one-line summary count ("rendered with N warnings").

#[cfg(feature = "db")]
use std::cell::Cell;
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

/// One line's severity — drives the `LEVEL` column and its color.
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

    /// SGR color code for the level label (used only when color is enabled).
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
    /// Monotonic counter handing out a 1-based id to each SQL query across all scopes (main report +
    /// every subreport), so a query's SQL / execution / row-count lines can be tagged `[query N]`.
    #[cfg(feature = "db")]
    query_seq: Cell<usize>,
}

impl Log {
    pub fn new(level: Level) -> Log {
        // Color only when stderr is a real terminal and the user hasn't opted out via NO_COLOR.
        let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        Log {
            level,
            color,
            warnings: RefCell::new(Vec::new()),
            #[cfg(feature = "db")]
            query_seq: Cell::new(0),
        }
    }

    /// Allocate the next 1-based query id, shared across the main report and all subreport scopes,
    /// so the SQL / execution / row-count lines of one query can be grouped by a `[query N]` tag.
    #[cfg(feature = "db")]
    pub fn next_query_id(&self) -> usize {
        let n = self.query_seq.get() + 1;
        self.query_seq.set(n);
        n
    }

    /// Is verbose (`-v`) detail being printed? Guard expensive-to-format `DEBUG` lines with this.
    /// Only the live-DB path has any (the generated SQL), so a DB-free build does not carry it.
    #[cfg(feature = "db")]
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

    /// An error-severity diagnostic from `comp`: the render still produced output, but something in it
    /// is wrong — a dropped row, a field that resolved to nothing.
    ///
    /// Always printed, `-q` included: unlike a fidelity warning, this says the output does not
    /// represent the data, which a scripted run must not miss. Counted in the summary.
    pub fn error_at(&self, comp: Comp, msg: impl Into<String>) {
        let m = msg.into();
        self.emit(Sev::Error, comp, &m);
        self.warnings.borrow_mut().push(m);
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
