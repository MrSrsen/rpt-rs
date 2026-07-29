//! Declarative record decoding: a record cursor plus a field table per record type.
//!
//! A record's content is a straight-line sequence of typed reads that stops when the record runs
//! out, so a field's position is a consequence of what precedes it — not a constant. This module
//! states that sequence as data ([`table::Table`]) and walks it with a cursor that reports
//! exhaustion instead of failing ([`cursor::ContentCursor`]).
//!
//! Two properties are deliberate:
//!
//! - **The table accounts for the record exactly.** A walk that ends with unread bytes, an
//!   undeclared child record, or a read blocked by one is a loud failure, not a tolerance — so a
//!   single wrong length moves every record of that type at once.
//! - **One table drives both directions.** [`table::read_strings`] decodes it, [`table::write_as`]
//!   emits it. What a table cannot state — a record's own header, which a reader is told and a
//!   writer must choose — is passed to both and declared beside the table in [`framing`].
//!
//! # The vocabulary
//!
//! A [`table::Table`] declares one record type: its type number, a name, and a slice of
//! [`table::Field`]s in wire order. A field is a name, a [`table::Kind`] — the wire type, which
//! names the width and byte order of a scalar and otherwise stands for a string, a blob, a nested
//! record or a repeated body — and a [`table::Presence`] saying when a record of the type carries
//! the field at all: always, only while content remains, from a schema version on or at one alone,
//! or while a predicate over the fields read so far holds. A repeated body runs [`table::Count`]
//! times, either fixed or taken from an earlier field's value.
//!
//! Walking a table over a record produces a [`table::Row`]: the [`table::Cell`]s the record
//! actually carried, under their field names, in table order. A field the record ended before is
//! absent from the row rather than zero, which is how a consumer tells "not stored" from "stored as
//! zero".
//!
//! A declaration reads down the record's content, one line per field:
//!
//! ```text
//! Table {
//!     rtype: 0x00be,
//!     name: "ObjectPosition",
//!     fields: &[
//!         Field::new("left", Kind::VarU32),  // two narrowing twips, so the second field's
//!         Field::new("top", Kind::VarU32),   // position follows the first one's magnitude
//!     ],
//! }
//! ```
//!
//! [`Field::new`](table::Field::new) is the unconditional entry;
//! [`optional`](table::Field::optional), [`from_schema`](table::Field::from_schema) and
//! [`when`](table::Field::when) are the same entry under the other presences. The tables themselves
//! are in [`tables`], one per record type.
//!
//! The module's contact with the rest of the crate is two functions: [`content_of`], which turns a
//! decoded [`RecordNode`](crate::raw::RecordNode) into the cursor's own
//! [`cursor::RecordContent`], and [`declared_children`], which answers the record-tree reader's
//! question of what may nest inside a record type — so that a type with a table has its children
//! declared rather than scanned for.

// Part of the module is reached only from the harness below: the header a writer stamps, the
// exactness diagnostics, and the vocabulary no table declares. The harness is inline because
// what it adjudicates is `pub(crate)`, so to a build without it those items read as unused. Under
// `cfg(test)` the harness is a caller and the allow lifts, so an item dead to it as well is still
// reported.
#![cfg_attr(not(test), allow(dead_code))]

pub(crate) mod cursor;
pub(crate) mod framing;
pub(crate) mod table;
pub(crate) mod tables;

#[cfg(test)]
pub(crate) mod corpus;
#[cfg(test)]
mod parity;

use crate::codec::tslv::{Flags, StringFormat};
use crate::codec::{Dialect, PieceSpan, RecordNode};
use cursor::{ChildRef, Piece, RecordContent};
use table::{Field, Kind};

/// The child record types a record type's table declares.
///
/// This is the reader's *ask*: a child record exists at a point in the parent's field sequence
/// because the parent's declaration says so, by type. A fixed-capacity set so a declaration can be
/// carried down a parse level without allocating.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DeclaredChildren {
    types: [u16; Self::CAP],
    len: u8,
}

impl DeclaredChildren {
    /// More child types than any record type declares; a table that exceeded it would silently
    /// lose the excess, so it is asserted on in debug builds.
    pub(crate) const CAP: usize = 16;

    /// Add a declared child type (idempotent — a table may declare the same type at two points).
    pub(crate) fn insert(&mut self, rtype: u16) {
        if self.declares(rtype) {
            return;
        }
        let n = usize::from(self.len);
        debug_assert!(
            n < Self::CAP,
            "a record type declares more than {} children",
            Self::CAP
        );
        if let Some(slot) = self.types.get_mut(n) {
            *slot = rtype;
            self.len += 1;
        }
    }

    /// Whether a record of this type is asked for here.
    pub(crate) fn declares(&self, rtype: u16) -> bool {
        self.types[..usize::from(self.len)].contains(&rtype)
    }
}

/// The children a record type declares in `dialect`, or `None` for a type with no table.
///
/// This is the record-tree reader's child rule: where a table declares a `Kind::Child`, the reader
/// asks for a record of that type instead of scanning the content for anything header-shaped.
pub(crate) fn declared_children(
    rtype: u16,
    schema: u16,
    dialect: Dialect,
) -> Option<DeclaredChildren> {
    let table = tables::for_record(rtype, schema, dialect)?;
    let mut out = DeclaredChildren::default();
    collect(table.fields, &mut out);
    Some(out)
}

fn collect(fields: &'static [Field], out: &mut DeclaredChildren) {
    for f in fields {
        match f.kind {
            Kind::Child(rtype) => out.insert(rtype),
            Kind::Repeat { body, .. } => collect(body, out),
            _ => {}
        }
    }
}

/// Project a record from the tree reader into the cursor's content model: field-byte runs and
/// child records, in wire order.
///
/// Everything else in the module operates on [`RecordContent`] alone.
pub(crate) fn content_of(node: &RecordNode, logical: &[u8]) -> RecordContent {
    RecordContent {
        rtype: node.rtype,
        schema: node.schema,
        pieces: node
            .pieces()
            .map(|piece| match piece {
                PieceSpan::Run { start, end } => Piece::Run(node.demasked(logical, start, end)),
                PieceSpan::Child(child) => Piece::Child(ChildRef {
                    rtype: child.rtype,
                    schema: child.schema,
                    framed_len: child.framed_len(),
                }),
            })
            .collect(),
    }
}

/// The string wire form `node` declares for its own content.
///
/// The choice is stated in the record's header, not inferred from the stream or supplied by the
/// field table, so it is read back from the header bytes — which are masked one level out from the
/// content, under the mask in effect when the header itself was read.
pub(crate) fn strings_format_of(node: &RecordNode, logical: &[u8]) -> StringFormat {
    let header_mask = node.mask ^ (node.rtype as u8);
    let at = |i: usize| logical.get(node.offset + i).map_or(0, |b| b ^ header_mask);
    Flags::decode(&[at(0), at(1)]).string_format()
}
