# The formula engine

Crystal Reports formulas are a small expression/statement language embedded in a report: field-derived columns
(`{@formula}`), record- and group-selection conditions, and conditional formatting all evaluate a formula against the
current record. The **`crystal-formula`** crate is a complete, safe-Rust implementation of that language: a lexer, a
recursive-descent parser, an AST, a static type system, a bytecode compiler, and a stack VM. Consumers across the
workspace (the data/layout pipeline and the XML exporter) depend on `crystal-formula` directly.

## Quick start

The crate depends only on `rpt-format-value`, so parsing and evaluating a formula stands on its own. Parse a body under
a `Syntax`, bind the values it references through an `EvalContext`, and evaluate:

```rust
use crystal_formula::eval::{eval, MapContext};
use crystal_formula::{parse, RefKind, Syntax, Value};

let (ast, diags) = parse("{Orders.Quantity} * {Orders.Price}", Syntax::Crystal);
assert!(diags.is_empty());

let ctx = MapContext::default()
    .with_field(RefKind::Field, "Orders.Quantity", Value::Number(3.0))
    .with_field(RefKind::Field, "Orders.Price", Value::Number(25.0));

assert_eq!(eval(&ast, &ctx).unwrap(), Value::Number(75.0));
```

Both author-facing dialects parse the same way — pass `Syntax::Basic` for Basic syntax. See
[01 — Architecture › Using the crate](01-architecture.md#using-the-crate) for the Basic form and a literal-only
example, and [04 — Validation](04-validation.md) for editor-grade diagnostics.

### Why a standalone crate

The formula language lives in its own crate rather than as a module of the reader or the XML exporter:

- **It is genuinely independent of the `.rpt` binary container.** A formula body is a text language; parsing and
  evaluating it has nothing to do with the CFB/OLE2 file format, so it belongs behind its own boundary.
- **It has no dependency on the `rpt` decoder** — only on `rpt-format-value` (a dependency-free leaf, because a `Value`
  carries `Date`/`Time`). So it is reusable without pulling in the whole binary decoder: the upcoming Crystal LSP server,
  a WASM formula sandbox, and a standalone validator/playground can all depend on just `crystal-formula`.
- **Cross-boundary type mappings stay with their consumers.** `crystal-formula` exposes its own `ResultKind`; any code
  that needs to relate a formula's result kind to the `rpt` model's `FieldValueType` does so in the consumer that knows
  both types, never by coupling the formula crate to the model.

The extraction was a mechanical move — the parser, VM, builtins, and type tables are unchanged — so downstream code was
unaffected.

This folder documents the engine as it ships, in four parts:

| Doc | Covers |
| --- | ------ |
| [01 — Architecture & VM](01-architecture.md) | The pipeline (source → lexer → parser → AST → compiler → VM), the value model, variable scopes, references, the per-record cache, and error handling. |
| [02 — Language reference](02-language.md) | Both dialects (Crystal & Basic syntax): lexis, operators/precedence, expressions, statement bodies, literals, comments — with an EBNF sketch and examples. |
| [03 — Builtin functions](03-builtins.md) | The builtin library by family (string / math / date-time / conversion / …): signatures, semantics, Crystal-specific rules, and implementation status. |
| [04 — Validation](04-validation.md) | The semantic diagnostics pass: diagnostic categories, the injected `ValidationContext`, span sourcing, and how it feeds the Crystal LSP. |

The grammar `crystal-formula` implements — token codes, the 17-level precedence ladder, statement productions, and the
Crystal-vs-Basic split — was reconstructed from the native engine; this documentation describes what the crate actually
implements.

## Source map (`crates/crystal-formula/src/`)

| Path | Role |
| ---- | ---- |
| `token.rs` | Token kinds, the five `{...}` reference classes, and the unified operator/punctuation codes. |
| `lexer.rs` | Error-tolerant tokenizer (`tokenize`) for both syntaxes. |
| `ast.rs` | The `Node` AST. |
| `parser.rs` | Error-recovering recursive descent (`parse`) — never panics; emits `Diagnostic`s. |
| `types.rs` (+ `types_table.rs`) | Static result-kind and string-length deduction, keyed by the engine's funcID table. |
| `refs.rs` | Token-stream reference extraction (`references`) used by reference counting — independent of the parser. |
| `validate.rs` | Semantic diagnostics pass (`validate` / `validate_str`, `ValidationContext`): unknown functions, arity, operator type errors, unknown references — the source for the Crystal LSP (see [04](04-validation.md)). |
| `eval/mod.rs` | The tree-walking `Evaluator` (the reference implementation) + shared operator/value logic + `EvalContext`. |
| `eval/vm.rs` | The bytecode compiler (`compile`) and stack VM (`run`) — the default runtime path. |
| `eval/value.rs` | The runtime `Value` union and its default text coercion. |
| `eval/builtins/` | The builtin library, split by family (`string`, `math`, `datetime`, `conversion`, `financial`, `statistical`, `numeral`), with the null-propagation rule and the name→variant router in `mod.rs`. |
