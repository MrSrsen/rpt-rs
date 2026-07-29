//! The typed record tree — a tree of domain structs, one per record type.
//!
//! [`Node`] is a sum type over the modelled domain structs plus an [`Unknown`] variant for
//! unmodelled record types. `Unknown` keeps the record's content as an ordered list of [`Part`]s —
//! runs of its own field bytes and the records nested between them — so every record is
//! represented while the modelled parts stay strongly typed. Byte-exact round-trip is guaranteed
//! independently by the raw records the stream retains.
//!
//! Keeping content and children in **one** wire-ordered list is what makes a field's position
//! readable. A run is contiguous in the file; bytes on either side of a child are not adjacent, so
//! a reader that concatenates the runs addresses a buffer the file does not contain. With the
//! parts in order, a run's length and a child's framed length together give every byte's place in
//! the record.
//!
//! This is a view over the raw records, built on demand from a [`RecordStream`]
//! (see [`crate::Rpt::typed_record_tree`]); it is not part of the format-neutral semantic model.
//!
//! [`RecordStream`]: crate::raw::RecordStream

use super::RecordTag;
use crate::bytes::lp_string_at;
use crate::codec::Dialect;
use crate::model::FieldDef;

/// A decoded field value within a record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Value {
    /// A length-prefixed / NUL-terminated printable string.
    Text(String),
    /// Undecoded field bytes (kept verbatim so nothing is lost).
    Bytes(Vec<u8>),
}

/// A node in the typed record tree: a known domain struct, or an [`Unknown`] record.
///
/// Exhaustive: every consumer matches all variants, so adding a new domain struct is a compile
/// error until it is handled everywhere.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Node {
    /// SDK field definition, from a `0x0073` record. Boxed to keep the enum small: `FieldDef` is
    /// far larger than the `Unknown` variant, and most nodes in a decoded tree are `Unknown`.
    FieldDef(Box<FieldDef>),
    /// An unmodelled record type — preserved with its decoded field values and recursed
    /// children.
    Unknown(Unknown),
}

/// One element of a record's content, in wire order.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Part {
    /// A run of the record's own field bytes, demasked and **contiguous in the file**.
    Run(Vec<u8>),
    /// A nested record.
    Child {
        /// What the nested record occupies in this record's content: its header plus its content.
        framed_len: usize,
        /// The nested record itself.
        node: Node,
    },
}

/// An unmodelled record, kept verbatim as its content in wire order.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Unknown {
    /// The raw record type.
    pub rtype: u16,
    /// The record's schema word (dialect marker + schema version), big-endian as stored.
    pub schema: u16,
    /// The record's content: runs of its own field bytes and the records nested between them.
    pub parts: Vec<Part>,
}

impl Unknown {
    /// The record tag.
    pub fn tag(&self) -> RecordTag {
        RecordTag(self.rtype)
    }

    /// The record's own field-byte runs, in order. Two runs are adjacent in this list but not in
    /// the file: a nested record sits between them.
    pub fn runs(&self) -> impl Iterator<Item = &[u8]> {
        self.parts.iter().filter_map(|p| match p {
            Part::Run(b) => Some(b.as_slice()),
            Part::Child { .. } => None,
        })
    }

    /// The nested records, in order.
    pub fn children(&self) -> impl Iterator<Item = &Node> {
        self.parts.iter().filter_map(|p| match p {
            Part::Child { node, .. } => Some(node),
            Part::Run(_) => None,
        })
    }

    /// The record's own content decoded into [`Value`]s, run by run in wire order.
    ///
    /// Decoding per run rather than over the concatenation is deliberate: a length-prefixed string
    /// cannot span a nested record, so a "string" that only appears once the runs are joined is an
    /// artifact of the join.
    pub fn values(&self) -> Vec<Value> {
        self.runs().flat_map(decode_run).collect()
    }

    /// A display name for export: the identified type name from the registry (e.g. `Formula`,
    /// `Area`), or `Type_0xNNNN` when the type is not named.
    ///
    /// `dialect` is the vocabulary the stream this record came from is written in. A record carries
    /// its type number and not the stream it was read from, so the caller states it: the same
    /// number names an unrelated record in each vocabulary, and answering from the wrong one names
    /// the record after something it is not.
    pub fn type_name(&self, dialect: Dialect) -> String {
        match self.tag().name(dialect) {
            Some(name) => name.to_string(),
            None => format!("Type_{:#06x}", self.rtype),
        }
    }
}

impl Node {
    /// Visit this node and all descendants (through `Unknown` children) in pre-order.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a Node)) {
        f(self);
        if let Node::Unknown(u) = self {
            for child in u.children() {
                child.walk(f);
            }
        }
    }

    /// Total number of nodes in this subtree.
    pub fn count(&self) -> usize {
        let mut n = 0;
        self.walk(&mut |_| n += 1);
        n
    }
}

/// Decode one contiguous run of a record's field bytes into a sequence of [`Value`]s:
/// length-prefixed printable strings become [`Value::Text`]; the remaining bytes are kept verbatim
/// as [`Value::Bytes`], so the split is lossless.
pub(crate) fn decode_run(bytes: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut raw: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if let Some((text, consumed)) = lp_string_at(bytes, i) {
            if !raw.is_empty() {
                out.push(Value::Bytes(std::mem::take(&mut raw)));
            }
            out.push(Value::Text(text));
            i += consumed;
        } else {
            raw.push(bytes[i]);
            i += 1;
        }
    }
    if !raw.is_empty() {
        out.push(Value::Bytes(raw));
    }
    out
}

/// One entry in a record inventory: a record type and how many of it occur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RecordTypeCount {
    /// The raw record type.
    pub tag: u16,
    /// Number of records of this type in the decoded stream.
    pub count: usize,
}

impl RecordTypeCount {
    /// The record type's symbolic name in `dialect`, if this type has been identified there;
    /// `None` for an unnamed type. Derived from [`tag`](Self::tag) via [`RecordTag::name`].
    pub fn name(&self, dialect: Dialect) -> Option<&'static str> {
        RecordTag(self.tag).name(dialect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lp(text: &str) -> Vec<u8> {
        let mut out = (text.len() as u32 + 1).to_be_bytes().to_vec();
        out.extend_from_slice(text.as_bytes());
        out.push(0);
        out
    }

    fn child(rtype: u16) -> Part {
        Part::Child {
            framed_len: 6,
            node: Node::Unknown(Unknown {
                rtype,
                schema: 0x0700,
                parts: Vec::new(),
            }),
        }
    }

    /// Runs and children keep their wire order, and each is reachable on its own.
    #[test]
    fn parts_keep_wire_order() {
        let u = Unknown {
            rtype: 0x0088,
            schema: 0x0700,
            parts: vec![Part::Run(vec![1, 2]), child(0x0151), Part::Run(vec![3, 4])],
        };
        assert_eq!(u.runs().collect::<Vec<_>>(), vec![&[1, 2][..], &[3, 4][..]]);
        assert_eq!(u.children().count(), 1);
        assert_eq!(
            u.values(),
            vec![Value::Bytes(vec![1, 2]), Value::Bytes(vec![3, 4])]
        );
    }

    /// A string is only decoded inside one run. Bytes on either side of a nested record are not
    /// adjacent in the file, so a length prefix in one run never frames text in the next.
    #[test]
    fn a_string_is_not_invented_across_a_nested_record() {
        let mut prefix = 3u32.to_be_bytes().to_vec();
        prefix.pop(); // the length's last byte lands in the next run
        let u = Unknown {
            rtype: 0x0088,
            schema: 0x0700,
            parts: vec![
                Part::Run(prefix),
                child(0x0151),
                Part::Run(b"\x03ab\0".to_vec()),
            ],
        };
        assert!(u.values().iter().all(|v| matches!(v, Value::Bytes(_))));
        // The same bytes with nothing between them do frame a string — which is exactly why the
        // two runs must not be joined.
        let joined: Vec<u8> = u.runs().flatten().copied().collect();
        assert_eq!(decode_run(&joined), vec![Value::Text("ab".into())]);
    }

    /// A record's own byte runs are decoded; its children's are not.
    #[test]
    fn values_cover_the_record_alone() {
        let u = Unknown {
            rtype: 0x0088,
            schema: 0x0700,
            parts: vec![Part::Run(lp("hi")), child(0x0151)],
        };
        assert_eq!(u.values(), vec![Value::Text("hi".into())]);
    }
}
