//! Build the semantic [`Report`] from a report's decoded records.
//!
//! This is the read half of the reader's top layer: each submodule walks the record tree for the
//! records of one domain and fills in the matching part of the model. It never drops a record —
//! anything unmodelled stays in the decoded records, reachable through the typed record tree
//! ([`build_typed_record_tree`]) and the record-type inventory ([`build_inventory`]).
//!
//! Building the model cannot fail: a record this reader does not understand yields a default, not
//! an error (see [`crate::error`]). What was missed is reported as decode coverage instead.
//!
//! Every record is read through its own field table ([`crate::field_table`]): a decoder names the
//! fields it wants and the table says where they sit, so no module here addresses bytes itself.
//! Three of the model's inputs are not records and so are read some other way — the CRMetaObjects
//! XML of the `PromptManager` stream and of a saved current value, which [`parameters`] reads as
//! text; the OLE embeddings behind pictures and embedded objects ([`picture`], [`embeds`]); and the
//! saved rows, which [`crate::codec::decode_saved_rows`] decodes once [`saved`] has told it each
//! column's type.

use crate::codec::{Dialect, RecordNode};
use crate::container::SummaryInformation;
use crate::field_table::table::{read_strings, Row, Table as FieldTable};
use crate::field_table::tables as ft;
use crate::field_table::{content_of, strings_format_of};
use crate::model::{AreaSectionKind, FieldKindData, ParameterValue, Report};
use crate::records::RecordStream;
use std::collections::BTreeMap;

const MAX_STRING_BYTES: i32 = 65534; // Crystal's max string field length: 32767 chars × 2 bytes

mod data_def;
mod database;
mod document;
mod embeds;
mod parameters;
mod picture;
mod print_options;
mod record_values;
mod report_def;
mod saved;
mod saved_catalog;
mod subreport;
mod tree_search;
mod typed_tree;

use data_def::build_data_definition;
use database::build_database;
use document::{build_document, build_summary_info, default_options};
use parameters::{build_orphan_param, build_parameters, parse_param_record, ParamRecord};
use print_options::build_print_options;
use report_def::build_report_definition;
use tree_search::nodes_where;
// The reserved condition-formula names and the slot reading built on them, re-exported for the
// field tables' own harness.
#[cfg(test)]
pub(crate) use report_def::{condition_slots, is_modeled_condition};

pub(crate) use embeds::build_embeds;
pub(crate) use parameters::parse_report_parameters;
pub(crate) use picture::attach_pictures;
// The per-record decoders, exposed to the crate so `annotate` can summarize a raw record with the
// same reading the model is built from.
pub(crate) use print_options::decode_devmode;
pub(crate) use report_def::{
    decode_boolean_format, decode_common_format, decode_date_format, decode_datetime_format,
    decode_numeric_format, decode_string_format, decode_time_format, field_format_table,
    special_field_name,
};
pub(crate) use saved::decode_saved_data;
pub(crate) use saved_catalog::read_catalog;
pub(crate) use subreport::{attach as attach_subreports, build_subreports};
pub(crate) use typed_tree::{build_inventory, build_typed_record_tree};

/// The row a record's own field table reads from it, framed as the record's header declares.
///
/// This is the one way this layer reads a record. The node carries both halves — its content and
/// the string wire form that content is framed in — so no decoder here picks a framing for a record
/// it did not write. A *synthetic* record, one this layer builds rather than reads, has no header to
/// ask and is read through [`read_strings`] with the form its builder chose.
pub(crate) fn row_of(node: &RecordNode, logical: &[u8], table: &FieldTable) -> Row {
    let strings = strings_format_of(node, logical);
    read_strings(table, &content_of(node, logical), strings).row
}

/// The first header byte of a hand-built record, declaring the enhanced (length-prefixed) string
/// form — the only form the record-tree reader admits a header for, and so the form every record
/// reaching this layer is framed in. [`row_of`] takes the framing from the header, so a synthetic
/// record states it there rather than leaving it to the record type's own low byte. The byte is
/// masked as a header is, one level out from the content.
#[cfg(test)]
pub(crate) fn enhanced_header_byte(rtype: u16, mask: u8) -> u8 {
    const STRINGS_ENHANCED: u8 = 0x10;
    STRINGS_ENHANCED ^ mask ^ (rtype as u8)
}

/// Project the `Contents` record stream (and report-level metadata) into a [`Report`].
///
/// The typed record tree and its per-type inventory are not built here — they are built on demand
/// from the decoded records (see [`build_typed_record_tree`]/[`build_inventory`] and
/// [`crate::Rpt::typed_record_tree`]).
pub(crate) fn build_report(
    contents: Option<&RecordStream>,
    qe: Option<&RecordStream>,
    prompt: Option<&RecordStream>,
    current_values: &BTreeMap<u16, Vec<ParameterValue>>,
    summary: Option<&SummaryInformation>,
) -> Report {
    let has_thumbnail = summary.is_some_and(|s| s.has_thumbnail);
    let mut report = Report {
        summary_info: summary.map(build_summary_info).unwrap_or_default(),
        report_options: default_options(has_thumbnail),
        ..Report::default()
    };

    // The database — tables, the SQL command, connection info, and the full field schema — lives
    // in the separately-encrypted `QESession` (Query Engine) stream. It is decoded first so the
    // field schema is available to the record-tree passes below.
    if let Some(stream) = qe {
        report.database = build_database(stream);
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
        let tree = stream.record_tree_in(Dialect::Contents);
        let logical = stream.logical_bytes();
        // What the report says about itself: the report root and the report-wide option bag, the
        // re-import descriptor, designer state, save metadata, and whether saved data is present.
        let doc = build_document(&tree, logical, has_thumbnail);
        report.report_options = doc.options;
        report.authoring_version = doc.authoring_version;
        report.summary_info.save_with_preview = doc.save_with_preview;
        report.has_saved_data = doc.has_saved_data;
        report.reimport = doc.reimport;
        report.designer_state = doc.designer_state;
        report.save_metadata = doc.save_metadata;
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
        report.data_definition = build_data_definition(&tree, logical, &field_types);
        report.report_definition =
            build_report_definition(&tree, logical, &report.data_definition.groups, &field_types);
        report.print_options = build_print_options(&tree, logical);
        for n in nodes_where(&tree, |n| n.rtype == ft::PARAMETER_RECORD.rtype) {
            let r = parse_param_record(n, logical);
            match &r.guid {
                Some(guid) => {
                    param_records.insert(guid.clone(), r);
                }
                None => orphan_params.push(r),
            }
        }
    }

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
    // stored properties are raised here; the engine's `InUse`/`DataFetching` usage flags are an
    // aggregation over the whole report, not stored, and rpt-rs does not report them.
    //
    // `HasCurrentValue` is True iff the parameter has a saved current value in the
    // `ReportParametersStream` (`!current_values.is_empty()` per param).
    if let Some(stream) = prompt {
        let params = crate::codec::decode_prompt_manager(stream.raw_bytes())
            .map(|xml| build_parameters(&xml, &param_records, current_values))
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
        if let Some(fd) = build_orphan_param(rec) {
            if !existing_param_names.contains(&fd.name) {
                report.data_definition.field_definitions.push(fd);
            }
        }
    }

    report
}
