//! Node-building helpers and the sparse-emission conventions shared across the mapping.
//!
//! [`Node`] is a small fluent wrapper over [`kdl::KdlNode`] that makes "emit only non-default values"
//! uniform: `prop_if`/`flag`/`child_opt` skip anything that carries no information, so the mapping
//! reads as a list of facts rather than a wall of conditionals.

use kdl::{KdlEntry, KdlNode, KdlValue};
use rpt_model::{Color, Twips};

/// A KDL integer value from any signed/unsigned model integer (KDL stores integers as `i128`).
pub(crate) fn int(v: impl Into<i128>) -> KdlValue {
    KdlValue::Integer(v.into())
}

/// A KDL integer value from a [`Twips`] length — geometry stays in raw twips, no unit conversion.
pub(crate) fn twips(t: Twips) -> KdlValue {
    KdlValue::Integer(t.0 as i128)
}

/// A [`Color`] as a KDL value: the `#rrggbb` hex string when opaque, or a packed `0xAARRGGBB`
/// integer when it carries a non-opaque alpha (which the hex form would silently drop).
pub(crate) fn color(c: Color) -> KdlValue {
    if c.a == 255 {
        KdlValue::from(c.to_hex())
    } else {
        let argb =
            ((c.a as i128) << 24) | ((c.r as i128) << 16) | ((c.g as i128) << 8) | (c.b as i128);
        KdlValue::Integer(argb)
    }
}

/// A fluent [`KdlNode`] builder with sparse-emission helpers.
pub(crate) struct Node {
    node: KdlNode,
}

impl Node {
    /// Start a node named `name` (the construct kind).
    pub(crate) fn new(name: &str) -> Self {
        Node {
            node: KdlNode::new(name),
        }
    }

    /// Append a positional argument (the identifying name is always the first argument).
    pub(crate) fn arg(mut self, v: impl Into<KdlValue>) -> Self {
        self.node.push(KdlEntry::new(v));
        self
    }

    /// Append a positional argument only when `cond` holds.
    pub(crate) fn arg_if(self, cond: bool, v: impl Into<KdlValue>) -> Self {
        if cond {
            self.arg(v)
        } else {
            self
        }
    }

    /// Append a `key=value` property.
    pub(crate) fn prop(mut self, key: &str, v: impl Into<KdlValue>) -> Self {
        self.node.push(KdlEntry::new_prop(key, v));
        self
    }

    /// Append a `key=value` property only when `cond` holds (the sparse-emission primitive).
    pub(crate) fn prop_if(self, cond: bool, key: &str, v: impl Into<KdlValue>) -> Self {
        if cond {
            self.prop(key, v)
        } else {
            self
        }
    }

    /// Append a boolean flag as `key=#true`, but only when it is set (a false flag is the default and
    /// emits nothing).
    pub(crate) fn flag(self, key: &str, b: bool) -> Self {
        self.prop_if(b, key, true)
    }

    /// Append a `key="value"` string property when `v` is `Some` and non-empty.
    pub(crate) fn opt_str(self, key: &str, v: Option<&str>) -> Self {
        match v {
            Some(s) if !s.is_empty() => self.prop(key, s),
            _ => self,
        }
    }

    /// Append a `key="value"` string property when `v` is non-empty.
    pub(crate) fn str_if(self, key: &str, v: &str) -> Self {
        self.prop_if(!v.is_empty(), key, v)
    }

    /// Append a child node.
    pub(crate) fn child(mut self, child: Node) -> Self {
        self.node.ensure_children().nodes_mut().push(child.build());
        self
    }

    /// Append a child node when `child` is `Some`.
    pub(crate) fn child_opt(self, child: Option<Node>) -> Self {
        match child {
            Some(c) => self.child(c),
            None => self,
        }
    }

    /// Append zero or more child nodes (the children doc is only created when an item is pushed, so an
    /// empty iterator leaves the node childless — no stray `{}`).
    pub(crate) fn children(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        for c in children {
            self.node.ensure_children().nodes_mut().push(c.build());
        }
        self
    }

    /// Finish and return the underlying [`KdlNode`].
    pub(crate) fn build(self) -> KdlNode {
        self.node
    }
}
