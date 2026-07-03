//! Shared low-level substrate helpers: the length-prefixed-string scanner, checked scalar
//! reads, the sequential [`Cursor`], tree queries, and coordinate/colour decoding.
//!
//! These are the raise layer's decoding vocabulary. The format rarely gives fixed offsets —
//! fields float behind variable-width markers and localized strings — so most decoders are
//! built from two primitives: [`lp_scan`] (find plausible strings anywhere in a leaf) and
//! [`Cursor`] (read a known sequential layout, degrading per-field on short leaves).

use super::*;

/// How [`lp_scan`] advances after a match.
///
/// - `Consume`: step past the matched string — for structural reads of back-to-back strings.
/// - `Slide`: step one byte even on a match — for searches that must tolerate *shadowed
///   framing*: a spurious short match (e.g. a stray `00 00 00 01 60` decoding to `` ` ``) can
///   begin a byte or two before the real string's length prefix, and consuming it would jump
///   the scan past the real one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Scan {
    Consume,
    Slide,
}

/// Iterator over every plausible length-prefixed string in `bytes` (per [`read_lp_string`]),
/// yielding `(offset, text, bytes_consumed)`.
pub(super) struct LpScan<'a> {
    bytes: &'a [u8],
    i: usize,
    scan: Scan,
}

impl Iterator for LpScan<'_> {
    type Item = (usize, String, usize);

    fn next(&mut self) -> Option<Self::Item> {
        while self.i + 4 <= self.bytes.len() {
            let at = self.i;
            if let Some((s, consumed)) = read_lp_string(&self.bytes[at..]) {
                self.i += match self.scan {
                    Scan::Consume => consumed,
                    Scan::Slide => 1,
                };
                return Some((at, s, consumed));
            }
            self.i += 1;
        }
        None
    }
}

/// Scan `bytes` for length-prefixed strings at any offset. See [`Scan`] for the discipline.
pub(super) fn lp_scan(bytes: &[u8], scan: Scan) -> LpScan<'_> {
    LpScan { bytes, i: 0, scan }
}

/// The first length-prefixed string in `bytes`, at any offset.
pub(super) fn first_lp(bytes: &[u8]) -> Option<String> {
    lp_scan(bytes, Scan::Slide).next().map(|(_, s, _)| s)
}

/// The longest length-prefixed string in `bytes`, scanning every offset (`Slide`, so a real
/// string shadowed by an overlapping false match is still found). First-wins on equal length.
pub(super) fn longest_lp(bytes: &[u8]) -> Option<String> {
    let mut best: Option<String> = None;
    for (_, s, _) in lp_scan(bytes, Scan::Slide) {
        if best.as_ref().is_none_or(|b| s.len() > b.len()) {
            best = Some(s);
        }
    }
    best
}

// ---- checked scalar reads (None past the end) ----

pub(super) fn i32_be(b: &[u8], off: usize) -> Option<i32> {
    Some(i32::from_be_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

pub(super) fn u32_be(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

pub(super) fn u16_be(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

pub(super) fn u16_le(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

/// A checked sequential reader over leaf bytes. Every read is `Option` (never panics), so a
/// decoder over a short or unexpected leaf degrades field-by-field instead of dropping the
/// whole record — the tolerance the parity corpus depends on.
pub(super) struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub(super) fn new(b: &'a [u8]) -> Cursor<'a> {
        Cursor { b, pos: 0 }
    }

    pub(super) fn at(b: &'a [u8], pos: usize) -> Cursor<'a> {
        Cursor { b, pos }
    }

    pub(super) fn pos(&self) -> usize {
        self.pos
    }

    pub(super) fn skip(&mut self, n: usize) -> &mut Self {
        self.pos += n;
        self
    }

    pub(super) fn u8(&mut self) -> Option<u8> {
        let v = self.b.get(self.pos).copied()?;
        self.pos += 1;
        Some(v)
    }

    pub(super) fn u16_be(&mut self) -> Option<u16> {
        let v = u16_be(self.b, self.pos)?;
        self.pos += 2;
        Some(v)
    }

    pub(super) fn u32_be(&mut self) -> Option<u32> {
        let v = u32_be(self.b, self.pos)?;
        self.pos += 4;
        Some(v)
    }

    pub(super) fn f64_be(&mut self) -> Option<f64> {
        let v = f64::from_be_bytes(self.b.get(self.pos..self.pos + 8)?.try_into().ok()?);
        self.pos += 8;
        Some(v)
    }

    pub(super) fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let v = self.b.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(v)
    }

    /// Read a length-prefixed string here (per [`read_lp_string`]'s validation).
    pub(super) fn lp_string(&mut self) -> Option<String> {
        let (s, consumed) = read_lp_string(self.b.get(self.pos..)?)?;
        self.pos += consumed;
        Some(s)
    }
}

/// All nodes of the tree (pre-order) satisfying `pred`.
pub(super) fn nodes_where(
    tree: &[RecordNode],
    pred: impl Fn(&RecordNode) -> bool,
) -> Vec<&RecordNode> {
    let mut out = Vec::new();
    for root in tree {
        root.walk(&mut |n| {
            if pred(n) {
                out.push(n);
            }
        });
    }
    out
}

/// The leaf bytes of every node of type `rtype`, anywhere in the tree, in pre-order.
pub(super) fn leaves_of(tree: &[RecordNode], logical: &[u8], rtype: u16) -> Vec<Vec<u8>> {
    nodes_where(tree, |n| n.rtype == rtype)
        .into_iter()
        .map(|n| n.leaf_bytes(logical))
        .collect()
}

pub(super) fn own_lp_strings(node: &RecordNode, logical: &[u8]) -> Vec<String> {
    lp_scan(&node.leaf_bytes(logical), Scan::Consume)
        .map(|(_, s, _)| s)
        .collect()
}

/// Find the first length-prefixed string in a record's content: each node's **own** leaf bytes
/// (a container record like a field object holds its data-source string in its own bytes,
/// alongside child records), scanning every offset, in pre-order.
pub(super) fn first_string(node: &RecordNode, logical: &[u8]) -> Option<String> {
    let mut found = None;
    node.walk(&mut |n| {
        if found.is_none() {
            found = first_lp(&n.leaf_bytes(logical));
        }
    });
    found
}

/// The trailing run of ASCII decimal digits of `s` (`"GroupHeaderArea4"` → `"4"`,
/// `"PageHeader"` → `""`) — used to link a group header to its matching footer.
pub(super) fn trailing_digits(s: &str) -> String {
    let digits: String = s.chars().rev().take_while(char::is_ascii_digit).collect();
    digits.chars().rev().collect()
}

/// Decode a `COLORREF` (`0x00BBGGRR`, big-endian) into a [`Color`]; `0xffffffff` is the
/// "default / no colour" sentinel, treated as White.
pub(super) fn raise_colorref(b: &[u8]) -> Color {
    let v = u32_be(b, 0).unwrap_or(0);
    if v == 0xffff_ffff {
        return Color::WHITE;
    }
    Color {
        a: 255,
        r: (v & 0xff) as u8,
        g: ((v >> 8) & 0xff) as u8,
        b: ((v >> 16) & 0xff) as u8,
    }
}

/// Flatten the record tree into document (pre-order) order — the order the engine wrote the
/// records, in which an object and its name/position records are adjacent.
pub(super) fn flatten(tree: &[RecordNode]) -> Vec<&RecordNode> {
    let mut out = Vec::new();
    for root in tree {
        root.walk(&mut |n| out.push(n));
    }
    out
}

/// Read a variable-length object coordinate (big-endian twips) at `off`, returning its value and
/// the offset past it. A coordinate below 32768 is a plain `u16`; at or above 32768 the high bit of
/// the first word is set as an escape and the value is `(word & 0x7fff) << 16 | next u16` — wide
/// export reports place objects past 32768 (and past 65536) twips.
pub(super) fn read_coord(b: &[u8], off: usize) -> Option<(i32, usize)> {
    let w = i32::from(u16_be(b, off)?);
    if w & 0x8000 != 0 {
        let low = i32::from(u16_be(b, off + 2)?);
        Some((((w & 0x7fff) << 16) | low, off + 4))
    } else {
        Some((w, off + 2))
    }
}

/// All length-prefixed strings in a record's content (every node's own leaf bytes), scanning
/// every offset, in pre-order.
pub(super) fn all_strings(node: &RecordNode, logical: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    node.walk(&mut |n| {
        out.extend(lp_scan(&n.leaf_bytes(logical), Scan::Consume).map(|(_, s, _)| s));
    });
    out
}

/// Validate a string field's declared byte span as real text: NUL-truncated, non-empty, valid
/// UTF-8, no control characters except tab/CR/LF (formula bodies span multiple lines). This
/// admits non-ASCII (localized) text while rejecting binary mis-reads (invalid UTF-8 or control
/// bytes from a wrong leaf / coincidental length). Returns the text and its NUL-truncated length.
fn valid_text(raw: &[u8]) -> Option<(&str, usize)> {
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let text = &raw[..end];
    if text.is_empty() {
        return None;
    }
    let s = std::str::from_utf8(text).ok()?;
    if s.chars()
        .any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n'))
    {
        return None;
    }
    Some((s, end))
}

/// If a length-prefixed printable string starts at `off`, return it and the bytes consumed
/// (4-byte big-endian length + that many bytes). Stricter than [`read_lp_string`]: the whole
/// declared field must be one NUL-terminated string (used by the lossless DOM projection).
pub(super) fn lp_string_at(bytes: &[u8], off: usize) -> Option<(String, usize)> {
    let len = u32_be(bytes, off)? as usize;
    if !(2..=4096).contains(&len) {
        return None;
    }
    let raw = bytes.get(off + 4..off + 4 + len)?;
    let (s, end) = valid_text(raw)?;
    // Require the whole declared field to be the NUL-terminated string.
    if end + 1 != len {
        return None;
    }
    Some((s.to_owned(), 4 + len))
}

/// Decode a length-prefixed string: 4-byte big-endian length, then that many bytes (a
/// trailing NUL terminator is dropped). Returns the string and the offset just past it, or
/// `None` if the framing is implausible.
pub(super) fn read_lp_string(bytes: &[u8]) -> Option<(String, usize)> {
    let len = u32_be(bytes, 0)? as usize;
    // Reject 0 and absurd lengths (mis-parse). The cap must clear large formula bodies — a big
    // multi-branch `switch` can run to several KB — so it is well above 4 KB; the slice bound below
    // still rejects any length past the record end.
    if len == 0 || len > 0x40000 {
        return None;
    }
    let raw = bytes.get(4..4 + len)?;
    let (s, _) = valid_text(raw)?;
    Some((s.to_owned(), 4 + len))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a NUL-terminated length-prefixed string.
    fn lp(s: &str) -> Vec<u8> {
        let mut v = ((s.len() + 1) as u32).to_be_bytes().to_vec();
        v.extend(s.as_bytes());
        v.push(0);
        v
    }

    #[test]
    fn consume_reads_back_to_back_strings() {
        let mut bytes = lp("alpha");
        bytes.extend(lp("beta"));
        let out: Vec<String> = lp_scan(&bytes, Scan::Consume).map(|(_, s, _)| s).collect();
        assert_eq!(out, ["alpha", "beta"]);
    }

    #[test]
    fn slide_finds_a_string_shadowed_by_an_earlier_false_match() {
        // A false match at offset 0 (len 9, NUL-truncating to "a") envelopes the real string
        // at offset 5. Consume jumps past it; Slide still finds it.
        let mut bytes = vec![0, 0, 0, 9, b'a'];
        bytes.extend(lp("b"));
        bytes.extend([0, 0]);
        let slide: Vec<String> = lp_scan(&bytes, Scan::Slide).map(|(_, s, _)| s).collect();
        let consume: Vec<String> = lp_scan(&bytes, Scan::Consume).map(|(_, s, _)| s).collect();
        assert_eq!(slide, ["a", "b"]);
        assert_eq!(consume, ["a"]);
    }

    #[test]
    fn cursor_reads_are_checked_and_sequential() {
        let mut bytes = vec![0x12, 0x00, 0x07];
        bytes.extend(lp("x"));
        let mut c = Cursor::new(&bytes);
        assert_eq!(c.u8(), Some(0x12));
        assert_eq!(c.u16_be(), Some(7));
        assert_eq!(c.lp_string().as_deref(), Some("x"));
        assert_eq!(c.u8(), None); // past the end: None, not a panic
    }
}
