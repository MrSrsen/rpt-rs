# Validation

The [`parser`](01-architecture.md) only reports *syntactic* recovery — an unexpected token it had to skip to keep making
progress. Everything that is well-formed but wrong — a misspelled function, a call with too few arguments, arithmetic on
a string, a reference to a field that doesn't exist — is the job of the **validation pass** in `validate.rs`. It is an
additive, parity-neutral analysis: it walks a parsed `Node` tree and produces `Diagnostic`s, and it touches neither
reference counting (`refs`) nor evaluation.

It is designed as the diagnostics source for a formula language server: every diagnostic is spanned and severity-tagged.

## Checking a whole report from the CLI

`rpt formulas <file.rpt>` runs everything on this page over every formula a report defines — formula fields, the record-
and group-selection formulas, and conditional-format formulas wherever they hang (on a section, an object's format, its
border, or a field/text object's font color), in the main report and every subreport — reading the `.rpt`
alone, with no database and no render. It **lists each formula**, marked `ok` / `warn` / `ERROR` / `empty`, with the
findings indented beneath (message, byte offset, offending source text), and exits 1 if any formula has an **error**
(warnings alone do not fail), so it works as a CI gate. `--source` quotes each formula's body under its line; `--json`
always includes the source plus each formula's kind, syntax, and size; `--quiet` reports only through the exit status.

This is the only thing in the project that *rejects* a broken formula. Everywhere else the parser recovers, the
evaluator runs the partial parse, and the field renders blank — indistinguishable from a null value or an unimplemented
builtin. On the render path that now raises a `FormulaParse` diagnostic, but reaching it means having a datasource and
rendering the whole report.

## Entry points

```rust
use rpt_formula::{parse, validate, validate_str, Syntax, ValidationContext};

let ctx = ValidationContext::default ();

// AST-only: whole-formula spans (0..0). Use when you already hold a parsed Node.
let (node, _parse_diags) = parse(src, Syntax::Crystal);
let ast_diags = validate( & node, & ctx);

// Source-aware: precise token spans, and includes the parser's own diagnostics. Prefer this in an
// LSP (which always has the source).
let src_diags = validate_str(src, Syntax::Crystal, & ctx);
```

Both are exposed as `rpt_formula::{validate, validate_str, ValidationContext}`.

Each `Diagnostic` carries a `message`, a byte span (`start..end`), and a `severity` — everything an editor needs to
underline the offending text:

```rust
use rpt_formula::{validate_str, Severity, Syntax, ValidationContext};

let ctx = ValidationContext::default ().with_fields(["Customer.Country"]);
for d in validate_str("{Customer.Country} & Uppercas(1)", Syntax::Crystal, & ctx) {
let kind = if d.severity == Severity::Error { "error" } else { "warning" };
println ! ("{kind} at {}..{}: {}", d.start, d.end, d.message);
}
```

## Diagnostic categories

| Category                | Fires on                                                                                                        | Severity                                                                                                                                                                                         |
|-------------------------|-----------------------------------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| **Unknown function**    | a `name(args…)` call whose name is neither a built-in (the type table) nor a declared custom function         | error when a custom-function set is supplied and the name is not in it; otherwise **warning** (it may be an undeclared custom function). A nearest built-in within edit distance 2 is suggested. |
| **Function arity**      | a built-in called with a structurally wrong argument count                                                      | error                                                                                                                                                                                            |
| **Operator type error** | a binary/unary operator applied to statically-known incompatible operand types (e.g. `"a" - 1`, `Not "x"`)      | error                                                                                                                                                                                            |
| **Unknown reference**   | a `{field}` / `{?param}` / `{@formula}` / `{#rt}` / `{%sql}` whose name is absent from the matching context set | error                                                                                                                                                                                            |

### Arity

The native engine's per-function signatures are not tabulated, so arity is only checked for the functions whose argument
*shape* the type system already encodes via its return rule: `IIf` (exactly 3), `Switch` (an even count ≥ 2), `Choose`
(≥ 2), and the aggregate / argument-copying families (≥ 1). Functions with a fixed or opaque return rule are not
arity-checked.

### Operator types

Operand types come from [`deduce_type`](01-architecture.md). The validator injects **no** reference type map, so a
`{field}`/`{?param}` resolves to `Unknown` and is never flagged — operator checks fire only on statically-typed operands
(literals and typed built-ins), which keeps the pass free of false positives on data-dependent values. `&`
(concatenation) always type-checks, since it coerces its operands.

## The `ValidationContext`

Cross-reference checks need to know which names exist, but `rpt-formula` is standalone and has no view of a report's
schema. The caller injects that knowledge through `ValidationContext`, which carries an **optional** set per reference
kind plus an optional custom-function set:

```rust
let ctx = ValidationContext::default()
    .with_fields(["Customer.Country", "Orders.Total"])
    .with_parameters(["Region"])
    .with_formulas(["subtotal"])
    .with_functions(["MyCustomFn"]);
```

Each set is optional and independent: a kind whose set is **absent** is not checked at all (an unknown field with no
field set supplied is silently accepted), so an empty `ValidationContext`
runs only the intrinsic function / arity / operator checks. Membership is case-insensitive.

## Spans

The `Node` AST carries no source offsets, so spans are recovered from the token stream:
`validate_str` tokenizes the source and points each **name** (unknown/arity) and **reference**
diagnostic at its exact token, matching occurrences in source order. Operator diagnostics use a whole-formula span (the
AST doesn't identify which operator token produced the type error). The AST-only `validate` cannot recover offsets and
reports every diagnostic against a `0..0` span — it is for callers that only have a `Node`; anything editor-facing
should use `validate_str`.

---

← [Builtin functions](03-builtins.md) · **Back to the** [formula-engine index](README.md)
