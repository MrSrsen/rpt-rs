//! Command-line argument parsing for the `rpt` CLI.
//!
//! A tiny, dependency-free, table-driven parser. [`parse`] takes the resolved subcommand and its
//! argument list and accepts **only that command's declared flags**, so a flag meant for one
//! command is rejected on another (`unknown option '--probe' for 'inspect'`) instead of being
//! silently swallowed. Each command maps to a [`Command`] variant carrying exactly the typed
//! options it needs — the per-command surfaces are explicit and unit-testable.

use crate::dump::DumpOpts;
use crate::util::{use_color, CliError};

/// A fully-parsed subcommand invocation, ready to dispatch. Each variant carries only the options
/// its command consumes.
#[derive(Debug)]
pub(crate) enum Command {
    Inspect {
        file: String,
        json: bool,
    },
    Inputs {
        file: String,
        json: bool,
    },
    Tree {
        file: String,
        json: bool,
        depth: Option<usize>,
        color: bool,
    },
    Streams {
        file: String,
        json: bool,
    },
    Formulas {
        file: String,
        json: bool,
        /// Report only through the exit status.
        quiet: bool,
        /// Print each formula's source under its listing line.
        source: bool,
    },
    Saved {
        file: String,
        json: bool,
        schema_only: bool,
        limit: Option<String>,
    },
    Dump {
        files: Vec<String>,
        opts: DumpOpts,
    },
    Sql {
        file: String,
        json: bool,
        dialect: Option<String>,
        color: bool,
    },
    JsonDump {
        input: String,
        output: Option<String>,
        /// Treat an incomplete decode as an error rather than a warning.
        strict: bool,
    },
    Kdl {
        input: String,
        output: Option<String>,
        /// Treat an incomplete decode as an error rather than a warning.
        strict: bool,
    },
    Anonymize {
        input: String,
        output: Option<String>,
        dry_run: bool,
        json: bool,
    },
    Reencode {
        input: String,
        output: String,
    },
    Patch {
        input: String,
        tag: String,
        nth: String,
        /// The field to change, by name, or `@<offset>` for the raw byte form.
        target: String,
        /// The new value, read at the field's declared wire type.
        value: String,
        output: String,
        /// Write without the write path's safety checks.
        force: bool,
    },
}

/// Why argument parsing failed.
#[derive(Debug)]
pub(crate) enum ArgsError {
    /// A clean usage error — an unknown flag or a malformed flag value. Report it verbatim (exit 2).
    Usage(CliError),
    /// A malformed invocation of an otherwise-known command (the wrong number of positional
    /// arguments). The caller prints that command's scoped help (exit 2).
    Malformed,
}

impl ArgsError {
    fn usage(msg: impl Into<String>) -> Self {
        ArgsError::Usage(CliError::usage(msg))
    }
}

/// Whether a declared flag consumes a following value (`--type Formula`) or is a bare switch
/// (`--json`).
#[derive(Clone, Copy)]
enum Arity {
    Flag,
    Value,
}

use Arity::{Flag, Value};

/// The flags and positionals collected for one command by [`collect`].
struct Bag {
    /// Bare switches that were present (stored by their canonical spec name).
    bools: Vec<&'static str>,
    /// Value flags and the value each was given (last occurrence wins on read).
    values: Vec<(&'static str, String)>,
    /// Positional arguments, in order.
    positionals: Vec<String>,
}

impl Bag {
    fn has(&self, name: &str) -> bool {
        self.bools.contains(&name)
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .rev()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }

    /// A value flag parsed as `usize`; a non-numeric value is a usage error.
    fn get_usize(&self, name: &str) -> Result<Option<usize>, ArgsError> {
        match self.get(name) {
            None => Ok(None),
            Some(v) => v
                .parse()
                .map(Some)
                .map_err(|_| ArgsError::usage(format!("{name} expects a number, got '{v}'"))),
        }
    }
}

/// Walk `args` against a command's declared flag table, gathering switches, value flags, and
/// positionals. An `--flag=value` inline form is accepted; a value flag otherwise consumes the next
/// token. An undeclared `--flag` is an `unknown option … for '<cmd>'` usage error.
fn collect(cmd: &str, specs: &[(&'static str, Arity)], args: &[String]) -> Result<Bag, ArgsError> {
    let mut bag = Bag {
        bools: Vec::new(),
        values: Vec::new(),
        positionals: Vec::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        let Some(body) = arg.strip_prefix("--") else {
            bag.positionals.push(arg.clone());
            continue;
        };
        let (name, inline) = match body.split_once('=') {
            Some((n, v)) => (format!("--{n}"), Some(v.to_string())),
            None => (format!("--{body}"), None),
        };
        match specs.iter().find(|(n, _)| *n == name) {
            Some((n, Flag)) => {
                if inline.is_some() {
                    return Err(ArgsError::usage(format!("option '{n}' takes no value")));
                }
                bag.bools.push(n);
            }
            Some((n, Value)) => {
                // The inline value, else the next token (a trailing value flag with no argument is
                // left unset, matching the original parser).
                let value = inline.or_else(|| {
                    let next = args.get(i).cloned();
                    if next.is_some() {
                        i += 1;
                    }
                    next
                });
                if let Some(v) = value {
                    bag.values.push((n, v));
                }
            }
            None => {
                return Err(ArgsError::usage(format!(
                    "unknown option '{name}' for '{cmd}'"
                )))
            }
        }
    }
    Ok(bag)
}

/// Exactly one positional (a single input file), else the invocation is malformed.
fn one_file(bag: &Bag) -> Result<String, ArgsError> {
    match bag.positionals.as_slice() {
        [file] => Ok(file.clone()),
        _ => Err(ArgsError::Malformed),
    }
}

/// One required input and an optional output positional (`<input> [output]`).
fn input_output(bag: &Bag) -> Result<(String, Option<String>), ArgsError> {
    match bag.positionals.as_slice() {
        [input] => Ok((input.clone(), None)),
        [input, output] => Ok((input.clone(), Some(output.clone()))),
        _ => Err(ArgsError::Malformed),
    }
}

/// The export/write commands emit a fixed format, so `--json` does not apply. Warn rather than
/// silently ignore it (the inspection commands honour it).
fn warn_json_ignored(cmd: &str, bag: &Bag) {
    if bag.has("--json") {
        eprintln!(
            "warning: --json does not apply to `{cmd}` (it emits a fixed format); ignoring it"
        );
    }
}

/// Parse the argument list `args` for the already-resolved subcommand `cmd` into a [`Command`].
///
/// `cmd` must be a known command name (the caller resolves it); only that command's flags are
/// accepted. `args` is the invocation with the command token removed.
pub(crate) fn parse(cmd: &str, args: &[String]) -> Result<Command, ArgsError> {
    match cmd {
        "inspect" => {
            let bag = collect(cmd, &[("--json", Flag)], args)?;
            Ok(Command::Inspect {
                file: one_file(&bag)?,
                json: bag.has("--json"),
            })
        }
        "inputs" => {
            let bag = collect(cmd, &[("--json", Flag)], args)?;
            Ok(Command::Inputs {
                file: one_file(&bag)?,
                json: bag.has("--json"),
            })
        }
        "tree" => {
            let bag = collect(
                cmd,
                &[
                    ("--json", Flag),
                    ("--depth", Value),
                    ("--color", Flag),
                    ("--no-color", Flag),
                ],
                args,
            )?;
            let depth = bag.get_usize("--depth")?;
            Ok(Command::Tree {
                file: one_file(&bag)?,
                json: bag.has("--json"),
                depth,
                color: use_color(bag.has("--color"), bag.has("--no-color")),
            })
        }
        "streams" => {
            let bag = collect(cmd, &[("--json", Flag)], args)?;
            Ok(Command::Streams {
                file: one_file(&bag)?,
                json: bag.has("--json"),
            })
        }
        "formulas" => {
            let bag = collect(
                cmd,
                &[("--json", Flag), ("--quiet", Flag), ("--source", Flag)],
                args,
            )?;
            Ok(Command::Formulas {
                file: one_file(&bag)?,
                json: bag.has("--json"),
                quiet: bag.has("--quiet"),
                source: bag.has("--source"),
            })
        }
        "saved" => {
            let bag = collect(
                cmd,
                &[("--json", Flag), ("--schema", Flag), ("--limit", Value)],
                args,
            )?;
            Ok(Command::Saved {
                file: one_file(&bag)?,
                json: bag.has("--json"),
                schema_only: bag.has("--schema"),
                limit: bag.get("--limit").map(str::to_string),
            })
        }
        "dump" => {
            let bag = collect(
                cmd,
                &[
                    ("--json", Flag),
                    ("--type", Value),
                    ("--stream", Value),
                    ("--nth", Value),
                    ("--offset", Value),
                    ("--len", Value),
                    ("--probe", Value),
                    ("--glob", Value),
                    ("--cols", Value),
                    ("--anchor-string", Value),
                    ("--grid", Flag),
                    ("--whole", Flag),
                    ("--saved", Flag),
                    ("--color", Flag),
                    ("--no-color", Flag),
                ],
                args,
            )?;
            let nth = bag.get_usize("--nth")?;
            let glob = bag.get("--glob").map(str::to_string);
            // A `dump` needs at least one file or a `--glob` to sweep.
            if bag.positionals.is_empty() && glob.is_none() {
                return Err(ArgsError::Malformed);
            }
            let opts = DumpOpts {
                ty: bag.get("--type").map(str::to_string),
                stream: bag.get("--stream").map(str::to_string),
                nth,
                offset: bag.get("--offset").map(str::to_string),
                len: bag.get("--len").map(str::to_string),
                probe: bag.get("--probe").map(str::to_string),
                glob,
                cols: bag.get("--cols").map(str::to_string),
                anchor_string: bag.get("--anchor-string").map(str::to_string),
                grid: bag.has("--grid"),
                whole: bag.has("--whole"),
                saved: bag.has("--saved"),
                json: bag.has("--json"),
                color: use_color(bag.has("--color"), bag.has("--no-color")),
            };
            Ok(Command::Dump {
                files: bag.positionals,
                opts,
            })
        }
        "sql" => {
            let bag = collect(
                cmd,
                &[
                    ("--json", Flag),
                    ("--dialect", Value),
                    ("--color", Flag),
                    ("--no-color", Flag),
                ],
                args,
            )?;
            Ok(Command::Sql {
                file: one_file(&bag)?,
                json: bag.has("--json"),
                dialect: bag.get("--dialect").map(str::to_string),
                color: use_color(bag.has("--color"), bag.has("--no-color")),
            })
        }
        "json-dump" => {
            let bag = collect(cmd, &[("--json", Flag), ("--strict", Flag)], args)?;
            warn_json_ignored(cmd, &bag);
            let (input, output) = input_output(&bag)?;
            Ok(Command::JsonDump {
                input,
                output,
                strict: bag.has("--strict"),
            })
        }
        "kdl" => {
            let bag = collect(cmd, &[("--json", Flag), ("--strict", Flag)], args)?;
            warn_json_ignored(cmd, &bag);
            let (input, output) = input_output(&bag)?;
            Ok(Command::Kdl {
                input,
                output,
                strict: bag.has("--strict"),
            })
        }
        "anonymize" => {
            let bag = collect(cmd, &[("--json", Flag), ("--dry-run", Flag)], args)?;
            let dry_run = bag.has("--dry-run");
            let (input, output) = input_output(&bag)?;
            Ok(Command::Anonymize {
                input,
                output,
                dry_run,
                json: bag.has("--json"),
            })
        }
        "reencode" => {
            let bag = collect(cmd, &[("--json", Flag)], args)?;
            warn_json_ignored(cmd, &bag);
            match bag.positionals.as_slice() {
                [input, output] => Ok(Command::Reencode {
                    input: input.clone(),
                    output: output.clone(),
                }),
                _ => Err(ArgsError::Malformed),
            }
        }
        "patch" => {
            let bag = collect(cmd, &[("--json", Flag), ("--force", Flag)], args)?;
            warn_json_ignored(cmd, &bag);
            let force = bag.has("--force");
            match bag.positionals.as_slice() {
                [input, tag, nth, target, value, output] => Ok(Command::Patch {
                    input: input.clone(),
                    tag: tag.clone(),
                    nth: nth.clone(),
                    target: target.clone(),
                    value: value.clone(),
                    output: output.clone(),
                    force,
                }),
                _ => Err(ArgsError::Malformed),
            }
        }
        // `parse` is only reached for a resolved command token; any other value is a caller bug.
        other => Err(ArgsError::usage(format!("unknown command '{other}'"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an owned argument slice from string literals.
    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    /// The `Display` text of a `Usage` error (else a panic, so mismatched variants fail loudly).
    fn usage_msg(err: ArgsError) -> String {
        match err {
            ArgsError::Usage(e) => e.to_string(),
            ArgsError::Malformed => panic!("expected a usage error, got Malformed"),
        }
    }

    #[test]
    fn valid_inspect_parses() {
        let cmd = parse("inspect", &argv(&["f.rpt"])).expect("valid");
        match cmd {
            Command::Inspect { file, json } => {
                assert_eq!(file, "f.rpt");
                assert!(!json);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn json_switch_is_recorded() {
        let cmd = parse("inspect", &argv(&["f.rpt", "--json"])).expect("valid");
        match cmd {
            Command::Inspect { json, .. } => assert!(json),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn unknown_flag_for_command_is_a_usage_error() {
        // `--probe` is a `dump` flag; it must be rejected for `inspect`, not silently ignored.
        let err = parse("inspect", &argv(&["f.rpt", "--probe", "u32"])).expect_err("rejected");
        assert_eq!(usage_msg(err), "unknown option '--probe' for 'inspect'");
    }

    #[test]
    fn non_numeric_flag_value_is_a_usage_error() {
        let err = parse("tree", &argv(&["f.rpt", "--depth", "abc"])).expect_err("rejected");
        assert_eq!(usage_msg(err), "--depth expects a number, got 'abc'");
    }

    #[test]
    fn numeric_flag_value_parses() {
        let cmd = parse("tree", &argv(&["f.rpt", "--depth", "3"])).expect("valid");
        match cmd {
            Command::Tree { depth, .. } => assert_eq!(depth, Some(3)),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn inline_value_form_is_accepted() {
        let cmd = parse("saved", &argv(&["f.rpt", "--limit=5"])).expect("valid");
        match cmd {
            Command::Saved { limit, .. } => assert_eq!(limit.as_deref(), Some("5")),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn dump_collects_files_and_value_flags() {
        let cmd = parse("dump", &argv(&["a.rpt", "--type", "0x76"])).expect("valid");
        match cmd {
            Command::Dump { files, opts } => {
                assert_eq!(files, vec!["a.rpt".to_string()]);
                assert_eq!(opts.ty.as_deref(), Some("0x76"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn dump_without_a_file_or_glob_is_malformed() {
        let err = parse("dump", &argv(&["--type", "0x76"])).expect_err("rejected");
        assert!(matches!(err, ArgsError::Malformed));
    }

    #[test]
    fn dump_with_only_a_glob_is_valid() {
        let cmd = parse(
            "dump",
            &argv(&["--glob", "reports/*.rpt", "--type", "0x76"]),
        )
        .expect("valid");
        match cmd {
            Command::Dump { files, opts } => {
                assert!(files.is_empty());
                assert_eq!(opts.glob.as_deref(), Some("reports/*.rpt"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn json_dump_takes_an_optional_output() {
        let one = parse("json-dump", &argv(&["in.rpt"])).expect("valid");
        match one {
            Command::JsonDump { output, .. } => assert_eq!(output, None),
            other => panic!("wrong variant: {other:?}"),
        }
        let two = parse("json-dump", &argv(&["in.rpt", "out.json"])).expect("valid");
        match two {
            Command::JsonDump {
                input,
                output,
                strict,
            } => {
                assert_eq!(input, "in.rpt");
                assert_eq!(output.as_deref(), Some("out.json"));
                assert!(!strict, "--strict is opt-in");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    /// `--strict` turns an incomplete decode from a warning into a failure, for CI use.
    #[test]
    fn strict_is_accepted_by_both_export_commands() {
        for cmd in ["json-dump", "kdl"] {
            let parsed = parse(cmd, &argv(&["in.rpt", "--strict"])).expect("valid");
            let strict = match parsed {
                Command::JsonDump { strict, .. } | Command::Kdl { strict, .. } => strict,
                other => panic!("wrong variant: {other:?}"),
            };
            assert!(strict, "{cmd} --strict did not set the flag");
        }
    }

    #[test]
    fn missing_positional_is_malformed() {
        assert!(matches!(
            parse("inspect", &argv(&[])),
            Err(ArgsError::Malformed)
        ));
        assert!(matches!(
            parse("reencode", &argv(&["only-one"])),
            Err(ArgsError::Malformed)
        ));
    }

    #[test]
    fn patch_requires_six_positionals() {
        let cmd = parse(
            "patch",
            &argv(&["in.rpt", "0x76", "0", "group_indent", "12", "out.rpt"]),
        )
        .expect("valid");
        match cmd {
            Command::Patch { target, value, .. } => {
                assert_eq!(target, "group_indent");
                assert_eq!(value, "12");
            }
            other => panic!("wrong variant: {other:?}"),
        }
        assert!(matches!(
            parse("patch", &argv(&["in.rpt", "0x76"])),
            Err(ArgsError::Malformed)
        ));
    }
}
