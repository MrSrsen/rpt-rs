//! Per-record decoded summaries for inspection tooling.
//!
//! Given a single [`Node`] and the vocabulary its stream is written in, [`summarize`] recognizes
//! the record types the model build decodes into semantic-model fields and returns a concise,
//! human-readable [`RecordSummary`] of the decoded values — the same values the model carries,
//! surfaced against the raw record so a tree/inspection view can show *what a record means* rather
//! than only its bytes. It fills in no part of the model itself: it reads each record through the
//! very decoders the model build reads it with.
//!
//! A record is read from its content in wire order ([`Unknown::parts`]), never from the runs
//! concatenated together. Two paths read it:
//!
//! - a record type with a **field table** is projected into that table's own content model and read
//!   declaratively, so a field on the far side of a nested record is reached by declaring that
//!   record rather than by an offset into a buffer the file does not contain;
//! - a record type without one is decoded by hand from its **first run** of field bytes, which is
//!   contiguous. A field in a later run sits past a nested record, so it is not read at all rather
//!   than read from bytes that are not adjacent to it.
//!
//! A record with neither — or one too short to decode — yields `None`, and the caller keeps its raw
//! preview.

use crate::build_model::{
    decode_boolean_format, decode_common_format, decode_date_format, decode_datetime_format,
    decode_devmode, decode_numeric_format, decode_string_format, decode_time_format,
    field_format_table, special_field_name,
};
use crate::codec::Dialect;
use crate::field_table::cursor::{ChildRef, Piece, RecordContent, StringFormat};
use crate::field_table::table::{read_strings, Cell, Row, Table};
use crate::field_table::tables as ft;
use crate::model::SummaryOperation;
use crate::records::rtype::*;
use crate::records::{Node, Part, Unknown};

/// A decoded, human-readable summary of one record's stored values: ordered `(key, value)` pairs,
/// each value rendered concisely (an enum by name, a number, a flag). Ordered so the display and
/// any structured (JSON) rendering of it are stable.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordSummary {
    /// The decoded fields, in a fixed, meaningful order.
    pub fields: Vec<(&'static str, String)>,
}

impl RecordSummary {
    fn new(fields: Vec<(&'static str, String)>) -> Self {
        RecordSummary { fields }
    }

    /// A one-line `key=value key=value` rendering of the decoded fields.
    pub fn one_line(&self) -> String {
        self.fields
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Project a record's content into the cursor's own model — the same wire-ordered runs and nested
/// records, so a field table walks it exactly as it walks a record read from the stream.
///
/// The two vocabularies are deliberately distinct rather than one type: a [`Part`] owns the nested
/// record, because the typed tree exists to carry it, while a [`Piece`] only names one — the type,
/// schema and framed length are all a field table needs to match the child it declared and step
/// over it. This is where they meet, and the only place either is translated into the other.
fn content(u: &Unknown) -> RecordContent {
    RecordContent {
        rtype: u.rtype,
        schema: u.schema,
        pieces: u
            .parts
            .iter()
            .map(|p| match p {
                Part::Run(b) => Piece::Run(b.clone()),
                Part::Child { framed_len, node } => Piece::Child(ChildRef {
                    rtype: node_rtype(node),
                    schema: node_schema(node),
                    framed_len: *framed_len,
                }),
            })
            .collect(),
    }
}

/// A nested node's record type. A modelled node is identified by its variant rather than by a
/// stored type word, so its own record type is named here.
fn node_rtype(node: &Node) -> u16 {
    match node {
        Node::FieldDef(_) => FIELD_DEFINITION,
        Node::Unknown(u) => u.rtype,
    }
}

fn node_schema(node: &Node) -> u16 {
    match node {
        Node::FieldDef(_) => 0,
        Node::Unknown(u) => u.schema,
    }
}

/// Read a record through its field table, framing its strings in the **enhanced** form.
///
/// The content is rebuilt from the node's parts and carries no header to declare a form, so the
/// only one the record-tree reader admits is stated here.
///
/// The summaries below take each value by name through [`Row::num`], which does not care what
/// width or signedness the table declares — a display summary should follow a corrected table
/// rather than report a default when a field's wire type is refined.
fn row(u: &Unknown, table: &Table) -> Row {
    read_strings(table, &content(u), StringFormat::Enhanced).row
}

/// The first run of the record's own field bytes. A run is contiguous in the file, so an offset
/// into it addresses the byte it names; bytes in a later run sit past a nested record and are not
/// reachable this way, so they are not read at all.
fn first_run(u: &Unknown) -> &[u8] {
    u.runs().next().unwrap_or_default()
}

/// A concise decoded summary of a record read in `dialect`, or `None` when the record type has no
/// decoder there (or its content is too short to decode). Never panics: every decoder tolerates a
/// short/malformed record.
///
/// The dialect is the vocabulary the stream the record came from is written in. A node carries a
/// type number and not the stream it was read from, and every decoder below reads a
/// report-definition record — `0x0007` is the page-setup DEVMODE there and a command parameter in
/// the query-engine session — so a record read in another vocabulary is left unsummarized rather
/// than decoded as the record this one happens to number the same.
pub fn summarize(node: &Node, dialect: Dialect) -> Option<RecordSummary> {
    // Field definitions are already surfaced by the typed tree (name + value type); leave their
    // preview to the caller and summarize only the unmodelled records this layer can decode.
    let Node::Unknown(u) = node else {
        return None;
    };
    if dialect != Dialect::Contents {
        return None;
    }
    // A field-format value record names its own table through the family table the model build
    // reads it by, so the summaries below name only the slot each one describes.
    if let Some(table) = field_format_table(u.rtype) {
        return field_format(u.rtype, &row(u, table));
    }
    match u.rtype {
        // Read through the record type's own field table.
        GROUP_AREA_FORMAT => Some(group_area(&row(u, &ft::GROUP_AREA_FORMAT))),
        SUMMARY_FIELD_DEFINITION => summary(&row(u, &ft::SUMMARY_FIELD_DEFINITION)),
        FIELD_OBJECT => special_field(&row(u, &ft::FIELD_OBJECT)),
        CROSSTAB_CUSTOM_MEMBERS => Some(custom_members(&row(u, &ft::CROSSTAB_CUSTOM_MEMBERS))),
        // Read by hand from the first run, until each type has a field table of its own.
        PAGE_DEVMODE => Some(devmode(first_run(u))),
        OLE_OBJECT_ITEM => ole_item(first_run(u)),
        _ => None,
    }
}

/// Summarize one field-format value record, already read through its own table: its type names the
/// format slot it fills, and each slot renders the members it stores.
fn field_format(value_rtype: u16, row: &Row) -> Option<RecordSummary> {
    Some(match value_rtype {
        NUMERIC_FIELD_FORMAT => numeric(row),
        STRING_FIELD_FORMAT => string(row),
        DATE_FIELD_FORMAT => date(row),
        TIME_FIELD_FORMAT => time(row),
        DATE_TIME_FIELD_FORMAT => datetime(row),
        BOOLEAN_FIELD_FORMAT => boolean(row),
        COMMON_FIELD_FORMAT => common(row),
        _ => return None,
    })
}

fn numeric(row: &Row) -> RecordSummary {
    let f = decode_numeric_format(row);
    RecordSummary::new(vec![
        ("DecimalPlaces", f.decimal_places.to_string()),
        ("Negative", format!("{:?}", f.negative)),
        ("CurrencySymbol", format!("{:?}", f.currency_symbol)),
        ("CurrencyPosition", format!("{:?}", f.currency_position)),
        ("Rounding", format!("{:?}", f.rounding)),
        ("ThousandsSeparator", f.thousands_separator.to_string()),
    ])
}

fn string(row: &Row) -> RecordSummary {
    let f = decode_string_format(row);
    RecordSummary::new(vec![
        ("EnableWordWrap", f.enable_word_wrap.to_string()),
        ("MaxNumberOfLines", f.max_number_of_lines.to_string()),
        ("TextFormat", format!("{:?}", f.text_format)),
        ("ReadingOrder", format!("{:?}", f.reading_order)),
        ("FirstLineIndent", f.indent.first_line_indent.0.to_string()),
        ("LeftIndent", f.indent.left_indent.0.to_string()),
        ("RightIndent", f.indent.right_indent.0.to_string()),
    ])
}

fn date(row: &Row) -> RecordSummary {
    let f = decode_date_format(row);
    RecordSummary::new(vec![
        ("DateOrder", format!("{:?}", f.date_order)),
        ("Year", format!("{:?}", f.year)),
        ("Month", format!("{:?}", f.month)),
        ("Day", format!("{:?}", f.day)),
    ])
}

fn time(row: &Row) -> RecordSummary {
    let f = decode_time_format(row);
    RecordSummary::new(vec![
        ("Hour", format!("{:?}", f.hour)),
        ("Minute", format!("{:?}", f.minute)),
        ("Second", format!("{:?}", f.second)),
    ])
}

fn datetime(row: &Row) -> RecordSummary {
    let f = decode_datetime_format(row);
    RecordSummary::new(vec![
        ("Order", format!("{:?}", f.order)),
        ("Separator", format!("{:?}", f.separator)),
    ])
}

fn boolean(row: &Row) -> RecordSummary {
    let f = decode_boolean_format(row);
    RecordSummary::new(vec![("OutputType", format!("{:?}", f.output_type))])
}

fn common(row: &Row) -> RecordSummary {
    let f = decode_common_format(row);
    RecordSummary::new(vec![
        ("SuppressIfDuplicated", f.suppress_if_duplicated.to_string()),
        ("UseSystemDefaults", f.use_system_defaults.to_string()),
    ])
}

/// Summarize a `0x88` group-area format. `VisibleGroupNumberPerPage` sits on the far side of the
/// record's nested `0x0151`, so the table's declaration of that child is what makes it reachable.
fn group_area(row: &Row) -> RecordSummary {
    RecordSummary::new(vec![
        (
            "RepeatGroupHeader",
            (row.num("repeat_group_header") != 0).to_string(),
        ),
        (
            "KeepGroupTogether",
            (row.num("keep_group_together") != 0).to_string(),
        ),
        (
            "VisibleGroupNumberPerPage",
            row.num("visible_groups_per_page").to_string(),
        ),
    ])
}

/// Summarize a `0x7e` summary/running-total definition: the aggregate operation, its operation
/// parameter (the N of an NthLargest / a percentile), the summarized field reference, and the
/// `IsPercentageSummary` flag. The record opens with a nested `NamedValue`, so its own fields begin
/// past a child. `None` when the field reference is absent (an incomplete record).
fn summary(row: &Row) -> Option<RecordSummary> {
    let operation = SummaryOperation::from_code(row.num("operation") as i32);
    let field = row.get("operand").and_then(Cell::text)?;
    let mut fields = vec![
        ("Operation", format!("{operation:?}")),
        (
            "OperationParameter",
            row.num("operation_parameter").to_string(),
        ),
        ("SummarizedField", field.to_owned()),
    ];
    if row.num("is_percentage") != 0 {
        fields.push(("IsPercentageSummary", "true".to_string()));
    }
    Some(RecordSummary::new(fields))
}

/// Summarize a page-setup DEVMODE (`0x07`): the four printer members the SDK exposes. Members the
/// record's `dmFields` mask says are absent are omitted rather than shown as a default.
fn devmode(run: &[u8]) -> RecordSummary {
    let dm = decode_devmode(run);
    let mut fields = Vec::new();
    if let Some(o) = dm.orientation {
        fields.push(("PaperOrientation", format!("{o:?}")));
    }
    if let Some(s) = dm.paper_size {
        fields.push(("PaperSize", format!("{s:?}")));
    }
    if let Some(s) = dm.source {
        fields.push(("PaperSource", format!("{s:?}")));
    }
    if let Some(d) = dm.duplex {
        fields.push(("PrinterDuplex", format!("{d:?}")));
    }
    RecordSummary::new(fields)
}

/// Summarize an OLE object item (`0xbd`): the 1-based `Embedding N` storage ordinal its picture's
/// bytes live in (bytes `[0..4]`, big-endian). The image's format and pixel dimensions are NOT here —
/// they are derived from the embedded bytes themselves, which live in that storage, not in this
/// record.
fn ole_item(run: &[u8]) -> Option<RecordSummary> {
    let ordinal = crate::bytes::u32_be(run, 0)?;
    Some(RecordSummary::new(vec![(
        "EmbeddingOrdinal",
        ordinal.to_string(),
    )]))
}

/// Summarize a field-object opener (`0x9f`) that carries a **special** field, as its canonical kind
/// name. Both halves come from the object's reference: the pool it names says the field is special,
/// and the low half of its index is the special type. `None` for any other pool — gating on it is
/// required, since the type code alone maps a plain `0` to a real special type and would label every
/// ordinary field object.
fn special_field(row: &Row) -> Option<RecordSummary> {
    let (pool, index) = row.get("data_source").and_then(Cell::handle)?;
    if crate::model::FieldRefKind::from_code(pool as u8) != crate::model::FieldRefKind::Special {
        return None;
    }
    let code = index.unwrap_or(crate::field_table::table::UNSET_FIELD_INDEX) as u8;
    let name = special_field_name(code)?;
    Some(RecordSummary::new(vec![(
        "SpecialFieldType",
        name.to_string(),
    )]))
}

/// Summarize a cross-tab custom-group-members opener (`0x017e`): the member count it brackets.
fn custom_members(row: &Row) -> RecordSummary {
    RecordSummary::new(vec![("MemberCount", row.u("member_count").to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::Unknown;

    /// A record whose content is one run of field bytes and no nested record.
    fn unknown(rtype: u16, run: Vec<u8>) -> Node {
        Node::Unknown(Unknown {
            rtype,
            schema: 0x0700,
            parts: vec![Part::Run(run)],
        })
    }

    /// A record whose content is a run, a nested record of type `child`, then a second run. An
    /// empty run contributes no part, as it does not on the wire.
    fn unknown_split(rtype: u16, head: Vec<u8>, child: u16, tail: Vec<u8>) -> Node {
        let mut parts = Vec::new();
        if !head.is_empty() {
            parts.push(Part::Run(head));
        }
        parts.push(Part::Child {
            framed_len: 6,
            node: Node::Unknown(Unknown {
                rtype: child,
                schema: 0x0700,
                parts: Vec::new(),
            }),
        });
        if !tail.is_empty() {
            parts.push(Part::Run(tail));
        }
        Node::Unknown(Unknown {
            rtype,
            schema: 0x0700,
            parts,
        })
    }

    #[test]
    fn field_def_node_has_no_summary() {
        assert_eq!(
            summarize(&Node::FieldDef(Box::default()), Dialect::Contents),
            None
        );
    }

    #[test]
    fn unrecognized_record_has_no_summary() {
        assert_eq!(
            summarize(&unknown(0x0064, vec![1, 2, 3, 4]), Dialect::Contents),
            None
        );
    }

    /// A `0x0088` from a report authored with the group limit set to 2. Its content is four
    /// scalars, a nested `0x0151`, then six more — and `VisibleGroupNumberPerPage` is in the
    /// second run, so it is only reachable by declaring the child.
    fn group_area_node() -> Node {
        unknown_split(
            GROUP_AREA_FORMAT,
            vec![0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            0x0151,
            vec![
                0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xff, 0xff,
            ],
        )
    }

    #[test]
    fn group_area_summary_reads_past_the_nested_record() {
        let s = summarize(&group_area_node(), Dialect::Contents).unwrap();
        assert_eq!(
            s.one_line(),
            "RepeatGroupHeader=true KeepGroupTogether=false VisibleGroupNumberPerPage=2"
        );
    }

    /// The defect this decoder exists to avoid: with the two runs joined into one buffer, the
    /// nested record is gone from the content and the field table stops at the child it declares —
    /// so the field past it is never read.
    #[test]
    fn the_same_bytes_joined_into_one_run_lose_the_field() {
        let Node::Unknown(u) = group_area_node() else {
            unreachable!()
        };
        let joined: Vec<u8> = u.runs().flatten().copied().collect();
        let s = summarize(&unknown(GROUP_AREA_FORMAT, joined), Dialect::Contents).unwrap();
        assert_eq!(
            s.one_line(),
            "RepeatGroupHeader=true KeepGroupTogether=false VisibleGroupNumberPerPage=0"
        );
    }

    #[test]
    fn summary_def_decodes_operation_and_field() {
        // The nested NamedValue comes first, then: operation byte 0 = 0 (Sum), a separator,
        // the parameter u16-BE, and the length-prefixed field reference.
        let mut tail = vec![0x00, 0x00, 0x00, 0x00];
        tail.extend_from_slice(&4u32.to_be_bytes()); // len = 3 chars + NUL
        tail.extend_from_slice(b"t.f\0");
        let s = summarize(
            &unknown_split(SUMMARY_FIELD_DEFINITION, Vec::new(), 0x0071, tail),
            Dialect::Contents,
        )
        .unwrap();
        let line = s.one_line();
        assert!(line.contains("Operation=Sum"), "{line}");
        assert!(line.contains("SummarizedField=t.f"), "{line}");
    }

    #[test]
    fn short_record_does_not_panic() {
        for rtype in [
            NUMERIC_FIELD_FORMAT,
            STRING_FIELD_FORMAT,
            DATE_FIELD_FORMAT,
            TIME_FIELD_FORMAT,
            DATE_TIME_FIELD_FORMAT,
            BOOLEAN_FIELD_FORMAT,
            COMMON_FIELD_FORMAT,
            GROUP_AREA_FORMAT,
            SUMMARY_FIELD_DEFINITION,
            PAGE_DEVMODE,
            OLE_OBJECT_ITEM,
            FIELD_OBJECT,
            CROSSTAB_CUSTOM_MEMBERS,
        ] {
            // An empty record must not panic (the length-driven decoders return None; the rest
            // decode defaults).
            let _ = summarize(
                &Node::Unknown(Unknown {
                    rtype,
                    schema: 0x0700,
                    parts: Vec::new(),
                }),
                Dialect::Contents,
            );
        }
    }

    /// Every decoder here reads a report-definition record, so a record read in another vocabulary
    /// is not summarized at all — `0x0007` is the page-setup DEVMODE in the report definition and a
    /// command parameter in the query-engine session, and the same bytes decode as both.
    #[test]
    fn a_record_of_another_vocabulary_is_not_summarized() {
        let devmode = unknown(
            PAGE_DEVMODE,
            vec![0x00, 0x00, 0x12, 0x00, 0x00, 0x02, 0x00, 0x09],
        );
        assert!(summarize(&devmode, Dialect::Contents).is_some());
        for dialect in [
            Dialect::QeSession,
            Dialect::Catalog,
            Dialect::ReportParameters,
        ] {
            assert_eq!(summarize(&devmode, dialect), None, "{dialect:?}");
        }
    }

    /// A DEVMODE run reports only the members its `dmFields` mask marks present: orientation and
    /// paper size always occupy the fixed header, and each further set bit consumes one big-endian
    /// `u16` in struct order — so paper source is only readable once the bits before it are counted.
    #[test]
    fn devmode_reports_present_members_only() {
        // dmFields = DM_DEFAULTSOURCE | DM_DUPLEX; landscape (2), A4 (9), then source 7, duplex 2.
        let run = vec![
            0x00, 0x00, // sub-type
            0x12, 0x00, // dmFields low word: 0x1200
            0x00, 0x02, // dmOrientation = Landscape
            0x00, 0x09, // dmPaperSize = A4
            0x00, 0x07, // dmDefaultSource
            0x00, 0x02, // dmDuplex
        ];
        let line = summarize(&unknown(PAGE_DEVMODE, run), Dialect::Contents)
            .unwrap()
            .one_line();
        assert!(line.contains("PaperOrientation=Landscape"), "{line}");
        assert!(line.contains("PaperSize=PaperA4"), "{line}");
        assert!(line.contains("PaperSource="), "{line}");
        assert!(line.contains("PrinterDuplex="), "{line}");

        // With no tail bits set, neither tail member is reported at all.
        let bare = vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x09];
        let line = summarize(&unknown(PAGE_DEVMODE, bare), Dialect::Contents)
            .unwrap()
            .one_line();
        assert!(!line.contains("PaperSource"), "{line}");
        assert!(!line.contains("PrinterDuplex"), "{line}");
    }

    /// An OLE object item reports the `Embedding N` ordinal its picture bytes live under.
    #[test]
    fn ole_item_reports_embedding_ordinal() {
        let run = vec![0x00, 0x00, 0x00, 0x03];
        let line = summarize(&unknown(OLE_OBJECT_ITEM, run), Dialect::Contents)
            .unwrap()
            .one_line();
        assert_eq!(line, "EmbeddingOrdinal=3");
    }

    /// A field-object opener is summarized ONLY when its reference names the special-field pool.
    /// Gating on the pool matters: the special type code `0` is a real type, so reading the code
    /// unconditionally would label every ordinary field object.
    #[test]
    fn special_field_summarized_only_for_special_kind() {
        // The reference is a length-prefixed name, the pool it lives in, and the index within it;
        // a field object nests its ObjectName ahead of all three.
        let opener = |pool: u8, code: u8| {
            let mut run = vec![0x00, 0x00, 0x00, 0x04];
            run.extend_from_slice(b"t.f\0");
            run.extend_from_slice(&[pool, 0x00, code]);
            unknown_split(FIELD_OBJECT, Vec::new(), OBJECT_NAME, run)
        };
        // A database-field opener (pool 0) is not summarized, whatever index follows.
        assert!(summarize(&opener(0, 0), Dialect::Contents).is_none());

        // Find the pool code that means Special and check it does get summarized.
        let special = (0u8..=32)
            .find(|c| {
                crate::model::FieldRefKind::from_code(*c) == crate::model::FieldRefKind::Special
            })
            .expect("a Special kind code exists");
        let s = summarize(&opener(special, 0), Dialect::Contents);
        assert!(s.is_some_and(|s| s.one_line().starts_with("SpecialFieldType=")));

        // Without the nested ObjectName the record is not a field object's content at all, and the
        // reference is not read from the bytes that happen to sit where it would be.
        let mut run = vec![0x00, 0x00, 0x00, 0x04];
        run.extend_from_slice(b"t.f\0");
        run.extend_from_slice(&[special, 0x00, 0x00]);
        assert!(summarize(&unknown(FIELD_OBJECT, run), Dialect::Contents).is_none());
    }

    /// The cross-tab custom-members opener reports the member count it brackets.
    #[test]
    fn custom_members_reports_count() {
        let run = vec![0x00, 0x00, 0x00, 0x00];
        let line = summarize(&unknown(CROSSTAB_CUSTOM_MEMBERS, run), Dialect::Contents)
            .unwrap()
            .one_line();
        assert_eq!(line, "MemberCount=0");
    }
}
