//! Render a record-/group-selection formula in the canonical form the engine produces from its
//! compiled selection tree. This canonical form (not the stored body) is emitted when the formula
//! is SQL-pushable; otherwise the stored text is emitted verbatim.
//!
//! Gate (from the stored body): re-render iff it contains no `If…Then…Else` AND its root is a
//! relational/logical operator. Canonical form: one outer `( … )`; each boolean operand (and each
//! `<> <= >=` comparison) wrapped; `and/or/not/in` upper-cased; `<>` never `!=`; string literals
//! single→double quoted (except `IN` operands keep authored quotes); `{table.field}` refs re-cased
//! to the schema's canonical alias. Spacing is a token-pad model: each token has a left/right pad,
//! and the gap between two tokens is the sum.

use std::collections::HashMap;

/// Render `body` as the engine's canonical selection text, or return `None` to emit it verbatim.
/// `alias_canon` maps a lowercased `alias.field` to the schema's canonical casing; it is applied to
/// `{alias.field}` refs only when the formula is SQL-pushable (see `canon` below).
pub(crate) fn render_selection(
    body: &str,
    alias_canon: &HashMap<String, String>,
) -> Option<String> {
    let toks = lex(body);
    // Gate: an `If` anywhere means the formula is not SQL-pushable (kept verbatim).
    if toks
        .iter()
        .any(|t| matches!(t, Tok::Id(s) if s.eq_ignore_ascii_case("if")))
    {
        return None;
    }
    let node = Parser { t: &toks, i: 0 }.expr();
    // Only re-render when the root is a relational/logical operator.
    if !matches!(
        node,
        N::Cmp(..) | N::In(..) | N::And(..) | N::Or(..) | N::Not(..)
    ) {
        return None;
    }
    // The engine only re-serializes the canonical form for a selection that touches the datasource,
    // i.e. one that references at least one `{alias.field}`. A formula over only builtins/constants
    // (e.g. `RecordNumber <= 5`) is kept verbatim.
    let is_db_ref = |s: &str| {
        let inner = s.trim_start_matches('{').trim_end_matches('}');
        inner.contains('.') && !inner.starts_with(['?', '@', '#', '%'])
    };
    if !toks
        .iter()
        .any(|t| matches!(t, Tok::Ref(s) if is_db_ref(s)))
    {
        return None;
    }
    // `{alias.field}` casing is canonicalised to the schema only when the formula is pushed to SQL.
    // A reference to another formula field (`{@…}`) blocks push-down, so the selection is evaluated
    // in-memory and every identifier keeps its authored casing instead.
    let canon = if toks
        .iter()
        .any(|t| matches!(t, Tok::Ref(s) if s.starts_with("{@")))
    {
        None
    } else {
        Some(alias_canon)
    };
    let mut e = Emit::new(canon);
    e.add("(", PAD_GROUP_L);
    e.node(&node);
    e.add(")", PAD_GROUP_R);
    Some(e.render())
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ref(String),
    Str(String),
    Num(String),
    Op(String), // <= >= <> < > =
    LP,
    RP,
    Comma,
    Kw(String), // and or not in to (lowercased)
    Id(String),
    Other(char),
}

fn lex(s: &str) -> Vec<Tok> {
    let b: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '{' {
            let start = i;
            while i < b.len() && b[i] != '}' {
                i += 1;
            }
            if i < b.len() {
                i += 1;
            }
            out.push(Tok::Ref(b[start..i].iter().collect()));
        } else if c == '"' || c == '\'' {
            let q = c;
            let start = i;
            i += 1;
            while i < b.len() && b[i] != q {
                i += 1;
            }
            if i < b.len() {
                i += 1;
            }
            out.push(Tok::Str(b[start..i].iter().collect()));
        } else if c.is_ascii_digit() {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == '.') {
                i += 1;
            }
            out.push(Tok::Num(b[start..i].iter().collect()));
        } else if matches!(c, '<' | '>' | '=') {
            // two-char <= >= <>, else single
            if c != '=' && i + 1 < b.len() && (b[i + 1] == '=' || b[i + 1] == '>') {
                out.push(Tok::Op([c, b[i + 1]].iter().collect()));
                i += 2;
            } else {
                out.push(Tok::Op(c.to_string()));
                i += 1;
            }
        } else if c == '(' {
            out.push(Tok::LP);
            i += 1;
        } else if c == ')' {
            out.push(Tok::RP);
            i += 1;
        } else if c == ',' {
            out.push(Tok::Comma);
            i += 1;
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_') {
                i += 1;
            }
            let w: String = b[start..i].iter().collect();
            let lw = w.to_ascii_lowercase();
            if matches!(lw.as_str(), "and" | "or" | "not" | "in" | "to") {
                out.push(Tok::Kw(lw));
            } else {
                out.push(Tok::Id(w));
            }
        } else {
            out.push(Tok::Other(c));
            i += 1;
        }
    }
    out
}

/// The selection-formula mini-AST (boolean expression grammar only).
enum N {
    Or(Box<N>, Box<N>),
    And(Box<N>, Box<N>),
    Not(Box<N>),
    Cmp(String, Box<N>, Box<N>),
    In(Box<N>, Box<N>),
    Range(Box<N>, Box<N>),
    Paren(Box<N>),
    Neg(Box<N>),
    Call(String, Vec<N>),
    Ref(String),
    Str(String),
    Num(String),
    Id(String),
}

struct Parser<'a> {
    t: &'a [Tok],
    i: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn eat(&mut self) -> Option<&Tok> {
        let x = self.t.get(self.i);
        self.i += 1;
        x
    }
    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Some(Tok::Kw(s)) if s == k)
    }
    fn expr(&mut self) -> N {
        self.or()
    }
    fn or(&mut self) -> N {
        let mut l = self.and();
        while self.is_kw("or") {
            self.i += 1;
            l = N::Or(Box::new(l), Box::new(self.and()));
        }
        l
    }
    fn and(&mut self) -> N {
        let mut l = self.not();
        while self.is_kw("and") {
            self.i += 1;
            l = N::And(Box::new(l), Box::new(self.not()));
        }
        l
    }
    fn not(&mut self) -> N {
        if self.is_kw("not") {
            self.i += 1;
            return N::Not(Box::new(self.not()));
        }
        self.cmp()
    }
    fn cmp(&mut self) -> N {
        let l = self.un();
        match self.peek() {
            Some(Tok::Op(o)) => {
                let o = o.clone();
                self.i += 1;
                N::Cmp(o, Box::new(l), Box::new(self.un()))
            }
            Some(Tok::Kw(k)) if k == "in" => {
                self.i += 1;
                let r = self.un();
                if self.is_kw("to") {
                    self.i += 1;
                    N::In(
                        Box::new(l),
                        Box::new(N::Range(Box::new(r), Box::new(self.un()))),
                    )
                } else {
                    N::In(Box::new(l), Box::new(r))
                }
            }
            _ => l,
        }
    }
    fn un(&mut self) -> N {
        if matches!(self.peek(), Some(Tok::Other('-'))) {
            self.i += 1;
            return N::Neg(Box::new(self.un()));
        }
        self.prim()
    }
    fn prim(&mut self) -> N {
        match self.eat() {
            Some(Tok::LP) => {
                let e = self.expr();
                if matches!(self.peek(), Some(Tok::RP)) {
                    self.i += 1;
                }
                N::Paren(Box::new(e))
            }
            Some(Tok::Ref(s)) => N::Ref(s.clone()),
            Some(Tok::Str(s)) => N::Str(s.clone()),
            Some(Tok::Num(s)) => N::Num(s.clone()),
            Some(Tok::Id(s)) => {
                let name = s.clone();
                if matches!(self.peek(), Some(Tok::LP)) {
                    self.i += 1;
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some(Tok::RP)) {
                        args.push(self.expr());
                        while matches!(self.peek(), Some(Tok::Comma)) {
                            self.i += 1;
                            args.push(self.expr());
                        }
                    }
                    if matches!(self.peek(), Some(Tok::RP)) {
                        self.i += 1;
                    }
                    N::Call(name, args)
                } else {
                    N::Id(name)
                }
            }
            other => N::Id(other.map(tok_text).unwrap_or_else(|| "?".into())),
        }
    }
}

fn tok_text(t: &Tok) -> String {
    match t {
        Tok::Ref(s) | Tok::Str(s) | Tok::Num(s) | Tok::Op(s) | Tok::Kw(s) | Tok::Id(s) => s.clone(),
        Tok::LP => "(".into(),
        Tok::RP => ")".into(),
        Tok::Comma => ",".into(),
        Tok::Other(c) => c.to_string(),
    }
}

// token (left, right) pads
const PAD_GROUP_L: (usize, usize) = (1, 2);
const PAD_GROUP_R: (usize, usize) = (2, 1);
const PAD_BOOL: (usize, usize) = (2, 2);
const PAD_REL: (usize, usize) = (1, 1);
const PAD_VAL: (usize, usize) = (0, 0);

struct Emit<'a> {
    toks: Vec<(String, (usize, usize))>,
    /// `Some` to re-case `{alias.field}` refs to the schema canonical, `None` to keep authored.
    canon: Option<&'a HashMap<String, String>>,
}

impl<'a> Emit<'a> {
    fn new(canon: Option<&'a HashMap<String, String>>) -> Self {
        Emit {
            toks: Vec::new(),
            canon,
        }
    }
    fn add(&mut self, t: &str, pad: (usize, usize)) {
        self.toks.push((t.to_string(), pad));
    }
    fn render(&self) -> String {
        let mut out = String::new();
        for (i, (t, p)) in self.toks.iter().enumerate() {
            let lead = if i == 0 {
                p.0
            } else {
                self.toks[i - 1].1 .1 + p.0
            };
            out.push_str(&" ".repeat(lead));
            out.push_str(t);
        }
        // trailing: last token's right pad + one global trailing space
        out.push_str(&" ".repeat(self.toks.last().map(|(_, p)| p.1).unwrap_or(0) + 1));
        out
    }
    /// A `{alias.field}` ref: re-cased to the schema canonical when the formula is SQL-pushable,
    /// else kept as authored. Params/formulas (`{?…}`/`{@…}`) are always verbatim.
    fn ref_text(&self, raw: &str) -> String {
        let Some(canon) = self.canon else {
            return raw.to_string();
        };
        let inner = raw.trim_start_matches('{').trim_end_matches('}');
        if inner.starts_with('?')
            || inner.starts_with('@')
            || inner.starts_with('#')
            || inner.starts_with('%')
        {
            return raw.to_string();
        }
        canon
            .get(&inner.to_ascii_lowercase())
            .map(|c| format!("{{{c}}}"))
            .unwrap_or_else(|| raw.to_string())
    }
    fn value_text(&self, n: &N, in_ctx: bool) -> String {
        match n {
            N::Ref(s) => self.ref_text(s),
            N::Str(s) => {
                if in_ctx {
                    s.clone() // IN operand keeps authored quotes
                } else {
                    format!("\"{}\"", &s[1..s.len() - 1]) // single -> double
                }
            }
            N::Num(s) | N::Id(s) => s.clone(),
            N::Neg(x) => format!("-{}", self.value_text(x, in_ctx)),
            N::Call(name, args) => format!(
                "{name}({})",
                args.iter()
                    .map(|a| self.value_text(a, in_ctx))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            N::Paren(x) => self.value_text(x, in_ctx),
            _ => "?".into(),
        }
    }
    fn emit_value(&mut self, n: &N, in_ctx: bool) {
        let t = self.value_text(n, in_ctx);
        self.add(&t, PAD_VAL);
    }
    fn node(&mut self, n: &N) {
        let n = unwrap(n);
        match n {
            N::Cmp(op, l, r) => {
                let lt = self.value_text(l, false);
                self.add(&lt, PAD_VAL);
                // The engine drops the operator's leading pad when the left operand renders with
                // internal whitespace — e.g. a multi-word field name `{Command.Client ID}=`.
                let pad = if lt.contains(char::is_whitespace) {
                    (0, PAD_REL.1)
                } else {
                    PAD_REL
                };
                self.add(op, pad);
                self.emit_value(r, false);
            }
            N::In(l, r) => {
                self.emit_value(l, true);
                self.add("IN", PAD_REL);
                match unwrap(r) {
                    N::Range(a, b) => {
                        self.emit_value(a, true);
                        self.add("to", PAD_REL);
                        self.emit_value(b, true);
                    }
                    other => self.emit_value(other, true),
                }
            }
            N::Not(x) => {
                self.add("NOT", PAD_BOOL);
                self.operand(x);
            }
            N::And(l, r) => {
                self.operand(l);
                self.add("AND", PAD_BOOL);
                self.operand(r);
            }
            N::Or(l, r) => {
                self.operand(l);
                self.add("OR", PAD_BOOL);
                self.operand(r);
            }
            other => self.emit_value(other, false),
        }
    }
    fn operand(&mut self, n: &N) {
        let n = unwrap(n);
        if need_wrap(n) {
            self.add("(", PAD_GROUP_L);
            self.node(n);
            self.add(")", PAD_GROUP_R);
        } else {
            self.node(n);
        }
    }
}

fn unwrap(mut n: &N) -> &N {
    while let N::Paren(x) = n {
        n = x;
    }
    n
}

fn need_wrap(n: &N) -> bool {
    match unwrap(n) {
        N::And(..) | N::Or(..) | N::Not(..) => true,
        N::Cmp(op, ..) => matches!(op.as_str(), "<>" | "<=" | ">="),
        _ => false,
    }
}
