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
mod daterange;
mod datetime;
mod financial;
mod math;
mod numeral;
mod statistical;
mod string;

use conversion::ConversionFn;
use daterange::DateRange;
use datetime::DateTimeFn;
use financial::FinancialFn;
use math::MathFn;
use numeral::NumeralFn;
use statistical::StatisticalFn;
use string::StringFn;

use super::value::Value;
use super::{EvalContext, EvalError};

/// An implemented eager builtin, tagged by the family module that owns it. Each variant wraps that
/// module's own function enum, so a family module's `call` matches exhaustively over only its own
/// operations (no catch-all) and adding a builtin touches just one family enum plus its [`TABLE`]
/// row. Name aliases (`Len`→`Length`, `CStr`→`ToText`, …) are folded in the [`TABLE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Builtin {
    String(StringFn),
    Conversion(ConversionFn),
    Math(MathFn),
    Financial(FinancialFn),
    Statistical(StatisticalFn),
    Numeral(NumeralFn),
    DateTime(DateTimeFn),
    Misc(MiscFn),
}

/// The builtins with no natural family: the null predicates and the `Color`/`RGB` constructor.
/// Implemented inline in [`misc_call`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MiscFn {
    IsNull,
    HasValue,
    Color,
}

/// Classification of a builtin name: an eager function (a [`Builtin`], which carries its owning
/// family module), a print/record-state special, an evaluation-time marker, a record-navigation
/// function, or a color constant (COLORREF layout: `r + g·256 + b·65536`).
#[derive(Clone, Copy)]
enum Kind {
    Func(Builtin),
    /// Reads print/record state from the context rather than computing.
    Special,
    /// A statement with no computational effect here (the data pipeline treats it as a
    /// cache-refresh boundary).
    Marker,
    /// Needs the record stream (data-pipeline phase).
    RecordNav,
    /// A Crystal color constant, its numeric COLORREF value.
    Color(f64),
    /// A 0-ary `cr*` enum constant (alignment / font style / line style / negative & currency
    /// format / calendar …) that evaluates to a bare number. The value carried is the engine funcID
    /// the constant occupies in [`crate::types::NAME_FUNCID`] — a stable, unique identifier the
    /// conditional-format consumers compare against. (The colour `cr*` constants have their own
    /// [`Kind::Color`] COLORREF value and are not `Const`.)
    Const(f64),
    /// A relative-date-range constant (`YearToDate`, `Last7Days`, `LastFullMonth`, …): evaluates to a
    /// [`Value::Range`] of dates computed from the context's reference "today" date.
    DateRange(DateRange),
}

/// [`TABLE`] entry for an eager function. The [`Builtin`] variant names the owning family module, so
/// dispatch routes without a runtime family lookup.
const fn func(name: &str, builtin: Builtin) -> (&str, Kind) {
    (name, Kind::Func(builtin))
}

/// The single sorted lowercase-name → [`Kind`] table (aliases included), binary-searched once per
/// call. Must stay sorted and duplicate-free (see the `table_is_sorted_and_unique` test).
const TABLE: &[(&str, Kind)] = &[
    func("abs", Builtin::Math(MathFn::Abs)),
    ("aged0to30days", Kind::DateRange(DateRange::Aged0To30Days)),
    ("aged31to60days", Kind::DateRange(DateRange::Aged31To60Days)),
    ("aged61to90days", Kind::DateRange(DateRange::Aged61To90Days)),
    (
        "alldatesfromtoday",
        Kind::DateRange(DateRange::AllDatesFromToday),
    ),
    (
        "alldatesfromtomorrow",
        Kind::DateRange(DateRange::AllDatesFromTomorrow),
    ),
    (
        "alldatestotoday",
        Kind::DateRange(DateRange::AllDatesToToday),
    ),
    (
        "alldatestoyesterday",
        Kind::DateRange(DateRange::AllDatesToYesterday),
    ),
    func("asc", Builtin::String(StringFn::Asc)),
    func("ascw", Builtin::String(StringFn::Asc)),
    func("atn", Builtin::Math(MathFn::Atn)),
    func("average", Builtin::Math(MathFn::Average)),
    ("beforereadingrecords", Kind::Marker),
    (
        "calendar1sthalf",
        Kind::DateRange(DateRange::Calendar1stHalf),
    ),
    ("calendar1stqtr", Kind::DateRange(DateRange::Calendar1stQtr)),
    (
        "calendar2ndhalf",
        Kind::DateRange(DateRange::Calendar2ndHalf),
    ),
    ("calendar2ndqtr", Kind::DateRange(DateRange::Calendar2ndQtr)),
    ("calendar3rdqtr", Kind::DateRange(DateRange::Calendar3rdQtr)),
    ("calendar4thqtr", Kind::DateRange(DateRange::Calendar4thQtr)),
    func("cbool", Builtin::Conversion(ConversionFn::CBool)),
    func("ccur", Builtin::Conversion(ConversionFn::CCur)),
    func("cdate", Builtin::DateTime(DateTimeFn::DateValue)),
    func("cdatetime", Builtin::DateTime(DateTimeFn::DateTimeCtor)),
    func("cdbl", Builtin::Conversion(ConversionFn::ToNumber)),
    func("ceiling", Builtin::Math(MathFn::Ceiling)),
    func("chr", Builtin::String(StringFn::Chr)),
    func("chrw", Builtin::String(StringFn::Chr)),
    func("color", Builtin::Misc(MiscFn::Color)),
    func("cos", Builtin::Math(MathFn::Cos)),
    func("count", Builtin::Math(MathFn::Count)),
    ("crampmafter", Kind::Const(292.0)),
    ("crampmbefore", Kind::Const(291.0)),
    ("craqua", Kind::Color(16776960.0)),
    ("crascendingorder", Kind::Const(368.0)),
    ("crblack", Kind::Color(0.0)),
    ("crblue", Kind::Color(16711680.0)),
    ("crbold", Kind::Const(329.0)),
    ("crbolditalic", Kind::Const(331.0)),
    ("crbottomaligned", Kind::Const(249.0)),
    ("crbracketednegatives", Kind::Const(301.0)),
    ("crcenteredhorizontally", Kind::Const(245.0)),
    ("crcenteredvertically", Kind::Const(250.0)),
    ("crcustom", Kind::Const(279.0)),
    ("crcyan", Kind::Color(16776960.0)),
    ("crdashedline", Kind::Const(254.0)),
    ("crdateonly", Kind::Const(258.0)),
    ("crdatethentime", Kind::Const(256.0)),
    ("crdaymonthyear", Kind::Const(262.0)),
    ("crdayofweekinfwparentheses", Kind::Const(319.0)),
    ("crdayofweekinfwsquarebrackets", Kind::Const(321.0)),
    ("crdayofweekinparentheses", Kind::Const(318.0)),
    ("crdayofweekinsquarebrackets", Kind::Const(320.0)),
    ("crdayofweeknotenclosed", Kind::Const(317.0)),
    ("crdefaulthoraligned", Kind::Const(242.0)),
    ("crdefaultorientation", Kind::Const(390.0)),
    ("crdefaultveraligned", Kind::Const(247.0)),
    ("crdescendingorder", Kind::Const(369.0)),
    ("crdottedline", Kind::Const(255.0)),
    ("crdoubleline", Kind::Const(253.0)),
    ("crfirstfourdays", Kind::Const(148.0)),
    ("crfirstfullweek", Kind::Const(149.0)),
    ("crfirstjan1", Kind::Const(147.0)),
    ("crfixedcurrencysymbol", Kind::Const(307.0)),
    ("crfloatingcurrencysymbol", Kind::Const(308.0)),
    ("crfriday", Kind::Const(145.0)),
    ("crfuchsia", Kind::Color(16711935.0)),
    ("crgray", Kind::Color(8421504.0)),
    ("crgreen", Kind::Color(32768.0)),
    ("crgregoriancalendar", Kind::Const(325.0)),
    ("crgregorianenglishcalendar", Kind::Const(326.0)),
    ("crhairline", Kind::Const(395.0)),
    ("crhalfpointline", Kind::Const(396.0)),
    ("crhtmltext", Kind::Const(311.0)),
    ("critalic", Kind::Const(330.0)),
    ("crjapanesecalendar", Kind::Const(327.0)),
    ("crjustified", Kind::Const(246.0)),
    ("crlandscape", Kind::Const(389.0)),
    ("crleadingcurrencyinsidenegative", Kind::Const(302.0)),
    ("crleadingcurrencyoutsidenegative", Kind::Const(303.0)),
    ("crleadingdayofweek", Kind::Const(315.0)),
    ("crleadingminus", Kind::Const(299.0)),
    ("crleadingzeroday", Kind::Const(272.0)),
    ("crleadingzerohour", Kind::Const(283.0)),
    ("crleadingzerominute", Kind::Const(286.0)),
    ("crleadingzeromonth", Kind::Const(267.0)),
    ("crleadingzerosecond", Kind::Const(289.0)),
    ("crleftaligned", Kind::Const(243.0)),
    ("crlefttorighttextreadingorder", Kind::Const(364.0)),
    ("crlime", Kind::Color(65280.0)),
    ("crlongdayofweek", Kind::Const(313.0)),
    ("crlongera", Kind::Const(323.0)),
    ("crlongleadingday", Kind::Const(276.0)),
    ("crlongmonth", Kind::Const(269.0)),
    ("crlongyear", Kind::Const(264.0)),
    ("crmagenta", Kind::Color(16711935.0)),
    ("crmaroon", Kind::Color(128.0)),
    ("crmonday", Kind::Const(141.0)),
    ("crmonthdayyear", Kind::Const(261.0)),
    ("crnavy", Kind::Color(8388608.0)),
    ("crnocolor", Kind::Color(-1.0)),
    ("crnocurrencysymbol", Kind::Const(306.0)),
    ("crnoday", Kind::Const(273.0)),
    ("crnodayofweek", Kind::Const(314.0)),
    ("crnoera", Kind::Const(324.0)),
    ("crnohour", Kind::Const(282.0)),
    ("crnoleadingday", Kind::Const(274.0)),
    ("crnoline", Kind::Const(251.0)),
    ("crnominute", Kind::Const(285.0)),
    ("crnomonth", Kind::Const(270.0)),
    ("crnonegativesign", Kind::Const(298.0)),
    ("crnosecond", Kind::Const(288.0)),
    ("crnoyear", Kind::Const(265.0)),
    ("crnumericday", Kind::Const(271.0)),
    ("crnumerichour", Kind::Const(284.0)),
    ("crnumericminute", Kind::Const(287.0)),
    ("crnumericmonth", Kind::Const(266.0)),
    ("crnumericsecond", Kind::Const(290.0)),
    ("crolive", Kind::Color(32896.0)),
    ("croneandhalfpointline", Kind::Const(398.0)),
    ("croneorzero", Kind::Const(297.0)),
    ("cronepointline", Kind::Const(397.0)),
    ("croriginalorder", Kind::Const(370.0)),
    func("crpi", Builtin::Math(MathFn::Pi)),
    ("crportrait", Kind::Const(388.0)),
    ("crpurple", Kind::Color(8388736.0)),
    ("crred", Kind::Color(255.0)),
    ("crregular", Kind::Const(328.0)),
    ("crrightaligned", Kind::Const(244.0)),
    ("crrighttolefttextreadingorder", Kind::Const(365.0)),
    ("crrtftext", Kind::Const(310.0)),
    ("crsaturday", Kind::Const(146.0)),
    ("crshortdayofweek", Kind::Const(312.0)),
    ("crshortera", Kind::Const(322.0)),
    ("crshortleadingday", Kind::Const(275.0)),
    ("crshortmonth", Kind::Const(268.0)),
    ("crshortyear", Kind::Const(263.0)),
    ("crsilver", Kind::Color(12632256.0)),
    ("crsingleline", Kind::Const(252.0)),
    ("crsunday", Kind::Const(140.0)),
    ("crteal", Kind::Color(8421376.0)),
    ("crthreeandhalfpointline", Kind::Const(402.0)),
    ("crthreepointline", Kind::Const(401.0)),
    ("crthursday", Kind::Const(144.0)),
    ("crtimeonly", Kind::Const(259.0)),
    ("crtimethendate", Kind::Const(257.0)),
    ("crtopaligned", Kind::Const(248.0)),
    ("crtorf", Kind::Const(294.0)),
    ("crtrailingcurrencyinsidenegative", Kind::Const(304.0)),
    ("crtrailingcurrencyoutsidenegative", Kind::Const(305.0)),
    ("crtrailingdayofweek", Kind::Const(316.0)),
    ("crtrailingminus", Kind::Const(300.0)),
    ("crtrueorfalse", Kind::Const(293.0)),
    ("crtuesday", Kind::Const(142.0)),
    ("crtwelvehour", Kind::Const(280.0)),
    ("crtwentyfourhour", Kind::Const(281.0)),
    ("crtwoandhalfpointline", Kind::Const(400.0)),
    ("crtwopointline", Kind::Const(399.0)),
    ("cruninterpretedtext", Kind::Const(309.0)),
    ("crusesystem", Kind::Const(139.0)),
    ("crwednesday", Kind::Const(143.0)),
    ("crwhite", Kind::Color(16777215.0)),
    ("crwindowslong", Kind::Const(277.0)),
    ("crwindowsshort", Kind::Const(278.0)),
    ("cryearmonthday", Kind::Const(260.0)),
    ("cryellow", Kind::Color(65535.0)),
    ("cryesorno", Kind::Const(295.0)),
    ("cryorn", Kind::Const(296.0)),
    func("cstr", Builtin::Conversion(ConversionFn::ToText)),
    func("ctime", Builtin::DateTime(DateTimeFn::TimeValue)),
    ("currentdate", Kind::Special),
    ("currentdatetime", Kind::Special),
    ("currentfieldvalue", Kind::Special),
    ("currenttime", Kind::Special),
    ("datadate", Kind::Special),
    ("datatime", Kind::Special),
    ("datatimezone", Kind::Special),
    func("date", Builtin::DateTime(DateTimeFn::DateCtor)),
    func("dateadd", Builtin::DateTime(DateTimeFn::DateAdd)),
    func("datediff", Builtin::DateTime(DateTimeFn::DateDiff)),
    func("datepart", Builtin::DateTime(DateTimeFn::DatePart)),
    func("dateserial", Builtin::DateTime(DateTimeFn::DateSerial)),
    func("datetime", Builtin::DateTime(DateTimeFn::DateTimeCtor)),
    func("datetimevalue", Builtin::DateTime(DateTimeFn::DateTimeCtor)),
    func("datevalue", Builtin::DateTime(DateTimeFn::DateValue)),
    func("day", Builtin::DateTime(DateTimeFn::Day)),
    func("dayofweek", Builtin::DateTime(DateTimeFn::DayOfWeek)),
    func("ddb", Builtin::Financial(FinancialFn::Ddb)),
    ("drilldowngrouplevel", Kind::Special),
    func("exp", Builtin::Math(MathFn::Exp)),
    func("filter", Builtin::String(StringFn::Filter)),
    func("fix", Builtin::Math(MathFn::Fix)),
    func("floor", Builtin::Math(MathFn::Floor)),
    func("fv", Builtin::Financial(FinancialFn::FV)),
    ("groupname", Kind::Special),
    ("groupnumber", Kind::Special),
    ("groupselection", Kind::Special),
    func("hasvalue", Builtin::Misc(MiscFn::HasValue)),
    func("hour", Builtin::DateTime(DateTimeFn::Hour)),
    func("instr", Builtin::String(StringFn::InStr)),
    func("instrrev", Builtin::String(StringFn::InStrRev)),
    func("int", Builtin::Math(MathFn::Int)),
    func("irr", Builtin::Financial(FinancialFn::Irr)),
    func("isdate", Builtin::DateTime(DateTimeFn::IsDate)),
    func("isdatetime", Builtin::DateTime(DateTimeFn::IsDateTime)),
    func("isnull", Builtin::Misc(MiscFn::IsNull)),
    func("isnumeric", Builtin::Conversion(ConversionFn::IsNumeric)),
    func("istime", Builtin::DateTime(DateTimeFn::IsTime)),
    func("join", Builtin::String(StringFn::Join)),
    (
        "last4weekstosun",
        Kind::DateRange(DateRange::Last4WeeksToSun),
    ),
    ("last7days", Kind::DateRange(DateRange::Last7Days)),
    ("lastfullmonth", Kind::DateRange(DateRange::LastFullMonth)),
    ("lastfullweek", Kind::DateRange(DateRange::LastFullWeek)),
    ("lastyearmtd", Kind::DateRange(DateRange::LastYearMtd)),
    ("lastyearytd", Kind::DateRange(DateRange::LastYearYtd)),
    func("lcase", Builtin::String(StringFn::LowerCase)),
    func("left", Builtin::String(StringFn::Left)),
    func("len", Builtin::String(StringFn::Length)),
    func("length", Builtin::String(StringFn::Length)),
    func("log", Builtin::Math(MathFn::Log)),
    func("lowercase", Builtin::String(StringFn::LowerCase)),
    func("ltrim", Builtin::String(StringFn::TrimLeft)),
    func("maximum", Builtin::Math(MathFn::Maximum)),
    func("mid", Builtin::String(StringFn::Mid)),
    func("minimum", Builtin::Math(MathFn::Minimum)),
    func("minute", Builtin::DateTime(DateTimeFn::Minute)),
    ("modificationdate", Kind::Special),
    ("modificationtime", Kind::Special),
    func("month", Builtin::DateTime(DateTimeFn::Month)),
    func("monthname", Builtin::DateTime(DateTimeFn::MonthName)),
    ("monthtodate", Kind::DateRange(DateRange::MonthToDate)),
    func("mround", Builtin::Math(MathFn::MRound)),
    ("next", Kind::RecordNav),
    ("next30days", Kind::DateRange(DateRange::Next30Days)),
    ("next31to60days", Kind::DateRange(DateRange::Next31To60Days)),
    ("next61to90days", Kind::DateRange(DateRange::Next61To90Days)),
    (
        "next91to365days",
        Kind::DateRange(DateRange::Next91To365Days),
    ),
    ("nextisnull", Kind::RecordNav),
    ("nextvalue", Kind::RecordNav),
    func("npv", Builtin::Financial(FinancialFn::Npv)),
    ("onfirstrecord", Kind::Special),
    ("onlastrecord", Kind::Special),
    ("over90days", Kind::DateRange(DateRange::Over90Days)),
    ("pagenofm", Kind::Special),
    ("pagenumber", Kind::Special),
    func("pmt", Builtin::Financial(FinancialFn::Pmt)),
    func(
        "populationstddev",
        Builtin::Statistical(StatisticalFn::PopulationStdDev),
    ),
    func(
        "populationvariance",
        Builtin::Statistical(StatisticalFn::PopulationVariance),
    ),
    ("previous", Kind::RecordNav),
    ("previousisnull", Kind::RecordNav),
    ("previousvalue", Kind::RecordNav),
    ("printdate", Kind::Special),
    ("printtime", Kind::Special),
    ("printtimezone", Kind::Special),
    func("propercase", Builtin::String(StringFn::ProperCase)),
    func("pv", Builtin::Financial(FinancialFn::PV)),
    func("rate", Builtin::Financial(FinancialFn::Rate)),
    ("recordnumber", Kind::Special),
    ("recordselection", Kind::Special),
    func("remainder", Builtin::Math(MathFn::Remainder)),
    func("replace", Builtin::String(StringFn::Replace)),
    func(
        "replicatestring",
        Builtin::String(StringFn::ReplicateString),
    ),
    func("rgb", Builtin::Misc(MiscFn::Color)),
    func("right", Builtin::String(StringFn::Right)),
    func("roman", Builtin::Numeral(NumeralFn::Roman)),
    func("round", Builtin::Math(MathFn::Round)),
    func("roundup", Builtin::Math(MathFn::RoundUp)),
    ("rowcounter", Kind::Special),
    func("rtrim", Builtin::String(StringFn::TrimRight)),
    func("second", Builtin::DateTime(DateTimeFn::Second)),
    func("sgn", Builtin::Math(MathFn::Sgn)),
    func("sin", Builtin::Math(MathFn::Sin)),
    func("sln", Builtin::Financial(FinancialFn::Sln)),
    func("space", Builtin::String(StringFn::Space)),
    func("split", Builtin::String(StringFn::Split)),
    func("sqr", Builtin::Math(MathFn::Sqr)),
    func("stddev", Builtin::Statistical(StatisticalFn::StdDev)),
    func("strcmp", Builtin::String(StringFn::StrCmp)),
    func("strreverse", Builtin::String(StringFn::StrReverse)),
    func("sum", Builtin::Math(MathFn::Sum)),
    func("syd", Builtin::Financial(FinancialFn::Syd)),
    func("tan", Builtin::Math(MathFn::Tan)),
    func("time", Builtin::DateTime(DateTimeFn::TimeCtor)),
    func("timeserial", Builtin::DateTime(DateTimeFn::TimeSerial)),
    ("timestring", Kind::Special),
    func("timevalue", Builtin::DateTime(DateTimeFn::TimeValue)),
    // `Today` is the engine's alias for `CurrentDate` (same funcID 154): a data-time special the
    // host supplies via `EvalContext::special`, resolved before the print pass.
    ("today", Kind::Special),
    func("tonumber", Builtin::Conversion(ConversionFn::ToNumber)),
    ("totalpagecount", Kind::Special),
    func("totext", Builtin::Conversion(ConversionFn::ToText)),
    func("towords", Builtin::Numeral(NumeralFn::ToWords)),
    func("trim", Builtin::String(StringFn::Trim)),
    func("trimleft", Builtin::String(StringFn::TrimLeft)),
    func("trimright", Builtin::String(StringFn::TrimRight)),
    func("truncate", Builtin::Math(MathFn::Truncate)),
    func("ubound", Builtin::Math(MathFn::UBound)),
    func("ucase", Builtin::String(StringFn::UpperCase)),
    func("uppercase", Builtin::String(StringFn::UpperCase)),
    func("val", Builtin::Conversion(ConversionFn::Val)),
    func("variance", Builtin::Statistical(StatisticalFn::Variance)),
    func("weekday", Builtin::DateTime(DateTimeFn::Weekday)),
    func("weekdayname", Builtin::DateTime(DateTimeFn::WeekdayName)),
    (
        "weektodatefromsun",
        Kind::DateRange(DateRange::WeekToDateFromSun),
    ),
    ("whileprintingrecords", Kind::Marker),
    ("whilereadingrecords", Kind::Marker),
    func("year", Builtin::DateTime(DateTimeFn::Year)),
    ("yeartodate", Kind::DateRange(DateRange::YearToDate)),
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
        matches!(
            self,
            Builtin::Misc(MiscFn::IsNull | MiscFn::HasValue)
                | Builtin::Conversion(ConversionFn::ToText)
        )
    }
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
        Kind::Const(n) => Ok(Value::Number(n)),
        Kind::DateRange(range) => daterange::eval(range, ctx),
        Kind::Func(builtin) => {
            // Table-driven arity gate: reject a structurally wrong argument count before the family
            // module, which would otherwise silently ignore extra args or read past the end.
            if let Some(id) = crate::types::func_id(name) {
                let sig = crate::types::sig(id);
                if !sig.accepts(args.len()) {
                    return Some(Err(EvalError::Arity {
                        name: name.to_string(),
                        expected: sig.expected(),
                        got: args.len(),
                    }));
                }
            }
            if !builtin.accepts_null() && args.iter().any(Value::is_null) {
                Ok(Value::Null)
            } else {
                call(builtin, name, args)
            }
        }
    })
}

/// Route a resolved [`Builtin`] to the family module named by its variant.
fn call(builtin: Builtin, name: &str, args: &[Value]) -> Result<Value, EvalError> {
    match builtin {
        Builtin::String(f) => string::call(f, name, args),
        Builtin::Conversion(f) => conversion::call(f, name, args),
        Builtin::Math(f) => math::call(f, name, args),
        Builtin::Financial(f) => financial::call(f, name, args),
        Builtin::Statistical(f) => statistical::call(f, name, args),
        Builtin::Numeral(f) => numeral::call(f, name, args),
        Builtin::DateTime(f) => datetime::call(f, name, args),
        Builtin::Misc(f) => misc_call(f, name, args),
    }
}

/// The family-less builtins: the null predicates and the `Color`/`RGB` constructor (COLORREF layout
/// `r + g·256 + b·65536`).
fn misc_call(f: MiscFn, name: &str, args: &[Value]) -> Result<Value, EvalError> {
    match f {
        MiscFn::IsNull => Ok(Value::Bool(args.first().is_none_or(Value::is_null))),
        MiscFn::HasValue => Ok(Value::Bool(!args.first().is_none_or(Value::is_null))),
        MiscFn::Color => {
            let (r, g, b) = (
                num_arg(name, args, 0)?,
                num_arg(name, args, 1)?,
                num_arg(name, args, 2)?,
            );
            Ok(Value::Number(r + g * 256.0 + b * 65536.0))
        }
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

/// The `i`-th argument by reference, or a missing-argument [`EvalError::BadArg`] when absent — the
/// checked alternative to `&args[i]` so a too-few-arguments call errors instead of panicking.
fn arg<'a>(name: &str, args: &'a [Value], i: usize) -> Result<&'a Value, EvalError> {
    args.get(i)
        .ok_or_else(|| EvalError::BadArg(format!("{name}: missing argument {}", i + 1)))
}

/// Number-or-Currency map over the `i`-th argument that preserves the currency-ness of the input.
fn map_numeric(
    name: &str,
    args: &[Value],
    i: usize,
    f: impl Fn(f64) -> f64,
) -> Result<Value, EvalError> {
    match arg(name, args, i)? {
        Value::Number(n) => Ok(Value::Number(f(*n))),
        Value::Currency(n) => Ok(Value::Currency(f(*n))),
        v => Err(mismatch(name, v)),
    }
}

/// The single array argument of an array-form aggregate, or the shared "needs a data context"
/// [`EvalError::Unsupported`]. The record-set forms (`Sum({field}, {group})`) need the data
/// pipeline this crate lacks, so a non-array argument reports that uniformly.
fn array_arg<'a>(name: &str, args: &'a [Value]) -> Result<&'a [Value], EvalError> {
    match args {
        [Value::Array(a)] => Ok(a),
        _ => Err(EvalError::Unsupported(format!(
            "{name} over records (needs data context)"
        ))),
    }
}

/// The non-null elements of `items` as `f64`, erroring on a non-numeric element. Nulls are skipped
/// (the aggregate/statistical null rule). Loses currency-ness, so callers that must preserve it
/// (e.g. `Sum`/`Average`) collect their own way.
fn numeric_non_null(name: &str, items: &[Value]) -> Result<Vec<f64>, EvalError> {
    let mut out = Vec::with_capacity(items.len());
    for v in items {
        if v.is_null() {
            continue;
        }
        out.push(v.as_number().ok_or_else(|| mismatch(name, v))?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{is_print_state_special, is_record_nav, Kind, PRINT_STATE_SPECIALS, TABLE};

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

    /// [`PRINT_STATE_SPECIALS`] is a hand-maintained second list of names that must all be
    /// `Kind::Special` entries in [`TABLE`] — otherwise a print-state name could drift out of the
    /// dispatch table while [`is_print_state_special`] still claimed it. Guards that subset relation.
    #[test]
    fn print_state_specials_are_a_subset_of_table_specials() {
        for name in PRINT_STATE_SPECIALS {
            assert!(
                matches!(super::lookup(name), Some(Kind::Special)),
                "PRINT_STATE_SPECIALS name `{name}` is not a Kind::Special in TABLE"
            );
        }
    }

    /// Every function implemented in the eval [`TABLE`] must also appear in the hand-maintained
    /// `NAME_FUNCID` type table (`types_table.rs`), which drives return-type / argument validation.
    /// The two tables are maintained separately, so this guards against a new builtin being wired
    /// into the evaluator while its type-system entry is forgotten (or vice versa).
    #[test]
    fn implemented_funcs_are_in_name_funcid() {
        use std::collections::HashSet;
        // The date/time *constructors* are handled specially by the type system (as type
        // constructors, not funcID-dispatched functions), so they are legitimately absent from the
        // funcID table. Every other implemented function must be present.
        const CONSTRUCTOR_EXCEPTIONS: &[&str] = &["date", "datetime", "time"];
        let typed: HashSet<&str> = crate::types::NAME_FUNCID.iter().map(|(n, _)| *n).collect();
        for (name, kind) in TABLE {
            if matches!(kind, Kind::Func(..)) && !CONSTRUCTOR_EXCEPTIONS.contains(name) {
                assert!(
                    typed.contains(name),
                    "builtin `{name}` is implemented in eval::builtins TABLE but is missing from the \
                     NAME_FUNCID table (types_table.rs) — its return type / arg validation \
                     would be unknown (add it there, or to CONSTRUCTOR_EXCEPTIONS if it is a special)"
                );
            }
        }
    }

    /// The non-`Func` names in [`TABLE`] — the print/record-state specials, markers, record-nav
    /// functions and colour constants — also produce a value, so `deduce_type` needs a `NAME_FUNCID`
    /// entry to infer each one's result kind (an absent name deduces as `Unknown`). `NAME_FUNCID`
    /// mirrors the engine's function funcID space exactly; the names in `KNOWN_ABSENT` below are
    /// **structurally** outside it, not merely unmapped: `crcyan`/`crmagenta`/`rowcounter`/
    /// `timestring` do not exist in the engine's lexer at all (crystal-formula supports them as a
    /// superset, dispatched by name), and `next`/`nextvalue`/`previous`/`previousvalue` are prefix
    /// operators in the engine's reserved-word namespace, not funcID-carrying functions — so no
    /// `NAME_FUNCID` entry is possible without corrupting the funcID index. They still evaluate
    /// correctly (`dispatch` handles them directly); only their deduced type is `Unknown`. Typing the
    /// record-nav operators would need arg-type propagation, not a funcID entry. This test locks the
    /// invariant from both sides: no *other* non-`Func` name may silently fall out of `NAME_FUNCID`,
    /// and any `KNOWN_ABSENT` name that ever gains a funcID entry must be removed from the list.
    #[test]
    fn non_func_names_funcid_membership_is_pinned() {
        use std::collections::HashSet;
        // Structurally absent from the engine funcID space NAME_FUNCID models (not an unmapped ID).
        const KNOWN_ABSENT: &[&str] = &[
            "crcyan",
            "crmagenta",
            "next",
            "nextvalue",
            "previous",
            "previousvalue",
            "rowcounter",
            "timestring",
        ];
        let typed: HashSet<&str> = crate::types::NAME_FUNCID.iter().map(|(n, _)| *n).collect();
        for (name, kind) in TABLE {
            if matches!(kind, Kind::Func(..)) {
                continue; // covered by `implemented_funcs_are_in_name_funcid`
            }
            let present = typed.contains(name);
            if KNOWN_ABSENT.contains(name) {
                assert!(
                    !present,
                    "`{name}` is now in NAME_FUNCID — remove it from KNOWN_ABSENT"
                );
            } else {
                assert!(
                    present,
                    "non-Func name `{name}` drifted out of NAME_FUNCID (its deduced type would be \
                     Unknown); add its funcID, or add it to KNOWN_ABSENT if the funcID is unknown"
                );
            }
        }
    }

    /// Every dispatched eager builtin ([`Kind::Func`]) must carry a real argument-count bound in the
    /// signature table ([`crate::types::sig`]) — a `Sig::Any` would let a wrong-arity call slip past
    /// the [`dispatch`](super::dispatch) arity gate. The date/time *constructors* have no funcID (see
    /// `implemented_funcs_are_in_name_funcid`), so there is nothing to key a signature on; they are
    /// excluded, matching that test's `CONSTRUCTOR_EXCEPTIONS`.
    #[test]
    fn dispatched_funcs_have_a_bounded_signature() {
        const CONSTRUCTOR_EXCEPTIONS: &[&str] = &["date", "datetime", "time"];
        for (name, kind) in TABLE {
            if !matches!(kind, Kind::Func(..)) || CONSTRUCTOR_EXCEPTIONS.contains(name) {
                continue;
            }
            let id = crate::types::func_id(name)
                .unwrap_or_else(|| panic!("dispatched builtin `{name}` has no funcID"));
            assert!(
                !matches!(crate::types::sig(id), crate::types::Sig::Any),
                "dispatched builtin `{name}` (funcID {id}) has an unbounded Sig::Any signature — \
                 add its (min, max) to `sig` in types_table.rs"
            );
        }
    }

    /// Every `cr*` constant the engine knows (`NAME_FUNCID`) must be dispatchable — a colour
    /// ([`Kind::Color`]), the `crpi` alias, or a bare [`Kind::Const`] — so no conditional-format
    /// formula returning a `cr*` constant falls through to [`EvalError::Unsupported`].
    #[test]
    fn all_cr_constants_are_dispatchable() {
        for (name, _) in crate::types::NAME_FUNCID {
            if name.starts_with("cr") {
                assert!(
                    super::lookup(name).is_some(),
                    "cr* constant `{name}` is in NAME_FUNCID but not dispatched (Unsupported)"
                );
            }
        }
    }

    /// Each bare `cr*` [`Kind::Const`] carries its engine funcID as its value; lock that invariant so
    /// a hand-typed constant cannot silently diverge from `NAME_FUNCID`.
    #[test]
    fn const_values_are_funcids() {
        use std::collections::HashMap;
        let funcid: HashMap<&str, u16> = crate::types::NAME_FUNCID.iter().copied().collect();
        for (name, kind) in TABLE {
            if let Kind::Const(v) = kind {
                assert_eq!(
                    Some(*v as u16),
                    funcid.get(name).copied(),
                    "Const `{name}` value {v} does not match its NAME_FUNCID entry"
                );
            }
        }
    }

    /// Every relative-date-range constant (the `NAME_FUNCID` funcID block 197..=223) dispatches as a
    /// [`Kind::DateRange`], so a record-selection formula using one evaluates to a range rather than
    /// failing as unsupported.
    #[test]
    fn all_date_range_constants_are_dispatchable() {
        use std::collections::HashMap;
        let funcid: HashMap<&str, u16> = crate::types::NAME_FUNCID.iter().copied().collect();
        let mut count = 0;
        for (name, kind) in TABLE {
            if funcid.get(name).is_some_and(|id| (197..=223).contains(id)) {
                assert!(
                    matches!(kind, Kind::DateRange(_)),
                    "date-range constant `{name}` is not a Kind::DateRange"
                );
                count += 1;
            }
        }
        assert_eq!(count, 27, "expected 27 date-range constants, found {count}");
    }

    /// A date-range constant evaluates to an inclusive date [`Value::Range`] against the context's
    /// `CurrentDate`, rather than erroring.
    #[test]
    fn date_range_evaluates_against_current_date() {
        use crate::eval::{Date, MapContext, Value};
        let ctx =
            MapContext::default().with_special("currentdate", Value::Date(Date::new(2024, 3, 15)));
        let v = super::dispatch("yeartodate", &[], &ctx)
            .expect("dispatched")
            .expect("evaluated");
        match v {
            Value::Range {
                lo,
                hi,
                lo_incl,
                hi_incl,
            } => {
                assert_eq!(*lo, Value::Date(Date::new(2024, 1, 1)));
                assert_eq!(*hi, Value::Date(Date::new(2024, 3, 15)));
                assert!(lo_incl && hi_incl);
            }
            other => panic!("expected Range, got {other:?}"),
        }
    }
}
