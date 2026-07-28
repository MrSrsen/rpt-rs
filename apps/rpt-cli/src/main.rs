//! `rpt` — a read-only CLI over the `rpt` library for inspecting `.rpt` files.
//!
//! Subcommands: `inspect` (report + per-stream summary), `inputs` (the report's parameters),
//! `tree` (a structural tree of the decoded record DOM), `streams` (raw record-substrate coverage
//! per stream), `dump` (the byte-layout workbench for a record's raw bytes), `saved`
//! (the report's decoded saved-data rows), `sql` (the SQL the report can run against its database),
//! `json-dump` (the exhaustive JSON export), and the write-path commands `anonymize` (strip
//! authoring metadata) / `reencode` / `patch` (run the writer to a new `.rpt`). Any command accepts `--json`; run `rpt <COMMAND> --help` for
//! command-specific options.
//!
//! Each command lives in its own module (`inspect`/`inputs`/`tree`/`streams`/`dump`/`saved`/`reencode`);
//! the `json-dump` and `kdl` export surfaces live in the `rpt-json` and `rpt-kdl` libraries. Shared
//! exit, JSON, and coloring helpers live in `util`. `main` only parses arguments, routes `--help`,
//! and dispatches.

mod anonymize;
mod args;
mod dump;
mod formulas;
mod inputs;
mod inspect;
mod kdl;
mod reencode;
mod saved;
mod sql;
mod streams;
mod tree;
mod util;

use std::process::ExitCode;

use args::{parse, ArgsError, Command};
use util::{run, CliError};

const USAGE: &str = "\
rpt — inspect Crystal Reports (.rpt) files

A read-only inspector for the .rpt binary format. It opens the OLE/CFB compound file, decrypts and
decodes its streams (Contents, QESession, PromptManager, …) into the record substrate, and reports
what is inside. Reads the file alone; no database connection is made.

USAGE:
    rpt <COMMAND> <file.rpt> [OPTIONS]
    rpt <COMMAND> --help
    rpt -h | --help

COMMANDS:
    inspect    one-screen report + per-stream summary
    inputs     the report's parameters and their types
    tree       structural tree of the decoded record DOM
    streams    raw record-substrate coverage per stream (decode-coverage meter)
    dump       byte-layout workbench: annotated hex dump of a record's leaf bytes
    saved      the report's decoded saved-data rows (schema + cached rowset)
    sql        the SQL the report can run against its database (generated + stored commands)
    formulas   check every formula in the report for syntax and semantic errors (no render)
    json-dump  export the report as exhaustive, deterministic JSON (the decoded model)
    kdl        export the report as a KDL document (human-readable authoring surface)
    anonymize  remove authoring metadata (author, last saver, import paths) to a new .rpt
    reencode   re-encode Contents via the writer (no-op round-trip) to a new .rpt
    patch      overwrite a same-size region of one record's leaf, writing a new .rpt

GLOBAL OPTIONS:
    --json         machine-readable JSON output
    -h, --help     show help (per command: `rpt <COMMAND> --help`)

    All commands are read-only. To export the whole decoded report, use `rpt json-dump <file.rpt>`;
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
        "formulas" => Some(formulas::HELP),
        "json-dump" => Some(rpt_json::JSON_DUMP_HELP),
        "kdl" => Some(kdl::HELP),
        "anonymize" => Some(anonymize::HELP),
        "reencode" => Some(reencode::HELP),
        "patch" => Some(reencode::PATCH_HELP),
        _ => None,
    }
}

fn main() -> ExitCode {
    // Always emit a full backtrace on panic, regardless of RUST_BACKTRACE. The hook also exits
    // quietly on a closed output pipe (`… | head`, or `… | less` then `q`).
    rpt::install_panic_hook();
    let argv: Vec<String> = std::env::args().skip(1).collect();

    // `-h`/`--help` anywhere requests help. The subcommand is the first token naming a known
    // command; flags may precede or follow it.
    let help = argv.iter().any(|a| a == "-h" || a == "--help");
    let cmd_idx = argv.iter().position(|a| help_for(a).is_some());

    // Help routing: `rpt <cmd> --help` prints that command's scoped help; a bare `rpt --help` (or
    // an unknown command) prints the top-level overview. Explicit help exits with a success code.
    if help {
        match cmd_idx.and_then(|i| help_for(&argv[i])) {
            Some(scoped) => print!("{scoped}"),
            None => print!("{USAGE}"),
        }
        return ExitCode::SUCCESS;
    }

    // No known command token: the top-level overview to stderr, a usage exit.
    let Some(idx) = cmd_idx else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    let cmd = &argv[idx];
    // The command's argument list is everything around the command token, order preserved.
    let mut rest: Vec<String> = Vec::with_capacity(argv.len() - 1);
    rest.extend_from_slice(&argv[..idx]);
    rest.extend_from_slice(&argv[idx + 1..]);

    match parse(cmd, &rest) {
        Ok(command) => dispatch(command),
        // An unknown flag or malformed flag value: a clean one-line usage error, exit 2.
        Err(ArgsError::Usage(e)) => run(Err(e)),
        // A known command with the wrong positionals: its scoped help to stderr, exit 2.
        Err(ArgsError::Malformed) => {
            if let Some(scoped) = help_for(cmd) {
                eprint!("{scoped}");
            }
            ExitCode::from(2)
        }
    }
}

/// Run a parsed command, mapping its result to a process exit code.
fn dispatch(command: Command) -> ExitCode {
    match command {
        Command::Inspect { file, json } => run(inspect::inspect(&file, json)),
        Command::Inputs { file, json } => run(inputs::inputs(&file, json)),
        Command::Tree {
            file,
            json,
            depth,
            color,
        } => run(tree::tree(&file, json, depth, color)),
        Command::Streams { file, json } => run(streams::streams(&file, json)),
        Command::Formulas {
            file,
            json,
            quiet,
            source,
        } => run(formulas::run(&file, json, quiet, source)),
        Command::Saved {
            file,
            json,
            schema_only,
            limit,
        } => run(saved::saved(&file, json, schema_only, limit.as_deref())),
        Command::Dump { files, opts } => run(dump::dump(&files, &opts)),
        Command::Sql {
            file,
            json,
            dialect,
            color,
        } => run(match sql::parse_dialect(dialect.as_deref()) {
            Ok(d) => sql::sql(&file, json, d, color),
            Err(e) => Err(e),
        }),
        Command::JsonDump {
            input,
            output,
            strict,
        } => run(rpt_json::export_json(&input, output.as_deref())
            .map_err(CliError::from)
            .and_then(|coverage| util::report_coverage(&coverage, &input, strict))),
        Command::Kdl {
            input,
            output,
            strict,
        } => run(kdl::run(&input, output.as_deref(), strict)),
        Command::Anonymize {
            input,
            output,
            dry_run,
            json,
        } => run(anonymize::anonymize(
            &input,
            output.as_deref(),
            dry_run,
            json,
        )),
        Command::Reencode { input, output } => run(reencode::reencode(&input, &output)),
        Command::Patch {
            input,
            tag,
            nth,
            offset,
            hexbytes,
            output,
            force,
        } => run(reencode::patch(
            &input, &tag, &nth, &offset, &hexbytes, &output, force,
        )),
    }
}
