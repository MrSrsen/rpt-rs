# Language reference

Crystal formulas come in two author-facing syntaxes — **Crystal syntax** (the primary) and **Basic syntax** — over one
core. The expression grammar and operator precedence are *identical*; only statement/declaration syntax and a few lexer
rules differ. Both lower to the same AST, type system, and evaluator.

## Lexical structure

| Aspect | Crystal syntax | Basic syntax |
| ------ | -------------- | ------------ |
| Statement separator | `;` | newline |
| Assignment | `:=` | `=` |
| String delimiters | `"` or `'` (doubled-quote escaping) | `"` only (`'` starts a comment) |
| Comments | `//` to end of line | `//`, `'`, or `Rem` to end of line |
| Return value | the last expression | the value assigned to the implicit `formula` variable |

Shared lexis: identifiers are `[A-Za-z_][A-Za-z0-9_]*`; numbers are `digits[.digits]`; `{...}` is a single reference
token read to the first `}`; `#...#` is a date/time literal; word operators (`And`, `Or`, `Mod`, `Not`, `To`, `In`,
`Like`, `StartsWith`, …) lex as identifiers and are recognised by the parser. The tokenizer is error-tolerant and never
panics.

## Operators and precedence

Lowest to highest binding (each level is a rung of the recursive-descent ladder). `=`/`<>` bind *looser* than the
relational operators, and neither chains (both are non-associative); `&` binds tighter than comparison but looser than
`+`/`-`; unary `-`/`Not` bind looser than `^` (so `-2^2` = `-(2^2)`).

| Level | Operators | Assoc. |
| ----- | --------- | ------ |
| 1–5 | `Imp`, `Eqv`, `Xor`, `Or`, `And` | left |
| 6 | `=` `<>` `In` `Like` `StartsWith` | non-assoc |
| 7 | `<` `>` `>=` `<=` | non-assoc |
| 8 | `&` (concat) | left |
| 9 | `To` `_To` `To_` `_To_` (range) | non-assoc |
| 10 | `+` `-` | left |
| 11 | `Mod` | left |
| 12 | `\` (integer division) | left |
| 13 | `*` `/` `%` | left |
| 14 | prefix `-` `+` `$` `Not` | right |
| 15 | `^` (power) | left |
| 16 | postfix `expr[index]` (1-based subscript) | left |
| 17 | primary atoms | — |

`$` is the currency prefix (`$2` is `Currency(2)`); binary `%` is "percent of" (`x % y` = `100*x/y`); `In` tests
substring / array membership / range containment; `Like` is VB wildcard matching (`*` any run, `?` any one char).

## Expressions

Primary atoms: number/string/boolean (`True`/`Yes`, `False`/`No`) literals, `#...#` date literals, `{...}` references,
`( expr )` grouping, `[ e, … ]` array literals, function calls `name(args)`, bare identifiers (variables / 0-ary
builtins), and — as *expressions* in Crystal syntax — `If` and `Select`:

```
If {orders.amount} > 1000 Then "large" Else If {orders.amount} > 100 Then "medium" Else "small"
```

`If` without `Else` yields the then-branch type's default (`0`, `""`, `False`, …). `IIf`, `Switch`, and `Choose` are
lazy: only the selected branch evaluates.

## Statements and declarations

A formula body is a statement sequence. Most statements are expressions/assignments; the control-flow forms below are
statements (in Basic) or, for `If`/`Select`, expressions (in Crystal). The parser models these as real AST nodes —
`Select … Case` and Basic `If … End If` are lowered to `If` nodes, `Dim` to a declaration node, and the loops to `While`
/`For` nodes.

### Variable declarations

- **Crystal:** `[Local|Global|Shared] <Type>Var [Array] name[, name…] [:= init]`, where `<Type>` is one of
  `Number`/`Currency`/`Boolean`/`Date`/`Time`/`DateTime`/`String`. No keyword → `Global`.
- **Basic:** `Dim name[(dims)][, name…] [As Type]`. `Dim` variables are `Local`; an omitted `As` type defaults to
  Number. (Array dimension bounds are accepted and skipped.)

An uninitialised variable takes its type's default (`0`, `Currency(0)`, `False`, `""`; Date/Time start null). A
re-declaration brings the name into scope without resetting an existing value.

### Assignment and the Basic result

`name := expr` (Crystal) / `name = expr` (Basic). In Basic, the formula's return value is whatever is assigned to the
implicit `formula` variable — the parser appends a read of `formula` when the body assigns it.

### If … Then … End If (Basic block form)

```
If cond Then
    statements
ElseIf cond2 Then
    statements
Else
    statements
End If
```

A newline after `Then` selects the block form; otherwise it is a single-line `If cond Then stmt [Else stmt]`. Both lower
to an `If` node. (`Else If` as two words is accepted alongside `ElseIf`.)

### For … Next / For … Do

```
For i = 1 To 10 Step 2     ' Basic          |  For i := 1 To 10 Step 2 Do   ' Crystal
    statements             '                |      statement
Next i                     '                |
```

The loop variable counts from the start to the limit inclusive; the direction follows the sign of `Step` (default `1`),
so a negative step counts down. The limit and step are evaluated once. In Basic the body is a statement sequence closed
by `Next [var]`; in Crystal it is the single statement after `Do`.

### While … Wend / While … Do

```
While cond            ' Basic     |  While cond Do statement   ' Crystal
    statements        '           |
Wend                  '           |
```

A pre-test loop.

### Do … Loop (Basic)

```
Do [While|Until cond]
    statements
Loop [While|Until cond]
```

The condition may lead (pre-test) or trail (post-test); `Until` is the negation of `While`. A `Do … Loop` with no
condition is flagged as a diagnostic (it would be an infinite loop).

### Select … Case

Crystal (an expression) and Basic (a `Select Case … End Select` statement):

```
Select Case {customer.region}          ' Basic
    Case "North", "South"
        statements
    Case Is > "M"
        statements
    Case "A" To "F"
        statements
    Case Else
        statements
End Select
```

Each case test lowers to a boolean condition on the subject: a bare value → `subject = value`; `Is <rel> v` →
`subject <rel> v`; `lo To hi` → `subject In (lo To hi)`; comma-separated tests are OR-ed. The whole construct lowers to
an `If`/`ElseIf` chain (`Case Else` in Basic / `Default` in Crystal becomes the `Else`).

### Exit (loop break)

`Exit For` / `Exit While` / `Exit Do` break out of the innermost enclosing loop; the loop keyword is kept for AST
fidelity but the break is independent of kind. Both evaluators (tree-walker and bytecode VM) implement it identically. An
`Exit` with no enclosing loop is a clean evaluation error (`BadArg`), not a panic.

## Literals

- **Numbers:** `123`, `1.5`, `.5`. A `$` prefix makes a currency value.
- **Strings:** `"…"` / `'…'` (Crystal) or `"…"` (Basic), with doubled-quote escaping.
- **Booleans:** `True`/`Yes`, `False`/`No`.
- **Date/time (`#…#`):** the internals are parsed to an actual `Date`/`Time`/`DateTime` value. Accepted forms:
  `#m/d/yyyy#`, `#yyyy-m-d#`, an optional `hh:mm[:ss] [AM|PM]` tail, or a bare time. Examples:

  ```
  #12/31/1999#            → Date(1999, 12, 31)
  #2004-02-29 23:59:59#   → DateTime(2004-02-29, 23:59:59)
  #10:30:00 pm#           → Time(22:30:00)
  ```

  A malformed literal is reported as a `Diagnostic` (the node still carries its raw text, keeping the tree total).
- **Arrays:** `[1, 2, 3]`; subscripting is 1-based. **Ranges:** `1 To 10`, with `_To`/`To_`/`_To_` excluding a bound.

## A grammar sketch (EBNF-ish)

```ebnf
formula     = stmtSeq ;
stmtSeq     = statement { (";" | newline) statement } ;
statement   = forLoop | whileLoop | doLoop            (* Basic: also ifBlock | selectCase | dimDecl | exit *)
            | varDecl | assignment | expr ;
forLoop     = "For" ident assign expr "To" expr [ "Step" expr ]
              ( "Do" statement | newline stmtSeq "Next" [ ident ] ) ;
whileLoop   = "While" expr ( "Do" statement | newline stmtSeq "Wend" ) ;
doLoop      = "Do" [ ("While"|"Until") expr ] stmtSeq "Loop" [ ("While"|"Until") expr ] ;   (* Basic *)
ifBlock     = "If" expr "Then" newline stmtSeq
              { "ElseIf" expr "Then" newline stmtSeq } [ "Else" newline stmtSeq ] "End" "If" ;
selectCase  = "Select" [ "Case" ] expr { caseClause } [ "Case" "Else" body | "Default" body ] [ "End" "Select" ] ;
caseClause  = "Case" caseTest { "," caseTest } [ ":" ] body ;
caseTest    = "Is" relop expr | expr [ "To" expr ] ;
varDecl     = [ "Local"|"Global"|"Shared" ] typeVar [ "Array" ] ident { "," ident } [ assign expr ]   (* Crystal *)
            | ("Dim"|"ReDim") ident [ "(" … ")" ] { "," ident } [ "As" typeName ] ;                    (* Basic *)
assignment  = ident assign expr ;                 (* assign = ":=" (Crystal) | "=" (Basic) *)
expr        = <precedence ladder, levels 1..17> ;
primary     = number | string | boolean | dateLit | reference | "(" expr ")"
            | "[" [ expr { "," expr } ] "]" | call | ident | ifExpr | selectExpr ;   (* ifExpr/selectExpr: Crystal *)
```
