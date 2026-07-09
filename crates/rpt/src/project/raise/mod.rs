//! Read path — build the semantic [`Report`] from the decoded record substrate.

use std::collections::BTreeMap;

use crate::codec::RecordNode;
use crate::container::SummaryInformation;
use crate::model::{
    Alignment, Area, AreaSectionKind, ChartDefinition, ChartGraphType, ChartGridType,
    ChartLegendPosition, Color, ConnectionInfo, DataDefinition, Database, DbField, DbFieldDef,
    FieldDef, FieldKindData, FieldObject, FieldRefKind, FieldValueType, Font, FontColor, Formula,
    FormulaField, FormulaVariable, FormulaVariableScope, Group, Hyperlink, HyperlinkType,
    LineStyle, Node, ParameterField, ParameterValue, ParameterValueKind, PrintOptions,
    RecordTypeCount, Rect, Report, ReportDefinition, ReportObject, ReportObjectKind,
    ResetConditionType, RunningTotalField, SaveMetadataEntry, Section, Sort, SummaryInfo,
    SummaryOperation, Table, TableJoinType, TableLink, TextObject, Twips, Unknown, Value,
};
use crate::records::{RecordStream, RecordTag};

// Every record-type `u16` is named once in `records::rtype`; glob-import them so this module and
// its submodules (via `use super::*`) classify records by name.
pub(crate) use crate::records::rtype::*;

const MAX_STRING_BYTES: i32 = 65534; // Crystal's max string field length: 32767 chars × 2 bytes

mod common;
mod data_def;
mod database;
mod dom;
mod parameters;
mod print_options;
mod report_def;
mod subreport;

use common::*;
use data_def::*;
use database::*;
use dom::*;
pub(crate) use parameters::parse_report_parameters;
use parameters::*;
use print_options::*;
use report_def::*;
pub(crate) use subreport::{
    resolve_sf_handle, subreport_link_bindings, subreport_links, subreport_param_index_names,
};

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

/// A `(Julian-day, time-fraction)` timestamp pair inside a `0x0142` re-import descriptor: two
/// big-endian `u32`s.
fn read_reimport_timestamp(b: &[u8], off: usize) -> crate::model::ReimportTimestamp {
    crate::model::ReimportTimestamp {
        julian_day: u32_be(b, off).unwrap_or(0),
        time_fraction: u32_be(b, off + 4).unwrap_or(0),
    }
}

/// Decode a `0x0142` `SubreportReimportInfo` leaf:
/// `[u32 BE L][source path: L bytes incl NUL][imported_at: 2×u32][enum 1B][source_saved_at: 2×u32]`.
/// The path is empty (`L == 1`) across the corpus; the trailer is a fixed 17 bytes.
fn decode_reimport_info(leaf: &[u8]) -> crate::model::SubreportReimportInfo {
    let path_len = u32_be(leaf, 0).unwrap_or(0) as usize;
    let path_end = 4 + path_len;
    // The stored path includes a trailing NUL; strip it (and any trailing NULs) for the model.
    let source_path = leaf
        .get(4..path_end)
        .map(|p| {
            String::from_utf8_lossy(p)
                .trim_end_matches('\0')
                .to_string()
        })
        .unwrap_or_default();
    crate::model::SubreportReimportInfo {
        source_path,
        imported_at: read_reimport_timestamp(leaf, path_end),
        reimport_when_opening: leaf.get(path_end + 8).copied().unwrap_or(0),
        source_saved_at: read_reimport_timestamp(leaf, path_end + 9),
    }
}

/// Decode the designer/IDE geometry: the `0x010c` snap guidelines and `0x0111` object-connection
/// edges scattered across the `Contents` tree. Structural.
fn decode_designer_state(tree: &[RecordNode], logical: &[u8]) -> crate::model::DesignerState {
    let guidelines = nodes_where(tree, |n| n.rtype == GUIDELINE_ENTRY)
        .into_iter()
        .map(|n| {
            let leaf = n.leaf_bytes(logical);
            crate::model::Guideline {
                position: Twips(u32_be(&leaf, 0).unwrap_or(0) as i32),
                flags: u16_be(&leaf, 4).unwrap_or(0),
            }
        })
        .collect();
    let connections = nodes_where(tree, |n| n.rtype == OBJECT_CONNECTION)
        .into_iter()
        .map(|n| {
            let leaf = n.leaf_bytes(logical);
            crate::model::ObjectConnection {
                source: u16_be(&leaf, 0).unwrap_or(0),
                destination: u16_be(&leaf, 2).unwrap_or(0),
                kind: u16_be(&leaf, 12).unwrap_or(0),
            }
        })
        .collect();
    crate::model::DesignerState {
        guidelines,
        connections,
    }
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
    // GUID-less `0x007a` records — parameters referenced only by a formula, with no PromptManager
    // entry to join to. Synthesized into `ParameterField`s after the PromptManager join below.
    let mut orphan_params: Vec<ParamRecord> = Vec::new();

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
        // Save-time environment metadata (`0x0178`): each record's leaf is a length-prefixed
        // key/value string pair (`read_lp_string` reads the key, then the value from just past it).
        // Kept in stream order so per-save groups stay together.
        report.save_metadata = nodes_where(&tree, |n| n.rtype == SAVE_METADATA)
            .into_iter()
            .filter_map(|n| {
                let leaf = n.leaf_bytes(logical);
                let (key, consumed) = read_lp_string(&leaf)?;
                let value = read_lp_string(&leaf[consumed..])
                    .map(|(v, _)| v)
                    .unwrap_or_default();
                Some(SaveMetadataEntry { key, value })
            })
            .collect();
        for n in nodes_where(&tree, |n| n.rtype == PARAM_RECORD) {
            if let Some(r) = parse_param_leaf(&n.leaf_bytes(logical)) {
                match &r.guid {
                    Some(guid) => {
                        param_records.insert(guid.clone(), r);
                    }
                    None => orphan_params.push(r),
                }
            }
        }
        // Subreport re-import provenance (`0x0142`, one per report): source `.rpt` path + import
        // timestamps. Structural — not on the XML surface.
        report.reimport = nodes_where(&tree, |n| n.rtype == REIMPORT_INFO)
            .first()
            .map(|n| decode_reimport_info(&n.leaf_bytes(logical)));
        // Designer/IDE state (`0x010c` guidelines + `0x0111` object connections). Structural.
        report.designer_state = decode_designer_state(&tree, logical);
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
    // aggregation computed in the export layer (see `rpt xml-dump`), like `Field.UseCount`.
    //
    // `HasCurrentValue` is True iff the parameter has a saved current value in the
    // `ReportParametersStream` (`!current_values.is_empty()` per param).
    if let Some(stream) = prompt {
        let params = crate::codec::decode_prompt_manager(stream.raw_bytes())
            .map(|xml| raise_parameters(&xml, &param_records, current_values))
            .unwrap_or_default();
        report.data_definition.field_definitions.extend(params);
    }

    // GUID-less parameters (used only in a formula, absent from the PromptManager) are synthesized
    // directly. Skip any whose name was already emitted from the PromptManager, so a joined parameter
    // is never duplicated.
    let existing_param_names: std::collections::HashSet<String> = report
        .data_definition
        .field_definitions
        .iter()
        .filter(|f| matches!(&f.kind, FieldKindData::Parameter(_)))
        .map(|f| f.name.clone())
        .collect();
    for rec in &orphan_params {
        if let Some(fd) = raise_orphan_param(rec) {
            if !existing_param_names.contains(&fd.name) {
                report.data_definition.field_definitions.push(fd);
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::{decode_reimport_info, find_preview_flag};

    // A `0x0142` leaf: empty path (`L == 1`, lone NUL) + 17-byte trailer of two `(JDN, fraction)`
    // timestamps around a 1-byte re-import enum.
    #[test]
    fn reimport_info_empty_path_and_timestamps() {
        let mut leaf = vec![0, 0, 0, 1, 0]; // L=1, single NUL path
        leaf.extend_from_slice(&2_454_607u32.to_be_bytes()); // imported_at JDN
        leaf.extend_from_slice(&57_007u32.to_be_bytes()); // imported_at fraction
        leaf.push(1); // reimport enum
        leaf.extend_from_slice(&0u32.to_be_bytes()); // source_saved_at JDN
        leaf.extend_from_slice(&0u32.to_be_bytes()); // source_saved_at fraction
        let ri = decode_reimport_info(&leaf);
        assert_eq!(ri.source_path, "");
        assert_eq!(ri.imported_at.julian_day, 2_454_607);
        assert_eq!(ri.imported_at.time_fraction, 57_007);
        assert_eq!(ri.reimport_when_opening, 1);
        assert_eq!(ri.source_saved_at.julian_day, 0);
    }

    #[test]
    fn reimport_info_with_source_path() {
        // A populated path: L includes the trailing NUL.
        let path = b"C:\\r.rpt";
        let mut leaf = ((path.len() + 1) as u32).to_be_bytes().to_vec();
        leaf.extend_from_slice(path);
        leaf.push(0);
        leaf.extend_from_slice(&[0u8; 17]); // trailer
        let ri = decode_reimport_info(&leaf);
        assert_eq!(ri.source_path, "C:\\r.rpt");
    }

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
