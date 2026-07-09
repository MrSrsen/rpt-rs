//! `rpt` — a read-only CLI over the `rpt` library for inspecting `.rpt` files.
//!
//! Subcommands: `inspect` (report + per-stream summary), `inputs` (the report's parameters),
//! `tree` (a structural tree of the decoded record DOM), `streams` (raw record-substrate coverage
//! per stream), `dump` (the byte-layout workbench for reverse-engineering a record), `saved`
//! (the report's decoded saved-data rows), `sql` (the SQL the report can run against its database),
//! `xml-dump` (the RptToXml-compatible XML export), and the two write-path commands `reencode` /
//! `patch` (run the writer to a new `.rpt`). Any command accepts `--json`; run
//! `rpt <COMMAND> --help` for command-specific options.
//!
//! Each command lives in its own module (`inspect`/`inputs`/`tree`/`streams`/`dump`/`saved`/`export`/`reencode`);
//! shared exit, JSON, and coloring helpers live in `util`. `main` only parses arguments, routes
//! `--help`, and dispatches.

mod dump;
mod export;
mod inputs;
mod inspect;
mod reencode;
mod saved;
mod sql;
mod streams;
mod tree;
mod util;

use std::process::ExitCode;

use dump::DumpOpts;
use util::{run, use_color};

const USAGE: &str = "\
rpt — inspect Crystal Reports (.rpt) files

A read-only inspector for the .rpt binary format. It opens the OLE/CFB compound file, decrypts and
decodes its streams (Contents, QESession, PromptManager, …) into the record substrate, and reports
what is inside — no Crystal Reports runtime and no database connection needed.

USAGE:
    rpt <COMMAND> <file.rpt> [OPTIONS]
    rpt <COMMAND> --help
    rpt -h | --help

COMMANDS:
    inspect    one-screen report + per-stream summary
    inputs     the report's parameters and their types
    tree       structural tree of the decoded record DOM
    streams    raw record-substrate coverage per stream (decode-coverage meter)
    dump       byte-layout workbench: hex-dump a record's bytes for reverse-engineering
    saved      the report's decoded saved-data rows (schema + cached rowset)
    sql        the SQL the report can run against its database (generated + stored commands)
    xml-dump   export the report as RptToXml-compatible XML (add --full for the raw record tree)
    reencode   re-encode Contents via the writer (no-op round-trip) to a new .rpt
    patch      overwrite a same-size region of one record's leaf, writing a new .rpt

GLOBAL OPTIONS:
    --json         machine-readable JSON output
    -h, --help     show help (per command: `rpt <COMMAND> --help`)

    All commands are read-only. To export the whole report as XML, use `rpt xml-dump <file.rpt>`;
    to render it to HTML / PDF / SVG, use `rpt-render <file.rpt> -o <output>`.

ABOUT:
    Part of the rpt-rs project — a pure-Rust reader for the Crystal Reports (.rpt) format.
    Homepage:     https://github.com/MrSrsen/rpt-rs
    Report bugs:  https://github.com/MrSrsen/rpt-rs/issues
";

/// The scoped `--help` text for a command, or `None` if the token is not a command.
fn help_for(cmd: &str) -> Option<&'static str> {
    match cmd {
        "inspect" => Some(inspect::HELP),
        "inputs" => Some(inputs::HELP),
        "tree" => Some(tree::HELP),
        "streams" => Some(streams::HELP),
        "dump" => Some(dump::HELP),
        "saved" => Some(saved::HELP),
        "sql" => Some(sql::HELP),
        "xml-dump" => Some(export::HELP),
        "reencode" => Some(reencode::HELP),
        "patch" => Some(reencode::PATCH_HELP),
        _ => None,
    }
}

fn main() -> ExitCode {
    // Always emit a full backtrace on panic, regardless of RUST_BACKTRACE. The hook also exits
    // quietly on a closed output pipe (`… | head`, or `… | less` then `q`).
    rpt::install_panic_hook();
    let mut json = false;
    let mut help = false;
    let mut depth: Option<usize> = None;
    let mut no_color = false;
    let mut force_color = false;
    // `dump`-only options.
    let mut dump_type: Option<String> = None;
    let mut dump_stream: Option<String> = None;
    let mut dump_nth: Option<usize> = None;
    let mut dump_offset: Option<String> = None;
    let mut dump_len: Option<String> = None;
    let mut dump_probe: Option<String> = None;
    let mut dump_glob: Option<String> = None;
    let mut dump_cols: Option<String> = None;
    let mut dump_anchor_string: Option<String> = None;
    let mut dump_whole = false;
    let mut dump_saved = false;
    // `xml-dump`-only option.
    let mut xml_full = false;
    // `saved`-only options.
    let mut saved_schema_only = false;
    let mut saved_limit: Option<String> = None;
    // `sql`-only option.
    let mut sql_dialect: Option<String> = None;
    let mut pos: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    // Read a `--flag value` / `--flag=value` option value: the inline form when present, else the
    // next argument.
    let take =
        |arg: &str, prefix: &str, args: &mut dyn Iterator<Item = String>| -> Option<String> {
            if let Some(v) = arg.strip_prefix(prefix).and_then(|s| s.strip_prefix('=')) {
                Some(v.to_string())
            } else {
                args.next()
            }
        };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "-h" | "--help" => help = true,
            "--depth" => depth = args.next().and_then(|v| v.parse().ok()),
            _ if arg.starts_with("--depth=") => {
                depth = arg["--depth=".len()..].parse().ok();
            }
            "--no-color" => no_color = true,
            "--color" => force_color = true,
            "--whole" => dump_whole = true,
            "--saved" => dump_saved = true,
            "--full" => xml_full = true,
            "--schema" => saved_schema_only = true,
            _ if arg == "--limit" || arg.starts_with("--limit=") => {
                saved_limit = take(&arg, "--limit", &mut args);
            }
            _ if arg == "--dialect" || arg.starts_with("--dialect=") => {
                sql_dialect = take(&arg, "--dialect", &mut args);
            }
            _ if arg == "--type" || arg.starts_with("--type=") => {
                dump_type = take(&arg, "--type", &mut args);
            }
            _ if arg == "--stream" || arg.starts_with("--stream=") => {
                dump_stream = take(&arg, "--stream", &mut args);
            }
            _ if arg == "--nth" || arg.starts_with("--nth=") => {
                dump_nth = take(&arg, "--nth", &mut args).and_then(|v| v.parse().ok());
            }
            _ if arg == "--offset" || arg.starts_with("--offset=") => {
                dump_offset = take(&arg, "--offset", &mut args);
            }
            _ if arg == "--len" || arg.starts_with("--len=") => {
                dump_len = take(&arg, "--len", &mut args);
            }
            _ if arg == "--probe" || arg.starts_with("--probe=") => {
                dump_probe = take(&arg, "--probe", &mut args);
            }
            _ if arg == "--glob" || arg.starts_with("--glob=") => {
                dump_glob = take(&arg, "--glob", &mut args);
            }
            _ if arg == "--cols" || arg.starts_with("--cols=") => {
                dump_cols = take(&arg, "--cols", &mut args);
            }
            _ if arg == "--anchor-string" || arg.starts_with("--anchor-string=") => {
                dump_anchor_string = take(&arg, "--anchor-string", &mut args);
            }
            _ => pos.push(arg),
        }
    }
    let color = use_color(force_color, no_color);

    // Help routing (bd-style): `rpt <cmd> --help` prints that command's scoped help; a bare
    // `rpt --help` prints the top-level overview. Explicit help exits with a success code.
    if help {
        match pos.first().and_then(|c| help_for(c)) {
            Some(scoped) => print!("{scoped}"),
            None => print!("{USAGE}"),
        }
        return ExitCode::SUCCESS;
    }

    match pos.as_slice() {
        [cmd, file] if cmd == "inspect" => run(inspect::inspect(file, json)),
        [cmd, file] if cmd == "inputs" => run(inputs::inputs(file, json)),
        [cmd, file] if cmd == "tree" => run(tree::tree(file, json, depth, color)),
        [cmd, file] if cmd == "streams" => run(streams::streams(file, json)),
        [cmd, file] if cmd == "saved" => run(saved::saved(
            file,
            json,
            saved_schema_only,
            saved_limit.as_deref(),
        )),
        [cmd, files @ ..] if cmd == "dump" && (!files.is_empty() || dump_glob.is_some()) => {
            let opts = DumpOpts {
                ty: dump_type,
                stream: dump_stream,
                nth: dump_nth,
                offset: dump_offset,
                len: dump_len,
                probe: dump_probe,
                whole: dump_whole,
                saved: dump_saved,
                json,
                color,
                glob: dump_glob,
                cols: dump_cols,
                anchor_string: dump_anchor_string,
            };
            run(dump::dump(files, &opts))
        }
        [cmd, file] if cmd == "sql" => run(match sql::parse_dialect(sql_dialect.as_deref()) {
            Ok(dialect) => sql::sql(file, json, dialect, color),
            Err(e) => Err(e),
        }),
        [cmd, input] if cmd == "xml-dump" => run(export::run(input, None, xml_full)),
        [cmd, input, output] if cmd == "xml-dump" => {
            run(export::run(input, Some(output), xml_full))
        }
        [cmd, input, output] if cmd == "reencode" => run(reencode::reencode(input, output)),
        [cmd, input, tag, nth, offset, hexbytes, output] if cmd == "patch" => {
            run(reencode::patch(input, tag, nth, offset, hexbytes, output))
        }
        // A malformed invocation of a known command prints that command's scoped help to stderr;
        // anything else prints the top-level overview. Both exit non-zero.
        _ => {
            match pos.first().and_then(|c| help_for(c)) {
                Some(scoped) => eprint!("{scoped}"),
                None => eprint!("{USAGE}"),
            }
            ExitCode::from(2)
        }
    }
}
