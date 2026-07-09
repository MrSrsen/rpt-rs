//! Number-to-text builtins: `ToWords` (an amount spelled out, cheque style) and `Roman` (classic
//! Roman numerals).

use super::{bad_arg, num_arg, opt_num, Builtin};
use crate::eval::{EvalError, Value};

/// Handle a numeral [`Builtin`] (routed here by [`super::Builtin::family`]).
pub(super) fn call(b: Builtin, name: &str, args: &[Value]) -> Result<Value, EvalError> {
    match b {
        Builtin::ToWords => {
            let value = num_arg(name, args, 0)?;
            // The optional second argument is the number of decimal places (default 2); 0 suppresses
            // the fractional clause entirely.
            let decimals = opt_num(args, 1).unwrap_or(2.0).max(0.0) as u32;
            Ok(Value::Str(to_words(value, decimals)))
        }
        Builtin::Roman => {
            let n = num_arg(name, args, 0)?.round() as i64;
            // Only the classic form is implemented; the graduated simplified forms (1-4) report
            // Unsupported rather than guess at their per-level rules.
            if let Some(form) = opt_num(args, 1) {
                if form != 0.0 {
                    return Err(EvalError::Unsupported(
                        "Roman simplified forms (1-4)".into(),
                    ));
                }
            }
            roman(name, n).map(Value::Str)
        }
        other => unreachable!("non-numeral builtin {other:?} routed to numeral"),
    }
}

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];
const SCALES: [&str; 7] = [
    "",
    " thousand",
    " million",
    " billion",
    " trillion",
    " quadrillion",
    " quintillion",
];

/// Spell out an amount cheque-style: the integer part in words, then (when `decimals` > 0) the
/// fraction as `and NN / 10ᵈ`. Negative values take a `negative ` prefix. The fraction numerator is
/// zero-padded to `decimals` digits.
fn to_words(value: f64, decimals: u32) -> String {
    let neg = value < 0.0;
    let scale = 10f64.powi(decimals as i32);
    let scaled = (value.abs() * scale).round() as u64;
    let denom = 10u64.pow(decimals);
    let int_part = scaled / denom;
    let frac = scaled % denom;
    let mut s = int_to_words(int_part);
    if decimals > 0 {
        s.push_str(&format!(
            " and {frac:0width$} / {denom}",
            width = decimals as usize
        ));
    }
    if neg {
        format!("negative {s}")
    } else {
        s
    }
}

/// A non-negative integer spelled out (lowercase, hyphenated tens-units, no "and" between groups).
fn int_to_words(mut n: u64) -> String {
    if n == 0 {
        return "zero".to_string();
    }
    let mut groups: Vec<u16> = Vec::new();
    while n > 0 {
        groups.push((n % 1000) as u16);
        n /= 1000;
    }
    let mut parts: Vec<String> = Vec::new();
    for (i, &g) in groups.iter().enumerate().rev() {
        if g == 0 {
            continue;
        }
        let scale = SCALES.get(i).copied().unwrap_or("");
        parts.push(format!("{}{scale}", three_digit_words(g)));
    }
    parts.join(" ")
}

/// Words for 1..=999 (`"one hundred forty-five"`), no leading/trailing spaces.
fn three_digit_words(g: u16) -> String {
    let mut out = String::new();
    let (h, rest) = (g / 100, g % 100);
    if h > 0 {
        out.push_str(ONES[h as usize]);
        out.push_str(" hundred");
    }
    if rest > 0 {
        if h > 0 {
            out.push(' ');
        }
        out.push_str(&two_digit_words(rest));
    }
    out
}

/// Words for 1..=99 (`"forty-five"`).
fn two_digit_words(n: u16) -> String {
    if n < 20 {
        ONES[n as usize].to_string()
    } else {
        let t = TENS[(n / 10) as usize];
        match n % 10 {
            0 => t.to_string(),
            u => format!("{t}-{}", ONES[u as usize]),
        }
    }
}

/// Classic Roman numerals for 1..=3999 (`Roman(1998)` = `MCMXCVIII`); 0 yields the empty string.
fn roman(name: &str, n: i64) -> Result<String, EvalError> {
    if n == 0 {
        return Ok(String::new());
    }
    if !(1..=3999).contains(&n) {
        return Err(bad_arg(name, "number out of range 1..3999"));
    }
    const TABLE: [(i64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    let mut v = n;
    for (val, sym) in TABLE {
        while v >= val {
            out.push_str(sym);
            v -= val;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::eval::{eval, EmptyContext, EvalError, Value};
    use crate::{parse, Syntax};

    fn run(src: &str) -> Result<Value, EvalError> {
        let (ast, diags) = parse(src, Syntax::Crystal);
        assert!(diags.is_empty(), "parse diagnostics for `{src}`: {diags:?}");
        eval(&ast, &EmptyContext)
    }
    fn text(src: &str) -> String {
        match run(src) {
            Ok(Value::Str(s)) => s,
            other => panic!("`{src}` → {other:?}"),
        }
    }

    #[test]
    fn towords_cheque_style() {
        assert_eq!(
            text("ToWords(1145.31)"),
            "one thousand one hundred forty-five and 31 / 100"
        );
        // A zero decimals argument suppresses the fraction clause.
        assert_eq!(text("ToWords(100, 0)"), "one hundred");
        // Default is two decimals; the numerator is zero-padded.
        assert_eq!(text("ToWords(5)"), "five and 00 / 100");
        assert_eq!(
            text("ToWords(1234.05)"),
            "one thousand two hundred thirty-four and 05 / 100"
        );
    }

    #[test]
    fn towords_edges() {
        assert_eq!(text("ToWords(0, 0)"), "zero");
        assert_eq!(text("ToWords(19, 0)"), "nineteen");
        assert_eq!(text("ToWords(20, 0)"), "twenty");
        assert_eq!(text("ToWords(21, 0)"), "twenty-one");
        assert_eq!(text("ToWords(1000000, 0)"), "one million");
        assert_eq!(text("ToWords(2001, 0)"), "two thousand one");
        // Rounding carries into the integer part.
        assert_eq!(text("ToWords(1.999)"), "two and 00 / 100");
        assert_eq!(text("ToWords(-5, 0)"), "negative five");
    }

    #[test]
    fn roman_classic() {
        assert_eq!(text("Roman(1998)"), "MCMXCVIII");
        assert_eq!(text("Roman(4)"), "IV");
        assert_eq!(text("Roman(2024)"), "MMXXIV");
        assert_eq!(text("Roman(3999)"), "MMMCMXCIX");
        assert_eq!(text("Roman(0)"), "");
        // An explicit classic form (0) is accepted.
        assert_eq!(text("Roman(1998, 0)"), "MCMXCVIII");
    }

    #[test]
    fn roman_errors_and_deferrals() {
        assert!(matches!(run("Roman(4000)"), Err(EvalError::BadArg(_))));
        assert!(matches!(run("Roman(-1)"), Err(EvalError::BadArg(_))));
        // Simplified forms are recognised but deferred.
        assert!(matches!(
            run("Roman(1998, 2)"),
            Err(EvalError::Unsupported(_))
        ));
    }
}
