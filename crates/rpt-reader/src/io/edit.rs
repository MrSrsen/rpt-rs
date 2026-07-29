//! The write path — turning an opened report back into `.rpt` file bytes, with or without a change.
//!
//! Every method here ends the same way: the `Contents` stream's logical bytes, edited or not, are
//! re-framed, deflated, encrypted and spliced into a fresh copy of the container. What differs is
//! how the replacement logical bytes are produced — not at all ([`Rpt::reencode`]), by overwriting a
//! byte region of one record ([`Rpt::patch_record_bytes`], [`Rpt::patch_record_bytes_resize`]), or
//! by writing a value under the name its record type's field table gives it
//! ([`Rpt::patch_record_field`]).
//!
//! Each operation comes in a pair: the short form takes the default [`EditPolicy`], the `_with` form
//! states one. The policy is what stands between a caller and a file that re-opens cleanly while
//! being semantically corrupt, so it is a per-call decision rather than a property of the report.

use crate::codec::Dialect;
use crate::error::{EditErrorKind, Error, Result};
use crate::records::RecordTag;
use crate::{Rpt, StreamId};

use super::cleared;
use super::patch::{not_found, nth_node, nth_node_path, patch_joined_region};

/// Whether the writer demands evidence that an edit is safe. It does by default.
///
/// What counts as evidence depends on how the edit is addressed, because the two ways of
/// addressing one say different amounts about what is being written:
///
/// - **A byte region** ([`Rpt::patch_record_bytes`], [`Rpt::patch_record_bytes_resize`]) — the
///   record *type* must be on the cleared-for-editing allow-list.
/// - **A named field** ([`Rpt::patch_record_field`]) — *this* record must round-trip through its
///   own field table byte for byte, and the written record must read back with that field at its
///   new value and every other field unchanged. The allow-list is not consulted.
///
/// The asymmetry is deliberate. A region edit states an offset and some bytes and says nothing
/// about what they mean, so the only evidence available is a standing judgement about the record
/// type — a list, maintained by hand, and nearly empty. A field edit names something the table
/// declares, which lets the table demonstrate on the record in front of it that it accounts for
/// every byte; that evidence the file supplies, per record. So one record type can be refused by
/// region and accepted by name, and neither answer is the other's mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditPolicy {
    /// Demand whichever evidence the addressing makes available. The default: an edit nothing
    /// vouches for is refused rather than silently producing a damaged file.
    #[default]
    Checked,
    /// Write regardless, for callers whose purpose *is* to produce a record the reader does not
    /// fully model. Skips whichever gate [`Checked`](EditPolicy::Checked) would have applied. The
    /// mechanical bounds still hold, and a field edit still has to name a field the table declares.
    Forced,
}

impl Rpt {
    /// Re-encode the report's `Contents` stream from its current logical bytes and return the new
    /// `.rpt` file bytes — a no-op run of the write pipeline (TSLV logical → deflate → AES-CFB
    /// encrypt → CFB rewrite, every other stream verbatim). The result re-opens to byte-identical
    /// logical/record bytes; the file bytes differ because deflate is non-canonical.
    ///
    /// # Errors
    ///
    /// - [`Error::Container`](crate::Error::Container) — the report has no `Contents` stream, or the
    ///   container cannot be rewritten.
    /// - [`Error::Codec`](crate::Error::Codec) / [`Error::Crypto`](crate::Error::Crypto) — the stream
    ///   could not be re-framed, deflated, or encrypted.
    pub fn reencode(&self) -> Result<Vec<u8>> {
        let contents = self.contents_stream()?;
        self.reencode_contents(contents.logical_bytes())
    }

    /// Change a decoded record's **field** — the value stored under a name its record type's field
    /// table declares — and return the new `.rpt` file bytes.
    ///
    /// Locates the `nth` (0-based, pre-order) record whose type is `tag` in the `Contents` record
    /// tree and replaces the bytes of the field `field` names ([`FieldRead::path`](crate::fields::FieldRead::path)) with
    /// `value` encoded at that field's declared wire type. The table decides where the bytes are
    /// and how wide they are, so the caller states a name and a value and never an offset — the one
    /// coordinate that moves with the string before it, the repeat before it, and the record's
    /// schema version.
    ///
    /// A replacement of a different width is written: the record's own length prefix and every
    /// enclosing record's are recomputed by the delta, exactly as
    /// [`Rpt::patch_record_bytes_resize`] does.
    ///
    /// Under [`EditPolicy::Checked`] a tabled record type replaces the hand-maintained clearance
    /// list with a property the file itself demonstrates: the record must round-trip through its own
    /// table byte for byte, and the written record must read back with the field at its new value
    /// and every other field unchanged. Both are checked, and a failure of either writes nothing.
    ///
    /// # Errors
    ///
    /// - [`Error::Edit`](crate::Error::Edit) with [`FieldEdit`](crate::EditErrorKind::FieldEdit) —
    ///   the record type has no field table, no field of that path was read, or `value` does not
    ///   fit the field's wire type.
    /// - [`Error::Edit`](crate::Error::Edit) with
    ///   [`UnclearedRecordEdit`](crate::EditErrorKind::UnclearedRecordEdit) — the table does not
    ///   reproduce this record byte for byte, so it does not describe it well enough to edit.
    /// - [`Error::Edit`](crate::Error::Edit) with
    ///   [`EditNotVerified`](crate::EditErrorKind::EditNotVerified) — the written record does not
    ///   read back as the edit asked for.
    /// - [`Error::Codec`](crate::Error::Codec) — fewer than `nth + 1` records of `tag` exist, or a
    ///   recomputed length prefix would overflow its on-disk field width.
    /// - [`Error::Container`](crate::Error::Container) — the report has no `Contents` stream.
    pub fn patch_record_field(
        &self,
        tag: RecordTag,
        nth: usize,
        field: &str,
        value: &crate::fields::FieldEdit,
    ) -> Result<Vec<u8>> {
        self.patch_record_field_with(tag, nth, field, value, EditPolicy::default())
    }

    /// As [`Rpt::patch_record_field`], with an explicit [`EditPolicy`].
    ///
    /// [`EditPolicy::Forced`] skips both the round-trip clearance and the read-back verification,
    /// for callers whose purpose *is* to write a record the reader does not fully model. The field
    /// still has to be one the table names: a forced edit writes a wrong value, not a value nowhere.
    ///
    /// # Errors
    ///
    /// As [`Rpt::patch_record_field`], less the clearance and verification refusals.
    pub fn patch_record_field_with(
        &self,
        tag: RecordTag,
        nth: usize,
        field: &str,
        value: &crate::fields::FieldEdit,
        policy: EditPolicy,
    ) -> Result<Vec<u8>> {
        let contents = self.contents_stream()?;
        let logical = contents.logical_bytes();
        let tree = contents.record_tree();
        let (node, ancestors) =
            nth_node_path(&tree, tag.0, nth).ok_or_else(|| not_found(tag, nth))?;

        let patch = crate::fields::edit(node, logical, Dialect::Contents, field, value)?;
        // The table reproducing the record it just read is the evidence that nothing in the record
        // is left for an edit to desynchronize — the property the hand-maintained clearance list
        // stands in for everywhere else.
        if policy != EditPolicy::Forced
            && !crate::fields::round_trips(node, logical, Dialect::Contents)
        {
            return Err(Error::Edit {
                kind: EditErrorKind::UnclearedRecordEdit,
                detail: format!(
                    "the field table for record type {:#06x} does not reproduce this record byte \
                     for byte, so it does not describe the record well enough to edit by name. \
                     Pass `EditPolicy::Forced` to edit anyway",
                    tag.0
                ),
            });
        }
        let region = patch.field.joined.start..patch.field.joined.end;
        let new_logical =
            crate::codec::resize_joined_region(logical, node, &ancestors, region, &patch.bytes)?;
        if policy != EditPolicy::Forced {
            // The other half of the same property, over the bytes about to be written. A resize
            // moved every offset past the edit, so the record is addressed again in a fresh tree.
            let new_tree =
                crate::codec::parse_tree(&new_logical, Some(crate::field_table::declared_children));
            let written = nth_node(&new_tree, tag.0, nth).ok_or_else(|| {
                crate::fields::not_verified(format!(
                    "editing `{field}` left no record #{nth} of type {:#06x} in the record tree",
                    tag.0
                ))
            })?;
            crate::fields::verify_edit(
                written,
                &new_logical,
                Dialect::Contents,
                field,
                value,
                &patch,
            )?;
        }
        self.reencode_contents(&new_logical)
    }

    /// Change a same-size region of a decoded record's field bytes and return the new `.rpt` file
    /// bytes.
    ///
    /// Locates the `nth` (0-based, pre-order) record whose type is `tag` in the `Contents` record
    /// tree, then overwrites `new_bytes.len()` bytes of its **field bytes** starting at `at`,
    /// re-masking each byte with the record's stack mask. A record's field bytes are its content
    /// less its nested child records, so `at` indexes the record's own runs joined end to end — a
    /// buffer that exists nowhere in the file. **Same-size only**: `new_bytes` replaces an
    /// equal-length region, so the logical stream length never changes.
    ///
    /// Prefer [`Rpt::patch_record_field`] where the record type has a field table: a byte offset
    /// into a record has to be recomputed whenever a string, a repeat count or the schema version
    /// ahead of it changes, and nothing checks that it was.
    ///
    /// Refuses an edit to a record type that is not cleared for safe editing; see
    /// [`Rpt::patch_record_bytes_with`] to override that.
    ///
    /// # Errors
    ///
    /// - [`Error::Edit`](crate::Error::Edit) with
    ///   [`UnclearedRecordEdit`](crate::EditErrorKind::UnclearedRecordEdit) — `tag` is not cleared
    ///   for editing. Refused before any bytes are produced.
    /// - [`Error::Codec`](crate::Error::Codec) — fewer than `nth + 1` records of `tag` exist, or the
    ///   region `[at, at + new_bytes.len())` overruns the record's field bytes.
    /// - [`Error::Container`](crate::Error::Container) — the report has no `Contents` stream.
    pub fn patch_record_bytes(
        &self,
        tag: RecordTag,
        nth: usize,
        at: usize,
        new_bytes: &[u8],
    ) -> Result<Vec<u8>> {
        self.patch_record_bytes_with(tag, nth, at, new_bytes, EditPolicy::default())
    }

    /// As [`Rpt::patch_record_bytes`], with an explicit [`EditPolicy`].
    ///
    /// [`EditPolicy::Forced`] skips the clearance check, for callers whose purpose *is* to write a
    /// record the reader does not fully model.
    ///
    /// # Errors
    ///
    /// As [`Rpt::patch_record_bytes`], less the clearance refusal when `policy` is
    /// [`EditPolicy::Forced`].
    pub fn patch_record_bytes_with(
        &self,
        tag: RecordTag,
        nth: usize,
        at: usize,
        new_bytes: &[u8],
        policy: EditPolicy,
    ) -> Result<Vec<u8>> {
        cleared::check(tag.0, policy)?;
        let contents = self.contents_stream()?;
        let logical = contents.logical_bytes();
        let tree = contents.record_tree();
        let node = nth_node(&tree, tag.0, nth).ok_or_else(|| not_found(tag, nth))?;

        let mut new_logical = logical.to_vec();
        patch_joined_region(node, &mut new_logical, at, new_bytes)?;
        self.reencode_contents(&new_logical)
    }

    /// Replace a **region** of a decoded record's field bytes with `new_bytes` of a **possibly
    /// different length**, and return the new `.rpt` file bytes.
    ///
    /// Locates the `nth` (0-based, pre-order) record of type `tag` in the `Contents` record tree and
    /// replaces its field bytes `[region.start, region.end)` — offsets into the record's runs joined
    /// end to end — with `new_bytes` (any length). The record's own length prefix and every
    /// enclosing record's length prefix are recomputed by the size delta; because the `Contents`
    /// tree holds no absolute byte offsets, nothing else needs fixing. When `region.len() ==
    /// new_bytes.len()` this is an in-place overwrite (equivalent to [`Rpt::patch_record_bytes`]).
    ///
    /// Prefer [`Rpt::patch_record_field`] where the record type has a field table, which finds the
    /// region from a field name rather than taking one on trust.
    ///
    /// Refuses an edit to a record type that is not cleared for safe editing; see
    /// [`Rpt::patch_record_bytes_resize_with`] to override that.
    ///
    /// # Errors
    ///
    /// Nothing is written in any of these cases:
    ///
    /// - [`Error::Edit`](crate::Error::Edit) with
    ///   [`UnclearedRecordEdit`](crate::EditErrorKind::UnclearedRecordEdit) — `tag` is not cleared
    ///   for editing.
    /// - [`Error::Codec`](crate::Error::Codec) — the record is not found, `region` is outside the
    ///   record's field bytes or straddles a nested child record, or a recomputed length prefix
    ///   would overflow its on-disk field width.
    /// - [`Error::Container`](crate::Error::Container) — the report has no `Contents` stream.
    pub fn patch_record_bytes_resize(
        &self,
        tag: RecordTag,
        nth: usize,
        region: std::ops::Range<usize>,
        new_bytes: &[u8],
    ) -> Result<Vec<u8>> {
        self.patch_record_bytes_resize_with(tag, nth, region, new_bytes, EditPolicy::default())
    }

    /// As [`Rpt::patch_record_bytes_resize`], with an explicit [`EditPolicy`].
    ///
    /// # Errors
    ///
    /// As [`Rpt::patch_record_bytes_resize`], less the clearance refusal when `policy` is
    /// [`EditPolicy::Forced`].
    pub fn patch_record_bytes_resize_with(
        &self,
        tag: RecordTag,
        nth: usize,
        region: std::ops::Range<usize>,
        new_bytes: &[u8],
        policy: EditPolicy,
    ) -> Result<Vec<u8>> {
        cleared::check(tag.0, policy)?;
        let contents = self.contents_stream()?;
        let logical = contents.logical_bytes();
        let tree = contents.record_tree();
        let (node, ancestors) =
            nth_node_path(&tree, tag.0, nth).ok_or_else(|| not_found(tag, nth))?;
        let new_logical =
            crate::codec::resize_joined_region(logical, node, &ancestors, region, new_bytes)?;
        self.reencode_contents(&new_logical)
    }

    /// Re-encode `Contents` from replacement logical bytes and splice it into a fresh copy of the
    /// container — the last step of every operation in this module.
    fn reencode_contents(&self, new_logical: &[u8]) -> Result<Vec<u8>> {
        let raw = self.contents_stream()?.raw_bytes();
        let new_stream = crate::codec::encode_contents(raw, new_logical)?;
        crate::container::rewrite_stream(&self.original, &StreamId::Contents, &new_stream)
    }
}
