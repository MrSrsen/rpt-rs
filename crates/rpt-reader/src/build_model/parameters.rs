//! Parameter fields — PromptManager XML joined with the `0x007a` Contents detail records.

use super::row_of;
use crate::codec::{Dialect, RecordNode};
use crate::field_table::table::{Cell, Row};
use crate::field_table::tables as ft;
use crate::model::{
    FieldDef, FieldKindData, FieldValueType, ParameterField, ParameterValue, ParameterValueKind,
};
use crate::records::RecordStream;
use std::collections::BTreeMap;

/// The factor the engine scales a stored `Number`/`Currency` value by before writing it, so reading
/// one back means dividing. The saved-data batches store their numeric cells at the same scale.
const STORED_NUMERIC_SCALE: f64 = 100.0;

/// The parameter type a stored-procedure parameter states; a report parameter states another.
const STORED_PROCEDURE_PARAMETER: u32 = 2;

/// The SDK `FieldValueType` codes a parameter states its stored values under. The ordinals are the
/// public enum's own and are contiguous, so a code this does not name is one for a type no
/// parameter carries.
mod sdk_value_type {
    pub(super) const NUMBER: u32 = 6;
    pub(super) const CURRENCY: u32 = 7;
    pub(super) const BOOLEAN: u32 = 8;
    pub(super) const DATE: u32 = 9;
    pub(super) const TIME: u32 = 10;
    pub(super) const STRING: u32 = 11;
    pub(super) const DATE_TIME: u32 = 15;
}

/// The decoded detail of one parameter, from its `0x007a` Contents record. The PromptText, the
/// report-vs-stored-procedure kind, and the optional-prompt flag are not in the PromptManager XML
/// (which carries only name, value type and panel visibility) — they live in this record.
pub(super) struct ParamRecord {
    /// The `crobj://{…}` identity that joins this record to its PromptManager entry, or `None` for
    /// a record that carries none — a parameter used only in a formula, with no PromptManager
    /// entry. Such records are synthesized directly into a `ParameterField` by
    /// [`build_orphan_param`] rather than joined.
    pub(super) guid: Option<String>,
    pub(super) prompt_text: String,
    pub(super) is_sp_param: bool,
    pub(super) is_optional: bool,
    pub(super) allow_multiple: bool,
    pub(super) allow_custom_values: bool,
    pub(super) allow_editing_default: bool,
    /// The engine's global parameter index. The saved current-value records in the top-level
    /// `ReportParametersStream` (`0x0031`) reference a parameter by this same index — it is the join
    /// key between a parameter and its saved last-used value.
    pub(super) index: u16,
    /// The parameter's default pick-list (`<ParameterDefaultValues>`): the allowed values plus their
    /// descriptions. Empty for a plain parameter with no pick list.
    pub(super) default_values: Vec<ParameterValue>,
    /// SDK `@DefaultValueDisplayType`.
    pub(super) display_type: crate::model::ParameterDisplayType,
    /// SDK `@DefaultValueSortOrder`.
    pub(super) sort_order: crate::model::ParameterSortOrder,
    /// The raw Crystal value-type code (SDK `FieldValueType`) the record states its stored values
    /// under. Used only to resolve the [`ParameterValueKind`] for a record with no PromptManager
    /// entry, which has no `ValueType` element to read.
    pub(super) value_type_code: u32,
}

/// Decode a parameter detail record (`0x007a`). See
/// [`PARAMETER_RECORD`](crate::field_table::tables::PARAMETER_RECORD) for the record's layout.
pub(super) fn parse_param_record(node: &RecordNode, logical: &[u8]) -> ParamRecord {
    use crate::model::{ParameterDisplayType, ParameterSortOrder};
    let row = row_of(node, logical, &ft::PARAMETER_RECORD);
    // A parameter whose values come from a list allows neither a custom current value nor editing
    // of the stored default; a parameter typed into by hand allows both.
    let dynamic = row.u("dynamic_values") != 0;
    let id = row.text("id");
    ParamRecord {
        guid: (!id.is_empty()).then(|| id.to_owned()),
        prompt_text: row.text("prompt_text").to_owned(),
        is_sp_param: row.u("parameter_type") == STORED_PROCEDURE_PARAMETER,
        is_optional: row.u("optional_prompt") != 0,
        allow_multiple: row.u("allow_multiple_values") != 0,
        allow_custom_values: !dynamic,
        allow_editing_default: !dynamic,
        index: row.u("parameter_index") as u16,
        default_values: default_value_list(&row),
        display_type: match row.u("default_value_display_type") {
            1 => ParameterDisplayType::Description,
            _ => ParameterDisplayType::DescriptionAndValue,
        },
        sort_order: match row.u("default_value_sort_order") {
            1 => ParameterSortOrder::AlphabeticalAscending,
            _ => ParameterSortOrder::NoSort,
        },
        value_type_code: row.u("value_type"),
    }
}

/// The [`ParameterValueKind`] for a parameter with no PromptManager entry, resolved from the
/// [`sdk_value_type`] its record states. Unknown codes fall back to a string parameter.
fn value_kind_from_code(code: u32) -> ParameterValueKind {
    match code {
        sdk_value_type::NUMBER => ParameterValueKind::NumberParameter,
        sdk_value_type::CURRENCY => ParameterValueKind::CurrencyParameter,
        sdk_value_type::BOOLEAN => ParameterValueKind::BooleanParameter,
        sdk_value_type::DATE => ParameterValueKind::DateParameter,
        sdk_value_type::TIME => ParameterValueKind::TimeParameter,
        sdk_value_type::STRING => ParameterValueKind::StringParameter,
        sdk_value_type::DATE_TIME => ParameterValueKind::DateTimeParameter,
        _ => ParameterValueKind::StringParameter,
    }
}

/// Synthesize a `ParameterField` from a `0x007a` record with no identity (a parameter referenced
/// only by a formula, absent from the PromptManager). Its Name and PromptText are the record's
/// prompt text; the value kind comes from the value type the record states; the flag attributes are
/// the ones already decoded from the record. Initial/current value lists are empty (there is no
/// PromptManager entry or `ReportParametersStream` join). Returns `None` if the record carries no
/// usable name.
pub(super) fn build_orphan_param(rec: &ParamRecord) -> Option<FieldDef> {
    let name = rec.prompt_text.clone();
    if name.is_empty() {
        return None;
    }
    let value_kind = value_kind_from_code(rec.value_type_code);
    Some(FieldDef {
        kind: FieldKindData::Parameter(Box::new(ParameterField {
            value_kind,
            parameter_type: crate::model::ParameterType::ReportParameter,
            prompt_text: Some(name.clone()),
            show_on_panel: false,
            editable_on_panel: false,
            optional_prompt: rec.is_optional,
            has_current_value: false,
            allow_multiple_values: rec.allow_multiple,
            allow_custom_values: rec.allow_custom_values,
            allow_editing_default_value: rec.allow_editing_default,
            default_values: rec.default_values.clone(),
            initial_values: Vec::new(),
            current_values: Vec::new(),
            default_value_display_type: rec.display_type,
            default_value_sort_order: rec.sort_order,
            ..Default::default()
        })),
        value_type: param_value_type(value_kind),
        name,
        ..Default::default()
    })
}

/// The parameter's stored pick-list: each value paired with the description stored beside it. A
/// value the record states in a form with no text here yields no list at all, rather than a guess at
/// one.
fn default_value_list(row: &Row) -> Vec<ParameterValue> {
    let value_type = row.u("value_type");
    let descriptions = row.seq("descriptions");
    row.seq("values")
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            Some(ParameterValue {
                value: value_text(entry, value_type)?,
                description: Some(
                    descriptions
                        .get(i)
                        .map(|d| d.text("text").to_owned())
                        .unwrap_or_default(),
                ),
                range: None,
            })
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default()
}

/// Write a Number/Currency parameter value in the shortest form that reads back as the same double.
/// No grouping, no fixed decimal count and a `.` decimal point: the value is the stored double, and
/// how many digits a reader shows it with is that reader's decision, not this one's.
fn number_literal(v: f64) -> String {
    format!("{v}")
}

/// Write a date serial as the literal the file itself uses for a stored date parameter value,
/// `Date(YYYY,MM,DD)` — the same form the `PromptManager` document states its dates in, so one
/// parameter's values read alike whichever of the two records stated them.
fn date_literal(serial: u32) -> String {
    // The serial is one day past the astronomical Julian Day Number at this epoch.
    let j = serial as i64 + 1;
    let a = j + 32044;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = 100 * b + d - 4800 + m / 10;
    format!("Date({year},{month:02},{day:02})")
}

/// One stored value, written in the literal form its value type gives it. `None` for a value type
/// with no literal form here, rather than a guess at one.
///
/// Each form states the stored value and nothing else: no locale decides a separator, an order or a
/// digit count here, because the decoder holds no locale and a value it rendered for one could not
/// be read back and rendered for another.
fn value_text(entry: &Row, value_type: u32) -> Option<String> {
    match value_type {
        sdk_value_type::NUMBER | sdk_value_type::CURRENCY => Some(number_literal(
            entry.get("number").and_then(Cell::f)? / STORED_NUMERIC_SCALE,
        )),
        // Date: a day serial.
        sdk_value_type::DATE => Some(date_literal(entry.i("int32") as u32)),
        sdk_value_type::STRING => Some(entry.text("text").to_owned()),
        _ => None,
    }
}

/// A value's description: the first of the string list stored beside it. A value with no list has
/// none.
fn current_value_description(strings: Option<&Row>) -> String {
    strings
        .and_then(|s| s.seq("strings").first())
        .map(|s| s.text("text").to_owned())
        .unwrap_or_default()
}

/// Build the map of saved current parameter values keyed by the engine's global parameter index,
/// decoded from the top-level `ReportParametersStream`.
///
/// The values are read in that stream's own vocabulary and no other: `0x0031` names a saved current
/// value there and an unrelated record in the report definition, so the tree is taken only where the
/// stream is written in it. Any other stream carries no saved current values, whatever its records
/// are numbered.
///
/// A `0x0031` record states its values twice: as typed payloads, and as a prompting-form
/// `<CRMetaObjects>` document. The two describe the same values, and which one carries the whole
/// answer depends on the shape: a range's payloads are its two bounds and say nothing about whether
/// each end is included, so a document that describes a range wins; otherwise the payloads do,
/// being the typed form.
pub(crate) fn parse_report_parameters(stream: &RecordStream) -> BTreeMap<u16, Vec<ParameterValue>> {
    let mut out: BTreeMap<u16, Vec<ParameterValue>> = BTreeMap::new();
    let logical = stream.logical_bytes();
    for root in stream.record_tree_in(Dialect::ReportParameters) {
        root.walk(&mut |n| {
            if n.rtype != ft::CURRENT_VALUE_RECORD.rtype {
                return;
            }
            let row = row_of(n, logical, &ft::CURRENT_VALUE_RECORD);
            let idx = row.u("parameter_index") as u16;
            let value_type = row.u("value_type");
            let prompting = match row.get("prompting_info") {
                Some(Cell::Bytes(b)) => String::from_utf8_lossy(b).into_owned(),
                _ => String::new(),
            };
            let ranges = parse_range_values(&prompting);
            if !ranges.is_empty() {
                out.insert(idx, ranges);
                return;
            }
            let strings = row.seq("value_strings");
            let values: Option<Vec<ParameterValue>> = row
                .seq("values")
                .iter()
                .enumerate()
                .map(|(i, entry)| {
                    Some(ParameterValue {
                        value: value_text(entry, value_type)?,
                        description: Some(current_value_description(strings.get(i))),
                        range: None,
                    })
                })
                .collect();
            match values {
                Some(values) if !values.is_empty() => {
                    out.insert(idx, values);
                }
                // A value the record states in no form this reads: the record's presence alone still
                // means the parameter has a saved current value.
                _ => {
                    out.entry(idx).or_default();
                }
            }
        });
    }
    out
}

/// Parse the `<Value xsi:type="DiscreteValue">` entries out of a `<Values>` / `<DefaultValues>`
/// CRMetaObjects fragment. Each yields a [`ParameterValue`] from the inner `<Value>` text;
/// `with_description` controls whether the `<Description>` is captured (current values carry one,
/// initial values do not).
fn parse_discrete_values(xml: &str, with_description: bool) -> Vec<ParameterValue> {
    let mut out = Vec::new();
    for chunk in xml.split("<Value xsi:type=\"DiscreteValue\"").skip(1) {
        // The first <Value>/<Description> in the chunk belong to this discrete value (they precede
        // the next "<Value xsi:type=\"DiscreteValue\"" split boundary).
        let Some(value) = xml_tag(chunk, "Value") else {
            continue;
        };
        let description = if with_description {
            Some(xml_tag(chunk, "Description").unwrap_or_default())
        } else {
            None
        };
        out.push(ParameterValue {
            value,
            description,
            range: None,
        });
    }
    out
}

/// Parse the `<Value xsi:type="RangeValue">` entries out of a saved-current-value record's embedded
/// CRMetaObjects fragment. Each range value carries `LBound`/`UBound` (the lower/upper bound values,
/// already in human form) and `LBoundType`/`UBoundType` (`Inclusive`/`Exclusive`/`Unbounded`). An
/// absent `UBound` (open upper end) yields an empty `end_value`.
fn parse_range_values(xml: &str) -> Vec<ParameterValue> {
    let mut out = Vec::new();
    for chunk in xml.split("<Value xsi:type=\"RangeValue\"").skip(1) {
        out.push(ParameterValue {
            value: xml_tag(chunk, "LBound").unwrap_or_default(),
            description: None,
            range: Some(crate::model::ParameterRange {
                end_value: xml_tag(chunk, "UBound").unwrap_or_default(),
                lower_bound: range_bound_type(xml_tag(chunk, "LBoundType").as_deref()),
                upper_bound: range_bound_type(xml_tag(chunk, "UBoundType").as_deref()),
            }),
        });
    }
    out
}

/// Map a CRMetaObjects range bound-type token to [`RangeBoundType`]. `Unbounded` (or absent) is an
/// open end (`NoBound`).
fn range_bound_type(token: Option<&str>) -> crate::model::RangeBoundType {
    use crate::model::RangeBoundType;
    match token {
        Some("Inclusive") => RangeBoundType::BoundInclusive,
        Some("Exclusive") => RangeBoundType::BoundExclusive,
        _ => RangeBoundType::NoBound,
    }
}

/// Extract parameter field definitions from the `PromptManager` CRMetaObjects XML, joined to their
/// `0x007a` detail records (`param_records`, keyed by GUID) for the PromptText, ParameterType,
/// optional-prompt flag and the DefaultValues pick list, plus the saved CurrentValues (`current_values`,
/// keyed by the engine parameter index). Panel visibility comes from the XML's `Int_ShowOnViewerPanel`;
/// PromptText is `None` when the parameter has no detail record to store one.
/// `HasCurrentValue` is set per parameter to whether it has a decoded saved current value.
pub(super) fn build_parameters(
    xml: &str,
    param_records: &BTreeMap<String, ParamRecord>,
    current_values: &BTreeMap<u16, Vec<ParameterValue>>,
) -> Vec<FieldDef> {
    let mut out = Vec::new();
    for meta in xml.split("<MetaObject").skip(1) {
        // Only parameter meta-objects; the inner `<Object xsi:type="Parameter">` holds name/type.
        let Some((_, obj)) = meta.split_once("<Object xsi:type=\"Parameter\"") else {
            continue;
        };
        let Some(name) = xml_tag(obj, "Name") else {
            continue;
        };
        let value_kind = match xml_tag(obj, "ValueType").as_deref() {
            Some("String") => ParameterValueKind::StringParameter,
            Some("Number") => ParameterValueKind::NumberParameter,
            Some("Currency") => ParameterValueKind::CurrencyParameter,
            Some("Boolean") => ParameterValueKind::BooleanParameter,
            Some("Date") => ParameterValueKind::DateParameter,
            Some("Time") => ParameterValueKind::TimeParameter,
            Some("DateTime") => ParameterValueKind::DateTimeParameter,
            _ => ParameterValueKind::default(),
        };
        // The parameter is shown on (and editable on) the viewer panel iff the flag is 1.
        let show_on_panel = meta
            .split_once("Int_ShowOnViewerPanel</Name><Value VariantType=\"Integer\">")
            .and_then(|(_, v)| v.trim_start().chars().next())
            == Some('1');
        // Prompt-group linkage: the group GUID plus the two group-membership property flags. A
        // cascading (parent->child) group shares one PromptGroupRef GUID across its ordered levels;
        // a standalone parameter carries its own auto-generated singleton group and PartOfGroup=0.
        let prompt_group = xml_tag(obj, "PromptGroupRef");
        let part_of_group = prop_flag(meta, "Boolean_PartOfGroup");
        let mutually_exclusive_group = prop_flag(meta, "Boolean_MutuallyExclusiveGroup");
        let guid = xml_tag(meta, "ID").unwrap_or_default();
        let rec = param_records.get(&guid);
        let parameter_type = match rec {
            Some(r) if r.is_sp_param => crate::model::ParameterType::StoreProcedureParameter,
            _ => crate::model::ParameterType::ReportParameter,
        };
        // The three value collections:
        //  - DefaultValues (pick list) from the `0x007a` detail record;
        //  - InitialValues from the PromptManager `<DefaultValues>` element (the stored default, with
        //    no Description);
        //  - CurrentValues (saved last-used) from the `ReportParametersStream`, joined by the param's
        //    engine index.
        let default_values = rec.map(|r| r.default_values.clone()).unwrap_or_default();
        let initial_values = parse_discrete_values(obj, false);
        let current = rec
            .and_then(|r| current_values.get(&r.index))
            .cloned()
            .unwrap_or_default();
        out.push(FieldDef {
            kind: FieldKindData::Parameter(Box::new(ParameterField {
                value_kind,
                parameter_type,
                // The stored PromptText, from the `0x007a` detail record. A parameter with no detail
                // record (an auto-generated command parameter) stores none; the engine synthesizes
                // one at prompt time, which is not a fact about the file.
                prompt_text: rec.map(|r| r.prompt_text.clone()),
                show_on_panel,
                editable_on_panel: show_on_panel,
                optional_prompt: rec.is_some_and(|r| r.is_optional),
                // Presence of a current-value record (discrete *or* range) sets HasCurrentValue;
                // `current` may be empty for a range whose discrete entries we can't recover.
                has_current_value: rec.and_then(|r| current_values.get(&r.index)).is_some(),
                allow_multiple_values: rec.is_some_and(|r| r.allow_multiple),
                // Default True when no detail record exists (e.g. auto-generated command params).
                allow_custom_values: rec.is_none_or(|r| r.allow_custom_values),
                allow_editing_default_value: rec.is_none_or(|r| r.allow_editing_default),
                default_values,
                initial_values,
                current_values: current,
                // DefaultValueDisplayType / DefaultValueSortOrder decoded from the `0x007a` record;
                // engine defaults (DescriptionAndValue / NoSort) when no detail record exists.
                default_value_display_type: rec.map(|r| r.display_type).unwrap_or_default(),
                default_value_sort_order: rec.map(|r| r.sort_order).unwrap_or_default(),
                prompt_group,
                part_of_group,
                mutually_exclusive_group,
                ..Default::default()
            })),
            value_type: param_value_type(value_kind),
            name,
            ..Default::default()
        });
    }
    out
}

/// Read a boolean-valued PromptManager `<Property>` flag by name: finds
/// `<Name>{name}</Name><Value …>X</Value>` and returns `true` iff `X` is a nonzero integer — the
/// VARIANT_BOOL semantics the engine writes with, where a true is `-1` and an integer flag is `1`.
/// A non-integer or absent property ⇒ `false`.
fn prop_flag(meta: &str, name: &str) -> bool {
    let tag = format!("<Name>{name}</Name><Value ");
    meta.split_once(&tag)
        .and_then(|(_, rest)| rest.split_once('>'))
        .and_then(|(_, v)| v.split_once("</Value>"))
        .and_then(|(v, _)| v.trim().parse::<i64>().ok())
        .is_some_and(|v| v != 0)
}

/// The field value type a parameter exposes, from its value kind.
pub(super) fn param_value_type(kind: ParameterValueKind) -> FieldValueType {
    match kind {
        ParameterValueKind::NumberParameter => FieldValueType::Number,
        ParameterValueKind::CurrencyParameter => FieldValueType::Currency,
        ParameterValueKind::BooleanParameter => FieldValueType::Boolean,
        ParameterValueKind::DateParameter => FieldValueType::Date,
        ParameterValueKind::TimeParameter => FieldValueType::Time,
        ParameterValueKind::DateTimeParameter => FieldValueType::DateTime,
        _ => FieldValueType::String,
    }
}

/// The text of the first `<tag>…</tag>` in `s` (the CRMetaObjects XML is flat and unescaped).
pub(super) fn xml_tag(s: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let start = s.find(&open)? + open.len();
    let end = s[start..].find(&format!("</{tag}>"))? + start;
    Some(s[start..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StreamId;

    /// A string in the wire form the record declares: a big-endian byte count, then the text and
    /// its terminator.
    fn lp(text: &[u8]) -> Vec<u8> {
        let mut out = ((text.len() + 1) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(text);
        out.push(0);
        out
    }

    /// A `0x007a` record with no identity: a number parameter offering no values and stopping
    /// before its tail — the shape of a parameter used only by a formula, which has no
    /// PromptManager entry to join to and so is synthesized on its own.
    fn identity_less_number_record(name: &[u8]) -> (Vec<u8>, RecordNode) {
        let mut run = vec![0x00, 0x00]; // parameter index
        run.extend_from_slice(&lp(name)); // prompt text
        run.extend_from_slice(&[0x00, 0x00]); // _u0
        run.extend_from_slice(&lp(b"")); // the field reference: no field …
        run.extend_from_slice(&[0x00, 0xff, 0xff]); // … its pool, and the unset index
        run.push(0x06); // value type: Number
        run.extend_from_slice(&[0x00, 0x00]); // value count
        run.extend_from_slice(&[0x00, 0x00]); // no bounds …
        run.extend_from_slice(&[0u8; 16]); // … and the two doubles that state them
        run.extend_from_slice(&lp(b"")); // edit mask
        param_record(&run)
    }

    /// A `0x007a` record over `run`, opened by the `0x0071` definition every one of them carries.
    /// Its header declares the enhanced string form, as a stored record's does.
    fn param_record(run: &[u8]) -> (Vec<u8>, RecordNode) {
        let mut logical = vec![0u8; 8];
        logical[0] = crate::build_model::enhanced_header_byte(ft::PARAMETER_RECORD.rtype, 0);
        let child_start = logical.len();
        logical.extend_from_slice(&[0u8; 4]);
        let child_end = logical.len();
        logical.extend_from_slice(run);
        let end = logical.len();
        let child = RecordNode {
            rtype: 0x0071,
            schema: 0x0700,
            offset: child_start,
            content_start: child_start,
            content_end: child_end,
            mask: 0,
            children: Vec::new(),
        };
        (
            logical,
            RecordNode {
                rtype: ft::PARAMETER_RECORD.rtype,
                schema: 0x0701,
                offset: 0,
                content_start: child_start,
                content_end: end,
                mask: 0,
                children: vec![child],
            },
        )
    }

    #[test]
    fn parse_identity_less_param_is_orphan_number() {
        let (logical, node) = identity_less_number_record(b"some_param");
        let rec = parse_param_record(&node, &logical);
        assert_eq!(rec.guid, None, "a record with no identity joins to nothing");
        assert_eq!(rec.prompt_text, "some_param");
        assert_eq!(rec.value_type_code, 6);
        assert!(!rec.is_optional);
        assert!(rec.allow_custom_values && rec.allow_editing_default);
        assert!(!rec.allow_multiple);
    }

    #[test]
    fn orphan_param_synthesizes_number_parameter() {
        let (logical, node) = identity_less_number_record(b"some_param");
        let rec = parse_param_record(&node, &logical);
        let fd = build_orphan_param(&rec).expect("synthesize");
        assert_eq!(fd.name, "some_param");
        assert_eq!(fd.value_type, FieldValueType::Number);
        let FieldKindData::Parameter(p) = &fd.kind else {
            panic!("expected a parameter field");
        };
        assert_eq!(p.value_kind, ParameterValueKind::NumberParameter);
        assert_eq!(p.prompt_text.as_deref(), Some("some_param"));
        assert!(!p.has_current_value);
        assert_eq!(
            p.parameter_type,
            crate::model::ParameterType::ReportParameter
        );
        assert!(p.default_values.is_empty());
    }

    /// One `0x0031` record framed as a whole logical stream: the four-byte header form, its version
    /// word, and a content run masked with the record's own type — a saved date value for parameter
    /// `index`, framed exactly as the parameter-values stream frames one.
    fn one_current_value(index: u32, date_serial: i32) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend_from_slice(&index.to_be_bytes()); // the parameter it names
        content.push(9); // its value type: Date
        content.extend_from_slice(&[0x00, 0x00]); // _u0
        content.extend_from_slice(&[0x00, 0x01]); // one value …
        content.extend_from_slice(&4u32.to_be_bytes()); // … four bytes of it …
        content.extend_from_slice(&date_serial.to_be_bytes()); // … a day number
        content.extend_from_slice(&[0x00, 0x00]); // no markers, and no tail

        // Flags: a four-byte length field, a schema word, length-prefixed strings, masked content.
        let mut out = vec![0xf8, 0x31, 0x07, 0x02];
        out.extend_from_slice(&(content.len() as u32).to_be_bytes());
        out.extend(content.iter().map(|b| b ^ 0x31));
        out
    }

    /// A record numbered `0x0031` is a saved current value in the parameter-values stream and an
    /// unrelated record in the report definition, so the same bytes must yield a value in the first
    /// stream and nothing at all in the second.
    ///
    /// Both readings find the record — the number is reachable either way — which is what makes the
    /// stream's vocabulary, and not the number, the thing that keeps a report-definition record out
    /// of a report's parameter values.
    #[test]
    fn a_saved_value_is_read_only_in_the_stream_whose_vocabulary_numbers_it() {
        let logical = one_current_value(7, 2_455_334);
        let as_values = RecordStream::from_logical_bytes(
            StreamId::ReportParametersStream("ReportParametersStream 1l".to_owned()),
            &logical,
        );
        let as_definition = RecordStream::from_logical_bytes(StreamId::Contents, &logical);

        for stream in [&as_values, &as_definition] {
            let types: Vec<u16> = stream.record_tree().iter().map(|n| n.rtype).collect();
            assert_eq!(
                types,
                vec![ft::CURRENT_VALUE_RECORD.rtype],
                "the record is framed the same in {:?}",
                stream.id()
            );
        }

        let values = parse_report_parameters(&as_values);
        assert_eq!(
            values[&7],
            vec![ParameterValue {
                value: "Date(2010,05,18)".to_owned(),
                description: Some(String::new()),
                range: None,
            }]
        );
        assert!(
            parse_report_parameters(&as_definition).is_empty(),
            "a report-definition record must not be read as a saved parameter value"
        );
    }

    /// A stored value is written as a literal of its type, with no locale in it: a date in the same
    /// `Date(YYYY,MM,DD)` form the `PromptManager` document uses, so both records that can state one
    /// parameter's values state them alike; a number in the shortest decimal that reads back as the
    /// same double, with nothing rounded away and nothing padded on.
    #[test]
    fn a_stored_value_is_written_as_a_literal_of_its_type() {
        assert_eq!(date_literal(2_455_334), "Date(2010,05,18)");
        // Single-digit month and day are padded, so every date is the same width.
        assert_eq!(date_literal(2_451_604), "Date(2000,03,01)");

        assert_eq!(number_literal(10.0), "10");
        assert_eq!(number_literal(4.05), "4.05");
        // A value near a whole number keeps the digits that make it near one rather than becoming it.
        assert_eq!(number_literal(9.999_999_999), "9.999999999");
        assert_eq!(number_literal(-0.5), "-0.5");
    }

    #[test]
    fn value_kind_codes_map_to_their_parameter_kinds() {
        assert_eq!(value_kind_from_code(6), ParameterValueKind::NumberParameter);
        assert_eq!(
            value_kind_from_code(7),
            ParameterValueKind::CurrencyParameter
        );
        assert_eq!(
            value_kind_from_code(8),
            ParameterValueKind::BooleanParameter
        );
        assert_eq!(value_kind_from_code(9), ParameterValueKind::DateParameter);
        assert_eq!(value_kind_from_code(10), ParameterValueKind::TimeParameter);
        assert_eq!(
            value_kind_from_code(11),
            ParameterValueKind::StringParameter
        );
        assert_eq!(
            value_kind_from_code(15),
            ParameterValueKind::DateTimeParameter
        );
        // Unknown code falls back to a string parameter (conservative default).
        assert_eq!(
            value_kind_from_code(200),
            ParameterValueKind::StringParameter
        );
    }

    /// Wrap `value` in a `<Property>` named `name`, as the PromptManager XML stores it.
    fn property(name: &str, variant: &str, value: &str) -> String {
        format!(
            "<Property><Name>{name}</Name>\
             <Value VariantType=\"{variant}\">{value}</Value></Property>"
        )
    }

    #[test]
    fn prop_flag_reads_the_whole_integer_value() {
        // A VB VARIANT_BOOL true is -1, not 1; both are true, and both variant labels are read the
        // same way.
        assert!(prop_flag(&property("F", "Boolean", "-1"), "F"));
        assert!(prop_flag(&property("F", "Boolean", "1"), "F"));
        assert!(prop_flag(&property("F", "Integer", "1"), "F"));
        assert!(prop_flag(&property("F", "Integer", "-1"), "F"));
        // Any other nonzero integer is true; only zero (in either sign) is false.
        assert!(prop_flag(&property("F", "Integer", "2"), "F"));
        assert!(prop_flag(&property("F", "Integer", "-2"), "F"));
        assert!(!prop_flag(&property("F", "Boolean", "0"), "F"));
        assert!(!prop_flag(&property("F", "Integer", "-0"), "F"));
        // Surrounding whitespace is not part of the value.
        assert!(prop_flag(&property("F", "Boolean", " -1 "), "F"));
        // A value that is not an integer, an empty value, and an absent property are all false.
        assert!(!prop_flag(&property("F", "String", "true"), "F"));
        assert!(!prop_flag(&property("F", "Boolean", ""), "F"));
        assert!(!prop_flag(&property("F", "Boolean", "1"), "Other"));
        // Only the named property's value is read, not a neighbour's.
        let two = property("A", "Boolean", "0") + &property("B", "Boolean", "-1");
        assert!(!prop_flag(&two, "A"));
        assert!(prop_flag(&two, "B"));
    }

    #[test]
    fn build_parameters_decodes_group_linkage() {
        // A minimal two-parameter PromptManager document: one standalone (PartOfGroup=0), one that
        // is a group member (PartOfGroup=1, mutually exclusive) — exercises the group-linkage decode.
        let xml = "<CRMetaObjects>\
            <MetaObject xsi:type=\"CRMetaObject\" id=\"1\">\
              <ID>crobj://{AAAA}</ID><Desc>solo</Desc><Type>Parameter</Type>\
              <Properties>\
                <Property><Name>Int_ShowOnViewerPanel</Name><Value VariantType=\"Integer\">1</Value></Property>\
              </Properties>\
              <Object xsi:type=\"Parameter\" id=\"2\"><Name>solo</Name><ValueType>String</ValueType>\
                <PromptGroupRef>crobj://{GRP1}</PromptGroupRef></Object>\
            </MetaObject>\
            <MetaObject xsi:type=\"CRMetaObject\" id=\"3\">\
              <ID>crobj://{BBBB}</ID><Desc>child</Desc><Type>Parameter</Type>\
              <Properties>\
                <Property><Name>Boolean_MutuallyExclusiveGroup</Name><Value VariantType=\"Boolean\">1</Value></Property>\
                <Property><Name>Boolean_PartOfGroup</Name><Value VariantType=\"Boolean\">1</Value></Property>\
              </Properties>\
              <Object xsi:type=\"Parameter\" id=\"4\"><Name>child</Name><ValueType>Number</ValueType>\
                <PromptGroupRef>crobj://{GRP2}</PromptGroupRef></Object>\
            </MetaObject>\
            </CRMetaObjects>";
        let recs = BTreeMap::new();
        let cur = BTreeMap::new();
        let fields = build_parameters(xml, &recs, &cur);
        assert_eq!(fields.len(), 2);
        let solo = fields.iter().find(|f| f.name == "solo").unwrap();
        let child = fields.iter().find(|f| f.name == "child").unwrap();
        let FieldKindData::Parameter(sp) = &solo.kind else {
            panic!("expected parameter");
        };
        let FieldKindData::Parameter(cp) = &child.kind else {
            panic!("expected parameter");
        };
        assert_eq!(sp.prompt_group.as_deref(), Some("crobj://{GRP1}"));
        assert!(!sp.part_of_group);
        assert!(!sp.mutually_exclusive_group);
        assert_eq!(cp.prompt_group.as_deref(), Some("crobj://{GRP2}"));
        assert!(cp.part_of_group);
        assert!(cp.mutually_exclusive_group);
        assert!(cp.dynamic_lov.is_none());
    }

    #[test]
    fn parameter_value_models_a_range() {
        use crate::model::{ParameterRange, ParameterValue, RangeBoundType};
        // A range value: value = lower bound, range.end_value = upper bound, per-end inclusivity.
        let v = ParameterValue {
            value: "1/1/2024".into(),
            description: None,
            range: Some(ParameterRange {
                end_value: "12/31/2024".into(),
                lower_bound: RangeBoundType::BoundInclusive,
                upper_bound: RangeBoundType::BoundExclusive,
            }),
        };
        let r = v.range.as_ref().unwrap();
        assert_eq!(v.value, "1/1/2024");
        assert_eq!(r.end_value, "12/31/2024");
        assert_eq!(r.lower_bound, RangeBoundType::BoundInclusive);
        assert_eq!(r.upper_bound, RangeBoundType::BoundExclusive);
        // A discrete value leaves range unset.
        let d = ParameterValue {
            value: "x".into(),
            description: None,
            range: None,
        };
        assert!(d.range.is_none());
    }
}
