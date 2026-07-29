//! The header a writer stamps on a record, per record type.
//!
//! Reading needs none of this: a header states its own shape and the reader takes it. Emitting has
//! to choose, and the choice is load-bearing — a record's schema word is a version, and a version
//! decides which fields the content carries and how wide they are ([`super::table::write_as`]), so
//! a header that misstates it describes content the table did not write.
//!
//! Each record type is written at one version throughout — the writer does not vary it per
//! instance in these streams — and
//! [`tests::every_record_carries_the_framing_its_type_declares`] holds the declarations below to
//! that.

use crate::codec::Dialect;

/// The header facts a writer supplies for a record type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Framing {
    /// The record's version, big-endian on the wire.
    pub schema: u16,
    /// Whether the header carries the version word at all.
    ///
    /// A header may leave it out, and a reader that meets one takes its stream's default in its
    /// place; that is the four-byte header form. Leaving it out is a choice the writer makes per
    /// record type, not a consequence of the value: `Contents` and the parameter-values stream
    /// state the word on every record including those already at their stream default, and
    /// `QESession` states it on every type but one.
    pub schema_stated: bool,
}

/// The version a `Contents` record is written at, save for [`CONTENTS_0701`].
const CONTENTS_SCHEMA: u16 = 0x0700;

/// The `Contents` record types written at `0x0701` instead.
///
/// `0x0044` and `0x0062` are the empty end markers that close `0x0043` and `0x0061`, and carry
/// their partner's version.
const CONTENTS_0701: &[u16] = &[
    0x0029, // RecordSortField
    0x0043, // FormatObject
    0x0044, // the end marker closing FormatObject
    0x0061, // SavedData
    0x0062, // the end marker closing SavedData
    0x007a,
];

/// The version a `QESession` record is written at, save for [`QE_SESSION_SCHEMAS`].
const QE_SESSION_SCHEMA: u16 = 0x0900;

/// The `QESession` record types written at a version of their own.
const QE_SESSION_SCHEMAS: &[(u16, u16)] = &[
    (0x0001, 0x0902), // the connection's enclosing record
    (0x0002, 0x0902), // QeConnection
    (0x0003, 0x0905), // QeTable
    (0x0004, 0x0905), // QeField
    (0x0009, 0x0901), // QeLogonProperty
    (0x000a, 0x0901), // QeTableLink
];

/// The `QESession` record types whose header states no version, leaving a reader to take
/// [`QE_SESSION_SCHEMA`].
const QE_SESSION_UNSTATED: &[u16] = &[
    0x0008, // QeIndex
];

/// The version a `ReportParametersStream` record is written at, save for
/// [`REPORT_PARAMETERS_SCHEMAS`] — the version of the records making up one parameter's saved entry.
const REPORT_PARAMETERS_SCHEMA: u16 = 0x0701;

/// The `ReportParametersStream` record types written at a version of their own: the two carrying a
/// parameter's saved current value, and the pair bracketing the stream.
const REPORT_PARAMETERS_SCHEMAS: &[(u16, u16)] = &[
    (0x0031, 0x0702), // CurrentValueRecord
    (0x0032, 0x0702),
    (0x012f, 0x0700), // DataSourceParametersHeader
    (0x0130, 0x0700), // DataSourceParametersFooter
];

/// The version `schemas` gives `rtype`, or `default` where it names no version of its own.
fn schema_of(schemas: &[(u16, u16)], rtype: u16, default: u16) -> u16 {
    schemas
        .iter()
        .find(|(t, _)| *t == rtype)
        .map_or(default, |(_, s)| *s)
}

/// The header a writer gives a record of type `rtype` in `dialect`.
pub(crate) fn framing(rtype: u16, dialect: Dialect) -> Framing {
    match dialect {
        Dialect::QeSession => Framing {
            schema: schema_of(QE_SESSION_SCHEMAS, rtype, QE_SESSION_SCHEMA),
            schema_stated: !QE_SESSION_UNSTATED.contains(&rtype),
        },
        Dialect::ReportParameters => Framing {
            schema: schema_of(REPORT_PARAMETERS_SCHEMAS, rtype, REPORT_PARAMETERS_SCHEMA),
            schema_stated: true,
        },
        // The catalog declares no framing of its own — it is written by several components at once,
        // so a type number there has no single header shape — and falls in with the report
        // definition's, which nothing emits it under.
        Dialect::Contents | Dialect::Catalog => Framing {
            schema: if CONTENTS_0701.contains(&rtype) {
                0x0701
            } else {
                CONTENTS_SCHEMA
            },
            schema_stated: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{framing, Dialect, Framing};
    use crate::codec::tslv::Flags;
    use crate::codec::RecordNode;

    /// The fewest records the sweep must reach to have said anything. The committed fixtures alone
    /// reach about 112,800, and a private corpus only adds to that.
    const FLOOR: usize = 100_000;

    /// A record's own header bytes, which are masked one level out from its content — under the
    /// mask in effect when the header itself was read.
    fn header_flags(node: &RecordNode, logical: &[u8]) -> Flags {
        let header_mask = node.mask ^ (node.rtype as u8);
        let at = |i: usize| logical.get(node.offset + i).map_or(0, |b| b ^ header_mask);
        Flags::decode(&[at(0), at(1)])
    }

    /// Every record the corpus holds is framed the way its type declares.
    ///
    /// This is what makes the declarations emission-ready: a writer that stamps them reproduces
    /// every header in every file, and a report that frames a type some other way fails here
    /// rather than being discovered by a reader of what we wrote.
    #[test]
    fn every_record_carries_the_framing_its_type_declares() {
        let mut seen = 0usize;
        // (dialect, type) -> the framings found, with a file that showed each.
        let mut wrong: BTreeMap<(Dialect, u16), BTreeMap<Framing, String>> = BTreeMap::new();

        crate::field_table::corpus::for_each_record(|dialect, node, logical, path| {
            // The catalog declares no framing: it is written by several components at once, so a
            // type number there has no single header shape to hold anything to.
            if dialect == Dialect::Catalog {
                return;
            }
            seen += 1;
            let found = Framing {
                schema: node.schema,
                schema_stated: header_flags(node, logical).has_schema,
            };
            if found != framing(node.rtype, dialect) {
                wrong
                    .entry((dialect, node.rtype))
                    .or_default()
                    .entry(found)
                    .or_insert_with(|| path.display().to_string());
            }
        });

        assert!(
            seen >= FLOOR,
            "the sweep reached {seen} records, below the {FLOOR} the committed fixtures alone hold"
        );
        assert!(
            wrong.is_empty(),
            "record types framed other than their declaration: {wrong:#?}"
        );
    }
}
