//! Area / section construction and the canonical band ordering — opening areas and sections from
//! their marker records, and folding + sorting them into the SDK's `Areas` band sequence.

use super::*;

/// Open an area (`0x8a`). The detail area-pair's auxiliary `DetailHeader` / `DetailFooter` halves
/// are folded into the single `Detail` area, so they are skipped (their objects, if any, attach to
/// the preceding area).
/// Returns `true` if a real area was opened, `false` if this was an auxiliary detail-pair
/// Header/Footer marker that the engine folds away (the caller suppresses its trailing records).
pub(super) fn open_area(areas: &mut Vec<Area>, node: &RecordNode, logical: &[u8]) -> bool {
    let name = first_string(node, logical).unwrap_or_default();
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

/// Open a section (`0x8c`) in the current area, reading its Height (u32 BE twips) + Name.
pub(super) fn open_section(areas: &mut [Area], node: &RecordNode, logical: &[u8]) {
    let b = node.leaf_bytes(logical);
    let height = i32_be(&b, 0).unwrap_or(0);
    let name = b.get(4..).and_then(first_lp).unwrap_or_default();
    if let Some(area) = areas.last_mut() {
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

/// Map an area name (e.g. `PageHeaderArea1`, `DetailArea1`) to its [`AreaSectionKind`].
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
    // 4=ReportFooter, 5=PageFooter (group bands always carry explicit `GroupHeader/FooterArea`
    // names, so they never reach here).
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

/// Map each `GroupHeader` area suffix to its canonical group level (first-appearing header =
/// outermost = 1), keyed by the trailing-digit suffix its header and footer share. Crystal numbers
/// area suffixes in UI-creation order, not nesting order, so a footer's suffix need not equal its
/// group's nesting index — this first-appearance mapping is the authoritative level.
///
/// The single source of the level map: `sort_areas_canonical` calls it on the finished areas, and
/// the object walk calls it on the areas built so far to scope each summary/group-name object (a
/// group's header always precedes its own fields, so the prefix already holds the needed entry).
pub(super) fn canonical_group_levels(areas: &[Area]) -> std::collections::HashMap<String, usize> {
    let mut suffix_level: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut next = 1usize;
    for area in areas {
        if area.kind == AreaSectionKind::GroupHeader {
            suffix_level
                .entry(trailing_digits(&area.name))
                .or_insert_with(|| {
                    let l = next;
                    next += 1;
                    l
                });
        }
    }
    suffix_level
}

/// Reorder areas into the canonical Crystal Reports band sequence —
/// `ReportHeader, PageHeader, GroupHeader[1..N], Detail, GroupFooter[N..1], ReportFooter,
/// PageFooter` — matching the order the SDK's `Areas` collection presents.
/// The native binary stores them in raw storage order (page/report bands first, then interleaved
/// group header/footer pairs, then detail), which is not the band order. Note ReportFooter prints
/// *before* PageFooter even though the enum value is larger, so the band rank is explicit.
///
/// Group nesting level (1 = outermost) is assigned by the order in which `GroupHeader` areas
/// appear in the binary; the matching `GroupFooter` is linked by the trailing-digit suffix shared
/// with its header (e.g. `GroupHeaderArea4` ↔ `GroupFooterArea4`).
pub(super) fn sort_areas_canonical(areas: &mut [Area]) {
    let suffix_level = canonical_group_levels(areas);
    let n = suffix_level.len();

    areas.sort_by_key(|a| {
        use AreaSectionKind::*;
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
            GroupHeader => *suffix_level.get(&trailing_digits(&a.name)).unwrap_or(&0),
            GroupFooter => match suffix_level.get(&trailing_digits(&a.name)) {
                Some(&lv) => n + 1 - lv,
                None => 0,
            },
            _ => 0,
        };
        (band, sub)
    });
}
