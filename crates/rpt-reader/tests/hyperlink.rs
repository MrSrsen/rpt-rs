//! Structural decode test for an object hyperlink (`ObjectFormat.hyperlink`, from the `0x00fc`
//! record's trailing bytes), mirroring RAS `Format.HyperlinkText` / `HyperlinkType`. Skips if the
//! fixture is absent.

use rpt_reader::model::{HyperlinkType, TextRotationAngle};

fn open(name: &str) -> Option<rpt_reader::Rpt> {
    let path = rpt_test_support::fixture(format!("tests/fixtures/reports/synthetic/{name}.rpt"));
    rpt_reader::Rpt::open(&path).ok()
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
    // RAS: crHyperlinkTypeWebsite → Website (distinct from Html in the RAS model).
    assert_eq!(links[0].kind, HyperlinkType::Website);
}

/// The rotation angle lives in the two filler bytes *after* `HyperlinkText`, so a non-empty link
/// target pushes it further into the field bytes. This fixture pairs two rotations with two target lengths,
/// which is what distinguishes the walk from a fixed offset: at a fixed byte 20 the two rotated
/// objects read ASCII out of their own URLs instead of an angle.
#[test]
fn rotation_offset_tracks_the_hyperlink_target_length() {
    let Some(rpt) = open("text_rotation_hyperlink") else {
        eprintln!("[skip] synthetic/text_rotation_hyperlink.rpt absent");
        return;
    };
    let got: Vec<_> = rpt
        .report()
        .report_definition
        .areas
        .iter()
        .flat_map(|a| &a.sections)
        .flat_map(|s| &s.objects)
        .map(|o| {
            let link = o.format.hyperlink.as_ref();
            (
                o.name.as_str(),
                o.format.text_rotation,
                link.map(|h| h.text.as_str()).unwrap_or_default(),
            )
        })
        .collect();

    assert_eq!(
        got,
        vec![
            (
                "RotateHyperlinkShort",
                TextRotationAngle::Rotate90,
                "https://a.example",
            ),
            (
                "RotateHyperlinkLong",
                TextRotationAngle::Rotate270,
                "https://example.com/a/much/longer/target/path",
            ),
            ("RotateOnlyControl", TextRotationAngle::Rotate90, ""),
            (
                "HyperlinkOnlyControl",
                TextRotationAngle::Rotate0,
                "https://example.org/plain",
            ),
        ],
    );
}

#[test]
fn text_object_without_hyperlink_decodes_none() {
    let Some(rpt) = open("single_group") else {
        eprintln!("[skip] synthetic/single_group.rpt absent");
        return;
    };
    // Every text/field object's HyperlinkText is empty here (RAS: crHyperlinkTypeUndefined), so no
    // object should carry a decoded hyperlink — the empty target must not pick up a later format
    // string.
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
