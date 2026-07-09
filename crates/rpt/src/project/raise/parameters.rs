//! Parameter fields — PromptManager XML joined with the `0x007a` Contents detail records.

use super::*;

/// The decoded detail of one parameter, from its `0x007a` Contents record. The PromptText, the
/// report-vs-stored-procedure kind, and the optional-prompt flag are not in the PromptManager XML
/// (which carries only name, value type and panel visibility) — they live in this record.
pub(super) struct ParamRecord {
    /// The `crobj://{…}` GUID that joins this record to its PromptManager entry, or `None` for a
    /// GUID-less record — a parameter used only in a formula, with no PromptManager entry. GUID-less
    /// records are synthesized directly into a `ParameterField` by
    /// [`raise_orphan_param`] rather than joined.
    pub(super) guid: Option<String>,
    pub(super) prompt_text: String,
    pub(super) is_sp_param: bool,
    pub(super) is_optional: bool,
    pub(super) allow_multiple: bool,
    pub(super) allow_custom_values: bool,
    pub(super) allow_editing_default: bool,
    /// The engine's global parameter index (u16 BE at leaf `[0..2]`). The saved current-value
    /// records in the top-level `ReportParametersStream` (`0x0031`) reference a parameter by this
    /// same index — it is the join key between a parameter and its saved last-used value.
    pub(super) index: u16,
    /// The parameter's default pick-list (`<ParameterDefaultValues>`): the allowed values plus their
    /// descriptions, decoded from the value-list block of this `0x007a` record. Empty for a plain
    /// parameter with no pick list.
    pub(super) default_values: Vec<ParameterValue>,
    /// SDK `@DefaultValueDisplayType` — decoded from a flag byte in this `0x007a` record.
    pub(super) display_type: crate::model::ParameterDisplayType,
    /// SDK `@DefaultValueSortOrder` — decoded from a flag byte in this `0x007a` record.
    pub(super) sort_order: crate::model::ParameterSortOrder,
    /// The raw Crystal value-type code (`CrFieldValueType`) from the record's `ff ff <vt>` value-list
    /// marker (`6` = Number, `7` = Currency, `9` = Date, `11` = String — see [`parse_value_entries`]).
    /// Used only to resolve the [`ParameterValueKind`] for a GUID-less record, which has no
    /// PromptManager `ValueType` to read.
    pub(super) value_type_code: Option<u8>,
}

/// Decode a parameter detail record (`0x007a`, leaf already de-obfuscated by the XOR-0x7a record
/// mask): the PromptText (length-prefixed at offset 5), the ParameterType byte after the 32-byte
/// `0xFF` block (`0x02` = stored-procedure), the optional-prompt flag (last byte), and the
/// `crobj://{…}` GUID that joins to the PromptManager entry.
pub(super) fn parse_param_leaf(leaf: &[u8]) -> Option<ParamRecord> {
    if leaf.len() < 15 {
        return None;
    }
    let is_optional = *leaf.last()? == 1;
    let lp_len = *leaf.get(5)? as usize;
    // The PromptText is UTF-8 (it can carry localized text), so decode the byte span as UTF-8
    // rather than byte-per-char (Latin-1).
    let raw = leaf.get(6..6 + lp_len)?;
    let raw = &raw[..raw.iter().position(|&b| b == 0).unwrap_or(raw.len())];
    let prompt_text = String::from_utf8_lossy(raw).into_owned();
    // ParameterType: the type byte (`0x02` = stored-procedure, `0x00` = report parameter) sits
    // immediately after a fixed 32-byte `0xFF` block. The block's offset shifts with the value-type
    // bounds layout (Date is +8 vs String) and with command-bound params that embed a `Command.<col>`
    // LP string, so anchor on the `0xFF` run itself instead of a fixed offset.
    let p = 6 + lp_len;
    // Both the ParameterType byte and the EnableAllowMultipleValue flag are positioned relative to
    // the record's fixed 32-byte `0xFF` block: the type byte sits at the block end, the multi-value
    // flag 6 bytes past it. `0x01` = multiple values allowed.
    let ff_end = ff_block_end(leaf, p);
    let is_sp_param = ff_end.and_then(|e| leaf.get(e).copied()) == Some(0x02);
    let allow_multiple = ff_end
        .and_then(|e| leaf.get(e + 6).copied())
        .is_some_and(|b| b == 1);
    // Dynamic / list-of-values parameters disallow both AllowCustomCurrentValues and
    // EnableAllowEditingDefaultValue (static parameters allow both). Two encodings mark this: a
    // `0x01` at `ff_block_end + 4` (SAP Business One `$[…]`/`Object@` params), or a `0x01` 12 bytes
    // before the `/crobj://{…}` GUID reference (Crystal LOV params).
    let dynamic = ff_end
        .and_then(|e| leaf.get(e + 4).copied())
        .is_some_and(|b| b == 1)
        || leaf
            .windows(6)
            .position(|w| w == b"/crobj")
            .and_then(|p| p.checked_sub(12))
            .and_then(|q| leaf.get(q).copied())
            == Some(1);
    let allow_custom_values = !dynamic;
    let allow_editing_default = !dynamic;
    // A GUID-less record (parameter used only in a formula, no PromptManager entry) is kept — it is
    // synthesized into a `ParameterField` directly by `raise_orphan_param`.
    let guid = find_guid_lp(leaf);
    let index = u16_be(leaf, 0)?;
    let value_type_code = find_value_marker(leaf, p).and_then(|m| leaf.get(m + 2).copied());
    let default_values = parse_default_value_list(leaf, p).unwrap_or_default();
    let (display_type, sort_order) = ff_end
        .and_then(|e| parse_default_value_flags(leaf, e))
        .unwrap_or_default();
    Some(ParamRecord {
        guid,
        prompt_text,
        is_sp_param,
        is_optional,
        allow_multiple,
        allow_custom_values,
        allow_editing_default,
        index,
        default_values,
        display_type,
        sort_order,
        value_type_code,
    })
}

/// The [`ParameterValueKind`] for a GUID-less parameter, resolved from its `0x007a` value-list marker
/// code (`CrFieldValueType`). `6`/`7`/`9`/`11` (Number/Currency/Date/String) are confirmed by the
/// value-entry decoder; `8`/`10` (Boolean/Time) are the natural fill and marked unverified (no corpus
/// sample of a GUID-less parameter with those codes). Unknown codes fall back to a string parameter.
fn value_kind_from_code(code: u8) -> ParameterValueKind {
    match code {
        6 => ParameterValueKind::NumberParameter,
        7 => ParameterValueKind::CurrencyParameter,
        8 => ParameterValueKind::BooleanParameter, // unverified: no corpus sample
        9 => ParameterValueKind::DateParameter,
        10 => ParameterValueKind::TimeParameter, // unverified: no corpus sample
        11 => ParameterValueKind::StringParameter,
        _ => ParameterValueKind::StringParameter,
    }
}

/// Synthesize a `ParameterField` from a GUID-less `0x007a` record (a parameter referenced only by a
/// formula, absent from the PromptManager). Its Name and PromptText are the record's LP-string (offset
/// 6); the value kind comes from the value-list marker; the flag attributes are the ones already
/// decoded from the record. Initial/current value lists are empty (there is no PromptManager entry or
/// `ReportParametersStream` join). Returns `None` if the record carries no usable name.
pub(super) fn raise_orphan_param(rec: &ParamRecord) -> Option<FieldDef> {
    let name = rec.prompt_text.clone();
    if name.is_empty() {
        return None;
    }
    let value_kind = rec
        .value_type_code
        .map(value_kind_from_code)
        .unwrap_or_default();
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

/// Decode the SDK `@DefaultValueDisplayType` and `@DefaultValueSortOrder` flags from a `0x007a`
/// parameter record. Both are single bytes positioned relative to the parameter-name LP-string (the
/// first length-prefixed printable string after the 32-byte `0xFF` bounds block, `ff_end`): the
/// display-type byte sits 4 bytes past the end of the name field, the sort-order byte 5 bytes past
/// it. Values: display `1` = `Description` (else `DescriptionAndValue`); sort `1` =
/// `AlphabeticalAscending` (else `NoSort`). Absent ⇒ engine defaults.
fn parse_default_value_flags(
    leaf: &[u8],
    ff_end: usize,
) -> Option<(
    crate::model::ParameterDisplayType,
    crate::model::ParameterSortOrder,
)> {
    use crate::model::{ParameterDisplayType, ParameterSortOrder};
    const DISPLAY_TYPE_OFFSET: usize = 4;
    const SORT_ORDER_OFFSET: usize = 5;
    let (name_pos, name_len) = first_lp_after(leaf, ff_end)?;
    let name_end = name_pos + 1 + name_len;
    let display = match leaf.get(name_end + DISPLAY_TYPE_OFFSET).copied() {
        Some(1) => ParameterDisplayType::Description,
        _ => ParameterDisplayType::DescriptionAndValue,
    };
    let sort = match leaf.get(name_end + SORT_ORDER_OFFSET).copied() {
        Some(1) => ParameterSortOrder::AlphabeticalAscending,
        _ => ParameterSortOrder::NoSort,
    };
    Some((display, sort))
}

/// The `(length-byte index, content length)` of the first length-prefixed printable string at or
/// after `from` — anchors on the parameter-name LP-string. Mirrors [`read_lp_strings`]'s per-string
/// validation (a `u8` length in `1..=64` with printable, non-empty content).
fn first_lp_after(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut p = from;
    while p < bytes.len() {
        let len = bytes[p] as usize;
        if (1..=64).contains(&len) && p + 1 + len <= bytes.len() {
            let raw = &bytes[p + 1..p + 1 + len];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            if end > 0 && raw[..end].iter().all(|&b| (0x20..0x7f).contains(&b)) {
                return Some((p, len));
            }
        }
        p += 1;
    }
    None
}

/// Decode the default pick-list (`<ParameterDefaultValues>`) from a `0x007a` parameter record.
///
/// After the PromptText the record carries, in order: a 2-byte `ff ff` marker, a value-type byte
/// (`CrFieldValueType`), a `u16` BE count, then `count` value entries; later (after the 32-byte
/// `0xFF` bounds block) the parameter-name LP-string followed by `count` description LP-strings, then
/// the `crobj://` GUID. Value `[i]` pairs with description `[i]` (the engine's pick-list order).
///
/// Value entry encoding by type:
/// - Number / Currency: `[u32 BE 8][f64 BE]` — the stored double is the human value **×100**
///   (Crystal's fixed-2-decimal internal scale for Number/Currency parameter values), so the emitted
///   value is `double / 100`.
/// - Date: `[u32 BE 4][u32 BE]` — a Julian Day Number (+1 day vs the astronomical JDN epoch).
/// - String: `[u32 BE len][u32 BE len][len bytes incl NUL]` — stored verbatim.
fn parse_default_value_list(leaf: &[u8], prompt_end: usize) -> Option<Vec<ParameterValue>> {
    let m = find_value_marker(leaf, prompt_end)?;
    let value_type = *leaf.get(m + 2)?;
    let count = u16_be(leaf, m + 3)? as usize;
    if count == 0 {
        return Some(Vec::new());
    }
    let (values, _) = parse_value_entries(leaf, m + 5, count, value_type)?;
    // Descriptions: the LP-strings after the parameter-name LP-string (which follows the `0xFF`
    // bounds block), bounded by the `crobj://` GUID. The first is the parameter name; the next
    // `count` are the per-value descriptions (fewer ⇒ the remaining values have an empty one).
    let descs = read_descriptions(leaf, m, count);
    Some(
        values
            .into_iter()
            .enumerate()
            .map(|(i, value)| ParameterValue {
                value,
                description: Some(descs.get(i).cloned().unwrap_or_default()),
                range: None,
            })
            .collect(),
    )
}

/// Decode `count` value entries of `value_type` starting at `p`, returning the formatted values and
/// the offset just past them. Shared by the `0x007a` default pick-list and the `ReportParametersStream`
/// `0x0031` current-value records (same per-type entry encoding). Returns `None` for an unknown type
/// (rather than guess a wrong value).
fn parse_value_entries(
    leaf: &[u8],
    p: usize,
    count: usize,
    value_type: u8,
) -> Option<(Vec<String>, usize)> {
    let mut c = Cursor::at(leaf, p);
    let mut values: Vec<String> = Vec::with_capacity(count);
    for _ in 0..count {
        // Every entry is length-prefixed by a u32 BE byte count.
        let _len = c.u32_be()?;
        match value_type {
            // Number (6) / Currency (7): an 8-byte BE double, scaled ×100.
            6 | 7 => values.push(format_number(c.f64_be()? / 100.0)),
            // Date (9): a 4-byte BE Julian Day Number.
            9 => values.push(format_date(c.u32_be()?)),
            // String (11): a redundant second length word, then the NUL-terminated bytes.
            11 => {
                let slen = c.u32_be()? as usize;
                let raw = c.bytes(slen)?;
                let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                values.push(String::from_utf8_lossy(&raw[..end]).into_owned());
            }
            // Unknown value type — don't guess (would risk wrong values).
            _ => return None,
        }
    }
    Some((values, c.pos()))
}

/// The index of the 2-byte `ff ff` value-list marker at/after `from`: the first `0xff 0xff` pair
/// whose following byte is **not** `0xff` (which would be the longer `0xFF` bounds block). The
/// value-type byte sits at the returned index + 2.
fn find_value_marker(leaf: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 2 < leaf.len() {
        if leaf[i] == 0xff && leaf[i + 1] == 0xff && leaf[i + 2] != 0xff {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Collect up to `count` per-value description strings: the printable LP-strings (skipping `0x00`
/// padding) between the `0xFF` bounds block and the `crobj://` GUID. The first LP-string is the
/// parameter name and is dropped; the rest are the descriptions in value order.
fn read_descriptions(leaf: &[u8], marker: usize, count: usize) -> Vec<String> {
    let start = match ff_block_end(leaf, marker) {
        Some(e) => e,
        None => return Vec::new(),
    };
    // Stop before the `crobj://` GUID's LP length byte (so it is never read as a description).
    let limit = leaf
        .windows(8)
        .position(|w| w == b"crobj://")
        .map(|pos| pos.saturating_sub(1))
        .unwrap_or(leaf.len());
    // The first LP-string is the parameter name; drop it, keeping the descriptions.
    let mut out = read_lp_strings(&leaf[start.min(limit)..limit], count + 1);
    if !out.is_empty() {
        out.remove(0);
    }
    out
}

/// Collect up to `max` non-empty printable NUL-terminated **byte-length-prefixed** strings from
/// `bytes` (a `u8` length ≤ 64, unlike the u32-prefixed [`read_lp_string`] flavor), skipping any
/// non-conforming padding/marker bytes between them (advance one byte on a non-match).
///
/// A deliberate `u8`-length, printable-filtered variant — a different framing from the
/// `u32`-prefixed `read_be_lp_string_lossy`, so it is not collapsed onto it.
fn read_lp_strings(bytes: &[u8], max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut p = 0;
    while p < bytes.len() && out.len() < max {
        let len = bytes[p] as usize;
        if len == 0 || len > 64 || p + 1 + len > bytes.len() {
            p += 1;
            continue;
        }
        let raw = &bytes[p + 1..p + 1 + len];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        if end == 0 || !raw[..end].iter().all(|&b| (0x20..0x7f).contains(&b)) {
            p += 1;
            continue;
        }
        out.push(String::from_utf8_lossy(&raw[..end]).into_owned());
        p += 1 + len;
    }
    out
}

/// Format a Number/Currency parameter value: whole numbers without a decimal point, otherwise the
/// shortest round-trip form.
fn format_number(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v}")
    }
}

/// Format a Crystal date serial (Julian Day Number) as `M/D/YYYY 12:00:00 AM`.
fn format_date(jdn: u32) -> String {
    // Crystal's serial is one day past the astronomical JDN at this epoch.
    let j = jdn as i64 + 1;
    let a = j + 32044;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = 100 * b + d - 4800 + m / 10;
    format!("{month}/{day}/{year} 12:00:00 AM")
}

/// The index immediately following the first run of at least 32 consecutive `0xFF` bytes at or after
/// `from` — the parameter record's type/flag block anchor, robust to value-type and command-binding
/// layout shifts. The ParameterType byte sits at this index; EnableAllowMultipleValue at index + 6.
/// Returns `None` if no such run exists.
fn ff_block_end(leaf: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < leaf.len() {
        if leaf[i] == 0xff {
            let start = i;
            while i < leaf.len() && leaf[i] == 0xff {
                i += 1;
            }
            if i - start >= 32 {
                return Some(i);
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Find the `crobj://{…}` GUID stored as a length-prefixed string anywhere in a parameter record's
/// leaf (the byte before `"crobj://"` is its LP length).
pub(super) fn find_guid_lp(leaf: &[u8]) -> Option<String> {
    let prefix = b"crobj://";
    for i in 0..leaf.len().saturating_sub(prefix.len() + 1) {
        let lp_len = leaf[i] as usize;
        if lp_len > prefix.len() && leaf.get(i + 1..i + 1 + prefix.len()) == Some(&prefix[..]) {
            let content = leaf.get(i + 1..i + lp_len)?;
            return Some(
                content
                    .iter()
                    .take_while(|&&b| b != 0)
                    .map(|&b| b as char)
                    .collect(),
            );
        }
    }
    None
}

/// Build the map of saved current parameter values keyed by the engine's global parameter index,
/// decoded from the top-level `ReportParametersStream`. Each `0x0031` record holds the index (u32 BE
/// at `[0..4]`) and an embedded `<CRMetaObjects>` `<Object xsi:type="Values">` document whose
/// `<Value xsi:type="DiscreteValue">` children carry the saved `<Description>` and `<Value>` verbatim
/// (the value is already the human form — no ×100 scaling, unlike the binary pick-list doubles).
pub(crate) fn parse_report_parameters(stream: &RecordStream) -> BTreeMap<u16, Vec<ParameterValue>> {
    const CURRENT_VALUE_RECORD: u16 = 0x0031;
    let mut out: BTreeMap<u16, Vec<ParameterValue>> = BTreeMap::new();
    let logical = stream.logical_bytes();
    for root in stream.record_tree() {
        root.walk(&mut |n| {
            if n.rtype != CURRENT_VALUE_RECORD {
                return;
            }
            let leaf = n.leaf_bytes(logical);
            // Header: u32 BE index, value-type byte, u32 BE value count, then the binary value
            // entries (same encoding as the `0x007a` pick-list — Number doubles are ×100-scaled, so
            // the binary form is authoritative and avoids the embedded XML's value entirely).
            let Some(idx) = u32_be(&leaf, 0) else {
                return;
            };
            let Some(&value_type) = leaf.get(4) else {
                return;
            };
            let Some(count) = u32_be(&leaf, 5).map(|n| n as usize) else {
                return;
            };
            let Some((raw_values, after)) = parse_value_entries(&leaf, 9, count, value_type) else {
                // A range (non-discrete) current value: entries can't be recovered, but the record's
                // presence means the parameter has a saved current value. Mark it with an empty entry.
                out.entry(idx as u16).or_default();
                return;
            };
            // Descriptions are LP-string(s) trailing the embedded `</CRMetaObjects>` (when the record
            // carries one) or the binary values otherwise. The XML's own `<Description>` is unreliable
            // (often empty even when the engine reports a description), so the trailing form wins.
            let xml = String::from_utf8_lossy(&leaf);
            let desc_start = xml
                .find("</CRMetaObjects>")
                .map(|c| c + "</CRMetaObjects>".len())
                .unwrap_or(after);
            let descs = read_lp_strings(&leaf[desc_start.min(leaf.len())..], count);
            let values: Vec<ParameterValue> = raw_values
                .into_iter()
                .enumerate()
                .map(|(i, value)| ParameterValue {
                    value,
                    description: Some(descs.get(i).cloned().unwrap_or_default()),
                    range: None,
                })
                .collect();
            if !values.is_empty() {
                out.insert(idx as u16, values);
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

/// Extract parameter field definitions from the `PromptManager` CRMetaObjects XML, joined to their
/// `0x007a` detail records (`param_records`, keyed by GUID) for the PromptText, ParameterType,
/// optional-prompt flag and the DefaultValues pick list, plus the saved CurrentValues (`current_values`,
/// keyed by the engine parameter index). Panel visibility comes from the XML's `Int_ShowOnViewerPanel`;
/// PromptText falls back to a synthesised `Enter {name}:` when the detail record is absent.
/// `HasCurrentValue` is set per parameter to whether it has a decoded saved current value.
pub(super) fn raise_parameters(
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
                prompt_text: Some(
                    rec.map(|r| r.prompt_text.clone())
                        .unwrap_or_else(|| format!("Enter {name}:")),
                ),
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
/// `<Name>{name}</Name><Value …>X</Value>` and returns `true` iff `X` begins with `1` (the engine
/// writes `Boolean`/`Integer` variant flags as `0`/`1`). Absent property ⇒ `false`.
fn prop_flag(meta: &str, name: &str) -> bool {
    let tag = format!("<Name>{name}</Name><Value ");
    meta.split_once(&tag)
        .and_then(|(_, rest)| rest.split_once('>'))
        .and_then(|(_, v)| v.trim_start().chars().next())
        == Some('1')
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

    /// A GUID-less `0x007a` leaf: index, an offset-5 LP name, a `ff ff <vt>` value marker with a
    /// zero count, then a 32-byte `0xFF` bounds block — the shape of a formula-only parameter.
    /// `find_guid_lp` finds no `crobj://`, so `guid` is `None` and the record is treated as an
    /// orphan; `value_type_code` is read from the marker.
    fn guidless_number_leaf(name: &[u8]) -> Vec<u8> {
        let mut leaf = vec![0u8; 5]; // index (u16 BE) + padding
        leaf.push(name.len() as u8); // offset 5: LP length
        leaf.extend_from_slice(name); // offset 6..: name
        leaf.extend_from_slice(&[0xff, 0xff, 0x06]); // value marker, vt = 6 (Number)
        leaf.extend_from_slice(&[0x00, 0x00]); // value count = 0
        leaf.extend(std::iter::repeat_n(0xff, 32)); // 32-byte 0xFF bounds block
        leaf.extend_from_slice(&[0u8; 8]); // trailer past ff_block_end
        leaf.push(0x00); // last byte: is_optional = false
        leaf
    }

    #[test]
    fn parse_guidless_param_is_orphan_number() {
        let leaf = guidless_number_leaf(b"some_param");
        let rec = parse_param_leaf(&leaf).expect("parse");
        assert_eq!(rec.guid, None, "GUID-less record must have no GUID");
        assert_eq!(rec.prompt_text, "some_param");
        assert_eq!(rec.value_type_code, Some(6));
        assert!(!rec.is_optional);
        assert!(rec.allow_custom_values && rec.allow_editing_default);
        assert!(!rec.allow_multiple);
    }

    #[test]
    fn orphan_param_synthesizes_number_parameter() {
        let leaf = guidless_number_leaf(b"some_param");
        let rec = parse_param_leaf(&leaf).expect("parse");
        let fd = raise_orphan_param(&rec).expect("synthesize");
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

    #[test]
    fn value_kind_codes_map_to_confirmed_kinds() {
        assert_eq!(value_kind_from_code(6), ParameterValueKind::NumberParameter);
        assert_eq!(
            value_kind_from_code(7),
            ParameterValueKind::CurrencyParameter
        );
        assert_eq!(value_kind_from_code(9), ParameterValueKind::DateParameter);
        assert_eq!(
            value_kind_from_code(11),
            ParameterValueKind::StringParameter
        );
        // Unknown code falls back to a string parameter (conservative default).
        assert_eq!(
            value_kind_from_code(200),
            ParameterValueKind::StringParameter
        );
    }

    #[test]
    fn prop_flag_reads_boolean_and_integer_variants() {
        let meta = "<Property><Name>Boolean_PartOfGroup</Name>\
             <Value VariantType=\"Boolean\">1</Value></Property>\
             <Property><Name>Boolean_MutuallyExclusiveGroup</Name>\
             <Value VariantType=\"Boolean\">0</Value></Property>\
             <Property><Name>Boolean_GroupNumber</Name>\
             <Value VariantType=\"Integer\">1</Value></Property>";
        assert!(prop_flag(meta, "Boolean_PartOfGroup"));
        assert!(!prop_flag(meta, "Boolean_MutuallyExclusiveGroup"));
        // Reads the value regardless of the VariantType label.
        assert!(prop_flag(meta, "Boolean_GroupNumber"));
        // Absent property -> false.
        assert!(!prop_flag(meta, "Boolean_Missing"));
    }

    #[test]
    fn raise_parameters_decodes_group_linkage() {
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
        let fields = raise_parameters(xml, &recs, &cur);
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
        // Range/dynamic default to the discrete/no-binding shape (the corpus carries neither).
        assert_eq!(
            cp.discrete_or_range_kind,
            crate::model::DiscreteOrRangeKind::DiscreteValue
        );
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
