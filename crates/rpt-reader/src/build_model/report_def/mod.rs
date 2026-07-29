//! Report definition — the area/section/object layout tree. This module root holds the record
//! grammar ([`RdRecord`]) and the entry point ([`build_report_definition`]) that drives it; the
//! submodules own the parsing detail:
//!
//! - [`walk`] — the ordered walk itself: the cursor into the tree being built and one handler per
//!   record that writes into it.
//! - [`sections`] — area/section construction and the canonical band ordering.
//! - [`objects`] — object cursors and the post-walk object-tree transforms.
//! - [`summary`] — the non-running-total summary definitions a Summary object indexes.
//! - [`bindings`] — the shared chart / cross-tab binding-scope walk, and the pass that hands each
//!   decoded object to the file named for its record family.
//! - [`chart`] / [`crosstab`] — chart definitions and cross-tab dimensions + grid formatting, each
//!   including the assembly of its own object kind.
//! - [`formats`] (per-object/area/section format records), [`conditions`] (conditional-format
//!   formula slots), [`data_source`] (field/object reference text and the field object built on it).

use crate::build_model::row_of;
use crate::build_model::tree_search::{flatten, nodes_where};
use crate::codec::RecordNode;
use crate::field_table::tables as ft;
use crate::model::{AreaSectionKind, Group, ReportDefinition};
use crate::records::rtype::*;

mod bindings;
mod chart;
mod conditions;
mod crosstab;
pub(super) mod data_source;
mod formats;
mod objects;
mod sections;
mod summary;
mod walk;

use bindings::{attach_grid_bindings, collect_chart_object_names};
use formats::is_field_format_wrapper;
use objects::{demote_orphan_headings, reclassify_picture_openers};
use sections::sort_areas_canonical;
use walk::RdWalk;

// The reserved condition-formula names are the vocabulary a condition slot is recognised by, and
// `condition_slots` is the reading of a wrapper's slots in that vocabulary; the field tables' own
// harness checks both against the byte scan they replace.
#[cfg(test)]
pub(crate) use conditions::{condition_slots, is_modeled_condition};
// The per-object format-record decoders, the field-format family's own table lookup, and the
// special-field kind names, raised to the parent `build_model` as the crate's reading of those
// records.
pub(crate) use data_source::special_field_name;
pub(crate) use formats::{
    decode_boolean_format, decode_common_format, decode_date_format, decode_datetime_format,
    decode_numeric_format, decode_string_format, decode_time_format, field_format_table,
};

/// A record's role in the `ReportDefinition` stream. The stream is a flat, ordered sequence:
/// area/section markers delimit the layout tree, an *opener* starts a report object, and the
/// *attribute* records that follow decorate the most-recently-opened object until the next opener.
pub(super) enum RdRecord {
    /// `0x8a` — opens an area (`ReportHeaderArea1`, `DetailArea1`, …).
    Area,
    /// `0x8c` — opens a section within the current area.
    Section,
    /// A band marker (`0x8d`/`0x8f`/`0x91`/`0x93`/`0x95`/`0x97`/`0x99`) — parents the following
    /// `0x8c` section and authoritatively names its band kind, since the area/section *name* is
    /// user-renameable (a group band is commonly named after its group field, e.g. `nameHeader`).
    Band(AreaSectionKind),
    /// `0xa5` — opens a text object. Whether it is a field-heading object is only known once (and
    /// if) its [`RdRecord::FieldHeadingLink`] record is seen, so it always opens as a text object.
    OpenText,
    /// `0x0166` — names the FieldObject this text object is the heading for (promotes it to a
    /// FieldHeadingObject). A plain text object has no such record.
    FieldHeadingLink,
    /// `0x9f` — opens a field object.
    OpenField,
    /// `0xa9` — opens a drawing object (line or box, told apart by geometry).
    OpenShape,
    /// `0xae` — opens a picture/OLE object.
    OpenPicture,
    /// `0xb1` — wraps the picture opener of a blob-field object, and names the database field the
    /// picture comes from. The opener is its first child.
    BlobFieldRef,
    /// `0xbd` — decorates the just-opened static/OLE picture; bytes `[0..4]` (big-endian) are the
    /// 1-based `Embedding N` storage ordinal whose `CONTENTS` stream holds the image bytes.
    OleObjectItem,
    /// `0xa3` — opens a subreport placeholder object.
    OpenSubreport,
    /// `0xb8` — opens a cross-tab object (wrapped by `0xb9`; the `0x9e` name nests inside it).
    OpenCrossTab,
    /// `0x9c` — the current area's `SectionCodeHeaderFooter`, directly parenting the `0x9b`
    /// `SectionCodeAreaType` record that carries the area's group nesting level (byte 2, for a group
    /// area). Streams right after the area marker, so it decorates the most-recently-opened area.
    SectionCodeHeaderFooter,
    /// `0x9e` — the current object's Name + Width/Height.
    Name,
    /// `0xbe` — the current object's Left/Top.
    Position,
    /// `0xfc` — the current object's format flags (horizontal alignment).
    Format,
    /// `0xfd` — the current object's conditional-format formula slot array.
    ObjectCondition,
    /// `0xfe` — the current area's or section's format flags (the record states which).
    AreaSectionFormat,
    /// `0xff` — the current section's conditional-format formula slot array.
    SectionCondition,
    /// `0x0101` — the current object's font conditional-format formula slot array.
    FontCondition,
    /// `0xec` — the current object's border styles + border/background colors.
    Border,
    /// `0xed` — wrapper parenting `0xec`; carries the border's color condition-formula slots
    /// (`@Fore_Color` → BorderColor, `@Back_Color` → BackgroundColor). Visited before its `0xec`
    /// child, so it stashes pending conditions that the `Border` arm attaches after rebuilding.
    BorderCondition,
    /// `0x0100` — the current object's font color.
    FontColor,
    /// `0x08` — the current object's font.
    Font,
    /// `0xc0` — a text/heading object's paragraph format (the authoritative horizontal alignment).
    TextObjectFormat,
    /// `0xc4` — an embedded field/formula/parameter reference inside the current text object.
    EmbeddedField,
    /// `0xc2` — the current text object's literal text.
    TextContent,
    /// One of the typed field-format wrappers named by [`FIELD_FORMAT_RECORDS`]. Each parents its
    /// value child, and those children populate the current field object's `FieldFormat`.
    FieldFormatBlock,
    /// Any record not part of the object layout.
    Other,
}

impl RdRecord {
    /// Classify a record by its type.
    fn classify(node: &RecordNode) -> RdRecord {
        match node.rtype {
            AREA => RdRecord::Area,
            SECTION => RdRecord::Section,
            REPORT_HEADER_BAND => RdRecord::Band(AreaSectionKind::ReportHeader),
            REPORT_FOOTER_BAND => RdRecord::Band(AreaSectionKind::ReportFooter),
            PAGE_HEADER_BAND => RdRecord::Band(AreaSectionKind::PageHeader),
            PAGE_FOOTER_BAND => RdRecord::Band(AreaSectionKind::PageFooter),
            DETAIL_BAND => RdRecord::Band(AreaSectionKind::Detail),
            GROUP_HEADER_BAND => RdRecord::Band(AreaSectionKind::GroupHeader),
            GROUP_FOOTER_BAND => RdRecord::Band(AreaSectionKind::GroupFooter),
            TEXT_OBJECT_CONTAINER => RdRecord::OpenText,
            FIELD_HEADING_LINK => RdRecord::FieldHeadingLink,
            FIELD_OBJECT => RdRecord::OpenField,
            DRAWING_OBJECT => RdRecord::OpenShape,
            PICTURE_OBJECT => RdRecord::OpenPicture,
            BLOB_FIELD_WRAPPER => RdRecord::BlobFieldRef,
            OLE_OBJECT_ITEM => RdRecord::OleObjectItem,
            SUBREPORT_OBJECT => RdRecord::OpenSubreport,
            CROSSTAB_OBJECT => RdRecord::OpenCrossTab,
            SECTION_CODE_HEADER_FOOTER => RdRecord::SectionCodeHeaderFooter,
            OBJECT_NAME => RdRecord::Name,
            OBJECT_POSITION => RdRecord::Position,
            OBJECT_FORMAT => RdRecord::Format,
            OBJECT_FORMAT_WRAPPER => RdRecord::ObjectCondition,
            AREA_SECTION_FORMAT => RdRecord::AreaSectionFormat,
            SECTION_FORMAT_WRAPPER => RdRecord::SectionCondition,
            FONT_CONDITION_FORMAT => RdRecord::FontCondition,
            BORDER => RdRecord::Border,
            BORDER_WRAPPER => RdRecord::BorderCondition,
            FONT_COLOR => RdRecord::FontColor,
            FONT => RdRecord::Font,
            TEXT_OBJECT_FORMAT => RdRecord::TextObjectFormat,
            TEXT_EMBEDDED_FIELD => RdRecord::EmbeddedField,
            TEXT_OBJECT => RdRecord::TextContent,
            rt if is_field_format_wrapper(rt) => RdRecord::FieldFormatBlock,
            _ => RdRecord::Other,
        }
    }

    /// Whether this record opens a report object, ending the previous object's run of attribute
    /// records and starting its own.
    fn opens_object(&self) -> bool {
        matches!(
            self,
            RdRecord::OpenText
                | RdRecord::OpenField
                | RdRecord::OpenShape
                | RdRecord::OpenPicture
                | RdRecord::OpenSubreport
                | RdRecord::OpenCrossTab
        )
    }
}

/// The kind of a `0xa9` drawing object, taken from byte 25 of its `0xec` border record: `1` = box,
/// `2` = line — the only two values in use, so the set is closed. An ellipse/oval is not a third
/// kind but an `IBoxObject` (byte 25 = `1`) whose corner-ellipse (bytes 26-29) rounds the box fully,
/// and a rounded rectangle is the same box with a partial corner-ellipse. `Other` is a fallback for
/// any unobserved value, treated as the `0xa9` default: a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrawingShapeKind {
    Box,
    Line,
    Other(u8),
}

impl DrawingShapeKind {
    fn from_byte(b: u8) -> Self {
        match b {
            1 => DrawingShapeKind::Box,
            2 => DrawingShapeKind::Line,
            other => DrawingShapeKind::Other(other),
        }
    }
}

/// SDK `ReportDefinition`: the area / section / object layout tree. The flat record stream is fed to
/// an [`RdWalk`] in document order — the order is part of the grammar, see [`RdRecord`] — and the
/// transforms that need the finished tree run after it.
pub(super) fn build_report_definition(
    tree: &[RecordNode],
    logical: &[u8],
    groups: &[Group],
    field_types: &std::collections::HashMap<String, crate::model::FieldValueType>,
) -> ReportDefinition {
    let mut walk = RdWalk::new(tree, logical, groups);
    for node in flatten(tree) {
        walk.feed(node);
    }
    let mut areas = walk.into_areas();
    demote_orphan_headings(&mut areas);
    reclassify_picture_openers(&mut areas, &collect_chart_object_names(tree, logical));
    attach_grid_bindings(tree, logical, &mut areas, groups, field_types);
    sort_areas_canonical(&mut areas);
    let (kind, style) = report_kind_and_style(tree, logical);
    ReportDefinition { kind, style, areas }
}

/// The report-level `ReportKind` (SDK `CRReportKind`) and `ReportStyle`, from the page-setup record
/// (`0x66`).
///
/// The record opens with three narrowing enums before the four big-endian margin longs: the report
/// kind, one unnamed enum, and the report's canned formatting style. Both are stored facts.
/// The kind is not a restatement of the multi-column geometry — the engine loads it on its own,
/// ahead of the multi-column format block, and every report stores such a block whether or not it is
/// a multi-column report. The style is the record's *third* value, not its third byte: each enum
/// ahead of it widens to two bytes once its value reaches `0x80`.
fn report_kind_and_style(
    tree: &[RecordNode],
    logical: &[u8],
) -> (crate::model::ReportKind, crate::model::ReportStyle) {
    let Some(node) = nodes_where(tree, |n| n.rtype == PAGE_SETUP)
        .first()
        .copied()
    else {
        return Default::default();
    };
    let row = row_of(node, logical, &ft::PAGE_SETUP);
    (
        crate::model::ReportKind::from_code(row.u("report_kind") as i32),
        crate::model::ReportStyle::from_code(row.u("report_style") as i32),
    )
}

#[cfg(test)]
mod tests {
    use crate::field_table::cursor::{Piece, RecordContent, StringFormat};
    use crate::field_table::table::read_strings;
    use crate::field_table::tables as ft;
    use crate::model::{ReportKind, ReportStyle};

    fn kind_of(rel: &str) -> ReportKind {
        let path = rpt_test_support::fixture("tests/fixtures/reports").join(rel);
        let rpt = crate::Rpt::open(&path).unwrap_or_else(|e| panic!("open {rel}: {e}"));
        rpt.report().report_definition.kind
    }

    /// The report style read from a page-setup run through the record's own field table.
    ///
    /// A record built here carries no header to declare a string form, so the reading names the
    /// enhanced form — the one the record-tree reader admits — rather than leaving it assumed.
    fn style_of(run: &[u8]) -> ReportStyle {
        let content = RecordContent {
            rtype: ft::PAGE_SETUP.rtype,
            schema: 0x0700,
            pieces: vec![Piece::Run(run.to_vec())],
        };
        ReportStyle::from_code(
            read_strings(&ft::PAGE_SETUP, &content, StringFormat::Enhanced)
                .row
                .u("report_style") as i32,
        )
    }

    /// `ReportStyle` is the page-setup record's third value, not its third byte.
    ///
    /// The two enums ahead of it are narrowing: either widens to two bytes once its value reaches
    /// `0x80`, and the style moves with it. A reader that took byte 2 would report the wide kind's
    /// second byte as the style.
    #[test]
    fn report_style_follows_the_two_enums_ahead_of_it() {
        assert_eq!(style_of(&[0x00, 0x00, 0x03]), ReportStyle::Table);
        assert_eq!(style_of(&[0x00, 0x04, 0x09]), ReportStyle::MaroonTealBox);
        // The kind widens: the style is now byte 3, and byte 2 is the kind's low half.
        assert_eq!(style_of(&[0x80, 0x82, 0x00, 0x07]), ReportStyle::Shade);
        // The middle enum widens instead.
        assert_eq!(
            style_of(&[0x00, 0x80, 0x90, 0x08]),
            ReportStyle::RedBlueBorder
        );
        // Outside the engine's `0..=9` switch the code is kept verbatim rather than named.
        assert_eq!(style_of(&[0x00, 0x00, 0x80, 0xff]), ReportStyle::Other(255));
    }

    /// The default style reads as a stored `0` (Standard), not an absence of the field.
    #[test]
    fn report_style_decodes_from_the_corpus() {
        for rel in [
            "worrall/USStatesWithAbbreviations.rpt",
            "benbrahim777/Canada - Cross Tab.rpt",
        ] {
            let path = rpt_test_support::fixture("tests/fixtures/reports").join(rel);
            let rpt = crate::Rpt::open(&path).unwrap_or_else(|e| panic!("open {rel}: {e}"));
            assert_eq!(
                rpt.report().report_definition.style,
                ReportStyle::Standard,
                "{rel}"
            );
        }
    }

    /// `ReportKind` comes from byte 0 of the page-setup record, and only byte 0.
    ///
    /// The page-setup record opens with three one-byte enums; the neighbouring one at offset 1 also
    /// varies independently of the report kind, so the three columnar fixtures below — which each
    /// carry a different value of it — pin the offset. Reading offset 1 instead would still
    /// separate the multi-column report from the first columnar one, and only these controls catch
    /// it.
    #[test]
    fn report_kind_from_page_setup_byte_0() {
        assert_eq!(
            kind_of("worrall/USStatesWithAbbreviations.rpt"),
            ReportKind::MultiColumnReport
        );
        for columnar in [
            "worrall/AlphaISOsByCountry.rpt",
            "synthetic/underlay_span_nested.rpt",
            "benbrahim777/Canada - Cross Tab.rpt",
        ] {
            assert_eq!(
                kind_of(columnar),
                ReportKind::ColumnarReport,
                "{columnar} is a columnar report"
            );
        }
    }
}
