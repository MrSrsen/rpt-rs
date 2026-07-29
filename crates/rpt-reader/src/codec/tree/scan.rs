//! Finding the records: the scan that builds a tree out of a logical stream.
//!
//! Header detection reads the bit-packed TSLV header through [`crate::codec::tslv::decode_header`]
//! — the same framing decode [`crate::codec::flat_records`] reads — demasked with the current stack
//! mask, and then judges the result. Decoding never panics and is bounded by each record's declared
//! length.
//!
//! # How a child record is found here, and how the format finds one
//!
//! In the format, a child record exists because the parent's reader **asked for one, by type**, at
//! a point in its own field sequence. Nothing about the bytes marks the spot; the position does.
//!
//! This reader has no field sequence to walk — it builds the tree before anything reads a record's
//! fields — so it **scans**: every byte offset is probed for something header-shaped. Scanning has
//! no notion of what should be there, so it fails silently in both directions, and neither failure
//! can raise a diagnostic:
//!
//! - with too little discrimination, field data that happens to look like a header becomes a
//!   record — and a false header that begins mid-field takes the rest of that field with it, so
//!   the parent loses field bytes;
//! - with too much, a genuine record that does not fit the filter's shape is swallowed into its
//!   parent's field bytes.
//!
//! The two filters that hold that balance are [`Dialect::scan_schema_prefix`] and the length-width
//! restriction in [`read_header`]. **Both are properties of this reader, not rules of the format**
//! — the format constrains neither. They are scaffolding: better constants than no constants, and
//! still constants.
//!
//! # Which string format the records read here are in
//!
//! A record declares, in its own header, which of the format's two string wire forms its content
//! uses ([`crate::codec::tslv::Flags::strings_enhanced`]). This reader reads only the
//! length-prefixed one, so it takes only records that declare it; see the check in
//! [`read_header`]. That is an assumption about what the corpus contains, held at the door rather
//! than inside every string read, and the reader refuses a record it would otherwise mis-frame
//! rather than trusting it.
//!
//! [`ChildRule`] is how the scaffolding comes down. Where a record type has a field table, the
//! table declares which types may nest inside it, and a header for anything else is rejected
//! however plausible its bytes — that is the *what* of the format's ask, and it removes the
//! guessing for those types. The *where* is what makes the filters unnecessary, and it arrives only
//! when the tree is built by walking the tables themselves; until then the filters stay, and no
//! reader should mistake them for the format.
//!
//! # Records that state no version
//!
//! A header carries its schema word only when that version differs from the default the writing
//! archive was opened at, so a record type that has never been revised is written four bytes
//! narrower with no word at all ([`Dialect::default_schema`]). Such a header has no version for
//! the prefix heuristic to weigh, so it is accepted only where a declaration asks for that very
//! type — never at the top level of a stream, and never inside a record type that has no table.
//! Refusing it outright is not free: its bytes stay in the parent's field data, where the header
//! reads as two stray bytes and the content as one opaque counted block.

use crate::field_table::DeclaredChildren;

use super::node::RecordNode;
use crate::codec::dialect::Dialect;

/// The length field's widest form, in bytes — the only width the scan accepts for a record with
/// content.
const LEN_KIND_WIDE: u8 = 4;

/// No length field at all — how an empty record is framed.
const LEN_KIND_EMPTY: u8 = 0;

/// How deep the scan will descend before it stops looking inside a record.
const MAX_DEPTH: usize = 32;

/// What a record type declares its content's children to be, given its type and schema version.
/// `None` for a record type with no declaration — the reader falls back to scanning inside it.
///
/// This is a parameter of the parse, not a property of the stream: the declarations live with the
/// field tables, and the caller that knows both halves supplies the rule.
///
/// The schema is a parameter because a type number can host structurally different records at
/// different versions; a declaration written for one must not be applied to the other.
pub(crate) type ChildRule =
    fn(rtype: u16, schema: u16, dialect: Dialect) -> Option<DeclaredChildren>;

/// Try to read a record header at `pos` under stack mask `m`, bounded by `limit`.
/// Returns `(rtype, schema, content_len, header_len)` if the bytes are header-shaped *and* pass the
/// scan's filters.
///
/// The framing itself is decoded once for the whole crate, in [`crate::codec::tslv::decode_header`];
/// everything below it here is judgement about whether a header belongs at this offset, which is
/// this reader's problem alone. A reader told where the records are needs none of it.
///
/// `expect` is the enclosing record type's declaration of its children, when it has one: a header
/// is then accepted only for a type the declaration asks for.
///
/// A declaration **narrows**; it does not replace the scan heuristics. It answers *what* may nest
/// here but not *where*, and a position-blind reader needs both: with the heuristics dropped for a
/// declared type, the tail of an object name (`…SalesEmplo` + `yee1`) decodes as a header for a
/// declared child type and truncates the name. So the type filter removes false positives and adds
/// none, and the heuristics stay until position replaces them.
fn read_header(
    d: &[u8],
    pos: usize,
    m: u8,
    limit: usize,
    dialect: Dialect,
    expect: Option<DeclaredChildren>,
) -> Option<(u16, u16, usize, usize)> {
    let h = crate::codec::tslv::decode_header(d, pos, m)?;
    let flags = h.flags;

    // Bit 3, the running XOR mask, is set by every record a report writer emits, so a candidate
    // offset without it is field data. The two bits above it size the length field and the two
    // below carry the type's high byte, so they are left free; bits 4 and 5 are checked below.
    if !flags.simple_encryption {
        return None;
    }
    // Every string this reader reads is framed as the *enhanced* form — a big-endian `u32` byte
    // count then the bytes. That is not the only form the format has: the flag byte's bit 4 names
    // which of the two a record's content uses ([`crate::codec::tslv::Flags::strings_enhanced`]),
    // and a record written in the *simple* form frames its strings NUL-terminated with no count at
    // all. Read with the wrong assumption, the first four characters of the text become a length,
    // so such a record is refused here rather than decoded into plausible nonsense.
    //
    // The engine's own writer never emits the simple form for a record here, which is why the bit
    // doubles as scan evidence: dropping it from the filter admits header-shaped field data and
    // invents records.
    if !flags.strings_enhanced {
        return None;
    }
    // The framing layer sizes the length field to the payload and permits all four widths, but a
    // reader that probes every byte offset cannot afford them all: the narrower the header, the less
    // of it is evidence that it *is* one. Report files use exactly two widths — 4 bytes for a record
    // with content, none for an empty record — and admitting the 1- and 2-byte forms costs real
    // decodes, misreading field data as nested records. Which streams the empty form is affordable
    // in is [`Dialect::scans_empty_records`]; the rest keep the widest form as their only shape.
    //
    // An empty record must also state its version. With neither a length nor a schema word, a
    // header is a flag byte and a type, and a type of `0x0000` is what a spare pair of zero bytes
    // decodes to — so the form carries no evidence at all where it is most easily imitated. Every
    // empty record any stream really holds states its version ([`crate::field_table::framing`]), so
    // requiring it costs nothing and keeps a declaration from turning field data into records.
    let empty_form =
        flags.len_kind == LEN_KIND_EMPTY && dialect.scans_empty_records() && flags.has_schema;
    if flags.len_kind != LEN_KIND_WIDE && !empty_form {
        return None;
    }

    // A declaration says which record types may nest here; anything else is field data, whatever
    // it looks like. The prefix heuristic still runs underneath it — see above.
    if expect.is_some_and(|decl| !decl.declares(h.rtype)) {
        return None;
    }

    let schema = match h.schema {
        // The version the header states is weighed against the prefix the dialect's records share:
        // one outside it is well formed but indistinguishable from field data to a scan.
        Some(schema) => {
            if dialect
                .scan_schema_prefix()
                .is_some_and(|prefix| (schema >> 8) as u8 != prefix)
            {
                return None;
            }
            schema
        }
        // Without the word there is no version to check the prefix heuristic against, and a
        // four-byte header is thin evidence on its own. What stands in its place is the stronger
        // filter: the enclosing record's declaration must ask for this very type here. So the
        // short form is read where a record was expected and nowhere else — never at the top level
        // of a stream, and never inside a record type that has no field table to ask with.
        None => {
            if !expect.is_some_and(|decl| decl.declares(h.rtype)) {
                return None;
            }
            dialect.default_schema()?
        }
    };

    if pos + h.header_len + h.content_len > limit {
        return None;
    }
    Some((h.rtype, schema, h.content_len, h.header_len))
}

/// Parse `logical[span]` as a sequence of records under stack mask `m`.
///
/// `expect` is the declaration of the enclosing record, when its type has a field table; at the top
/// level of a stream, and inside any record type that has none, it is `None` and the sequence is
/// scanned for.
fn parse_seq(
    d: &[u8],
    span: std::ops::Range<usize>,
    m: u8,
    depth: usize,
    dialect: Dialect,
    rule: Option<ChildRule>,
    expect: Option<DeclaredChildren>,
) -> Vec<RecordNode> {
    let mut out = Vec::new();
    let mut p = span.start;
    let end = span.end;
    while p < end {
        let Some((rtype, schema, len, header_len)) = read_header(d, p, m, end, dialect, expect)
        else {
            // Field byte: not a record header here. Advance; the surrounding record's declared
            // length keeps us anchored.
            p += 1;
            continue;
        };
        let content_start = p + header_len;
        let content_end = content_start + len;
        let child_mask = m ^ (rtype as u8);
        let children = if depth < MAX_DEPTH {
            parse_seq(
                d,
                content_start..content_end,
                child_mask,
                depth + 1,
                dialect,
                rule,
                rule.and_then(|rule| rule(rtype, schema, dialect)),
            )
        } else {
            Vec::new()
        };
        out.push(RecordNode {
            rtype,
            schema,
            offset: p,
            content_start,
            content_end,
            mask: child_mask,
            children,
        });
        p = content_end;
    }
    out
}

/// Parse the whole logical report into a recursive record tree of report-definition records
/// (top-level records read under mask 0). The top level itself is scanned — no record encloses it —
/// and inside each record `rule` declares the children, for the types it answers for.
pub(crate) fn parse_tree(logical: &[u8], rule: Option<ChildRule>) -> Vec<RecordNode> {
    parse_seq(
        logical,
        0..logical.len(),
        0,
        0,
        Dialect::Contents,
        rule,
        None,
    )
}

/// Parse a `QESession` logical record stream into a recursive tree. Same bit-packed TSLV framing +
/// stack-XOR mask as [`parse_tree`], over the query engine's own record vocabulary.
pub(crate) fn parse_tree_qe_session(logical: &[u8], rule: Option<ChildRule>) -> Vec<RecordNode> {
    parse_seq(
        logical,
        0..logical.len(),
        0,
        0,
        Dialect::QeSession,
        rule,
        None,
    )
}

/// Parse the saved parameter-values stream into a recursive tree. The same TSLV framing and
/// stack-XOR mask as [`parse_tree`], over the vocabulary the `ReportParametersStream` is written in
/// — its type numbers are its own, so it cannot be read as report-definition records.
pub(crate) fn parse_tree_report_parameters(
    logical: &[u8],
    rule: Option<ChildRule>,
) -> Vec<RecordNode> {
    parse_seq(
        logical,
        0..logical.len(),
        0,
        0,
        Dialect::ReportParameters,
        rule,
        None,
    )
}

/// Parse a QE-framed stream whose records are written by several components at once — the saved-data
/// `DataSourceManager` catalog. Its record types share no schema prefix, so at the top level of the
/// stream the flag byte is the whole header filter; inside a record type `rule` answers for, the
/// declaration is.
pub(crate) fn parse_tree_catalog(logical: &[u8], rule: Option<ChildRule>) -> Vec<RecordNode> {
    parse_seq(
        logical,
        0..logical.len(),
        0,
        0,
        Dialect::Catalog,
        rule,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::super::wired::{parse_tree, parse_tree_qe_session};
    use super::*;
    use crate::codec::dialect::{CONTENTS_DEFAULT_SCHEMA, QE_SESSION_DEFAULT_SCHEMA};

    #[test]
    fn parses_a_nested_record() {
        // Outer record type 0x10 (flag f8, schema 0x0700, len = 8) whose content is one inner
        // record type 0x03 with 0 content. Inner header is masked by the outer mask 0x10.
        let inner_mask = 0x10u8;
        let inner: Vec<u8> = [0xf8u8, 0x03, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00]
            .iter()
            .map(|b| b ^ inner_mask)
            .collect();
        let mut stream = vec![0xf8u8, 0x10, 0x07, 0x00, 0x00, 0x00, 0x00, 0x08];
        stream.extend(inner);

        let tree = parse_tree(&stream);
        assert_eq!(tree.len(), 1);
        let outer = &tree[0];
        assert_eq!(outer.rtype, 0x10);
        assert_eq!(outer.mask, 0x10);
        assert_eq!(outer.children.len(), 1);
        let child = &outer.children[0];
        assert_eq!(child.rtype, 0x03);
        assert_eq!(child.mask, 0x10 ^ 0x03);
        assert!(child.is_leaf());
    }

    /// The schema word is one big-endian number, kept whole. Read little-endian its bytes swap and
    /// a `0x0701` record reports `0x0107` — a *lower* version than `0x0700`, which inverts every
    /// comparison a version is for.
    #[test]
    fn the_schema_word_is_one_big_endian_number() {
        let stream = vec![0xf8u8, 0x29, 0x07, 0x01, 0x00, 0x00, 0x00, 0x00];
        let tree = parse_tree(&stream);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].rtype, 0x0029);
        assert_eq!(tree[0].schema, 0x0701);
        assert!(tree[0].schema > 0x0700, "a later version compares greater");
    }

    /// An empty record is written in the narrow form — a 4-byte header with no length field at all.
    /// Rejecting it does not fail loudly: its bytes silently become field data of whatever contains
    /// it, which shifts every field the decoder reads after that point.
    #[test]
    fn an_empty_record_has_no_length_field() {
        // A 4-byte record inside an 8-byte one, whose own content is exactly that child.
        let mask = 0x10u8;
        let inner: Vec<u8> = [0x38u8, 0x51, 0x07, 0x00]
            .iter()
            .map(|b| b ^ mask)
            .collect();
        let mut stream = vec![0xf8u8, 0x10, 0x07, 0x00, 0x00, 0x00, 0x00, 0x04];
        stream.extend(inner);

        let tree = parse_tree(&stream);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].children.len(), 1, "the empty record is a child");
        assert_eq!(tree[0].children[0].rtype, 0x0051);
        assert_eq!(tree[0].children[0].content_start, tree[0].content_end);
        assert!(
            tree[0].joined_runs(&stream).is_empty(),
            "its header is framing, not field data"
        );
    }

    /// A record that declares the *simple* (NUL-terminated, unprefixed) string form is not read
    /// here, because every string this reader frames is length-prefixed and reading one form as the
    /// other turns the first four characters of the text into a byte count. The two headers below
    /// are identical but for that one bit.
    #[test]
    fn a_simple_string_record_is_not_read_as_a_length_prefixed_one() {
        let enhanced = vec![0xf8u8, 0x29, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00];
        let tree = parse_tree(&enhanced);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].rtype, 0x0029);

        // Same record with bit 4 cleared: the content's strings would be framed the other way.
        let mut simple = enhanced.clone();
        simple[0] &= !0b0001_0000;
        assert!(
            parse_tree(&simple).is_empty(),
            "a record declaring the simple string form must not be read as length-prefixed"
        );
    }

    /// Both string forms are decoded off the flag byte, so a writer has one place to set the mode
    /// and a reader one place to learn it.
    #[test]
    fn the_flag_byte_names_the_string_form() {
        use crate::codec::tslv::Flags;
        assert!(Flags::decode(&[0xf8, 0x29]).strings_enhanced);
        assert!(!Flags::decode(&[0xe8, 0x29]).strings_enhanced);
    }

    /// The `Qe` dialect has no schema prefix to cross-check a candidate header against, so there
    /// the flag byte is the whole filter and only the widest form is accepted.
    #[test]
    fn the_qe_dialect_takes_only_the_widest_header() {
        let stream = vec![0x38u8, 0x51, 0x09, 0x00];
        assert!(parse_tree_catalog(&stream, None).is_empty());
    }

    /// The 1- and 2-byte length widths are legal at the framing layer and never written to a report,
    /// so accepting them buys nothing and costs decodes: ordinary field data matches them.
    /// These bytes are a real section run — a height, a count and a name length — whose middle
    /// reads as a 1-byte-length header if the width is allowed, swallowing seven bytes of the run.
    #[test]
    fn a_narrow_length_field_is_not_a_header() {
        let run = [
            0x00, 0x00, 0x01, 0x7e, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x0f,
        ];
        let mask = 0x8cu8;
        let mut stream = vec![0xf8u8, 0x8c, 0x07, 0x00, 0x00, 0x00, 0x00, run.len() as u8];
        stream.extend(run.iter().map(|b| b ^ mask));

        let tree = parse_tree(&stream);
        assert_eq!(tree.len(), 1);
        assert!(tree[0].children.is_empty(), "field data is not a record");
        assert_eq!(tree[0].joined_runs(&stream), run);
    }

    /// The scan's schema-prefix filter, and its cost. A version outside the prefix is a perfectly
    /// well-formed record — read without the filter it decodes fine — but a scan that keeps every
    /// candidate cannot separate a record from field data, so it is dropped. This is the reader's
    /// bargain, not the format's rule, and it is what loses a record type written at a version
    /// outside the prefix.
    #[test]
    fn a_schema_outside_the_prefix_is_dropped_by_the_scan() {
        let stream = vec![0xf8u8, 0x29, 0x05, 0x50, 0x00, 0x00, 0x00, 0x00];
        assert!(parse_tree(&stream).is_empty());
        // The same bytes under a dialect with no prefix to check: a well-formed record.
        let unfiltered = parse_tree_catalog(&stream, None);
        assert_eq!(unfiltered.len(), 1);
        assert_eq!(unfiltered[0].rtype, 0x0029);
        assert_eq!(unfiltered[0].schema, 0x0550);
    }

    /// A record type with a field table declares which types nest inside it, and that declaration
    /// is a filter the bytes cannot talk their way past: an undeclared type stays field data no
    /// matter how well-formed its header. `0x008a` declares only `0x0151`.
    #[test]
    fn a_declared_type_is_the_only_child_taken() {
        // An 8-byte record of type `T` holding one empty `0x0165` record.
        let framed = |t: u8| {
            let mask = t;
            let inner: Vec<u8> = [0x39u8, 0x65, 0x07, 0x00]
                .iter()
                .map(|b| b ^ mask)
                .collect();
            let mut s = vec![0xf8u8, t, 0x07, 0x00, 0x00, 0x00, 0x00, 0x04];
            s.extend(inner);
            s
        };

        // Inside `0x008a`, whose table declares `0x0151` and nothing else.
        let declared = framed(0x8a);
        let tree = parse_tree(&declared);
        assert_eq!(tree.len(), 1);
        assert!(
            tree[0].children.is_empty(),
            "an undeclared type is field data, not a child"
        );
        assert_eq!(tree[0].joined_runs(&declared).len(), 4);

        // The identical bytes inside `0x0010`, which has no table: nothing declares anything, so
        // the reader falls back to the scan and promotes them.
        let scanned = framed(0x10);
        let tree = parse_tree(&scanned);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].rtype, 0x0165);
    }

    /// The declaration is keyed on the record's version as well as its type, because one type
    /// number can host structurally unrelated records at two versions. `0x0007` has a table at
    /// `0x0700` only, so a `0x0701` record of that type is scanned rather than constrained.
    #[test]
    fn a_declaration_does_not_reach_across_versions() {
        use crate::field_table::declared_children;
        assert!(declared_children(0x0007, 0x0700, Dialect::Contents).is_some());
        assert!(declared_children(0x0007, 0x0701, Dialect::Contents).is_none());
    }

    /// A record whose version is its stream's default states no schema word, and the reader hands
    /// it that default back. The short header is four bytes narrower, so the type, the length and
    /// the content all sit two bytes earlier than in the stated form.
    #[test]
    fn a_header_without_a_schema_word_takes_the_streams_default() {
        // A `0x008a`, whose table declares `0x0151` and nothing else, holding one empty `0x0151`
        // written in the short form: no schema word between the type and the length.
        let mask = 0x8au8;
        let inner: Vec<u8> = [0xd9u8, 0x51, 0x00, 0x00, 0x00, 0x00]
            .iter()
            .map(|b| b ^ mask)
            .collect();
        let mut stream = vec![0xf8u8, 0x8a, 0x07, 0x00, 0x00, 0x00, 0x00, 0x06];
        stream.extend(inner);

        let tree = parse_tree(&stream);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].children.len(), 1, "the short form is a record");
        assert_eq!(tree[0].children[0].rtype, 0x0151);
        assert_eq!(tree[0].children[0].schema, CONTENTS_DEFAULT_SCHEMA);
        assert!(
            tree[0].joined_runs(&stream).is_empty(),
            "its header is framing, not field data"
        );
    }

    /// Without the schema word there is no version for the prefix heuristic to weigh, so the short
    /// form is taken only where the enclosing record's declaration asks for that very type. The
    /// identical bytes inside a record type that declares nothing stay field data.
    #[test]
    fn a_header_without_a_schema_word_is_taken_only_where_it_is_declared() {
        let framed = |t: u8| {
            let inner: Vec<u8> = [0xd9u8, 0x51, 0x00, 0x00, 0x00, 0x00]
                .iter()
                .map(|b| b ^ t)
                .collect();
            let mut s = vec![0xf8u8, t, 0x07, 0x00, 0x00, 0x00, 0x00, 0x06];
            s.extend(inner);
            s
        };

        // `0x0010` has no field table, so nothing declares anything and the scan is all there is.
        let undeclared = framed(0x10);
        let tree = parse_tree(&undeclared);
        assert_eq!(tree.len(), 1);
        assert!(tree[0].children.is_empty());
        assert_eq!(tree[0].joined_runs(&undeclared).len(), 6);
    }

    /// The query engine versions its records independently, so its default is its own — and its
    /// index records are a real short-form case: a `0x0003 QeTable` declares them, and their
    /// content is masked with the type's low byte like any other record's.
    #[test]
    fn the_query_engines_default_schema_is_its_own() {
        let mask = 0x03u8;
        let inner: Vec<u8> = [0xd8u8, 0x08, 0x00, 0x00, 0x00, 0x00]
            .iter()
            .map(|b| b ^ mask)
            .collect();
        let mut stream = vec![0xf8u8, 0x03, 0x09, 0x05, 0x00, 0x00, 0x00, 0x06];
        stream.extend(inner);

        let tree = parse_tree_qe_session(&stream);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].schema, 0x0905, "the parent states its own version");
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].rtype, 0x0008);
        assert_eq!(tree[0].children[0].schema, QE_SESSION_DEFAULT_SCHEMA);
        assert_eq!(tree[0].children[0].mask, 0x03 ^ 0x08);
    }
}
