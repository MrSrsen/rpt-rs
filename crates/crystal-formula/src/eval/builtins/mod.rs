//! Eager builtin functions, split by family (string / math / date-time / conversion / financial /
//! statistical / numeral) behind one name→variant table.
//!
//! Dispatch is enum-based with a single sorted name→[`Kind`] table ([`TABLE`]): aliases and the
//! print-state specials, markers, record-nav functions and color constants all live there behind one
//! binary search. [`dispatch`] returns `None` when the (lowercase) name is not a builtin this module
//! knows, letting the evaluator distinguish *unknown name* from *known-but-unimplemented* via the
//! funcID table. Print-state specials route through [`EvalContext::special`]. Unless noted, a `Null`
//! argument yields `Null` (the propagation rule).
//!
//! [`call`] resolves a variant to its family module ([`string`]/[`math`]/[`datetime`]/[`conversion`]/
//! [`financial`]/[`statistical`]/[`numeral`]); null/color constants are handled here. Each family
//! owns its own arm implementations and tests.

mod conversion;
mod datetime;
mod financial;
mod math;
mod numeral;
mod statistical;
mod string;

use super::value::Value;
use super::{EvalContext, EvalError};

/// The implemented eager builtins. One variant per operation; name aliases (`Len`→`Length`,
/// `CStr`→`ToText`, …) are folded in the [`TABLE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Builtin {
    // string
    Length,
    UpperCase,
    LowerCase,
    ProperCase,
    Trim,
    TrimLeft,
    TrimRight,
    Left,
    Right,
    Mid,
    InStr,
    InStrRev,
    Replace,
    ReplicateString,
    Space,
    StrReverse,
    Split,
    Join,
    Filter,
    StrCmp,
    Chr,
    Asc,
    // conversion
    Val,
    IsNumeric,
    ToNumber,
    CCur,
    CBool,
    ToText,
    // math
    Abs,
    Sgn,
    Int,
    Fix,
    Truncate,
    Round,
    RoundUp,
    MRound,
    Floor,
    Ceiling,
    Remainder,
    Sqr,
    Exp,
    Log,
    Sin,
    Cos,
    Tan,
    Atn,
    Pi,
    // collections
    Minimum,
    Maximum,
    Sum,
    Average,
    Count,
    UBound,
    // financial
    Pmt,
    FV,
    PV,
    Npv,
    Irr,
    Rate,
    Ddb,
    Sln,
    Syd,
    // statistical
    StdDev,
    Variance,
    PopulationStdDev,
    PopulationVariance,
    // numeral
    ToWords,
    Roman,
    // null
    IsNull,
    HasValue,
    // date & time
    DateCtor,
    TimeCtor,
    DateTimeCtor,
    DateValue,
    TimeValue,
    DateSerial,
    TimeSerial,
    DatePart,
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    DayOfWeek,
    Weekday,
    MonthName,
    WeekdayName,
    DateAdd,
    DateDiff,
    IsDate,
    IsTime,
    IsDateTime,
    // color
    Color,
}

/// Classification of a builtin name: an eager function (with its owning [`Family`], baked in at
/// compile time), a print/record-state special, an evaluation-time marker, a record-navigation
/// function, or a color constant (COLORREF layout: `r + g·256 + b·65536`).
#[derive(Clone, Copy)]
enum Kind {
    Func(Builtin, Family),
    /// Reads print/record state from the context rather than computing.
    Special,
    /// A statement with no computational effect here (the data pipeline treats it as a
    /// cache-refresh boundary).
    Marker,
    /// Needs the record stream (data-pipeline phase).
    RecordNav,
    /// A Crystal color constant, its numeric COLORREF value.
    Color(f64),
}

/// Table entry for an eager function, with its family derived from the variant at compile time so
/// [`family`](Builtin::family) is the single source of truth and dispatch does no runtime lookup.
const fn func(name: &str, builtin: Builtin) -> (&str, Kind) {
    (name, Kind::Func(builtin, builtin.family()))
}

/// The single sorted lowercase-name → [`Kind`] table (aliases included), binary-searched once per
/// call. Must stay sorted and duplicate-free (see the `table_is_sorted_and_unique` test).
const TABLE: &[(&str, Kind)] = &[
    func("abs", Builtin::Abs),
    func("asc", Builtin::Asc),
    func("ascw", Builtin::Asc),
    func("atn", Builtin::Atn),
    func("average", Builtin::Average),
    ("beforereadingrecords", Kind::Marker),
    func("cbool", Builtin::CBool),
    func("ccur", Builtin::CCur),
    func("cdate", Builtin::DateValue),
    func("cdatetime", Builtin::DateTimeCtor),
    func("cdbl", Builtin::ToNumber),
    func("ceiling", Builtin::Ceiling),
    func("chr", Builtin::Chr),
    func("chrw", Builtin::Chr),
    func("color", Builtin::Color),
    func("cos", Builtin::Cos),
    func("count", Builtin::Count),
    ("craqua", Kind::Color(16776960.0)),
    ("crblack", Kind::Color(0.0)),
    ("crblue", Kind::Color(16711680.0)),
    ("crcyan", Kind::Color(16776960.0)),
    ("crfuchsia", Kind::Color(16711935.0)),
    ("crgray", Kind::Color(8421504.0)),
    ("crgreen", Kind::Color(32768.0)),
    ("crlime", Kind::Color(65280.0)),
    ("crmagenta", Kind::Color(16711935.0)),
    ("crmaroon", Kind::Color(128.0)),
    ("crnavy", Kind::Color(8388608.0)),
    ("crnocolor", Kind::Color(-1.0)),
    ("crolive", Kind::Color(32896.0)),
    func("crpi", Builtin::Pi),
    ("crpurple", Kind::Color(8388736.0)),
    ("crred", Kind::Color(255.0)),
    ("crsilver", Kind::Color(12632256.0)),
    ("crteal", Kind::Color(8421376.0)),
    ("crwhite", Kind::Color(16777215.0)),
    ("cryellow", Kind::Color(65535.0)),
    func("cstr", Builtin::ToText),
    func("ctime", Builtin::TimeValue),
    ("currentdate", Kind::Special),
    ("currentdatetime", Kind::Special),
    ("currentfieldvalue", Kind::Special),
    ("currenttime", Kind::Special),
    ("datadate", Kind::Special),
    ("datatime", Kind::Special),
    ("datatimezone", Kind::Special),
    func("date", Builtin::DateCtor),
    func("dateadd", Builtin::DateAdd),
    func("datediff", Builtin::DateDiff),
    func("datepart", Builtin::DatePart),
    func("dateserial", Builtin::DateSerial),
    func("datetime", Builtin::DateTimeCtor),
    func("datetimevalue", Builtin::DateTimeCtor),
    func("datevalue", Builtin::DateValue),
    func("day", Builtin::Day),
    func("dayofweek", Builtin::DayOfWeek),
    func("ddb", Builtin::Ddb),
    ("drilldowngrouplevel", Kind::Special),
    func("exp", Builtin::Exp),
    func("filter", Builtin::Filter),
    func("fix", Builtin::Fix),
    func("floor", Builtin::Floor),
    func("fv", Builtin::FV),
    ("groupname", Kind::Special),
    ("groupnumber", Kind::Special),
    ("groupselection", Kind::Special),
    func("hasvalue", Builtin::HasValue),
    func("hour", Builtin::Hour),
    func("instr", Builtin::InStr),
    func("instrrev", Builtin::InStrRev),
    func("int", Builtin::Int),
    func("irr", Builtin::Irr),
    func("isdate", Builtin::IsDate),
    func("isdatetime", Builtin::IsDateTime),
    func("isnull", Builtin::IsNull),
    func("isnumeric", Builtin::IsNumeric),
    func("istime", Builtin::IsTime),
    func("join", Builtin::Join),
    func("lcase", Builtin::LowerCase),
    func("left", Builtin::Left),
    func("len", Builtin::Length),
    func("length", Builtin::Length),
    func("log", Builtin::Log),
    func("lowercase", Builtin::LowerCase),
    func("ltrim", Builtin::TrimLeft),
    func("maximum", Builtin::Maximum),
    func("mid", Builtin::Mid),
    func("minimum", Builtin::Minimum),
    func("minute", Builtin::Minute),
    ("modificationdate", Kind::Special),
    ("modificationtime", Kind::Special),
    func("month", Builtin::Month),
    func("monthname", Builtin::MonthName),
    func("mround", Builtin::MRound),
    ("next", Kind::RecordNav),
    ("nextisnull", Kind::RecordNav),
    ("nextvalue", Kind::RecordNav),
    func("npv", Builtin::Npv),
    ("onfirstrecord", Kind::Special),
    ("onlastrecord", Kind::Special),
    ("pagenofm", Kind::Special),
    ("pagenumber", Kind::Special),
    func("pmt", Builtin::Pmt),
    func("populationstddev", Builtin::PopulationStdDev),
    func("populationvariance", Builtin::PopulationVariance),
    ("previous", Kind::RecordNav),
    ("previousisnull", Kind::RecordNav),
    ("previousvalue", Kind::RecordNav),
    ("printdate", Kind::Special),
    ("printtime", Kind::Special),
    ("printtimezone", Kind::Special),
    func("propercase", Builtin::ProperCase),
    func("pv", Builtin::PV),
    func("rate", Builtin::Rate),
    ("recordnumber", Kind::Special),
    ("recordselection", Kind::Special),
    func("remainder", Builtin::Remainder),
    func("replace", Builtin::Replace),
    func("replicatestring", Builtin::ReplicateString),
    func("rgb", Builtin::Color),
    func("right", Builtin::Right),
    func("roman", Builtin::Roman),
    func("round", Builtin::Round),
    func("roundup", Builtin::RoundUp),
    ("rowcounter", Kind::Special),
    func("rtrim", Builtin::TrimRight),
    func("second", Builtin::Second),
    func("sgn", Builtin::Sgn),
    func("sin", Builtin::Sin),
    func("sln", Builtin::Sln),
    func("space", Builtin::Space),
    func("split", Builtin::Split),
    func("sqr", Builtin::Sqr),
    func("stddev", Builtin::StdDev),
    func("strcmp", Builtin::StrCmp),
    func("strreverse", Builtin::StrReverse),
    func("sum", Builtin::Sum),
    func("syd", Builtin::Syd),
    func("tan", Builtin::Tan),
    func("time", Builtin::TimeCtor),
    func("timeserial", Builtin::TimeSerial),
    ("timestring", Kind::Special),
    func("timevalue", Builtin::TimeValue),
    func("tonumber", Builtin::ToNumber),
    ("totalpagecount", Kind::Special),
    func("totext", Builtin::ToText),
    func("towords", Builtin::ToWords),
    func("trim", Builtin::Trim),
    func("trimleft", Builtin::TrimLeft),
    func("trimright", Builtin::TrimRight),
    func("truncate", Builtin::Truncate),
    func("ubound", Builtin::UBound),
    func("ucase", Builtin::UpperCase),
    func("uppercase", Builtin::UpperCase),
    func("val", Builtin::Val),
    func("variance", Builtin::Variance),
    func("weekday", Builtin::Weekday),
    func("weekdayname", Builtin::WeekdayName),
    ("whileprintingrecords", Kind::Marker),
    ("whilereadingrecords", Kind::Marker),
    func("year", Builtin::Year),
];

/// Classify a name via [`TABLE`] (already lowercased by the caller).
fn lookup(name: &str) -> Option<Kind> {
    TABLE
        .binary_search_by(|(n, _)| n.cmp(&name))
        .ok()
        .map(|i| TABLE[i].1)
}

/// The [`Kind::Special`] subset that is only resolvable during the format/print pass — the specials
/// that read positional print state (page/record/group position). The data-time specials like
/// `CurrentDate`/`DataDate` are deliberately excluded: they resolve earlier, before the print pass.
/// Names are lowercase.
const PRINT_STATE_SPECIALS: &[&str] = &[
    "pagenumber",
    "pagenofm",
    "totalpagecount",
    "recordnumber",
    "groupnumber",
    "groupselection",
    "onfirstrecord",
    "onlastrecord",
    "rowcounter",
    "drilldowngrouplevel",
];

/// Whether `name` is a print-pass special — the `Kind::Special` subset that reads positional print
/// state (page/record/group position) and is therefore only resolvable during the format/print pass.
/// `name` is lowercased before matching. Data-time specials (`CurrentDate`/`DataDate`/…) are **not**
/// print-state and return `false`; they resolve before the print pass.
pub fn is_print_state_special(name: &str) -> bool {
    PRINT_STATE_SPECIALS.contains(&name.to_lowercase().as_str())
}

/// Whether `name` is a record-navigation builtin (`Previous`/`Next`/…) — a real `TABLE` lookup
/// matching `Kind::RecordNav`, so the name set never drifts from the evaluator's. `name` is
/// lowercased before matching.
pub fn is_record_nav(name: &str) -> bool {
    matches!(lookup(&name.to_lowercase()), Some(Kind::RecordNav))
}

impl Builtin {
    /// Builtins that must see `Null` arguments rather than have them propagate.
    fn accepts_null(self) -> bool {
        matches!(self, Builtin::IsNull | Builtin::HasValue | Builtin::ToText)
    }

    /// Which family module owns this variant's implementation. `const` so [`func`] can bake the
    /// family into the [`TABLE`] entry at compile time.
    const fn family(self) -> Family {
        use Builtin as B;
        match self {
            B::Length
            | B::UpperCase
            | B::LowerCase
            | B::ProperCase
            | B::Trim
            | B::TrimLeft
            | B::TrimRight
            | B::Left
            | B::Right
            | B::Mid
            | B::InStr
            | B::InStrRev
            | B::Replace
            | B::ReplicateString
            | B::Space
            | B::StrReverse
            | B::Split
            | B::Join
            | B::Filter
            | B::StrCmp
            | B::Chr
            | B::Asc => Family::String,
            B::Val | B::IsNumeric | B::ToNumber | B::CCur | B::CBool | B::ToText => {
                Family::Conversion
            }
            B::Abs
            | B::Sgn
            | B::Int
            | B::Fix
            | B::Truncate
            | B::Round
            | B::RoundUp
            | B::MRound
            | B::Floor
            | B::Ceiling
            | B::Remainder
            | B::Sqr
            | B::Exp
            | B::Log
            | B::Sin
            | B::Cos
            | B::Tan
            | B::Atn
            | B::Pi
            | B::Minimum
            | B::Maximum
            | B::Sum
            | B::Average
            | B::Count
            | B::UBound => Family::Math,
            B::Pmt | B::FV | B::PV | B::Npv | B::Irr | B::Rate | B::Ddb | B::Sln | B::Syd => {
                Family::Financial
            }
            B::StdDev | B::Variance | B::PopulationStdDev | B::PopulationVariance => {
                Family::Statistical
            }
            B::ToWords | B::Roman => Family::Numeral,
            B::DateCtor
            | B::TimeCtor
            | B::DateTimeCtor
            | B::DateValue
            | B::TimeValue
            | B::DateSerial
            | B::TimeSerial
            | B::DatePart
            | B::Year
            | B::Month
            | B::Day
            | B::Hour
            | B::Minute
            | B::Second
            | B::DayOfWeek
            | B::Weekday
            | B::MonthName
            | B::WeekdayName
            | B::DateAdd
            | B::DateDiff
            | B::IsDate
            | B::IsTime
            | B::IsDateTime => Family::DateTime,
            B::IsNull | B::HasValue | B::Color => Family::Misc,
        }
    }
}

/// The module that implements a [`Builtin`].
#[derive(Clone, Copy)]
enum Family {
    String,
    Math,
    DateTime,
    Conversion,
    Financial,
    Statistical,
    Numeral,
    Misc,
}

/// Resolve a called name to a value, folding the shared unknown-name-vs-unsupported-builtin
/// decision the two evaluators must keep identical: a name [`dispatch`] doesn't know is an
/// [`EvalError::Unsupported`] when it is a known-but-unimplemented builtin (in the funcID table),
/// else an [`EvalError::UnknownName`]. `name` is used verbatim in the error; dispatch is
/// case-insensitive.
pub(super) fn resolve(
    name: &str,
    args: &[Value],
    ctx: &dyn EvalContext,
) -> Result<Value, EvalError> {
    let lname = name.to_lowercase();
    match dispatch(&lname, args, ctx) {
        Some(r) => r,
        None => Err(if crate::types::func_id(&lname).is_some() {
            EvalError::Unsupported(name.to_string())
        } else {
            EvalError::UnknownName(name.to_string())
        }),
    }
}

pub(super) fn dispatch(
    name: &str,
    args: &[Value],
    ctx: &dyn EvalContext,
) -> Option<Result<Value, EvalError>> {
    Some(match lookup(name)? {
        Kind::Special => ctx
            .special(name)
            .ok_or_else(|| EvalError::Unsupported(format!("`{name}` needs print/record context"))),
        Kind::Marker => Ok(Value::Bool(true)),
        Kind::RecordNav => Err(EvalError::Unsupported(format!(
            "`{name}` needs record context"
        ))),
        Kind::Color(c) => Ok(Value::Number(c)),
        Kind::Func(builtin, family) => {
            if !builtin.accepts_null() && args.iter().any(Value::is_null) {
                Ok(Value::Null)
            } else {
                call(builtin, family, name, args)
            }
        }
    })
}

/// Route a resolved [`Builtin`] to its family module; null/color are handled here.
fn call(builtin: Builtin, family: Family, name: &str, args: &[Value]) -> Result<Value, EvalError> {
    use Builtin as B;
    match family {
        Family::String => string::call(builtin, name, args),
        Family::Math => math::call(builtin, name, args),
        Family::DateTime => datetime::call(builtin, name, args),
        Family::Conversion => conversion::call(builtin, name, args),
        Family::Financial => financial::call(builtin, name, args),
        Family::Statistical => statistical::call(builtin, name, args),
        Family::Numeral => numeral::call(builtin, name, args),
        Family::Misc => match builtin {
            B::IsNull => Ok(Value::Bool(args.first().is_none_or(Value::is_null))),
            B::HasValue => Ok(Value::Bool(!args.first().is_none_or(Value::is_null))),
            B::Color => {
                let (r, g, b) = (
                    num_arg(name, args, 0)?,
                    num_arg(name, args, 1)?,
                    num_arg(name, args, 2)?,
                );
                Ok(Value::Number(r + g * 256.0 + b * 65536.0))
            }
            other => unreachable!("non-misc builtin {other:?} routed to misc"),
        },
    }
}

// ---- shared argument/error helpers (visible to the family modules) ----

fn mismatch(name: &str, got: &Value) -> EvalError {
    EvalError::TypeMismatch {
        what: name.to_string(),
        got: got.type_name().to_string(),
    }
}

/// A `{name}: {msg}` bad-argument error.
fn bad_arg(name: &str, msg: &str) -> EvalError {
    EvalError::BadArg(format!("{name}: {msg}"))
}

fn str_arg(name: &str, args: &[Value], i: usize) -> Result<String, EvalError> {
    match args.get(i) {
        Some(Value::Str(s)) => Ok(s.clone()),
        Some(v) => Err(mismatch(name, v)),
        None => Err(EvalError::BadArg(format!(
            "{name}: missing argument {}",
            i + 1
        ))),
    }
}

fn num_arg(name: &str, args: &[Value], i: usize) -> Result<f64, EvalError> {
    match args.get(i) {
        Some(v) => v.as_number().ok_or_else(|| mismatch(name, v)),
        None => Err(EvalError::BadArg(format!(
            "{name}: missing argument {}",
            i + 1
        ))),
    }
}

fn opt_num(args: &[Value], i: usize) -> Option<f64> {
    args.get(i).and_then(Value::as_number)
}

/// Number-or-Currency map that preserves the currency-ness of the input.
fn map_numeric(v: &Value, name: &str, f: impl Fn(f64) -> f64) -> Result<Value, EvalError> {
    match v {
        Value::Number(n) => Ok(Value::Number(f(*n))),
        Value::Currency(n) => Ok(Value::Currency(f(*n))),
        v => Err(mismatch(name, v)),
    }
}

#[cfg(test)]
mod tests {
    use super::{is_print_state_special, is_record_nav, Kind, TABLE};

    #[test]
    fn print_state_specials_are_recognised() {
        for name in [
            "pagenumber",
            "PageNoFM",
            "TotalPageCount",
            "recordnumber",
            "groupnumber",
            "GroupSelection",
            "onfirstrecord",
            "OnLastRecord",
            "rowcounter",
            "drilldowngrouplevel",
        ] {
            assert!(
                is_print_state_special(name),
                "`{name}` should be print-state"
            );
        }
    }

    #[test]
    fn data_time_specials_are_not_print_state() {
        // These are `Kind::Special` but resolve before the print pass, so they must not classify as
        // print-state (they force read-time, not print-time, evaluation downstream).
        for name in [
            "currentdate",
            "datadate",
            "currentdatetime",
            "printdate",
            "groupname",
        ] {
            assert!(!is_print_state_special(name), "`{name}` is not print-state");
            assert!(!is_record_nav(name), "`{name}` is not record-nav");
        }
    }

    #[test]
    fn record_nav_names_are_recognised() {
        for name in [
            "previous",
            "Next",
            "previousvalue",
            "NextValue",
            "previousisnull",
            "nextisnull",
        ] {
            assert!(is_record_nav(name), "`{name}` should be record-nav");
            assert!(
                !is_print_state_special(name),
                "record-nav `{name}` is not a print-state special"
            );
        }
    }

    #[test]
    fn unknown_names_are_neither() {
        for name in ["orders", "amount", "next_ship_date", ""] {
            assert!(!is_print_state_special(name));
            assert!(!is_record_nav(name));
        }
    }

    /// Every `Kind::RecordNav` name must be reported by [`is_record_nav`], and no other kind may be —
    /// the predicate is the single authority the data pipeline depends on.
    #[test]
    fn record_nav_predicate_matches_table() {
        for (name, kind) in TABLE {
            assert_eq!(
                is_record_nav(name),
                matches!(kind, Kind::RecordNav),
                "is_record_nav disagrees with TABLE for `{name}`"
            );
        }
    }

    /// [`lookup`](super::lookup) binary-searches [`TABLE`] — it must stay sorted and duplicate-free.
    #[test]
    fn table_is_sorted_and_unique() {
        for pair in TABLE.windows(2) {
            assert!(pair[0].0 < pair[1].0, "`{}` >= `{}`", pair[0].0, pair[1].0);
        }
    }

    /// Every function implemented in the eval [`TABLE`] must also appear in the generated
    /// `NAME_FUNCID` type table (`types_table.rs`), which drives return-type / argument validation.
    /// The two tables are maintained separately, so this guards against a new builtin being wired
    /// into the evaluator while its type-system entry is forgotten (or vice versa).
    #[test]
    fn implemented_funcs_are_in_name_funcid() {
        use std::collections::HashSet;
        // The date/time *constructors* are handled specially by the type system (as type
        // constructors, not funcID-dispatched functions), so they are legitimately absent from the
        // generated funcID table. Every other implemented function must be present.
        const CONSTRUCTOR_EXCEPTIONS: &[&str] = &["date", "datetime", "time"];
        let typed: HashSet<&str> = crate::types::NAME_FUNCID.iter().map(|(n, _)| *n).collect();
        for (name, kind) in TABLE {
            if matches!(kind, Kind::Func(..)) && !CONSTRUCTOR_EXCEPTIONS.contains(name) {
                assert!(
                    typed.contains(name),
                    "builtin `{name}` is implemented in eval::builtins TABLE but is missing from the \
                     generated NAME_FUNCID table (types_table.rs) — its return type / arg validation \
                     would be unknown (add it there, or to CONSTRUCTOR_EXCEPTIONS if it is a special)"
                );
            }
        }
    }
}
