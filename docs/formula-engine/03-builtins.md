# Builtin functions

The builtin library lives in `eval/builtins/`, split by family. `mod.rs` holds the `Builtin` enum, the single
lowercase-name→variant table (`NAMES`, where aliases live), the null-propagation rule, and a router that dispatches a
resolved variant to its family module; each family module (`string`, `math`, `datetime`, `conversion`, `financial`,
`statistical`, `numeral`) owns its arm implementations and co-located tests.

Names are case-insensitive. Unless noted, a `Null` argument makes the whole call return `Null`. Function *names* Crystal
knows but this engine does not implement (the cross-tab, hierarchy, grid, and context/record-set aggregate functions,
plus the handful of deferred scalars below) are still recognised: they fail with `EvalError::Unsupported`
(known-but-unimplemented), *not* `UnknownName`. Truly unknown names are `UnknownName`.

**Deferred, with reasons** (recognised → `Unsupported`, deliberately not implemented):

- **Context/record-set aggregates** — `Sum`/`Count`/`Average`/`StdDev`/`Median`/`Mode`/`PercentOfSum`/… over a *field*
  and group scope, plus all cross-tab / hierarchy / grid functions. These need the data pipeline's record set and group
  tree; the array-literal overloads (e.g. `Sum([1,2,3])`, `StdDev([…])`) *are* implemented.
- **`Median` / `Mode`** — Crystal has **no array-literal form** for these (unlike `StdDev`/`Variance`); they exist only
  as summary functions over records, so they are deferred with the aggregates above rather than faked over an array.
- **`Rnd`** — Crystal's VBA-derived seeded PRNG needs persistent mutable seed state across calls (an evaluation-context
  concern, not a pure function), and the native negative-seed reseed rule is unconfirmed. Deferred until the eval
  context owns a PRNG seed.
- **`Timer`** — reads the system clock (seconds since midnight); an environmental, non-deterministic runtime value like
  the print-state specials.
- **`Roman(n, form)` simplified forms (1–4)** — only the classic form (0 / omitted) is implemented; the graduated
  simplified forms' exact per-level rules are unconfirmed.

Control forms `IIf`, `Switch`, and `Choose` are handled in the evaluator itself (they are lazy — only the selected branch
runs), not in the builtins module.

## String (`string.rs`)

| Function (aliases) | Signature → return | Semantics |
| ------------------ | ------------------ | --------- |
| `Length` / `Len` | `(String) → Number` | Character count. |
| `UpperCase` / `UCase` | `(String) → String` | |
| `LowerCase` / `LCase` | `(String) → String` | |
| `ProperCase` | `(String) → String` | Title-case each word. |
| `Trim` / `TrimLeft`(`LTrim`) / `TrimRight`(`RTrim`) | `(String) → String` | Strip whitespace. |
| `Left` / `Right` | `(String, Number) → String` | First / last *n* characters (clamped). |
| `Mid` | `(String, start[, len]) → String` | 1-based substring; to end if `len` omitted. |
| `InStr` | `([start,] hay, needle) → Number` | 1-based index of first match, `0` if absent. |
| `InStrRev` | `(hay, needle[, start]) → Number` | 1-based index of *last* match; `start` caps the search window. |
| `Replace` | `(s, find, repl) → String` | Replace all occurrences. |
| `ReplicateString` | `(String, Number) → String` | Repeat *n* times. |
| `Space` | `(Number) → String` | *n* spaces. |
| `StrReverse` | `(String) → String` | |
| `Split` | `(String[, delim]) → String array` | Default delimiter is a space; an empty delimiter yields the whole string. |
| `Join` | `(array[, delim]) → String` | Default delimiter is a space. |
| `Filter` | `(array, match[, include]) → String array` | Elements containing (or, `include=false`, omitting) `match`. |
| `StrCmp` | `(a, b[, ignoreCase]) → Number` | `-1`/`0`/`1`; a truthy third argument folds case. |
| `Chr` / `ChrW` | `(Number) → String` | Unicode code point → character. |
| `Asc` / `AscW` | `(String) → Number` | First character → code point. |

## Math (`math.rs`)

| Function | Signature → return | Semantics |
| -------- | ------------------ | --------- |
| `Abs` | `(num) → num` | Absolute value (preserves Currency). |
| `Sgn` | `(num) → Number` | `-1`/`0`/`1`. |
| `Int` | `(num) → num` | Floor (toward −∞). |
| `Fix` | `(num) → num` | Truncate toward zero. |
| `Truncate` | `(num[, places]) → num` | Truncate to `places` decimals. |
| `Round` | `(num[, places]) → num` | Round half away from zero. |
| `RoundUp` | `(num[, places]) → num` | Round away from zero. |
| `MRound` | `(num, multiple) → num` | Nearest multiple (zero multiple → 0). |
| `Floor` / `Ceiling` | `(num[, multiple]) → num` | Down / up to a multiple (default 1; zero multiple → divide-by-zero). |
| `Remainder` | `(a, b) → Number` | `a % b` (divide-by-zero errors). |
| `Sqr` | `(Number) → Number` | Square **root** (Crystal's `Sqr`). |
| `Exp` / `Log` | `(Number) → Number` | `eˣ` / natural log. |
| `Sin` / `Cos` / `Tan` / `Atn` | `(Number) → Number` | Radians. |
| `crPi` | `→ Number` | π (a 0-ary constant). |
| `Sum` / `Average` / `Minimum` / `Maximum` / `Count` | `(array) → …` | Aggregate over an **array literal**. (The record-set forms, e.g. `Sum({field}, {group})`, need the data pipeline and report `Unsupported`.) |
| `UBound` | `(array) → Number` | Element count. |

The rounding family preserves the input's Number/Currency kind.

## Date & time (`datetime.rs`)

Built on `rpt_format_value::civil` (`Date`/`Time` day-number and second arithmetic — no external date dependency). Crystal
follows VBA semantics; the notable rules are called out below.

| Function (aliases) | Signature → return | Semantics |
| ------------------ | ------------------ | --------- |
| `Date` | `(y,m,d)` / `(serial)` / `(datetime\|string) → Date` | A numeric argument is an **OLE serial** (see below). |
| `Time` | `(h,m,s)` / `(datetime\|string) → Time` | |
| `DateTime` (`CDateTime`, `DateTimeValue`) | `(date[,time])` / `(y,m,d[,h,m,s])` / `(serial)` / `(string) → DateTime` | Fractional serial = date + time. |
| `DateValue` / `CDate` | `(date\|datetime\|string\|serial\|y,m,d) → Date` | |
| `TimeValue` / `CTime` | `(time\|datetime\|string\|h,m,s) → Time` | |
| `DateSerial` | `(y,m,d) → Date` | **Rolls over** out-of-range months/days (no clamping): `DateSerial(2004,13,1)` = 2005-01-01. |
| `TimeSerial` | `(h,m,s) → Time` | Wraps modulo one day. |
| `Year`/`Month`/`Day`/`Hour`/`Minute`/`Second` | `(temporal) → Number` | Components. |
| `DayOfWeek` / `Weekday` | `(date[, firstDayOfWeek]) → Number` | 1..7 relative to `firstDayOfWeek` (default Sunday). |
| `MonthName` | `(1..12[, abbrev]) → String` | |
| `WeekdayName` | `(1..7[, abbrev[, firstDayOfWeek]]) → String` | `n` is **relative to `firstDayOfWeek`**, not absolute Sunday. |
| `DatePart` | `(interval, date[, firstDayOfWeek[, firstWeekOfYear]]) → Number` | Component named by the interval code. |
| `DateAdd` | `(interval, n, date) → DateTime` | Adds `n` intervals; month/quarter/year **clamp** to end-of-month. |
| `DateDiff` | `(interval, d1, d2[, firstDayOfWeek]) → Number` | Interval count between two dates. |
| `IsDate` / `IsTime` / `IsDateTime` | `(value) → Boolean` | Type / parseable-string test. |

**Crystal-specific rules mirrored here:**

- **OLE date epoch.** A numeric date serial counts **days since 1899-12-30** (the OLE Automation epoch), via
  `civil::Date::from_ole_days`. So `DateValue(35000)` = 1995-10-28 and `DateValue(0)` = 1899-12-30; `DateTime(35000.5)`
  adds the fractional half-day as noon.
- **Interval codes** (`DateAdd`/`DateDiff`/`DatePart`): `yyyy` year, `q` quarter, `m` month, `d` day, `y` day-of-year,
  `w` weekday, `ww` week, `h` hour, `n` minute, `s` second. Note `w`/`ww` differ by function — in `DateAdd`/`DatePart`,
  `w` is a single weekday and `ww` a 7-day period; in `DateDiff`, `w` counts whole 7-day spans and `ww` counts
  `firstDayOfWeek`-boundary crossings (e.g. `DateDiff("ww", #5/1/2003#, #6/1/2003#, crMonday)` = 4).
- **`firstDayOfWeek`**: `crUseSystem`=0 and an omitted argument both default to Sunday (`crSunday`=1 … `crSaturday`=7).
  Threaded through `DayOfWeek`/`Weekday`/`WeekdayName`, `DatePart("w"/"ww", …)`, and `DateDiff("ww", …)`.
- **`firstWeekOfYear`** (`DatePart` only): `crFirstJan1` (0/1 — week 1 contains January 1, late-December never rolls
  forward), `crFirstFourDays` (2 — week 1 is the first week with ≥4 days in the new year; with a Monday start this is
  exactly ISO-8601), and `crFirstFullWeek` (3 — week 1 is the first full week). Modes 2/3 assign a boundary date to the
  year whose week 1 contains it, so late-December dates can be week 1 of the next year and early-January the last week of
  the previous year.
- **End-of-month clamping**: `DateAdd("m", 1, #1/31/2004#)` = 2004-02-29 (clamped), whereas `DateSerial` rolls over.

## Conversion (`conversion.rs`)

| Function (aliases) | Signature → return | Semantics |
| ------------------ | ------------------ | --------- |
| `ToNumber` / `CDbl` | `(num\|Bool\|numeric String) → Number` | Non-numeric string → `BadArg`. |
| `Val` | `(String) → Number` | Longest leading numeric prefix, `0` if none (VB `Val`). |
| `CCur` | `(num\|String) → Currency` | Strips `$` and thousands separators. |
| `CBool` | `(num\|Bool) → Boolean` | Nonzero → true. |
| `IsNumeric` | `(value) → Boolean` | Number/Currency, or a numeric string. |
| `ToText` / `CStr` | `(value[, …]) → String` | See below. |

`ToText` forms: a bare scalar (numbers get grouped 2-decimal en-US formatting, dates/times the default patterns); a
Number/Currency with a **picture string** (`ToText(x, "#,##0.00")`) or a numeric `decimals` argument plus optional
thousands/decimal separator strings (`ToText(x, 2, ".", ",")`). Date/time *format strings* are the layout engine's job
and report `Unsupported` here; a bare date/time uses the default pattern. `ToText` is one of the few builtins that sees
`Null` (yielding `""`) rather than propagating it.

## Financial (`financial.rs`)

Time-value-of-money and depreciation, following the Excel/VB6 financial functions Crystal mirrors. `type` is 0 (end of
period, default) or 1 (beginning); the cash-flow sign convention is Excel's (money paid out is negative), so a positive
present value yields a negative payment.

| Function | Signature → return | Semantics |
| -------- | ------------------ | --------- |
| `Pmt` | `(rate, nPeriods, pv[, fv[, type]]) → Number` | Periodic payment of an annuity. |
| `FV` | `(rate, nPeriods, pmt[, pv[, type]]) → Number` | Future value. |
| `PV` | `(rate, nPeriods, pmt[, fv[, type]]) → Number` | Present value. |
| `NPV` | `(rate, array) → Number` | Net present value; each flow discounted from period 1 (one **array** arg, not variadic). |
| `IRR` | `(array[, guess]) → Number` | Internal rate of return; first flow at period 0. Default `guess` = 0.1. |
| `Rate` | `(nPeriods, pmt, pv[, fv[, type[, guess]]]) → Number` | Periodic rate (Newton's method). Default `guess` = 0.1. |
| `DDB` | `(cost, salvage, life, period[, factor]) → Number` | Double-declining-balance depreciation (default `factor` = 2). |
| `SLN` | `(cost, salvage, life) → Number` | Straight-line depreciation. |
| `SYD` | `(cost, salvage, life, period) → Number` | Sum-of-years'-digits depreciation. |

`IRR`/`Rate` solve by Newton iteration and report `BadArg` if they do not converge. `NPV`/`IRR` require an array of
cash flows.

## Statistical (`statistical.rs`)

Dispersion over an **array** argument. As with `Sum`/`Average`, the record-set forms (`StdDev({field}, {group})`) need
the data pipeline and report `Unsupported`. Crystal has no array-literal `Median`/`Mode` (summary-only — see the deferred
list at the top).

| Function | Signature → return | Semantics |
| -------- | ------------------ | --------- |
| `StdDev` / `Variance` | `(array) → Number` | **Sample** dispersion (divisor n − 1). A single value errors. |
| `PopulationStdDev` / `PopulationVariance` | `(array) → Number` | **Population** dispersion (divisor n). |

Nulls in the array are skipped (matching the aggregate builtins); an all-null/empty array yields `Null`.

## Numeral (`numeral.rs`)

| Function | Signature → return | Semantics |
| -------- | ------------------ | --------- |
| `ToWords` | `(number[, decimals]) → String` | Cheque-style spelling: `ToText(1145.31)` → `"one thousand one hundred forty-five and 31 / 100"`. Default `decimals` = 2; `0` suppresses the fraction. Lowercase, hyphenated tens-units; the fraction is `NN / 10ᵈ` zero-padded. |
| `Roman` | `(number[, form]) → String` | Classic Roman numerals for 1..3999 (`Roman(1998)` → `"MCMXCVIII"`); `0` → `""`. The graduated simplified forms 1–4 are deferred (`Unsupported`). |

## Null, colour, and 0-ary constants (`mod.rs`)

- `IsNull(value) → Boolean` and `HasValue(value) → Boolean` see through null.
- `Color` / `RGB` `(r,g,b) → Number` build a COLORREF (`r + g·256 + b·65536`); the named colour constants
  (`crBlack`, `crRed`, `crWhite`, `crNoColor`, …) resolve to their COLORREF value.
- Print-state specials (`PageNumber`, `CurrentDate`, …) and the `WhilePrintingRecords`/`WhileReadingRecords` markers are
  described in [Architecture › References](01-architecture.md#references-and-the-evaluation-context).

## Adding a builtin

1. Add a variant to the `Builtin` enum (in the family's section) and its lowercase name(s) to `NAMES` (kept sorted — a
   test enforces it).
2. Classify it in `Builtin::family`.
3. Implement the arm in the family module's `call`, using the shared helpers (`str_arg`, `num_arg`, `opt_num`,
   `map_numeric`, `mismatch`, `bad_arg`).
4. Add tests (normal, edge, and error cases) in that module's `#[cfg(test)] mod tests`.

If the function's static return type isn't already in `types_table.rs`, add it there so type deduction and the exporter
agree with evaluation.
