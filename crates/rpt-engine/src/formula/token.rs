//! Token kinds, reference-token classification, and the unified token codes for the Crystal/Basic
//! formula lexer.

/// Which formula surface syntax to lex. Crystal is the primary; Basic differs only in
/// comment / string / statement-separator handling. The expression grammar and precedence are
/// identical across both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syntax {
    /// Crystal syntax: `//` comments, `"`/`'` string delimiters, `;` statement sep, `:=` assign.
    Crystal,
    /// Basic syntax: `//`/`'`/`Rem` comments, `"`-only strings, newline statement sep, `=` assign.
    Basic,
}

/// The class of a `{...}` reference token, decided by its first inner character (the sigil).
/// The lexer reads the whole `{...}` as one token; the grammar layer is prefix-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefKind {
    /// `{table.field}` — a database field (no sigil).
    Field,
    /// `{?name}` — a parameter.
    Parameter,
    /// `{@name}` — a formula.
    Formula,
    /// `{#name}` — a running total.
    RunningTotal,
    /// `{%name}` — a SQL expression.
    SqlExpr,
}

/// Unified punctuation/operator token codes (shared Crystal+Basic).
pub mod op {
    pub const ASSIGN: u8 = 0x1d; // `:=` (Crystal)
    pub const SEMI: u8 = 0x1e; // `;`
    pub const LPAREN: u8 = 0x1f; // `(`
    pub const RPAREN: u8 = 0x20; // `)`
    pub const LBRACKET: u8 = 0x21; // `[`
    pub const RBRACKET: u8 = 0x22; // `]`
    pub const COMMA: u8 = 0x23; // `,`
    pub const PERCENT: u8 = 0x24; // `%`
    pub const STAR: u8 = 0x26; // `*`
    pub const SLASH: u8 = 0x27; // `/`
    pub const CARET: u8 = 0x28; // `^`
    pub const BACKSLASH: u8 = 0x29; // `\` integer division
    pub const AMP: u8 = 0x2b; // `&` concat
    pub const PLUS: u8 = 0x2c; // `+`
    pub const MINUS: u8 = 0x2d; // `-`
    pub const DOLLAR: u8 = 0x2e; // `$` currency prefix
    pub const LT: u8 = 0x39; // `<`
    pub const GT: u8 = 0x3a; // `>`
    pub const GE: u8 = 0x3b; // `>=`
    pub const LE: u8 = 0x3c; // `<=`
    pub const EQ: u8 = 0x3d; // `=`
    pub const NE: u8 = 0x3e; // `<>`
    pub const NEWLINE: u8 = 0x58; // newline (Basic statement separator)
    pub const COLON: u8 = 0x59; // `:`
}

/// The lexical category of a [`Token`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    /// Identifier / keyword / function name (name resolution happens later, in the parser/deducer).
    Ident,
    /// A `{...}` reference token. The sigil-stripped inner name is carried in [`Token::text`].
    Reference(RefKind),
    /// A string literal. The escape-resolved content is carried in [`Token::text`].
    Str,
    /// A numeric literal.
    Number,
    /// A `#...#` date/time literal (internal grammar deferred).
    DateLit,
    /// An operator or punctuation token; the byte is an [`op`] code.
    Op(u8),
    /// A `//` (or Basic `'` / `Rem`) line comment.
    Comment,
    /// A newline.
    Newline,
    /// End of input.
    Eof,
    /// Any byte the lexer did not recognise (error-tolerant; never panics).
    Unknown,
}

/// A lexed token with its source span `[start, end)` (byte offsets) and a `text` payload.
///
/// `text` carries: the sigil-stripped inner name for [`TokenKind::Reference`], the
/// escape-resolved content for [`TokenKind::Str`], and the verbatim source slice otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

impl Token {
    pub(crate) fn new(kind: TokenKind, start: usize, end: usize, text: impl Into<String>) -> Self {
        Token {
            kind,
            start,
            end,
            text: text.into(),
        }
    }
}
