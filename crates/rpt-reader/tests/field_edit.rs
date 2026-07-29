//! Field-addressed edits: `Rpt::patch_record_field` over the committed synthetic fixtures.
//!
//! Every assertion is on the re-opened report, not on the bytes: the point of naming a field
//! instead of an offset is that the reader agrees the value moved and nothing else did. A
//! width-changing edit is checked the same way, since the record's length prefix and every
//! enclosing record's are recomputed rather than asserted.

use rpt_reader::fields::FieldEdit;
use rpt_reader::raw::RecordTag;
use rpt_reader::{EditErrorKind, Error, Rpt, StreamId};
use std::path::Path;

/// A report definition's section record, whose table declares a fixed-width integer, a string and
/// a nested child record — the three shapes an edit has to place differently.
const SECTION: RecordTag = RecordTag(0x008c);

fn blank() -> Rpt {
    let path = rpt_test_support::fixture(
        Path::new("tests/fixtures/reports").join("synthetic/blank_report.rpt"),
    );
    Rpt::open(&path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
}

fn reopen(bytes: &[u8]) -> Rpt {
    Rpt::read(bytes).expect("the patched report re-opens")
}

/// Every field of the `nth` record of `tag`, as (name, value) pairs.
fn reading(rpt: &Rpt, tag: RecordTag, nth: usize) -> Vec<(String, String)> {
    let r = rpt
        .record_fields(tag, nth)
        .unwrap_or_else(|e| panic!("record #{nth} of {tag:?} has a field table: {e}"));
    assert!(r.exact(), "{tag:?} #{nth} reads exactly under its table");
    r.fields
        .iter()
        .map(|f| (f.path.clone(), format!("{:?}", f.value)))
        .collect()
}

fn logical_len(rpt: &Rpt) -> usize {
    rpt.stream(&StreamId::Contents)
        .expect("Contents stream")
        .logical_bytes()
        .len()
}

fn edit_error(rpt: &Rpt, field: &str, value: &FieldEdit) -> (EditErrorKind, String) {
    match rpt.patch_record_field(SECTION, 0, field, value) {
        Ok(_) => panic!("editing `{field}` should have been refused"),
        Err(Error::Edit { kind, detail }) => (kind, detail),
        Err(other) => panic!("expected an edit refusal, got {other:?}"),
    }
}

/// A same-width edit moves exactly one value and leaves the stream the length it was.
#[test]
fn a_same_width_edit_moves_one_value_and_nothing_else() {
    let rpt = blank();
    let before = reading(&rpt, SECTION, 0);
    let bytes = rpt
        .patch_record_field(SECTION, 0, "height", &FieldEdit::Int(1234))
        .expect("a section's height is editable");

    let after_rpt = reopen(&bytes);
    assert_eq!(logical_len(&after_rpt), logical_len(&rpt));
    let after = reading(&after_rpt, SECTION, 0);
    let changed: Vec<_> = before
        .iter()
        .zip(&after)
        .filter(|(a, b)| a != b)
        .map(|(a, b)| (a.clone(), b.clone()))
        .collect();
    assert_eq!(changed.len(), 1, "one field changed: {changed:?}");
    assert_eq!(changed[0].0 .0, "height");
    assert_eq!(changed[0].1 .1, "Int(1234)");
}

/// A string that no longer fits is written anyway: the record's length prefix and every enclosing
/// record's absorb the delta, and the report re-opens with the rest of its records intact.
#[test]
fn a_width_changing_edit_is_written_and_the_lengths_follow() {
    let rpt = blank();
    let base_len = logical_len(&rpt);
    let base_name = reading(&rpt, SECTION, 0)
        .into_iter()
        .find(|(n, _)| n == "name")
        .expect("a section carries a name")
        .1;
    // The stored block is the text plus its terminating NUL, so the delta is the text delta.
    let old_text = base_name
        .trim_start_matches("Text(\"")
        .trim_end_matches("\")");

    for new_text in ["A", "AVeryMuchLongerSectionName"] {
        let bytes = rpt
            .patch_record_field(SECTION, 0, "name", &FieldEdit::Text(new_text.to_string()))
            .unwrap_or_else(|e| panic!("writing {new_text:?}: {e}"));
        let after_rpt = reopen(&bytes);
        assert_eq!(
            logical_len(&after_rpt) as i64 - base_len as i64,
            new_text.len() as i64 - old_text.len() as i64,
            "the stream grows by exactly the text delta for {new_text:?}"
        );
        let after = reading(&after_rpt, SECTION, 0);
        assert!(
            after.contains(&("name".to_string(), format!("Text({new_text:?})"))),
            "{new_text:?} reads back: {after:?}"
        );
        // The record tree still partitions the whole stream: a wrong ancestor length would derail
        // the framing from the splice onwards, and every record after it with it.
        let contents = after_rpt
            .stream(&StreamId::Contents)
            .expect("Contents stream");
        assert_eq!(
            contents.serialize_tree(),
            contents.logical_bytes(),
            "the tree still frames the whole stream after writing {new_text:?}"
        );
    }
}

/// A field whose value decides how much of the record follows it is refused, because writing it
/// alone leaves the rest of the record read at the wrong offsets. Nothing in the table says which
/// fields those are — a repeat's count and a conditional field's guard are predicates, not
/// declarations — so the rule is enforced by reading the written record back.
///
/// A group's `order_pair_count` is one: it counts the sort pairs that follow it.
#[test]
fn a_field_that_decides_what_follows_it_is_refused_but_can_be_forced() {
    let path = rpt_test_support::fixture(
        Path::new("tests/fixtures/reports").join("synthetic/single_group.rpt"),
    );
    let rpt = Rpt::open(&path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let group = RecordTag(0x00e5);
    assert_eq!(
        rpt.record_field(group, 0, "order_pair_count")
            .expect("a group counts its sort pairs")
            .value,
        rpt_reader::fields::FieldValue::Uint(0)
    );

    match rpt.patch_record_field(group, 0, "order_pair_count", &FieldEdit::Int(3)) {
        Err(Error::Edit { kind, detail }) => {
            assert_eq!(kind, EditErrorKind::EditNotVerified);
            assert!(detail.contains("order_pair_count"), "{detail}");
        }
        other => panic!("expected the read-back check to refuse, got {other:?}"),
    }
    // Writing a record the reader cannot make sense of is a legitimate thing to want, and is what
    // forcing is for.
    assert!(rpt
        .patch_record_field_with(
            group,
            0,
            "order_pair_count",
            &FieldEdit::Int(3),
            rpt_reader::EditPolicy::Forced,
        )
        .is_ok());
}

/// A field the record's table does not name is refused, and the refusal says what it does name.
#[test]
fn an_unnamed_field_is_refused() {
    let (kind, detail) = edit_error(&blank(), "no_such_field", &FieldEdit::Int(1));
    assert_eq!(kind, EditErrorKind::FieldEdit);
    assert!(detail.contains("no_such_field"), "{detail}");
    assert!(detail.contains("height"), "{detail}");
}

/// A value the field's width cannot hold is refused rather than truncated into it.
#[test]
fn a_value_the_width_cannot_hold_is_refused() {
    let (kind, detail) = edit_error(&blank(), "object_count", &FieldEdit::Int(99_999));
    assert_eq!(kind, EditErrorKind::FieldEdit);
    assert!(detail.contains("u16be"), "{detail}");
}

/// A value of the wrong shape for the field is refused: a string does not go into an integer.
#[test]
fn a_value_of_the_wrong_shape_is_refused() {
    let (kind, _) = edit_error(&blank(), "height", &FieldEdit::Text("tall".into()));
    assert_eq!(kind, EditErrorKind::FieldEdit);
}

/// A record type with no field table has no field to name, and says so rather than guessing an
/// offset. `0x018b` opens an interactive-sort entry and has no table.
#[test]
fn an_untabled_record_type_is_refused() {
    let rpt = blank();
    let untabled = RecordTag(0x018b);
    match rpt.record_fields(untabled, 0) {
        Err(Error::Edit { kind, detail }) => {
            assert_eq!(kind, EditErrorKind::FieldEdit);
            assert!(detail.contains("no field table"), "{detail}");
        }
        other => panic!("0x0076 is the untabled type this test needs: {other:?}"),
    }
    match rpt.patch_record_field(untabled, 0, "anything", &FieldEdit::Int(0)) {
        Err(Error::Edit { kind, detail }) => {
            assert_eq!(kind, EditErrorKind::FieldEdit);
            assert!(detail.contains("no field table"), "{detail}");
        }
        other => panic!("expected an edit refusal, got {other:?}"),
    }
}

/// The value a caller asked for is what the field reads back as, at each wire type the table
/// offers a section.
#[test]
fn parsing_a_literal_follows_the_fields_wire_type() {
    use rpt_reader::fields::FieldKind;
    assert_eq!(
        FieldEdit::parse("0x2a", FieldKind::I32Be),
        Some(FieldEdit::Int(42))
    );
    assert_eq!(
        FieldEdit::parse("-7", FieldKind::I16Be),
        Some(FieldEdit::Int(-7))
    );
    assert_eq!(
        FieldEdit::parse("true", FieldKind::Bool),
        Some(FieldEdit::Int(1))
    );
    assert_eq!(
        FieldEdit::parse("1.5", FieldKind::F64Le),
        Some(FieldEdit::Float(1.5))
    );
    assert_eq!(
        FieldEdit::parse("hi", FieldKind::Text),
        Some(FieldEdit::Text("hi".into()))
    );
    assert_eq!(
        FieldEdit::parse("00ff", FieldKind::Skip),
        Some(FieldEdit::Bytes(vec![0, 0xff]))
    );
    // A composite the vocabulary reads as one unit is not addressed as one value.
    assert_eq!(FieldEdit::parse("x", FieldKind::FieldRef), None);
    assert_eq!(
        FieldEdit::parse("1", FieldKind::Text),
        Some(FieldEdit::Text("1".into()))
    );
    assert_eq!(FieldEdit::parse("nope", FieldKind::U8), None);
}
