//! Host calls must agree between the VM and reference evaluator, including failure behavior.

use std::cell::RefCell;

use crate::eval::{eval, eval_spanned, vm, EmptyContext, EvalContext, EvalError, Evaluator, Value};
use crate::token::RefKind;
use crate::{parse, validate_str, Syntax, ValidationContext};

#[derive(Default)]
struct Host {
    calls: RefCell<Vec<(String, Vec<Value>)>>,
}

impl EvalContext for Host {
    fn resolve(&self, _kind: RefKind, name: &str) -> Option<Value> {
        match name.to_ascii_lowercase().as_str() {
            "source.value" => Some(Value::Number(21.0)),
            "source.empty" => Some(Value::Null),
            _ => None,
        }
    }

    fn call_function(&self, name: &str, args: &[Value]) -> Option<Result<Value, EvalError>> {
        self.calls.borrow_mut().push((name.into(), args.to_vec()));
        match name {
            "hostdouble" => Some(match args {
                [Value::Number(n)] => Ok(Value::Number(n * 2.0)),
                [value] => Err(EvalError::TypeMismatch {
                    what: name.into(),
                    got: value.type_name().into(),
                }),
                _ => Err(EvalError::Arity {
                    name: name.into(),
                    expected: "1 argument".into(),
                    got: args.len(),
                }),
            }),
            "hostecho" => Some(Ok(args.first().cloned().unwrap_or(Value::Null))),
            "hostfail" => Some(Err(EvalError::BadArg("host rejected input".into()))),
            "hostpending" => Some(Err(EvalError::Unsupported(
                "host implementation pending".into(),
            ))),
            "hostzero" => Some(Ok(Value::Number(7.0))),
            _ => None,
        }
    }
}

fn both(source: &str, expected: Result<Value, EvalError>) -> Vec<(String, Vec<Value>)> {
    let (ast, diagnostics) = parse(source, Syntax::Crystal);
    assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
    let vm_host = Host::default();
    let tree_host = Host::default();
    assert_eq!(eval(&ast, &vm_host), expected);
    assert_eq!(Evaluator::new(&tree_host).eval(&ast), expected);
    assert_eq!(*vm_host.calls.borrow(), *tree_host.calls.borrow());
    vm_host.calls.into_inner()
}

#[test]
fn nested_mixed_case_calls_receive_evaluated_arguments() {
    let calls = both(
        "HostDouble(HostDouble({source.value}) + 1)",
        Ok(Value::Number(86.0)),
    );
    assert_eq!(
        calls,
        vec![
            ("hostdouble".into(), vec![Value::Number(21.0)]),
            ("hostdouble".into(), vec![Value::Number(43.0)]),
        ]
    );
}

#[test]
fn arguments_are_eager_and_left_to_right() {
    let calls = both(
        "HostEcho(HostDouble(1), HostDouble(2))",
        Ok(Value::Number(2.0)),
    );
    assert_eq!(
        calls,
        vec![
            ("hostdouble".into(), vec![Value::Number(1.0)]),
            ("hostdouble".into(), vec![Value::Number(2.0)]),
            (
                "hostecho".into(),
                vec![Value::Number(2.0), Value::Number(4.0)]
            ),
        ]
    );
    assert!(both("HostEcho(1 / 0)", Err(EvalError::DivideByZero)).is_empty());
}

#[test]
fn host_owns_null_and_argument_validation() {
    assert_eq!(
        both("HostEcho({source.empty})", Ok(Value::Null)),
        vec![("hostecho".into(), vec![Value::Null])]
    );
    both(
        "HostDouble()",
        Err(EvalError::Arity {
            name: "hostdouble".into(),
            expected: "1 argument".into(),
            got: 0,
        }),
    );
    both(
        "HostDouble(1, 2)",
        Err(EvalError::Arity {
            name: "hostdouble".into(),
            expected: "1 argument".into(),
            got: 2,
        }),
    );
    both(
        "HostDouble(\"text\")",
        Err(EvalError::TypeMismatch {
            what: "hostdouble".into(),
            got: Value::Str(String::new()).type_name().into(),
        }),
    );
}

#[test]
fn host_errors_are_preserved_and_spanned_at_call() {
    both(
        "HostFail()",
        Err(EvalError::BadArg("host rejected input".into())),
    );
    both(
        "HostPending()",
        Err(EvalError::Unsupported("host implementation pending".into())),
    );
    let source = "1 + HostFail()";
    let (ast, diagnostics) = parse(source, Syntax::Crystal);
    assert!(diagnostics.is_empty());
    let error = eval_spanned(&ast, &Host::default()).unwrap_err();
    assert_eq!(error.error, EvalError::BadArg("host rejected input".into()));
    assert_eq!(&source[error.span.start..error.span.end], "HostFail()");
}

#[test]
fn unknown_functions_and_existing_contexts_still_error() {
    let (ast, diagnostics) = parse("missing(1)", Syntax::Crystal);
    assert!(diagnostics.is_empty());
    assert_eq!(
        eval(&ast, &EmptyContext),
        Err(EvalError::UnknownName("missing".into()))
    );
    both("missing(1)", Err(EvalError::UnknownName("missing".into())));
}

#[test]
fn bare_identifiers_do_not_call_host_but_explicit_zero_arity_does() {
    assert!(both("hostzero", Err(EvalError::UnknownName("hostzero".into()))).is_empty());
    assert_eq!(
        both("HostZero()", Ok(Value::Number(7.0))),
        vec![("hostzero".into(), vec![])]
    );
}

#[test]
fn builtins_and_lazy_branches_do_not_reach_host() {
    assert!(both("Abs(-2)", Ok(Value::Number(2.0))).is_empty());
    assert!(both(
        "Abs()",
        Err(EvalError::Arity {
            name: "abs".into(),
            expected: "1 argument".into(),
            got: 0,
        })
    )
    .is_empty());
    assert!(both("If True Then 1 Else HostFail()", Ok(Value::Number(1.0))).is_empty());
    // A name in the type table but not implemented by the engine remains reserved.
    assert!(crate::is_builtin("accrint"));
    assert!(both("accrint()", Err(EvalError::Unsupported("accrint".into()))).is_empty());
    // Dispatch also knows names absent from the type table. They must remain reserved.
    assert!(both("crcyan()", Ok(Value::Number(16776960.0))).is_empty());
    assert!(both(
        "RowCounter()",
        Err(EvalError::Unsupported(
            "`rowcounter` needs print/record context".into()
        ))
    )
    .is_empty());
}

#[test]
fn basic_syntax_and_reused_bytecode_support_host_calls() {
    let source = "Formula = HostDouble(21)";
    let (ast, diagnostics) = parse(source, Syntax::Basic);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let chunk = vm::compile(&ast);
    let host = Host::default();
    assert_eq!(vm::run(&chunk, &host), Ok(Value::Number(42.0)));
    assert_eq!(
        Evaluator::new(&Host::default()).eval(&ast),
        Ok(Value::Number(42.0))
    );
    // The chunk captures no host: the same program without the extension still fails.
    assert_eq!(
        vm::run(&chunk, &EmptyContext),
        Err(EvalError::UnknownName("hostdouble".into()))
    );
}

#[test]
fn existing_validation_declarations_accept_host_names() {
    let ctx = ValidationContext::default().with_functions(["HostDouble"]);
    assert!(validate_str("HOSTDOUBLE(21)", Syntax::Crystal, &ctx).is_empty());
    assert!(!validate_str("HostMissing(21)", Syntax::Crystal, &ctx).is_empty());
}
