//! Formula **type system** — result-kind and string-length deduction over the [`super::ast::Node`]
//! tree, matching the Crystal engine's behaviour.
//!
//! The funcID-keyed lookup tables in `types_table.rs` are `include!`d at the bottom of this module so
//! they share this module's `ReturnRule`/`StrLenRule`. Result-kind raw codes match the engine's so a
//! node can carry the engine's kind; scalars map to [`rpt::model::FieldValueType`] via
//! [`ResultKind::to_field_value_type`].

use super::ast::Node;
use super::token::{op, RefKind};

/// Crystal's max string field length: 32767 chars × 2 bytes = 65534 (`0xfffe`), which is also the
/// "unbounded" sentinel (memo fields, open params).
pub const MAX_STRING_BYTES: i32 = 0xfffe;

// Default (no-format) character counts used by the `(chars+1)*4` coercion. They govern the `&`/`+`
// concat coercion path (where the operand carries no ToText format args); an actual `ToText(...)`
// call with format args is handled format-aware by `number_chars`.
//
//   number   = thouGroups(5)*thouSep(5) + decDigits(10) + intDigits(18) + decSep(15) + 7  = 75
//   currency = same but +12 instead of +7                                                 = 80
const COERCE_BOOLEAN_CHARS: i32 = 15; // localized True/False/Yes/No, ×4 no +1
const COERCE_DATE_CHARS: i32 = 60; // bare-date default, no format-string arg
const COERCE_TIME_CHARS: i32 = 30; // bare-time default
const COERCE_NUMBER_CHARS: i32 = 75; // bare-number default
const COERCE_CURRENCY_CHARS: i32 = 80; // bare-currency default
                                       // DateTime default = date(60) + time(30) = 90.

/// A formula node's **result kind**, matching the engine's `resultKind` codes. `Unknown` (0) covers
/// statically-undeterminable nodes (an undeclared variable, a runtime builtin, an unresolved name);
/// the engine likewise leaves such a node's kind unresolved until runtime/binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ResultKind {
    Unknown = 0,
    // Scalars 1..7
    Number = 1,
    Currency = 2,
    Boolean = 3,
    Date = 4,
    Time = 5,
    DateTime = 6,
    String = 7,
    /// Bare callee leaf (a function reference) — engine code 8.
    FunctionRef = 8,
    // Range block 9..0xe (compacted — NO Boolean range; Boolean is not orderable).
    NumberRange = 0x9,
    CurrencyRange = 0xa,
    DateRange = 0xb,
    TimeRange = 0xc,
    DateTimeRange = 0xd,
    StringRange = 0xe,
    // Array block 0xf..0x15 = scalar + 0xe.
    NumberArray = 0xf,
    CurrencyArray = 0x10,
    BooleanArray = 0x11,
    DateArray = 0x12,
    TimeArray = 0x13,
    DateTimeArray = 0x14,
    StringArray = 0x15,
    // Range-array block 0x16..0x1b (array-of-range).
    NumberRangeArray = 0x16,
    CurrencyRangeArray = 0x17,
    DateRangeArray = 0x18,
    TimeRangeArray = 0x19,
    DateTimeRangeArray = 0x1a,
    StringRangeArray = 0x1b,
}

impl ResultKind {
    /// The raw result-kind code.
    pub fn raw(self) -> u8 {
        self as u8
    }

    /// Reconstruct from a raw result-kind code; `None` for codes outside 0..=0x1b.
    pub fn from_raw(code: u8) -> Option<Self> {
        use ResultKind::*;
        Some(match code {
            0 => Unknown,
            1 => Number,
            2 => Currency,
            3 => Boolean,
            4 => Date,
            5 => Time,
            6 => DateTime,
            7 => String,
            8 => FunctionRef,
            0x9 => NumberRange,
            0xa => CurrencyRange,
            0xb => DateRange,
            0xc => TimeRange,
            0xd => DateTimeRange,
            0xe => StringRange,
            0xf => NumberArray,
            0x10 => CurrencyArray,
            0x11 => BooleanArray,
            0x12 => DateArray,
            0x13 => TimeArray,
            0x14 => DateTimeArray,
            0x15 => StringArray,
            0x16 => NumberRangeArray,
            0x17 => CurrencyRangeArray,
            0x18 => DateRangeArray,
            0x19 => TimeRangeArray,
            0x1a => DateTimeRangeArray,
            0x1b => StringRangeArray,
            _ => return None,
        })
    }

    /// A 1..=7 scalar result kind.
    pub fn is_scalar(self) -> bool {
        (1..=7).contains(&self.raw())
    }
    /// A 9..=0xe range result kind.
    pub fn is_range(self) -> bool {
        (0x9..=0xe).contains(&self.raw())
    }
    /// A 0xf..=0x15 array result kind.
    pub fn is_array(self) -> bool {
        (0xf..=0x15).contains(&self.raw())
    }
    /// A 0x16..=0x1b range-array result kind.
    pub fn is_range_array(self) -> bool {
        (0x16..=0x1b).contains(&self.raw())
    }

    /// Collapse a range / array / range-array down to its element **scalar** (the engine's
    /// "de-array" / range-element operation, e.g. for subscript and `GetLowerBound`). A scalar maps
    /// to itself; `Unknown`/`FunctionRef` are returned unchanged.
    pub fn to_scalar(self) -> ResultKind {
        let r = self.raw();
        let s = if self.is_array() {
            r - 0xe // array = scalar + 0xe
        } else if self.is_range() {
            // compacted range -> scalar (Boolean skipped)
            match self {
                ResultKind::NumberRange => 1,
                ResultKind::CurrencyRange => 2,
                ResultKind::DateRange => 4,
                ResultKind::TimeRange => 5,
                ResultKind::DateTimeRange => 6,
                ResultKind::StringRange => 7,
                _ => r,
            }
        } else if self.is_range_array() {
            // range-array -> range -> scalar
            return ResultKind::from_raw(r - (0x16 - 0x9))
                .map(|x| x.to_scalar())
                .unwrap_or(ResultKind::Unknown);
        } else {
            return self;
        };
        ResultKind::from_raw(s).unwrap_or(ResultKind::Unknown)
    }

    /// Lift a scalar to its **array** kind (scalar + 0xe). Non-scalars are returned unchanged.
    pub fn to_array(self) -> ResultKind {
        if self.is_scalar() {
            ResultKind::from_raw(self.raw() + 0xe).unwrap_or(self)
        } else {
            self
        }
    }

    /// Lift a scalar to its **range** kind (the compacted block; Boolean has no range so it is
    /// returned unchanged). Non-scalars are returned unchanged.
    pub fn to_range(self) -> ResultKind {
        match self {
            ResultKind::Number => ResultKind::NumberRange,
            ResultKind::Currency => ResultKind::CurrencyRange,
            ResultKind::Date => ResultKind::DateRange,
            ResultKind::Time => ResultKind::TimeRange,
            ResultKind::DateTime => ResultKind::DateTimeRange,
            ResultKind::String => ResultKind::StringRange,
            other => other,
        }
    }

    /// Map a scalar result kind to the exported [`FieldValueType`](rpt::model::FieldValueType).
    /// Non-scalar kinds (a formula cannot finally yield a range/array) and `Unknown`/`FunctionRef`
    /// map to `Unknown`.
    pub fn to_field_value_type(self) -> rpt::model::FieldValueType {
        use rpt::model::FieldValueType as F;
        match self {
            ResultKind::Number => F::Number,
            ResultKind::Currency => F::Currency,
            ResultKind::Boolean => F::Boolean,
            ResultKind::Date => F::Date,
            ResultKind::Time => F::Time,
            ResultKind::DateTime => F::DateTime,
            ResultKind::String => F::String,
            _ => F::Unknown,
        }
    }
}

/// The per-function return-type rule. `Fixed` carries a raw result-kind code.
#[derive(Debug, Clone, Copy)]
pub(super) enum ReturnRule {
    /// A constant result kind regardless of args.
    Fixed(u8),
    /// Copy the (first) argument's type — `Abs`/`Int`/`Round` group, `Previous`/`Next` (group 0x2).
    CopyArg,
    /// Numeric type of the value arg — `Sum`/`Average`/`StdDev`/… aggregations:
    /// Currency if the arg is Currency, else Number.
    AggNumeric,
    /// `Maximum`/`Minimum`: the arg's scalar type (de-array/de-range a collection arg).
    MaxMinArg,
    /// `GetLowerBound`/`GetUpperBound`-style: map an array/range arg to its element scalar.
    DeArray,
    /// `IIf(cond, a, b)` — unify branches a (arg1) and b (arg2).
    Iif,
    /// `Choose(idx, v1, v2, …)` — unify the value args (arg1..).
    Choose,
    /// `Switch(c1, v1, c2, v2, …)` — unify the value args (odd indices 1,3,5,…).
    Switch,
    /// Runtime/bound-object dependent (`COMPLEX`) or an overload we don't resolve statically.
    Complex,
}

/// The per-function string-length rule, consulted only when a call's deduced result kind is
/// String/StringArray.
#[derive(Debug, Clone, Copy)]
pub(super) enum StrLenRule {
    /// `ToText`/`CStr`/`GetValueDescriptions` — coerce the conversion-target arg to text length.
    Coerce,
    /// Length-preserving — copy arg1's byte width (Trim*/Upper/Lower/ProperCase/StrReverse/Filter/…).
    CopyArg1,
    /// Constant byte width.
    Const(i32),
    /// `Left`/`Right` — arg2 (a const number of wide chars) × 4.
    ArgTimes4,
    /// `Space` — arg1 (const number) × 2 + 2.
    SpaceRule,
    /// `ReplicateString` — (arg1.bytes − 2) × arg2_const + 2.
    ReplicateRule,
    /// `IIf`/`Choose`/`Switch` — max over the branch lengths.
    BranchMax,
    /// Unbounded → `MAX_STRING_BYTES`.
    Unbounded,
}

/// Look up a builtin/function/special-field name (case-insensitive) → funcID.
pub fn func_id(name: &str) -> Option<u16> {
    let lname = name.to_ascii_lowercase();
    NAME_FUNCID
        .binary_search_by(|(n, _)| n.cmp(&lname.as_str()))
        .ok()
        .map(|i| NAME_FUNCID[i].1)
}

/// Deduce the [`ResultKind`] of a parsed formula node.
///
/// `ref_lookup` resolves a `{field}`/`{?param}`/`{@formula}`/`{#rt}`/`{%sql}` reference to its bound
/// result kind (from the field/param/RT/formula's declared data type). Returning `None` yields
/// [`ResultKind::Unknown`] for that reference.
pub fn deduce_type(
    node: &Node,
    ref_lookup: &dyn Fn(RefKind, &str) -> Option<ResultKind>,
) -> ResultKind {
    use ResultKind as K;
    match node {
        Node::Number(_) => K::Number,
        Node::Str(_) => K::String,
        Node::Bool(_) => K::Boolean,
        Node::DateLit(s) => classify_date_lit(s),
        Node::Reference { kind, name } => ref_lookup(*kind, name).unwrap_or(K::Unknown),
        // A bare identifier is a 0-ary builtin/special-field if known, else an (undeclared) variable.
        Node::Ident(name) => match func_id(name) {
            Some(id) => apply_return_rule(return_rule(id), &[], ref_lookup),
            None => K::Unknown,
        },
        Node::Call { name, args } => match func_id(name) {
            Some(id) => apply_return_rule(return_rule(id), args, ref_lookup),
            None => K::Unknown, // custom function — declared return type not modelled here
        },
        Node::Index { base, .. } => deduce_type(base, ref_lookup).to_scalar(),
        Node::Array(items) => {
            // array kind = element scalar + 0xe; empty/unknown -> Unknown.
            let elem = items
                .iter()
                .map(|n| deduce_type(n, ref_lookup))
                .find(|k| *k != K::Unknown)
                .unwrap_or(K::Unknown);
            elem.to_array()
        }
        Node::Unary { op, expr } => match *op {
            op::DOLLAR => K::Currency,                    // `$` currency prefix
            0x79 | 0x7a => deduce_type(expr, ref_lookup), // unary `+`/`-` copy operand
            0x25 => K::Boolean,                           // `Not`
            _ => deduce_type(expr, ref_lookup),
        },
        Node::Binary { op, left, right } => deduce_binary(*op, left, right, ref_lookup),
        Node::If {
            then, elifs, els, ..
        } => {
            let mut acc = deduce_type(then, ref_lookup);
            for (_, v) in elifs {
                acc = unify(acc, deduce_type(v, ref_lookup));
            }
            if let Some(e) = els {
                acc = unify(acc, deduce_type(e, ref_lookup));
            }
            acc
        }
        Node::Assign { value, .. } => deduce_type(value, ref_lookup),
        Node::Seq(stmts) => stmts
            .last()
            .map(|n| deduce_type(n, ref_lookup))
            .unwrap_or(K::Unknown),
        Node::Unparsed(_) | Node::Error | Node::Empty => K::Unknown,
    }
}

/// Classify a `#...#` date/time literal by its content: a `:` with no date separator → Time; both a
/// date separator and a `:` → DateTime; otherwise Date.
fn classify_date_lit(s: &str) -> ResultKind {
    let has_time = s.contains(':');
    let has_date = s.contains('/') || s.contains('-') || s.contains(',');
    match (has_date, has_time) {
        (false, true) => ResultKind::Time,
        (true, true) => ResultKind::DateTime,
        _ => ResultKind::Date,
    }
}

fn deduce_binary(
    code: u8,
    left: &Node,
    right: &Node,
    ref_lookup: &dyn Fn(RefKind, &str) -> Option<ResultKind>,
) -> ResultKind {
    use ResultKind as K;
    let lt = || deduce_type(left, ref_lookup);
    let rt = || deduce_type(right, ref_lookup);
    match code {
        op::AMP => K::String, // `&` always String (coerces operands)
        op::PLUS => add_type(lt(), rt()),
        op::MINUS => sub_type(lt(), rt()),
        // `*` `/` `\` `Mod` `%` -> numeric (Currency if either operand Currency, else Number)
        op::STAR | op::SLASH | op::BACKSLASH | 0x2a /*Mod*/ => numeric_promote(lt(), rt()),
        op::PERCENT => K::Number, // `%`
        op::CARET => K::Number,   // `^` always Number
        // comparisons: `=` `<>` `<` `>` `>=` `<=`
        op::EQ | op::NE | op::LT | op::GT | op::GE | op::LE => K::Boolean,
        // `In` (0x38) `Like` (0x5a) `StartsWith` (0x5b)
        0x38 | 0x5a | 0x5b => K::Boolean,
        // `And` `Or` `Xor` `Eqv` `Imp`
        0x3f..=0x43 => K::Boolean,
        _ => K::Unknown,
    }
}

/// `+` operator typing: numeric+numeric → Number/Currency promotion; date/time/datetime + numeric
/// keeps the temporal kind; String+String → String.
fn add_type(l: ResultKind, r: ResultKind) -> ResultKind {
    use ResultKind as K;
    let l = l.to_scalar();
    let r = r.to_scalar();
    let numeric = |k: ResultKind| matches!(k, K::Number | K::Currency);
    let temporal = |k: ResultKind| matches!(k, K::Date | K::Time | K::DateTime);
    if l == K::String && r == K::String {
        K::String
    } else if numeric(l) && numeric(r) {
        numeric_promote(l, r)
    } else if temporal(l) && numeric(r) {
        l
    } else if temporal(r) && numeric(l) {
        r
    } else if l == K::Unknown || r == K::Unknown {
        K::Unknown
    } else {
        l
    }
}

/// `-` operator typing: numeric promotion; temporal − numeric keeps the temporal kind; temporal −
/// same-temporal → Number (a duration). No String.
fn sub_type(l: ResultKind, r: ResultKind) -> ResultKind {
    use ResultKind as K;
    let l = l.to_scalar();
    let r = r.to_scalar();
    let numeric = |k: ResultKind| matches!(k, K::Number | K::Currency);
    let temporal = |k: ResultKind| matches!(k, K::Date | K::Time | K::DateTime);
    if numeric(l) && numeric(r) {
        numeric_promote(l, r)
    } else if temporal(l) && temporal(r) {
        K::Number // date - date = number of days, etc.
    } else if temporal(l) && numeric(r) {
        l
    } else if l == K::Unknown || r == K::Unknown {
        K::Unknown
    } else {
        l
    }
}

/// Number↔Currency promotion: Currency if either operand is Currency, else Number.
fn numeric_promote(l: ResultKind, r: ResultKind) -> ResultKind {
    use ResultKind as K;
    if l == K::Currency || r == K::Currency {
        K::Currency
    } else {
        K::Number
    }
}

/// Branch unification for `If`/`IIf`/`Choose`/`Switch`. Equal kinds unify to themselves; `Unknown` is
/// absorbed; two numerics promote (Currency wins); otherwise (incompatible — the engine would error)
/// yield `Unknown`.
fn unify(a: ResultKind, b: ResultKind) -> ResultKind {
    use ResultKind as K;
    if a == b {
        a
    } else if a == K::Unknown {
        b
    } else if b == K::Unknown {
        a
    } else if matches!(a, K::Number | K::Currency) && matches!(b, K::Number | K::Currency) {
        numeric_promote(a, b)
    } else {
        K::Unknown
    }
}

fn apply_return_rule(
    rule: ReturnRule,
    args: &[Node],
    ref_lookup: &dyn Fn(RefKind, &str) -> Option<ResultKind>,
) -> ResultKind {
    use ResultKind as K;
    let arg = |i: usize| {
        args.get(i)
            .map(|n| deduce_type(n, ref_lookup))
            .unwrap_or(K::Unknown)
    };
    match rule {
        ReturnRule::Fixed(raw) => K::from_raw(raw).unwrap_or(K::Unknown),
        ReturnRule::CopyArg => arg(0),
        ReturnRule::AggNumeric => {
            let s = arg(0).to_scalar();
            if s == K::Currency {
                K::Currency
            } else {
                K::Number
            }
        }
        ReturnRule::MaxMinArg => arg(0).to_scalar(),
        ReturnRule::DeArray => arg(0).to_scalar(),
        ReturnRule::Iif => unify(arg(1), arg(2)),
        ReturnRule::Choose => args
            .iter()
            .skip(1)
            .map(|n| deduce_type(n, ref_lookup))
            .fold(K::Unknown, unify),
        ReturnRule::Switch => args
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 2 == 1)
            .map(|(_, n)| deduce_type(n, ref_lookup))
            .fold(K::Unknown, unify),
        ReturnRule::Complex => K::Unknown,
    }
}

// ---------------------------------------------------------------------------------------------
// String-length pass — maximum byte width of a String result.
// ---------------------------------------------------------------------------------------------

/// Compute a String/String-array node's maximum byte width, capped at [`MAX_STRING_BYTES`]; a
/// computed 0 becomes `MAX_STRING_BYTES` (engine convention).
///
/// `ref_type` resolves a reference's result kind; `ref_bytes` resolves a `{field}`'s declared byte
/// width (a memo field → `MAX_STRING_BYTES`). Number/Currency/Date/Time/DateTime char counts are
/// **locale/runtime dependent** (see the `COERCE_*` constants) — a caller with format info should
/// supply its own; this uses conservative defaults.
pub fn string_max_bytes(
    node: &Node,
    ref_type: &dyn Fn(RefKind, &str) -> Option<ResultKind>,
    ref_bytes: &dyn Fn(RefKind, &str) -> Option<i32>,
) -> i32 {
    cap(string_max_bytes_raw(node, ref_type, ref_bytes))
}

fn string_max_bytes_raw(
    node: &Node,
    ref_type: &dyn Fn(RefKind, &str) -> Option<ResultKind>,
    ref_bytes: &dyn Fn(RefKind, &str) -> Option<i32>,
) -> i32 {
    match node {
        // String literal: (chars + 1) * sizeof(NCHAR=2) — UTF-16 + null.
        Node::Str(s) => (s.chars().count() as i32 + 1) * 2,
        Node::Reference { kind, name } => ref_bytes(*kind, name).unwrap_or(MAX_STRING_BYTES),
        Node::Binary { op, left, right } => match *op {
            // `&` concat: strByteLen(L) + strByteLen(R) − 2 (drop one UTF-16 null). Operands coerced.
            op::AMP => {
                coerce_str_bytes(left, ref_type, ref_bytes)
                    + coerce_str_bytes(right, ref_type, ref_bytes)
                    - 2
            }
            // `+` (string): both operands already String; L.bytes + R.bytes − 2.
            op::PLUS => {
                string_max_bytes_raw(left, ref_type, ref_bytes)
                    + string_max_bytes_raw(right, ref_type, ref_bytes)
                    - 2
            }
            _ => MAX_STRING_BYTES,
        },
        // Array literal / branches: max element width.
        Node::Array(items) => items
            .iter()
            .map(|n| string_max_bytes_raw(n, ref_type, ref_bytes))
            .max()
            .unwrap_or(MAX_STRING_BYTES),
        Node::If {
            then, elifs, els, ..
        } => {
            let mut m = string_max_bytes_raw(then, ref_type, ref_bytes);
            for (_, v) in elifs {
                m = m.max(string_max_bytes_raw(v, ref_type, ref_bytes));
            }
            if let Some(e) = els {
                m = m.max(string_max_bytes_raw(e, ref_type, ref_bytes));
            }
            m
        }
        Node::Index { base, .. } => {
            // String-array element → one wide char (6); else copy base.
            if deduce_type(base, ref_type) == ResultKind::StringArray {
                6
            } else {
                string_max_bytes_raw(base, ref_type, ref_bytes)
            }
        }
        Node::Call { name, args } => {
            let id = match func_id(name) {
                Some(i) => i,
                None => return MAX_STRING_BYTES,
            };
            call_str_bytes(str_len_rule(id), args, ref_type, ref_bytes)
        }
        Node::Seq(stmts) => stmts
            .last()
            .map(|n| string_max_bytes_raw(n, ref_type, ref_bytes))
            .unwrap_or(MAX_STRING_BYTES),
        Node::Assign { value, .. } => string_max_bytes_raw(value, ref_type, ref_bytes),
        // Non-string-producing nodes shouldn't reach here when deduce_type==String, but be safe.
        _ => MAX_STRING_BYTES,
    }
}

/// The byte length of an operand **coerced to text**. A String operand returns its own byte width
/// directly (no ×4); other scalars use `(chars+1)*4`.
fn coerce_str_bytes(
    node: &Node,
    ref_type: &dyn Fn(RefKind, &str) -> Option<ResultKind>,
    ref_bytes: &dyn Fn(RefKind, &str) -> Option<i32>,
) -> i32 {
    use ResultKind as K;
    let k = deduce_type(node, ref_type);
    match k {
        K::String => string_max_bytes_raw(node, ref_type, ref_bytes),
        K::Boolean => COERCE_BOOLEAN_CHARS * 4, // 15*4 = 60 (no +1)
        K::Number => (COERCE_NUMBER_CHARS + 1) * 4,
        K::Currency => (COERCE_CURRENCY_CHARS + 1) * 4,
        K::Date => (COERCE_DATE_CHARS + 1) * 4, // bare = (60+1)*4 = 244
        K::Time => (COERCE_TIME_CHARS + 1) * 4,
        K::DateTime => (COERCE_DATE_CHARS + COERCE_TIME_CHARS + 1) * 4,
        _ => MAX_STRING_BYTES,
    }
}

fn call_str_bytes(
    rule: StrLenRule,
    args: &[Node],
    ref_type: &dyn Fn(RefKind, &str) -> Option<ResultKind>,
    ref_bytes: &dyn Fn(RefKind, &str) -> Option<i32>,
) -> i32 {
    let bytes = |n: &Node| string_max_bytes_raw(n, ref_type, ref_bytes);
    match rule {
        // `ToText`/`CStr`: coerce arg0 to text. Unlike a bare `&`-operand coercion, the call can
        // carry format args (args[1..]) that change the Number/Currency char count,
        // e.g. `ToText(x,'#')` → 41 chars → 168 bytes (not the bare-number 75 → 304).
        StrLenRule::Coerce => match args.first() {
            None => MAX_STRING_BYTES,
            Some(target) => {
                use ResultKind as K;
                match deduce_type(target, ref_type) {
                    K::String => string_max_bytes_raw(target, ref_type, ref_bytes),
                    K::Boolean => COERCE_BOOLEAN_CHARS * 4,
                    K::Number => (number_chars(args, false) + 1) * 4,
                    K::Currency => (number_chars(args, true) + 1) * 4,
                    K::Date => (COERCE_DATE_CHARS + 1) * 4,
                    K::Time => (COERCE_TIME_CHARS + 1) * 4,
                    K::DateTime => (COERCE_DATE_CHARS + COERCE_TIME_CHARS + 1) * 4,
                    _ => MAX_STRING_BYTES,
                }
            }
        },
        StrLenRule::CopyArg1 => args.first().map(bytes).unwrap_or(MAX_STRING_BYTES),
        StrLenRule::Const(n) => n,
        StrLenRule::ArgTimes4 => const_number(args.get(1))
            .map(|n| n * 4)
            .unwrap_or(MAX_STRING_BYTES),
        StrLenRule::SpaceRule => const_number(args.first())
            .map(|n| n * 2 + 2)
            .unwrap_or(MAX_STRING_BYTES),
        StrLenRule::ReplicateRule => match (args.first(), const_number(args.get(1))) {
            (Some(a), Some(n)) => (bytes(a) - 2) * n + 2,
            _ => MAX_STRING_BYTES,
        },
        StrLenRule::BranchMax => args.iter().map(bytes).max().unwrap_or(MAX_STRING_BYTES),
        StrLenRule::Unbounded => MAX_STRING_BYTES,
    }
}

/// The wide-char length of a string-literal argument, or `None` if the arg is absent / not a string
/// literal.
fn str_lit_len(node: Option<&Node>) -> Option<i32> {
    match node {
        Some(Node::Str(s)) => Some(s.chars().count() as i32),
        _ => None,
    }
}

/// The character count of a Number/Currency coerced to text, as a function of the `ToText` args.
/// `args[0]` is the value; `args[1..]` are the optional format args. Bytes = `(result + 1) * 4`.
fn number_chars(args: &[Node], currency: bool) -> i32 {
    let tail = if currency { 12 } else { 7 }; // sign + padding (Number +7, Currency +12)
                                              // Form A vs B depends on whether arg1 is a string literal (the `'#'`-style format string).
    match args.get(1) {
        Some(Node::Str(fmt)) => {
            // Form A: explicit format string. Thousand/decimal separators come from args[2]/args[3].
            let chars: Vec<char> = fmt.chars().collect();
            let dot = chars.iter().position(|&c| c == '.').map(|p| p as i32);
            let int_digits = match dot {
                Some(p) if (0..=0x11).contains(&p) => p,
                _ => 18,
            };
            let dec_digits = chars.len() as i32 - 1 - dot.unwrap_or(-1);
            let groups = chars.iter().filter(|&&c| c == ',').count() as i32;
            let thou_sep = str_lit_len(args.get(2)).unwrap_or(5); // default 5
            let dec_sep = str_lit_len(args.get(3)).unwrap_or(15); // default 15
            groups * thou_sep + dec_digits + int_digits + dec_sep + tail
        }
        a1 => {
            // Form B: numeric / absent places arg. intDigits is always 18; groups always 5.
            let dec_digits = match a1 {
                Some(n) => const_number(Some(n)).unwrap_or(10), // numeric-literal value
                None => 10,                                     // default
            };
            let thou_sep = str_lit_len(args.get(2)).unwrap_or(5);
            let dec_sep = str_lit_len(args.get(3)).unwrap_or(15);
            5 * thou_sep + dec_digits + 18 + dec_sep + tail
        }
    }
}

/// A literal non-negative integer arg, for the const-folded length rules (Left/Right/Space/…).
fn const_number(node: Option<&Node>) -> Option<i32> {
    match node {
        Some(Node::Number(s)) => s.trim().parse::<f64>().ok().map(|v| v.max(0.0) as i32),
        _ => None,
    }
}

fn cap(n: i32) -> i32 {
    if n <= 0 || n > MAX_STRING_BYTES {
        MAX_STRING_BYTES
    } else {
        n
    }
}

// funcID lookup tables (NAME_FUNCID, return_rule, str_len_rule). Included into this module so they
// share `ReturnRule`/`StrLenRule`.
include!("types_table.rs");
