//! `reencode` / `patch` — the write path of the `rpt` library, exposed for tooling.
//!
//! `reencode` runs the no-op writer (decode → re-encode the `Contents` stream, byte-identical
//! inflated) and writes the result to an explicit output path. `patch` overwrites a same-size
//! region of a decoded record's demasked leaf ([`rpt::Rpt::patch_record_leaf`]) and writes the
//! result out. Both only ever write the single output path passed on the command line.

use rpt::raw::RecordTag;
use rpt::{EditPolicy, Rpt};

use crate::util::CliError;

pub(crate) const HELP: &str = "\
rpt reencode — re-encode a report's Contents stream (no-op writer round-trip)

Decodes <in.rpt> and re-encodes its Contents stream from its own logical bytes, writing a fresh
.rpt to <out.rpt>. The result re-opens to byte-identical record bytes; only the compressed file
bytes differ (deflate is non-canonical). Used to prove the writer's output round-trips through both
our own decoder and the native Crystal engine.

USAGE:
    rpt reencode <in.rpt> <out.rpt>
";

pub(crate) const PATCH_HELP: &str = "\
rpt patch — overwrite a same-size region of one record's demasked leaf

Locates the <nth> (0-based, pre-order) record of type <tag> in the Contents record tree and
overwrites len(<hexbytes>) bytes of its demasked leaf starting at <offset>, then re-encodes and
writes a fresh .rpt to <out.rpt>. Same-size only.

An edit to a record type that is not cleared for safe editing is REFUSED and nothing is written: a
record whose leaf carries an internal offset table, count, or checksum can be overwritten into a
file that re-opens perfectly but is semantically corrupt, with nothing to point back at the edit.
--force writes it anyway. That is the right flag when writing a deliberately invalid record is the
point — probing what a field means — and the wrong one for editing a report you intend to keep.

USAGE:
    rpt patch [--force] <in.rpt> <tag> <nth> <offset> <hexbytes> <out.rpt>

    <tag>       record type, hex (e.g. 0x64) or decimal
    <nth>       0-based occurrence of that record type in pre-order
    <offset>    byte offset into the demasked leaf
    <hexbytes>  replacement bytes as hex (e.g. 01ff2a); its length is the region size
    --force     edit a record type that is not cleared for safe editing (risks silent corruption)
";

/// Re-encode `input`'s Contents stream and write the resulting `.rpt` to `output`.
pub(crate) fn reencode(input: &str, output: &str) -> Result<(), CliError> {
    let rpt = Rpt::open(input)?;
    let bytes = rpt.reencode()?;
    std::fs::write(output, &bytes)
        .map_err(|e| CliError::io(format!("cannot write `{output}`"), e))?;
    eprintln!("reencode: {input} -> {output} ({} bytes)", bytes.len());
    Ok(())
}

/// Patch a same-size leaf region of `input`'s Contents and write the result to `output`.
///
/// `force` edits a record type that is not cleared for safe editing. Without it an uncleared edit is
/// refused by the library before any bytes exist, so `output` is never touched.
pub(crate) fn patch(
    input: &str,
    tag: &str,
    nth: &str,
    offset: &str,
    hexbytes: &str,
    output: &str,
    force: bool,
) -> Result<(), CliError> {
    let tag = parse_u16(tag).ok_or_else(|| CliError::usage(format!("bad <tag>: {tag}")))?;
    let nth: usize = nth
        .parse()
        .map_err(|_| CliError::usage(format!("bad <nth>: {nth}")))?;
    let offset: usize = offset
        .parse()
        .map_err(|_| CliError::usage(format!("bad <offset>: {offset}")))?;
    let new_bytes = parse_hex(hexbytes)
        .ok_or_else(|| CliError::usage(format!("bad <hexbytes>: {hexbytes}")))?;

    let policy = if force {
        EditPolicy::Forced
    } else {
        EditPolicy::Checked
    };
    let rpt = Rpt::open(input)?;
    let bytes = rpt.patch_record_leaf_with(RecordTag(tag), nth, offset, &new_bytes, policy)?;
    std::fs::write(output, &bytes)
        .map_err(|e| CliError::io(format!("cannot write `{output}`"), e))?;
    eprintln!(
        "patch: {input} tag={tag:#06x} nth={nth} offset={offset} len={} -> {output} ({} bytes)",
        new_bytes.len(),
        bytes.len()
    );
    Ok(())
}

/// Parse a `u16` in hex (`0x64`/`64` with a leading `0x`) or decimal.
fn parse_u16(s: &str) -> Option<u16> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

/// Parse an even-length hex string into bytes.
fn parse_hex(s: &str) -> Option<Vec<u8>> {
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    if s.is_empty() || !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}
