//! Reference-extraction tests: every reference form, enclosing-call attribution, nested calls,
//! comments/strings excluded.

use crate::refs;
use crate::token::{RefKind, Syntax};

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
    assert!(!refs::references(body)
        .iter()
        .any(|r| r.kind == RefKind::Formula));
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

/// Every argument of a call carries that call's name, whichever argument slot it sits in.
#[test]
fn every_argument_carries_the_enclosing_call_name() {
    let refs = refs::references("Sum({Command.value}, {Command.group})");
    for r in &refs {
        assert_eq!(r.enclosing_fn.as_deref(), Some("Sum"), "{}", r.name);
    }
}

/// A reference is attributed to the innermost call that encloses it, not the outermost.
#[test]
fn nested_call_uses_innermost_frame() {
    let refs = refs::references("IIf(true, Sum({a.x}, {a.y}), {a.z})");
    let of = |n: &str| {
        refs.iter()
            .find(|r| r.name == n)
            .unwrap()
            .enclosing_fn
            .clone()
    };
    assert_eq!(of("a.x").as_deref(), Some("Sum"));
    assert_eq!(of("a.y").as_deref(), Some("Sum"));
    assert_eq!(of("a.z").as_deref(), Some("IIf"));
}

/// Whitespace, newlines, and comments between the function name and its `(` do not break the
/// name→call association; the name is matched case-insensitively by [`refs::is_aggregation_function`].
#[test]
fn call_name_survives_whitespace_and_newlines() {
    let refs = refs::references("SUM\n ( {a.x} ,\n {a.grp} )");
    for r in &refs {
        assert_eq!(r.enclosing_fn.as_deref(), Some("SUM"), "{}", r.name);
        assert!(refs::is_aggregation_function(
            r.enclosing_fn.as_deref().unwrap()
        ));
    }
}

#[test]
fn enclosing_fn_after_close_paren_is_none() {
    // `)(` — the `(` has no function ident before it.
    let refs = refs::references("(a)({t.f})");
    let f = refs.iter().find(|r| r.name == "t.f").unwrap();
    assert_eq!(f.enclosing_fn, None);
}

/// References of every kind are yielded in source order, each tagged with its kind.
#[test]
fn mixed_kinds_yield_in_source_order() {
    let refs = refs::references("{@A} + {?P} + Sum({t.f}) + {@B}");
    let got: Vec<_> = refs.iter().map(|r| (r.kind, r.name.as_str())).collect();
    assert_eq!(
        got,
        vec![
            (RefKind::Formula, "A"),
            (RefKind::Parameter, "P"),
            (RefKind::Field, "t.f"),
            (RefKind::Formula, "B"),
        ]
    );
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

/// `Sum({t.x},{t.grp})` with no spaces extracts identically to the spaced form.
#[test]
fn no_whitespace_call_extracts_like_the_spaced_form() {
    let tight = refs::references("Sum({t.x},{t.grp})");
    let spaced = refs::references("Sum( {t.x} , {t.grp} )");
    assert_eq!(tight, spaced);
    assert_eq!(tight.len(), 2);
    assert!(tight
        .iter()
        .all(|r| r.enclosing_fn.as_deref() == Some("Sum")));
}

#[test]
fn never_panics_on_garbage() {
    for body in ["", "{{{{", "Sum(", ",,,)", "@#$%^&*", "{?", "{@", "{%}{"] {
        let _ = refs::references(body);
    }
}
