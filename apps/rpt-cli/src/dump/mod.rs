//! `dump` — the byte-layout workbench for a record's raw bytes.
//!
//! It selects records by type and dumps each one's **own field bytes**, demasked and with its
//! nested records spliced out, as an annotated hex dump alongside the length-prefixed strings they
//! contain — read through the reader's own scanner, so the workbench and the decoder cannot disagree
//! about what is text. Two or more files trigger a byte-aligned diff
//! of the first match in each — the minimal-pair view of an opaque record tail.
//!
//! What follows the hex depends on whether the record's type is decoded from a declarative field
//! table. A record type that has one gets the **field-table view** ([`fields`]): each field's
//! value with the bytes it came from, and whether the table consumed the record exactly. One that
//! does not gets the **scalar-probe grid** (u16/u32, big- and little-endian, at every offset),
//! which maps 1:1 onto the reader vocabulary a new decoder is written in — the instrument for
//! finding a field that is not yet named.
//!
//! The command is split by responsibility: [`parse`] (option/selector/column parsing + byte-view
//! primitives), [`collect`] (record-tree walking into [`DumpMatch`]), [`render`] (the human-readable
//! per-record view, type index, and raw-span escape hatch), [`fields`] (the field-table view),
//! [`sweep`] (the minimal-pair diff and the corpus-sweep table), [`json`] (the `--json` per-record
//! output), and [`saved`] (the saved-data batch view).

use std::fmt::Write as _;

use rpt_reader::fields::RecordFields;
use rpt_reader::raw::Dialect;
use rpt_reader::Rpt;

use crate::util::{print_json, CliError};

mod collect;
mod fields;
mod json;
mod parse;
mod render;
mod saved;
mod sweep;

use collect::{gather, selector_dialect};
use json::build_dump_json;
use parse::{parse_num, parse_type_selector, type_label};
use render::{dump_raw_span, dump_type_index, render_match};
use saved::dump_saved;
use sweep::{diff_matches, dump_sweep, expand_glob};

pub(crate) const HELP: &str = "\
rpt dump — the byte-layout workbench for a record's raw bytes

Selects records by type and dumps each one's own field bytes, demasked, as an annotated hex dump
plus the length-prefixed strings they contain, read through the reader's own scanner.

What follows the hex depends on the record type:
  • a type decoded from a declarative field table gets that table's own reading — every field's
    name, value and byte range, its skip runs left visible as unknowns, and whether the table
    consumed the record exactly (the check a new table has to pass);
  • a type without one gets the scalar-probe grid (u16/u32, big- & little-endian, at every
    offset), which maps 1:1 onto the reader vocabulary you write a new decoder with.
The view is chosen by whether a table exists; --grid forces the grid either way.

A record's content is a sequence of pieces — runs of its own field bytes and the records nested
between them — and each run is contiguous in the file. The hex JOINS the runs, so it is a buffer of
the tool's own making: a record with children shows its seam offsets, and bytes either side of a
seam are not adjacent on disk. Byte ranges in the field-table view are absolute offsets into the
decoded stream; `joined` is the same byte's offset in the hex above. They are NOT the same
coordinate, and they diverge past every child.

With no --type it prints the stream's record-type index (the table of contents). Given two or more
files it also prints a byte-aligned diff of the first match in each: author two reports identical
but for one property, then `dump --type 0xNNNN a.rpt b.rpt` shows exactly which bytes moved.

--glob sweeps a whole corpus into one table (one row per file×record), and --cols pulls specific
bytes into columns — the corpus-wide confound-breaker. Name a DIRECTORY to sweep every .rpt beneath
it recursively (a report corpus is a tree of subdirectories, and a one-level sweep over one matches
nothing while looking like an answer); '<dir>/**/*.rpt' does the same. Offsets can be absolute or anchored at a
decoded LP-string's end (`used±N`, or --anchor-string to pick the string by a marker), so a field's
trailing tail is read relative to the field-ref end even as its absolute offset shifts.

USAGE:
    rpt dump <file.rpt> [file2.rpt …] --type <T> [--nth N] [--stream S] [--probe N] [--grid] [--whole]
    rpt dump --glob '<corpus-dir>' --type <T> [--cols SPEC,…] [--anchor-string TXT] [--nth N]
    rpt dump <file.rpt> --stream S --offset O --len L     (raw span escape hatch)
    rpt dump <file.rpt> --saved                           (saved-data batches)
    rpt dump <file.rpt>                                   (record-type index of the stream)

OPTIONS:
    --type T      record type to dump: hex (0x76 / 76) or a registry name from any stream's
                  vocabulary (Formula, QeIndex, SavedBatchEntry, CurrentValueRecord). Omit to print
                  the stream's record-type index instead
    --nth N       dump only the Nth match (0-based, pre-order across the selected streams)
    --stream S    which stream to read: contents (default), qe, all, or a stream-id substring
                  (e.g. Subdocument, DataSourceManager, ReportParametersStream). A record type
                  number is per stream, so every record is named and read in its own stream's
                  vocabulary: 0x0003 is PrinterInfo in Contents and QeTable in QESession, each with
                  its own field table. Four vocabularies are read — the report definition, the
                  query-engine session, the saved-data catalog, and the saved parameter values
    --glob P      sweep every file matching glob P into a compact table — one row per (file,
                  record) of --type. P is a directory (every .rpt under it, recursively),
                  '<dir>/**/*.rpt' (same), or 'dir/*.rpt' (that one directory; * and ? apply to the
                  filename). Combines with positional files. Pairs with --json. Requires --type
    --cols SPEC   comma-separated sweep columns (implies a table even for one file). Each SPEC is:
                    N | 0xN[:TYPE]   absolute offset into the joined runs (default TYPE u8)
                    used             the anchor position (where the anchoring LP-string ends)
                    used±N[:TYPE]    a scalar N bytes from the anchor (the field-ref tail)
                    str | strN       the anchoring LP-string's text, or the Nth string (classify by
                                     a marker). TYPE ∈ u8,u16le,u16be,u32le,u32be
    --anchor-string TXT
                  anchor `used` at the end of the first LP-string containing TXT (case-insensitive).
                  Without it, `used` is the end of the last LP-string in the hex
    --offset O    raw escape hatch: dump --len bytes of the stream's logical buffer at byte offset O
                  (hex 0x… or decimal), verbatim (masked, as stored)
    --len L       byte count for --offset
    --probe N     cap the scalar-probe grid at N bytes (default 64); 0 disables it, `all` probes
                  every byte shown. With an anchor, grid rows are annotated used±N
    --grid        show the scalar-probe grid even for a record type that has a field table (which
                  otherwise replaces it)
    --whole       dump the record's whole on-disk span (header + children, still masked) instead of
                  only its own demasked field bytes
    --saved       inspect the saved-data batches (decoded schema, batch directory, and each
                  batch's derived decrypt IV + whether it inflates; a failing batch reports its
                  directory + derived-IV metadata). The instrument for inspecting a saved-data batch
                  class — see also `rpt saved` (decoded rows)
    --json        emit the dump (metadata, hex, strings, scalar grid; or the sweep table) as JSON

EXAMPLES:
    rpt dump report.rpt                             # what record types are in Contents?
    rpt dump report.rpt --type Formula --nth 0      # hex+annotations of the first 0x0076
    rpt dump report.rpt --type 0x0088               # field-table reading of a GroupAreaFormat
    rpt dump report.rpt --type 0x0121 --whole       # a chart-styling blob, whole masked span
    rpt dump base.rpt variant.rpt --type 0x0121     # minimal-pair diff: which bytes changed?
    rpt dump report.rpt --stream qe --type 0x8005   # a QESession record
    rpt dump report.rpt --stream ReportParametersStream --type CurrentValueRecord
                                                    # a parameter's saved last-used value
    rpt dump --glob 'parking/*.rpt' --type 0x011c   # which reports carry a 0x011c, and how many?
    rpt dump --glob 'parking/*.rpt' --type 0x011c --nth 0 --cols used,used+2,used+3,4:u16le
                                                    # tabulate the field-ref tail across the corpus
    rpt dump report.rpt --type 0x011c --anchor-string field_name   # probe grid read as used±N
";

/// Options for the `dump` subcommand, bundled so the dispatch site stays readable.
#[derive(Debug)]
pub(crate) struct DumpOpts {
    /// Record-type selector (hex `0x76`/`76` or a registry name); `None` prints the type index.
    pub ty: Option<String>,
    /// Stream selector: `contents` (default), `qe`, `all`, or a stream-id substring.
    pub stream: Option<String>,
    /// Dump only this match (0-based, pre-order across the selected streams).
    pub nth: Option<usize>,
    /// Raw escape hatch: byte offset into the stream's logical buffer (with `len`).
    pub offset: Option<String>,
    /// Byte count for `offset`.
    pub len: Option<String>,
    /// Scalar-probe cap: `None` = default 64, `Some("all")` = every byte shown, `Some("0")` = off.
    pub probe: Option<String>,
    /// Corpus-sweep glob (`dir/*.rpt`): dump one row per `(file, record)` across every match into a
    /// compact table instead of per-record hex. Expanded into the file list before dispatch.
    pub glob: Option<String>,
    /// Sweep-table column specs (comma-separated): each is an absolute offset (`4`, `0x1c:u16le`), an
    /// anchor-relative offset (`used+2`, `used-1:u16be`), the anchor position (`used`), or an
    /// LP-string (`str`, `str1`). Drives the table; also usable on a single file as an extract.
    pub cols: Option<String>,
    /// Anchor the `used` offset (and `used±N` columns / probe labels) at the end of the first
    /// LP-string whose text contains this marker (case-insensitive). Without it, `used` is the end of
    /// the last LP-string shown.
    pub anchor_string: Option<String>,
    /// Dump the whole on-disk span (masked) instead of the record's own demasked field bytes.
    pub whole: bool,
    /// Force the scalar-probe grid for a record type that has a field table (whose reading would
    /// otherwise replace it).
    pub grid: bool,
    /// Inspect the saved-data batches (schema, batch directory, per-batch IV + decrypt).
    pub saved: bool,
    pub json: bool,
    pub color: bool,
}

/// A single record selected for dumping, with its position, byte spans, and the two byte views:
/// `joined_runs` (the record's own field-byte runs, demasked and concatenated) and `whole` (the
/// verbatim on-disk span including header + children, still masked).
struct DumpMatch {
    stream: String,
    /// The record vocabulary the stream is written in — what every type number in this match
    /// (its own, its path, its children) is named and read under.
    dialect: Dialect,
    rtype: u16,
    schema: u16,
    offset: usize,
    content_start: usize,
    content_end: usize,
    mask: u8,
    depth: usize,
    /// Ancestor record types, root-most first (the record's structural path).
    path: Vec<u16>,
    /// Immediate child record types.
    children: Vec<u16>,
    joined_runs: Vec<u8>,
    /// The length of each field-byte run that went into `joined_runs`, in wire order. More than one
    /// means the buffer joins bytes the file does not put next to each other.
    run_lengths: Vec<usize>,
    whole: Vec<u8>,
    /// The record type's declarative field table applied to this record, when it has one. Its
    /// presence is what selects the field-table view over the scalar-probe grid.
    table: Option<RecordFields>,
}

/// The `dump` subcommand.
pub(crate) fn dump(files: &[String], opts: &DumpOpts) -> Result<(), CliError> {
    // Combine positional files with any `--glob` expansion (corpus sweep) and shadow `files`, so
    // every mode below sees the full set.
    let glob_expanded: Vec<String>;
    let files: &[String] = if let Some(g) = &opts.glob {
        let mut v = files.to_vec();
        v.extend(expand_glob(g));
        if v.is_empty() {
            return Err(bad_arg("--glob (matched no files)"));
        }
        glob_expanded = v;
        &glob_expanded
    } else {
        files
    };

    // Saved-data batch view (schema + batch directory + per-batch IV + decrypt attempt).
    if opts.saved {
        return dump_saved(files, opts);
    }

    // Corpus sweep: `--glob` and/or `--cols` collapse the per-record hex into one row per
    // (file, record) across every file. Requires a record type.
    if opts.glob.is_some() || opts.cols.is_some() {
        let sel = opts
            .ty
            .as_deref()
            .ok_or_else(|| bad_arg("--type (required with --glob/--cols)"))?;
        let want = parse_type_selector(sel).ok_or_else(|| bad_arg("--type"))?;
        return dump_sweep(files, opts, want);
    }

    // Escape hatch: --offset/--len dumps a raw span of the first selected stream, verbatim.
    if let (Some(off), Some(len)) = (&opts.offset, &opts.len) {
        let off = parse_num(off).ok_or_else(|| bad_arg("--offset"))?;
        let len = parse_num(len).ok_or_else(|| bad_arg("--len"))?;
        return dump_raw_span(files, opts, off, len);
    }

    // No --type: print the record-type index (table of contents) for each file.
    let Some(sel) = &opts.ty else {
        for file in files {
            let rpt = Rpt::open(file)?;
            let mut out = String::new();
            let _ = writeln!(out, "{file}:");
            dump_type_index(&mut out, &rpt, opts);
            print!("{out}");
        }
        return Ok(());
    };
    let want = parse_type_selector(sel).ok_or_else(|| bad_arg("--type"))?;

    // Gather matches per file (kept for the cross-file diff below).
    let per_file: Vec<(String, Vec<DumpMatch>)> = files
        .iter()
        .map(|f| Rpt::open(f).map(|rpt| (f.clone(), gather(&rpt, opts, want))))
        .collect::<rpt_reader::Result<_>>()?;

    if opts.json {
        let out = build_dump_json(&per_file, opts);
        print_json(&out);
        return Ok(());
    }

    // The heading names the type before any record is in hand, so it takes the selector's dialect;
    // once a record is found, its own stream's is authoritative.
    let selected = selector_dialect(opts.stream.as_deref());
    let mut out = String::new();
    for (file, matches) in &per_file {
        let dialect = matches.first().map_or(selected, |m| m.dialect);
        let _ = writeln!(
            out,
            "{file}: {} record(s) of type {}",
            matches.len(),
            type_label(want, dialect)
        );
        let total = matches.len();
        for (i, m) in matches.iter().enumerate() {
            render_match(&mut out, m, i, total, opts);
        }
    }

    // Two or more files → minimal-pair diff of the first match in each against the first file's.
    if per_file.len() >= 2 {
        let base = per_file[0].1.first();
        let _ = writeln!(
            out,
            "=== DIFF (first match of each vs {}) ===",
            per_file[0].0
        );
        for (file, matches) in &per_file[1..] {
            match (base, matches.first()) {
                (Some(a), Some(b)) => {
                    let (av, bv) = if opts.whole {
                        (a.whole.as_slice(), b.whole.as_slice())
                    } else {
                        (a.joined_runs.as_slice(), b.joined_runs.as_slice())
                    };
                    diff_matches(&mut out, (&per_file[0].0, av), (file, bv));
                }
                _ => {
                    let _ = writeln!(out, "{file}: missing a match to diff");
                }
            }
        }
    }
    print!("{out}");
    Ok(())
}

/// A usage error for a malformed `dump` option value.
fn bad_arg(what: &str) -> CliError {
    CliError::usage(format!("invalid value for {what}"))
}
