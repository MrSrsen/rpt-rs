//! Round-trip and same-size patch tests for the `.rpt` writer (uy6.13 / uy6.14 / uy6.15).
//!
//! The write pipeline is the inverse of decode: retained logical record bytes → deflate → AES-CFB
//! encrypt → CFB rewrite. Deflate is non-canonical, so a re-encode is byte-identical only at the
//! **inflated** (logical) level, never at the file level — every assertion here is on the decoded
//! logical bytes, not the raw file. Fixtures are the committed synthetic reports; a missing fixture
//! skips so the suite stays green on a bare checkout.

use rpt::raw::RecordTag;
use rpt::{Rpt, StreamId};
use std::path::{Path, PathBuf};

/// The three committed synthetic fixtures the writer is exercised over.
const SYNTHETIC: [&str; 3] = [
    "synthetic/blank_report.rpt",
    "synthetic/chart_baseline.rpt",
    "synthetic/single_group.rpt",
];

fn fixture(rel: &str) -> PathBuf {
    rpt_test_support::fixture(Path::new("tests/fixtures/reports").join(rel))
}

fn open(rel: &str) -> Option<Rpt> {
    Rpt::open(fixture(rel)).ok()
}

/// The Contents stream's logical (inflated) record bytes.
fn contents_logical(rpt: &Rpt) -> Vec<u8> {
    rpt.stream(&StreamId::Contents)
        .expect("Contents stream")
        .logical_bytes()
        .to_vec()
}

/// uy6.13 — the re-serializable raw record tree reconstructs the logical stream byte-for-byte for
/// every synthetic fixture (the tree's node spans partition the inflated record bytes exactly).
#[test]
fn serialize_tree_reconstructs_logical() {
    let mut ran = false;
    for rel in SYNTHETIC {
        let Some(rpt) = open(rel) else {
            eprintln!("[skip] {rel} absent");
            continue;
        };
        ran = true;
        let contents = rpt.stream(&StreamId::Contents).expect("Contents stream");
        assert_eq!(
            contents.serialize_tree(),
            contents.logical_bytes(),
            "{rel}: serialize_tree must reconstruct the logical record stream"
        );
    }
    if !ran {
        eprintln!("[skip] synthetic fixtures absent");
    }
}

/// uy6.14 — a NO-OP re-encode re-opens to byte-identical logical bytes for every synthetic fixture.
/// The whole inverse pipeline runs (deflate → encrypt → CFB rewrite), and the report still opens.
#[test]
fn noop_reencode_round_trips_logical() {
    let mut ran = false;
    for rel in SYNTHETIC {
        let Some(rpt) = open(rel) else {
            eprintln!("[skip] {rel} absent");
            continue;
        };
        ran = true;
        let before = contents_logical(&rpt);
        let file = rpt.reencode().expect("reencode");
        let reopened = Rpt::read(std::io::Cursor::new(file)).expect("re-open re-encoded report");
        assert_eq!(
            contents_logical(&reopened),
            before,
            "{rel}: no-op re-encode must round-trip the logical record stream"
        );
    }
    if !ran {
        eprintln!("[skip] synthetic fixtures absent");
    }
}

/// uy6.14 — an identity patch (overwrite a region with its current bytes) also round-trips the
/// logical bytes exactly, confirming the re-mask on the patch path is a true inverse of the demask.
#[test]
fn identity_patch_round_trips_logical() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    let before = contents_logical(&rpt);
    let (leaf_off, orig) = first_section_name(&rpt);
    let file = rpt
        .patch_record_leaf(RecordTag(SECTION_RECORD), 0, leaf_off, &orig)
        .expect("identity patch");
    let reopened = Rpt::read(std::io::Cursor::new(file)).expect("re-open");
    assert_eq!(
        contents_logical(&reopened),
        before,
        "an identity patch must be a no-op at the logical level"
    );
}

/// uy6.15 — a same-size patch of a Section-name record changes exactly that decoded string and
/// leaves every other logical byte untouched, and the re-encoded report re-opens cleanly.
#[test]
fn same_size_patch_changes_only_the_target() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    let before = contents_logical(&rpt);
    let (leaf_off, orig) = first_section_name(&rpt);

    // A different, equal-length name (rotate each ASCII letter by one, keeping the byte count).
    let replacement: Vec<u8> = orig
        .iter()
        .map(|&b| if b == b'Z' { b'A' } else { b + 1 })
        .collect();
    assert_eq!(replacement.len(), orig.len());
    assert_ne!(replacement, orig);

    let file = rpt
        .patch_record_leaf(RecordTag(SECTION_RECORD), 0, leaf_off, &replacement)
        .expect("same-size patch");
    let reopened = Rpt::read(std::io::Cursor::new(file)).expect("re-open patched report");
    let after = contents_logical(&reopened);

    // The stream is the same length; only the patched name bytes differ.
    assert_eq!(
        after.len(),
        before.len(),
        "same-size patch keeps stream length"
    );
    let diffs: Vec<usize> = before
        .iter()
        .zip(&after)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        diffs.len(),
        orig.len(),
        "exactly the patched region changed"
    );

    // The decoded Section name reflects the edit.
    let (_, patched_name) = first_section_name(&reopened);
    assert_eq!(patched_name, replacement, "decoded name reflects the patch");
    assert_ne!(patched_name, orig);
}

/// uy6.15 — the writer refuses an edit that would overrun the record's leaf (a length change).
#[test]
fn patch_rejects_out_of_bounds_region() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    let (leaf_off, orig) = first_section_name(&rpt);
    // Extend past the end of the section-name run by one byte.
    let too_long: Vec<u8> = orig.iter().copied().chain(std::iter::once(b'X')).collect();
    let far_off = leaf_off + 1_000_000;
    assert!(
        rpt.patch_record_leaf(RecordTag(SECTION_RECORD), 0, far_off, &orig)
            .is_err(),
        "an offset past the leaf must be rejected"
    );
    // A region that starts in-bounds but overruns the leaf end is also rejected.
    let node_leaf_len = section_leaf_len(&rpt);
    assert!(
        rpt.patch_record_leaf(RecordTag(SECTION_RECORD), 0, node_leaf_len - 1, &too_long)
            .is_err(),
        "a region overrunning the leaf must be rejected"
    );
}

/// uy6.15 — a target record that does not exist is an error, not a silent no-op.
#[test]
fn patch_rejects_missing_record() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    assert!(
        rpt.patch_record_leaf(RecordTag(0xDEAD), 0, 0, &[0])
            .is_err(),
        "an absent record type must be rejected"
    );
    // Present type, but not enough occurrences.
    assert!(
        rpt.patch_record_leaf(RecordTag(SECTION_RECORD), 100_000, 0, &[0])
            .is_err(),
        "an out-of-range occurrence index must be rejected"
    );
}

/// The first Section record's inner length-prefixed name: `(demasked leaf, lp_prefix_offset,
/// declared_len, name_bytes_incl_any_trailing_nul)`. The name is `[u32-BE len][len bytes]` and lives
/// after the 4-byte height, so the scan starts at leaf offset 4.
fn first_section_lp(rpt: &Rpt) -> (Vec<u8>, usize, usize, Vec<u8>) {
    let leaf = first_section_leaf(rpt);
    let mut i = 4;
    while i + 4 < leaf.len() {
        let len = u32::from_be_bytes([leaf[i], leaf[i + 1], leaf[i + 2], leaf[i + 3]]) as usize;
        if (2..=4096).contains(&len) && i + 4 + len <= leaf.len() {
            let text = &leaf[i + 4..i + 4 + len];
            // Printable run (a trailing NUL is allowed and kept in the byte count).
            let body = if text.last() == Some(&0) {
                &text[..len - 1]
            } else {
                text
            };
            if !body.is_empty() && body.iter().all(|&b| (0x20..0x7f).contains(&b)) {
                return (leaf.clone(), i, len, text.to_vec());
            }
        }
        i += 1;
    }
    panic!("no length-prefixed Section name found");
}

/// uy6.17 — a LONGER Section name: replace the inner length-prefixed name (its `u32-BE` length +
/// text) with a longer one, recomputing the record (and every ancestor) length prefix. The
/// re-encoded report re-opens, the stream grows by exactly the delta, and the decoder reads back the
/// new longer name.
#[test]
fn resize_grows_a_section_name() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    let before = contents_logical(&rpt);
    let (_, prefix_off, decl_len, name) = first_section_lp(&rpt);

    // Insert " LONGER" before any trailing NUL; rebuild the length-prefixed name.
    let had_nul = name.last() == Some(&0);
    let body = if had_nul {
        &name[..name.len() - 1]
    } else {
        &name[..]
    };
    let mut new_name: Vec<u8> = body.to_vec();
    new_name.extend_from_slice(b" LONGER");
    if had_nul {
        new_name.push(0);
    }
    let mut new_bytes = (new_name.len() as u32).to_be_bytes().to_vec();
    new_bytes.extend_from_slice(&new_name);

    let region = prefix_off..prefix_off + 4 + decl_len;
    let old_region_len = region.len();
    let file = rpt
        .patch_record_leaf_resize(RecordTag(SECTION_RECORD), 0, region, &new_bytes)
        .expect("resize (grow) a Section name");
    let reopened = Rpt::read(std::io::Cursor::new(file)).expect("re-open resized report");
    let after = contents_logical(&reopened);

    // The stream grew by exactly the size delta.
    let delta = new_bytes.len() as i64 - old_region_len as i64;
    assert!(delta > 0, "the new name must be longer");
    assert_eq!(
        after.len() as i64,
        before.len() as i64 + delta,
        "stream length changes by exactly the byte delta"
    );

    // The re-framed tree still parses, and the decoder reads the new (longer) name.
    let (_, _, _, patched) = first_section_lp(&reopened);
    let trim = |b: &[u8]| -> Vec<u8> { b.iter().copied().take_while(|&c| c != 0).collect() };
    assert_eq!(
        trim(&patched),
        trim(&new_name),
        "decoded name reflects the longer value"
    );
    assert_ne!(trim(&patched), trim(&name));
}

/// uy6.17 — a SHORTER Section name shrinks the stream by exactly the delta and re-reads correctly.
#[test]
fn resize_shrinks_a_section_name() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    let before = contents_logical(&rpt);
    let (_, prefix_off, decl_len, name) = first_section_lp(&rpt);
    // Need at least one droppable body byte (besides any trailing NUL).
    let had_nul = name.last() == Some(&0);
    let body_len = if had_nul { name.len() - 1 } else { name.len() };
    if body_len < 2 {
        eprintln!("[skip] section name too short to shrink");
        return;
    }
    // Drop the last body byte; keep any NUL.
    let mut new_name: Vec<u8> = name[..body_len - 1].to_vec();
    if had_nul {
        new_name.push(0);
    }
    let mut new_bytes = (new_name.len() as u32).to_be_bytes().to_vec();
    new_bytes.extend_from_slice(&new_name);
    let region = prefix_off..prefix_off + 4 + decl_len;
    let old_region_len = region.len();

    let file = rpt
        .patch_record_leaf_resize(RecordTag(SECTION_RECORD), 0, region, &new_bytes)
        .expect("resize (shrink) a Section name");
    let reopened = Rpt::read(std::io::Cursor::new(file)).expect("re-open shrunk report");
    let after = contents_logical(&reopened);
    let delta = new_bytes.len() as i64 - old_region_len as i64;
    assert!(delta < 0);
    assert_eq!(after.len() as i64, before.len() as i64 + delta);
    let (_, _, _, patched) = first_section_lp(&reopened);
    let trim = |b: &[u8]| -> Vec<u8> { b.iter().copied().take_while(|&c| c != 0).collect() };
    assert_eq!(trim(&patched), trim(&new_name));
}

/// uy6.17 — a same-length resize is byte-identical to a same-size patch (no length prefix changes).
#[test]
fn resize_same_length_matches_same_size_patch() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    let (leaf_off, orig) = first_section_name(&rpt);
    let replacement: Vec<u8> = orig
        .iter()
        .map(|&b| if b == b'Z' { b'A' } else { b + 1 })
        .collect();
    let via_resize = rpt
        .patch_record_leaf_resize(
            RecordTag(SECTION_RECORD),
            0,
            leaf_off..leaf_off + orig.len(),
            &replacement,
        )
        .expect("same-length resize");
    let via_patch = rpt
        .patch_record_leaf(RecordTag(SECTION_RECORD), 0, leaf_off, &replacement)
        .expect("same-size patch");
    let a = contents_logical(&Rpt::read(std::io::Cursor::new(via_resize)).unwrap());
    let b = contents_logical(&Rpt::read(std::io::Cursor::new(via_patch)).unwrap());
    assert_eq!(a, b, "a same-length resize equals the same-size patch");
}

/// uy6.17 — guard probes: a missing record, and a region that overruns the leaf, both `Err` with no
/// file produced.
#[test]
fn resize_guards_reject_bad_edits() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    // Absent record type.
    assert!(rpt
        .patch_record_leaf_resize(RecordTag(0xDEAD), 0, 0..1, &[0, 1, 2])
        .is_err());
    // Region overruns the leaf.
    let leaf_len = section_leaf_len(&rpt);
    assert!(rpt
        .patch_record_leaf_resize(RecordTag(SECTION_RECORD), 0, 0..leaf_len + 5, b"xyz")
        .is_err());
    // Region far past the leaf.
    assert!(rpt
        .patch_record_leaf_resize(RecordTag(SECTION_RECORD), 0, 1_000_000..1_000_001, b"z")
        .is_err());
}

/// Corpus round-trip: every `.rpt` that decodes must re-encode losslessly at the logical level.
///
/// Walks all `.rpt` files under `tests/fixtures/reports`, plus any extra corpus directories named in
/// the colon-separated `RPT_EXTRA_CORPUS` env var (so a local run can sweep a larger private corpus
/// without naming it in this committed file). For each report that opens, both writer paths must
/// reproduce the Contents logical (inflated) record stream byte-for-byte: `serialize_tree` (the raw
/// record tree) and a full no-op `reencode` re-opened from its bytes. Byte-identity is asserted at
/// the inflated level only — deflate is non-canonical, so the file bytes legitimately differ.
/// Reports that fail to *decode* are skipped (a decode-coverage concern, not a writer concern); the
/// test is that anything we can read, we can write back unchanged.
#[test]
fn corpus_reencode_round_trips_logical() {
    let mut roots: Vec<PathBuf> = vec![rpt_test_support::fixture("tests/fixtures/reports")];
    if let Ok(extra) = std::env::var("RPT_EXTRA_CORPUS") {
        roots.extend(
            extra
                .split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
        );
    }

    let mut rpts: Vec<PathBuf> = Vec::new();
    for root in &roots {
        collect_rpts(root, &mut rpts);
    }
    rpts.sort();

    let (mut opened, mut skipped) = (0usize, 0usize);
    for path in &rpts {
        let Ok(rpt) = Rpt::open(path) else {
            skipped += 1;
            continue;
        };
        opened += 1;
        let name = path.display();

        let Some(contents) = rpt.stream(&StreamId::Contents) else {
            continue;
        };
        let logical = contents.logical_bytes().to_vec();

        // Path 1: the record tree reconstructs the logical stream exactly.
        assert_eq!(
            contents.serialize_tree(),
            logical,
            "{name}: serialize_tree must reconstruct the logical record stream"
        );

        // Path 2: the full inverse pipeline re-opens to byte-identical logical bytes.
        let file = rpt
            .reencode()
            .unwrap_or_else(|e| panic!("{name}: reencode failed: {e}"));
        let reopened = Rpt::read(std::io::Cursor::new(file))
            .unwrap_or_else(|e| panic!("{name}: re-encoded report failed to re-open: {e}"));
        assert_eq!(
            contents_logical(&reopened),
            logical,
            "{name}: no-op re-encode must round-trip the logical record stream"
        );
    }

    eprintln!("[corpus round-trip] {opened} reports round-tripped, {skipped} un-decodable skipped");
    if opened == 0 {
        eprintln!("[skip] no decodable .rpt fixtures found");
    }
}

/// Recursively collect every `.rpt` file under `dir` (silently ignores a missing directory).
fn collect_rpts(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rpts(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("rpt"))
        {
            out.push(path);
        }
    }
}

/// The Section record tag (`0x008c`) — a leaf record whose name is an ASCII run in its leaf.
const SECTION_RECORD: u16 = 0x008c;

/// The `(leaf_offset, bytes)` of the first Section record's name — the first alphabetic ASCII run in
/// the first `0x008c` record's demasked leaf.
fn first_section_name(rpt: &Rpt) -> (usize, Vec<u8>) {
    let leaf = first_section_leaf(rpt);
    let start = leaf
        .iter()
        .position(|&b| b.is_ascii_alphabetic())
        .expect("section leaf has an ASCII name");
    let end = leaf[start..]
        .iter()
        .position(|&b| !(0x20..0x7f).contains(&b))
        .map(|e| start + e)
        .unwrap_or(leaf.len());
    (start, leaf[start..end].to_vec())
}

fn section_leaf_len(rpt: &Rpt) -> usize {
    first_section_leaf(rpt).len()
}

/// The demasked leaf bytes of the first (pre-order) `0x008c` Section record.
fn first_section_leaf(rpt: &Rpt) -> Vec<u8> {
    let contents = rpt.stream(&StreamId::Contents).expect("Contents stream");
    let logical = contents.logical_bytes();
    let mut found: Option<Vec<u8>> = None;
    for root in contents.record_tree() {
        root.walk(&mut |n| {
            if found.is_none() && n.rtype == SECTION_RECORD {
                found = Some(n.leaf_bytes(logical));
            }
        });
        if found.is_some() {
            break;
        }
    }
    found.expect("fixture has a Section record")
}
