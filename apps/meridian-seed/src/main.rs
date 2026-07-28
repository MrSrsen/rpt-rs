//! `meridian-seed` — deterministic seed-database generator for the synthetic
//! "Meridian Global Logistics" render-test corpus.
//!
//! Emits portable SQL (DDL + batched multi-row `INSERT`s) for either PostgreSQL
//! or SQLite from a single fixed PRNG seed. Running it twice with the same
//! arguments produces byte-identical output.
//!
//! ```text
//! meridian-seed --tier <small|large> --dialect <postgres|sqlite> --out <path>
//! meridian-seed --ddl-only --dialect postgres --out schema.sql
//! ```

mod calendar;
mod fact;
mod market;
mod master;
mod png;
mod pools;
mod rng;
mod sql;
mod world;

use sql::{emit_schema, emit_seed, Dialect};
use std::process::ExitCode;
use world::{Tier, World};

/// Parsed command-line options.
#[derive(Debug)]
struct Options {
    tier: Tier,
    dialect: Dialect,
    out: Option<String>,
    ddl_only: bool,
}

const USAGE: &str = "\
meridian-seed — deterministic Meridian Global Logistics seed generator

USAGE:
    meridian-seed [--tier <small|large>] [--dialect <postgres|sqlite>] [--out <path>] [--ddl-only]

OPTIONS:
    --tier <small|large>        Corpus size (default: small).
    --dialect <postgres|sqlite> Target SQL dialect (default: postgres).
    --out <path>                Output file (default: stdout).
    --ddl-only                  Emit CREATE TABLE statements only, no data.
    -h, --help                  Show this help.";

fn parse_args() -> Result<Options, String> {
    let mut tier = Tier::Small;
    let mut dialect = Dialect::Postgres;
    let mut out = None;
    let mut ddl_only = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--tier" => {
                let v = args.next().ok_or("--tier needs a value")?;
                tier = Tier::parse(&v).ok_or_else(|| format!("unknown tier: {v}"))?;
            }
            "--dialect" => {
                let v = args.next().ok_or("--dialect needs a value")?;
                dialect = Dialect::parse(&v).ok_or_else(|| format!("unknown dialect: {v}"))?;
            }
            "--out" => {
                out = Some(args.next().ok_or("--out needs a value")?);
            }
            "--ddl-only" => ddl_only = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unexpected argument: {other}")),
        }
    }

    Ok(Options {
        tier,
        dialect,
        out,
        ddl_only,
    })
}

fn run(opts: &Options) -> std::io::Result<()> {
    let tables = World::build(opts.tier);
    let mut sql = String::new();
    if opts.ddl_only {
        emit_schema(&tables, opts.dialect, &mut sql);
    } else {
        emit_seed(&tables, opts.dialect, &mut sql);
    }
    match &opts.out {
        Some(path) => std::fs::write(path, sql),
        None => {
            use std::io::Write as _;
            std::io::stdout().write_all(sql.as_bytes())
        }
    }
}

fn main() -> ExitCode {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = run(&opts) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::{emit_schema, Dialect, Tier, World};

    /// The committed `schema.sql` is a frozen snapshot of the generator's default DDL output
    /// (`--ddl-only --dialect postgres`, the small tier). This guards it against silent drift when
    /// the seeder's table definitions change: regenerate the file with
    /// `cargo run -p meridian-seed -- --ddl-only --dialect postgres --out apps/meridian-seed/schema.sql`
    /// after an intentional schema change, then review the diff.
    #[test]
    fn committed_schema_matches_generator() {
        let committed = include_str!("../schema.sql");
        let tables = World::build(Tier::Small);
        let mut generated = String::new();
        emit_schema(&tables, Dialect::Postgres, &mut generated);
        assert_eq!(
            committed, generated,
            "apps/meridian-seed/schema.sql is out of sync with the DDL generator; regenerate it \
             (see this test's doc comment)"
        );
    }
}
