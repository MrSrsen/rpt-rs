//! Parsing and byte-view primitives shared across the `dump` submodules: option/selector parsing,
//! length-prefixed string decoding, the hex/scalar formatting helpers, and the `--cols` column
//! grammar (`Width`/`Col`).

/// Parse a number written as hex (`0x1a`) or decimal — used for `--offset` / `--len`.
pub(super) fn parse_num(s: &str) -> Option<usize> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        usize::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

/// Resolve a `--type` selector to a record-type word: `0xNN` / bare hex, or a registry name
/// (case-insensitive, e.g. `Formula` → `0x0076`). Returns `None` if it matches nothing.
pub(super) fn parse_type_selector(s: &str) -> Option<u16> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u16::from_str_radix(hex, 16).ok();
    }
    // A bare token of only hex digits is a hex type word; otherwise try the name registry.
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit()) {
        if let Ok(v) = u16::from_str_radix(s, 16) {
            return Some(v);
        }
    }
    // Match a registry name (`RecordTag::name`) case-insensitively by scanning the type space.
    (0u16..=0xffff).find(|&t| {
        rpt::raw::RecordTag(t)
            .name()
            .is_some_and(|n| n.eq_ignore_ascii_case(s))
    })
}

/// A record type's `Name(0x00nn)` label (or bare hex for unnamed types).
pub(super) fn type_label(rtype: u16) -> String {
    match rpt::raw::RecordTag(rtype).name() {
        Some(n) => format!("{n}(0x{rtype:04x})"),
        None => format!("0x{rtype:04x}"),
    }
}

/// Every length-prefixed string in `bytes` (4-byte big-endian length + that many bytes, trailing
/// NUL dropped), scanning each offset and stepping past a match — the same framing the reader's
/// `read_lp_string` accepts. Yields `(offset, text, bytes_consumed)`.
pub(super) fn lp_strings(bytes: &[u8]) -> Vec<(usize, String, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if let Some((s, consumed)) = read_lp_at(&bytes[i..]) {
            out.push((i, s, consumed));
            i += consumed;
        } else {
            i += 1;
        }
    }
    out
}

/// Decode a length-prefixed string at the start of `b`: 4-byte BE length, then that many bytes,
/// NUL-truncated. Rejects empty/absurd lengths and any run with control bytes (bar tab/CR/LF) or
/// invalid UTF-8 — the mis-read guard the reader uses.
fn read_lp_at(b: &[u8]) -> Option<(String, usize)> {
    let len = u32::from_be_bytes(b.get(0..4)?.try_into().ok()?) as usize;
    if len == 0 || len > 0x40000 {
        return None;
    }
    let raw = b.get(4..4 + len)?;
    let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
    let text = raw.get(..end)?;
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

/// A `hexdump -C`-style rendering of `bytes`: `OFFSET  hex×16  |ascii|`, offsets relative to the
/// slice start.
pub(super) fn hexdump_lines(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(i, chunk)| {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
            let ascii: String = chunk
                .iter()
                .map(|&b| {
                    if (0x20..0x7f).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            format!("  {:04x}  {:<47}  |{ascii}|", i * 16, hex.join(" "))
        })
        .collect()
}

/// Resolve the scalar-probe cap: default 64 bytes, `all` = the whole leaf, a number = that many,
/// 0 = off.
pub(super) fn probe_cap(opt: Option<&str>, leaf_len: usize) -> usize {
    match opt {
        None => 64.min(leaf_len),
        Some(s) if s.eq_ignore_ascii_case("all") => leaf_len,
        Some(s) => parse_num(s).unwrap_or(64).min(leaf_len),
    }
}

// ── Corpus sweep + anchor-relative columns ────────────────────────────────────────────────────
//
// The single-record dump is per-file; RE that separates a confound needs the *same* derived value
// pulled from a record type across a whole corpus into one table (e.g. `used+2`/`used+3`/`@4` over
// every chart report). `--glob` sweeps a directory into that table; `--cols` says which bytes, and
// `used`/`--anchor-string` anchor an offset at a decoded LP-string's end rather than an absolute
// leaf offset (a field's trailing tail moves when the field name's length changes).

/// Width + endianness of a scalar-probe column.
#[derive(Clone, Copy)]
pub(super) enum Width {
    U8,
    U16le,
    U16be,
    U32le,
    U32be,
}

impl Width {
    fn parse(s: &str) -> Option<Width> {
        match s.to_ascii_lowercase().as_str() {
            "u8" => Some(Width::U8),
            "u16le" => Some(Width::U16le),
            "u16be" => Some(Width::U16be),
            "u32le" => Some(Width::U32le),
            "u32be" => Some(Width::U32be),
            _ => None,
        }
    }
    /// Read the scalar at `off` (returns `None` past the end).
    pub(super) fn read(self, b: &[u8], off: usize) -> Option<u32> {
        match self {
            Width::U8 => b.get(off).map(|&x| x as u32),
            Width::U16le => b
                .get(off..off + 2)
                .map(|s| u16::from_le_bytes([s[0], s[1]]) as u32),
            Width::U16be => b
                .get(off..off + 2)
                .map(|s| u16::from_be_bytes([s[0], s[1]]) as u32),
            Width::U32le => b
                .get(off..off + 4)
                .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])),
            Width::U32be => b
                .get(off..off + 4)
                .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])),
        }
    }
    /// Hex digit count for formatting (2/4/8).
    pub(super) fn digits(self) -> usize {
        match self {
            Width::U8 => 2,
            Width::U16le | Width::U16be => 4,
            Width::U32le | Width::U32be => 8,
        }
    }
}

/// One sweep-table column: what to extract from each matched record's dumped bytes.
pub(super) enum Col {
    /// The anchor's byte offset within the leaf (where the anchoring LP-string ends).
    Anchor,
    /// A scalar at a leaf offset — absolute (`anchored = false`) or relative to the anchor.
    Scalar {
        anchored: bool,
        off: isize,
        width: Width,
    },
    /// An LP-string's text: the anchoring one (`None`) or the Nth in scan order.
    Str(Option<usize>),
}

/// Parse a signed offset written as hex (`0x1c`) or decimal, with an optional leading `+`/`-`.
fn parse_ioffset(s: &str) -> Option<isize> {
    let s = s.trim();
    let (neg, body) = match s.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let v = if let Some(h) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        isize::from_str_radix(h, 16).ok()?
    } else {
        body.parse::<isize>().ok()?
    };
    Some(if neg { -v } else { v })
}

/// Split a `[offset][:type]` tail into its (optional) offset and width (default `u8`).
fn split_off_type(rest: &str) -> Option<(Option<isize>, Width)> {
    let (offpart, typepart) = match rest.split_once(':') {
        Some((o, t)) => (o, Some(t)),
        None => (rest, None),
    };
    let off = if offpart.is_empty() {
        None
    } else {
        Some(parse_ioffset(offpart)?)
    };
    let width = match typepart {
        Some(t) => Width::parse(t)?,
        None => Width::U8,
    };
    Some((off, width))
}

/// Parse one column spec to `(header, Col)`. `str`/`strN`, `used`/`used±N[:type]`, or an absolute
/// `offset[:type]`. Returns `None` on a malformed spec.
fn parse_col(spec: &str) -> Option<(String, Col)> {
    let raw = spec.trim().to_string();
    let s = raw.as_str();
    if let Some(rest) = s.strip_prefix("str") {
        return match rest {
            "" => Some((raw, Col::Str(None))),
            n => n.parse::<usize>().ok().map(|k| (raw, Col::Str(Some(k)))),
        };
    }
    if let Some(rest) = s.strip_prefix("used") {
        if rest.is_empty() {
            return Some((raw, Col::Anchor));
        }
        let (off, width) = split_off_type(rest)?;
        return Some((
            raw,
            Col::Scalar {
                anchored: true,
                off: off.unwrap_or(0),
                width,
            },
        ));
    }
    let (off, width) = split_off_type(s)?;
    Some((
        raw,
        Col::Scalar {
            anchored: false,
            off: off?,
            width,
        },
    ))
}

/// Parse a comma-separated `--cols` list. Returns `None` if any spec is malformed.
pub(super) fn parse_cols(spec: &str) -> Option<Vec<(String, Col)>> {
    spec.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_col)
        .collect()
}

/// Index of the LP-string the `used` anchor sits at the end of: the first string containing
/// `needle` (case-insensitive) when given, else the last string in the leaf. `None` if no string
/// qualifies.
pub(super) fn anchor_string_index(
    strings: &[(usize, String, usize)],
    needle: Option<&str>,
) -> Option<usize> {
    match needle {
        Some(n) => {
            let nl = n.to_lowercase();
            strings
                .iter()
                .position(|(_, t, _)| t.to_lowercase().contains(&nl))
        }
        None => strings.len().checked_sub(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ioffset_hex_dec_and_sign() {
        assert_eq!(parse_ioffset("4"), Some(4));
        assert_eq!(parse_ioffset("+2"), Some(2));
        assert_eq!(parse_ioffset("-1"), Some(-1));
        assert_eq!(parse_ioffset("0x1c"), Some(28));
        assert_eq!(parse_ioffset("-0x10"), Some(-16));
        assert_eq!(parse_ioffset("zz"), None);
    }

    #[test]
    fn parse_col_variants() {
        // absolute scalar, default width u8
        assert!(matches!(
            parse_col("4"),
            Some((
                _,
                Col::Scalar {
                    anchored: false,
                    off: 4,
                    width: Width::U8
                }
            ))
        ));
        // absolute with explicit type
        assert!(matches!(
            parse_col("0x1c:u16le"),
            Some((
                _,
                Col::Scalar {
                    anchored: false,
                    off: 28,
                    width: Width::U16le
                }
            ))
        ));
        // anchor position
        assert!(matches!(parse_col("used"), Some((_, Col::Anchor))));
        // anchored scalar with offset + type
        assert!(matches!(
            parse_col("used+2:u16be"),
            Some((
                _,
                Col::Scalar {
                    anchored: true,
                    off: 2,
                    width: Width::U16be
                }
            ))
        ));
        // anchored scalar, offset only
        assert!(matches!(
            parse_col("used-1"),
            Some((
                _,
                Col::Scalar {
                    anchored: true,
                    off: -1,
                    width: Width::U8
                }
            ))
        ));
        // strings
        assert!(matches!(parse_col("str"), Some((_, Col::Str(None)))));
        assert!(matches!(parse_col("str2"), Some((_, Col::Str(Some(2))))));
        // malformed
        assert!(parse_col("4:u64le").is_none());
        assert!(parse_col("nonsense").is_none());
    }

    #[test]
    fn parse_cols_list_and_headers() {
        let cols = parse_cols("used, used+2 , 4:u16be").unwrap();
        assert_eq!(
            cols.iter().map(|(h, _)| h.as_str()).collect::<Vec<_>>(),
            ["used", "used+2", "4:u16be"]
        );
        assert!(parse_cols("used, bogus:u9").is_none());
    }

    #[test]
    fn anchor_index_marker_vs_last() {
        let strings = vec![
            (0usize, "field_name".to_string(), 14usize),
            (14, "note".to_string(), 8),
        ];
        // default = last string
        assert_eq!(anchor_string_index(&strings, None), Some(1));
        // marker match (case-insensitive substring)
        assert_eq!(anchor_string_index(&strings, Some("FIELD")), Some(0));
        // no qualifying string
        assert_eq!(anchor_string_index(&strings, Some("zzz")), None);
        assert_eq!(anchor_string_index(&[], None), None);
    }

    #[test]
    fn width_reads_endianness() {
        let b = [0x01, 0x02, 0x03, 0x04];
        assert_eq!(Width::U16be.read(&b, 0), Some(0x0102));
        assert_eq!(Width::U16le.read(&b, 0), Some(0x0201));
        assert_eq!(Width::U32be.read(&b, 0), Some(0x01020304));
        assert_eq!(Width::U32le.read(&b, 0), Some(0x04030201));
        assert_eq!(Width::U8.read(&b, 3), Some(0x04));
        assert_eq!(Width::U16be.read(&b, 3), None);
    }
}
