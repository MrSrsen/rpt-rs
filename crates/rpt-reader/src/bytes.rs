//! The crate's search vocabulary: checked scalar reads, and the length-prefixed-string readers that
//! find a string where nothing declares one.
//!
//! A record's content is a sequence, not a layout: fields float behind variable-width markers and
//! authored strings, so an offset is a consequence of what precedes it rather than a constant. A
//! record type whose sequence has been stated is read from its field table and needs none of
//! what is here. What is left for here is the container's little-endian header words, and
//! [`lp_strings`] and its relatives, which find plausible strings anywhere in a run for the case
//! where the sequence around them is not stated.
//!
//! That case is what makes these readers strict. A search has no declaration to lean on, so
//! plausibility — a length that fits, a NUL-terminated span, valid UTF-8, no stray control bytes —
//! is its only evidence a string is there at all, and loosening it makes the scan match noise. The
//! price is that a strict reader also refuses text that is genuinely stored: an empty string, or
//! one written in a legacy code page. So a stated sequence must not read through here. It reads
//! its strings with [`crate::field_table::table::text_of`], which takes the span its table names
//! and accepts whatever is in it.
//!
//! These reads address a flat slice by offset and take no view of a record's shape, so they will
//! not grow: a read that has to respect the content's structure — never stepping over a nested
//! record, stopping where the record ends — is
//! [`ContentCursor`](crate::field_table::cursor::ContentCursor)'s, and the two spell every scalar
//! the same way so a call names which one is in hand.
//!
//! No decoder finds a string by scanning any more — every one reads it at the position its record's
//! field table states — so a scan is only ever a *second* reading. Two callers want one: the
//! field-table harness, where a table that reads the right bytes in the wrong order accounts for
//! its record perfectly and only a reading arrived at another way contradicts it; and the
//! byte-layout workbench, which is pointed at bytes no table describes. The second is why
//! [`lp_strings`] is public: a caller with a slice and no declaration must be able to ask this
//! reader what it reads as text, rather than keeping its own copy of the rule to drift from.

/// One length-prefixed string found in a byte slice by [`lp_strings`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LpString {
    /// Where the framing begins — the offset of the 4-byte length, not of the text.
    pub offset: usize,
    /// The decoded text, with the trailing NUL and anything after it dropped.
    pub text: String,
    /// Bytes the whole framing occupies: the 4-byte length plus the bytes it counts. Not the
    /// length of [`text`](Self::text), which stops at the first NUL.
    pub len: usize,
}

impl LpString {
    /// The offset just past the framing — where a field following the string begins.
    pub fn end(&self) -> usize {
        self.offset + self.len
    }
}

/// How [`lp_strings`] advances after a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LpScan {
    /// Step past the matched string — for structural reads of back-to-back strings.
    Consume,
    /// Step one byte even on a match, for searches that must tolerate *shadowed framing*: a
    /// spurious short match (e.g. a stray `00 00 00 01 60` decoding to `` ` ``) can begin a byte or
    /// two before the real string's length prefix, and consuming it would jump the scan past the
    /// real one.
    Slide,
}

/// Every plausible length-prefixed string in `bytes`, at any offset — a 4-byte big-endian length
/// then that many bytes, of which the text is everything up to the first NUL.
///
/// This is a **scan**, not a read: nothing in the bytes marks where a string begins, so every offset
/// is probed and plausibility is the only evidence one is there. It is therefore the reading for
/// bytes no field table describes. A record type that has a table reads its strings at the position
/// the table states, through [`RecordStream::fields`](crate::raw::RecordStream::fields), which takes
/// the declared span and accepts whatever is in it — including text this scan refuses.
pub fn lp_strings(bytes: &[u8], scan: LpScan) -> Vec<LpString> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if let Some((text, len)) = read_lp_string(&bytes[i..]) {
            out.push(LpString {
                offset: i,
                text,
                len,
            });
            i += match scan {
                LpScan::Consume => len,
                LpScan::Slide => 1,
            };
        } else {
            i += 1;
        }
    }
    out
}

/// The first length-prefixed string in `bytes`, at any offset.
#[cfg(test)]
pub(crate) fn first_lp(bytes: &[u8]) -> Option<String> {
    lp_strings(bytes, LpScan::Slide)
        .into_iter()
        .next()
        .map(|s| s.text)
}

// ---- checked scalar reads (None past the end) ----

pub(crate) fn u32_be(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

/// The big-endian half-word. Every record that stores one is read from its field table, so this
/// stands with the scanner: it addresses a byte by offset, which only a second reading does.
#[cfg(test)]
pub(crate) fn u16_be(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

pub(crate) fn u16_le(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

pub(crate) fn u32_le(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

/// Validate a string field's declared byte span as real text: NUL-truncated, non-empty, valid
/// UTF-8, no control characters except tab/CR/LF (formula bodies span multiple lines). This
/// admits non-ASCII (localized) text while rejecting binary mis-reads (invalid UTF-8 or control
/// bytes from a wrong run / coincidental length). Returns the text and its NUL-truncated length.
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

/// The single `u32`-big-endian length-prefixed string reader the offset-based LP primitives share.
///
/// Reads a 4-byte BE length at `off`, requires it no greater than `cap`, then validates that many
/// bytes as text. When `exact`, the whole declared span must be exactly the string plus one
/// trailing NUL (the NUL-truncated length + 1 equals `len`) — rejecting a span with trailing bytes
/// after the NUL. Returns the string and the bytes consumed from `off` (`4 + len`), or `None` if
/// the framing is implausible or runs past the end.
///
/// A caller states only the cap and the exactness; there is no lower bound to state, because a span
/// too short to hold text yields no text and [`valid_text`] rejects it.
fn read_lp_u32(bytes: &[u8], off: usize, cap: usize, exact: bool) -> Option<(String, usize)> {
    let len = u32_be(bytes, off)? as usize;
    if len > cap {
        return None;
    }
    let raw = bytes.get(off + 4..off + 4 + len)?;
    let (s, end) = valid_text(raw)?;
    if exact && end + 1 != len {
        return None;
    }
    Some((s.to_owned(), 4 + len))
}

/// If a length-prefixed printable string starts at `off`, return it and the bytes consumed
/// (4-byte big-endian length + that many bytes). Stricter than [`read_lp_string`]: the whole
/// declared field must be one NUL-terminated string (used when splitting a record's field bytes into typed values).
pub(crate) fn lp_string_at(bytes: &[u8], off: usize) -> Option<(String, usize)> {
    read_lp_u32(bytes, off, 4096, true)
}

/// Decode a length-prefixed string: 4-byte big-endian length, then that many bytes, of which the
/// text is everything up to the first NUL. Returns the string and the offset just past the whole
/// declared span, or `None` if the framing is implausible.
///
/// The cap must clear large formula bodies — a big multi-branch `switch` can run to several KB —
/// so it is well above 4 KB; the slice bound in [`read_lp_u32`] still rejects any length past the
/// record end.
pub(crate) fn read_lp_string(bytes: &[u8]) -> Option<(String, usize)> {
    read_lp_u32(bytes, 0, 0x40000, false)
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

    fn texts(bytes: &[u8], scan: LpScan) -> Vec<String> {
        lp_strings(bytes, scan)
            .into_iter()
            .map(|s| s.text)
            .collect()
    }

    #[test]
    fn consume_reads_back_to_back_strings() {
        let mut bytes = lp("alpha");
        bytes.extend(lp("beta"));
        assert_eq!(texts(&bytes, LpScan::Consume), ["alpha", "beta"]);
        // Each match reports where its framing sits and what it spans, so a field after a string
        // is addressable from the string's end rather than from a constant.
        let found = lp_strings(&bytes, LpScan::Consume);
        assert_eq!((found[0].offset, found[0].len, found[0].end()), (0, 10, 10));
        assert_eq!(found[1].offset, 10);
    }

    #[test]
    fn slide_finds_a_string_shadowed_by_an_earlier_false_match() {
        // A false match at offset 0 (len 9, NUL-truncating to "a") envelopes the real string
        // at offset 5. Consume jumps past it; Slide still finds it.
        let mut bytes = vec![0, 0, 0, 9, b'a'];
        bytes.extend(lp("b"));
        bytes.extend([0, 0]);
        assert_eq!(texts(&bytes, LpScan::Slide), ["a", "b"]);
        assert_eq!(texts(&bytes, LpScan::Consume), ["a"]);
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
        // A zero-length span declares no text, and is refused by validation alone — the reader
        // states no lower bound on the length.
        assert_eq!(read_lp_string(&lp_u32(0, b"")), None);
        // One character and no terminator is still text; only `exact` demands the NUL.
        assert_eq!(read_lp_string(&lp_u32(1, b"a")), Some(("a".to_owned(), 5)));
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
        // A one-byte span cannot be one NUL-terminated string either way: a lone NUL leaves no
        // text, and a lone character leaves no terminator.
        assert_eq!(lp_string_at(&lp_u32(1, b"\0"), 0), None);
        assert_eq!(lp_string_at(&lp_u32(1, b"a"), 0), None);
        // Honors the offset.
        let mut framed = vec![0xAA, 0xBB];
        framed.extend(lp_u32(3, b"ab\0"));
        assert_eq!(lp_string_at(&framed, 2), Some(("ab".to_owned(), 7)));
    }
}
