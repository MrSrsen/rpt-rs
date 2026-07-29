//! Parse and evaluate arbitrary strings without panicking.
//!
//! Formula text comes from an arbitrary `.rpt`, and this crate is meant to be embedded in an LSP, a
//! validator, or a WASM sandbox — places where a panic kills the host instead of producing a
//! diagnostic. So the contract is: any input at all yields an AST plus diagnostics, and evaluating it
//! yields a value or an `EvalError`. Never a panic, never a hang.
//!
//! Deterministic by construction — a fixed-seed xorshift generator rather than a property-testing
//! dependency, so a failure names the exact seed and iteration and reproduces byte-for-byte, and the
//! crate keeps its dependency-free test surface.

use rpt_formula::eval::{vm, EmptyContext};
use rpt_formula::{parse, validate_str, Syntax, ValidationContext};

/// xorshift64*, so the corpus is reproducible without a PRNG dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Fragments biased toward things the parser has real structure for, so the generated strings reach
/// past the lexer instead of bouncing off it. Pure random bytes rarely produce a nested `If`.
const FRAGMENTS: &[&str] = &[
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    ";",
    ",",
    ":=",
    "=",
    "<>",
    "<",
    ">",
    "+",
    "-",
    "*",
    "/",
    "^",
    "%",
    "If",
    "Then",
    "Else",
    "Select",
    "Case",
    "For",
    "To",
    "Step",
    "Do",
    "While",
    "Local",
    "Global",
    "Shared",
    "NumberVar",
    "StringVar",
    "DateVar",
    "And",
    "Or",
    "Not",
    "Sum",
    "Average",
    "ToText",
    "Length",
    "Left",
    "Mid",
    "{table.field}",
    "{@formula}",
    "{?Param}",
    "{%sqlexpr}",
    "\"str\"",
    "'other'",
    "#2024-01-03#",
    "1",
    "0",
    "-1",
    "3.14",
    "1e400",
    "True",
    "False",
    "//comment\n",
    "\n",
    " ",
    "\t",
    "\u{202e}",
    "é",
    "日本",
    "\u{0}",
];

fn generate(rng: &mut Rng) -> String {
    let len = 1 + rng.below(24);
    let mut s = String::new();
    for _ in 0..len {
        // Mostly structured fragments, occasionally a raw byte to probe the lexer's edges.
        if rng.below(8) == 0 {
            s.push(char::from(u8::try_from(rng.below(256)).unwrap_or(b'?')));
        } else {
            s.push_str(FRAGMENTS[rng.below(FRAGMENTS.len())]);
        }
    }
    s
}

/// Parse → compile → evaluate. Any panic here fails the test with the reproducing seed.
fn exercise(src: &str, syntax: Syntax) {
    let (ast, _diags) = parse(src, syntax);
    // Compiling and running the *recovery* AST is exactly what the pipeline does with a formula that
    // did not parse, so the fuzz must cover that path rather than only well-formed input.
    let chunk = vm::compile(&ast);
    let _ = vm::run(&chunk, &EmptyContext);
    // The validator walks the same AST and is what an LSP would call on every keystroke.
    let _ = validate_str(src, syntax, &ValidationContext::default());
}

#[test]
fn arbitrary_input_never_panics() {
    const SEED: u64 = 0x5DEE_CE66_D1CE_B00C;
    // Enough to shake out the structural cases in a couple of seconds; the seed is fixed, so raising
    // this locally explores further without changing what CI checks.
    const ITERATIONS: usize = 4_000;

    let mut rng = Rng(SEED);
    for i in 0..ITERATIONS {
        let src = generate(&mut rng);
        for syntax in [Syntax::Crystal, Syntax::Basic] {
            // The panic hook prints the location; this names the input that produced it.
            let probe = std::panic::catch_unwind(|| exercise(&src, syntax));
            assert!(
                probe.is_ok(),
                "panicked on iteration {i} (seed {SEED:#x}, {syntax:?}) with input:\n{src:?}"
            );
        }
    }
}

/// Known awkward inputs, kept as a named corpus so a regression is legible rather than hiding
/// behind a seed.
#[test]
fn known_awkward_inputs_never_panic() {
    let cases = [
        "",
        " ",
        "\0",
        "(",
        ")",
        "((((((((((((((((((((",
        "))))))))))))))))))))",
        "{",
        "{}",
        "{unterminated",
        "\"unterminated",
        "#not-a-date#",
        "1e999999",
        "-1e999999",
        "0/0",
        "1/0",
        "If",
        "If Then Else",
        "Select Case",
        "For To Step Do",
        "x :=",
        ":= x",
        ";;;;;;;;",
        "Local NumberVar x := x;", // self-referential initialiser
        "Sum()",
        "Sum({a}, {b}, {c}, {d})",
        "ToText()",
        "\u{202e}\u{202d}", // bidi overrides
        "日本語 := 1",
        "{table.field}[999999999]",
        "1 + + + + + 1",
    ];
    for src in cases {
        for syntax in [Syntax::Crystal, Syntax::Basic] {
            let probe = std::panic::catch_unwind(|| exercise(src, syntax));
            assert!(probe.is_ok(), "panicked on {src:?} ({syntax:?})");
        }
    }
}

/// Deep nesting must not blow the stack — a recursive-descent parser's other failure mode, which a
/// short random string never reaches.
#[test]
fn deep_nesting_does_not_overflow_the_stack() {
    for depth in [100, 1_000, 10_000] {
        let src = format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
        let probe = std::panic::catch_unwind(|| exercise(&src, Syntax::Crystal));
        assert!(probe.is_ok(), "panicked at nesting depth {depth}");
    }
}
