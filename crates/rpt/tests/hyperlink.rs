//! Structural decode test for an object hyperlink (`ObjectFormat.hyperlink`, from the `0x00fc`
//! CSArchive tail). RptToXml never emits a hyperlink, so this is verified against the decoded model
//! (cross-checked against RAS `Format.HyperlinkText`/`HyperlinkType`). Skips if the fixture is absent.

use rpt::model::HyperlinkType;
use std::path::Path;

fn open(name: &str) -> Option<rpt::Rpt> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../tests/fixtures/reports/synthetic/{name}.rpt"));
    rpt::Rpt::open(&path).ok()
}

#[test]
fn decodes_object_hyperlink_text_and_type() {
    let Some(rpt) = open("hyperlink") else {
        eprintln!("[skip] synthetic/hyperlink.rpt absent");
        return;
    };
    let links: Vec<_> = rpt
        .report()
        .report_definition
        .areas
        .iter()
        .flat_map(|a| &a.sections)
        .flat_map(|s| &s.objects)
        .filter_map(|o| o.format.hyperlink.as_ref())
        .collect();

    assert_eq!(links.len(), 1, "one object carries a hyperlink");
    assert_eq!(links[0].text, "https://google.com");
    // RAS: crHyperlinkTypeWebsite → AFileOrWebSite.
    assert_eq!(links[0].kind, HyperlinkType::AFileOrWebSite);
}

#[test]
fn text_object_without_hyperlink_decodes_none() {
    let Some(rpt) = open("single_group") else {
        eprintln!("[skip] synthetic/single_group.rpt absent");
        return;
    };
    // Every text/field object's HyperlinkText is empty here (RAS: crHyperlinkTypeUndefined), so no
    // object should carry a decoded hyperlink — the empty CSArchive target must not pick up a later
    // format string.
    for obj in rpt
        .report()
        .report_definition
        .areas
        .iter()
        .flat_map(|a| &a.sections)
        .flat_map(|s| &s.objects)
    {
        assert!(
            obj.format.hyperlink.is_none(),
            "{} should have no hyperlink",
            obj.name,
        );
    }
}
