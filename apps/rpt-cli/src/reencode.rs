//! `reencode` / `patch` — the write path of the `rpt-reader` library, exposed for tooling.
//!
//! `reencode` runs the no-op writer (decode → re-encode the `Contents` stream, byte-identical
//! inflated) and writes the result to an explicit output path. `patch` changes one field of one
//! record, addressed by the name its record type's field table gives it, and writes the result out.
//! Both only ever write the single output path passed on the command line.

use rpt_reader::fields::FieldEdit;
use rpt_reader::raw::RecordTag;
use rpt_reader::{EditPolicy, Rpt};

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
rpt patch — change one field of one record

Locates the <nth> (0-based, pre-order) record of type <tag> in the Contents record tree, stores
<value> in the field <target> names, then re-encodes and writes a fresh .rpt to <out.rpt>.

A field is addressed by the name its record type's field table gives it, and the table decides
where its bytes are and how wide they are. A value that does not fit the width it had is written
anyway: the record's length prefix and every enclosing record's are recomputed.

The edit is refused, and nothing is written, unless the record type's field table reproduces the
record byte for byte and the written record reads back with that field at its new value and every
other field unchanged. --force skips both. That is the right flag when writing a deliberately
invalid record is the point — probing what a field means — and the wrong one for editing a report
you intend to keep.

USAGE:
    rpt patch [--force] <in.rpt> <tag> <nth> <target> <value> <out.rpt>

    <tag>       record type, hex (e.g. 0x64) or decimal
    <nth>       0-based occurrence of that record type in pre-order
    <target>    the field to change, by name (`group_indent`, `element_styles[3].weight`), or
                @<offset> to overwrite raw bytes at an offset into the record's field bytes
    <value>     the new value at the field's declared wire type: a decimal or 0x number, a float,
                a string, true/false, or hex bytes for an undecoded run. For @<offset>, hex bytes,
                whose length is the region size — a raw edit is same-size only.
    --force     write without the round-trip and read-back checks (risks silent corruption)

`rpt dump <in.rpt> --type <tag> --nth <nth>` lists the field names that record offers.
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

/// Change one field of one record in `input`'s Contents and write the result to `output`.
///
/// `target` is a field name, or `@<offset>` for the raw byte form kept for record types that have
/// no field table. `force` skips the write path's safety checks; without it a refused edit is
/// refused before any bytes exist, so `output` is never touched.
pub(crate) fn patch(
    input: &str,
    tag: &str,
    nth: &str,
    target: &str,
    value: &str,
    output: &str,
    force: bool,
) -> Result<(), CliError> {
    let tag_num = parse_u16(tag).ok_or_else(|| CliError::usage(format!("bad <tag>: {tag}")))?;
    let nth: usize = nth
        .parse()
        .map_err(|_| CliError::usage(format!("bad <nth>: {nth}")))?;
    let policy = if force {
        EditPolicy::Forced
    } else {
        EditPolicy::Checked
    };
    let rpt = Rpt::open(input)?;
    let tag = RecordTag(tag_num);

    let (bytes, what) = match target.strip_prefix('@') {
        Some(offset) => {
            let offset: usize = offset
                .parse()
                .map_err(|_| CliError::usage(format!("bad <target> offset: @{offset}")))?;
            let new_bytes = parse_hex(value)
                .ok_or_else(|| CliError::usage(format!("bad hex value: {value}")))?;
            let bytes = rpt.patch_record_bytes_with(tag, nth, offset, &new_bytes, policy)?;
            (bytes, format!("@{offset} = {} byte(s)", new_bytes.len()))
        }
        None => {
            let field = rpt.record_field(tag, nth, target)?;
            let edit = FieldEdit::parse(value, field.kind).ok_or_else(|| {
                CliError::usage(format!(
                    "`{target}` is a {}, which reads no value from `{value}`",
                    field.kind.label()
                ))
            })?;
            let bytes = rpt.patch_record_field_with(tag, nth, target, &edit, policy)?;
            (bytes, format!("{target}: {:?} -> {value}", field.value))
        }
    };
    std::fs::write(output, &bytes)
        .map_err(|e| CliError::io(format!("cannot write `{output}`"), e))?;
    eprintln!(
        "patch: {input} tag={tag_num:#06x} nth={nth} {what} -> {output} ({} bytes)",
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
