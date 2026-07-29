//! The Page IR projected onto a PDF structure tree.
//!
//! A tagged PDF carries, beside the drawing operators, a tree saying what the marks *mean* and in
//! what order they are read. The Page IR is a flat, paint-ordered op list, so the tree is
//! reconstructed from the two things every op carries: its [`ObjectRef`] (section name, object name,
//! object kind, per-placement instance id) and its position in paint order.
//!
//! - **Bands.** The layout engine emits one section occurrence contiguously, so a maximal run of
//!   consecutive ops sharing a section name is one band. A band that repeats — the detail band, once
//!   per row — is split again at each section background and at each repeat of an object name, so one
//!   row is one group rather than the whole page being one.
//! - **Units.** Within a band instance, a maximal run of consecutive ops sharing an object identity
//!   (name + instance + kind) is one placed object. A text object's lines become `Span`s under one
//!   `P`; a picture or chart becomes one `Figure`; a rule, a border or a section background becomes an
//!   artifact and never enters the tree.
//! - **Reading order.** Paint order within a band is report-definition order, not reading order, so a
//!   band's *tree children* are banded into rows (by vertical overlap, not by equal `top` — a
//!   larger-pointed field in the same row starts a few twips higher) and read left to right within a
//!   row. Only the tree is reordered; the draws stay in paint order, which is what decides what
//!   covers what.
//!
//! Everything here is a decision about *structure*, so it is deliberately krilla-free and
//! independently testable: the writer turns a [`Unit`] into marked content, it does not decide what a
//! unit is.
//!
//! Which band a section is comes from the document's own
//! [`sections`](rpt_pages::PagedDocument::sections) dictionary — the producer knows it and the stored
//! section name (`Section3`, `TSection7`) does not say. Mapping a band onto a PDF artifact role is
//! this backend's policy, not the IR's, so [`artifact_roles`] holds it here; a caller that disagrees
//! overrides the whole classification through [`Semantics::artifact_sections`]. Alternate text stays
//! caller-supplied, since nothing in the IR describes a picture. Where either is absent the
//! classification stays fail-safe — unclassifiable content is tagged as content, never demoted to an
//! artifact, because a dropped artifact is the one error a reader of the file cannot notice.

use crate::{ArtifactRole, Semantics};
use rpt_model::AreaSectionKind;
use rpt_pages::{DrawOp, ObjectKind, ObjectRef, Page, SectionInfo, TextRun};
use std::collections::BTreeMap;
use std::ops::Range;

/// How a run of marks is tagged, as the structure planner sees it (krilla's own artifact and tag
/// types stay in the writer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UnitKind {
    /// Real text: every op in the run is a [`DrawOp::Text`] line of one placed object. Each line is
    /// tagged as its own `Span` and the lines together form one `P` — krilla asks that a span hold at
    /// most one line, and a wrapped field is exactly a paragraph of such lines.
    Paragraph,
    /// Graphical content — a picture, or the many paths and labels of one chart — as a single
    /// `Figure`. `alt` is the caller's alternate text; `None` means undescribed.
    Figure {
        /// The report object's name, for the failure message when `alt` is missing.
        object: String,
        /// The alternate text describing the figure, when the caller supplied one.
        alt: Option<String>,
    },
    /// Not part of the logical content: a rule, a box border, a section background, a figure the
    /// caller marked decorative, or anything in a section classified as page furniture. Marked as an
    /// artifact so assistive technology skips it, and left out of the tree entirely.
    Artifact(ArtifactKind),
}

/// The artifact classes this backend emits — the two PDF distinguishes for a report's marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArtifactKind {
    /// Cosmetic geometry: rules, borders, shading, section backgrounds.
    Layout,
    /// Page furniture repeated by pagination, in the role its section's band takes.
    Pagination(ArtifactRole),
}

/// One placed object's ops and what they mean.
#[derive(Debug, Clone)]
pub(crate) struct Unit {
    /// The half-open range of page-op indices this unit covers.
    pub(crate) ops: Range<usize>,
    /// How the run is tagged.
    pub(crate) kind: UnitKind,
    /// The run's bounds in twips as `(top, bottom, left)` — the reading-order key.
    extent: (i32, i32, i32),
}

/// One band occurrence on a page: the ops of a single section instance, split into units.
#[derive(Debug, Clone)]
pub(crate) struct Band {
    /// The units of this band, in paint order.
    pub(crate) units: Vec<Unit>,
}

impl Band {
    /// The indices into [`Self::units`] of the units that become tree nodes, in reading order:
    /// banded into rows top-to-bottom, then left-to-right within a row.
    ///
    /// Returns indices rather than reordering the units, because the writer must draw in paint order
    /// — reordering the draws would change what covers what.
    pub(crate) fn reading_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.units.len())
            .filter(|&i| !matches!(self.units[i].kind, UnitKind::Artifact(_)))
            .collect();
        // Sort by top first so the row sweep below sees candidates in vertical order; `sort_by_key`
        // is stable, so units that tie keep paint order.
        order.sort_by_key(|&i| self.units[i].extent.0);
        let mut out = Vec::with_capacity(order.len());
        let mut row: Vec<usize> = Vec::new();
        for i in order {
            if row
                .iter()
                .any(|&j| same_row(self.units[j].extent, self.units[i].extent))
            {
                row.push(i);
                continue;
            }
            flush_row(&mut row, &self.units, &mut out);
            row.push(i);
        }
        flush_row(&mut row, &self.units, &mut out);
        out
    }
}

/// Emit a completed row left-to-right and clear it.
fn flush_row(row: &mut Vec<usize>, units: &[Unit], out: &mut Vec<usize>) {
    row.sort_by_key(|&i| units[i].extent.2);
    out.append(row);
}

/// Whether two runs sit on the same line of the page: their vertical extents overlap by more than
/// half the shorter one. Equal `top` is the wrong test — a larger-pointed field in the same row
/// starts higher, and a field's fill rect starts above its text.
fn same_row(a: (i32, i32, i32), b: (i32, i32, i32)) -> bool {
    let overlap = a.1.min(b.1) - a.0.max(b.0);
    if overlap <= 0 {
        return false;
    }
    let shorter = (a.1 - a.0).min(b.1 - b.0);
    // A zero-height run (a baseline rule) shares a row with anything it touches.
    shorter <= 0 || overlap * 2 > shorter
}

/// Which sections are page furniture, and in what role — resolved once per document.
///
/// Derived from the document's band dictionary: a page header or footer is the running furniture
/// PDF/UA-1 §7.8 asks be marked as such, and every other band — including a report header, which
/// prints once and *is* document content — reads. A caller that disagrees with the document replaces
/// the whole classification via [`Semantics::artifact_sections`]; `Some` of an empty map therefore
/// means "classified, and nothing is furniture", not "unclassified".
///
/// A section with no entry is absent from the result and so reads as content, which is the direction
/// a missing classification must fail in.
pub(crate) fn artifact_roles(
    sections: &BTreeMap<String, SectionInfo>,
    semantics: &Semantics,
) -> BTreeMap<String, ArtifactRole> {
    if let Some(by_caller) = &semantics.artifact_sections {
        return by_caller.clone();
    }
    sections
        .iter()
        .filter_map(|(name, info)| Some((name.clone(), role_of(info.band)?)))
        .collect()
}

/// The artifact role a band takes, or `None` for a band whose content reads.
fn role_of(band: AreaSectionKind) -> Option<ArtifactRole> {
    match band {
        AreaSectionKind::PageHeader => Some(ArtifactRole::Header),
        AreaSectionKind::PageFooter => Some(ArtifactRole::Footer),
        _ => None,
    }
}

/// Split a page's ops into band instances and units. The unit ranges partition `0..page.ops.len()`
/// in order, so the writer draws straight through the op list while tagging each run.
pub(crate) fn plan(
    page: &Page,
    roles: &BTreeMap<String, ArtifactRole>,
    semantics: &Semantics,
) -> Vec<Band> {
    let mut bands = Vec::new();
    for run in section_runs(page) {
        let role = section_of(&page.ops[run.start])
            .and_then(|s| roles.get(s))
            .copied();
        let units = units_in(page, run, role, semantics);
        bands.extend(split_instances(units, page).map(|units| Band { units }));
    }
    bands
}

/// The maximal runs of consecutive ops sharing a section name.
fn section_runs(page: &Page) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut start = 0usize;
    while start < page.ops.len() {
        let section = section_of(&page.ops[start]);
        let mut end = start + 1;
        while end < page.ops.len() && section_of(&page.ops[end]) == section {
            end += 1;
        }
        runs.push(start..end);
        start = end;
    }
    runs
}

/// Split one section's units into the band's separate occurrences (one detail row each).
///
/// A new occurrence starts at a section background, which the layout engine emits first for every
/// band, and otherwise where an object name comes round again — a band places each of its objects
/// once, so a name reappearing is the next occurrence. One object contributes several adjacent units
/// (its fill, its text lines, its border) under one name, so a repeat only splits when it belongs to
/// a different *placement* — which the per-placement instance id says exactly.
///
/// Getting this wrong merges or splits sibling groups; it cannot mistake content for an artifact.
fn split_instances(units: Vec<Unit>, page: &Page) -> impl Iterator<Item = Vec<Unit>> {
    let mut out: Vec<Vec<Unit>> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    let mut previous: Option<(&str, Option<u32>)> = None;
    for unit in units {
        let source = page.ops[unit.ops.start].source();
        let placement = source.and_then(|s| Some((s.object_name.as_deref()?, s.instance)));
        let is_background = matches!(source.map(|s| s.kind), Some(ObjectKind::Section));
        let repeated =
            placement.is_some_and(|(name, _)| placement != previous && seen.contains(&name));
        if out.is_empty() || is_background || repeated {
            out.push(Vec::new());
            seen.clear();
        }
        if let Some((name, _)) = placement {
            seen.push(name);
        }
        previous = placement;
        out.last_mut().expect("a group was just pushed").push(unit);
    }
    out.into_iter()
}

/// The section name an op belongs to, or `None` for an op with no source (chart internals from a
/// producer that assigns none, and synthetic ops).
fn section_of(op: &DrawOp) -> Option<&str> {
    op.source().map(|s| s.section.as_str())
}

/// The identity that keeps consecutive ops in one unit: the placed object they came from. An op with
/// no source stands alone — nothing says it belongs with its neighbour.
fn identity(op: &DrawOp) -> Option<(&str, Option<&str>, Option<u32>, ObjectKind)> {
    op.source().map(|s: &ObjectRef| {
        (
            s.section.as_str(),
            s.object_name.as_deref(),
            s.instance,
            s.kind,
        )
    })
}

/// Split one band's op range into per-object units.
fn units_in(
    page: &Page,
    range: Range<usize>,
    furniture: Option<ArtifactRole>,
    semantics: &Semantics,
) -> Vec<Unit> {
    let mut units = Vec::new();
    let mut start = range.start;
    while start < range.end {
        let id = identity(&page.ops[start]);
        let mut end = start + 1;
        // `None == None` would pool unrelated sourceless ops, so only an identified op extends a run.
        if id.is_some() {
            while end < range.end && identity(&page.ops[end]) == id {
                end += 1;
            }
        }
        let ops = start..end;
        units.push(Unit {
            kind: classify(&page.ops[ops.clone()], furniture, semantics),
            extent: extent_of(&page.ops[ops.clone()]),
            ops,
        });
        start = end;
    }
    units
}

/// A run's `(top, bottom, left)` in twips.
fn extent_of(ops: &[DrawOp]) -> (i32, i32, i32) {
    let top = ops.iter().map(|o| o.bounds().top.0).min().unwrap_or(0);
    let bottom = ops
        .iter()
        .map(|o| o.bounds().top.0 + o.bounds().height.0)
        .max()
        .unwrap_or(0);
    let left = ops.iter().map(|o| o.bounds().left.0).min().unwrap_or(0);
    (top, bottom, left)
}

/// Decide what a run of ops from one placed object means.
///
/// Fail-safe: a run this cannot place is treated as content, never as an artifact.
fn classify(ops: &[DrawOp], furniture: Option<ArtifactRole>, semantics: &Semantics) -> UnitKind {
    let Some(source) = ops.first().and_then(|o| o.source()) else {
        // No identity at all. Text still reads; anything else is decoration.
        return match ops.first() {
            Some(DrawOp::Text(_)) => UnitKind::Paragraph,
            _ => UnitKind::Artifact(ArtifactKind::Layout),
        };
    };
    // Everything in a section classified as page furniture is pagination, whatever it draws.
    if let Some(role) = furniture {
        return UnitKind::Artifact(ArtifactKind::Pagination(role));
    }
    // A picture or a chart is one graphic however many paths and labels it draws — including a
    // metafile picture, whose replayed shapes *and* text all carry the picture's identity. Splitting
    // one graphic into "its text reads, its shapes do not" would be incoherent.
    if matches!(source.kind, ObjectKind::Image | ObjectKind::Chart) {
        let name = source.object_name.clone().unwrap_or_default();
        return match semantics.alt_text.get(&name).map(String::as_str) {
            // The HTML `alt=""` convention: the caller has looked and says it carries no information.
            Some("") => UnitKind::Artifact(ArtifactKind::Layout),
            alt => UnitKind::Figure {
                object: name,
                alt: alt.map(str::to_string),
            },
        };
    }
    // Otherwise the op type decides. A rule, a border, a fill and a section background carry no text
    // and exist only to shape the page — the textbook layout artifact. Deciding on the op type rather
    // than the object kind catches the cases an enumeration misses, such as a cross-tab's grid lines
    // and cell fills, which carry the cross-tab's kind rather than `Line`/`Box`.
    if ops.iter().any(|o| matches!(o, DrawOp::Text(_))) {
        UnitKind::Paragraph
    } else {
        UnitKind::Artifact(ArtifactKind::Layout)
    }
}

/// The text a line's span should declare as its actual content, or `None` to let the glyphs speak
/// for themselves.
///
/// One case needs it, and only one, because `ActualText` replaces the extracted text of the whole
/// sequence — a wrong one is worse than none: **a wrapped line**, because the wrapper consumes the
/// space it broke at, so consecutive lines of one paragraph would extract concatenated (`fieldname`
/// from `field` + `name`). A line's own interior spaces are drawn as glyphs, justified or not, so
/// they need no declaring.
///
/// The string comes only from [`TextRun::text`], which already holds the interior spaces — it is
/// never reconstructed from the glyphs.
pub(crate) fn actual_text(run: &TextRun, last_line: bool) -> Option<String> {
    if last_line {
        return None;
    }
    Some(format!("{} ", run.text))
}

#[cfg(test)]
mod tests;
