//! Reference-extraction tests: every reference form, aggregation-arg detection (structural),
//! nested calls, comments/strings excluded, formula/parameter helpers.

use crate::formula::refs;
use crate::formula::token::{RefKind, Syntax};

fn field_names(body: &str) -> Vec<String> {
    refs::references(body)
        .into_iter()
        .filter(|r| r.kind == RefKind::Field)
        .map(|r| r.name)
        .collect()
}

#[test]
fn every_reference_form_with_kind() {
    let body = "{Table.f} + {?p} + {@form} + {#rt} + {%sql}";
    let got: Vec<_> = refs::references(body)
        .into_iter()
        .map(|r| (r.kind, r.name))
        .collect();
    assert_eq!(
        got,
        vec![
            (RefKind::Field, "Table.f".into()),
            (RefKind::Parameter, "p".into()),
            (RefKind::Formula, "form".into()),
            (RefKind::RunningTotal, "rt".into()),
            (RefKind::SqlExpr, "sql".into()),
        ]
    );
}

#[test]
fn comment_excludes_refs() {
    let body = "{Table.live} // {Table.dead} {@dead}";
    assert_eq!(field_names(body), vec!["Table.live"]);
    assert!(refs::formula_names(body).next().is_none());
}

#[test]
fn string_literal_excludes_refs() {
    assert_eq!(
        field_names("\"see {Table.f}\" + {Table.g}"),
        vec!["Table.g"]
    );
}

#[test]
fn doubled_quote_string_excludes_inner_ref() {
    let body = "\"a \"\"{Table.f}\"\" b\" & {Table.g}";
    assert_eq!(field_names(body), vec!["Table.g"]);
}

#[test]
fn aggregation_first_arg_counts_second_excluded() {
    let refs = refs::references("Sum({Command.value}, {Command.group})");
    let value = refs.iter().find(|r| r.name == "Command.value").unwrap();
    let group = refs.iter().find(|r| r.name == "Command.group").unwrap();
    assert!(!value.is_aggregation_group_arg());
    assert!(group.is_aggregation_group_arg());
}

#[test]
fn non_aggregation_second_arg_counts() {
    let refs = refs::references("IIf({a.x} = 1, {a.y}, {a.z})");
    for r in &refs {
        assert!(!r.is_aggregation_group_arg(), "{} excluded wrongly", r.name);
    }
}

#[test]
fn nested_call_uses_innermost_frame() {
    let refs = refs::references("IIf(true, Sum({a.x}, {a.y}), {a.z})");
    let y = refs.iter().find(|r| r.name == "a.y").unwrap();
    let x = refs.iter().find(|r| r.name == "a.x").unwrap();
    let z = refs.iter().find(|r| r.name == "a.z").unwrap();
    assert!(y.is_aggregation_group_arg());
    assert!(!x.is_aggregation_group_arg());
    assert!(!z.is_aggregation_group_arg());
}

#[test]
fn aggregation_name_case_insensitive_with_whitespace_and_newlines() {
    let refs = refs::references("SUM\n ( {a.x} ,\n {a.grp} )");
    let grp = refs.iter().find(|r| r.name == "a.grp").unwrap();
    assert!(grp.is_aggregation_group_arg());
    let x = refs.iter().find(|r| r.name == "a.x").unwrap();
    assert!(!x.is_aggregation_group_arg());
}

#[test]
fn enclosing_fn_after_close_paren_is_none() {
    // `)(` — the `(` has no function ident before it.
    let refs = refs::references("(a)({t.f})");
    let f = refs.iter().find(|r| r.name == "t.f").unwrap();
    assert_eq!(f.enclosing_fn, None);
}

#[test]
fn formula_and_parameter_name_helpers() {
    let body = "{@A} + {?P} + Sum({t.f}) + {@B}";
    let fs: Vec<_> = refs::formula_names(body).collect();
    let ps: Vec<_> = refs::parameter_names(body).collect();
    assert_eq!(fs, vec!["A", "B"]);
    assert_eq!(ps, vec!["P"]);
}

#[test]
fn third_arg_of_aggregation_is_still_excluded() {
    // Sum({x}, {grp}, "monthly") — the 3rd arg is a literal; {grp} (2nd) is still a group arg.
    let refs = refs::references("Sum({a.x}, {a.grp}, \"monthly\")");
    let grp = refs.iter().find(|r| r.name == "a.grp").unwrap();
    assert!(grp.is_aggregation_group_arg());
}

#[test]
fn basic_syntax_single_quote_comment_drops_ref() {
    let names: Vec<_> = refs::references_with_syntax("{t.live} ' {t.dead}", Syntax::Basic)
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(names, vec!["t.live"]);
}

#[test]
fn no_whitespace_both_operands_count() {
    // `{t.a}-{t.b}` (no spaces) — both fields are value references and count.
    assert_eq!(field_names("{t.a}-{t.b}"), vec!["t.a", "t.b"]);
    assert_eq!(field_names("{t.a} - {t.b}"), vec!["t.a", "t.b"]);
}

#[test]
fn no_whitespace_aggregation_group_arg_still_excluded() {
    // `Sum({t.x},{t.grp})` with no spaces must behave like the spaced form.
    let tight = refs::references("Sum({t.x},{t.grp})");
    let spaced = refs::references("Sum( {t.x} , {t.grp} )");
    for refs in [&tight, &spaced] {
        let grp = refs.iter().find(|r| r.name == "t.grp").unwrap();
        let x = refs.iter().find(|r| r.name == "t.x").unwrap();
        assert!(grp.is_aggregation_group_arg());
        assert!(!x.is_aggregation_group_arg());
    }
}

#[test]
fn never_panics_on_garbage() {
    for body in ["", "{{{{", "Sum(", ",,,)", "@#$%^&*", "{?", "{@", "{%}{"] {
        let _ = refs::references(body);
    }
}
