//! Read path — build the semantic [`Report`] from the decoded record substrate.

use std::collections::BTreeMap;

use crate::codec::RecordNode;
use crate::container::SummaryInformation;
use crate::model::{
    Alignment, Area, AreaSectionKind, Color, ConnectionInfo, DataDefinition, Database, DbField,
    DbFieldDef, FieldDef, FieldKindData, FieldObject, FieldRefKind, FieldValueType, Font,
    FontColor, Formula, FormulaField, Group, LineStyle, Node, ParameterField, ParameterValue,
    ParameterValueKind, PrintOptions, RecordTypeCount, Rect, Report, ReportDefinition,
    ReportObject, ReportObjectKind, ResetConditionType, RunningTotalField, Section, Sort,
    SummaryInfo, SummaryOperation, Table, TableJoinType, TableLink, TextObject, Twips, Unknown,
    Value,
};
use crate::records::{RecordStream, RecordTag};

// `Contents` stream record types.
const FIELD_DEF: u16 = 0x73; // a referenced field definition (name + value-type + length)
const FORMULA: u16 = 0x76; // a formula body (field refs + the formula body text)
const NAMED_VALUE: u16 = 0x71; // a named value; immediately follows a formula body to name it
const PRINTER: u16 = 0x03; // printer info (driver / name / port)
const PAGE_SETUP: u16 = 0x66; // page setup: the four page margins (BE u32 twips)
const PAPER_RECT: u16 = 0x018e; // the page rectangle: paper width + height (BE u32 twips)
const PAPER_DEVMODE: u16 = 0x0007; // page-setup DEVMODE: orientation / paper size / source
const AREA_MARKER: u16 = 0x8a; // an area, named e.g. "DetailArea1"
const SECTION_MARKER: u16 = 0x8c; // a section: Height (u32 BE twips) + Name

// Each report object is a flat run of records: an *opener* (text / field / shape / picture)
// followed by the *attribute* records that decorate it (name+size, position, format, border,
// font colour, font, and — for text objects — the literal text) until the next opener.
const TEXT_OBJECT: u16 = 0xa5; // opens a text object; byte 15 == 1 marks a field heading
const TEXT_OBJECT_FORMAT: u16 = 0xc0; // a text/heading object's paragraph format (alignment in byte 12)
const TEXT_CONTENT: u16 = 0xc2; // a text object's literal text content
const TEXT_EMBEDDED_FIELD: u16 = 0xc4; // an embedded field/formula/parameter reference in a text object
const FIELD_HEADING_LINK: u16 = 0x0166; // names the FieldObject a text object is the heading for
const FIELD_OBJECT: u16 = 0x9f; // opens a field object (its data-source reference)
const LINE_OBJECT: u16 = 0xa9; // opens a line/box drawing object (geometry distinguishes them)
const PICTURE_OBJECT: u16 = 0xae; // opens a picture/OLE object
const BLOB_FIELD_REF: u16 = 0xb1; // wraps a picture opener; its leaf holds the bound blob field ref
const SUBREPORT_OBJECT: u16 = 0xa3; // opens a subreport placeholder object
const CROSSTAB_OBJECT: u16 = 0xb8; // opens a cross-tab object (wrapped by 0xb9; parents the 0x9e name)
const CROSSTAB_WRAPPER: u16 = 0xb9; // wraps the 0xb8 cross-tab opener; starts the cross-tab binding block
const CHART_BINDING: u16 = 0xb4; // starts a chart's binding block (nests the chart's ObjectName)
const CHART_DATA: u16 = 0x7f; // wraps a chart's data ("show value") field ref (0x7e child)
const OBJECT_NAME: u16 = 0x9e; // an object's Name + Width/Height
const OBJECT_POS: u16 = 0xbe; // an object's Left/Top (u16 twips)
const OBJECT_FORMAT: u16 = 0xfc; // an object's format flags (horizontal alignment in byte 2)
const OBJECT_COND: u16 = 0xfd; // an object's conditional-format formula slot array
const OBJECT_BORDER: u16 = 0xec; // an object's border styles + border/background colours
const OBJECT_BORDER_COND: u16 = 0xed; // wrapper parenting `0xec`; carries border colour cond slots
const AREA_SECTION_FORMAT: u16 = 0xfe; // an area's or section's format flags (52-byte block)
const PARAM_RECORD: u16 = 0x007a; // a parameter field's detail record (XOR-0x7a obfuscated)
const SECTION_COND: u16 = 0xff; // a section's conditional-format formula slot array
const FONT_COLOR: u16 = 0x0100; // an object's font colour (COLORREF 0x00BBGGRR)
const FONT_COND: u16 = 0x0101; // an object's font conditional-format formula slot array
const FONT: u16 = 0x08; // an object's font (name + size + weight)
const GROUP: u16 = 0xe5; // a group: its condition field (+ "@Group #N Order")
const GROUP_OPTIONS: u16 = 0x88; // GroupAreaFormat of the group whose 0xe5 immediately follows it
const RECORD_SORT_FIELD: u16 = 0x29; // a record-level sort: field ref + direction (last byte)
const SUMMARY_DEF: u16 = 0x7e; // a summary/running-total def (operation byte + summarized field)
const RT_RESET: u16 = 0x80; // a running total's reset condition (precedes its 0x7e)
const REPORT_HEADER: u16 = 0x0064; // top-level report header (option bits: byte 24 bit 0 = save-data)
const SAVED_DATA: u16 = 0x0061; // saved-data block descriptor (present ⟺ ReportDocument.HasSavedData)
const MAX_STRING_BYTES: i32 = 65534; // Crystal's max string field length: 32767 chars × 2 bytes

// `QESession` (Query Engine) record types — the database/connection metadata.
const QE_CONNECTION: u16 = 0x02; // connection container (Database_DLL / type / database name)
const QE_TABLE: u16 = 0x03; // a table: name (+ alias), the SQL command text, and its fields
const QE_FIELD: u16 = 0x04; // a table data field: name + value-type code + length
const QE_TABLE_LINK: u16 = 0x0a; // a table link: src/dst field ids + join type

mod common;
mod data_def;
mod database;
mod dom;
mod parameters;
mod print_options;
mod report_def;

use common::*;
use data_def::*;
use database::*;
use dom::*;
pub(crate) use parameters::parse_report_parameters;
use parameters::*;
use print_options::*;
use report_def::*;

/// The 5-byte marker that begins the report-header's saved-environment trailer (`0x0064`),
/// immediately following the `EnableSavePreviewPicture` flag byte and preceding the
/// length-prefixed timezone string.
const PREVIEW_TRAILER_MARKER: [u8; 5] = [0x10, 0x01, 0x00, 0x00, 0x00];

/// Decode `EnableSavePreviewPicture` from a `0x0064` (report-header) leaf: the byte directly
/// before the saved-environment trailer marker. Returns `None` when the trailer is absent
/// (older/edge formats), in which case the caller keeps the thumbnail-presence fallback.
fn find_preview_flag(leaf: &[u8]) -> Option<bool> {
    leaf.windows(PREVIEW_TRAILER_MARKER.len())
        .position(|w| w == PREVIEW_TRAILER_MARKER)
        .filter(|&i| i >= 1)
        .map(|i| leaf[i - 1] != 0)
}

/// Project the `Contents` record stream (and report-level metadata) into a [`Report`].
///
/// Total by construction: every decoded record is reflected in the record inventory, in type order.
pub(crate) fn raise(
    contents: Option<&RecordStream>,
    qe: Option<&RecordStream>,
    prompt: Option<&RecordStream>,
    current_values: &BTreeMap<u16, Vec<ParameterValue>>,
    summary: Option<&SummaryInformation>,
) -> Report {
    let mut report = Report {
        summary_info: summary.map(raise_summary).unwrap_or_default(),
        // ReportOptions: EnableSaveSummariesWithReport is always True and EnableUseDummyData / the
        // initial-context strings are their defaults. EnableSaveDataWithReport and
        // EnableSavePreviewPicture are stored flags decoded from the report-header record (0x0064)
        // below. `save_preview_picture` is seeded here from the stored preview thumbnail as a fallback
        // for reports whose 0x0064 lacks the saved-environment trailer; the authoritative read
        // overrides it below.
        report_options: crate::model::ReportOptions {
            save_summaries_with_report: true,
            save_preview_picture: summary.is_some_and(|s| s.has_thumbnail),
            ..Default::default()
        },
        ..Report::default()
    };

    // The database — tables, the SQL command, connection info, and the full field schema — lives
    // in the separately-encrypted `QESession` (Query Engine) stream. It is decoded first so the
    // field schema is available when formulas are stale-checked below.
    if let Some(stream) = qe {
        report.database = raise_database(stream);
    }

    // Parameter detail records (`0x007a`), keyed by their `crobj://{…}` GUID — joined to the
    // PromptManager parameters below. Populated from the Contents tree.
    let mut param_records: BTreeMap<String, ParamRecord> = BTreeMap::new();

    if let Some(stream) = contents {
        if let Some(h) = stream.header() {
            report.version = h.version;
        }
        report.record_inventory = inventory(stream);
        report.records = raise_dom(stream);
        let tree = stream.record_tree();
        let logical = stream.logical_bytes();
        // Saved-data signals, read structurally (the saved rows themselves are never decoded):
        //  - `HasSavedData` ⟺ a saved-data block descriptor record (`SAVED_DATA` 0x0061) is present;
        //  - `EnableSaveDataWithReport` is bit 0 of leaf byte 24 of the report-header record (0x0064).
        for root in &tree {
            root.walk(&mut |n| {
                if n.rtype == SAVED_DATA {
                    report.has_saved_data = true;
                }
            });
            if root.rtype == REPORT_HEADER {
                let leaf = root.leaf_bytes(logical);
                if let Some(&b) = leaf.get(24) {
                    report.report_options.save_data_with_report = b & 0x01 != 0;
                }
                // EnableSavePreviewPicture (`SummaryInfo.IsSavingWithPreview`) is a stored design-time
                // flag, NOT merely "a preview thumbnail was written". It lives one byte before the
                // report's saved-environment trailer in the 0x0064 leaf: `[flag:u8] 10 01 00 00 00
                // [tz-string-len:u8][timezone string][locale]`. The trailer floats (the saved-data
                // descriptor ahead of it varies in size), so it is located by its fixed 5-byte marker
                // rather than a fixed offset. The flag is `01` when the option is on, `00` when off. The
                // thumbnail (OLE PID 0x11) is written alongside only when the report is rendered before
                // saving, so it is a lossy proxy — this flag is the source.
                if let Some(p) = find_preview_flag(&leaf) {
                    report.report_options.save_preview_picture = p;
                }
            }
        }
        // The set of live database fields (lowercase `alias.name`) — a formula referencing a field
        // not in this set no longer type-checks, so the engine reports it as UnknownField/0.
        let known_db_fields: std::collections::HashSet<String> = report
            .database
            .tables
            .iter()
            .flat_map(|t| {
                t.data_fields
                    .iter()
                    .map(move |f| format!("{}.{}", t.alias, f.name).to_lowercase())
            })
            .collect();
        // Field types (lowercase `alias.name` -> value type), for date-grouping condition decode.
        let field_types: std::collections::HashMap<String, crate::model::FieldValueType> = report
            .database
            .tables
            .iter()
            .flat_map(|t| {
                t.data_fields.iter().map(move |f| {
                    (
                        format!("{}.{}", t.alias, f.name).to_lowercase(),
                        f.value_type,
                    )
                })
            })
            .collect();
        report.data_definition =
            raise_data_definition(&tree, logical, &known_db_fields, &field_types);
        report.report_definition =
            raise_report_definition(&tree, logical, &report.data_definition.groups);
        report.print_options = raise_print_options(&tree, logical);
        for root in &tree {
            root.walk(&mut |n| {
                if n.rtype == PARAM_RECORD {
                    if let Some(r) = parse_param_leaf(&n.leaf_bytes(logical)) {
                        param_records.insert(r.guid.clone(), r);
                    }
                }
            });
        }
    }

    // With the database decoded, each display field object's value type is known, so headings left
    // at `DefaultAlign` over a `DefaultAlign` field can be resolved (numeric → right, else left).
    resolve_heading_alignment(&mut report);

    // Attach each group's decoded GroupAreaFormat to its GroupHeader area (both are in canonical
    // outermost-first order, so they zip 1:1).
    let group_formats: Vec<crate::model::GroupAreaFormat> = report
        .data_definition
        .groups
        .iter()
        .map(|g| g.area_format)
        .collect();
    let mut gi = 0;
    for area in &mut report.report_definition.areas {
        if area.kind == AreaSectionKind::GroupHeader {
            if let Some(gf) = group_formats.get(gi) {
                area.format.group = Some(*gf);
            }
            gi += 1;
        }
    }

    // Parameter field definitions live in the `PromptManager` stream (CRMetaObjects XML). Only the
    // stored properties are raised here; the derived `InUse`/`DataFetching` usage flags are an
    // aggregation computed in the export layer (see `rpt-to-xml`), like `Field.UseCount`.
    //
    // `HasCurrentValue` is True iff the parameter has a saved current value in the
    // `ReportParametersStream` (`!current_values.is_empty()` per param).
    if let Some(stream) = prompt {
        let params = crate::codec::decode_prompt_manager(stream.raw_bytes())
            .map(|xml| raise_parameters(&xml, &param_records, current_values))
            .unwrap_or_default();
        report.data_definition.field_definitions.extend(params);
    }

    report
}

#[cfg(test)]
mod tests {
    use super::find_preview_flag;

    // 0x0064 leaf tails: `… [flag] 10 01 00 00 00 [tz-len] <tz string>`. The marker floats (a
    // preceding variable-size saved-data descriptor shifts it), so the flag is found relative to the
    // marker, not at a fixed offset.
    fn leaf(flag: u8, lead: &[u8]) -> Vec<u8> {
        let mut v = lead.to_vec();
        v.push(flag);
        v.extend_from_slice(&[0x10, 0x01, 0x00, 0x00, 0x00]);
        v.extend_from_slice(b"\x15Eastern Standard Time,300");
        v
    }

    #[test]
    fn preview_flag_on_and_off() {
        assert_eq!(
            find_preview_flag(&leaf(0x01, &[0x40, 0, 0, 0, 0])),
            Some(true)
        );
        assert_eq!(
            find_preview_flag(&leaf(0x00, &[0x40, 0, 0, 0, 0])),
            Some(false)
        );
    }

    #[test]
    fn preview_flag_floats_with_leading_length() {
        // A longer lead (e.g. a populated saved-data descriptor) must not change the result.
        let long = vec![0xaa; 23];
        assert_eq!(find_preview_flag(&leaf(0x01, &long)), Some(true));
    }

    #[test]
    fn preview_flag_absent_marker_is_none() {
        assert_eq!(find_preview_flag(&[0x01, 0x02, 0x03, 0x04]), None);
    }
}
