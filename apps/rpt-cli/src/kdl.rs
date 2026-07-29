//! `kdl` — export the report as a KDL document (the human-readable authoring surface).
//!
//! Decodes the `.rpt` directly (no runtime, no database) and serialises its semantic model to KDL
//! via `rpt-kdl`. Embedded picture bytes are never written into the KDL: each references a sidecar
//! file, which this command writes next to the output document when an output path is given.

use std::path::Path;

use rpt_reader::Rpt;

use crate::util::CliError;

pub(crate) const HELP: &str = "\
rpt kdl — export a Crystal Reports (.rpt) file to KDL

Decodes the .rpt binary directly and serialises its semantic model to a KDL document: construct
kinds as nodes, names as arguments, scalars as key=value properties, nested structs and lists as
child nodes, and only non-default values emitted. Geometry stays in raw twips, colors render as
#rrggbb, enums as kebab-case tokens, and formula bodies as multi-line strings.

Embedded picture bytes never enter the KDL: each picture references a sidecar file (embed-N.<ext>,
namespaced per subreport). When an output path is given, those sidecar files are written alongside
the .kdl document.

USAGE:
    rpt kdl <file.rpt> [out.kdl] [--strict]

ARGS:
    <file.rpt>    the report to read
    [out.kdl]     output path; if omitted, KDL is written to stdout (sidecar assets are not written)

OPTIONS:
    --strict      fail instead of warning when the report did not decode completely
    -h, --help    show this help

ABOUT:
    Part of the rpt-rs project — a pure-Rust reader for the Crystal Reports (.rpt) format.
    Homepage:     https://github.com/MrSrsen/rpt-rs
    Report bugs:  https://github.com/MrSrsen/rpt-rs/issues
";

/// Export `input` to KDL. With `output`, writes the document there plus any embedded picture assets
/// as sidecar files; without it, prints the KDL to stdout (assets are not written).
///
/// Warns when `input` did not decode completely, so a partial export is not mistaken for a faithful
/// one; `strict` makes that an error instead.
pub(crate) fn run(input: &str, output: Option<&str>, strict: bool) -> Result<(), CliError> {
    let rpt = Rpt::open(input)?;
    let coverage = rpt.decode_coverage();
    let report = rpt.report();
    let kdl = rpt_kdl::to_kdl_string(report);
    match output {
        None => print!("{kdl}"),
        Some(path) => {
            std::fs::write(path, &kdl)
                .map_err(|e| CliError::io(format!("cannot write `{path}`"), e))?;
            let base = Path::new(path).parent().unwrap_or_else(|| Path::new(""));
            let assets = rpt_kdl::assets(report);
            for asset in &assets {
                let dest = base.join(&asset.path);
                if let Some(dir) = dest.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| {
                        CliError::io(format!("cannot create `{}`", dir.display()), e)
                    })?;
                }
                std::fs::write(&dest, &asset.bytes)
                    .map_err(|e| CliError::io(format!("cannot write `{}`", dest.display()), e))?;
            }
            eprintln!(
                "kdl: {input} -> {path} ({} bytes, {} asset{})",
                kdl.len(),
                assets.len(),
                if assets.len() == 1 { "" } else { "s" }
            );
        }
    }
    // Reported after the write, as `json-dump` does: the document is still produced and useful
    // for diagnosis, and `--strict` changes only the exit status.
    crate::util::report_coverage(&coverage, input, strict)?;
    Ok(())
}
