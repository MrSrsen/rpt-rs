//! The type-strict report DOM — a tree of domain DTOs, one struct per record type.
//!
//! [`Node`] is a sum type over the modelled domain DTOs plus an [`Unknown`] variant for record
//! types not yet modelled. `Unknown` recurses its children and keeps the decoded leaf values, so
//! every record is represented while the DOM stays type-strict for the modelled parts. Byte-exact
//! round-trip is guaranteed independently by the lossless record substrate.

use super::data_def::FieldDef;
use crate::RecordTag;

/// A decoded leaf value within a record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Value {
    /// A length-prefixed / NUL-terminated printable string.
    Text(String),
    /// A 32-bit integer field.
    Int(i32),
    /// Undecoded leaf bytes (kept verbatim so nothing is lost).
    Bytes(Vec<u8>),
}

/// A node in the type-strict DOM: a known domain DTO, or an [`Unknown`] record.
///
/// Exhaustive: every consumer matches all variants, so adding a new domain DTO is a compile error
/// until it is handled everywhere.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Node {
    /// SDK field definition.
    FieldDef(FieldDef),
    /// A record type not yet modelled — preserved with its decoded leaf values and recursed
    /// children.
    Unknown(Unknown),
}

/// An unmodelled record, kept verbatim (decoded leaf values + child nodes).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Unknown {
    /// The raw record type.
    pub rtype: u16,
    /// The record subtype word.
    pub subtype: u16,
    /// Decoded leaf values (this record's own content).
    pub values: Vec<Value>,
    /// Child nodes (nested records).
    pub children: Vec<Node>,
}

impl Unknown {
    /// The record tag.
    pub fn tag(&self) -> RecordTag {
        RecordTag(self.rtype)
    }

    /// A display name for export: the identified type name from the registry (e.g. `Formula`,
    /// `Area`), or `Type_0xNNNN` when the type is not yet named.
    pub fn type_name(&self) -> String {
        match self.tag().name() {
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
            for child in &u.children {
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
