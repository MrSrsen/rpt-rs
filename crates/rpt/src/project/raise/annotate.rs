//! Per-record decoded summaries for inspection tooling.
//!
//! Given a single DOM [`Node`], [`summarize`] recognizes the record types the raise layer decodes
//! into semantic-model fields and returns a concise, human-readable [`RecordSummary`] of the decoded
//! values — the same values the model carries, surfaced against the raw record so a tree/inspection
//! view can show *what a record means* rather than only its bytes.
//!
//! The DOM keeps a record's own content as split [`Value`]s (length-prefixed strings + verbatim
//! byte runs); [`leaf_bytes`] losslessly reassembles the demasked leaf from them, so the existing
//! `decode_*` raise decoders (which read the same leaf bytes) can be reused directly rather than
//! duplicating the byte layout. A record with no decoder — or a leaf too short to decode — yields
//! `None`, and the caller keeps its raw preview.

use crate::model::SummaryOperation;
use crate::records::rtype::*;
use crate::records::{Node, Value};

use super::data_def::decode_group_area_format;
use super::print_options::decode_devmode;
use super::report_def::{
    decode_boolean_format, decode_common_format, decode_date_format, decode_datetime_format,
    decode_numeric_format, decode_string_format, decode_time_format,
};

/// A decoded, human-readable summary of one record's stored values: ordered `(key, value)` pairs,
/// each value rendered concisely (an enum by name, a number, a flag). Ordered so the display and
/// any structured (JSON) projection are stable.
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

/// Losslessly reassemble a DOM node's demasked leaf bytes from its split [`Value`]s — the inverse of
/// the projection that produced them. A [`Value::Text`] was a length-prefixed printable string
/// (`u32`-BE length = text bytes + trailing NUL); a [`Value::Bytes`] run is kept verbatim. Only the
/// node's *own* content is reassembled (child records are separate nodes), matching what the raise
/// `decode_*` helpers read.
fn leaf_bytes(values: &[Value]) -> Vec<u8> {
    let mut out = Vec::new();
    for v in values {
        match v {
            Value::Text(s) => {
                let len = s.len() as u32 + 1; // text bytes + one NUL terminator
                out.extend_from_slice(&len.to_be_bytes());
                out.extend_from_slice(s.as_bytes());
                out.push(0);
            }
            Value::Int(i) => out.extend_from_slice(&i.to_le_bytes()),
            Value::Bytes(b) => out.extend_from_slice(b),
        }
    }
    out
}

/// A concise decoded summary of a record, or `None` when the record type has no decoder (or its
/// leaf is too short to decode). Never panics: every decoder tolerates a short/malformed leaf.
pub fn summarize(node: &Node) -> Option<RecordSummary> {
    // Field definitions are already surfaced by the DOM (name + value type); leave their preview
    // to the caller and summarize only the unmodelled records the raise layer can decode.
    let Node::Unknown(u) = node else {
        return None;
    };
    let leaf = leaf_bytes(&u.values);
    match u.rtype {
        FF_NUMERIC_VALUE => Some(numeric(&leaf)),
        FF_STRING_VALUE => Some(string(&leaf)),
        FF_DATE_VALUE => Some(date(&leaf)),
        FF_TIME_VALUE => Some(time(&leaf)),
        FF_DATETIME_VALUE => Some(datetime(&leaf)),
        FF_BOOLEAN_VALUE => Some(boolean(&leaf)),
        FF_COMMON_VALUE => Some(common(&leaf)),
        GROUP_OPTIONS => Some(group_area(&leaf)),
        SUMMARY_DEF => summary(&leaf),
        PAPER_DEVMODE => Some(devmode(&leaf)),
        OLE_OBJECT_ITEM => ole_item(&leaf),
        FIELD_OBJECT => special_field(&leaf),
        CROSSTAB_CUSTOM_MEMBERS_BEGIN => custom_members(&leaf),
        _ => None,
    }
}

fn numeric(leaf: &[u8]) -> RecordSummary {
    let f = decode_numeric_format(leaf);
    RecordSummary::new(vec![
        ("DecimalPlaces", f.decimal_places.to_string()),
        ("Negative", format!("{:?}", f.negative)),
        ("CurrencySymbol", format!("{:?}", f.currency_symbol)),
        ("CurrencyPosition", format!("{:?}", f.currency_position)),
        ("Rounding", format!("{:?}", f.rounding)),
        ("ThousandsSeparator", f.thousands_separator.to_string()),
    ])
}

fn string(leaf: &[u8]) -> RecordSummary {
    let f = decode_string_format(leaf);
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

fn date(leaf: &[u8]) -> RecordSummary {
    let f = decode_date_format(leaf);
    RecordSummary::new(vec![
        ("DateOrder", format!("{:?}", f.date_order)),
        ("Year", format!("{:?}", f.year)),
        ("Month", format!("{:?}", f.month)),
        ("Day", format!("{:?}", f.day)),
    ])
}

fn time(leaf: &[u8]) -> RecordSummary {
    let f = decode_time_format(leaf);
    RecordSummary::new(vec![
        ("Hour", format!("{:?}", f.hour)),
        ("Minute", format!("{:?}", f.minute)),
        ("Second", format!("{:?}", f.second)),
    ])
}

fn datetime(leaf: &[u8]) -> RecordSummary {
    let f = decode_datetime_format(leaf);
    RecordSummary::new(vec![
        ("Order", format!("{:?}", f.order)),
        ("Separator", format!("{:?}", f.separator)),
    ])
}

fn boolean(leaf: &[u8]) -> RecordSummary {
    let f = decode_boolean_format(leaf);
    RecordSummary::new(vec![("OutputType", format!("{:?}", f.output_type))])
}

fn common(leaf: &[u8]) -> RecordSummary {
    let f = decode_common_format(leaf);
    RecordSummary::new(vec![
        ("SuppressIfDuplicated", f.suppress_if_duplicated.to_string()),
        ("UseSystemDefaults", f.use_system_defaults.to_string()),
    ])
}

fn group_area(leaf: &[u8]) -> RecordSummary {
    let f = decode_group_area_format(leaf);
    RecordSummary::new(vec![
        ("RepeatGroupHeader", f.repeat_group_header.to_string()),
        ("KeepGroupTogether", f.keep_group_together.to_string()),
        (
            "VisibleGroupNumberPerPage",
            f.visible_groups_per_page.to_string(),
        ),
    ])
}

/// Summarize a `0x7e` summary/running-total definition: the aggregate operation (leaf byte 0), its
/// operation parameter (`u16`-BE at byte 2 — the N of an NthLargest / a percentile), the summarized
/// field reference (length-prefixed at byte 4), and the `IsPercentageSummary` flag (byte 12 past the
/// field reference). Mirrors the leaf layout the summary raise reads. `None` when the field
/// reference is absent (an incomplete leaf).
fn summary(leaf: &[u8]) -> Option<RecordSummary> {
    use crate::bytes::{read_lp_string, u16_be};
    let operation = SummaryOperation::from_code(i32::from(leaf.first().copied().unwrap_or(0)));
    let operation_parameter = u16_be(leaf, 2).map_or(0, i32::from);
    let (field, consumed) = leaf.get(4..).and_then(read_lp_string)?;
    let is_percentage = leaf.get(4 + consumed + 12).is_some_and(|&b| b != 0);
    let mut fields = vec![
        ("Operation", format!("{operation:?}")),
        ("OperationParameter", operation_parameter.to_string()),
        ("SummarizedField", field),
    ];
    if is_percentage {
        fields.push(("IsPercentageSummary", "true".to_string()));
    }
    Some(RecordSummary::new(fields))
}

/// Summarize a page-setup DEVMODE (`0x07`): the four printer members the SDK exposes. Members the
/// leaf's `dmFields` mask says are absent are omitted rather than shown as a default.
fn devmode(leaf: &[u8]) -> RecordSummary {
    let dm = decode_devmode(leaf);
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
/// bytes live in (leaf `[0..4]`, big-endian). The image's format and pixel dimensions are NOT here —
/// they are derived from the embedded bytes themselves, which live in that storage, not in this
/// record.
fn ole_item(leaf: &[u8]) -> Option<RecordSummary> {
    let ordinal = crate::bytes::u32_be(leaf, 0)?;
    Some(RecordSummary::new(vec![(
        "EmbeddingOrdinal",
        ordinal.to_string(),
    )]))
}

/// Summarize a field-object opener (`0x9f`) that carries a **special** field, as its canonical kind
/// name. The opener is `[u32 length][NUL-terminated reference][kind byte]…`: the kind byte sits at
/// `p = 4 + leaf[3]` and the special type code two bytes past it, mirroring what the object raise
/// reads. `None` for any other field kind — gating on the kind byte is required, since the type code
/// alone maps a plain `0` to a real special type and would label every ordinary field object.
fn special_field(leaf: &[u8]) -> Option<RecordSummary> {
    let p = 4 + *leaf.get(3)? as usize;
    if crate::model::FieldRefKind::from_code(*leaf.get(p)?) != crate::model::FieldRefKind::Special {
        return None;
    }
    let name = super::report_def::data_source::special_field_name(*leaf.get(p + 2)?)?;
    Some(RecordSummary::new(vec![(
        "SpecialFieldType",
        name.to_string(),
    )]))
}

/// Summarize a cross-tab custom-group-members opener (`0x017e`): the `u32` member count it brackets.
fn custom_members(leaf: &[u8]) -> Option<RecordSummary> {
    let count = crate::bytes::u32_be(leaf, 0)?;
    Some(RecordSummary::new(vec![("MemberCount", count.to_string())]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::Unknown;

    fn unknown(rtype: u16, values: Vec<Value>) -> Node {
        Node::Unknown(Unknown {
            rtype,
            values,
            ..Default::default()
        })
    }

    /// A `Value::Text` reassembles to exactly the length-prefixed NUL-terminated span the DOM split
    /// it from, so a `decode_*` helper reading the reassembled leaf sees the original bytes.
    #[test]
    fn text_value_round_trips_to_length_prefixed_span() {
        let leaf = leaf_bytes(&[Value::Text("ab".into())]);
        // u32-BE length = 3 (2 text bytes + NUL), then "ab", then NUL.
        assert_eq!(leaf, vec![0, 0, 0, 3, b'a', b'b', 0]);
    }

    /// Mixed byte runs and text reassemble in order, verbatim.
    #[test]
    fn mixed_values_reassemble_in_order() {
        let leaf = leaf_bytes(&[
            Value::Bytes(vec![0x01, 0x02]),
            Value::Text("x".into()),
            Value::Bytes(vec![0xff]),
        ]);
        assert_eq!(leaf, vec![0x01, 0x02, 0, 0, 0, 2, b'x', 0, 0xff]);
    }

    #[test]
    fn field_def_node_has_no_summary() {
        assert_eq!(summarize(&Node::FieldDef(Box::default())), None);
    }

    #[test]
    fn unrecognized_record_has_no_summary() {
        assert_eq!(summarize(&unknown(0x0064, vec![Value::Int(1)])), None);
    }

    #[test]
    fn group_area_summary_decodes_scalars() {
        // RepeatGroupHeader (u16-BE @0) = 1, KeepGroupTogether (@2) = 0,
        // VisibleGroupNumberPerPage (u16-BE @4) = 3; bytes [6..8] are a separate group property
        // (here 0x012c) that the count must not span.
        let leaf = vec![0x00, 0x01, 0x00, 0x00, 0x00, 0x03, 0x01, 0x2c];
        let s = summarize(&unknown(GROUP_OPTIONS, vec![Value::Bytes(leaf)])).unwrap();
        assert_eq!(
            s.one_line(),
            "RepeatGroupHeader=true KeepGroupTogether=false VisibleGroupNumberPerPage=3"
        );
    }

    #[test]
    fn summary_def_decodes_operation_and_field() {
        // op byte 0 = 0 (Sum), separator, param u16-BE @2 = 0, then LP field ref "t.f" @4.
        let mut leaf = vec![0x00, 0x00, 0x00, 0x00];
        leaf.extend_from_slice(&4u32.to_be_bytes()); // len = 3 chars + NUL
        leaf.extend_from_slice(b"t.f\0");
        let s = summarize(&unknown(SUMMARY_DEF, vec![Value::Bytes(leaf)])).unwrap();
        let line = s.one_line();
        assert!(line.contains("Operation=Sum"), "{line}");
        assert!(line.contains("SummarizedField=t.f"), "{line}");
    }

    #[test]
    fn short_leaf_does_not_panic() {
        for rtype in [
            FF_NUMERIC_VALUE,
            FF_STRING_VALUE,
            FF_DATE_VALUE,
            FF_TIME_VALUE,
            FF_DATETIME_VALUE,
            FF_BOOLEAN_VALUE,
            FF_COMMON_VALUE,
            GROUP_OPTIONS,
            SUMMARY_DEF,
            PAPER_DEVMODE,
            OLE_OBJECT_ITEM,
            FIELD_OBJECT,
            CROSSTAB_CUSTOM_MEMBERS_BEGIN,
        ] {
            // An empty leaf must not panic (the length-driven decoders return None; the rest
            // decode defaults).
            let _ = summarize(&unknown(rtype, vec![]));
        }
    }

    /// A DEVMODE leaf reports only the members its `dmFields` mask marks present: orientation and
    /// paper size always occupy the fixed header, and each further set bit consumes one big-endian
    /// `u16` in struct order — so paper source is only readable once the bits before it are counted.
    #[test]
    fn devmode_reports_present_members_only() {
        // dmFields = DM_DEFAULTSOURCE | DM_DUPLEX; landscape (2), A4 (9), then source 7, duplex 2.
        let leaf = vec![
            0x00, 0x00, // sub-type
            0x12, 0x00, // dmFields low word: 0x1200
            0x00, 0x02, // dmOrientation = Landscape
            0x00, 0x09, // dmPaperSize = A4
            0x00, 0x07, // dmDefaultSource
            0x00, 0x02, // dmDuplex
        ];
        let line = summarize(&unknown(PAPER_DEVMODE, vec![Value::Bytes(leaf)]))
            .unwrap()
            .one_line();
        assert!(line.contains("PaperOrientation=Landscape"), "{line}");
        assert!(line.contains("PaperSize=PaperA4"), "{line}");
        assert!(line.contains("PaperSource="), "{line}");
        assert!(line.contains("PrinterDuplex="), "{line}");

        // With no tail bits set, neither tail member is reported at all.
        let bare = vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x09];
        let line = summarize(&unknown(PAPER_DEVMODE, vec![Value::Bytes(bare)]))
            .unwrap()
            .one_line();
        assert!(!line.contains("PaperSource"), "{line}");
        assert!(!line.contains("PrinterDuplex"), "{line}");
    }

    /// An OLE object item reports the `Embedding N` ordinal its picture bytes live under.
    #[test]
    fn ole_item_reports_embedding_ordinal() {
        let leaf = vec![0x00, 0x00, 0x00, 0x03];
        let line = summarize(&unknown(OLE_OBJECT_ITEM, vec![Value::Bytes(leaf)]))
            .unwrap()
            .one_line();
        assert_eq!(line, "EmbeddingOrdinal=3");
    }

    /// A field-object opener is summarized ONLY when its kind byte says the field is special.
    /// Gating on the kind matters: the special type code `0` is a real type, so reading the code
    /// unconditionally would label every ordinary field object.
    #[test]
    fn special_field_summarized_only_for_special_kind() {
        // `[u32 length][ref NUL][kind byte][?][type code]`; length byte at [3] = 4 ("t.f\0").
        let opener = |kind: u8, code: u8| {
            let mut leaf = vec![0x00, 0x00, 0x00, 0x04];
            leaf.extend_from_slice(b"t.f\0");
            leaf.extend_from_slice(&[kind, 0x00, code]);
            leaf
        };
        // A database-field opener (kind 0) is not summarized, whatever the following byte holds.
        assert!(summarize(&unknown(FIELD_OBJECT, vec![Value::Bytes(opener(0, 0))])).is_none());

        // Find the kind code that means Special and check it does get summarized.
        let special = (0u8..=32)
            .find(|c| {
                crate::model::FieldRefKind::from_code(*c) == crate::model::FieldRefKind::Special
            })
            .expect("a Special kind code exists");
        let s = summarize(&unknown(
            FIELD_OBJECT,
            vec![Value::Bytes(opener(special, 0))],
        ));
        assert!(s.is_some_and(|s| s.one_line().starts_with("SpecialFieldType=")));
    }

    /// The cross-tab custom-members opener reports the member count it brackets.
    #[test]
    fn custom_members_reports_count() {
        let leaf = vec![0x00, 0x00, 0x00, 0x00];
        let line = summarize(&unknown(
            CROSSTAB_CUSTOM_MEMBERS_BEGIN,
            vec![Value::Bytes(leaf)],
        ))
        .unwrap()
        .one_line();
        assert_eq!(line, "MemberCount=0");
    }
}
