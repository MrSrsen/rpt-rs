//! Tokenizer tests: reference token forms, comments in/out of strings, doubled-quote escapes.

use crate::token::{RefKind, Syntax, TokenKind};
use crate::tokenize;

#[test]
fn lex_basic_reference_token() {
    let toks = tokenize("{Command.drug_name}", Syntax::Crystal);
    assert_eq!(toks[0].kind, TokenKind::Reference(RefKind::Field));
    assert_eq!(toks[0].text, "Command.drug_name");
    assert!(matches!(toks.last().unwrap().kind, TokenKind::Eof));
}

#[test]
fn lex_all_reference_sigils() {
    let kinds: Vec<_> = tokenize("{t.f}{?p}{@fm}{#rt}{%s}", Syntax::Crystal)
        .into_iter()
        .filter_map(|t| match t.kind {
            TokenKind::Reference(k) => Some((k, t.text)),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            (RefKind::Field, "t.f".into()),
            (RefKind::Parameter, "p".into()),
            (RefKind::Formula, "fm".into()),
            (RefKind::RunningTotal, "rt".into()),
            (RefKind::SqlExpr, "s".into()),
        ]
    );
}

#[test]
fn line_comment_to_eol() {
    let toks = tokenize("a // comment {t.f}\nb", Syntax::Crystal);
    assert!(toks.iter().any(|t| t.kind == TokenKind::Comment));
    // The field token in the comment is NOT lexed as a reference.
    assert!(!toks
        .iter()
        .any(|t| matches!(t.kind, TokenKind::Reference(_))));
}

#[test]
fn double_slash_inside_string_is_not_a_comment() {
    let toks = tokenize("\"http://x\"", Syntax::Crystal);
    assert_eq!(toks[0].kind, TokenKind::Str);
    assert_eq!(toks[0].text, "http://x");
    assert!(!toks.iter().any(|t| t.kind == TokenKind::Comment));
}

#[test]
fn doubled_quote_escape_resolves_to_single() {
    let toks = tokenize("\"a\"\"b\"", Syntax::Crystal);
    assert_eq!(toks[0].kind, TokenKind::Str);
    assert_eq!(toks[0].text, "a\"b");
}

#[test]
fn single_quote_string_in_crystal() {
    let toks = tokenize("'it''s'", Syntax::Crystal);
    assert_eq!(toks[0].kind, TokenKind::Str);
    assert_eq!(toks[0].text, "it's");
}

#[test]
fn basic_single_quote_is_comment() {
    let toks = tokenize("a ' a comment", Syntax::Basic);
    assert!(toks.iter().any(|t| t.kind == TokenKind::Comment));
    assert!(!toks.iter().any(|t| t.kind == TokenKind::Str));
}

#[test]
fn basic_rem_keyword_is_comment() {
    let toks = tokenize("x Rem trailing note", Syntax::Basic);
    assert!(toks.iter().any(|t| t.kind == TokenKind::Comment));
}

#[test]
fn unterminated_reference_emits_stray_brace_not_ref() {
    // No `}` before newline → `{` is Unknown, and the next line's ref still lexes.
    let toks = tokenize("{t.f\n{t.g}", Syntax::Crystal);
    let refs: Vec<_> = toks
        .iter()
        .filter_map(|t| match &t.kind {
            TokenKind::Reference(_) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(refs, vec!["t.g"]);
}

#[test]
fn operators_and_two_char_ops() {
    use crate::token::op;
    let toks = tokenize("a <= b <> c := d", Syntax::Crystal);
    let ops: Vec<u8> = toks
        .iter()
        .filter_map(|t| match t.kind {
            TokenKind::Op(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(ops, vec![op::LE, op::NE, op::ASSIGN]);
}

#[test]
fn unicode_in_string_does_not_panic_and_is_preserved() {
    let toks = tokenize("\"héllo → wörld\" & {t.f}", Syntax::Crystal);
    assert_eq!(toks[0].kind, TokenKind::Str);
    assert_eq!(toks[0].text, "héllo → wörld");
}

#[test]
fn date_literal_same_line() {
    let toks = tokenize("#2020-01-01#", Syntax::Crystal);
    assert_eq!(toks[0].kind, TokenKind::DateLit);
}

#[test]
fn no_whitespace_minus_lexes_same_as_spaced() {
    use crate::token::op;
    let kinds = |s| {
        tokenize(s, Syntax::Crystal)
            .into_iter()
            .filter(|t| !matches!(t.kind, TokenKind::Eof))
            .map(|t| t.kind)
            .collect::<Vec<_>>()
    };
    let expected = vec![TokenKind::Ident, TokenKind::Op(op::MINUS), TokenKind::Ident];
    assert_eq!(kinds("a-b"), expected);
    assert_eq!(kinds("a - b"), expected);
}

#[test]
fn no_whitespace_between_reference_tokens() {
    // `{t.a}-{t.b}` and `{t.a}+{t.b}` must split into two reference tokens with the operator between.
    let toks = tokenize("{t.a}-{t.b}", Syntax::Crystal);
    let refs: Vec<_> = toks
        .iter()
        .filter_map(|t| match &t.kind {
            TokenKind::Reference(_) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(refs, vec!["t.a", "t.b"]);
}

#[test]
fn split_reference_sigils() {
    use crate::token::split_reference;
    assert_eq!(
        split_reference("table.field"),
        (RefKind::Field, "table.field")
    );
    assert_eq!(split_reference("?p"), (RefKind::Parameter, "p"));
    assert_eq!(
        split_reference("@From Date"),
        (RefKind::Formula, "From Date")
    );
    assert_eq!(split_reference("#rt"), (RefKind::RunningTotal, "rt"));
    assert_eq!(split_reference("%sql"), (RefKind::SqlExpr, "sql"));
    assert_eq!(split_reference(""), (RefKind::Field, ""));
}

#[test]
fn brace_groups_scan() {
    use crate::token::brace_groups;
    let g: Vec<_> = brace_groups(" ({Command.d}, \"Daily\") + {@x}").collect();
    assert_eq!(g, vec!["{Command.d}", "{@x}"]);
    // No braces, and an unclosed brace, both yield nothing further.
    assert_eq!(brace_groups("no refs").count(), 0);
    assert_eq!(brace_groups("a{b").count(), 0);
    assert_eq!(brace_groups("{}").collect::<Vec<_>>(), vec!["{}"]);
}
