//! String builtins.

use super::{bad_arg, mismatch, num_arg, opt_num, str_arg, Builtin};
use crate::eval::{EvalError, Value};

/// Handle a string [`Builtin`] (routed here by [`super::Builtin::family`]).
pub(super) fn call(b: Builtin, name: &str, args: &[Value]) -> Result<Value, EvalError> {
    use Builtin as B;
    match b {
        B::Length => Ok(Value::Number(str_arg(name, args, 0)?.chars().count() as f64)),
        B::UpperCase => Ok(Value::Str(str_arg(name, args, 0)?.to_uppercase())),
        B::LowerCase => Ok(Value::Str(str_arg(name, args, 0)?.to_lowercase())),
        B::ProperCase => Ok(Value::Str(proper_case(str_arg(name, args, 0)?))),
        B::Trim => Ok(Value::Str(str_arg(name, args, 0)?.trim().to_string())),
        B::TrimLeft => Ok(Value::Str(str_arg(name, args, 0)?.trim_start().to_string())),
        B::TrimRight => Ok(Value::Str(str_arg(name, args, 0)?.trim_end().to_string())),
        B::Left => {
            let s = str_arg(name, args, 0)?;
            let n = num_arg(name, args, 1)?.max(0.0) as usize;
            Ok(Value::Str(s.chars().take(n).collect()))
        }
        B::Right => {
            let s = str_arg(name, args, 0)?;
            let n = num_arg(name, args, 1)?.max(0.0) as usize;
            let chars: Vec<char> = s.chars().collect();
            Ok(Value::Str(
                chars[chars.len().saturating_sub(n)..].iter().collect(),
            ))
        }
        B::Mid => {
            let s = str_arg(name, args, 0)?;
            let start = (num_arg(name, args, 1)?.max(1.0) as usize) - 1; // 1-based
            let it = s.chars().skip(start);
            Ok(Value::Str(match args.get(2) {
                Some(v) => it
                    .take(v.as_number().map(|n| n.max(0.0) as usize).unwrap_or(0))
                    .collect(),
                None => it.collect(),
            }))
        }
        B::InStr => {
            // InStr(hay, needle) or InStr(start, hay, needle) — 1-based, 0 = absent.
            let (start, hay, needle) = if args.first().and_then(Value::as_number).is_some() {
                (
                    num_arg(name, args, 0)?.max(1.0) as usize - 1,
                    str_arg(name, args, 1)?,
                    str_arg(name, args, 2)?,
                )
            } else {
                (0, str_arg(name, args, 0)?, str_arg(name, args, 1)?)
            };
            let hay_chars: Vec<char> = hay.chars().collect();
            let hay_tail: String = hay_chars.iter().skip(start).collect();
            Ok(Value::Number(match hay_tail.find(needle.as_str()) {
                // Byte offset → char offset for the 1-based result.
                Some(b) => (hay_tail[..b].chars().count() + start + 1) as f64,
                None => 0.0,
            }))
        }
        B::InStrRev => {
            // Last occurrence of `needle` in `hay`, 1-based (0 = absent). Optional `start`
            // (1-based) caps the search to `hay[..start]`.
            let hay = str_arg(name, args, 0)?;
            let needle = str_arg(name, args, 1)?;
            let hay_chars: Vec<char> = hay.chars().collect();
            let end = match opt_num(args, 2) {
                Some(n) => (n.max(0.0) as usize).min(hay_chars.len()),
                None => hay_chars.len(),
            };
            let window: String = hay_chars[..end].iter().collect();
            Ok(Value::Number(match window.rfind(needle.as_str()) {
                Some(b) => (window[..b].chars().count() + 1) as f64,
                None => 0.0,
            }))
        }
        B::Replace => {
            let s = str_arg(name, args, 0)?;
            let find = str_arg(name, args, 1)?;
            let repl = str_arg(name, args, 2)?;
            Ok(Value::Str(s.replace(find.as_str(), &repl)))
        }
        B::ReplicateString => {
            let s = str_arg(name, args, 0)?;
            let n = num_arg(name, args, 1)?.max(0.0) as usize;
            Ok(Value::Str(s.repeat(n)))
        }
        B::Space => Ok(Value::Str(
            " ".repeat(num_arg(name, args, 0)?.max(0.0) as usize),
        )),
        B::StrReverse => Ok(Value::Str(str_arg(name, args, 0)?.chars().rev().collect())),
        B::Split => {
            let s = str_arg(name, args, 0)?;
            // VB `Split`: default delimiter is a space; an empty delimiter yields the whole string.
            let delim = match args.get(1) {
                Some(Value::Str(d)) => d.clone(),
                None => " ".to_string(),
                Some(v) => return Err(mismatch(name, v)),
            };
            let parts: Vec<Value> = if delim.is_empty() {
                vec![Value::Str(s)]
            } else {
                s.split(delim.as_str())
                    .map(|p| Value::Str(p.to_string()))
                    .collect()
            };
            Ok(Value::Array(parts))
        }
        B::Join => {
            let items = match &args[0] {
                Value::Array(a) => a,
                v => return Err(mismatch(name, v)),
            };
            // VB `Join`: default delimiter is a space.
            let delim = match args.get(1) {
                Some(Value::Str(d)) => d.clone(),
                None => " ".to_string(),
                Some(v) => return Err(mismatch(name, v)),
            };
            let parts: Result<Vec<String>, EvalError> = items
                .iter()
                .map(|v| match v {
                    Value::Str(s) => Ok(s.clone()),
                    v => v.to_text_default().ok_or_else(|| mismatch(name, v)),
                })
                .collect();
            match parts {
                Ok(parts) => Ok(Value::Str(parts.join(&delim))),
                Err(e) => Err(e),
            }
        }
        B::Filter => {
            // Elements of a string array that contain (or, with include=false, omit) `match`.
            let items = match &args[0] {
                Value::Array(a) => a,
                v => return Err(mismatch(name, v)),
            };
            let needle = str_arg(name, args, 1)?;
            let include = match args.get(2) {
                Some(Value::Bool(b)) => *b,
                Some(v) => match v.as_number() {
                    Some(n) => n != 0.0,
                    None => return Err(mismatch(name, v)),
                },
                None => true,
            };
            let mut out = Vec::new();
            for v in items {
                let s = match v {
                    Value::Str(s) => s.clone(),
                    v => return Err(mismatch(name, v)),
                };
                if s.contains(needle.as_str()) == include {
                    out.push(Value::Str(s));
                }
            }
            Ok(Value::Array(out))
        }
        B::StrCmp => {
            // -1 / 0 / 1 for a < b / a == b / a > b. An optional truthy third arg folds case.
            let a = str_arg(name, args, 0)?;
            let b = str_arg(name, args, 1)?;
            let fold = matches!(args.get(2), Some(Value::Bool(true)))
                || opt_num(args, 2).is_some_and(|n| n != 0.0);
            let ord = if fold {
                a.to_lowercase().cmp(&b.to_lowercase())
            } else {
                a.cmp(&b)
            };
            Ok(Value::Number(match ord {
                std::cmp::Ordering::Less => -1.0,
                std::cmp::Ordering::Equal => 0.0,
                std::cmp::Ordering::Greater => 1.0,
            }))
        }
        B::Chr => {
            let n = num_arg(name, args, 0)? as u32;
            match char::from_u32(n) {
                Some(c) => Ok(Value::Str(c.to_string())),
                None => Err(bad_arg(name, "invalid code point")),
            }
        }
        B::Asc => match str_arg(name, args, 0)?.chars().next() {
            Some(c) => Ok(Value::Number(c as u32 as f64)),
            None => Err(bad_arg(name, "empty string")),
        },
        other => unreachable!("non-string builtin {other:?} routed to string"),
    }
}

fn proper_case(s: String) -> String {
    let mut out = String::with_capacity(s.len());
    let mut at_word_start = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if at_word_start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            at_word_start = false;
        } else {
            out.push(c);
            at_word_start = true;
        }
    }
    out
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
    fn num(src: &str) -> f64 {
        match run(src) {
            Ok(Value::Number(n)) => n,
            other => panic!("`{src}` → {other:?}"),
        }
    }
    fn text(src: &str) -> String {
        match run(src) {
            Ok(Value::Str(s)) => s,
            other => panic!("`{src}` → {other:?}"),
        }
    }

    #[test]
    fn case_and_trim() {
        assert_eq!(text(r#"UpperCase("aBc")"#), "ABC");
        assert_eq!(text(r#"LowerCase("aBc")"#), "abc");
        assert_eq!(
            text(r#"ProperCase("john SMITH-jones")"#),
            "John Smith-Jones"
        );
        assert_eq!(text(r#"Trim("  x  ")"#), "x");
        assert_eq!(text(r#"TrimLeft("  x  ")"#), "x  ");
        assert_eq!(text(r#"TrimRight("  x  ")"#), "  x");
        assert_eq!(num(r#"Length("abcd")"#), 4.0);
        assert_eq!(num(r#"Length("")"#), 0.0);
    }

    #[test]
    fn slicing() {
        assert_eq!(text(r#"Left("crystal", 3)"#), "cry");
        assert_eq!(text(r#"Left("ab", 9)"#), "ab"); // over-length is clamped
        assert_eq!(text(r#"Left("ab", 0)"#), "");
        assert_eq!(text(r#"Right("crystal", 3)"#), "tal");
        assert_eq!(text(r#"Right("ab", 9)"#), "ab");
        assert_eq!(text(r#"Mid("crystal", 2, 3)"#), "rys");
        assert_eq!(text(r#"Mid("crystal", 4)"#), "stal"); // to end
    }

    #[test]
    fn search() {
        assert_eq!(num(r#"InStr("hello", "ll")"#), 3.0);
        assert_eq!(num(r#"InStr("hello", "z")"#), 0.0);
        assert_eq!(num(r#"InStr(3, "hello hello", "he")"#), 7.0);
        assert_eq!(num(r#"InStrRev("hello hello", "he")"#), 7.0);
        assert_eq!(num(r#"InStrRev("hello", "x")"#), 0.0);
        assert_eq!(num(r#"InStrRev("abcabc", "bc", 4)"#), 2.0);
    }

    #[test]
    fn build_and_reverse() {
        assert_eq!(text(r#"Replace("a-b-c", "-", "+")"#), "a+b+c");
        assert_eq!(text(r#"ReplicateString("ab", 3)"#), "ababab");
        assert_eq!(text(r#"ReplicateString("ab", 0)"#), "");
        assert_eq!(text("Space(3)"), "   ");
        assert_eq!(text(r#"StrReverse("abc")"#), "cba");
        assert_eq!(text("Chr(65)"), "A");
        assert_eq!(num(r#"Asc("A")"#), 65.0);
    }

    #[test]
    fn split_join_filter() {
        assert_eq!(text(r#"Split("a,b,c", ",")[2]"#), "b");
        assert_eq!(num(r#"UBound(Split("a,b,c", ","))"#), 3.0);
        assert_eq!(text(r#"Split("abc", "")[1]"#), "abc"); // empty delim → whole string
        assert_eq!(text(r#"Join(["a", "b", "c"], "-")"#), "a-b-c");
        assert_eq!(
            text(r#"Filter(["apple", "banana", "cherry"], "a")[2]"#),
            "banana"
        );
        assert_eq!(
            num(r#"UBound(Filter(["apple", "banana", "cherry"], "a"))"#),
            2.0
        );
    }

    #[test]
    fn strcmp_cases() {
        assert_eq!(num(r#"StrCmp("a", "b")"#), -1.0);
        assert_eq!(num(r#"StrCmp("b", "b")"#), 0.0);
        assert_eq!(num(r#"StrCmp("c", "b")"#), 1.0);
        assert_eq!(num(r#"StrCmp("A", "a")"#), -1.0); // case-sensitive by default
        assert_eq!(num(r#"StrCmp("A", "a", true)"#), 0.0);
    }

    #[test]
    fn error_cases() {
        assert!(matches!(
            run(r#"Length(5)"#),
            Err(EvalError::TypeMismatch { .. })
        ));
        assert!(matches!(run(r#"Left("ab")"#), Err(EvalError::BadArg(_))));
        assert!(matches!(run(r#"Asc("")"#), Err(EvalError::BadArg(_))));
        assert!(matches!(run("Chr(1114112)"), Err(EvalError::BadArg(_)))); // above U+10FFFF
        assert!(matches!(
            run(r#"Join("notarray", ",")"#),
            Err(EvalError::TypeMismatch { .. })
        ));
    }
}
