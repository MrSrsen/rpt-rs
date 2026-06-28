//! Error-tolerant tokenizer for Crystal (primary) and Basic formula syntax.
//!
//! Lexing rules:
//! - `//` inside a string literal is NOT a comment.
//! - doubled-quote escaping inside string literals (`""` → `"`, `''` → `'` in Crystal).
//! - a `{...}` reference is a single token (read to the first `}`).
//!
//! Two deviations from the engine, chosen so reference extraction on the counting path can never
//! lose tokens:
//! - A string literal runs to its matching delimiter or EOF, **spanning newlines**, rather than
//!   erroring at a newline (an unterminated quote swallows the rest, not re-exposing it as code).
//! - A `{...}` with no closing `}` before the next newline/EOF is NOT treated as a reference; the
//!   `{` is emitted as [`TokenKind::Unknown`] so a stray brace cannot fabricate a reference.

use super::token::{op, RefKind, Syntax, Token, TokenKind};

/// Tokenize `src` under `syntax`. Never panics; unrecognised bytes become [`TokenKind::Unknown`].
/// The final token is always [`TokenKind::Eof`].
pub fn tokenize(src: &str, syntax: Syntax) -> Vec<Token> {
    let b = src.as_bytes();
    let n = b.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let c = b[i];
        match c {
            b' ' | b'\t' | b'\r' => i += 1,
            b'\n' => {
                out.push(Token::new(TokenKind::Newline, i, i + 1, "\n"));
                i += 1;
            }
            // `//` line comment (both syntaxes).
            b'/' if i + 1 < n && b[i + 1] == b'/' => {
                let start = i;
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
                out.push(Token::new(TokenKind::Comment, start, i, &src[start..i]));
            }
            // Basic `'` line comment (in Crystal `'` is a string delimiter — handled below).
            b'\'' if syntax == Syntax::Basic => {
                let start = i;
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
                out.push(Token::new(TokenKind::Comment, start, i, &src[start..i]));
            }
            // String literal: Crystal `"`/`'`, Basic `"` only (Basic `'` handled above).
            b'"' | b'\'' => {
                let (tok, ni) = scan_string(src, b, n, i);
                out.push(tok);
                i = ni;
            }
            // Reference token `{...}`.
            b'{' => {
                if let Some((tok, ni)) = scan_reference(src, b, n, i) {
                    out.push(tok);
                    i = ni;
                } else {
                    out.push(Token::new(TokenKind::Unknown, i, i + 1, "{"));
                    i += 1;
                }
            }
            // Date/time literal `#...#` (same-line). Internal grammar deferred.
            b'#' => {
                if let Some((tok, ni)) = scan_date(src, b, n, i) {
                    out.push(tok);
                    i = ni;
                } else {
                    out.push(Token::new(TokenKind::Unknown, i, i + 1, "#"));
                    i += 1;
                }
            }
            b'0'..=b'9' => {
                let (tok, ni) = scan_number(src, b, n, i);
                out.push(tok);
                i = ni;
            }
            b'.' if i + 1 < n && b[i + 1].is_ascii_digit() => {
                let (tok, ni) = scan_number(src, b, n, i);
                out.push(tok);
                i = ni;
            }
            c if is_ident_start(c) => {
                let start = i;
                i += 1;
                while i < n && is_ident_cont(b[i]) {
                    i += 1;
                }
                let text = &src[start..i];
                // Basic `Rem` keyword = line comment to EOL.
                if syntax == Syntax::Basic && text.eq_ignore_ascii_case("rem") {
                    let cstart = start;
                    while i < n && b[i] != b'\n' {
                        i += 1;
                    }
                    out.push(Token::new(TokenKind::Comment, cstart, i, &src[cstart..i]));
                } else {
                    out.push(Token::new(TokenKind::Ident, start, i, text));
                }
            }
            _ => {
                let (tok, ni) = scan_op(src, b, n, i);
                out.push(tok);
                i = ni;
            }
        }
    }
    out.push(Token::new(TokenKind::Eof, n, n, ""));
    out
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}
fn is_ident_cont(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// UTF-8 lead-byte length (1..=4); 1 for ASCII / invalid lead bytes (so we always advance).
fn char_len(c: u8) -> usize {
    if c < 0x80 {
        1
    } else if c >> 5 == 0b110 {
        2
    } else if c >> 4 == 0b1110 {
        3
    } else if c >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

/// Scan a string literal starting at the opening delimiter `b[i]`. Resolves doubled-quote
/// escapes; runs to the matching delimiter or EOF (spanning newlines — see module docs).
fn scan_string(src: &str, b: &[u8], n: usize, i: usize) -> (Token, usize) {
    let q = b[i];
    let start = i;
    let mut j = i + 1;
    let mut buf: Vec<u8> = Vec::new();
    loop {
        if j >= n {
            break;
        }
        let ch = b[j];
        if ch == q {
            if j + 1 < n && b[j + 1] == q {
                buf.push(q); // doubled delimiter → literal
                j += 2;
                continue;
            }
            j += 1; // closing delimiter
            break;
        }
        buf.push(ch);
        j += 1;
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    let _ = src;
    (Token::new(TokenKind::Str, start, j, text), j)
}

/// Scan a `{...}` reference. Returns `None` (caller emits a stray-`{`) when no closing `}`
/// appears before the next newline / EOF.
fn scan_reference(src: &str, b: &[u8], n: usize, i: usize) -> Option<(Token, usize)> {
    let mut j = i + 1;
    while j < n && b[j] != b'}' && b[j] != b'\n' {
        j += 1;
    }
    if j >= n || b[j] != b'}' {
        return None;
    }
    let inner = &src[i + 1..j]; // raw inner, sigil included
    let (kind, name) = classify_reference(inner);
    Some((
        Token::new(TokenKind::Reference(kind), i, j + 1, name),
        j + 1,
    ))
}

/// Classify a `{...}` inner string by its sigil and return `(kind, sigil-stripped name)`.
fn classify_reference(inner: &str) -> (RefKind, String) {
    match inner.as_bytes().first() {
        Some(b'?') => (RefKind::Parameter, inner[1..].to_string()),
        Some(b'@') => (RefKind::Formula, inner[1..].to_string()),
        Some(b'#') => (RefKind::RunningTotal, inner[1..].to_string()),
        Some(b'%') => (RefKind::SqlExpr, inner[1..].to_string()),
        _ => (RefKind::Field, inner.to_string()),
    }
}

/// Scan a `#...#` date literal on a single line. Returns `None` when no closing `#` is found
/// before the next newline / EOF (caller emits a stray `#`). The internals are not parsed; date
/// bodies carry no braces/parens/commas so this never hides a reference.
fn scan_date(src: &str, b: &[u8], n: usize, i: usize) -> Option<(Token, usize)> {
    let mut j = i + 1;
    while j < n && b[j] != b'#' && b[j] != b'\n' {
        j += 1;
    }
    if j >= n || b[j] != b'#' {
        return None;
    }
    Some((
        Token::new(TokenKind::DateLit, i, j + 1, &src[i..j + 1]),
        j + 1,
    ))
}

fn scan_number(src: &str, b: &[u8], n: usize, i: usize) -> (Token, usize) {
    let start = i;
    let mut j = i;
    while j < n && b[j].is_ascii_digit() {
        j += 1;
    }
    if j < n && b[j] == b'.' {
        j += 1;
        while j < n && b[j].is_ascii_digit() {
            j += 1;
        }
    }
    (Token::new(TokenKind::Number, start, j, &src[start..j]), j)
}

/// Scan an operator/punctuation token (two-char forms first). Unrecognised bytes become
/// [`TokenKind::Unknown`], advancing by one full UTF-8 char so spans stay on char boundaries.
fn scan_op(src: &str, b: &[u8], n: usize, i: usize) -> (Token, usize) {
    let two = |code: u8| {
        (
            Token::new(TokenKind::Op(code), i, i + 2, &src[i..i + 2]),
            i + 2,
        )
    };
    if i + 1 < n {
        match (b[i], b[i + 1]) {
            (b':', b'=') => return two(op::ASSIGN),
            (b'<', b'>') => return two(op::NE),
            (b'<', b'=') => return two(op::LE),
            (b'>', b'=') => return two(op::GE),
            _ => {}
        }
    }
    let one = |code: u8| {
        (
            Token::new(TokenKind::Op(code), i, i + 1, &src[i..i + 1]),
            i + 1,
        )
    };
    match b[i] {
        b';' => one(op::SEMI),
        b'(' => one(op::LPAREN),
        b')' => one(op::RPAREN),
        b'[' => one(op::LBRACKET),
        b']' => one(op::RBRACKET),
        b',' => one(op::COMMA),
        b'%' => one(op::PERCENT),
        b'*' => one(op::STAR),
        b'/' => one(op::SLASH),
        b'^' => one(op::CARET),
        b'\\' => one(op::BACKSLASH),
        b'&' => one(op::AMP),
        b'+' => one(op::PLUS),
        b'-' => one(op::MINUS),
        b'$' => one(op::DOLLAR),
        b'<' => one(op::LT),
        b'>' => one(op::GT),
        b'=' => one(op::EQ),
        b':' => one(op::COLON),
        c => {
            let len = char_len(c).min(n - i);
            (
                Token::new(TokenKind::Unknown, i, i + len, &src[i..i + len]),
                i + len,
            )
        }
    }
}
