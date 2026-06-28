//! Shared low-level substrate helpers: length-prefixed strings, coordinate/colour decoding.

use super::*;

pub(super) fn own_lp_strings(node: &RecordNode, logical: &[u8]) -> Vec<String> {
    let bytes = node.leaf_bytes(logical);
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if let Some((s, consumed)) = read_lp_string(&bytes[i..]) {
            out.push(s);
            i += consumed;
        } else {
            i += 1;
        }
    }
    out
}

/// Find the first length-prefixed string in a record's content: each node's **own** leaf bytes
/// (a container record like a field object holds its data-source string in its own bytes,
/// alongside child records), scanning every offset, in pre-order.
pub(super) fn first_string(node: &RecordNode, logical: &[u8]) -> Option<String> {
    let mut found = None;
    node.walk(&mut |n| {
        if found.is_none() {
            let bytes = n.leaf_bytes(logical);
            let mut i = 0;
            while i + 4 <= bytes.len() {
                if let Some((s, _)) = read_lp_string(&bytes[i..]) {
                    found = Some(s);
                    break;
                }
                i += 1;
            }
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
    let v = b
        .get(0..4)
        .map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]))
        .unwrap_or(0);
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
    let w = i32::from(u16::from_be_bytes(b.get(off..off + 2)?.try_into().ok()?));
    if w & 0x8000 != 0 {
        let low = i32::from(u16::from_be_bytes(
            b.get(off + 2..off + 4)?.try_into().ok()?,
        ));
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
        let bytes = n.leaf_bytes(logical);
        let mut i = 0;
        while i + 4 <= bytes.len() {
            if let Some((s, consumed)) = read_lp_string(&bytes[i..]) {
                out.push(s);
                i += consumed;
            } else {
                i += 1;
            }
        }
    });
    out
}

/// If a length-prefixed printable string starts at `off`, return it and the bytes consumed
/// (4-byte big-endian length + that many bytes).
pub(super) fn lp_string_at(bytes: &[u8], off: usize) -> Option<(String, usize)> {
    let len = u32::from_be_bytes(bytes.get(off..off + 4)?.try_into().ok()?) as usize;
    if !(2..=4096).contains(&len) {
        return None;
    }
    let raw = bytes.get(off + 4..off + 4 + len)?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let text = &raw[..end];
    // Require the whole declared field to be a NUL-terminated valid-UTF-8 string with no control
    // chars except tab/CR/LF (avoids matching arbitrary binary that happens to start with a small
    // big-endian length). Non-ASCII is allowed — reports are localized.
    if text.len() + 1 != len || text.is_empty() {
        return None;
    }
    let s = std::str::from_utf8(text).ok()?;
    if s.chars()
        .any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n'))
    {
        return None;
    }
    Some((s.to_owned(), 4 + len))
}

/// Decode a length-prefixed string: 4-byte big-endian length, then that many bytes (a
/// trailing NUL terminator is dropped). Returns the string and the offset just past it, or
/// `None` if the framing is implausible.
pub(super) fn read_lp_string(bytes: &[u8]) -> Option<(String, usize)> {
    let len = u32::from_be_bytes(bytes.get(0..4)?.try_into().ok()?) as usize;
    // Reject 0 and absurd lengths (mis-parse). The cap must clear large formula bodies — a big
    // multi-branch `switch` can run to several KB — so it is well above 4 KB; the slice bound below
    // still rejects any length past the record end.
    if len == 0 || len > 0x40000 {
        return None;
    }
    let raw = bytes.get(4..4 + len)?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let text = &raw[..end];
    // A real string is non-empty valid UTF-8 with no control characters except tab/CR/LF (formula
    // bodies span multiple lines). This admits non-ASCII (localized) text while still rejecting
    // binary mis-reads (invalid UTF-8 or control bytes from a wrong leaf / coincidental length).
    if text.is_empty() {
        return None;
    }
    let s = std::str::from_utf8(text).ok()?;
    if s.chars()
        .any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n'))
    {
        return None;
    }
    Some((s.to_owned(), 4 + len))
}
