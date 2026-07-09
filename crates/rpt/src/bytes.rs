//! The crate's binary-decoding vocabulary: checked scalar reads, the sequential [`Cursor`], and
//! the length-prefixed-string scanner.
//!
//! `.rpt` leaves rarely give fixed offsets — fields float behind variable-width markers and
//! localized strings — so most decoders are built from two primitives: [`lp_scan`] (find plausible
//! strings anywhere in a leaf) and [`Cursor`] (read a known sequential layout, degrading per-field
//! on short leaves). Both the `project::raise` projection and the `io` orchestration facade decode
//! through these, so they live crate-wide rather than inside one layer.

/// How [`lp_scan`] advances after a match.
///
/// - `Consume`: step past the matched string — for structural reads of back-to-back strings.
/// - `Slide`: step one byte even on a match — for searches that must tolerate *shadowed
///   framing*: a spurious short match (e.g. a stray `00 00 00 01 60` decoding to `` ` ``) can
///   begin a byte or two before the real string's length prefix, and consuming it would jump
///   the scan past the real one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scan {
    Consume,
    Slide,
}

/// Iterator over every plausible length-prefixed string in `bytes` (per [`read_lp_string`]),
/// yielding `(offset, text, bytes_consumed)`.
pub(crate) struct LpScan<'a> {
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
pub(crate) fn lp_scan(bytes: &[u8], scan: Scan) -> LpScan<'_> {
    LpScan { bytes, i: 0, scan }
}

/// The first length-prefixed string in `bytes`, at any offset.
pub(crate) fn first_lp(bytes: &[u8]) -> Option<String> {
    lp_scan(bytes, Scan::Slide).next().map(|(_, s, _)| s)
}

/// The longest length-prefixed string in `bytes`, scanning every offset (`Slide`, so a real
/// string shadowed by an overlapping false match is still found). First-wins on equal length.
pub(crate) fn longest_lp(bytes: &[u8]) -> Option<String> {
    let mut best: Option<String> = None;
    for (_, s, _) in lp_scan(bytes, Scan::Slide) {
        if best.as_ref().is_none_or(|b| s.len() > b.len()) {
            best = Some(s);
        }
    }
    best
}

// ---- checked scalar reads (None past the end) ----

pub(crate) fn i32_be(b: &[u8], off: usize) -> Option<i32> {
    Some(i32::from_be_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

pub(crate) fn u32_be(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

pub(crate) fn u16_be(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

pub(crate) fn u16_le(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

pub(crate) fn u32_le(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

/// A checked sequential reader over leaf bytes. Every read is `Option` (never panics), so a
/// decoder over a short or unexpected leaf degrades field-by-field instead of dropping the
/// whole record — the tolerance the parity corpus depends on.
pub(crate) struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(b: &'a [u8]) -> Cursor<'a> {
        Cursor { b, pos: 0 }
    }

    pub(crate) fn at(b: &'a [u8], pos: usize) -> Cursor<'a> {
        Cursor { b, pos }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn skip(&mut self, n: usize) -> &mut Self {
        self.pos += n;
        self
    }

    pub(crate) fn u8(&mut self) -> Option<u8> {
        let v = self.b.get(self.pos).copied()?;
        self.pos += 1;
        Some(v)
    }

    pub(crate) fn u16_be(&mut self) -> Option<u16> {
        let v = u16_be(self.b, self.pos)?;
        self.pos += 2;
        Some(v)
    }

    pub(crate) fn u32_be(&mut self) -> Option<u32> {
        let v = u32_be(self.b, self.pos)?;
        self.pos += 4;
        Some(v)
    }

    pub(crate) fn f64_be(&mut self) -> Option<f64> {
        let v = f64::from_be_bytes(self.b.get(self.pos..self.pos + 8)?.try_into().ok()?);
        self.pos += 8;
        Some(v)
    }

    pub(crate) fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let v = self.b.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(v)
    }

    /// Read a length-prefixed string here (per [`read_lp_string`]'s validation).
    pub(crate) fn lp_string(&mut self) -> Option<String> {
        let (s, consumed) = read_lp_string(self.b.get(self.pos..)?)?;
        self.pos += consumed;
        Some(s)
    }
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

/// How a `u32`-length-prefixed span is decoded into text by [`read_lp_u32`].
///
/// - `Strict`: validate the span as clean text via [`valid_text`] (NUL-truncated, non-empty,
///   valid UTF-8, no control chars except tab/CR/LF) — rejects binary mis-reads.
/// - `Lossy`: NUL-truncate and decode with `from_utf8_lossy`, tolerating non-text bytes — for
///   spans framed by a preceding fixed binary header rather than validated as clean text.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LpText {
    Strict,
    Lossy,
}

/// The single `u32`-big-endian length-prefixed string reader the offset-based LP primitives share.
///
/// Reads a 4-byte BE length at `off`, requires it within `bounds`, then decodes that many bytes per
/// `text`. When `exact`, the whole declared span must be exactly the string plus one trailing NUL
/// (the NUL-truncated length + 1 equals `len`) — rejecting a span with trailing bytes after the NUL.
/// Returns the string and the bytes consumed from `off` (`4 + len`), or `None` if the framing is
/// implausible or runs past the end.
fn read_lp_u32(
    bytes: &[u8],
    off: usize,
    bounds: std::ops::RangeInclusive<usize>,
    text: LpText,
    exact: bool,
) -> Option<(String, usize)> {
    let len = u32_be(bytes, off)? as usize;
    if !bounds.contains(&len) {
        return None;
    }
    let raw = bytes.get(off + 4..off + 4 + len)?;
    let (s, end) = match text {
        LpText::Strict => {
            let (s, end) = valid_text(raw)?;
            (s.to_owned(), end)
        }
        LpText::Lossy => {
            let end = raw.iter().position(|&x| x == 0).unwrap_or(raw.len());
            (String::from_utf8_lossy(&raw[..end]).into_owned(), end)
        }
    };
    if exact && end + 1 != len {
        return None;
    }
    Some((s, 4 + len))
}

/// If a length-prefixed printable string starts at `off`, return it and the bytes consumed
/// (4-byte big-endian length + that many bytes). Stricter than [`read_lp_string`]: the whole
/// declared field must be one NUL-terminated string (used by the lossless DOM projection).
pub(crate) fn lp_string_at(bytes: &[u8], off: usize) -> Option<(String, usize)> {
    read_lp_u32(bytes, off, 2..=4096, LpText::Strict, true)
}

/// Decode a length-prefixed string: 4-byte big-endian length, then that many bytes (a
/// trailing NUL terminator is dropped). Returns the string and the offset just past it, or
/// `None` if the framing is implausible.
///
/// The cap must clear large formula bodies — a big multi-branch `switch` can run to several KB —
/// so it is well above 4 KB; the slice bound in [`read_lp_u32`] still rejects any length past the
/// record end.
pub(crate) fn read_lp_string(bytes: &[u8]) -> Option<(String, usize)> {
    read_lp_u32(bytes, 0, 1..=0x40000, LpText::Strict, false)
}

/// Decode a big-endian length-prefixed string at `off`: a `u32`-BE byte count (including any
/// trailing NUL), then that many bytes taken up to the first NUL, decoded **lossily**
/// (`from_utf8_lossy`). Returns the text and the bytes consumed from `off` (`4 + len`), or `None`
/// if the length is absurd (> 4096) or runs past the end.
///
/// The lossy, cap-4096 counterpart to the strict [`read_lp_string`]: it tolerates non-text bytes
/// rather than rejecting them, for leaves whose string span is framed by a preceding fixed binary
/// header (chart/cross-tab records) rather than validated as clean text.
pub(crate) fn read_be_lp_string_lossy(b: &[u8], off: usize) -> Option<(String, usize)> {
    read_lp_u32(b, off, 0..=4096, LpText::Lossy, false)
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

    /// A `u32`-BE length-prefixed blob: `len` as a 4-byte BE prefix, then `body` verbatim.
    fn lp_u32(len: u32, body: &[u8]) -> Vec<u8> {
        let mut v = len.to_be_bytes().to_vec();
        v.extend(body);
        v
    }

    #[test]
    fn read_lp_string_strips_trailing_nul_and_reports_consumed() {
        // 4-byte BE len (incl. NUL) + "hi\0"; returns the text and 4+len consumed.
        let bytes = lp("hi");
        assert_eq!(read_lp_string(&bytes), Some(("hi".to_owned(), 7)));
    }

    #[test]
    fn read_lp_string_allows_trailing_bytes_after_the_string() {
        // Not `exact`: bytes after the NUL within the declared span are ignored, still consuming len.
        let bytes = lp_u32(5, b"ab\0XY");
        assert_eq!(read_lp_string(&bytes), Some(("ab".to_owned(), 9)));
    }

    #[test]
    fn read_lp_string_rejects_zero_length_and_control_bytes() {
        assert_eq!(read_lp_string(&lp_u32(0, b"")), None);
        // A control byte (0x01) that is not tab/CR/LF fails strict validation.
        assert_eq!(read_lp_string(&lp_u32(2, b"\x01\0")), None);
    }

    #[test]
    fn read_lp_string_accepts_tab_cr_lf_in_body() {
        let bytes = lp_u32(5, b"a\tb\n\0");
        assert_eq!(read_lp_string(&bytes), Some(("a\tb\n".to_owned(), 9)));
    }

    #[test]
    fn lp_string_at_requires_the_whole_field_to_be_one_nul_terminated_string() {
        // Exact: text length + 1 must equal len — a clean "ab\0" passes.
        assert_eq!(
            lp_string_at(&lp_u32(3, b"ab\0"), 0),
            Some(("ab".to_owned(), 7))
        );
        // Trailing bytes after the NUL make it not-exact — rejected (unlike read_lp_string).
        assert_eq!(lp_string_at(&lp_u32(5, b"ab\0XY"), 0), None);
        // A declared span with no NUL terminator is also rejected by exact.
        assert_eq!(lp_string_at(&lp_u32(2, b"ab"), 0), None);
        // Honors the offset.
        let mut framed = vec![0xAA, 0xBB];
        framed.extend(lp_u32(3, b"ab\0"));
        assert_eq!(lp_string_at(&framed, 2), Some(("ab".to_owned(), 7)));
    }

    #[test]
    fn read_be_lp_string_lossy_tolerates_non_text_and_zero_length() {
        // Invalid UTF-8 (0xFF) is kept (replacement char), where strict would reject.
        let (s, used) = read_be_lp_string_lossy(&lp_u32(3, b"a\xff\0"), 0).unwrap();
        assert_eq!((s.as_str(), used), ("a\u{fffd}", 7));
        // Length 0 yields an empty string consuming just the 4-byte prefix.
        assert_eq!(
            read_be_lp_string_lossy(&lp_u32(0, b""), 0),
            Some((String::new(), 4))
        );
    }

    #[test]
    fn read_be_lp_string_lossy_rejects_oversized_length() {
        assert_eq!(read_be_lp_string_lossy(&lp_u32(4097, b""), 0), None);
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
