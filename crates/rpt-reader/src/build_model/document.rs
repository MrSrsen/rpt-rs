//! What the report says about itself, rather than about its data or its layout: the authoring
//! version and options on the report root, the report-wide option bag, the re-import descriptor,
//! the designer's guides and connections, and the save-time metadata.
//!
//! These records are all document-level — at most one of each per report, all read from the roots
//! of the `Contents` tree — and together they fill the parts of the model that no domain submodule
//! owns.

use super::row_of;
use super::tree_search::nodes_where;
use crate::codec::RecordNode;
use crate::container::SummaryInformation;
use crate::field_table::table::Cell;
use crate::field_table::tables as ft;
use crate::model::{
    AuthoringVersion, DesignerState, Guideline, ObjectConnection, ReimportTimestamp, ReportOptions,
    SaveMetadataEntry, SubreportReimportInfo, SummaryInfo, Twips,
};
use crate::records::rtype::*;

/// What a report's document-level records say about it.
#[derive(Debug)]
pub(super) struct Document {
    /// The report options, seeded from [`default_options`] and overridden by what is stored.
    pub options: ReportOptions,
    /// The designer that wrote the report, as the report states it.
    pub authoring_version: AuthoringVersion,
    /// The report root's stored preview-picture flag, which the summary info reports as well.
    pub save_with_preview: bool,
    /// Whether a saved-data block descriptor is present anywhere in the tree.
    pub has_saved_data: bool,
    /// Where the report was imported from, when it was imported at all.
    pub reimport: Option<SubreportReimportInfo>,
    /// The design-surface guides and object connections.
    pub designer_state: DesignerState,
    /// The save-time environment key/value pairs, in stream order.
    pub save_metadata: Vec<SaveMetadataEntry>,
}

/// The report options that hold before any record is read.
///
/// `EnableSaveSummariesWithReport` is always True, and `EnableUseDummyData` / the initial-context
/// strings are their defaults. `EnableSavePreviewPicture` is seeded from the container's stored
/// preview thumbnail, which stands in for a report root that ends before the flag.
pub(super) fn default_options(has_thumbnail: bool) -> ReportOptions {
    ReportOptions {
        save_summaries_with_report: true,
        save_preview_picture: has_thumbnail,
        ..Default::default()
    }
}

/// Read every document-level record of a `Contents` tree.
pub(super) fn build_document(tree: &[RecordNode], logical: &[u8], has_thumbnail: bool) -> Document {
    let mut doc = Document {
        options: default_options(has_thumbnail),
        authoring_version: AuthoringVersion::default(),
        save_with_preview: false,
        has_saved_data: false,
        reimport: None,
        designer_state: decode_designer_state(tree, logical),
        save_metadata: nodes_where(tree, |n| n.rtype == SAVE_METADATA)
            .into_iter()
            .filter_map(|n| save_metadata_entry(n, logical))
            .collect(),
    };

    for root in tree {
        // `HasSavedData` ⟺ a saved-data block descriptor (`SAVED_DATA` 0x0061) is present; the
        // saved rows themselves are read elsewhere, on demand.
        root.walk(&mut |n| {
            if n.rtype == SAVED_DATA {
                doc.has_saved_data = true;
            }
        });
        if root.rtype == ft::REPORT_ROOT.rtype {
            read_report_root(&mut doc, root, logical);
        }
        // The report-wide option bag (`0x0160`, one per report) holds the two NULL-conversion
        // options, each a word of its own.
        if root.rtype == ft::REPORT_OPTIONS.rtype {
            let row = row_of(root, logical, &ft::REPORT_OPTIONS);
            doc.options.convert_null_field_to_default = row.i("convert_null_field_to_default") != 0;
            doc.options.convert_other_nulls_to_default =
                row.i("convert_other_nulls_to_default") != 0;
        }
    }

    // Subreport re-import provenance (`0x0142`, one per report): source `.rpt` path + import
    // timestamps. Structural — not on any output surface.
    doc.reimport = nodes_where(tree, |n| n.rtype == ft::SUBREPORT_REIMPORT_INFO.rtype)
        .first()
        .map(|n| decode_reimport_info(n, logical));
    doc
}

/// Read the report root (`0x0064`): the authoring version and the option word.
fn read_report_root(doc: &mut Document, root: &RecordNode, logical: &[u8]) {
    /// `EnableSaveDataWithReport`, bit 0 of the record's option word.
    const SAVE_DATA_WITH_REPORT: i32 = 0x01;
    /// `EnableVerifyOnEveryPrint`, bit 0 of the preview-picture word.
    const VERIFY_ON_EVERY_PRINT: i32 = 0x01;

    let row = row_of(root, logical, &ft::REPORT_ROOT);
    // The report's own statement of which designer wrote it: the record's first three fields,
    // ahead of everything else — major, minor, and a letter the engine renders as a character.
    doc.authoring_version = AuthoringVersion {
        major: row.u("major_version") as u16,
        minor: row.u("minor_version") as u16,
        letter: row.u("version_letter") as u8,
    };
    doc.options.save_data_with_report = row.i("options") & SAVE_DATA_WITH_REPORT != 0;
    // `EnableSavePreviewPicture` is a stored design-time flag, NOT merely "a preview thumbnail was
    // written": the thumbnail (OLE PID 0x11) is written alongside only when the report is rendered
    // before saving, so it is a lossy proxy and this word is the source. A record written before
    // the word existed simply ends short of it, and the thumbnail stands in.
    if let Some(v) = row.get("save_preview_picture").and_then(Cell::i) {
        doc.options.save_preview_picture = v != 0;
        // The same stored flag is the summary info's "saving with preview"; it is surfaced there
        // too, since that view is built from a stream that cannot see this record.
        doc.save_with_preview = v != 0;
        doc.options.enable_verify_on_every_print = v & VERIFY_ON_EVERY_PRINT != 0;
    }
}

/// Decode a `0x0142` `SubreportReimportInfo` record from its field table.
///
/// The two timestamps are kept in stored form — a Julian day and a same-day fraction each — and the
/// re-import policy is a narrowing enum the model carries as the byte it fits in.
fn decode_reimport_info(node: &RecordNode, logical: &[u8]) -> SubreportReimportInfo {
    let row = row_of(node, logical, &ft::SUBREPORT_REIMPORT_INFO);
    let stamp = |day: &str, fraction: &str| ReimportTimestamp {
        julian_day: row.i(day) as u32,
        time_fraction: row.i(fraction) as u32,
    };
    SubreportReimportInfo {
        source_path: row.text("source_path").to_owned(),
        imported_at: stamp("imported_at_julian_day", "imported_at_time_fraction"),
        reimport_when_opening: row.u("reimport_when_opening") as u8,
        source_saved_at: stamp(
            "source_saved_at_julian_day",
            "source_saved_at_time_fraction",
        ),
    }
}

/// Decode the designer/IDE geometry: the `0x010c` snap guidelines and `0x0111` object-connection
/// edges scattered across the `Contents` tree. Structural.
fn decode_designer_state(tree: &[RecordNode], logical: &[u8]) -> DesignerState {
    let guidelines = nodes_where(tree, |n| n.rtype == ft::GUIDELINE_ENTRY.rtype)
        .into_iter()
        .map(|n| {
            let row = row_of(n, logical, &ft::GUIDELINE_ENTRY);
            Guideline {
                position: Twips(row.i("position")),
                // The model's `flags` is the guide's object-connection count.
                flags: row.u("connection_count") as u16,
            }
        })
        .collect();
    let connections = nodes_where(tree, |n| n.rtype == ft::OBJECT_CONNECTION.rtype)
        .into_iter()
        .map(|n| {
            let row = row_of(n, logical, &ft::OBJECT_CONNECTION);
            ObjectConnection {
                // The two words of the connected object's identifier, and the two attachment-state
                // codes the model carries as one word, high byte first.
                source: row.i("object_kind") as u16,
                destination: row.i("object_index") as u16,
                kind: ((row.u("_u2") << 8) | row.u("_u3")) as u16,
            }
        })
        .collect();
    DesignerState {
        guidelines,
        connections,
    }
}

/// Build the document [`SummaryInfo`] from the container's OLE `SummaryInformation` property set.
pub(super) fn build_summary_info(si: &SummaryInformation) -> SummaryInfo {
    SummaryInfo {
        title: si.title.clone().unwrap_or_default(),
        subject: si.subject.clone().unwrap_or_default(),
        author: si.author.clone().unwrap_or_default(),
        keywords: si.keywords.clone().unwrap_or_default(),
        comments: si.comments.clone().unwrap_or_default(),
        // Authoring provenance from the OLE property set: `RevisionNumber` (PID 0x09, kept in the
        // engine's stored string form) and `LastSavedBy` (PID 0x08, `PID_LAST_AUTHOR`).
        revision_number: si.revision_number.clone().unwrap_or_default(),
        last_saved_by: si.last_author.clone().unwrap_or_default(),
        created: si.created,
        last_saved: si.last_saved,
        last_printed: si.last_printed,
        ..Default::default()
    }
}

/// One `0x0178 SaveMetadata` record: a key and its value. A record with no key names nothing and is
/// not an entry.
fn save_metadata_entry(node: &RecordNode, logical: &[u8]) -> Option<SaveMetadataEntry> {
    let row = row_of(node, logical, &ft::SAVE_METADATA);
    let key = row.text("key");
    (!key.is_empty()).then(|| SaveMetadataEntry {
        key: key.to_owned(),
        value: row.text("value").to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::{decode_reimport_info, save_metadata_entry, RecordNode};

    /// A childless record of `rtype` over a synthetic run, headed by its own declaration of the
    /// enhanced string form.
    fn record(rtype: u16, joined: &[u8]) -> (Vec<u8>, RecordNode) {
        let mut logical = vec![0u8; 8];
        logical[0] = crate::build_model::enhanced_header_byte(rtype, 0);
        let start = logical.len();
        logical.extend_from_slice(joined);
        let end = logical.len();
        (
            logical,
            RecordNode {
                rtype,
                schema: 0x0700,
                offset: 0,
                content_start: start,
                content_end: end,
                mask: 0,
                children: Vec::new(),
            },
        )
    }

    /// A `0x0178` record over a synthetic run.
    fn save_metadata(joined: &[u8]) -> (Vec<u8>, RecordNode) {
        record(super::SAVE_METADATA, joined)
    }

    /// A `0x0142` record over a synthetic run.
    fn reimport(joined: &[u8]) -> (Vec<u8>, RecordNode) {
        record(super::ft::SUBREPORT_REIMPORT_INFO.rtype, joined)
    }

    /// A save-time value is a stored fact whatever bytes it holds: it is framed by its own length,
    /// so it needs no validation as clean text, and rejecting it as such loses it.
    #[test]
    fn a_value_that_is_not_clean_text_is_still_a_value() {
        let mut joined = vec![0, 0, 0, 5];
        joined.extend_from_slice(b"host\0");
        joined.extend_from_slice(&[0, 0, 0, 4]);
        joined.extend_from_slice(&[b'P', 0xe9, b'C', 0]); // not valid UTF-8
        let (logical, node) = save_metadata(&joined);
        let e = save_metadata_entry(&node, &logical).expect("a keyed record is an entry");
        assert_eq!(e.key, "host");
        assert_eq!(e.value.chars().count(), 3);
        assert!(e.value.starts_with('P') && e.value.ends_with('C'));
    }

    /// A record with no key names nothing.
    #[test]
    fn a_record_with_no_key_is_not_an_entry() {
        let (logical, node) = save_metadata(&[0, 0, 0, 1, 0, 0, 0, 0, 1, 0]);
        assert!(save_metadata_entry(&node, &logical).is_none());
    }

    /// A re-import descriptor with no path: the string is its terminator alone, and the two
    /// timestamps and the policy enum follow it.
    #[test]
    fn reimport_info_empty_path_and_timestamps() {
        let mut joined = vec![0, 0, 0, 1, 0]; // L=1, single NUL path
        joined.extend_from_slice(&2_454_607u32.to_be_bytes()); // imported_at JDN
        joined.extend_from_slice(&57_007u32.to_be_bytes()); // imported_at fraction
        joined.push(1); // reimport enum
        joined.extend_from_slice(&0u32.to_be_bytes()); // source_saved_at JDN
        joined.extend_from_slice(&0u32.to_be_bytes()); // source_saved_at fraction
        let (logical, node) = reimport(&joined);
        let ri = decode_reimport_info(&node, &logical);
        assert_eq!(ri.source_path, "");
        assert_eq!(ri.imported_at.julian_day, 2_454_607);
        assert_eq!(ri.imported_at.time_fraction, 57_007);
        assert_eq!(ri.reimport_when_opening, 1);
        assert_eq!(ri.source_saved_at.julian_day, 0);
    }

    /// A stored path moves every field after it, so the timestamps are read where the path ends
    /// rather than at the offset the empty form puts them at.
    #[test]
    fn a_stored_path_moves_the_fields_after_it() {
        let path = b"C:\\r.rpt";
        let mut joined = ((path.len() + 1) as u32).to_be_bytes().to_vec();
        joined.extend_from_slice(path);
        joined.push(0);
        joined.extend_from_slice(&7u32.to_be_bytes()); // imported_at JDN
        joined.extend_from_slice(&[0; 4]);
        joined.push(1); // reimport enum
        joined.extend_from_slice(&[0; 8]);
        let (logical, node) = reimport(&joined);
        let ri = decode_reimport_info(&node, &logical);
        assert_eq!(ri.source_path, "C:\\r.rpt");
        assert_eq!(ri.imported_at.julian_day, 7);
        assert_eq!(ri.reimport_when_opening, 1);
    }
}
