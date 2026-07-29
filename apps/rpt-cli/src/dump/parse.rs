//! Parsing and byte-view primitives shared across the `dump` submodules: option/selector parsing,
//! length-prefixed string decoding, the hex/scalar formatting helpers, and the `--cols` column
//! grammar (`Width`/`Col`).

use rpt_reader::raw::{Dialect, LpScan, LpString};

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
///
/// A name is resolved by the reader's own registry lookup rather than against a list of
/// vocabularies kept here: the name identifies its own vocabulary — `QeIndex` names `0x0008`
/// wherever it is asked for — and a list kept here is one a vocabulary added later drops out of,
/// which once made every parameter-values record name unreachable.
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
    rpt_reader::raw::RecordTag::from_name(s).map(|(tag, _)| tag.value())
}

/// A record type's `Name(0x00nn)` label in `dialect` (or bare hex where it has no name there).
pub(super) fn type_label(rtype: u16, dialect: Dialect) -> String {
    rpt_reader::raw::RecordTag(rtype).label(dialect)
}

/// The length-prefixed strings in a record's shown bytes, read under the reader's own rule.
///
/// The dump is the instrument a record layout is read with, so what it calls a string must be what
/// the reader calls one: a second copy of the rule here would go on showing the old answer after the
/// reader tightened or loosened it, and a byte run the reader does not read as text would still
/// appear as a string in the annotation.
pub(super) fn lp_strings(bytes: &[u8]) -> Vec<LpString> {
    rpt_reader::raw::lp_strings(bytes, LpScan::Consume)
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

/// Resolve the scalar-probe cap: default 64 bytes, `all` = every byte shown, a number = that many,
/// 0 = off.
pub(super) fn probe_cap(opt: Option<&str>, shown_len: usize) -> usize {
    match opt {
        None => 64.min(shown_len),
        Some(s) if s.eq_ignore_ascii_case("all") => shown_len,
        Some(s) => parse_num(s).unwrap_or(64).min(shown_len),
    }
}

// ── Corpus sweep + anchor-relative columns ────────────────────────────────────────────────────
//
// The single-record dump is per-file; separating a confound needs the *same* derived value
// pulled from a record type across a whole corpus into one table (e.g. `used+2`/`used+3`/`@4` over
// every chart report). `--glob` sweeps a directory into that table; `--cols` says which bytes, and
// `used`/`--anchor-string` anchor an offset at a decoded LP-string's end rather than at an absolute
// position (a field's trailing tail moves when the field name's length changes).

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
    /// The anchor's byte offset within the bytes shown (where the anchoring LP-string ends).
    Anchor,
    /// A scalar at a byte offset — absolute (`anchored = false`) or relative to the anchor.
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
/// `needle` (case-insensitive) when given, else the last string shown. `None` if no string
/// qualifies.
pub(super) fn anchor_string_index(strings: &[LpString], needle: Option<&str>) -> Option<usize> {
    match needle {
        Some(n) => {
            let nl = n.to_lowercase();
            strings
                .iter()
                .position(|s| s.text.to_lowercase().contains(&nl))
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

    /// A record's label names it in the vocabulary of the stream it came from. Labelling a session
    /// record from the report definition's registry describes an unrelated record — a font for the
    /// query engine's index, a printer for its table.
    #[test]
    fn a_type_is_labelled_in_its_own_dialect() {
        assert_eq!(type_label(0x0008, Dialect::Contents), "Font(0x0008)");
        assert_eq!(type_label(0x0008, Dialect::QeSession), "QeIndex(0x0008)");
        assert_eq!(type_label(0x0003, Dialect::QeSession), "QeTable(0x0003)");
        // The saved-data catalog names only what its decoder reads; the rest stays hex.
        assert_eq!(
            type_label(0x0041, Dialect::Catalog),
            "SavedFieldHeader(0x0041)"
        );
        assert_eq!(type_label(0x0008, Dialect::Catalog), "0x0008");
    }

    /// `--type` takes a name from any stream's vocabulary, since the name identifies its own. Each
    /// name below is exclusive to one vocabulary, so every one of them resolving is what says the
    /// selector reaches them all — a vocabulary left out of the search refuses its own record names.
    #[test]
    fn a_type_selector_resolves_a_name_from_any_dialect() {
        assert_eq!(parse_type_selector("Font"), Some(0x0008));
        assert_eq!(parse_type_selector("QeIndex"), Some(0x0008));
        assert_eq!(parse_type_selector("qetable"), Some(0x0003));
        assert_eq!(parse_type_selector("SavedBatchEntry"), Some(0x006d));
        assert_eq!(parse_type_selector("CurrentValueRecord"), Some(0x0031));
        assert_eq!(
            parse_type_selector("datasourceparameterentry"),
            Some(0x003b)
        );
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
        let strings = lp_strings(&[
            0, 0, 0, 11, b'f', b'i', b'e', b'l', b'd', b'_', b'n', b'a', b'm', b'e', 0, //
            0, 0, 0, 5, b'n', b'o', b't', b'e', 0,
        ]);
        assert_eq!(
            strings.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
            ["field_name", "note"]
        );
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
