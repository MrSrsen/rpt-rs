//! Hand-written, error-recovering recursive-descent parser over the 17-level operator-precedence
//! ladder and the primary grammar. Crystal syntax is primary; the expression grammar is shared with
//! Basic. The parser never panics: on an unexpected token it records a [`Diagnostic`], emits
//! [`Node::Error`], and makes progress, so it is safe to run on any formula and is a foundation for
//! eval/validation/LSP.
//!
//! Deferred constructs (`#...#` date internals, full `Select…Case`, declarations —
//! `Local/Global/Shared`, `Dim` — and Basic statement bodies) parse to [`Node::Unparsed`]
//! best-effort. Reference *counting* never depends on this parser (see [`super::refs`]).

use super::ast::Node;
use super::lexer::tokenize;
use super::token::{op, Syntax, Token, TokenKind};

/// A parse diagnostic with the source span it concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub start: usize,
    pub end: usize,
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
    (node, p.diags)
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
        });
    }

    // statement sequence: stmt { (";" | newline) stmt }
    fn parse_stmt_seq(&mut self) -> Node {
        let mut stmts = Vec::new();
        loop {
            // skip separators
            while self.eat_op(op::SEMI)
                || (self.syntax == Syntax::Basic && matches!(self.cur().kind, TokenKind::Newline))
            {
                if self.syntax == Syntax::Basic && matches!(self.cur().kind, TokenKind::Newline) {
                    self.advance();
                }
            }
            if self.at_eof() {
                break;
            }
            let before = self.pos;
            stmts.push(self.parse_expr());
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

    fn parse_imp(&mut self) -> Node {
        self.left_assoc(&[("imp", 0x43)], Self::parse_eqv)
    }
    fn parse_eqv(&mut self) -> Node {
        self.left_assoc(&[("eqv", 0x42)], Self::parse_xor)
    }
    fn parse_xor(&mut self) -> Node {
        self.left_assoc(&[("xor", 0x41)], Self::parse_or)
    }
    fn parse_or(&mut self) -> Node {
        self.left_assoc(&[("or", 0x40)], Self::parse_and)
    }
    fn parse_and(&mut self) -> Node {
        self.left_assoc(&[("and", 0x3f)], Self::parse_equality)
    }

    // level 6: `=` `<>` `In` `Like` `StartsWith` — non-associative (at most one)
    fn parse_equality(&mut self) -> Node {
        let left = self.parse_relational();
        let code = if self.eat_op(op::EQ) {
            Some(op::EQ)
        } else if self.eat_op(op::NE) {
            Some(op::NE)
        } else if let Some(c) = self.word_op(&[("in", 0x38), ("like", 0x5a), ("startswith", 0x5b)])
        {
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
        let mut left = self.parse_range();
        while self.is_op(op::AMP) {
            self.advance();
            let right = self.parse_range();
            left = Node::Binary {
                op: op::AMP,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    // level 9: range `To` (+ half-open) — non-associative
    fn parse_range(&mut self) -> Node {
        let left = self.parse_additive();
        if let Some(code) =
            self.word_op(&[("to", 0x2f), ("_to", 0x30), ("to_", 0x31), ("_to_", 0x32)])
        {
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
        let mut left = self.parse_mod();
        loop {
            let code = if self.is_op(op::PLUS) {
                op::PLUS
            } else if self.is_op(op::MINUS) {
                op::MINUS
            } else {
                break;
            };
            self.advance();
            let right = self.parse_mod();
            left = Node::Binary {
                op: code,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    // level 11: `Mod` — left
    fn parse_mod(&mut self) -> Node {
        let mut left = self.parse_intdiv();
        while self.word_op(&[("mod", 0x2a)]).is_some() {
            self.advance();
            let right = self.parse_intdiv();
            left = Node::Binary {
                op: 0x2a,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    // level 12: `\` integer division — left
    fn parse_intdiv(&mut self) -> Node {
        let mut left = self.parse_mul();
        while self.is_op(op::BACKSLASH) {
            self.advance();
            let right = self.parse_mul();
            left = Node::Binary {
                op: op::BACKSLASH,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    // level 13: `*` `/` `%` — left
    fn parse_mul(&mut self) -> Node {
        let mut left = self.parse_unary();
        loop {
            let code = [op::STAR, op::SLASH, op::PERCENT]
                .into_iter()
                .find(|&c| self.is_op(c));
            let Some(code) = code else { break };
            self.advance();
            let right = self.parse_unary();
            left = Node::Binary {
                op: code,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    // level 14: prefix `-` `+` `$` `Not` — right associative
    fn parse_unary(&mut self) -> Node {
        let code = if self.is_op(op::MINUS) {
            Some(0x7a) // prefix `-` node kind
        } else if self.is_op(op::PLUS) {
            Some(0x79)
        } else if self.is_op(op::DOLLAR) {
            Some(op::DOLLAR)
        } else if self.word_op(&[("not", 0x25)]).is_some() {
            Some(0x25)
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
            TokenKind::DateLit => Node::DateLit(self.advance().text),
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
        // Deferred statement/declaration constructs — keep the tree total.
        if self.is_kw("select")
            || self.is_kw("local")
            || self.is_kw("global")
            || self.is_kw("shared")
            || self.is_kw("dim")
        {
            return self.parse_unparsed_to_stmt_end();
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
        // assignment: name := value (Crystal) / name = value (Basic)
        let assign = if self.syntax == Syntax::Crystal {
            self.eat_op(op::ASSIGN)
        } else {
            self.eat_op(op::EQ)
        };
        if assign {
            let value = self.parse_expr();
            return Node::Assign {
                name,
                value: Box::new(value),
            };
        }
        Node::Ident(name)
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

    fn peek_kw(&self, ahead: usize, kw: &str) -> bool {
        self.toks
            .get(self.pos + ahead)
            .is_some_and(|t| matches!(&t.kind, TokenKind::Ident if t.text.eq_ignore_ascii_case(kw)))
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
