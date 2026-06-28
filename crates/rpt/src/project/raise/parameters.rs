//! Parameter fields — PromptManager XML joined with the `0x007a` Contents detail records.

use super::*;

/// The decoded detail of one parameter, from its `0x007a` Contents record. The PromptText, the
/// report-vs-stored-procedure kind, and the optional-prompt flag are not in the PromptManager XML
/// (which carries only name, value type and panel visibility) — they live in this record.
pub(super) struct ParamRecord {
    pub(super) guid: String,
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
    // Dynamic / list-of-values parameters (SAP Business One `$[…]` and `Object@` params) store a
    // `0x01` at `ff_block_end + 4`; for those both AllowCustomCurrentValues and
    // EnableAllowEditingDefaultValue are False. Static parameters store `0x00` (both True).
    let dynamic = ff_end
        .and_then(|e| leaf.get(e + 4).copied())
        .is_some_and(|b| b == 1);
    let allow_custom_values = !dynamic;
    let allow_editing_default = !dynamic;
    let guid = find_guid_lp(leaf)?;
    let index = u16::from_be_bytes([*leaf.first()?, *leaf.get(1)?]);
    let default_values = parse_default_value_list(leaf, p).unwrap_or_default();
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
    })
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
    let count = u16::from_be_bytes([*leaf.get(m + 3)?, *leaf.get(m + 4)?]) as usize;
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
    mut p: usize,
    count: usize,
    value_type: u8,
) -> Option<(Vec<String>, usize)> {
    let mut values: Vec<String> = Vec::with_capacity(count);
    for _ in 0..count {
        // Every entry is length-prefixed by a u32 BE byte count.
        let len = u32::from_be_bytes(leaf.get(p..p + 4)?.try_into().ok()?) as usize;
        p += 4;
        match value_type {
            // Number (6) / Currency (7): an 8-byte BE double, scaled ×100.
            6 | 7 => {
                let f = f64::from_be_bytes(leaf.get(p..p + 8)?.try_into().ok()?) / 100.0;
                values.push(format_number(f));
                p += 8;
            }
            // Date (9): a 4-byte BE Julian Day Number.
            9 => {
                let jdn = u32::from_be_bytes(leaf.get(p..p + 4)?.try_into().ok()?);
                values.push(format_date(jdn));
                p += 4;
            }
            // String (11): a redundant second length word, then the NUL-terminated bytes.
            11 => {
                let slen = u32::from_be_bytes(leaf.get(p..p + 4)?.try_into().ok()?) as usize;
                p += 4;
                let raw = leaf.get(p..p + slen)?;
                let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                values.push(String::from_utf8_lossy(&raw[..end]).into_owned());
                p += slen;
            }
            // Unknown value type — don't guess (would risk wrong values).
            _ => return None,
        }
        let _ = len;
    }
    Some((values, p))
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
    let mut out = Vec::new();
    let mut p = start;
    // Collect the printable, non-empty LP-strings (first is the parameter name, dropped; the rest
    // are the descriptions). Padding and stray short markers (e.g. `01 00`) between them are not
    // valid LP-strings, so on a non-match advance one byte rather than stopping.
    while p < limit && out.len() <= count {
        let len = leaf[p] as usize;
        if len == 0 || len > 64 || p + 1 + len > limit {
            p += 1;
            continue;
        }
        let raw = &leaf[p + 1..p + 1 + len];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        // A description LP-string is non-empty printable text terminated within its length.
        if end == 0 || !raw[..end].iter().all(|&b| (0x20..0x7f).contains(&b)) {
            p += 1;
            continue;
        }
        out.push(String::from_utf8_lossy(&raw[..end]).into_owned());
        p += 1 + len;
    }
    // Drop the leading parameter-name LP-string.
    if !out.is_empty() {
        out.remove(0);
    }
    out
}

/// Collect up to `max` non-empty printable NUL-terminated LP-strings from `bytes`, skipping any
/// non-conforming padding/marker bytes between them (advance one byte on a non-match).
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
            let Some(idx) = leaf
                .get(0..4)
                .and_then(|b| <[u8; 4]>::try_from(b).ok())
                .map(u32::from_be_bytes)
            else {
                return;
            };
            let Some(&value_type) = leaf.get(4) else {
                return;
            };
            let Some(count) = leaf
                .get(5..9)
                .and_then(|b| <[u8; 4]>::try_from(b).ok())
                .map(|b| u32::from_be_bytes(b) as usize)
            else {
                return;
            };
            let Some((raw_values, after)) = parse_value_entries(&leaf, 9, count, value_type) else {
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
        out.push(ParameterValue { value, description });
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
                has_current_value: !current.is_empty(),
                allow_multiple_values: rec.is_some_and(|r| r.allow_multiple),
                // Default True when no detail record exists (e.g. auto-generated command params).
                allow_custom_values: rec.is_none_or(|r| r.allow_custom_values),
                allow_editing_default_value: rec.is_none_or(|r| r.allow_editing_default),
                default_values,
                initial_values,
                current_values: current,
                ..Default::default()
            })),
            value_type: param_value_type(value_kind),
            name,
            ..Default::default()
        });
    }
    out
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
