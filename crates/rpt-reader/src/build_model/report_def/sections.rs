//! Area / section construction and the canonical band ordering — opening areas and sections from
//! their marker records, and folding + sorting them into the SDK's `Areas` band sequence.

use crate::build_model::row_of;
use crate::codec::RecordNode;
use crate::field_table::tables as ft;
use crate::model::{Area, AreaSectionKind, Section, Twips};

/// Open an area (`0x8a`). The detail area-pair's auxiliary `DetailHeader` / `DetailFooter` halves
/// are folded into the single `Detail` area, so they are skipped (their objects, if any, attach to
/// the preceding area).
/// Returns `true` if a real area was opened, `false` if this was an auxiliary detail-pair
/// Header/Footer marker that the engine folds away (the caller suppresses its trailing records).
pub(super) fn open_area(areas: &mut Vec<Area>, node: &RecordNode, logical: &[u8]) -> bool {
    let name = row_of(node, logical, &ft::AREA).text("name").to_owned();
    if name.starts_with("DetailHeader") || name.starts_with("DetailFooter") {
        return false;
    }
    let kind = area_kind(&name);
    areas.push(Area {
        kind,
        name,
        sections: Vec::new(),
        ..Default::default()
    });
    true
}

/// Open a section (`0x8c`) in the current area, reading its Height (u32 BE twips) + Name. The
/// section's parent band marker (`band_kind`, when present) is the authoritative band kind: it
/// overrides the area's name-derived guess on both the area and the section, since the area/section
/// name cannot be trusted (a group band is commonly renamed after its group field, e.g.
/// `nameHeader`).
pub(super) fn open_section(
    areas: &mut [Area],
    node: &RecordNode,
    logical: &[u8],
    band_kind: Option<AreaSectionKind>,
) {
    let row = row_of(node, logical, &ft::SECTION);
    let height = row.i("height");
    let name = row.text("name").to_owned();
    if let Some(area) = areas.last_mut() {
        if let Some(kind) = band_kind {
            area.kind = kind;
        }
        let kind = area.kind;
        area.sections.push(Section {
            kind,
            height: Twips(height),
            name,
            ..Default::default()
        });
    }
}

pub(super) fn current_section(areas: &mut [Area]) -> Option<&mut crate::model::Section> {
    areas.last_mut()?.sections.last_mut()
}

/// Map an area name (e.g. `PageHeaderArea1`, `DetailArea1`) to its [`AreaSectionKind`]. This is only
/// the initial guess made when the area opens; the authoritative kind is set from the parent band
/// marker once the area's section opens (see [`open_section`]) — the name alone is unreliable, as a
/// group band is commonly renamed after its group field (`nameHeader`, `customeridHeader`).
pub(super) fn area_kind(name: &str) -> AreaSectionKind {
    for (prefix, kind) in [
        ("ReportHeader", AreaSectionKind::ReportHeader),
        ("ReportFooter", AreaSectionKind::ReportFooter),
        ("PageHeader", AreaSectionKind::PageHeader),
        ("PageFooter", AreaSectionKind::PageFooter),
        ("GroupHeader", AreaSectionKind::GroupHeader),
        ("GroupFooter", AreaSectionKind::GroupFooter),
        ("Detail", AreaSectionKind::Detail),
    ] {
        if name.starts_with(prefix) {
            return kind;
        }
    }
    // Some reports name the five fixed bands generically (`Area1`..`Area5`) instead of by band.
    // They are numbered in canonical band order: 1=ReportHeader, 2=PageHeader, 3=Detail,
    // 4=ReportFooter, 5=PageFooter. A misclassification here is corrected by the band marker.
    if let Some(suffix) = name.strip_prefix("Area") {
        return match suffix {
            "1" => AreaSectionKind::ReportHeader,
            "2" => AreaSectionKind::PageHeader,
            "3" => AreaSectionKind::Detail,
            "4" => AreaSectionKind::ReportFooter,
            "5" => AreaSectionKind::PageFooter,
            _ => AreaSectionKind::default(),
        };
    }
    AreaSectionKind::default()
}

/// Reorder areas into the canonical Crystal Reports band sequence —
/// `ReportHeader, PageHeader, GroupHeader[0..N], Detail, GroupFooter[N..0], ReportFooter,
/// PageFooter` — matching the order the SDK's `Areas` collection presents.
/// The native binary stores them in raw storage order (page/report bands first, then the group
/// header/footer areas, then detail), which is not the band order. Note ReportFooter prints
/// *before* PageFooter even though the enum value is larger, so the band rank is explicit.
///
/// Each group area carries its own 0-based nesting level ([`Area::group_level`], from the `0x9b`
/// `SectionCodeAreaType` record). Group headers order ascending by that level (outermost first);
/// group footers order descending (innermost first). This is authoritative regardless of the areas'
/// binary storage order or their (user-renameable) names.
pub(super) fn sort_areas_canonical(areas: &mut [Area]) {
    // One past the deepest group level: used to invert footer ordering (innermost footer first).
    let group_span = areas
        .iter()
        .filter_map(|a| a.group_level)
        .max()
        .map_or(0, |m| m + 1);

    areas.sort_by_key(|a| {
        use crate::model::AreaSectionKind::*;
        let band: u8 = match a.kind {
            ReportHeader => 0,
            PageHeader => 1,
            GroupHeader => 2,
            Detail => 3,
            GroupFooter => 4,
            ReportFooter => 5,
            PageFooter => 6,
            _ => 7,
        };
        let sub: usize = match a.kind {
            GroupHeader => a.group_level.map_or(0, |l| l + 1),
            GroupFooter => a.group_level.map_or(0, |l| group_span - l),
            _ => 0,
        };
        (band, sub)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::rtype::*;

    /// A childless `0x8c` section record over `field_bytes`, unmasked, headed by its own
    /// declaration of the enhanced string form.
    fn section_node(field_bytes: &[u8]) -> (Vec<u8>, RecordNode) {
        let mut logical = vec![0u8; 8];
        logical[0] = crate::build_model::enhanced_header_byte(SECTION, 0);
        let start = logical.len();
        logical.extend_from_slice(field_bytes);
        let end = logical.len();
        let node = RecordNode {
            rtype: SECTION,
            schema: 0x0700,
            offset: 0,
            content_start: start,
            content_end: end,
            mask: 0,
            children: Vec::new(),
        };
        (logical, node)
    }

    /// A group area is named after its group field (`nameHeader`), so `area_kind`
    /// guesses `ReportHeader`; the parent band marker's kind must override that on both the area and
    /// the section it opens.
    #[test]
    fn band_marker_kind_overrides_name_derived_area_kind() {
        let mut areas = vec![Area {
            kind: area_kind("nameHeader"),
            name: "nameHeader".to_string(),
            ..Default::default()
        }];
        assert_eq!(areas[0].kind, AreaSectionKind::ReportHeader);

        // Section field bytes: Height (u32 BE) = 320, no name.
        let (logical, node) = section_node(&320u32.to_be_bytes());
        open_section(
            &mut areas,
            &node,
            &logical,
            Some(AreaSectionKind::GroupHeader),
        );

        assert_eq!(areas[0].kind, AreaSectionKind::GroupHeader);
        assert_eq!(areas[0].sections[0].kind, AreaSectionKind::GroupHeader);
        assert_eq!(areas[0].sections[0].height, Twips(320));
    }

    /// A group area whose kind is set from the band marker.
    fn group_area(kind: AreaSectionKind, name: &str, level: usize) -> Area {
        Area {
            kind,
            name: name.to_string(),
            group_level: Some(level),
            ..Default::default()
        }
    }

    fn plain_area(kind: AreaSectionKind, name: &str) -> Area {
        Area {
            kind,
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// The canonical band sort orders group headers ascending and footers descending by each area's
    /// decoded `group_level` — never by the area name or the areas' binary storage order. This is the
    /// crux for a multi-group report whose group areas are named after their group field (no trailing
    /// digit) and/or stored out of nesting order: three custom-named groups whose storage order is
    /// scrambled must still land in level order (0,1,2 headers; 2,1,0 footers).
    #[test]
    fn group_bands_sort_by_decoded_level_not_name_or_storage_order() {
        use crate::model::AreaSectionKind::*;
        // Storage order deliberately scrambled and names non-monotonic (two share the base "name").
        let mut areas = vec![
            group_area(GroupFooter, "nameFooter1", 1),
            group_area(GroupHeader, "RepKeyHeader", 2),
            plain_area(Detail, "DetailArea1"),
            group_area(GroupHeader, "nameHeader", 0),
            group_area(GroupFooter, "RepKeyFooter", 2),
            plain_area(ReportHeader, "ReportHeaderArea1"),
            group_area(GroupHeader, "nameHeader1", 1),
            group_area(GroupFooter, "nameFooter", 0),
            plain_area(ReportFooter, "ReportFooterArea1"),
        ];
        sort_areas_canonical(&mut areas);
        let order: Vec<&str> = areas.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            order,
            vec![
                "ReportHeaderArea1",
                "nameHeader",   // group level 0 (outermost)
                "nameHeader1",  // group level 1
                "RepKeyHeader", // group level 2 (innermost)
                "DetailArea1",
                "RepKeyFooter", // footers reverse: innermost first
                "nameFooter1",
                "nameFooter",
                "ReportFooterArea1",
            ]
        );
    }

    /// With no band marker (the fallback), the section inherits the area's existing (name-derived)
    /// kind, preserving the behaviour for reports whose area names are authoritative.
    #[test]
    fn open_section_without_band_marker_keeps_area_kind() {
        let mut areas = vec![Area {
            kind: AreaSectionKind::PageHeader,
            name: "PageHeaderArea1".to_string(),
            ..Default::default()
        }];
        let (logical, node) = section_node(&100u32.to_be_bytes());
        open_section(&mut areas, &node, &logical, None);
        assert_eq!(areas[0].kind, AreaSectionKind::PageHeader);
        assert_eq!(areas[0].sections[0].kind, AreaSectionKind::PageHeader);
    }
}
