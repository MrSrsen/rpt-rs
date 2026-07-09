//! The multi-report views: the minimal-pair byte diff of the first match across files, and the
//! corpus-sweep table (`--glob`/`--cols`) that pulls a derived value from a record type across a
//! whole directory into one table (with the glob expansion and anchor-relative column extraction).

use std::fmt::Write as _;

use rpt::Rpt;
use serde::Serialize;

use crate::util::{paint, print_json, BOLD};

use super::collect::gather;
use super::parse::{anchor_string_index, lp_strings, parse_cols, type_label, Col};
use super::{bad_arg, DumpMatch, DumpOpts};

/// A byte-aligned diff of the first match's dumped bytes across files — the minimal-pair workflow.
/// Detects a whole-record modal XOR delta (a mask shift from a differing record sequence before
/// this record, only relevant to `--whole`); genuine changes are the deviations from it.
pub(super) fn diff_matches(out: &mut String, base: (&str, &[u8]), other: (&str, &[u8])) {
    let (name_a, a) = base;
    let (name_b, b) = other;
    let _ = writeln!(
        out,
        "-- {name_a} ({} B) vs {name_b} ({} B)",
        a.len(),
        b.len()
    );
    if a.len() != b.len() {
        let _ = writeln!(
            out,
            "   ! lengths differ by {} byte(s) — alignment past the first change is unreliable",
            (a.len() as isize - b.len() as isize).abs()
        );
    }
    let n = a.len().min(b.len());
    let mut hist = [0usize; 256];
    for i in 0..n {
        hist[(a[i] ^ b[i]) as usize] += 1;
    }
    let m = (0..256).max_by_key(|&d| hist[d]).unwrap_or(0) as u8;
    if m != 0 {
        let _ = writeln!(
            out,
            "   mask shifted by 0x{m:02x} ({}/{n} bytes match the shift) — real changes below:",
            hist[m as usize]
        );
    }
    let mut diffs = 0;
    for i in 0..n {
        if (a[i] ^ b[i]) != m {
            diffs += 1;
            let _ = writeln!(out, "   @0x{i:04x} ({i:>4}): {:02x} -> {:02x}", a[i], b[i]);
        }
    }
    match (diffs, m) {
        (0, 0) => {
            let _ = writeln!(out, "   (identical)");
        }
        (0, _) => {
            let _ = writeln!(out, "   (identical apart from the mask shift)");
        }
        _ => {
            let _ = writeln!(out, "   {diffs} genuine byte change(s)");
        }
    }
}

/// Render one column's cell for a record. `anchor` is the resolved anchor end-offset (if any) and
/// `aidx` the anchoring string's index (for the `str` anchoring-text column).
fn cell(
    col: &Col,
    leaf: &[u8],
    anchor: Option<usize>,
    strings: &[(usize, String, usize)],
    aidx: Option<usize>,
) -> String {
    match col {
        Col::Anchor => anchor.map_or_else(|| "-".into(), |a| a.to_string()),
        Col::Str(idx) => {
            let pick = match idx {
                Some(k) => strings.get(*k),
                None => aidx.and_then(|k| strings.get(k)),
            };
            pick.map_or_else(|| "-".into(), |(_, t, _)| t.clone())
        }
        Col::Scalar {
            anchored,
            off,
            width,
        } => {
            let base = if *anchored {
                match anchor {
                    Some(a) => a as isize,
                    None => return "-".into(),
                }
            } else {
                0
            };
            let pos = base + off;
            if pos < 0 {
                return "-".into();
            }
            width.read(leaf, pos as usize).map_or_else(
                || "-".into(),
                |v| format!("0x{:0width$x}", v, width = width.digits()),
            )
        }
    }
}

/// The bytes a column reads from a match, honoring `--whole`.
fn match_bytes<'a>(m: &'a DumpMatch, opts: &DumpOpts) -> &'a [u8] {
    if opts.whole {
        &m.whole
    } else {
        &m.leaf
    }
}

/// Expand a glob into matching paths. `*` and `?` apply to the final path component; the directory
/// part is literal. Dependency-free — enough for the `dir/*.rpt` corpus-sweep workflow.
pub(super) fn expand_glob(pattern: &str) -> Vec<String> {
    let path = std::path::Path::new(pattern);
    let Some(pat) = path.file_name().and_then(|f| f.to_str()) else {
        return Vec::new();
    };
    if !pat.contains('*') && !pat.contains('?') {
        return if path.exists() {
            vec![pattern.to_string()]
        } else {
            Vec::new()
        };
    }
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if wildcard_match(pat, name) {
                    out.push(e.path().to_string_lossy().into_owned());
                }
            }
        }
    }
    out.sort();
    out
}

/// Glob-match one path component: `*` matches any run (including empty), `?` exactly one char.
/// Linear-time backtracking (the standard two-pointer algorithm).
fn wildcard_match(pat: &str, name: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// A file's basename for the sweep table (falls back to the whole path).
fn basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(path)
        .to_string()
}

/// One sweep-table row: a matched record, or a per-file placeholder (no match / open error).
struct SweepRow {
    file: String,
    rec: Option<usize>,
    note: Option<String>,
    cells: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SweepRowJson {
    file: String,
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    rec: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    cells: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SweepJson {
    #[serde(rename = "type")]
    type_name: String,
    columns: Vec<String>,
    rows: Vec<SweepRowJson>,
}

/// Build the sweep rows for one file: one row per match (respecting `--nth`), or a single
/// placeholder row when the file has no match or fails to open.
fn sweep_file(file: &str, opts: &DumpOpts, want: u16, cols: &[(String, Col)]) -> Vec<SweepRow> {
    let base = basename(file);
    let dashes = || vec!["-".to_string(); cols.len()];
    let rpt = match Rpt::open(file) {
        Ok(r) => r,
        Err(e) => {
            return vec![SweepRow {
                file: base,
                rec: None,
                note: Some(format!("(open error: {e})")),
                cells: dashes(),
            }]
        }
    };
    let matches = gather(&rpt, opts, want);
    if matches.is_empty() {
        return vec![SweepRow {
            file: base,
            rec: None,
            note: Some("(no match)".into()),
            cells: dashes(),
        }];
    }
    let needle = opts.anchor_string.as_deref();
    matches
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let bytes = match_bytes(m, opts);
            let strings = lp_strings(bytes);
            let aidx = anchor_string_index(&strings, needle);
            let anchor = aidx.map(|k| strings[k].0 + strings[k].2);
            SweepRow {
                file: base.clone(),
                rec: Some(i),
                note: None,
                cells: cols
                    .iter()
                    .map(|(_, c)| cell(c, bytes, anchor, &strings, aidx))
                    .collect(),
            }
        })
        .collect()
}

/// The corpus-sweep table: one row per `(file, record)` across `files`, extracting `--cols`. With
/// no `--cols`, a coverage table (one row per file: how many records of the type it holds).
pub(super) fn dump_sweep(files: &[String], opts: &DumpOpts, want: u16) -> rpt::Result<()> {
    let cols = match opts.cols.as_deref() {
        Some(spec) => parse_cols(spec).ok_or_else(|| bad_arg("--cols"))?,
        None => Vec::new(),
    };

    // Coverage mode: no columns → just the per-file match count (which reports carry the record).
    if cols.is_empty() {
        let rows: Vec<SweepRow> = files
            .iter()
            .map(|f| {
                let base = basename(f);
                match Rpt::open(f) {
                    Ok(rpt) => SweepRow {
                        file: base,
                        rec: None,
                        note: None,
                        cells: vec![gather(&rpt, opts, want).len().to_string()],
                    },
                    Err(e) => SweepRow {
                        file: base,
                        rec: None,
                        note: Some(format!("(open error: {e})")),
                        cells: vec!["-".into()],
                    },
                }
            })
            .collect();
        return render_sweep(opts, want, &["matches".to_string()], rows, false);
    }

    let headers: Vec<String> = cols.iter().map(|(h, _)| h.clone()).collect();
    let rows: Vec<SweepRow> = files
        .iter()
        .flat_map(|f| sweep_file(f, opts, want, &cols))
        .collect();
    render_sweep(opts, want, &headers, rows, true)
}

/// Emit the sweep table as aligned text (or `--json`). `with_rec` prints the per-file match index
/// column (off for the coverage table, which has one row per file).
fn render_sweep(
    opts: &DumpOpts,
    want: u16,
    headers: &[String],
    rows: Vec<SweepRow>,
    with_rec: bool,
) -> rpt::Result<()> {
    if opts.json {
        let json = SweepJson {
            type_name: type_label(want),
            columns: headers.to_vec(),
            rows: rows
                .into_iter()
                .map(|r| SweepRowJson {
                    file: r.file,
                    rec: r.rec,
                    note: r.note,
                    cells: r.cells,
                })
                .collect(),
        };
        print_json(&json);
        return Ok(());
    }

    // Column widths: file, [rec], each data column — each at least its header width.
    let file_w = rows
        .iter()
        .map(|r| r.file.len())
        .chain([4])
        .max()
        .unwrap_or(4);
    let rec_w = 3usize;
    let mut col_w: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for r in &rows {
        for (i, c) in r.cells.iter().enumerate() {
            col_w[i] = col_w[i].max(c.len());
        }
    }
    let mut out = String::new();
    let heading = format!(
        "── sweep · {} · {} file-row(s) ──",
        type_label(want),
        rows.len()
    );
    let _ = writeln!(out, "{}", paint(opts.color, BOLD, &heading));
    // Header line.
    let _ = write!(out, "{:<file_w$}", "file");
    if with_rec {
        let _ = write!(out, "  {:>rec_w$}", "rec");
    }
    for (i, h) in headers.iter().enumerate() {
        let _ = write!(out, "  {:>w$}", h, w = col_w[i]);
    }
    let _ = writeln!(out);
    // Rows.
    for r in &rows {
        let _ = write!(out, "{:<file_w$}", r.file);
        if with_rec {
            match r.rec {
                Some(n) => {
                    let _ = write!(out, "  {n:>rec_w$}");
                }
                None => {
                    let _ = write!(out, "  {:>rec_w$}", "-");
                }
            }
        }
        for (i, c) in r.cells.iter().enumerate() {
            let _ = write!(out, "  {:>w$}", c, w = col_w[i]);
        }
        if let Some(note) = &r.note {
            let _ = write!(out, "  {note}");
        }
        let _ = writeln!(out);
    }
    print!("{out}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dump::parse::Width;

    #[test]
    fn wildcard_matches_star_question_and_literal() {
        assert!(wildcard_match("*.rpt", "chart_stock.rpt"));
        assert!(wildcard_match("chart_*.rpt", "chart_stock.rpt"));
        assert!(wildcard_match("orders_?d.rpt", "orders_3d.rpt"));
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("a*b*c", "axxbyyc"));
        assert!(!wildcard_match("chart_*.rpt", "orders.rpt"));
        assert!(!wildcard_match("orders_?d.rpt", "orders_3dd.rpt"));
        assert!(!wildcard_match("*.rpt", "report.xml"));
    }

    #[test]
    fn cell_absolute_anchored_and_string() {
        // leaf: [len=3]"id"\0 then tail 04 00 05 06
        let leaf = [0, 0, 0, 3, b'i', b'd', 0, 0x04, 0x00, 0x05, 0x06];
        let strings = lp_strings(&leaf);
        let aidx = anchor_string_index(&strings, None);
        let anchor = aidx.map(|k| strings[k].0 + strings[k].2); // = 7
        assert_eq!(anchor, Some(7));
        // absolute u8 at 7
        assert_eq!(
            cell(
                &Col::Scalar {
                    anchored: false,
                    off: 7,
                    width: Width::U8
                },
                &leaf,
                anchor,
                &strings,
                aidx
            ),
            "0x04"
        );
        // anchored used+0 == absolute 7
        assert_eq!(
            cell(
                &Col::Scalar {
                    anchored: true,
                    off: 0,
                    width: Width::U8
                },
                &leaf,
                anchor,
                &strings,
                aidx
            ),
            "0x04"
        );
        // anchored used+2 u16be = bytes[9..11] = 05 06
        assert_eq!(
            cell(
                &Col::Scalar {
                    anchored: true,
                    off: 2,
                    width: Width::U16be
                },
                &leaf,
                anchor,
                &strings,
                aidx
            ),
            "0x0506"
        );
        // anchor position column
        assert_eq!(cell(&Col::Anchor, &leaf, anchor, &strings, aidx), "7");
        // anchoring string text
        assert_eq!(cell(&Col::Str(None), &leaf, anchor, &strings, aidx), "id");
        // out of range -> dash
        assert_eq!(
            cell(
                &Col::Scalar {
                    anchored: false,
                    off: 999,
                    width: Width::U8
                },
                &leaf,
                anchor,
                &strings,
                aidx
            ),
            "-"
        );
        // anchored but no anchor -> dash
        assert_eq!(
            cell(
                &Col::Scalar {
                    anchored: true,
                    off: 0,
                    width: Width::U8
                },
                &leaf,
                None,
                &strings,
                aidx
            ),
            "-"
        );
    }
}
