//! The scan's inverse: writing a record tree back out as the bytes it was read from.

use super::node::{PieceSpan, RecordNode};

/// Re-serialize a record tree back into the logical byte stream it was parsed from.
///
/// The tree is a structural view over the retained `logical` bytes: each node's header
/// (`offset..content_start`) and the pieces of its content partition the stream contiguously.
/// Walking that structure and copying each span reconstructs `logical` byte-for-byte — the
/// re-serializable record bytes the writer (`encode_contents`) rests on. A structurally inconsistent
/// tree (overlapping or out-of-parent spans) produces bytes that differ from `logical`, so the
/// round-trip doubles as a check on the tree and on the partition every reader walks it by.
pub(crate) fn serialize_tree(nodes: &[RecordNode], logical: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(logical.len());
    let mut cursor = 0;
    for node in nodes {
        // Bytes between top-level records are not any record's content, so they are copied here
        // rather than reached as a piece.
        if let Some(gap) = logical.get(cursor..node.offset) {
            out.extend_from_slice(gap);
        }
        serialize_node(node, logical, &mut out);
        cursor = node.content_end;
    }
    if let Some(tail) = logical.get(cursor..) {
        out.extend_from_slice(tail);
    }
    out
}

/// Emit one record verbatim: its header, then each piece of its content — a run of its own field
/// bytes as it lies in the stream, a nested record recursively.
fn serialize_node(node: &RecordNode, logical: &[u8], out: &mut Vec<u8>) {
    if let Some(head) = logical.get(node.offset..node.content_start) {
        out.extend_from_slice(head);
    }
    for piece in node.pieces() {
        match piece {
            PieceSpan::Run { start, end } => {
                if let Some(run) = logical.get(start..end) {
                    out.extend_from_slice(run);
                }
            }
            PieceSpan::Child(child) => serialize_node(child, logical, out),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::wired::parse_tree;
    use super::serialize_tree;

    #[test]
    fn serialize_tree_round_trips_nested_records() {
        let inner_mask = 0x10u8;
        let inner: Vec<u8> = [0xf8u8, 0x03, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00]
            .iter()
            .map(|b| b ^ inner_mask)
            .collect();
        let mut stream = vec![0xf8u8, 0x10, 0x07, 0x00, 0x00, 0x00, 0x00, 0x08];
        stream.extend(inner);

        let tree = parse_tree(&stream);
        assert_eq!(serialize_tree(&tree, &stream), stream);
    }
}
