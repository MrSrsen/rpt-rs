//! The cleared-for-editing allow-list — which record types a **byte-region** edit may touch without
//! being forced. It is one of the two regimes [`EditPolicy`] governs, and it gates only the region
//! edits: an edit that names a field is cleared by evidence its own record supplies instead.
//!
//! The write path validates only *mechanical* bounds: the record exists, the region is inside its
//! field bytes, recomputed length prefixes fit. Those checks cannot catch the failure that matters. A
//! record whose field bytes carry their own offset table, an element count, or a checksum can be
//! overwritten into a file that is structurally perfect — it re-encodes, re-opens, and re-decodes
//! without complaint — yet semantically corrupt. Nothing warns at write time and nothing errors at
//! read time; the damage shows up later in the Crystal designer or as a silently wrong render, with
//! nothing pointing back at the edit.
//!
//! So the default posture is **refuse**, and a record type earns its place here only once there is
//! evidence that editing it cannot desynchronize anything. An empty-ish list is the honest state of
//! the world, not a gap: it says "one record type has been cleared", which is true, rather than
//! implying the writer understands every record's internals.
//!
//! Writing a record that is *not* cleared is a legitimate thing to want — probing what a field
//! means takes a deliberately invalid record. That is what [`EditPolicy::Forced`] is for, and it is
//! a per-call decision, not a global one.

use super::edit::EditPolicy;
use crate::error::{EditErrorKind, Error};
use crate::field_table::tables::SUBREPORT_REIMPORT_INFO;

/// A record type cleared for editing, and the evidence that clears it.
///
/// The type number and name are taken from the record's field table rather than restated here:
/// clearing a record type by a literal of its own would authorise an edit to whichever record that
/// number turned out to name, which is the one mistake this list exists to prevent.
struct Cleared {
    rtype: u16,
    name: &'static str,
    /// Why an edit here cannot desynchronize the record or its neighbours. This is the entry's
    /// justification, not a description — a new entry without one does not belong on the list.
    why: &'static str,
}

/// Every record type an edit is allowed to touch under [`EditPolicy::Checked`].
///
/// Clearance is per record *type* because no cleared record has needed a finer rule. A record
/// whose field bytes are only partly safe (a fixed-width scalar field beside an offset table, say) would add
/// a region bound to this entry rather than being cleared wholesale.
const CLEARED: &[Cleared] = &[Cleared {
    rtype: SUBREPORT_REIMPORT_INFO.rtype,
    name: SUBREPORT_REIMPORT_INFO.name,
    why: "its field bytes are a big-endian length-prefixed path followed by a fixed 17-byte trailer, so it \
          carries no offset table, count, or checksum — the only length in play is the record's own \
          prefix, which the writer recomputes. Editing it in bulk (the `anonymize` path) leaves \
          every report opening in the native engine with byte-identical output apart from the \
          intended change",
}];

/// Refuse an edit to `rtype` unless it is cleared or `policy` forces it.
///
/// # Errors
///
/// [`Error::Edit`] with [`EditErrorKind::UnclearedRecordEdit`], naming the record type and
/// saying what forcing risks. Refused *before* any bytes are produced, so nothing is written.
pub(crate) fn check(rtype: u16, policy: EditPolicy) -> Result<(), Error> {
    if policy == EditPolicy::Forced || CLEARED.iter().any(|c| c.rtype == rtype) {
        return Ok(());
    }
    // List what *is* cleared, with each entry's justification: it shows the user where the boundary
    // is and on what basis, which is what they need in order to judge whether to force the edit.
    let cleared = CLEARED
        .iter()
        .map(|c| format!("\n  {:#06x} ({}) — {}", c.rtype, c.name, c.why))
        .collect::<String>();
    Err(Error::Edit {
        kind: EditErrorKind::UnclearedRecordEdit,
        detail: format!(
            "record type {rtype:#06x} is not cleared for editing. Its field bytes may carry an internal \
             offset table, element count, or checksum that an edit would desynchronize, producing a \
             file that re-opens cleanly but is semantically corrupt. Pass --force (or \
             `EditPolicy::Forced`) to edit anyway — appropriate when writing an invalid record is \
             the point, not when editing a report you intend to keep. Cleared for \
             editing:{cleared}"
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cleared_record_passes() {
        // Spelled out rather than taken from the list: it pins *which* record type is cleared, so a
        // list that came to name a different one fails here instead of passing against itself.
        assert!(check(0x0142, EditPolicy::Checked).is_ok());
    }

    #[test]
    fn an_uncleared_record_is_refused_with_the_reason_and_the_escape_hatch() {
        let err = check(0x008c, EditPolicy::Checked).expect_err("0x008c is not cleared");
        let Error::Edit { kind, detail } = &err else {
            panic!("expected a Project error, got {err:?}");
        };
        assert_eq!(*kind, EditErrorKind::UnclearedRecordEdit);
        assert!(detail.contains("0x008c"), "{detail}");
        assert!(detail.contains("--force"), "{detail}");
        // The refusal lists what *is* cleared, so the user can see the boundary.
        assert!(detail.contains("SubreportReimportInfo"), "{detail}");
    }

    #[test]
    fn forcing_permits_any_record_type() {
        assert!(check(0x008c, EditPolicy::Forced).is_ok());
        assert!(check(0xdead, EditPolicy::Forced).is_ok());
    }

    #[test]
    fn every_entry_states_its_evidence() {
        for c in CLEARED {
            assert!(
                !c.why.is_empty() && !c.name.is_empty(),
                "record {:#06x} is on the allow-list without a justification",
                c.rtype
            );
        }
    }
}
