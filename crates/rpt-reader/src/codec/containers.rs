//! Finding a record the way the format finds one.
//!
//! A record stream is not addressed by position and it is not, at the top level, a tree. It is a
//! sequence of **containers**: runs of records closed by a designated record type — the *end
//! marker*. A reader never scans for a record; it asks for one, by type, bounded by the marker that
//! closes the container it is reading:
//!
//! ```text
//! search(want, end):
//!     loop over the records from here:
//!         this type == want  -> that is the record; the cursor is left past it
//!         this type == end   -> the container is finished: the record is ABSENT
//!         otherwise          -> step over it and keep looking
//! ```
//!
//! Three properties follow, and each is a behaviour a position-based reader cannot express:
//!
//! - **A record type the reader does not know is stepped over.** That is the format's
//!   forward-compatibility at record granularity, and it is why a file written by a newer version
//!   opens in an older engine.
//! - **Absence is an ordinary outcome, not a parse failure.** An optional record that was never
//!   written is simply not found before the marker; nothing in the file flags it as optional.
//! - **The marker bounds the search, so absence is distinguishable from running off the end.**
//!   Without it a reader asking for a record that is not there would consume the rest of the
//!   stream.
//!
//! A failed search is **not** a full rewind: the records stepped over on the way are consumed, and
//! the cursor comes to rest **on** the end marker, so every later search in that container also
//! reports absent until the container's own reader steps past the marker. Readers therefore ask in
//! stream order.
//!
//! End markers sit at the **root** of the stream — they close flat runs, not nested content — and
//! they carry no content of their own. Containers do nest, but their markers are distinguished by
//! type rather than by depth: a search bounded by an outer container's marker steps over an inner
//! container's marker like any other record.
//!
//! # Nothing in this crate uses it, and that is deliberate
//!
//! [`build_model`](crate::build_model) finds its records by walking the whole tree for a type
//! ([`build_model::tree_search`]), which is the opposite of the rule above. The two are **not**
//! interchangeable: on a report where a record type appears inside more than one container, a
//! bounded search finds a different set than a whole-tree walk, so migrating a decoder onto this is
//! a decode change with a baseline diff to read — one record type at a time, not a sweep.
//!
//! This is kept, unused, because it is the format's rule and the walk is ours. A reader meeting the
//! two docs should take this one as the description of the file and `tree_search`'s as the
//! description of what the layer does.
//!
//! [`build_model::tree_search`]: crate::build_model

use super::tree::RecordNode;

/// A cursor over one record sequence that finds records the way the format does.
///
/// Built over a slice of sibling records — the root of a stream (see
/// [`RecordStream::record_tree`](crate::raw::RecordStream::record_tree)) or one record's children —
/// and driven with [`RecordSearch::find`], which is the format's own bounded forward search.
#[derive(Debug, Clone)]
pub struct RecordSearch<'a> {
    records: &'a [RecordNode],
    pos: usize,
}

impl<'a> RecordSearch<'a> {
    /// Start a search at the beginning of `records`.
    pub fn new(records: &'a [RecordNode]) -> RecordSearch<'a> {
        RecordSearch { records, pos: 0 }
    }

    /// The next record of type `want` in the container closed by `end`, or `None` if the container
    /// holds none.
    ///
    /// Records of any other type between here and the answer are stepped over and consumed. On a
    /// hit the cursor is left just past the record; on a miss it is left on the end marker, and
    /// every further search in this container reports `None` until [`RecordSearch::pass_end`]
    /// steps over it.
    pub fn find(&mut self, want: u16, end: u16) -> Option<&'a RecordNode> {
        while let Some(rec) = self.records.get(self.pos) {
            if rec.rtype == end {
                return None;
            }
            self.pos += 1;
            if rec.rtype == want {
                return Some(rec);
            }
        }
        // Off the end of the sequence with no marker in sight: the container was never closed.
        None
    }

    /// Step over the end marker `end` if the cursor is on it, closing the container. Returns
    /// whether it was there — `false` means the run ended without its marker.
    pub fn pass_end(&mut self, end: u16) -> bool {
        let on_marker = self.records.get(self.pos).is_some_and(|r| r.rtype == end);
        if on_marker {
            self.pos += 1;
        }
        on_marker
    }

    /// The record the cursor is on, without consuming it.
    pub fn peek(&self) -> Option<&'a RecordNode> {
        self.records.get(self.pos)
    }

    /// How many records the cursor has consumed.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// True once every record in the sequence has been consumed.
    pub fn is_finished(&self) -> bool {
        self.pos >= self.records.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(rtype: u16) -> RecordNode {
        RecordNode {
            rtype,
            schema: 0x0700,
            offset: 0,
            content_start: 0,
            content_end: 0,
            mask: 0,
            children: Vec::new(),
        }
    }

    /// The search steps over record types it was not asked for, which is what lets a reader skip
    /// records it does not know.
    #[test]
    fn an_unwanted_record_is_stepped_over() {
        let recs = [rec(0x0064), rec(0x0066), rec(0x0071), rec(0x0065)];
        let mut s = RecordSearch::new(&recs);
        assert_eq!(s.find(0x0071, 0x0065).map(|r| r.rtype), Some(0x0071));
    }

    /// An absent optional record is a normal outcome bounded by the marker, not a run to the end of
    /// the stream — and the cursor stops **on** the marker rather than past it.
    #[test]
    fn the_end_marker_bounds_the_search() {
        let recs = [rec(0x0064), rec(0x0065), rec(0x0071)];
        let mut s = RecordSearch::new(&recs);
        assert!(
            s.find(0x0071, 0x0065).is_none(),
            "past the marker is absent"
        );
        assert_eq!(s.peek().map(|r| r.rtype), Some(0x0065));
        assert!(s.pass_end(0x0065));
        assert_eq!(s.peek().map(|r| r.rtype), Some(0x0071));
    }

    /// Once a container is exhausted every further search in it reports absent, so a reader that
    /// asks out of stream order gets nothing rather than a record from beyond the marker.
    #[test]
    fn a_finished_container_keeps_reporting_absent() {
        let recs = [rec(0x0064), rec(0x0065)];
        let mut s = RecordSearch::new(&recs);
        assert!(s.find(0x0071, 0x0065).is_none());
        assert!(s.find(0x0064, 0x0065).is_none(), "0x0064 is behind us");
    }

    /// A search that stepped over records keeps them consumed even when it fails; the format's
    /// readers ask in stream order and the cursor never goes backwards.
    #[test]
    fn a_failed_search_still_consumes_what_it_stepped_over() {
        let recs = [rec(0x0064), rec(0x0066), rec(0x0065)];
        let mut s = RecordSearch::new(&recs);
        assert!(s.find(0x0071, 0x0065).is_none());
        assert_eq!(s.position(), 2, "both records were stepped over");
    }

    /// An inner container's marker is not the outer container's, so a search bounded by the outer
    /// one walks straight past it.
    #[test]
    fn an_inner_marker_does_not_bound_an_outer_search() {
        let recs = [rec(0x0121), rec(0x00b5), rec(0x0083), rec(0x0065)];
        let mut s = RecordSearch::new(&recs);
        assert_eq!(s.find(0x0083, 0x0065).map(|r| r.rtype), Some(0x0083));
    }

    /// Every record type this reader treats as an end marker, including some no available report
    /// exercises — they are listed anyway, because the claim below is about all of them.
    const END_MARKERS: &[u16] = &[
        0x0002, 0x0005, 0x0013, 0x0019, 0x002e, 0x002f, 0x0033, 0x003b, 0x003c, 0x0065, 0x0067,
        0x006f, 0x007c, 0x00a6, 0x00b5, 0x00b7, 0x00bc, 0x011d, 0x0130, 0x0147, 0x0175, 0x017a,
        0x017c, 0x0183, 0x018c, 0x0191,
    ];

    /// End markers close **flat runs at the root of a stream**: not one of them is ever nested
    /// inside another record. That is what makes the container model a property of the root
    /// sequence rather than of the record tree, and it is the premise [`RecordSearch`] is built on
    /// — a search over a slice of siblings.
    ///
    /// Swept over every report tree, because a claim of the form "no record does X" is only worth
    /// as much as the corpus it was measured on.
    #[test]
    fn end_markers_are_never_nested() {
        let files = rpt_test_support::corpus_reports();
        let mut nested: Vec<String> = Vec::new();
        let mut streams = 0usize;
        for f in &files {
            let Ok(rpt) = crate::Rpt::open(f) else {
                continue;
            };
            for (id, s) in rpt.streams() {
                let tree = match id {
                    crate::StreamId::Contents => s.record_tree(),
                    crate::StreamId::Other(p) if p.ends_with("/Contents") => {
                        crate::records::RecordStream::decode(
                            crate::StreamId::Contents,
                            s.raw_bytes(),
                        )
                        .record_tree()
                    }
                    _ => continue,
                };
                if tree.is_empty() {
                    continue;
                }
                streams += 1;
                for root in &tree {
                    for child in &root.children {
                        child.walk(&mut |n| {
                            if END_MARKERS.contains(&n.rtype) {
                                nested.push(format!(
                                    "{}: 0x{:04x} nested under 0x{:04x}",
                                    f.display(),
                                    n.rtype,
                                    root.rtype
                                ));
                            }
                        });
                    }
                }
            }
        }
        assert!(streams > 0, "the sweep found no report-definition streams");
        assert!(
            nested.is_empty(),
            "end markers must close root-level runs; found {} nested:\n{}",
            nested.len(),
            nested.join("\n")
        );
    }

    /// A run with no marker at all ends the search rather than looping.
    #[test]
    fn an_unclosed_run_ends_the_search() {
        let recs = [rec(0x0064), rec(0x0066)];
        let mut s = RecordSearch::new(&recs);
        assert!(s.find(0x0071, 0x0065).is_none());
        assert!(s.is_finished());
        assert!(!s.pass_end(0x0065));
    }
}
