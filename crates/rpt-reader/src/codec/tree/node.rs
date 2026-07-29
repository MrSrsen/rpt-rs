//! The tree's node type, and the field-byte runs it exposes.
//!
//! A record's content is a sequence of pieces — runs of the record's own field bytes and the
//! records nested between them — so a node holds where its content lies and what nests inside it.
//! It stores spans, never bytes: the logical stream it was read from stays the one copy, and a run
//! is demasked out of it on demand.

/// A record in the nested tree: its type, the stack-XOR mask its content is read under, the
/// content span within the logical stream, and any nested child records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordNode {
    /// The record's type tag (the TSLV rtype word).
    pub rtype: u16,
    /// The record's schema word, big-endian as stored — an **opaque version number** for this
    /// record type, carried so a reader can branch on it.
    ///
    /// It is ordered, and that ordering is its whole meaning: a reader declares the newest schema
    /// it understands, refuses a record newer than that, and adapts its field sequence to anything
    /// older. It is not a pair of independent halves, and no part of it identifies a stream — the
    /// high byte is constant per stream only because each stream is written by one component at one
    /// version. Compare it numerically; never decompose it.
    pub schema: u16,
    /// Offset of the record header within the logical stream.
    pub offset: usize,
    /// The content byte span `[content_start, content_end)` within the logical stream.
    pub content_start: usize,
    /// End (exclusive) of the content byte span within the logical stream.
    pub content_end: usize,
    /// The XOR mask the content (and this record's own field bytes) are read under.
    pub mask: u8,
    /// Nested child records (empty for a leaf of the tree).
    pub children: Vec<RecordNode>,
}

/// One element of a record's content, as it lies in the logical stream.
///
/// The pieces of a record partition its content span contiguously and in wire order: every byte
/// between `content_start` and `content_end` belongs to exactly one run or one child. That is the
/// property the whole crate reads a record through, and the one re-serializing a tree rests on.
#[derive(Debug, Clone, Copy)]
pub(crate) enum PieceSpan<'a> {
    /// A run of the record's own field bytes, the logical span `[start, end)` — still masked, since
    /// the stream keeps the one copy.
    Run { start: usize, end: usize },
    /// A nested record, sitting between two of its parent's runs.
    Child(&'a RecordNode),
}

impl RecordNode {
    /// The record's type tag.
    pub fn tag(&self) -> crate::records::RecordTag {
        crate::records::RecordTag(self.rtype)
    }

    /// What this record occupies in its parent's content span: its header plus its content.
    pub(crate) fn framed_len(&self) -> usize {
        self.content_end.saturating_sub(self.offset)
    }

    /// True if this record is a leaf **of the tree**: it has no nested records, so its content is
    /// one run of its own field bytes.
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

    /// This record's own field-byte **runs**, in wire order: each demasked with the record's stack
    /// mask and **contiguous in the file**. A record with no nested record has exactly one.
    ///
    /// This is the shape the format has. A record's content is a sequence of pieces — runs of its
    /// own fields and the records nested between them — and a reader walks it in order, so a run is
    /// where a field's position is measured from. Two runs are adjacent in this list but not on
    /// disk: a nested record sits between them.
    pub fn runs<'a>(&'a self, logical: &'a [u8]) -> impl Iterator<Item = Vec<u8>> + 'a {
        self.run_spans()
            .map(move |(from, to)| self.demasked(logical, from, to))
    }

    /// This record's **first** field-byte run: its content up to its first nested record, or the
    /// whole content when it has none.
    ///
    /// A fixed offset into a record's content is an offset into this run — it is where the reader
    /// starts and it is contiguous — so a positional read belongs here rather than in the
    /// concatenation, which can put the offset on the far side of a spliced-out child.
    ///
    /// That holds only where the nested records are the ones the format put there, i.e. where the
    /// enclosing type's field table declared them. In a stream whose tree is scanned instead, a
    /// header-shaped run of field data becomes a child that is not one, and cutting it out of a
    /// fixed-layout record loses real field bytes.
    pub fn first_run(&self, logical: &[u8]) -> Vec<u8> {
        self.runs(logical).next().unwrap_or_default()
    }

    /// This record's field-byte runs **concatenated**, demasked with its stack mask.
    ///
    /// The result is a buffer of this reader's own making: it does not exist anywhere in the file,
    /// because it is built by splicing every nested record out and joining the fragments either
    /// side. A fixed offset into it therefore addresses bytes that need not be adjacent on disk,
    /// and a value framed across the join — a string whose length prefix is in one run and whose
    /// text is in the next — is an artifact of the join, not a value in the record. New code reads
    /// [`runs`](RecordNode::runs) or [`first_run`](RecordNode::first_run) instead.
    pub fn joined_runs(&self, logical: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for run in self.runs(logical) {
            out.extend_from_slice(&run);
        }
        out
    }

    /// This record's content as the pieces it is made of, in wire order.
    ///
    /// The one statement of how a record's content divides: each child takes the span it was framed
    /// in, and every stretch of content no child covers is a run of the record's own field bytes.
    /// Every other view of a record — its runs, its typed parts, the bytes it re-serializes to — is
    /// this walk with the pieces mapped.
    pub(crate) fn pieces(&self) -> impl Iterator<Item = PieceSpan<'_>> {
        let mut cursor = self.content_start;
        // The trailing run starts where the last child ends, which the children alone decide — so
        // it is known before the walk and needs nothing carried out of it.
        let after_children = self
            .children
            .last()
            .map_or(self.content_start, |last| last.content_end);
        self.children
            .iter()
            .flat_map(move |child| {
                let gap = (child.offset > cursor).then_some(PieceSpan::Run {
                    start: cursor,
                    end: child.offset,
                });
                cursor = child.content_end;
                gap.into_iter()
                    .chain(std::iter::once(PieceSpan::Child(child)))
            })
            .chain(
                (self.content_end > after_children).then_some(PieceSpan::Run {
                    start: after_children,
                    end: self.content_end,
                }),
            )
    }

    /// The logical byte span of each of this record's field-byte runs, in wire order — the pieces
    /// that are not children. Demasking each and concatenating them yields
    /// [`joined_runs`](RecordNode::joined_runs).
    pub(crate) fn run_spans(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.pieces().filter_map(|piece| match piece {
            PieceSpan::Run { start, end } => Some((start, end)),
            PieceSpan::Child(_) => None,
        })
    }

    /// The bytes of one of this record's content spans, demasked with its stack mask. Empty when
    /// the span falls outside the stream, so a malformed record yields no bytes rather than failing.
    pub(crate) fn demasked(&self, logical: &[u8], from: usize, to: usize) -> Vec<u8> {
        logical
            .get(from..to)
            .map(|s| s.iter().map(|b| b ^ self.mask).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::wired::parse_tree;
    use super::PieceSpan;

    /// A record's content is a sequence of runs, each contiguous in the file. The concatenation is
    /// a buffer of this reader's own making: past a child its offsets are not file positions, and a
    /// value framed across the join — here a length prefix in one run and its text in the next —
    /// exists only because the join put them next to each other.
    #[test]
    fn a_run_is_contiguous_and_the_concatenation_is_not() {
        // `0x008a` declares `0x0151` and nothing else: three leading bytes, an empty `0x0151`, then
        // the rest. The two runs are six bytes apart in the file.
        let mask = 0x8au8;
        let head = [0x00u8, 0x00, 0x00];
        let child: Vec<u8> = [0xd9u8, 0x51, 0x00, 0x00, 0x00, 0x00]
            .iter()
            .map(|b| b ^ mask)
            .collect();
        let tail = [0x02u8, b'h', b'i', 0x00];
        let mut stream = vec![0xf8u8, 0x8a, 0x07, 0x00, 0x00, 0x00, 0x00, 0x0d];
        stream.extend(head.iter().map(|b| b ^ mask));
        stream.extend(child);
        stream.extend(tail.iter().map(|b| b ^ mask));

        let tree = parse_tree(&stream);
        let node = &tree[0];
        let runs: Vec<Vec<u8>> = node.runs(&stream).collect();
        assert_eq!(runs, vec![head.to_vec(), tail.to_vec()]);
        assert_eq!(node.first_run(&stream), head.to_vec());

        // The joined buffer puts the two runs side by side, which is what makes byte 3 of it read as
        // the last byte of a four-byte count. In the file those bytes are six apart.
        let joined = node.joined_runs(&stream);
        assert_eq!(joined, [0x00, 0x00, 0x00, 0x02, b'h', b'i', 0x00]);
        let spans: Vec<_> = node.run_spans().collect();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].1 + 6, spans[1].0);
    }

    /// The pieces partition the content: the runs and the children between them cover
    /// `[content_start, content_end)` end to end, with no byte in two pieces and none in neither.
    #[test]
    fn the_pieces_partition_the_content() {
        let mask = 0x8au8;
        let child: Vec<u8> = [0xd9u8, 0x51, 0x00, 0x00, 0x00, 0x00]
            .iter()
            .map(|b| b ^ mask)
            .collect();
        let mut stream = vec![0xf8u8, 0x8a, 0x07, 0x00, 0x00, 0x00, 0x00, 0x0d];
        stream.extend([0x00u8, 0x00, 0x00].iter().map(|b| b ^ mask));
        stream.extend(child);
        stream.extend([0x02u8, b'h', b'i', 0x00].iter().map(|b| b ^ mask));

        let tree = parse_tree(&stream);
        let node = &tree[0];
        let mut at = node.content_start;
        let mut kinds = Vec::new();
        for piece in node.pieces() {
            match piece {
                PieceSpan::Run { start, end } => {
                    assert_eq!(start, at);
                    assert!(end > start, "a run is never empty");
                    at = end;
                    kinds.push("run");
                }
                PieceSpan::Child(child) => {
                    assert_eq!(child.offset, at);
                    at = child.content_end;
                    kinds.push("child");
                }
            }
        }
        assert_eq!(at, node.content_end);
        assert_eq!(kinds, ["run", "child", "run"]);
    }

    /// A record with nothing nested has one run, and it is the whole content — so the two views
    /// agree exactly where there is no splice to disagree about.
    #[test]
    fn a_record_without_children_has_one_run() {
        let run = [0x00u8, 0x00, 0x01, 0x7e];
        let mask = 0x8cu8;
        let mut stream = vec![0xf8u8, 0x8c, 0x07, 0x00, 0x00, 0x00, 0x00, run.len() as u8];
        stream.extend(run.iter().map(|b| b ^ mask));

        let tree = parse_tree(&stream);
        let node = &tree[0];
        assert_eq!(node.runs(&stream).count(), 1);
        assert_eq!(node.first_run(&stream), run.to_vec());
        assert_eq!(node.joined_runs(&stream), run.to_vec());
    }
}
