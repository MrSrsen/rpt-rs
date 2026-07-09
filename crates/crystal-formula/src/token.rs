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

/// Split a `{...}` reference's **inner** string (the braces already removed) into its [`RefKind`]
/// and the sigil-stripped name: `"@From Date"` → `(Formula, "From Date")`, `"table.field"` →
/// `(Field, "table.field")`. This is the one place the reference sigils are decoded.
pub fn split_reference(inner: &str) -> (RefKind, &str) {
    match inner.as_bytes().first() {
        Some(b'?') => (RefKind::Parameter, &inner[1..]),
        Some(b'@') => (RefKind::Formula, &inner[1..]),
        Some(b'#') => (RefKind::RunningTotal, &inner[1..]),
        Some(b'%') => (RefKind::SqlExpr, &inner[1..]),
        _ => (RefKind::Field, inner),
    }
}

/// Trim surrounding whitespace and `{ }` braces from a display reference:
/// `" {Table.field} "` → `"Table.field"`. The shared first step of reference-name normalization.
pub fn strip_braces(s: &str) -> &str {
    s.trim().trim_matches(['{', '}'])
}

/// The bare name after the last `.` of a reference — the segment a table-qualified field is matched
/// by (`"Command.Region"` → `"Region"`, `"Amount"` → `"Amount"`).
pub fn last_segment(s: &str) -> &str {
    s.rsplit('.').next().unwrap_or(s)
}

/// The short field key a display reference resolves to: brace-stripped, last `.`-segment, lowercased
/// (`"{Command.Region}"` → `"region"`). The canonical key for name-based field matching across the
/// data/layout pipeline.
pub fn short_name(s: &str) -> String {
    last_segment(strip_braces(s)).to_lowercase()
}

/// Iterate the `{…}` reference groups of `s` in order, each **including** its enclosing braces
/// (`"f({a}, {b})"` → `"{a}"`, `"{b}"`). An unclosed trailing `{` is skipped. References don't
/// nest, so this is a simple brace-pair scan.
pub fn brace_groups(s: &str) -> impl Iterator<Item = &str> {
    let mut i = 0;
    std::iter::from_fn(move || {
        let start = i + s[i..].find('{')?;
        let end = start + s[start..].find('}')?; // index of the closing `}`
        i = end + 1;
        Some(&s[start..=end])
    })
}

/// Unified punctuation/operator token codes (shared Crystal+Basic).
pub mod op {
    /// `:=` assignment (Crystal).
    pub const ASSIGN: u8 = 0x1d;
    /// `;` statement separator.
    pub const SEMI: u8 = 0x1e;
    /// `(` open paren.
    pub const LPAREN: u8 = 0x1f;
    /// `)` close paren.
    pub const RPAREN: u8 = 0x20;
    /// `[` open bracket (array/subscript).
    pub const LBRACKET: u8 = 0x21;
    /// `]` close bracket.
    pub const RBRACKET: u8 = 0x22;
    /// `,` argument/element separator.
    pub const COMMA: u8 = 0x23;
    /// `%` percent.
    pub const PERCENT: u8 = 0x24;
    /// `*` multiply.
    pub const STAR: u8 = 0x26;
    /// `/` divide.
    pub const SLASH: u8 = 0x27;
    /// `^` exponentiation.
    pub const CARET: u8 = 0x28;
    /// `\` integer division.
    pub const BACKSLASH: u8 = 0x29;
    /// `&` string concatenation.
    pub const AMP: u8 = 0x2b;
    /// `+` add / unary plus.
    pub const PLUS: u8 = 0x2c;
    /// `-` subtract / unary minus.
    pub const MINUS: u8 = 0x2d;
    /// `$` currency prefix.
    pub const DOLLAR: u8 = 0x2e;
    /// `<` less than.
    pub const LT: u8 = 0x39;
    /// `>` greater than.
    pub const GT: u8 = 0x3a;
    /// `>=` greater than or equal.
    pub const GE: u8 = 0x3b;
    /// `<=` less than or equal.
    pub const LE: u8 = 0x3c;
    /// `=` equality (also Basic assignment).
    pub const EQ: u8 = 0x3d;
    /// `<>` inequality.
    pub const NE: u8 = 0x3e;
    /// Newline (Basic statement separator).
    pub const NEWLINE: u8 = 0x58;
    /// `:` colon.
    pub const COLON: u8 = 0x59;

    // Word operators (lex as identifiers; the parser assigns these codes).

    /// `Not` logical negation.
    pub const NOT: u8 = 0x25;
    /// `Mod` modulo.
    pub const MOD: u8 = 0x2a;
    /// `To` range, both bounds inclusive.
    pub const RANGE_TO: u8 = 0x2f;
    /// `_To` range, low bound exclusive.
    pub const RANGE_LO_EXCL: u8 = 0x30;
    /// `To_` range, high bound exclusive.
    pub const RANGE_HI_EXCL: u8 = 0x31;
    /// `_To_` range, both bounds exclusive.
    pub const RANGE_BOTH_EXCL: u8 = 0x32;
    /// `In` membership test.
    pub const IN: u8 = 0x38;
    /// `And` logical conjunction.
    pub const AND: u8 = 0x3f;
    /// `Or` logical disjunction.
    pub const OR: u8 = 0x40;
    /// `Xor` exclusive-or.
    pub const XOR: u8 = 0x41;
    /// `Eqv` logical equivalence.
    pub const EQV: u8 = 0x42;
    /// `Imp` logical implication.
    pub const IMP: u8 = 0x43;
    /// `Like` pattern match.
    pub const LIKE: u8 = 0x5a;
    /// `StartsWith` prefix test.
    pub const STARTS_WITH: u8 = 0x5b;

    // Prefix operator node kinds (distinct from the binary `+`/`-` token codes).

    /// Unary prefix `+`.
    pub const UNARY_PLUS: u8 = 0x79;
    /// Unary prefix `-`.
    pub const UNARY_MINUS: u8 = 0x7a;

    /// A printable symbol for an operator code, for diagnostic messages. `?` for a non-operator code.
    pub fn symbol(code: u8) -> &'static str {
        match code {
            AMP => "&",
            PLUS => "+",
            MINUS => "-",
            STAR => "*",
            SLASH => "/",
            BACKSLASH => "\\",
            MOD => "Mod",
            CARET => "^",
            PERCENT => "%",
            LT => "<",
            GT => ">",
            GE => ">=",
            LE => "<=",
            EQ => "=",
            NE => "<>",
            AND => "And",
            OR => "Or",
            XOR => "Xor",
            EQV => "Eqv",
            IMP => "Imp",
            LIKE => "Like",
            STARTS_WITH => "StartsWith",
            NOT => "Not",
            UNARY_MINUS => "-",
            UNARY_PLUS => "+",
            DOLLAR => "$",
            _ => "?",
        }
    }
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
    /// The lexical category of the token.
    pub kind: TokenKind,
    /// Byte offset of the token's first byte in the source.
    pub start: usize,
    /// Byte offset one past the token's last byte in the source.
    pub end: usize,
    /// The token's text payload (see the type-level docs for what it carries per kind).
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
