//! Record-selection push-down.
//!
//! The native engine pushes the *translatable* part of a report's `RecordSelectionFormula` into the
//! SQL `WHERE` so the server returns fewer rows, then applies the rest per-record. This module does
//! the translatable half: it parses the Crystal formula and emits
//! SQL for the conjuncts it can prove equivalent, and reports whether the *whole* formula was
//! captured. Anything it cannot prove equivalent is left out; the pipeline still evaluates the full
//! formula client-side, so the result set is unchanged — this is a fetch-size optimization, never a
//! correctness lever, so when in doubt a construct is **not** pushed down.
//!
//! The translatable subset:
//! - field-vs-literal comparisons (`= <> < > <= >=`) combined with `And`/`Or`/`Not`,
//! - `In`-lists (`{f} in [a, b, c]` → `IN (…)`),
//! - `To`-ranges (`{f} in a to b` → `BETWEEN` / half-open bound pairs),
//! - `StartsWith` / `Like` (→ SQL `LIKE … ESCAPE '\'`, translating Crystal's `*`/`?` wildcards),
//! - `IsNull({f})` (→ `IS NULL`),
//! - date literals (`#2024-01-31#` → `DATE '…'`).
//!
//! Parameter binding: a `{?Name}` reference resolves to that parameter's current value
//! (supplied by the caller), emitted as its SQL literal — so a selection comparing a field to a
//! parameter pushes the concrete value into the `WHERE`.

use crate::{quote_ident, QueryColumn};
use crystal_formula::ast::Node;
use crystal_formula::eval::Value;
use crystal_formula::token::op;
use crystal_formula::{parse, RefKind, Syntax};
use std::collections::HashMap;

/// The result of pushing a selection formula down to SQL.
#[derive(Debug, Clone, PartialEq)]
pub struct PushDown {
    /// The `WHERE` body (without the `WHERE` keyword), or `None` if nothing was translatable.
    pub where_sql: Option<String>,
    /// Whether the *entire* formula was captured by `where_sql`. When `false`, the caller must still
    /// apply the formula per-row (it always may — this is purely an optimization hint).
    pub fully_pushed: bool,
}

/// Translate the pushable subset of `formula` against the query's `columns`. An empty/blank formula
/// pushes nothing and is trivially fully-captured. No parameter values are bound (a `{?Name}`
/// reference is left to the per-row pipeline); use [`push_down_selection_with_params`] to bind them.
pub fn push_down_selection(formula: &str, columns: &[QueryColumn]) -> PushDown {
    push_down_selection_with_params(formula, columns, &[])
}

/// Like [`push_down_selection`] but binds parameter current-values (`{?Name}` → its SQL literal),
/// so a selection comparing a field to a parameter pushes the concrete value into the `WHERE`.
/// `params` are `(name, value)` pairs (the name matched case-insensitively, with an
/// optional leading `?`).
pub fn push_down_selection_with_params(
    formula: &str,
    columns: &[QueryColumn],
    params: &[(String, Value)],
) -> PushDown {
    if formula.trim().is_empty() {
        return PushDown {
            where_sql: None,
            fully_pushed: true,
        };
    }
    let lookup = FieldLookup::new(columns);
    let params = ParamLookup::new(params);
    let (root, _) = parse(formula, Syntax::Crystal);

    // Split the top-level `And` chain: each conjunct can be pushed or kept independently.
    let conjuncts = split_and(&root);
    let mut pushed: Vec<String> = Vec::new();
    let mut all = true;
    for c in &conjuncts {
        match translate(c, &lookup, &params) {
            Some(sql) => pushed.push(sql),
            None => all = false,
        }
    }

    PushDown {
        where_sql: if pushed.is_empty() {
            None
        } else {
            Some(pushed.join(" AND "))
        },
        fully_pushed: all && !conjuncts.is_empty(),
    }
}

/// Flatten a top-level `And` chain (and unwrap a single-statement `Seq`) into its conjuncts.
fn split_and(node: &Node) -> Vec<&Node> {
    match node {
        Node::Binary { op, left, right } if *op == op::AND => {
            let mut v = split_and(left);
            v.extend(split_and(right));
            v
        }
        Node::Seq(stmts) if stmts.len() == 1 => split_and(&stmts[0]),
        other => vec![other],
    }
}

/// Translate a boolean expression node to SQL, or `None` if any part is untranslatable.
fn translate(node: &Node, lookup: &FieldLookup, params: &ParamLookup) -> Option<String> {
    match node {
        Node::Binary { op, left, right } => {
            let o = *op;
            if o == op::AND || o == op::OR {
                let l = translate(left, lookup, params)?;
                let r = translate(right, lookup, params)?;
                let kw = if o == op::AND { "AND" } else { "OR" };
                Some(format!("({l} {kw} {r})"))
            } else if let Some(cmp) = compare_op(o) {
                let l = operand(left, lookup, params)?;
                let r = operand(right, lookup, params)?;
                Some(format!("{l} {cmp} {r}"))
            } else if o == op::IN {
                translate_in(left, right, lookup, params)
            } else if o == op::LIKE || o == op::STARTS_WITH {
                translate_like(o, left, right, lookup, params)
            } else {
                None
            }
        }
        Node::Unary { op, expr } if *op == op::NOT => {
            let x = translate(expr, lookup, params)?;
            Some(format!("(NOT {x})"))
        }
        // `IsNull({f})` — the only translatable call form.
        Node::Call { name, args } if name.eq_ignore_ascii_case("isnull") && args.len() == 1 => {
            let f = field_sql(&args[0], lookup)?;
            Some(format!("{f} IS NULL"))
        }
        _ => None,
    }
}

/// Translate `{field} In <right>`: either an array literal (`IN (…)`) or a `To`-range (`BETWEEN` /
/// half-open bounds). The left side must be a field reference.
fn translate_in(
    left: &Node,
    right: &Node,
    lookup: &FieldLookup,
    params: &ParamLookup,
) -> Option<String> {
    let f = field_sql(left, lookup)?;
    match right {
        Node::Array(items) => {
            if items.is_empty() {
                return None; // `IN ()` is not valid SQL — leave it to the pipeline.
            }
            let vals = items
                .iter()
                .map(|i| operand(i, lookup, params))
                .collect::<Option<Vec<_>>>()?;
            Some(format!("{f} IN ({})", vals.join(", ")))
        }
        // `{f} in lo to hi` — a range membership test.
        Node::Binary {
            op: range_op,
            left: lo,
            right: hi,
        } if is_range_op(*range_op) => translate_range(&f, *range_op, lo, hi, lookup, params),
        _ => None,
    }
}

/// Whether `op` is one of the four `To`-range operators.
fn is_range_op(o: u8) -> bool {
    matches!(
        o,
        op::RANGE_TO | op::RANGE_LO_EXCL | op::RANGE_HI_EXCL | op::RANGE_BOTH_EXCL
    )
}

/// Translate `field <range-op> lo..hi` membership to SQL. The fully-closed `To` maps to `BETWEEN`;
/// the half-open forms map to an explicit `>`/`>=` … `AND` … `<`/`<=` pair.
fn translate_range(
    f: &str,
    range_op: u8,
    lo: &Node,
    hi: &Node,
    lookup: &FieldLookup,
    params: &ParamLookup,
) -> Option<String> {
    let lo = operand(lo, lookup, params)?;
    let hi = operand(hi, lookup, params)?;
    if range_op == op::RANGE_TO {
        return Some(format!("{f} BETWEEN {lo} AND {hi}"));
    }
    let lo_op = if range_op == op::RANGE_LO_EXCL || range_op == op::RANGE_BOTH_EXCL {
        ">"
    } else {
        ">="
    };
    let hi_op = if range_op == op::RANGE_HI_EXCL || range_op == op::RANGE_BOTH_EXCL {
        "<"
    } else {
        "<="
    };
    Some(format!("({f} {lo_op} {lo} AND {f} {hi_op} {hi})"))
}

/// Translate `{field} StartsWith "x"` / `{field} Like "pat"` to a SQL `LIKE … ESCAPE '\'`. The right
/// side must be a string literal (so the pattern is known at build time and can be escaped safely).
/// `StartsWith` is an anchored literal prefix; `Like` translates Crystal's `*`/`?` wildcards to SQL
/// `%`/`_` while escaping any literal `%`/`_`/`\` in the pattern.
fn translate_like(
    o: u8,
    left: &Node,
    right: &Node,
    lookup: &FieldLookup,
    _params: &ParamLookup,
) -> Option<String> {
    let f = field_sql(left, lookup)?;
    let Node::Str(pat) = right else {
        return None; // only a literal pattern can be translated safely
    };
    let sql_pat = if o == op::STARTS_WITH {
        // Anchored literal prefix: escape LIKE metacharacters, then append `%`.
        format!("{}%", escape_like_literal(pat))
    } else {
        crystal_like_to_sql(pat)
    };
    Some(format!("{f} LIKE {} ESCAPE '\\'", sql_string(&sql_pat)))
}

/// Escape the SQL `LIKE` metacharacters (`\ % _`) in a literal so it matches verbatim (paired with
/// `ESCAPE '\'`).
fn escape_like_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Translate a Crystal `Like` pattern (DOS-style `*` = any run, `?` = one char) to a SQL `LIKE`
/// pattern, escaping any literal SQL metacharacter (`% _ \`) so only the Crystal wildcards act.
fn crystal_like_to_sql(pat: &str) -> String {
    let mut out = String::with_capacity(pat.len());
    for ch in pat.chars() {
        match ch {
            '*' => out.push('%'),
            '?' => out.push('_'),
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Translate a comparison operand (a field reference, literal, or bound parameter) to SQL.
fn operand(node: &Node, lookup: &FieldLookup, params: &ParamLookup) -> Option<String> {
    match node {
        Node::Reference {
            kind: RefKind::Field,
            name,
        } => lookup.resolve(name),
        Node::Reference {
            kind: RefKind::Parameter,
            name,
        } => params.resolve(name),
        Node::Number(s) => {
            // Validate it is a real number literal before trusting it in SQL.
            s.trim().parse::<f64>().ok().map(|_| s.trim().to_string())
        }
        Node::Str(s) => Some(sql_string(s)),
        Node::Bool(b) => Some(if *b { "TRUE" } else { "FALSE" }.to_string()),
        Node::DateLit(s) => date_literal_sql(s),
        Node::Unary { op: o, expr } if *o == op::UNARY_MINUS => {
            if let Node::Number(s) = expr.as_ref() {
                s.trim()
                    .parse::<f64>()
                    .ok()
                    .map(|_| format!("-{}", s.trim()))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The SQL for a node that must be a plain field reference (used where a literal makes no sense —
/// `IN`, `Like`, `IsNull`).
fn field_sql(node: &Node, lookup: &FieldLookup) -> Option<String> {
    match node {
        Node::Reference {
            kind: RefKind::Field,
            name,
        } => lookup.resolve(name),
        _ => None,
    }
}

/// Map a comparison opcode to its SQL operator.
fn compare_op(o: u8) -> Option<&'static str> {
    Some(match o {
        op::EQ => "=",
        op::NE => "<>",
        op::LT => "<",
        op::GT => ">",
        op::LE => "<=",
        op::GE => ">=",
        _ => return None,
    })
}

/// Translate a Crystal `#…#` date/time literal to a SQL typed literal, or `None` if it isn't a plain
/// date / datetime (a bare time, or an out-of-range component, is left to the per-row pipeline).
///
/// The literal syntax is parsed by [`crystal_formula::parse_date_literal`] — the one shared parser —
/// so every spelling it accepts (numeric `#m/d/yyyy#` / `#yyyy-m-d#`, textual `#Month d, yyyy#`, an
/// `AM`/`PM` time tail) can push down. The component-range guard keeps push-down conservative: a
/// nonsensical value the parser tolerates (e.g. month 13) falls back rather than emitting a literal
/// the database would reject.
fn date_literal_sql(raw: &str) -> Option<String> {
    match crystal_formula::parse_date_literal(raw).ok()? {
        Value::Date(d) if (1..=12).contains(&d.month) && (1..=31).contains(&d.day) => {
            Some(format!("DATE '{:04}-{:02}-{:02}'", d.year, d.month, d.day))
        }
        Value::DateTime(d, t)
            if (1..=12).contains(&d.month)
                && (1..=31).contains(&d.day)
                && t.hour <= 23
                && t.minute <= 59
                && t.second <= 59 =>
        {
            Some(format!(
                "TIMESTAMP '{:04}-{:02}-{:02} {:02}:{:02}:{:02}'",
                d.year, d.month, d.day, t.hour, t.minute, t.second
            ))
        }
        _ => None,
    }
}

/// A SQL single-quoted string literal (doubling embedded quotes).
fn sql_string(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The SQL literal form of a bound parameter [`Value`], or `None` for the kinds that have no safe
/// literal (Null / Array / Range — the pipeline still filters those).
fn value_to_sql(v: &Value) -> Option<String> {
    Some(match v {
        Value::Number(n) | Value::Currency(n) => number_literal(*n)?,
        Value::Str(s) => sql_string(s),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Value::Date(d) => format!("DATE '{:04}-{:02}-{:02}'", d.year, d.month, d.day),
        Value::Time(t) => format!("TIME '{:02}:{:02}:{:02}'", t.hour, t.minute, t.second),
        Value::DateTime(d, t) => format!(
            "TIMESTAMP '{:04}-{:02}-{:02} {:02}:{:02}:{:02}'",
            d.year, d.month, d.day, t.hour, t.minute, t.second
        ),
        // A single-element array (a multi-value param collapsed to one) can still bind; anything
        // richer (multi-element arrays, ranges) is left to the pipeline.
        Value::Array(items) if items.len() == 1 => value_to_sql(&items[0])?,
        _ => return None,
    })
}

/// Format an `f64` parameter value as a SQL numeric literal (integers without a decimal point, so
/// `42.0` binds as `42`), rejecting non-finite values.
fn number_literal(n: f64) -> Option<String> {
    if !n.is_finite() {
        return None;
    }
    if n.fract() == 0.0 && n.abs() < 1e15 {
        Some(format!("{n:.0}"))
    } else {
        Some(format!("{n}"))
    }
}

/// Resolves a `{table.field}` reference name to its qualified `"alias"."field"` SQL, by full
/// `alias.field` key or by an unambiguous bare field name. SQL Expression columns (no table alias)
/// are excluded — a `{%name}` reference is never a `{table.field}` reference, and including them
/// could spuriously make a real field name ambiguous.
struct FieldLookup {
    by_key: HashMap<String, String>,
    by_field: HashMap<String, Option<String>>, // None = ambiguous (appears in >1 table)
}

impl FieldLookup {
    fn new(columns: &[QueryColumn]) -> FieldLookup {
        let mut by_key = HashMap::new();
        let mut by_field: HashMap<String, Option<String>> = HashMap::new();
        for c in columns {
            if c.expr.is_some() {
                continue; // SQL Expression column — not a table field reference.
            }
            let sql = format!("{}.{}", quote_ident(&c.alias), quote_ident(&c.field));
            by_key.insert(c.key().to_lowercase(), sql.clone());
            by_field
                .entry(c.field.to_lowercase())
                .and_modify(|e| *e = None) // seen twice → ambiguous
                .or_insert(Some(sql));
        }
        FieldLookup { by_key, by_field }
    }

    fn resolve(&self, name: &str) -> Option<String> {
        let lname = name.to_lowercase();
        if let Some(sql) = self.by_key.get(&lname) {
            return Some(sql.clone());
        }
        // Bare field name (or `table.field` whose alias differs) — accept only if unambiguous.
        let short = lname.rsplit('.').next().unwrap_or(&lname);
        self.by_field.get(short).cloned().flatten()
    }
}

/// Resolves a `{?Name}` parameter reference to the SQL literal of its bound current value.
struct ParamLookup {
    by_name: HashMap<String, String>, // normalized name → SQL literal
}

impl ParamLookup {
    fn new(params: &[(String, Value)]) -> ParamLookup {
        let mut by_name = HashMap::new();
        for (name, value) in params {
            if let Some(sql) = value_to_sql(value) {
                by_name.insert(normalize(name), sql);
            }
        }
        ParamLookup { by_name }
    }

    fn resolve(&self, name: &str) -> Option<String> {
        self.by_name.get(&normalize(name)).cloned()
    }
}

// Normalize a parameter name for matching (drop surrounding `{}`, a leading `?`, lowercase). Reuses
// the canonical implementation from `rpt-data` — rpt-query already depends on that crate — so the two
// can never drift.
use rpt_data::normalize_param_name as normalize;

#[cfg(test)]
mod tests {
    use super::*;
    use crystal_formula::eval::{Date, Time};
    use rpt_model::FieldValueType;

    fn cols() -> Vec<QueryColumn> {
        vec![
            QueryColumn {
                alias: "orders".into(),
                field: "amount".into(),
                value_type: FieldValueType::Number,
                expr: None,
            },
            QueryColumn {
                alias: "orders".into(),
                field: "status".into(),
                value_type: FieldValueType::String,
                expr: None,
            },
        ]
    }

    #[test]
    fn simple_numeric_comparison_pushes() {
        let p = push_down_selection("{orders.amount} > 100", &cols());
        assert_eq!(p.where_sql.as_deref(), Some(r#""orders"."amount" > 100"#));
        assert!(p.fully_pushed);
    }

    #[test]
    fn string_equality_pushes_with_quoting() {
        let p = push_down_selection("{orders.status} = \"o'brien\"", &cols());
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."status" = 'o''brien'"#)
        );
        assert!(p.fully_pushed);
    }

    #[test]
    fn and_of_two_conjuncts() {
        let p = push_down_selection(
            "{orders.amount} >= 10 And {orders.status} <> \"void\"",
            &cols(),
        );
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."amount" >= 10 AND "orders"."status" <> 'void'"#)
        );
        assert!(p.fully_pushed);
    }

    #[test]
    fn partial_push_keeps_translatable_conjunct_only() {
        // Second conjunct references a formula → untranslatable; first still pushes.
        let p = push_down_selection("{orders.amount} > 0 And {@custom} = 1", &cols());
        assert_eq!(p.where_sql.as_deref(), Some(r#""orders"."amount" > 0"#));
        assert!(
            !p.fully_pushed,
            "not fully captured — pipeline still filters"
        );
    }

    #[test]
    fn untranslatable_formula_pushes_nothing() {
        let p = push_down_selection("{@x} > 0", &cols());
        assert_eq!(p.where_sql, None);
        assert!(!p.fully_pushed);
    }

    #[test]
    fn empty_formula_is_noop() {
        let p = push_down_selection("", &cols());
        assert_eq!(p.where_sql, None);
        assert!(p.fully_pushed);
    }

    #[test]
    fn or_and_not_translate() {
        let p = push_down_selection(
            "Not ({orders.status} = \"x\") Or {orders.amount} = 5",
            &cols(),
        );
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#"((NOT "orders"."status" = 'x') OR "orders"."amount" = 5)"#)
        );
    }

    // --- richer predicates ---------------------------------------------------------------------

    #[test]
    fn in_list_pushes() {
        let p = push_down_selection("{orders.status} in [\"a\", \"b\", \"c\"]", &cols());
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."status" IN ('a', 'b', 'c')"#)
        );
        assert!(p.fully_pushed);
    }

    #[test]
    fn numeric_in_list_pushes() {
        let p = push_down_selection("{orders.amount} in [1, 2, 3]", &cols());
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."amount" IN (1, 2, 3)"#)
        );
    }

    #[test]
    fn to_range_becomes_between() {
        let p = push_down_selection("{orders.amount} in 10 to 20", &cols());
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."amount" BETWEEN 10 AND 20"#)
        );
        assert!(p.fully_pushed);
    }

    #[test]
    fn half_open_range_expands_to_bounds() {
        let p = push_down_selection("{orders.amount} in 10 to_ 20", &cols());
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#"("orders"."amount" >= 10 AND "orders"."amount" < 20)"#)
        );
    }

    #[test]
    fn startswith_pushes_as_like_prefix() {
        let p = push_down_selection("{orders.status} startswith \"po\"", &cols());
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."status" LIKE 'po%' ESCAPE '\'"#)
        );
        assert!(p.fully_pushed);
    }

    #[test]
    fn startswith_escapes_like_metacharacters() {
        let p = push_down_selection("{orders.status} startswith \"50%_\"", &cols());
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."status" LIKE '50\%\_%' ESCAPE '\'"#)
        );
    }

    #[test]
    fn like_translates_crystal_wildcards() {
        // Crystal `*` → SQL `%`, `?` → `_`; a literal `%` in the pattern is escaped.
        let p = push_down_selection("{orders.status} like \"A?C*\"", &cols());
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."status" LIKE 'A_C%' ESCAPE '\'"#)
        );
    }

    #[test]
    fn isnull_pushes_is_null() {
        let p = push_down_selection("IsNull({orders.status})", &cols());
        assert_eq!(p.where_sql.as_deref(), Some(r#""orders"."status" IS NULL"#));
        assert!(p.fully_pushed);
    }

    #[test]
    fn not_isnull_pushes() {
        let p = push_down_selection("Not IsNull({orders.status})", &cols());
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#"(NOT "orders"."status" IS NULL)"#)
        );
    }

    #[test]
    fn date_literal_pushes_as_typed_literal() {
        let mut c = cols();
        c.push(QueryColumn {
            alias: "orders".into(),
            field: "ship_date".into(),
            value_type: FieldValueType::Date,
            expr: None,
        });
        let p = push_down_selection("{orders.ship_date} >= #2024-01-31#", &c);
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."ship_date" >= DATE '2024-01-31'"#)
        );
    }

    #[test]
    fn datetime_literal_pushes_as_timestamp() {
        let mut c = cols();
        c.push(QueryColumn {
            alias: "orders".into(),
            field: "ship_date".into(),
            value_type: FieldValueType::DateTime,
            expr: None,
        });
        let p = push_down_selection("{orders.ship_date} < #2024-01-31 08:30:00#", &c);
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."ship_date" < TIMESTAMP '2024-01-31 08:30:00'"#)
        );
    }

    #[test]
    fn unparseable_date_literal_is_not_pushed() {
        let mut c = cols();
        c.push(QueryColumn {
            alias: "orders".into(),
            field: "ship_date".into(),
            value_type: FieldValueType::Date,
            expr: None,
        });
        // A garbage spelling the shared parser rejects → left to the pipeline.
        let p = push_down_selection("{orders.ship_date} >= #not a date#", &c);
        assert_eq!(p.where_sql, None);
        assert!(!p.fully_pushed);
    }

    #[test]
    fn us_numeric_date_literal_pushes_as_date() {
        let mut c = cols();
        c.push(QueryColumn {
            alias: "orders".into(),
            field: "ship_date".into(),
            value_type: FieldValueType::Date,
            expr: None,
        });
        // US `m/d/yyyy` spelling, accepted by the shared parser → normalized to ISO in the SQL.
        let p = push_down_selection("{orders.ship_date} >= #1/31/2024#", &c);
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."ship_date" >= DATE '2024-01-31'"#)
        );
    }

    #[test]
    fn textual_month_date_literal_pushes_as_date() {
        let mut c = cols();
        c.push(QueryColumn {
            alias: "orders".into(),
            field: "ship_date".into(),
            value_type: FieldValueType::Date,
            expr: None,
        });
        // The textual `Month d, yyyy` form now pushes down via the shared parser.
        let p = push_down_selection("{orders.ship_date} >= #January 5, 2024#", &c);
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."ship_date" >= DATE '2024-01-05'"#)
        );
    }

    #[test]
    fn am_pm_datetime_literal_pushes_as_timestamp() {
        let mut c = cols();
        c.push(QueryColumn {
            alias: "orders".into(),
            field: "ship_date".into(),
            value_type: FieldValueType::DateTime,
            expr: None,
        });
        // A `PM` time tail is folded to 24-hour before formatting the SQL literal.
        let p = push_down_selection("{orders.ship_date} < #2024-01-31 8:30:00 PM#", &c);
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."ship_date" < TIMESTAMP '2024-01-31 20:30:00'"#)
        );
    }

    #[test]
    fn out_of_range_date_literal_is_not_pushed() {
        let mut c = cols();
        c.push(QueryColumn {
            alias: "orders".into(),
            field: "ship_date".into(),
            value_type: FieldValueType::Date,
            expr: None,
        });
        // Month 13 parses but is nonsensical → left to the pipeline, never emitted as SQL.
        let p = push_down_selection("{orders.ship_date} >= #2024-13-01#", &c);
        assert_eq!(p.where_sql, None);
        assert!(!p.fully_pushed);
    }

    #[test]
    fn empty_in_list_not_pushed() {
        let p = push_down_selection("{orders.status} in []", &cols());
        assert_eq!(p.where_sql, None);
    }

    // --- parameter binding -----------------------------------------------------------------------

    #[test]
    fn parameter_value_binds_into_where() {
        let params = vec![("Threshold".to_string(), Value::Number(42.0))];
        let p = push_down_selection_with_params("{orders.amount} > {?Threshold}", &cols(), &params);
        assert_eq!(p.where_sql.as_deref(), Some(r#""orders"."amount" > 42"#));
        assert!(p.fully_pushed);
    }

    #[test]
    fn string_parameter_binds_quoted() {
        let params = vec![("Stat".to_string(), Value::Str("o'brien".into()))];
        let p = push_down_selection_with_params("{orders.status} = {?Stat}", &cols(), &params);
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."status" = 'o''brien'"#)
        );
    }

    #[test]
    fn date_parameter_binds_as_date_literal() {
        let mut c = cols();
        c.push(QueryColumn {
            alias: "orders".into(),
            field: "ship_date".into(),
            value_type: FieldValueType::Date,
            expr: None,
        });
        let params = vec![("D".to_string(), Value::Date(Date::new(2024, 3, 5)))];
        let p = push_down_selection_with_params("{orders.ship_date} >= {?D}", &c, &params);
        assert_eq!(
            p.where_sql.as_deref(),
            Some(r#""orders"."ship_date" >= DATE '2024-03-05'"#)
        );
    }

    #[test]
    fn time_parameter_binds() {
        // Just exercise value_to_sql for Time (via a param) to confirm the literal shape.
        let params = vec![("T".to_string(), Value::Time(Time::new(0, 0, 0)))];
        let pl = ParamLookup::new(&params);
        assert_eq!(pl.resolve("T").as_deref(), Some("TIME '00:00:00'"));
    }

    #[test]
    fn missing_parameter_is_not_pushed() {
        // No bound value → the conjunct can't be translated; the pipeline still filters.
        let p = push_down_selection_with_params("{orders.amount} > {?Unset}", &cols(), &[]);
        assert_eq!(p.where_sql, None);
        assert!(!p.fully_pushed);
    }

    #[test]
    fn null_parameter_is_not_pushed() {
        let params = vec![("N".to_string(), Value::Null)];
        let p = push_down_selection_with_params("{orders.amount} = {?N}", &cols(), &params);
        assert_eq!(p.where_sql, None);
    }

    #[test]
    fn expression_columns_excluded_from_field_lookup() {
        // An expr column named `amount` must not shadow the real `orders.amount` field.
        let mut c = cols();
        c.push(QueryColumn {
            alias: String::new(),
            field: "amount".into(),
            value_type: FieldValueType::String,
            expr: Some("1 + 1".into()),
        });
        let p = push_down_selection("{orders.amount} > 100", &c);
        assert_eq!(p.where_sql.as_deref(), Some(r#""orders"."amount" > 100"#));
    }
}
