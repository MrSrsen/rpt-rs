//! Round-trip and same-size patch tests for the `.rpt` writer.
//!
//! The write pipeline is the inverse of decode: retained logical record bytes → deflate → AES-CFB
//! encrypt → CFB rewrite. Deflate is non-canonical, so a re-encode is byte-identical only at the
//! **inflated** (logical) level, never at the file level — every assertion here is on the decoded
//! logical bytes, not the raw file. Fixtures are the committed synthetic reports; a missing fixture
//! skips so the suite stays green on a bare checkout.

use rpt_reader::raw::RecordTag;
use rpt_reader::{EditPolicy, Rpt, StreamId};
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

/// Where two byte strings first diverge, and how, as a line short enough to read.
///
/// A corpus-wide assertion on whole streams would otherwise report a failure by printing two
/// multi-megabyte buffers, which says only *that* they differ.
fn divergence(got: &[u8], want: &[u8]) -> Option<String> {
    if got == want {
        return None;
    }
    let at = got
        .iter()
        .zip(want)
        .position(|(a, b)| a != b)
        .unwrap_or(got.len().min(want.len()));
    Some(format!(
        "first differs at byte {at} (re-serialized {:02x?}, stream {:02x?}); lengths {} vs {}",
        got.get(at..(at + 8).min(got.len())).unwrap_or_default(),
        want.get(at..(at + 8).min(want.len())).unwrap_or_default(),
        got.len(),
        want.len(),
    ))
}

/// The re-serializable raw record tree reconstructs the logical stream byte-for-byte, for every
/// stream of every corpus report.
///
/// The tree's node spans must partition the inflated record bytes exactly — children partitioning
/// their parent's content, spans ascending, each lying inside its parent. Nothing in the types says
/// so; it is a runtime property of the span arithmetic, and a violation produces different bytes
/// silently. Its only guard is therefore how many trees it is checked against, which is why the
/// sweep is the corpus rather than a fixture list.
///
/// Every stream is walked, not only `Contents`: the same scan builds the query-engine,
/// parameter-value and saved-catalog trees, and a stream whose bytes are never framed as a tree at
/// all still has to come back whole.
#[test]
fn serialize_tree_reconstructs_logical() {
    let reports = rpt_test_support::corpus_reports();
    let (mut streams, mut with_records) = (0usize, 0usize);
    let mut diverged = Vec::new();

    for path in &reports {
        let name = path.display();
        let rpt = Rpt::open(path).unwrap_or_else(|e| panic!("{name}: does not open: {e}"));
        for (id, stream) in rpt.streams() {
            streams += 1;
            if !stream.record_tree().is_empty() {
                with_records += 1;
            }
            if let Some(how) = divergence(&stream.serialize_tree(), stream.logical_bytes()) {
                diverged.push(format!("  {name} [{id:?}]: {how}"));
            }
        }
    }

    eprintln!(
        "[tree round-trip] {with_records} of {streams} stream(s) across {} report(s) parse to a record tree",
        reports.len()
    );
    assert!(
        diverged.is_empty(),
        "{} stream(s) do not re-serialize to their logical bytes:\n{}",
        diverged.len(),
        diverged.join("\n")
    );
    // A sweep that framed no records would pass on emptiness alone.
    assert!(with_records > 0, "no stream parsed to a record tree");
}

/// A NO-OP re-encode re-opens to byte-identical logical bytes for every synthetic fixture.
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

/// An identity patch (overwrite a region with its current bytes) also round-trips the
/// logical bytes exactly, confirming the re-mask on the patch path is a true inverse of the demask.
#[test]
fn identity_patch_round_trips_logical() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    let before = contents_logical(&rpt);
    let (field_off, orig) = first_section_name(&rpt);
    let file = rpt
        .patch_record_bytes_with(RecordTag(SECTION_RECORD), 0, field_off, &orig, FORCED)
        .expect("identity patch");
    let reopened = Rpt::read(std::io::Cursor::new(file)).expect("re-open");
    assert_eq!(
        contents_logical(&reopened),
        before,
        "an identity patch must be a no-op at the logical level"
    );
}

/// A same-size patch of a Section-name record changes exactly that decoded string and
/// leaves every other logical byte untouched, and the re-encoded report re-opens cleanly.
#[test]
fn same_size_patch_changes_only_the_target() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    let before = contents_logical(&rpt);
    let (field_off, orig) = first_section_name(&rpt);

    // A different, equal-length name (rotate each ASCII letter by one, keeping the byte count).
    let replacement: Vec<u8> = orig
        .iter()
        .map(|&b| if b == b'Z' { b'A' } else { b + 1 })
        .collect();
    assert_eq!(replacement.len(), orig.len());
    assert_ne!(replacement, orig);

    let file = rpt
        .patch_record_bytes_with(
            RecordTag(SECTION_RECORD),
            0,
            field_off,
            &replacement,
            FORCED,
        )
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

/// A patch offset is a position in the record's **joined runs**, not in its content: on a record
/// that nests another one, an offset past the first run must land in the second run's file bytes,
/// on the far side of the nested record.
///
/// Read as a content offset instead, the same edit would write into the nested record's framing —
/// the two coordinates differ by exactly the nested record's framed length.
#[test]
fn a_patch_offset_past_a_child_lands_in_the_next_run() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    let before = contents_logical(&rpt);
    let contents = rpt.stream(&StreamId::Contents).expect("Contents stream");
    let logical = contents.logical_bytes();

    let mut target = None;
    for root in contents.record_tree() {
        root.walk(&mut |n| {
            if target.is_none() && n.rtype == GROUP_OPTIONS_RECORD && !n.children.is_empty() {
                target = Some(n.clone());
            }
        });
    }
    let node = target.expect("fixture has a group-options record with a nested one");
    let first_run_len = node.runs(logical).next().expect("a first run").len();
    let second_run_start = node.children[0].content_end;
    assert!(
        node.content_start + first_run_len < second_run_start,
        "the nested record occupies bytes between the two runs"
    );

    // The first byte of the second run, addressed in the joined coordinate.
    let at = first_run_len;
    let old = node.joined_runs(logical)[at];
    let new = old ^ 0x5a;

    let file = rpt
        .patch_record_bytes_with(RecordTag(GROUP_OPTIONS_RECORD), 0, at, &[new], FORCED)
        .expect("patch past the seam");
    let reopened = Rpt::read(std::io::Cursor::new(file)).expect("re-open patched report");
    let after = contents_logical(&reopened);

    let diffs: Vec<usize> = before
        .iter()
        .zip(&after)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        diffs,
        [second_run_start],
        "the second run's first byte moved"
    );
    assert_ne!(
        diffs[0],
        node.content_start + at,
        "a content offset would land inside the nested record"
    );
}

/// The writer refuses an edit that would overrun the record's field bytes (a length change).
#[test]
fn patch_rejects_out_of_bounds_region() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    let (field_off, orig) = first_section_name(&rpt);
    // Extend past the end of the section-name run by one byte.
    let too_long: Vec<u8> = orig.iter().copied().chain(std::iter::once(b'X')).collect();
    let far_off = field_off + 1_000_000;
    assert!(
        rpt.patch_record_bytes_with(RecordTag(SECTION_RECORD), 0, far_off, &orig, FORCED)
            .is_err(),
        "an offset past the field bytes must be rejected"
    );
    // A region that starts in-bounds but overruns the field bytes is also rejected.
    let field_bytes_len = section_bytes_len(&rpt);
    assert!(
        rpt.patch_record_bytes_with(
            RecordTag(SECTION_RECORD),
            0,
            field_bytes_len - 1,
            &too_long,
            FORCED
        )
        .is_err(),
        "a region overrunning the field bytes must be rejected"
    );
}

/// A target record that does not exist is an error, not a silent no-op.
#[test]
fn patch_rejects_missing_record() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    assert!(
        rpt.patch_record_bytes_with(RecordTag(0xDEAD), 0, 0, &[0], FORCED)
            .is_err(),
        "an absent record type must be rejected"
    );
    // Present type, but not enough occurrences.
    assert!(
        rpt.patch_record_bytes_with(RecordTag(SECTION_RECORD), 100_000, 0, &[0], FORCED)
            .is_err(),
        "an out-of-range occurrence index must be rejected"
    );
}

/// The first Section record's inner length-prefixed name: `(demasked field bytes, lp_prefix_offset,
/// declared_len, name_bytes_incl_any_trailing_nul)`. The name is `[u32-BE len][len bytes]` and lives
/// after the 4-byte height, so the scan starts at offset 4.
fn first_section_lp(rpt: &Rpt) -> (Vec<u8>, usize, usize, Vec<u8>) {
    let bytes = first_section_bytes(rpt);
    let mut i = 4;
    while i + 4 < bytes.len() {
        let len = u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        if (2..=4096).contains(&len) && i + 4 + len <= bytes.len() {
            let text = &bytes[i + 4..i + 4 + len];
            // Printable run (a trailing NUL is allowed and kept in the byte count).
            let body = if text.last() == Some(&0) {
                &text[..len - 1]
            } else {
                text
            };
            if !body.is_empty() && body.iter().all(|&b| (0x20..0x7f).contains(&b)) {
                return (bytes.clone(), i, len, text.to_vec());
            }
        }
        i += 1;
    }
    panic!("no length-prefixed Section name found");
}

/// A LONGER Section name: replace the inner length-prefixed name (its `u32-BE` length +
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
        .patch_record_bytes_resize_with(RecordTag(SECTION_RECORD), 0, region, &new_bytes, FORCED)
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

/// A SHORTER Section name shrinks the stream by exactly the delta and re-reads correctly.
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
        .patch_record_bytes_resize_with(RecordTag(SECTION_RECORD), 0, region, &new_bytes, FORCED)
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

/// A same-length resize is byte-identical to a same-size patch (no length prefix changes).
#[test]
fn resize_same_length_matches_same_size_patch() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    let (field_off, orig) = first_section_name(&rpt);
    let replacement: Vec<u8> = orig
        .iter()
        .map(|&b| if b == b'Z' { b'A' } else { b + 1 })
        .collect();
    let via_resize = rpt
        .patch_record_bytes_resize_with(
            RecordTag(SECTION_RECORD),
            0,
            field_off..field_off + orig.len(),
            &replacement,
            FORCED,
        )
        .expect("same-length resize");
    let via_patch = rpt
        .patch_record_bytes_with(
            RecordTag(SECTION_RECORD),
            0,
            field_off,
            &replacement,
            FORCED,
        )
        .expect("same-size patch");
    let a = contents_logical(&Rpt::read(std::io::Cursor::new(via_resize)).unwrap());
    let b = contents_logical(&Rpt::read(std::io::Cursor::new(via_patch)).unwrap());
    assert_eq!(a, b, "a same-length resize equals the same-size patch");
}

/// Guard probes: a missing record, and a region that overruns the field bytes, both `Err` with no
/// file produced.
#[test]
fn resize_guards_reject_bad_edits() {
    let Some(rpt) = open("synthetic/single_group.rpt") else {
        eprintln!("[skip] fixture absent");
        return;
    };
    // Absent record type.
    assert!(rpt
        .patch_record_bytes_resize_with(RecordTag(0xDEAD), 0, 0..1, &[0, 1, 2], FORCED)
        .is_err());
    // Region overruns the field bytes.
    let field_byte_len = section_bytes_len(&rpt);
    assert!(rpt
        .patch_record_bytes_resize_with(
            RecordTag(SECTION_RECORD),
            0,
            0..field_byte_len + 5,
            b"xyz",
            FORCED
        )
        .is_err());
    // Region far past the field bytes.
    assert!(rpt
        .patch_record_bytes_resize_with(
            RecordTag(SECTION_RECORD),
            0,
            1_000_000..1_000_001,
            b"z",
            FORCED
        )
        .is_err());
}

/// Corpus round-trip: every `.rpt` that decodes must re-encode losslessly at the logical level.
///
/// Walks the whole corpus ([`rpt_test_support::corpus_reports`]). For each report that opens, both
/// writer paths must
/// reproduce the Contents logical (inflated) record stream byte-for-byte: `serialize_tree` (the raw
/// record tree) and a full no-op `reencode` re-opened from its bytes. Byte-identity is asserted at
/// the inflated level only — deflate is non-canonical, so the file bytes legitimately differ.
/// Every report in the corpus must open and be round-tripped: a report that stops decoding silently
/// leaves the writer unexercised on it, so it fails here rather than shrinking the sweep.
#[test]
fn corpus_reencode_round_trips_logical() {
    let rpts = rpt_test_support::corpus_reports();

    let (mut round_tripped, mut unreadable) = (0usize, Vec::new());
    for path in &rpts {
        let name = path.display();
        let Ok(rpt) = Rpt::open(path) else {
            unreadable.push(format!("  {name}: does not open"));
            continue;
        };

        let Some(contents) = rpt.stream(&StreamId::Contents) else {
            unreadable.push(format!("  {name}: no Contents stream"));
            continue;
        };
        round_tripped += 1;
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

    eprintln!(
        "[corpus round-trip] {round_tripped} of {} report(s) round-tripped",
        rpts.len()
    );
    assert!(
        unreadable.is_empty(),
        "{} of {} corpus report(s) were never round-tripped, so the writer went unexercised on them:\n{}",
        unreadable.len(),
        rpts.len(),
        unreadable.join("\n")
    );
    assert_eq!(round_tripped, rpts.len());
}

/// The Section record tag (`0x008c`) — a record with no nested children, whose name is an ASCII run
/// in its field bytes.
const SECTION_RECORD: u16 = 0x008c;

/// The group-options record tag (`0x0088`) — a record that nests another one mid-sequence, so its
/// field bytes come in two runs that are not adjacent in the file.
const GROUP_OPTIONS_RECORD: u16 = 0x0088;

/// These tests exercise the writer's *mechanics* — round-tripping, length recompute, bounds
/// rejection — over the Section record, which is not on the cleared-for-editing allow-list. The
/// clearance gate is a separate concern with its own tests, so it is forced out of the way here
/// rather than being satisfied by clearing a record type just to keep a test green.
const FORCED: EditPolicy = EditPolicy::Forced;

/// The `(joined_offset, bytes)` of the first Section record's name — the first alphabetic ASCII run
/// in the first `0x008c` record's demasked field bytes.
fn first_section_name(rpt: &Rpt) -> (usize, Vec<u8>) {
    let bytes = first_section_bytes(rpt);
    let start = bytes
        .iter()
        .position(|&b| b.is_ascii_alphabetic())
        .expect("section record has an ASCII name");
    let end = bytes[start..]
        .iter()
        .position(|&b| !(0x20..0x7f).contains(&b))
        .map(|e| start + e)
        .unwrap_or(bytes.len());
    (start, bytes[start..end].to_vec())
}

fn section_bytes_len(rpt: &Rpt) -> usize {
    first_section_bytes(rpt).len()
}

/// The demasked field bytes of the first (pre-order) `0x008c` Section record.
fn first_section_bytes(rpt: &Rpt) -> Vec<u8> {
    let contents = rpt.stream(&StreamId::Contents).expect("Contents stream");
    let logical = contents.logical_bytes();
    let mut found: Option<Vec<u8>> = None;
    for root in contents.record_tree() {
        root.walk(&mut |n| {
            if found.is_none() && n.rtype == SECTION_RECORD {
                found = Some(n.joined_runs(logical));
            }
        });
        if found.is_some() {
            break;
        }
    }
    found.expect("fixture has a Section record")
}
