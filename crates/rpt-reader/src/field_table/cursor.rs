//! The record cursor: typed reads over a record's content that report exhaustion instead of
//! failing, and the matching encoder.
//!
//! A record's content is a sequence of [`Piece`]s — runs of a record's own field bytes and nested child
//! records — in wire order. The cursor walks that sequence; it never silently steps over a child,
//! so a field list that omits one is reported rather than reading bytes that are not adjacent in
//! the file.
//!
//! Strings are the one read whose framing is not fixed by the field list: the record's own header
//! declares which of the format's two wire forms its content uses, so the cursor carries that
//! choice ([`StringFormat`]) and the field list simply says "a string".

pub(crate) use crate::codec::tslv::StringFormat;

/// A nested record inside another record's content: its type, schema and framed byte length
/// (header plus content), which is what it occupies in the parent's content span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChildRef {
    pub rtype: u16,
    pub schema: u16,
    pub framed_len: usize,
}

/// One element of a record's content in wire order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Piece {
    /// A run of the record's own field bytes, demasked and contiguous in the file.
    Run(Vec<u8>),
    /// A nested record.
    Child(ChildRef),
}

/// A record's content: what it is, and the pieces it is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordContent {
    pub rtype: u16,
    pub schema: u16,
    pub pieces: Vec<Piece>,
}

impl RecordContent {
    /// Total field bytes across every piece.
    pub(crate) fn field_byte_len(&self) -> usize {
        self.pieces
            .iter()
            .map(|p| match p {
                Piece::Run(b) => b.len(),
                Piece::Child(_) => 0,
            })
            .sum()
    }
}

/// Why a read did not return a value, and so why a walk stopped short of its field list's end.
///
/// Running out of content is not an error: a record that ends early leaves its trailing fields at
/// their defaults. The two child stops are one — each is the field list and the content
/// disagreeing about where a nested record sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stop {
    /// The record ran out of content. Legal: the field's default stands.
    pub ended: bool,
    /// A nested child record sits where the read wanted bytes — the field list did not declare it.
    pub blocked_by_child: bool,
    /// A child was expected but the content held something else.
    pub child_mismatch: bool,
}

/// A sequential reader over a record's content.
///
/// It walks the content's pieces rather than addressing a flat slice by offset, and that is the
/// load-bearing difference the name states: an undecoded child record stops it, where an
/// offset-addressed read ([`crate::bytes`]) would read straight through one.
#[derive(Debug)]
pub(crate) struct ContentCursor<'a> {
    pieces: &'a [Piece],
    piece: usize,
    off: usize,
    stop: Stop,
    strings: StringFormat,
}

impl<'a> ContentCursor<'a> {
    /// A cursor over content framed in the string form the record's own header declared.
    pub(crate) fn with_strings(content: &'a RecordContent, strings: StringFormat) -> Self {
        Self {
            pieces: &content.pieces,
            piece: 0,
            off: 0,
            stop: Stop::default(),
            strings,
        }
    }

    /// The sticky stop state accumulated so far.
    pub(crate) fn stop(&self) -> Stop {
        self.stop
    }

    /// Field bytes not yet consumed, across the whole remaining content (children excluded) — the
    /// engine's `UnreadBytesForCurRec`, restricted to field data.
    pub(crate) fn remaining(&self) -> usize {
        let mut n = 0;
        for (i, p) in self.pieces.iter().enumerate().skip(self.piece) {
            if let Piece::Run(b) = p {
                n += if i == self.piece {
                    b.len().saturating_sub(self.off)
                } else {
                    b.len()
                };
            }
        }
        n
    }

    /// Field bytes consumed so far — the position in the record's **joined runs**, the buffer the
    /// runs form once the nested records between them are spliced out. A child advances it by
    /// nothing, so it is the coordinate a field's own bytes are named in.
    pub(crate) fn pos(&self) -> usize {
        self.pieces[..self.piece.min(self.pieces.len())]
            .iter()
            .map(|p| match p {
                Piece::Run(b) => b.len(),
                Piece::Child(_) => 0,
            })
            .sum::<usize>()
            + self.off
    }

    /// True once every piece has been consumed — no field bytes and no children left.
    pub(crate) fn at_end(&self) -> bool {
        self.remaining() == 0
            && !self.pieces[self.piece.min(self.pieces.len())..]
                .iter()
                .any(|p| matches!(p, Piece::Child(_)))
    }

    /// The next piece, if the current run is exhausted.
    fn next_child(&self) -> Option<&'a ChildRef> {
        let idx = match self.pieces.get(self.piece) {
            Some(Piece::Run(b)) if self.off < b.len() => return None,
            Some(Piece::Run(_)) => self.piece + 1,
            _ => self.piece,
        };
        match self.pieces.get(idx) {
            Some(Piece::Child(c)) => Some(c),
            _ => None,
        }
    }

    /// Consume `n` bytes of field data. Returns `None` and records why when the content is
    /// exhausted, or when a nested child record sits in the way.
    pub(crate) fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        // Step over an exhausted run only when another run follows directly; a child
        // must be declared by the field list, never skipped.
        loop {
            match self.pieces.get(self.piece) {
                Some(Piece::Run(b)) if self.off + n <= b.len() => {
                    let out = &b[self.off..self.off + n];
                    self.off += n;
                    return Some(out);
                }
                Some(Piece::Run(b)) if self.off >= b.len() => {
                    match self.pieces.get(self.piece + 1) {
                        Some(Piece::Run(_)) => {
                            self.piece += 1;
                            self.off = 0;
                        }
                        Some(Piece::Child(_)) => {
                            self.stop.blocked_by_child = true;
                            return None;
                        }
                        None => {
                            self.stop.ended = true;
                            return None;
                        }
                    }
                }
                // A partial run: the field wants more bytes than this run holds.
                Some(Piece::Run(_)) => {
                    if self.next_child().is_some() {
                        self.stop.blocked_by_child = true;
                    } else {
                        self.stop.ended = true;
                    }
                    return None;
                }
                Some(Piece::Child(_)) => {
                    self.stop.blocked_by_child = true;
                    return None;
                }
                None => {
                    self.stop.ended = true;
                    return None;
                }
            }
        }
    }

    /// What the table left behind: unconsumed field bytes, and undeclared child records.
    pub(crate) fn leftover(&self) -> (usize, usize) {
        let children = self.pieces[self.piece.min(self.pieces.len())..]
            .iter()
            .filter(|p| matches!(p, Piece::Child(_)))
            .count();
        (self.remaining(), children)
    }

    /// Consume the nested child record the field list declares here, checking its type.
    pub(crate) fn child_expect(&mut self, rtype: u16) -> Option<&'a ChildRef> {
        let c = self.child()?;
        if c.rtype == rtype {
            Some(c)
        } else {
            self.stop.child_mismatch = true;
            None
        }
    }

    /// Consume the nested child record the field list declares here.
    pub(crate) fn child(&mut self) -> Option<&'a ChildRef> {
        // Skip an exhausted run to reach the child.
        if let Some(Piece::Run(b)) = self.pieces.get(self.piece) {
            if self.off >= b.len() {
                self.piece += 1;
                self.off = 0;
            }
        }
        match self.pieces.get(self.piece) {
            Some(Piece::Child(c)) => {
                self.piece += 1;
                self.off = 0;
                Some(c)
            }
            Some(Piece::Run(_)) => {
                self.stop.child_mismatch = true;
                None
            }
            None => {
                self.stop.ended = true;
                None
            }
        }
    }

    pub(crate) fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }

    pub(crate) fn u16_be(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }

    pub(crate) fn u32_be(&mut self) -> Option<u32> {
        self.take(4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// The signed fixed-width integers, two's complement in the same big-endian order as their
    /// unsigned counterparts. The record layer has a distinct read for each signedness, and a
    /// signed field read as unsigned turns a small negative into a value near the type's ceiling.
    pub(crate) fn i8(&mut self) -> Option<i8> {
        self.take(1).map(|b| b[0] as i8)
    }

    pub(crate) fn i16_be(&mut self) -> Option<i16> {
        self.take(2).map(|b| i16::from_be_bytes([b[0], b[1]]))
    }

    pub(crate) fn i32_be(&mut self) -> Option<i32> {
        self.take(4)
            .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A single-precision float, **little-endian**.
    pub(crate) fn f32_le(&mut self) -> Option<f32> {
        self.take(4).map(|b| {
            let mut a = [0u8; 4];
            a.copy_from_slice(b);
            f32::from_le_bytes(a)
        })
    }

    /// A single-precision float, **big-endian**.
    pub(crate) fn f32_be(&mut self) -> Option<f32> {
        self.take(4).map(|b| {
            let mut a = [0u8; 4];
            a.copy_from_slice(b);
            f32::from_be_bytes(a)
        })
    }

    /// A double, **little-endian**.
    pub(crate) fn f64_le(&mut self) -> Option<f64> {
        self.take(8).map(|b| {
            let mut a = [0u8; 8];
            a.copy_from_slice(b);
            f64::from_le_bytes(a)
        })
    }

    /// A double, **big-endian** — the order the chart definition's axis min/max values are stored
    /// in, and the one every other scalar in the format uses.
    pub(crate) fn f64_be(&mut self) -> Option<f64> {
        self.take(8).map(|b| {
            let mut a = [0u8; 8];
            a.copy_from_slice(b);
            f64::from_be_bytes(a)
        })
    }

    /// A narrowing integer: `narrow` bytes when the value fits below the top bit, otherwise
    /// `2 * narrow` bytes with `0x80` set in the leading byte as the marker. A field table declares
    /// one as [`Kind::VarU16`](super::table::Kind::VarU16) or
    /// [`Kind::VarU32`](super::table::Kind::VarU32).
    ///
    /// The marker bit is the width, so the widest form still leaves the value's own top bit clear:
    /// a narrowing field is unsigned by construction and spans `0 ..= 0x7fff_ffff`. There is no
    /// signed narrowing read — the record layer clamps a negative to zero rather than encoding one.
    pub(crate) fn narrowing(&mut self, narrow: usize) -> Option<u32> {
        let lead = match self.pieces.get(self.piece) {
            Some(Piece::Run(b)) => b.get(self.off).copied(),
            _ => None,
        };
        let Some(lead) = lead else {
            // Let `take` classify why nothing is here.
            self.take(narrow)?;
            return None;
        };
        let width = if lead & 0x80 != 0 { narrow * 2 } else { narrow };
        let bytes = self.take(width)?;
        let mut v = 0u32;
        for &b in bytes {
            v = (v << 8) | u32::from(b);
        }
        if lead & 0x80 != 0 {
            v &= !(0x80u32 << (8 * (width - 1)));
        }
        Some(v)
    }

    /// A blob: a big-endian `u32` byte count, then that many raw bytes. The count is the bytes
    /// themselves — unlike a string's, which includes the terminator it counts up to.
    pub(crate) fn blob(&mut self) -> Option<&'a [u8]> {
        let len = self.u32_be()? as usize;
        self.take(len)
    }

    /// A string, framed in whichever of the two wire forms this cursor was opened for.
    ///
    /// Returns the raw block, terminating NUL included in both forms; the text is everything before
    /// the first NUL. Reading with the wrong form is not a subtle error: a counted read over
    /// NUL-terminated bytes takes a length out of the first four characters of the text.
    pub(crate) fn string(&mut self) -> Option<&'a [u8]> {
        match self.strings {
            StringFormat::Enhanced => {
                let len = self.u32_be()? as usize;
                self.take(len)
            }
            StringFormat::Simple => self.nul_terminated(),
        }
    }

    /// The simple form: bytes up to and including the next NUL, found by scanning rather than
    /// stated. A run that holds no terminator is a record that ended mid-string.
    fn nul_terminated(&mut self) -> Option<&'a [u8]> {
        loop {
            match self.pieces.get(self.piece) {
                Some(Piece::Run(b)) if self.off < b.len() => {
                    let Some(rel) = b[self.off..].iter().position(|&c| c == 0) else {
                        // The scan stops at the end of the run: a terminator on the far side of a
                        // child record is not adjacent to these bytes and does not close them.
                        if matches!(self.pieces.get(self.piece + 1), Some(Piece::Child(_))) {
                            self.stop.blocked_by_child = true;
                        } else {
                            self.stop.ended = true;
                        }
                        return None;
                    };
                    let out = &b[self.off..self.off + rel + 1];
                    self.off += rel + 1;
                    return Some(out);
                }
                // An exhausted run continues into the next one, but never past a child.
                Some(Piece::Run(_)) => match self.pieces.get(self.piece + 1) {
                    Some(Piece::Run(_)) => {
                        self.piece += 1;
                        self.off = 0;
                    }
                    Some(Piece::Child(_)) => {
                        self.stop.blocked_by_child = true;
                        return None;
                    }
                    None => {
                        self.stop.ended = true;
                        return None;
                    }
                },
                Some(Piece::Child(_)) => {
                    self.stop.blocked_by_child = true;
                    return None;
                }
                None => {
                    self.stop.ended = true;
                    return None;
                }
            }
        }
    }
}

/// Emits the pieces a field list describes — the write direction of the same table.
#[derive(Debug)]
pub(crate) struct Encoder {
    pieces: Vec<Piece>,
    run: Vec<u8>,
    strings: StringFormat,
}

impl Encoder {
    /// An encoder emitting `strings`.
    ///
    /// The form is not a default the format supplies: a record's header records which one its
    /// content is in, so an encoder and the header it is written under must agree, or the strings
    /// come back mis-framed.
    pub(crate) fn with_strings(strings: StringFormat) -> Self {
        Self {
            pieces: Vec::new(),
            run: Vec::new(),
            strings,
        }
    }

    pub(crate) fn bytes(&mut self, b: &[u8]) {
        self.run.extend_from_slice(b);
    }

    pub(crate) fn u8(&mut self, v: u8) {
        self.run.push(v);
    }

    pub(crate) fn u16_be(&mut self, v: u16) {
        self.run.extend_from_slice(&v.to_be_bytes());
    }

    pub(crate) fn u32_be(&mut self, v: u32) {
        self.run.extend_from_slice(&v.to_be_bytes());
    }

    pub(crate) fn i8(&mut self, v: i8) {
        self.run.push(v as u8);
    }

    pub(crate) fn i16_be(&mut self, v: i16) {
        self.run.extend_from_slice(&v.to_be_bytes());
    }

    pub(crate) fn i32_be(&mut self, v: i32) {
        self.run.extend_from_slice(&v.to_be_bytes());
    }

    pub(crate) fn f32_le(&mut self, v: f32) {
        self.run.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn f32_be(&mut self, v: f32) {
        self.run.extend_from_slice(&v.to_be_bytes());
    }

    pub(crate) fn f64_le(&mut self, v: f64) {
        self.run.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn f64_be(&mut self, v: f64) {
        self.run.extend_from_slice(&v.to_be_bytes());
    }

    /// The narrowing integer, choosing the form the record layer would: narrow while the value
    /// fits below the top bit of the narrow form, wide otherwise.
    pub(crate) fn narrowing(&mut self, narrow: usize, v: u32) {
        let narrow_max = (1u32 << (8 * narrow as u32 - 1)) - 1;
        if v <= narrow_max {
            let be = v.to_be_bytes();
            self.run.extend_from_slice(&be[4 - narrow..]);
        } else {
            let wide = narrow * 2;
            let marked = v | (0x80u32 << (8 * (wide - 1)));
            let be = marked.to_be_bytes();
            self.run.extend_from_slice(&be[4 - wide..]);
        }
    }

    /// A string block (terminating NUL included), framed in this encoder's wire form.
    /// A blob: its byte count, then the bytes.
    pub(crate) fn blob(&mut self, bytes: &[u8]) {
        self.u32_be(bytes.len() as u32);
        self.bytes(bytes);
    }

    pub(crate) fn string(&mut self, block: &[u8]) {
        if self.strings == StringFormat::Enhanced {
            self.u32_be(block.len() as u32);
        }
        self.run.extend_from_slice(block);
    }

    pub(crate) fn child(&mut self, c: ChildRef) {
        self.flush();
        self.pieces.push(Piece::Child(c));
    }

    fn flush(&mut self) {
        if !self.run.is_empty() {
            self.pieces.push(Piece::Run(std::mem::take(&mut self.run)));
        }
    }

    pub(crate) fn finish(mut self) -> Vec<Piece> {
        self.flush();
        self.pieces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_run(b: &[u8]) -> RecordContent {
        RecordContent {
            rtype: 0,
            schema: 0x0700,
            pieces: vec![Piece::Run(b.to_vec())],
        }
    }

    #[test]
    fn reading_past_the_end_is_legal_and_reports_exhaustion() {
        let c = one_run(&[0xbe, 0xef]);
        let mut cur = ContentCursor::with_strings(&c, StringFormat::Enhanced);
        assert_eq!(cur.u16_be(), Some(0xbeef));
        assert_eq!(cur.remaining(), 0);
        assert_eq!(cur.u32_be(), None);
        assert!(cur.stop().ended);
        assert!(!cur.stop().blocked_by_child);
    }

    /// The narrowing forms the record layer emits: `0x7f` → `7f`,
    /// `0x0100` → `81 00`, `0x1234` → `12 34`, `0x00123456` → `80 12 34 56`.
    #[test]
    fn narrowing_matches_the_record_layer() {
        for (narrow, bytes, value) in [
            (1usize, &[0x7f][..], 0x7fu32),
            (1, &[0x81, 0x00], 0x0100),
            (2, &[0x12, 0x34], 0x1234),
            (2, &[0x80, 0x12, 0x34, 0x56], 0x0012_3456),
            // A twip past the narrow ceiling: `saveTwip(100000)` → `80 01 86 a0`.
            (2, &[0x80, 0x01, 0x86, 0xa0], 100_000),
        ] {
            let c = one_run(bytes);
            let mut cur = ContentCursor::with_strings(&c, StringFormat::Enhanced);
            assert_eq!(cur.narrowing(narrow), Some(value), "{bytes:02x?}");
            assert_eq!(cur.remaining(), 0, "{bytes:02x?} consumed exactly");

            let mut enc = Encoder::with_strings(StringFormat::Enhanced);
            enc.narrowing(narrow, value);
            assert_eq!(enc.finish(), vec![Piece::Run(bytes.to_vec())]);
        }
    }

    /// A wide value shifts every field after it — the property a fixed-offset read cannot have.
    #[test]
    fn a_wide_narrowing_value_shifts_the_field_after_it() {
        // Narrow left, then a trailing marker byte.
        let narrow = one_run(&[0x02, 0xd0, 0xaa]);
        let mut cur = ContentCursor::with_strings(&narrow, StringFormat::Enhanced);
        assert_eq!(cur.narrowing(2), Some(720));
        assert_eq!(cur.u8(), Some(0xaa));
        // The same record with a wide left: the marker moves two bytes later.
        let wide = one_run(&[0x80, 0x00, 0x8b, 0x30, 0xaa]);
        let mut cur = ContentCursor::with_strings(&wide, StringFormat::Enhanced);
        assert_eq!(cur.narrowing(2), Some(35_632));
        assert_eq!(cur.u8(), Some(0xaa));
    }

    #[test]
    fn a_read_never_steps_over_an_undeclared_child() {
        let c = RecordContent {
            rtype: 0x88,
            schema: 0x0700,
            pieces: vec![
                Piece::Run(vec![0x00, 0x01]),
                Piece::Child(ChildRef {
                    rtype: 0x0151,
                    schema: 0x0700,
                    framed_len: 4,
                }),
                Piece::Run(vec![0x00, 0x02]),
            ],
        };
        let mut cur = ContentCursor::with_strings(&c, StringFormat::Enhanced);
        assert_eq!(cur.u16_be(), Some(1));
        // The next field data is on the far side of a child: the read stops rather than
        // reading bytes that are not adjacent in the file.
        assert_eq!(cur.u16_be(), None);
        assert!(cur.stop().blocked_by_child);
        assert!(!cur.stop().ended);
        // Declaring the child lets the sequence continue.
        let mut cur = ContentCursor::with_strings(&c, StringFormat::Enhanced);
        assert_eq!(cur.u16_be(), Some(1));
        assert_eq!(cur.child().map(|c| c.rtype), Some(0x0151));
        assert_eq!(cur.u16_be(), Some(2));
        assert_eq!(cur.remaining(), 0);
    }

    /// The position a field's bytes are named in counts field data alone: it advances with every
    /// read and stands still across a nested record, so it and `remaining` always split the
    /// record's field bytes between them.
    #[test]
    fn the_position_counts_field_bytes_and_a_child_advances_it_by_nothing() {
        let c = RecordContent {
            rtype: 0x88,
            schema: 0x0700,
            pieces: vec![
                Piece::Run(vec![0x00, 0x01]),
                Piece::Child(ChildRef {
                    rtype: 0x0151,
                    schema: 0x0700,
                    framed_len: 4,
                }),
                Piece::Run(vec![0x00, 0x02]),
            ],
        };
        let mut cur = ContentCursor::with_strings(&c, StringFormat::Enhanced);
        assert_eq!((cur.pos(), cur.remaining()), (0, 4));
        assert_eq!(cur.u16_be(), Some(1));
        assert_eq!((cur.pos(), cur.remaining()), (2, 2));
        assert!(cur.child().is_some());
        assert_eq!((cur.pos(), cur.remaining()), (2, 2));
        assert_eq!(cur.u16_be(), Some(2));
        assert_eq!((cur.pos(), cur.remaining()), (4, 0));
    }

    /// The signed reads sign-extend from their own width, and their unsigned counterparts over the
    /// same bytes do not — which is the whole reason both exist.
    #[test]
    fn the_signed_reads_extend_the_sign_and_the_unsigned_ones_do_not() {
        let c = one_run(&[0xff, 0xfd, 0x30, 0x80, 0x00, 0x00, 0x00]);
        let mut cur = ContentCursor::with_strings(&c, StringFormat::Enhanced);
        assert_eq!(cur.i8(), Some(-1));
        assert_eq!(cur.i16_be(), Some(-720));
        assert_eq!(cur.i32_be(), Some(i32::MIN));
        assert_eq!(cur.remaining(), 0);

        let mut cur = ContentCursor::with_strings(&c, StringFormat::Enhanced);
        assert_eq!(cur.u8(), Some(0xff));
        assert_eq!(cur.u16_be(), Some(0xfd30));
        assert_eq!(cur.u32_be(), Some(0x8000_0000));

        // And each writes back the bytes it read.
        let mut enc = Encoder::with_strings(StringFormat::Enhanced);
        enc.i8(-1);
        enc.i16_be(-720);
        enc.i32_be(i32::MIN);
        assert_eq!(enc.finish(), c.pieces);
    }

    /// A single-precision float in both orders, and the byte-for-byte write back.
    #[test]
    fn f32_reads_in_either_order() {
        let le = one_run(&[0xcd, 0xcc, 0xcc, 0x3d]);
        assert_eq!(
            ContentCursor::with_strings(&le, StringFormat::Enhanced).f32_le(),
            Some(0.1)
        );
        let be = one_run(&[0x3d, 0xcc, 0xcc, 0xcd]);
        assert_eq!(
            ContentCursor::with_strings(&be, StringFormat::Enhanced).f32_be(),
            Some(0.1)
        );

        let mut enc = Encoder::with_strings(StringFormat::Enhanced);
        enc.f32_le(0.1);
        assert_eq!(enc.finish(), le.pieces);
        let mut enc = Encoder::with_strings(StringFormat::Enhanced);
        enc.f32_be(0.1);
        assert_eq!(enc.finish(), be.pieces);
    }

    /// The narrowing forms cannot express a negative: the marker bit is the width, so the widest
    /// value still leaves its own sign bit clear.
    #[test]
    fn a_narrowing_value_is_never_negative() {
        let c = one_run(&[0xff, 0xff, 0xff, 0xff]);
        let mut cur = ContentCursor::with_strings(&c, StringFormat::Enhanced);
        assert_eq!(cur.narrowing(2), Some(0x7fff_ffff));
        assert_eq!(cur.remaining(), 0);
    }

    #[test]
    fn the_enhanced_form_reads_the_declared_block() {
        let c = one_run(b"\x00\x00\x00\x03hi\0\xaa");
        let mut cur = ContentCursor::with_strings(&c, StringFormat::Enhanced);
        assert_eq!(cur.string(), Some(&b"hi\0"[..]));
        assert_eq!(cur.u8(), Some(0xaa));
    }

    /// The two forms are different bytes for the same string, and each is read only by the cursor
    /// opened for it: swap the forms over and the counted read takes a length out of the text while
    /// the scanned read stops inside the count.
    #[test]
    fn each_string_form_is_read_only_by_the_cursor_opened_for_it() {
        let enhanced = one_run(b"\x00\x00\x00\x03hi\0\xaa");
        let simple = one_run(b"hi\0\xaa");

        let mut cur = ContentCursor::with_strings(&simple, StringFormat::Simple);
        assert_eq!(cur.string(), Some(&b"hi\0"[..]));
        assert_eq!(cur.u8(), Some(0xaa));
        assert_eq!(cur.remaining(), 0);

        // The counted read over NUL-terminated bytes: `hi\0\xaa` is a length of 0x686900aa.
        let mut cur = ContentCursor::with_strings(&simple, StringFormat::Enhanced);
        assert_eq!(cur.string(), None);
        assert!(cur.stop().ended);

        // The scanned read over counted bytes stops at the count's own leading NUL, so the block
        // it returns is a prefix of the length field rather than any of the text.
        let mut cur = ContentCursor::with_strings(&enhanced, StringFormat::Simple);
        assert_eq!(cur.string(), Some(&b"\0"[..]));
        assert_eq!(cur.remaining(), 7);
    }

    /// The empty string, which is where the two forms differ most: five bytes counted, one byte
    /// terminated. Each round-trips through the encoder opened for the same form.
    #[test]
    fn an_empty_string_round_trips_in_both_forms() {
        for (form, bytes) in [
            (StringFormat::Enhanced, &b"\x00\x00\x00\x01\x00"[..]),
            (StringFormat::Simple, &b"\x00"[..]),
        ] {
            let c = one_run(bytes);
            let mut cur = ContentCursor::with_strings(&c, form);
            assert_eq!(cur.string(), Some(&b"\0"[..]), "{form:?}");
            assert_eq!(cur.remaining(), 0, "{form:?}");

            let mut enc = Encoder::with_strings(form);
            enc.string(b"\0");
            assert_eq!(enc.finish(), vec![Piece::Run(bytes.to_vec())], "{form:?}");
        }
    }

    /// A simple-form string that never terminates is a record that ended mid-string, not a read of
    /// whatever follows.
    #[test]
    fn an_unterminated_simple_string_reports_exhaustion() {
        let c = one_run(b"hi");
        let mut cur = ContentCursor::with_strings(&c, StringFormat::Simple);
        assert_eq!(cur.string(), None);
        assert!(cur.stop().ended);
    }

    /// A string is never read across a nested record, in either form: the bytes on the far side of
    /// a child are not adjacent to the ones before it.
    #[test]
    fn a_string_never_scans_past_a_child() {
        let c = RecordContent {
            rtype: 0x88,
            schema: 0x0700,
            pieces: vec![
                Piece::Run(b"hi".to_vec()),
                Piece::Child(ChildRef {
                    rtype: 0x0151,
                    schema: 0x0700,
                    framed_len: 4,
                }),
                Piece::Run(b"\0".to_vec()),
            ],
        };
        let mut cur = ContentCursor::with_strings(&c, StringFormat::Simple);
        assert_eq!(cur.string(), None);
        assert!(cur.stop().blocked_by_child);
        assert!(!cur.stop().ended);
    }
}
