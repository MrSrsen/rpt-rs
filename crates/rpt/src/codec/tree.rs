//! L1 — the recursive record-tree reader.
//!
//! A report record's content is itself a sequence of **nested** TSLV records (or leaf field
//! data). Content is read under a **stack-XOR mask**: the per-byte XOR mask is the XOR of the
//! record types currently on the parse stack (it un-XORs on pop).
//!
//! Header detection uses the bit-packed TSLV header (the same [`super::tslv::Flags`] decoding
//! [`super::tile`] uses), flag byte `0xf8`/`0xf9`, demasked with the current stack mask. The
//! [`Dialect`] selects the validation: `Contents` records always carry subtype low byte `0x07`
//! (suppressing false-positive headers in leaf data), while `QESession` records use varied
//! subtypes and relax it. Decoding never panics and is bounded by each record's declared
//! length.

/// A record in the nested tree: its type, the stack-XOR mask its content is read under, the
/// content span within the logical stream, and any nested child records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordNode {
    pub rtype: u16,
    pub subtype: u16,
    /// Offset of the record header within the logical stream.
    pub offset: usize,
    /// The content byte span `[content_start, content_end)` within the logical stream.
    pub content_start: usize,
    pub content_end: usize,
    /// The XOR mask the content (and this record's own leaf bytes) are read under.
    pub mask: u8,
    /// Nested child records (empty for a leaf).
    pub children: Vec<RecordNode>,
}

impl RecordNode {
    /// The record's type tag.
    pub fn tag(&self) -> crate::records::RecordTag {
        crate::records::RecordTag(self.rtype)
    }

    /// True if this record has no nested records (its content is leaf field data).
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }

    /// Visit this node and all descendants in pre-order.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a RecordNode)) {
        f(self);
        for child in &self.children {
            child.walk(f);
        }
    }

    /// This record's own leaf bytes (the content spans not covered by any child), demasked
    /// with its stack mask. For a leaf record this is the whole content.
    pub fn leaf_bytes(&self, logical: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cursor = self.content_start;
        for child in &self.children {
            push_demasked(&mut out, logical, cursor, child.offset, self.mask);
            cursor = child.content_end;
        }
        push_demasked(&mut out, logical, cursor, self.content_end, self.mask);
        out
    }
}

fn push_demasked(out: &mut Vec<u8>, data: &[u8], from: usize, to: usize, mask: u8) {
    if let Some(slice) = data.get(from..to) {
        out.extend(slice.iter().map(|b| b ^ mask));
    }
}

/// Which record dialect a stream uses — selects how record headers are validated. `Contents`
/// records all carry subtype low byte `0x07` (the constraint suppresses false-positive headers
/// in leaf field data); `QESession` records use varied subtypes, so its parse relaxes that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Contents,
    Qe,
}

/// Try to read a record header at `pos` under stack mask `m`, bounded by `limit`.
/// Returns `(rtype, subtype, content_len, header_len)` if the bytes match the bit-packed TSLV
/// header shape (the same `Flags` decoding [`super::tile`] uses, generalised over the subtype
/// word so it serves both `Contents` and `QESession` records).
fn read_header(
    d: &[u8],
    pos: usize,
    m: u8,
    limit: usize,
    dialect: Dialect,
) -> Option<(u16, u16, usize, usize)> {
    use super::tslv::{self, Flags};
    // Demask a byte of the header (headers are read under the current stack mask).
    let at = |i: usize| -> Option<u8> { d.get(pos + i).map(|b| b ^ m) };

    let mut fw = [at(0)?, at(1)?];
    // flag: bits 3..7 set (0xf8/0xf9) — extended type + 4-byte length, the report-record shape.
    if fw[0] & 0xf8 != 0xf8 {
        return None;
    }
    if dialect == Dialect::Contents && at(2)? != 0x07 {
        return None;
    }
    let flags = Flags::decode(&fw);
    let mut q = 2usize;

    let rtype = if flags.extended_value {
        let v = u16::from_le_bytes([at(q)?, at(q + 1)?]);
        q += 2;
        v
    } else {
        tslv::clear_flag_bits(&mut fw);
        (u16::from(fw[0]) << 8) | u16::from(fw[1])
    };

    let subtype = if flags.extended_type {
        let st = u16::from_le_bytes([at(q)?, at(q + 1)?]);
        q += 2;
        st
    } else {
        0
    };

    let len = if flags.len_kind != 0 {
        let n = flags.len_kind as usize;
        let mut bytes = [0u8; 4];
        for (k, slot) in bytes[..n].iter_mut().enumerate() {
            *slot = at(q + k)?;
        }
        q += n;
        tslv::be_scalar(&bytes[..n]) as usize
    } else {
        0
    };

    let header_len = q;
    if pos + header_len + len > limit {
        return None;
    }
    Some((rtype, subtype, len, header_len))
}

/// Parse `logical[start..end)` as a sequence of records under stack mask `m`.
fn parse_seq(
    d: &[u8],
    start: usize,
    end: usize,
    m: u8,
    depth: usize,
    dialect: Dialect,
) -> Vec<RecordNode> {
    let mut out = Vec::new();
    let mut p = start;
    while p < end {
        let Some((rtype, subtype, len, header_len)) = read_header(d, p, m, end, dialect) else {
            // Leaf byte: not a record header here. Advance; the surrounding record's declared
            // length keeps us anchored.
            p += 1;
            continue;
        };
        let content_start = p + header_len;
        let content_end = content_start + len;
        let child_mask = m ^ (rtype as u8);
        let children = if depth < MAX_DEPTH {
            parse_seq(
                d,
                content_start,
                content_end,
                child_mask,
                depth + 1,
                dialect,
            )
        } else {
            Vec::new()
        };
        out.push(RecordNode {
            rtype,
            subtype,
            offset: p,
            content_start,
            content_end,
            mask: child_mask,
            children,
        });
        p = content_end;
    }
    out
}

const MAX_DEPTH: usize = 32;

/// Parse the whole logical report into a recursive record tree (top-level records read under
/// mask 0). For `Contents`-style streams (strict subtype-`0x07` headers).
pub(crate) fn parse_tree(logical: &[u8]) -> Vec<RecordNode> {
    parse_seq(logical, 0, logical.len(), 0, 0, Dialect::Contents)
}

/// Parse a `QESession` logical record stream into a recursive tree. Same bit-packed TSLV
/// framing + stack-XOR mask as [`parse_tree`], but with the relaxed (non-`0x07`) subtype
/// constraint that `QESession` records use.
pub(crate) fn parse_tree_qe(logical: &[u8]) -> Vec<RecordNode> {
    parse_seq(logical, 0, logical.len(), 0, 0, Dialect::Qe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_nested_record() {
        // Outer record type 0x10 (flag f8, subtype 07 00, len = 8) whose content is one inner
        // record type 0x03 with 0 content. Inner header is masked by the outer mask 0x10.
        let inner_mask = 0x10u8;
        let inner: Vec<u8> = [0xf8u8, 0x03, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00]
            .iter()
            .map(|b| b ^ inner_mask)
            .collect();
        let mut stream = vec![0xf8u8, 0x10, 0x07, 0x00, 0x00, 0x00, 0x00, 0x08];
        stream.extend(inner);

        let tree = parse_tree(&stream);
        assert_eq!(tree.len(), 1);
        let outer = &tree[0];
        assert_eq!(outer.rtype, 0x10);
        assert_eq!(outer.mask, 0x10);
        assert_eq!(outer.children.len(), 1);
        let child = &outer.children[0];
        assert_eq!(child.rtype, 0x03);
        assert_eq!(child.mask, 0x10 ^ 0x03);
        assert!(child.is_leaf());
    }
}
