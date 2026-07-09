//! Hand-written, error-recovering recursive-descent parser over the 17-level operator-precedence
//! ladder and the primary grammar. Crystal syntax is primary; the expression grammar is shared with
//! Basic. The parser never panics: on an unexpected token it records a [`Diagnostic`], emits
//! [`Node::Error`], and makes progress, so it is safe to run on any formula and is a foundation for
//! eval/validation/LSP.
//!
//! Control flow (`If`/`Select…Case`, `For`/`While`/`Do` loops), declarations (`Local/Global/Shared
//! <Type>Var`, Basic `Dim`), and Basic statement bodies all parse to real AST nodes. Crystal
//! `If`/`Select` are expressions; the loops are statements. `Exit For/While/Do` parses to a
//! [`Node::Exit`] that breaks the innermost enclosing loop. Reference *counting* never depends on
//! this parser (see [`super::refs`]).

use super::ast::{ExitKind, Node, VarKind, VarScope};
use super::lexer::tokenize;
use super::token::{op, Syntax, Token, TokenKind};

/// The `<Type>Var` declaration keywords (Crystal syntax).
const TYPE_VAR_KWS: [(&str, VarKind); 7] = [
    ("numbervar", VarKind::Number),
    ("currencyvar", VarKind::Currency),
    ("booleanvar", VarKind::Boolean),
    ("datevar", VarKind::Date),
    ("timevar", VarKind::Time),
    ("datetimevar", VarKind::DateTime),
    ("stringvar", VarKind::String),
];

/// Diagnostic severity. Parser diagnostics are always [`Severity::Error`]; the semantic
/// [`validator`](mod@super::validate) also emits [`Severity::Warning`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Severity {
    /// A hard error (invalid syntax or a semantic violation).
    #[default]
    Error,
    /// A non-fatal warning (semantic validator only).
    Warning,
}

/// A diagnostic with the source span it concerns. Produced by both the [`parser`](self) (syntactic
/// recovery) and the [`validator`](mod@super::validate) (semantic checks); the `severity` field
/// distinguishes hard errors from warnings for an LSP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The human-readable diagnostic message.
    pub message: String,
    /// Byte offset of the span's first byte in the source.
    pub start: usize,
    /// Byte offset one past the span's last byte in the source.
    pub end: usize,
    /// Whether this is an error or a warning.
    pub severity: Severity,
}

/// Parse `src` under `syntax`, returning the (possibly partial) AST and any diagnostics.
pub fn parse(src: &str, syntax: Syntax) -> (Node, Vec<Diagnostic>) {
    // Significant tokens only: comments are never syntactic; newlines are whitespace in Crystal
    // but a statement separator in Basic.
    let toks: Vec<Token> = tokenize(src, syntax)
        .into_iter()
        .filter(|t| {
            let drop = matches!(t.kind, TokenKind::Comment)
                || (matches!(t.kind, TokenKind::Newline) && syntax == Syntax::Crystal);
            !drop
        })
        .collect();
    let mut p = Parser {
        toks,
        pos: 0,
        syntax,
        diags: Vec::new(),
    };
    let node = p.parse_stmt_seq();
    // In Basic syntax the return value is whatever was assigned to the implicit `formula` variable,
    // not the last statement — so read it back at the end when the body assigns it.
    let node = if syntax == Syntax::Basic && assigns_formula(&node) {
        Node::Seq(vec![node, Node::Ident("formula".to_string())])
    } else {
        node
    };
    (node, p.diags)
}

/// Whether a (sub)tree contains an assignment to the Basic `formula` result variable.
fn assigns_formula(node: &Node) -> bool {
    match node {
        Node::Assign { name, .. } => name.eq_ignore_ascii_case("formula"),
        Node::Seq(stmts) => stmts.iter().any(assigns_formula),
        Node::If {
            then, elifs, els, ..
        } => {
            assigns_formula(then)
                || elifs.iter().any(|(_, b)| assigns_formula(b))
                || els.as_deref().is_some_and(assigns_formula)
        }
        Node::While { body, .. } | Node::For { body, .. } => assigns_formula(body),
        _ => false,
    }
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    syntax: Syntax,
    diags: Vec<Diagnostic>,
}

impl Parser {
    fn cur(&self) -> &Token {
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }
    fn at_eof(&self) -> bool {
        matches!(self.cur().kind, TokenKind::Eof)
    }
    fn advance(&mut self) -> Token {
        let t = self.cur().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn is_op(&self, code: u8) -> bool {
        matches!(self.cur().kind, TokenKind::Op(c) if c == code)
    }
    fn eat_op(&mut self, code: u8) -> bool {
        if self.is_op(code) {
            self.advance();
            true
        } else {
            false
        }
    }
    /// If the current token is an identifier equal (case-insensitively) to `kw`, consume it.
    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.is_kw(kw) {
            self.advance();
            true
        } else {
            false
        }
    }
    fn is_kw(&self, kw: &str) -> bool {
        matches!(&self.cur().kind, TokenKind::Ident if self.cur().text.eq_ignore_ascii_case(kw))
    }
    /// If the current ident is one of `names`, return its operator code (without consuming).
    /// Used for word operators (`And`, `Or`, `Mod`, …) which lex as identifiers.
    fn word_op(&self, names: &[(&str, u8)]) -> Option<u8> {
        if let TokenKind::Ident = self.cur().kind {
            let t = &self.cur().text;
            for (n, code) in names {
                if t.eq_ignore_ascii_case(n) {
                    return Some(*code);
                }
            }
        }
        None
    }
    fn diag(&mut self, message: impl Into<String>) {
        let t = self.cur();
        self.diags.push(Diagnostic {
            message: message.into(),
            start: t.start,
            end: t.end,
            severity: Severity::Error,
        });
    }

    // top-level statement sequence: stmt { (";" | newline) stmt }
    fn parse_stmt_seq(&mut self) -> Node {
        self.parse_stmt_seq_until(&[])
    }

    /// A statement sequence that stops (without consuming) at EOF or any block-terminator keyword in
    /// `terminators` (`End`/`Else`/`ElseIf`/`Wend`/`Loop`/`Next`/`Case`) — the foundation for Basic
    /// block bodies. Statement separators (`;`, and newlines in Basic) are skipped between statements.
    fn parse_stmt_seq_until(&mut self, terminators: &[&str]) -> Node {
        let mut stmts = Vec::new();
        loop {
            self.skip_separators();
            if self.at_eof() || self.at_terminator(terminators) {
                break;
            }
            let before = self.pos;
            stmts.push(self.parse_statement());
            if self.pos == before {
                // No progress (stray token) — recover by consuming it.
                self.diag("unexpected token");
                self.advance();
            }
        }
        match stmts.len() {
            0 => Node::Empty,
            1 => stmts.pop().unwrap(),
            _ => Node::Seq(stmts),
        }
    }

    fn skip_separators(&mut self) {
        while self.eat_op(op::SEMI)
            || (self.syntax == Syntax::Basic && matches!(self.cur().kind, TokenKind::Newline))
        {
            if self.syntax == Syntax::Basic && matches!(self.cur().kind, TokenKind::Newline) {
                self.advance();
            }
        }
    }

    /// The current token is a block-terminator keyword the caller is waiting for.
    fn at_terminator(&self, terminators: &[&str]) -> bool {
        terminators.iter().any(|kw| self.is_kw(kw))
    }

    /// A single statement: the keyword-led control-flow forms (loops in both syntaxes; Basic
    /// `If`/`Select Case`/`Dim`/`Exit`), else an expression/assignment (Crystal `If`/`Select` are
    /// expressions handled in `parse_primary`).
    fn parse_statement(&mut self) -> Node {
        if self.is_kw("for") {
            return self.parse_for();
        }
        if self.is_kw("while") {
            return self.parse_while();
        }
        if self.syntax == Syntax::Basic {
            if self.is_kw("do") {
                return self.parse_do();
            }
            if self.is_kw("if") {
                return self.parse_basic_if();
            }
            if self.is_kw("select") {
                return self.parse_select();
            }
            if self.is_kw("dim") || self.is_kw("redim") {
                return self.parse_dim();
            }
            if self.is_kw("exit") {
                return self.parse_exit();
            }
            // Assignment `name = expr` — only at statement position. Inside a condition or any
            // other expression a Basic `=` is the equality operator (see `parse_atom`). A
            // declaration (`[scope] <Type>Var name …`) never matches: its second token is the
            // variable name, not `=`.
            if matches!(self.cur().kind, TokenKind::Ident) && self.peek_op(1, op::EQ) {
                let name = self.advance().text;
                self.advance(); // `=`
                let value = self.parse_expr();
                return Node::Assign {
                    name,
                    value: Box::new(value),
                };
            }
        }
        self.parse_expr()
    }

    fn parse_expr(&mut self) -> Node {
        self.parse_imp()
    }

    // --- precedence ladder (low → high) ---

    /// Generic left-associative binary level over word operators (`And`/`Or`/… lex as idents).
    /// Node-kind == op code.
    fn left_assoc(&mut self, ops: &[(&str, u8)], next: fn(&mut Self) -> Node) -> Node {
        let mut left = next(self);
        while let Some(code) = self.word_op(ops) {
            self.advance();
            let right = next(self);
            left = Node::Binary {
                op: code,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    /// [`left_assoc`](Self::left_assoc) for **symbol** operators (matched by [`is_op`](Self::is_op),
    /// tried in slice order). Node-kind == op code.
    fn left_assoc_ops(&mut self, ops: &[u8], next: fn(&mut Self) -> Node) -> Node {
        let mut left = next(self);
        while let Some(code) = ops.iter().copied().find(|&c| self.is_op(c)) {
            self.advance();
            let right = next(self);
            left = Node::Binary {
                op: code,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    fn parse_imp(&mut self) -> Node {
        self.left_assoc(&[("imp", op::IMP)], Self::parse_eqv)
    }
    fn parse_eqv(&mut self) -> Node {
        self.left_assoc(&[("eqv", op::EQV)], Self::parse_xor)
    }
    fn parse_xor(&mut self) -> Node {
        self.left_assoc(&[("xor", op::XOR)], Self::parse_or)
    }
    fn parse_or(&mut self) -> Node {
        self.left_assoc(&[("or", op::OR)], Self::parse_and)
    }
    fn parse_and(&mut self) -> Node {
        self.left_assoc(&[("and", op::AND)], Self::parse_equality)
    }

    // level 6: `=` `<>` `In` `Like` `StartsWith` — non-associative (at most one)
    fn parse_equality(&mut self) -> Node {
        let left = self.parse_relational();
        let code = if self.eat_op(op::EQ) {
            Some(op::EQ)
        } else if self.eat_op(op::NE) {
            Some(op::NE)
        } else if let Some(c) = self.word_op(&[
            ("in", op::IN),
            ("like", op::LIKE),
            ("startswith", op::STARTS_WITH),
        ]) {
            self.advance();
            Some(c)
        } else {
            None
        };
        match code {
            Some(op) => {
                let right = self.parse_relational();
                Node::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                }
            }
            None => left,
        }
    }

    // level 7: `<` `>` `>=` `<=` — non-associative
    fn parse_relational(&mut self) -> Node {
        let left = self.parse_concat();
        let code = [op::LT, op::GT, op::GE, op::LE]
            .into_iter()
            .find(|&c| self.is_op(c));
        match code {
            Some(op) => {
                self.advance();
                let right = self.parse_concat();
                Node::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                }
            }
            None => left,
        }
    }

    // level 8: `&` concat — left
    fn parse_concat(&mut self) -> Node {
        self.left_assoc_ops(&[op::AMP], Self::parse_range)
    }

    // level 9: range `To` (+ half-open) — non-associative
    fn parse_range(&mut self) -> Node {
        let left = self.parse_additive();
        if let Some(code) = self.word_op(&[
            ("to", op::RANGE_TO),
            ("_to", op::RANGE_LO_EXCL),
            ("to_", op::RANGE_HI_EXCL),
            ("_to_", op::RANGE_BOTH_EXCL),
        ]) {
            self.advance();
            let right = self.parse_additive();
            return Node::Binary {
                op: code,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    // level 10: `+` `-` — left
    fn parse_additive(&mut self) -> Node {
        self.left_assoc_ops(&[op::PLUS, op::MINUS], Self::parse_mod)
    }

    // level 11: `Mod` — left
    fn parse_mod(&mut self) -> Node {
        self.left_assoc(&[("mod", op::MOD)], Self::parse_intdiv)
    }

    // level 12: `\` integer division — left
    fn parse_intdiv(&mut self) -> Node {
        self.left_assoc_ops(&[op::BACKSLASH], Self::parse_mul)
    }

    // level 13: `*` `/` `%` — left
    fn parse_mul(&mut self) -> Node {
        self.left_assoc_ops(&[op::STAR, op::SLASH, op::PERCENT], Self::parse_unary)
    }

    // level 14: prefix `-` `+` `$` `Not` — right associative
    fn parse_unary(&mut self) -> Node {
        let code = if self.is_op(op::MINUS) {
            Some(op::UNARY_MINUS)
        } else if self.is_op(op::PLUS) {
            Some(op::UNARY_PLUS)
        } else if self.is_op(op::DOLLAR) {
            Some(op::DOLLAR)
        } else if self.word_op(&[("not", op::NOT)]).is_some() {
            Some(op::NOT)
        } else {
            None
        };
        match code {
            Some(op) => {
                self.advance();
                let expr = self.parse_unary();
                Node::Unary {
                    op,
                    expr: Box::new(expr),
                }
            }
            None => self.parse_power(),
        }
    }

    // level 15: `^` power — left
    fn parse_power(&mut self) -> Node {
        let mut left = self.parse_subscript();
        while self.is_op(op::CARET) {
            self.advance();
            let right = self.parse_subscript();
            left = Node::Binary {
                op: op::CARET,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    // level 16: postfix subscript `expr[index]` — left
    fn parse_subscript(&mut self) -> Node {
        let mut base = self.parse_primary();
        while self.is_op(op::LBRACKET) {
            self.advance();
            let index = self.parse_expr();
            if !self.eat_op(op::RBRACKET) {
                self.diag("expected `]`");
            }
            base = Node::Index {
                base: Box::new(base),
                index: Box::new(index),
            };
        }
        base
    }

    // level 17: primary atoms
    fn parse_primary(&mut self) -> Node {
        match self.cur().kind.clone() {
            TokenKind::Number => Node::Number(self.advance().text),
            TokenKind::Str => Node::Str(self.advance().text),
            TokenKind::DateLit => {
                let t = self.advance();
                // Validate the internals now so a malformed literal surfaces as a diagnostic; the
                // node keeps the raw text and the value is parsed at eval time.
                if let Err(e) = crate::eval::parse_date_literal(&t.text) {
                    self.diags.push(Diagnostic {
                        message: format!("invalid date/time literal: {e}"),
                        start: t.start,
                        end: t.end,
                        severity: Severity::Error,
                    });
                }
                Node::DateLit(t.text)
            }
            TokenKind::Reference(kind) => {
                let name = self.advance().text;
                Node::Reference { kind, name }
            }
            TokenKind::Op(op::LPAREN) => {
                self.advance();
                let inner = self.parse_expr();
                if !self.eat_op(op::RPAREN) {
                    self.diag("expected `)`");
                }
                inner
            }
            TokenKind::Op(op::LBRACKET) => {
                self.advance();
                let mut items = Vec::new();
                if !self.is_op(op::RBRACKET) {
                    items.push(self.parse_expr());
                    while self.eat_op(op::COMMA) {
                        items.push(self.parse_expr());
                    }
                }
                if !self.eat_op(op::RBRACKET) {
                    self.diag("expected `]`");
                }
                Node::Array(items)
            }
            TokenKind::Ident => self.parse_ident_primary(),
            TokenKind::Eof => {
                self.diag("unexpected end of input");
                Node::Error
            }
            _ => {
                self.diag("unexpected token");
                self.advance();
                Node::Error
            }
        }
    }

    fn parse_ident_primary(&mut self) -> Node {
        // boolean literals
        if self.is_kw("true") || self.is_kw("yes") {
            self.advance();
            return Node::Bool(true);
        }
        if self.is_kw("false") || self.is_kw("no") {
            self.advance();
            return Node::Bool(false);
        }
        // Crystal If-expression
        if self.is_kw("if") {
            return self.parse_if();
        }
        // Variable declarations: `[Local|Global|Shared] <Type>Var [Array] name[, …] [:= init]`.
        // A scope keyword or a bare `<Type>Var` keyword starts one; a scope keyword NOT followed
        // by a `<Type>Var` falls back to the deferred path (keeps the tree total on bad input).
        if self.is_kw("local") || self.is_kw("global") || self.is_kw("shared") {
            if self.toks.get(self.pos + 1).is_some_and(
                |t| matches!(&t.kind, TokenKind::Ident if type_var_kind(&t.text).is_some()),
            ) {
                return self.parse_declare();
            }
            return self.parse_unparsed_to_stmt_end();
        }
        if type_var_kind(&self.cur().text).is_some() {
            return self.parse_declare();
        }
        // Crystal `Select … Case` is an expression.
        if self.is_kw("select") {
            return self.parse_select();
        }
        let name = self.advance().text;
        // call: name(args)
        if self.is_op(op::LPAREN) {
            self.advance();
            let mut args = Vec::new();
            if !self.is_op(op::RPAREN) {
                args.push(self.parse_expr());
                while self.eat_op(op::COMMA) {
                    args.push(self.parse_expr());
                }
            }
            if !self.eat_op(op::RPAREN) {
                self.diag("expected `)`");
            }
            return Node::Call { name, args };
        }
        // Assignment `name := value` (Crystal). In Basic, `=` at statement position is an
        // assignment handled in `parse_statement`; the atom is reached only in expression
        // position, where a Basic `=` is the equality operator (left for `parse_equality`),
        // so the atom never consumes `=` as an assignment.
        if self.syntax == Syntax::Crystal && self.eat_op(op::ASSIGN) {
            let value = self.parse_expr();
            return Node::Assign {
                name,
                value: Box::new(value),
            };
        }
        Node::Ident(name)
    }

    /// `[Local|Global|Shared] <Type>Var [Array] name[, name…] [:= init]`. The caller has verified
    /// the shape up to the `<Type>Var` keyword.
    fn parse_declare(&mut self) -> Node {
        let scope = if self.eat_kw("local") {
            VarScope::Local
        } else if self.eat_kw("shared") {
            VarScope::Shared
        } else {
            self.eat_kw("global");
            VarScope::Global
        };
        // The `<Type>Var` keyword (verified present by the caller).
        let kind = type_var_kind(&self.advance().text).unwrap_or(VarKind::Number);
        let array = self.eat_kw("array");
        let mut names = Vec::new();
        loop {
            if let TokenKind::Ident = self.cur().kind {
                names.push(self.advance().text);
            } else {
                self.diag("expected variable name");
                break;
            }
            if !self.eat_op(op::COMMA) {
                break;
            }
        }
        let assign = if self.syntax == Syntax::Crystal {
            self.eat_op(op::ASSIGN)
        } else {
            self.eat_op(op::EQ)
        };
        let init = assign.then(|| Box::new(self.parse_expr()));
        Node::Declare {
            scope,
            kind,
            array,
            names,
            init,
        }
    }

    fn parse_if(&mut self) -> Node {
        self.advance(); // `If`
        let cond = self.parse_expr();
        if !self.eat_kw("then") {
            self.diag("expected `Then`");
        }
        let then = self.parse_expr();
        let mut elifs = Vec::new();
        loop {
            // `Else If` (two words) or `ElseIf` (one word)
            let is_elif = self.is_kw("elseif") || {
                if self.is_kw("else") && self.peek_kw(1, "if") {
                    self.advance(); // else
                    true
                } else {
                    false
                }
            };
            if !is_elif {
                break;
            }
            self.advance(); // `If` / `ElseIf`
            let econd = self.parse_expr();
            if !self.eat_kw("then") {
                self.diag("expected `Then`");
            }
            let ethen = self.parse_expr();
            elifs.push((econd, ethen));
        }
        let els = if self.eat_kw("else") {
            Some(Box::new(self.parse_expr()))
        } else {
            None
        };
        Node::If {
            cond: Box::new(cond),
            then: Box::new(then),
            elifs,
            els,
        }
    }

    /// `While cond Wend` (Basic) or `While cond Do stmt` (Crystal). A pre-test loop.
    fn parse_while(&mut self) -> Node {
        self.advance(); // While
        let cond = self.parse_expr();
        let body = if self.syntax == Syntax::Basic {
            let b = self.parse_stmt_seq_until(&["wend"]);
            if !self.eat_kw("wend") {
                self.diag("expected `Wend`");
            }
            b
        } else {
            if !self.eat_kw("do") {
                self.diag("expected `Do`");
            }
            self.parse_statement()
        };
        Node::While {
            cond: Box::new(cond),
            body: Box::new(body),
            test_after: false,
        }
    }

    /// `Do [While|Until cond] … Loop [While|Until cond]` (Basic). The condition may lead (pre-test)
    /// or trail (post-test); `Until` desugars to a `Not`-wrapped condition.
    fn parse_do(&mut self) -> Node {
        self.advance(); // Do
        let pre = self.parse_loop_cond();
        let body = self.parse_stmt_seq_until(&["loop"]);
        if !self.eat_kw("loop") {
            self.diag("expected `Loop`");
        }
        let post = self.parse_loop_cond();
        let (cond, test_after) = match (pre, post) {
            (Some(c), None) => (c, false),
            (None, Some(c)) => (c, true),
            // A bare `Do … Loop` with no guard is an infinite loop, exited via `Exit Do`
            // (the loop cap in the evaluator guards against a missing `Exit`).
            (None, None) => (Node::Bool(true), false),
            (Some(c), Some(_)) => {
                self.diag("`Do` has both a leading and a trailing condition");
                (c, false)
            }
        };
        Node::While {
            cond: Box::new(cond),
            body: Box::new(body),
            test_after,
        }
    }

    /// A `While cond` / `Until cond` loop condition; `Until` desugars to `Not (cond)`.
    fn parse_loop_cond(&mut self) -> Option<Node> {
        if self.eat_kw("while") {
            Some(self.parse_expr())
        } else if self.eat_kw("until") {
            let c = self.parse_expr();
            Some(Node::Unary {
                op: op::NOT,
                expr: Box::new(c),
            })
        } else {
            None
        }
    }

    /// `For i := a To b [Step s] Do stmt` (Crystal) or `For i = a To b [Step s] … Next [i]` (Basic).
    /// The bounds parse at additive precedence so the `To` isn't consumed as a range operator.
    fn parse_for(&mut self) -> Node {
        self.advance(); // For
        let var = if let TokenKind::Ident = self.cur().kind {
            self.advance().text
        } else {
            self.diag("expected loop variable");
            String::new()
        };
        let assigned = if self.syntax == Syntax::Crystal {
            self.eat_op(op::ASSIGN)
        } else {
            self.eat_op(op::EQ)
        };
        if !assigned {
            self.diag("expected loop-variable assignment");
        }
        let from = self.parse_additive();
        if !self.eat_kw("to") {
            self.diag("expected `To`");
        }
        let to = self.parse_additive();
        let step = self.eat_kw("step").then(|| Box::new(self.parse_additive()));
        let body = if self.syntax == Syntax::Basic {
            let b = self.parse_stmt_seq_until(&["next"]);
            if !self.eat_kw("next") {
                self.diag("expected `Next`");
            }
            // An optional loop variable may follow `Next` (on the same line).
            if let TokenKind::Ident = self.cur().kind {
                self.advance();
            }
            b
        } else {
            if !self.eat_kw("do") {
                self.diag("expected `Do`");
            }
            self.parse_statement()
        };
        Node::For {
            var,
            from: Box::new(from),
            to: Box::new(to),
            step,
            body: Box::new(body),
        }
    }

    /// Basic `If cond Then …`. Block form when a newline follows `Then` (`… [ElseIf] [Else] End If`),
    /// else a single-line `If cond Then stmt [Else stmt]`. Both lower to [`Node::If`].
    fn parse_basic_if(&mut self) -> Node {
        self.advance(); // If
        let cond = self.parse_expr();
        if !self.eat_kw("then") {
            self.diag("expected `Then`");
        }
        if matches!(self.cur().kind, TokenKind::Newline) {
            return self.parse_basic_if_block(cond);
        }
        // Single-line form.
        let then = self.parse_statement();
        let els = self
            .eat_kw("else")
            .then(|| Box::new(self.parse_statement()));
        Node::If {
            cond: Box::new(cond),
            then: Box::new(then),
            elifs: Vec::new(),
            els,
        }
    }

    fn parse_basic_if_block(&mut self, cond: Node) -> Node {
        let then = self.parse_stmt_seq_until(&["elseif", "else", "end"]);
        let mut elifs = Vec::new();
        loop {
            let is_elif = self.is_kw("elseif") || (self.is_kw("else") && self.peek_kw(1, "if"));
            if !is_elif {
                break;
            }
            if self.is_kw("elseif") {
                self.advance();
            } else {
                self.advance(); // Else
                self.advance(); // If
            }
            let econd = self.parse_expr();
            if !self.eat_kw("then") {
                self.diag("expected `Then`");
            }
            let ebody = self.parse_stmt_seq_until(&["elseif", "else", "end"]);
            elifs.push((econd, ebody));
        }
        let els = if self.eat_kw("else") {
            Some(Box::new(self.parse_stmt_seq_until(&["end"])))
        } else {
            None
        };
        if !self.eat_kw("end") {
            self.diag("expected `End If`");
        }
        self.eat_kw("if");
        Node::If {
            cond: Box::new(cond),
            then: Box::new(then),
            elifs,
            els,
        }
    }

    /// `Select expr … Case … [Default|Case Else]` — a Crystal expression or a Basic `Select Case …
    /// End Select` statement. Lowered to an `If`/`ElseIf` chain: each case's test list becomes a
    /// boolean condition on the (re-evaluated) subject.
    fn parse_select(&mut self) -> Node {
        self.advance(); // Select
        if self.syntax == Syntax::Basic {
            self.eat_kw("case"); // `Select Case`
        }
        let subject = self.parse_expr();
        let mut clauses: Vec<(Node, Node)> = Vec::new();
        let mut default: Option<Node> = None;
        loop {
            self.skip_separators();
            if !self.is_kw("case") {
                break;
            }
            self.advance(); // Case
            if self.is_kw("else") {
                self.advance();
                self.eat_op(op::COLON);
                default = Some(self.parse_case_body());
                break;
            }
            let mut cond = self.parse_case_test(&subject);
            while self.eat_op(op::COMMA) {
                let t = self.parse_case_test(&subject);
                cond = Node::Binary {
                    op: op::OR,
                    left: Box::new(cond),
                    right: Box::new(t),
                };
            }
            self.eat_op(op::COLON);
            clauses.push((cond, self.parse_case_body()));
        }
        if self.syntax == Syntax::Crystal && self.is_kw("default") {
            self.advance();
            self.eat_op(op::COLON);
            default = Some(self.parse_case_body());
        }
        if self.syntax == Syntax::Basic {
            if !self.eat_kw("end") {
                self.diag("expected `End Select`");
            }
            self.eat_kw("select");
        }
        if clauses.is_empty() {
            return default.unwrap_or(Node::Empty);
        }
        let (cond, then) = clauses.remove(0);
        Node::If {
            cond: Box::new(cond),
            then: Box::new(then),
            elifs: clauses,
            els: default.map(Box::new),
        }
    }

    /// A single `Case` test, lowered to a boolean condition on `subject`: `Is <rel> v` →
    /// `subject <rel> v`, `lo To hi` → `subject In (lo To hi)`, bare `v` → `subject = v`.
    fn parse_case_test(&mut self, subject: &Node) -> Node {
        if self.is_kw("is") {
            self.advance();
            let code = if self.eat_op(op::LT) {
                op::LT
            } else if self.eat_op(op::LE) {
                op::LE
            } else if self.eat_op(op::GT) {
                op::GT
            } else if self.eat_op(op::GE) {
                op::GE
            } else if self.eat_op(op::NE) {
                op::NE
            } else if self.eat_op(op::EQ) {
                op::EQ
            } else {
                self.diag("expected a comparison after `Is`");
                op::EQ
            };
            let rhs = self.parse_additive();
            return Node::Binary {
                op: code,
                left: Box::new(subject.clone()),
                right: Box::new(rhs),
            };
        }
        let lo = self.parse_additive();
        if self.eat_kw("to") {
            let hi = self.parse_additive();
            let range = Node::Binary {
                op: op::RANGE_TO,
                left: Box::new(lo),
                right: Box::new(hi),
            };
            Node::Binary {
                op: op::IN,
                left: Box::new(subject.clone()),
                right: Box::new(range),
            }
        } else {
            Node::Binary {
                op: op::EQ,
                left: Box::new(subject.clone()),
                right: Box::new(lo),
            }
        }
    }

    /// A `Case` clause body: a single expression in Crystal, a statement sequence in Basic.
    fn parse_case_body(&mut self) -> Node {
        if self.syntax == Syntax::Basic {
            self.parse_stmt_seq_until(&["case", "end"])
        } else {
            self.parse_expr()
        }
    }

    /// Basic `Dim name[(dims)][, name…] [As Type]` — lowered to a `Local` [`Node::Declare`].
    fn parse_dim(&mut self) -> Node {
        self.advance(); // Dim / ReDim
        let mut names = Vec::new();
        let mut array = false;
        loop {
            if let TokenKind::Ident = self.cur().kind {
                names.push(self.advance().text);
            } else {
                self.diag("expected variable name");
                break;
            }
            if self.eat_op(op::LPAREN) {
                array = true;
                while !self.is_op(op::RPAREN)
                    && !self.at_eof()
                    && !matches!(self.cur().kind, TokenKind::Newline)
                {
                    self.advance();
                }
                self.eat_op(op::RPAREN);
            }
            if !self.eat_op(op::COMMA) {
                break;
            }
        }
        let kind = if self.eat_kw("as") {
            let k = dim_type_kind(&self.cur().text);
            if matches!(self.cur().kind, TokenKind::Ident) {
                self.advance();
            }
            k
        } else {
            VarKind::Number
        };
        Node::Declare {
            scope: VarScope::Local,
            kind,
            array,
            names,
            init: None,
        }
    }

    /// `Exit For`/`Exit While`/`Exit Do` — breaks the innermost enclosing loop. The loop keyword is
    /// captured for AST fidelity (`For` when absent/unknown); evaluation breaks regardless of kind.
    fn parse_exit(&mut self) -> Node {
        self.advance(); // Exit
        let kind = if matches!(self.cur().kind, TokenKind::Ident) {
            let k = match self.cur().text.to_ascii_lowercase().as_str() {
                "while" => ExitKind::While,
                "do" => ExitKind::Do,
                _ => ExitKind::For,
            };
            self.advance(); // For / While / Do
            k
        } else {
            ExitKind::For
        };
        Node::Exit(kind)
    }

    fn peek_kw(&self, ahead: usize, kw: &str) -> bool {
        self.toks
            .get(self.pos + ahead)
            .is_some_and(|t| matches!(&t.kind, TokenKind::Ident if t.text.eq_ignore_ascii_case(kw)))
    }

    /// Whether the token `ahead` of the cursor is the operator `code` (without consuming).
    fn peek_op(&self, ahead: usize, code: u8) -> bool {
        self.toks
            .get(self.pos + ahead)
            .is_some_and(|t| matches!(t.kind, TokenKind::Op(c) if c == code))
    }

    /// Best-effort recovery for a deferred construct: collect primaries until a statement end.
    fn parse_unparsed_to_stmt_end(&mut self) -> Node {
        let mut children = Vec::new();
        while !self.at_eof() && !self.is_op(op::SEMI) {
            if matches!(self.cur().kind, TokenKind::Newline) {
                break;
            }
            let before = self.pos;
            match self.cur().kind {
                TokenKind::Reference(_) | TokenKind::Number | TokenKind::Str => {
                    children.push(self.parse_primary());
                }
                _ => {
                    self.advance();
                }
            }
            if self.pos == before {
                self.advance();
            }
        }
        Node::Unparsed(children)
    }
}

/// Map a `<Type>Var` keyword (case-insensitive) to its [`VarKind`].
fn type_var_kind(text: &str) -> Option<VarKind> {
    TYPE_VAR_KWS
        .iter()
        .find(|(kw, _)| text.eq_ignore_ascii_case(kw))
        .map(|(_, k)| *k)
}

/// Map a Basic `Dim … As <Type>` type name to its [`VarKind`] (unknown/absent → Number).
fn dim_type_kind(text: &str) -> VarKind {
    match text.to_ascii_lowercase().as_str() {
        "string" => VarKind::String,
        "currency" => VarKind::Currency,
        "boolean" => VarKind::Boolean,
        "date" => VarKind::Date,
        "time" => VarKind::Time,
        "datetime" => VarKind::DateTime,
        _ => VarKind::Number,
    }
}
