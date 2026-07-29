//! `anonymize` — strip personally identifying authoring metadata, writing a clean `.rpt`.
//!
//! A thin driver over [`rpt_reader::Rpt::anonymize`]: it reports what was removed and writes the single
//! output path passed on the command line. `--dry-run` reports without writing anything, which is
//! how you inspect a corpus before rewriting it.

use rpt_reader::Rpt;

use crate::util::{print_json, CliError};

pub(crate) const HELP: &str = "\
rpt anonymize — remove personally identifying authoring metadata from a report

Reports record who made them and where. The OLE SummaryInformation property set holds the author and
the last person to save; a re-imported subreport holds the full path of the .rpt it came from
(\\\\HOST\\user\\Documents\\...). None of it affects how the report renders, and all of it leaks a real
person and a real machine layout into any corpus the file is committed to.

WHAT IS CHANGED
    author, last_saved_by        blanked — identity and nothing else
    reimport.source_path         reduced to its file name, NOT blanked: it is the only evidence in
                                 the file that a subreport was imported, so emptying it would turn
                                 SubreportObject.IsImported silently false. The directory prefix is
                                 the identifying part; the file name is the subreport's own name,
                                 which the Subdocument storage already records.

WHAT IS NOT
    The database connection's stored path is left alone — it is a live datasource locator, not
    authoring metadata, and blanking it would break the report against its own data.

Every edit is same-length: a value's length prefix is untouched and only its characters are
rewritten, then padded with NULs. No record length, property offset or section size moves, so the
result is a structurally identical file, and the decoded model is unchanged apart from those fields.
A report with nothing to remove is copied byte-for-byte.

USAGE:
    rpt anonymize <in.rpt> <out.rpt>
    rpt anonymize <in.rpt> --dry-run

ARGS:
    <in.rpt>     the report to read
    <out.rpt>    where to write the cleaned report; omit it with --dry-run

OPTIONS:
    --dry-run    report what would be removed and write nothing
    --json       machine-readable output
    -h, --help   show this help

ABOUT:
    Part of the rpt-rs project — a pure-Rust reader for the Crystal Reports (.rpt) format.
    Homepage:     https://github.com/MrSrsen/rpt-rs
    Report bugs:  https://github.com/MrSrsen/rpt-rs/issues
";

/// One reported removal, for `--json`.
#[derive(serde::Serialize)]
struct RemovalJson {
    field: &'static str,
    stream: String,
    value: String,
    replacement: String,
}

/// The `--json` shape: what was removed, and whether anything was written.
#[derive(serde::Serialize)]
struct AnonymizeJson {
    input: String,
    output: Option<String>,
    removals: Vec<RemovalJson>,
}

/// Anonymize `input`, writing the result to `output` unless `dry_run`.
pub(crate) fn anonymize(
    input: &str,
    output: Option<&str>,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    // Requiring the output path (unless --dry-run) is deliberate: this command rewrites report
    // bytes, so it never guesses a destination and never edits in place.
    let out = match (output, dry_run) {
        (Some(path), _) => Some(path),
        (None, true) => None,
        (None, false) => {
            return Err(CliError::usage(
                "anonymize needs an <out.rpt>, or --dry-run to report without writing",
            ))
        }
    };

    let rpt = Rpt::open(input)?;
    let (bytes, report) = rpt.anonymize()?;

    if json {
        print_json(&AnonymizeJson {
            input: input.to_string(),
            output: out.filter(|_| !dry_run).map(str::to_string),
            removals: report
                .removals
                .iter()
                .map(|r| RemovalJson {
                    field: r.field,
                    stream: r.stream.clone(),
                    value: r.value.clone(),
                    replacement: r.replacement.clone(),
                })
                .collect(),
        });
    } else if report.is_empty() {
        println!("{input}: nothing to remove");
    } else {
        println!("{input}:");
        for r in &report.removals {
            let to = if r.replacement.is_empty() {
                "(removed)".to_string()
            } else {
                format!("{:?}", r.replacement)
            };
            println!("  {} · {} = {:?} -> {to}", r.stream, r.field, r.value);
        }
    }

    if let (Some(path), false) = (out, dry_run) {
        std::fs::write(path, &bytes)
            .map_err(|e| CliError::io(format!("cannot write `{path}`"), e))?;
        if !json {
            eprintln!(
                "anonymize: {input} -> {path} ({} bytes, {} removal(s))",
                bytes.len(),
                report.removals.len()
            );
        }
    }
    Ok(())
}
