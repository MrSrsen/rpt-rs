# Host-provided functions

A formula may call a function supplied by its original application rather than by Crystal itself.
An application embedding `rpt-formula` can implement `EvalContext::call_function` to supply its own
Rust implementation. A call such as `HostDouble({orders.amount})` then evaluates through the host
without changing the formula or adding a vendor-specific entry to the built-in table.

## Contract

- The hook receives a lowercase name and arguments already evaluated from left to right. An error
  evaluating an argument prevents the host call. Calls in branches that are not evaluated do not run.
- `None` means the function is not registered, preserving the existing unknown-name error.
  `Some(Ok(Value::Null))` is a successfully returned null. `Some(Err(error))` propagates the host's
  failure; the spanned VM API identifies the call expression that failed.
- The host validates argument count and types and decides how to handle null values. Unlike ordinary
  built-ins, host calls receive null arguments without automatic null propagation. Field-level
  `NullTreatment::DefaultValue` still substitutes field defaults before arguments reach the hook.
- Only explicit calls (`HostZero()`) reach the hook; a bare `HostZero` retains identifier semantics.
- Built-in names remain reserved, including functions recognized by the type table but not yet
  implemented. A host cannot override built-ins, special fields, or language constructs.
- The default implementation returns `None`. Existing `EvalContext` implementations do not need new
  methods or fields, and the bytecode VM and reference evaluator use the same dispatch path.

Callbacks are trusted application code, not a sandbox. They must bound their own execution: the
formula VM's loop limit cannot interrupt a callback. No dynamic DLL loading, UFL binary compatibility,
or automatic execution of function bodies stored in reports is provided by this hook.

## Example

Keep a registry in the embedding application and use its keys for validation as well as execution.
This example adds a numeric function without adding dependencies or changing the parser:

```rust
use std::collections::BTreeMap;
use rpt_formula::{parse, validate_str, Syntax, ValidationContext};
use rpt_formula::eval::{eval, EvalContext, EvalError, MapContext, Value};
use rpt_formula::token::RefKind;

type Function = fn(&[Value]) -> Result<Value, EvalError>;

struct Host {
    fields: MapContext,
    functions: BTreeMap<String, Function>,
}

impl EvalContext for Host {
    fn resolve(&self, kind: RefKind, name: &str) -> Option<Value> {
        self.fields.resolve(kind, name)
    }

    fn call_function(&self, name: &str, args: &[Value]) -> Option<Result<Value, EvalError>> {
        self.functions.get(name).map(|function| function(args))
    }
}

fn double(args: &[Value]) -> Result<Value, EvalError> {
    match args {
        [Value::Number(value)] => Ok(Value::Number(value * 2.0)),
        [Value::Null] => Ok(Value::Null),
        [value] => Err(EvalError::TypeMismatch {
            what: "hostdouble".into(), got: value.type_name().into(),
        }),
        _ => Err(EvalError::Arity {
            name: "hostdouble".into(), expected: "1 argument".into(), got: args.len(),
        }),
    }
}

let mut host = Host {
    fields: MapContext::default()
        .with_field(RefKind::Field, "orders.amount", Value::Number(21.0)),
    functions: BTreeMap::new(),
};
host.functions.insert("hostdouble".into(), double);

let source = "HostDouble({orders.amount})";
let validation = ValidationContext::default().with_functions(host.functions.keys().cloned());
assert!(validate_str(source, Syntax::Crystal, &validation).is_empty());
let (ast, diagnostics) = parse(source, Syntax::Crystal);
assert!(diagnostics.is_empty());
assert_eq!(eval(&ast, &host), Ok(Value::Number(42.0)));
```

The validator's existing `with_functions` declares names, not signatures: a declared host function's
arity and argument types are checked by the callback at runtime. Static return-type inference remains
`Unknown`. Declaring a name does not make it executable without the corresponding host implementation.

The example implements only field resolution and host calls. A host with parameters, print-state
specials, shared variables, summaries, or grouping must also provide the relevant `EvalContext`
methods. Wrapping an existing context does not automatically forward its optional methods.

## Scope

This API is for applications that directly evaluate formulas with `rpt-formula`. It does not add a
function registry to `rpt-data`, `RenderOptions`, `rpt formulas`, or `rpt-render`. Those paths continue
to use their existing contexts; exposing host functions throughout report rendering is a separate
integration change. Implementations of vendor-specific date conversions require documented behavior
or verified input/output examples and are deliberately outside the engine's built-in function table.

← [Validation](04-validation.md) · [Index](README.md)
