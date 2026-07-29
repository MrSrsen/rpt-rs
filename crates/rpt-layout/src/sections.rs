//! The document-level section dictionary: which band each named section belongs to.
//!
//! A draw-op carries only its section's stored *name*, which most reports leave non-semantic
//! (`Section1`, `TSection4`). The band is what a consumer needs to tell page furniture from document
//! content, so the producer — which holds the areas — records it once per document.

use rpt_model::Report;
use rpt_pages::SectionInfo;
use std::collections::{btree_map::Entry, BTreeMap};

/// Accumulates [`SectionInfo`] by section name across a report and every subreport merged into it.
///
/// Subreport section names are not namespaced at merge, so two reports can use one name for
/// different bands. A key whose entries all agree keeps that value; a key whose entries disagree is
/// **poisoned** and yields nothing, so the consumer finds no classification and falls back to
/// treating the content as document content — the safe direction, where a guess would misfile it as
/// furniture and drop it from the reading order.
#[derive(Debug, Default)]
pub(crate) struct SectionMap(BTreeMap<String, Option<SectionInfo>>);

impl SectionMap {
    /// Classify every section of `report` from its **area**: the area's kind and its `group_level`
    /// are authoritative, where the section name is user-renameable and the section code carries no
    /// nesting level.
    pub(crate) fn from_report(report: &Report) -> SectionMap {
        let mut map = SectionMap::default();
        for area in &report.report_definition.areas {
            let info = SectionInfo {
                band: area.kind,
                group_level: area.group_level,
            };
            for section in &area.sections {
                map.insert(section.name.clone(), info);
            }
        }
        map
    }

    fn insert(&mut self, name: String, info: SectionInfo) {
        match self.0.entry(name) {
            Entry::Vacant(e) => {
                e.insert(Some(info));
            }
            // A poisoned entry stays poisoned: a later agreeing writer must not resurrect a name that
            // has already been shown to mean two things.
            Entry::Occupied(mut e) => {
                if *e.get() != Some(info) {
                    e.insert(None);
                }
            }
        }
    }

    /// Merge a formatted subreport's dictionary in, under the same collision rule.
    pub(crate) fn merge(&mut self, other: &BTreeMap<String, SectionInfo>) {
        for (name, info) in other {
            self.insert(name.clone(), *info);
        }
    }

    /// The classified sections: every name that stayed unambiguous.
    pub(crate) fn finish(self) -> BTreeMap<String, SectionInfo> {
        self.0
            .into_iter()
            .filter_map(|(name, info)| info.map(|i| (name, i)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpt_model::AreaSectionKind;

    fn info(band: AreaSectionKind, group_level: Option<usize>) -> SectionInfo {
        SectionInfo { band, group_level }
    }

    #[test]
    fn an_agreeing_collision_keeps_the_value_and_a_disagreeing_one_drops_the_key() {
        let mut map = SectionMap::default();
        map.insert("Details".into(), info(AreaSectionKind::Detail, None));
        map.insert(
            "PageFooterA".into(),
            info(AreaSectionKind::PageFooter, None),
        );

        let mut child = BTreeMap::new();
        child.insert("Details".to_string(), info(AreaSectionKind::Detail, None));
        child.insert(
            "PageFooterA".to_string(),
            info(AreaSectionKind::GroupHeader, Some(0)),
        );
        child.insert(
            "Section1".to_string(),
            info(AreaSectionKind::ReportHeader, None),
        );
        map.merge(&child);

        // A second, *agreeing* writer must not resurrect the poisoned key.
        let mut later = BTreeMap::new();
        later.insert(
            "PageFooterA".to_string(),
            info(AreaSectionKind::GroupHeader, Some(0)),
        );
        map.merge(&later);

        let out = map.finish();
        assert_eq!(
            out.get("Details"),
            Some(&info(AreaSectionKind::Detail, None))
        );
        assert_eq!(
            out.get("Section1"),
            Some(&info(AreaSectionKind::ReportHeader, None))
        );
        assert_eq!(out.get("PageFooterA"), None, "a disagreeing key is dropped");
    }

    #[test]
    fn a_group_level_difference_alone_is_a_disagreement() {
        let mut map = SectionMap::default();
        map.insert(
            "GroupHeader1".into(),
            info(AreaSectionKind::GroupHeader, Some(0)),
        );
        let mut child = BTreeMap::new();
        child.insert(
            "GroupHeader1".to_string(),
            info(AreaSectionKind::GroupHeader, Some(1)),
        );
        map.merge(&child);
        assert!(map.finish().is_empty());
    }
}
