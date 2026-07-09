//! Runtime [`Value`] model for formula evaluation.
//!
//! Calendar types ([`Date`]/[`Time`]) and all value→string formatting come from
//! [`rpt_format_value`] — there is one calendar and one formatter across the workspace. This module
//! adds the tagged [`Value`] union, its arithmetic-facing accessors, and the *default* text
//! coercion used by `&`/bare `ToText` (format-spec-driven formatting for placed fields is the
//! layout engine's job, using the same [`rpt_format_value`] functions with real specs).

use rpt_format_value::{
    format_bool, format_currency, format_date, format_datetime, format_time, BoolFormat,
    CurrencyFormat, DateFormat, NumberFormat, TimeFormat,
};
use std::fmt;

pub use rpt_format_value::{format_number, Date, Time};

/// A runtime formula value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A number.
    Number(f64),
    /// A currency amount (distinct type; formats with the currency symbol).
    Currency(f64),
    /// A string.
    Str(String),
    /// A boolean.
    Bool(bool),
    /// A date.
    Date(Date),
    /// A time of day.
    Time(Time),
    /// A combined date and time.
    DateTime(Date, Time),
    /// An array of values.
    Array(Vec<Value>),
    /// A `To` range. The `_` variants of the operator exclude the marked bound.
    Range {
        /// The low bound.
        lo: Box<Value>,
        /// The high bound.
        hi: Box<Value>,
        /// Whether the low bound is included.
        lo_incl: bool,
        /// Whether the high bound is included.
        hi_incl: bool,
    },
    /// A null database/parameter value. Propagates through operators and most builtins
    /// (the engine's "convert nulls" options are a later, per-report concern).
    Null,
}

impl Value {
    /// The value's type name, for diagnostics.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Number(_) => "Number",
            Value::Currency(_) => "Currency",
            Value::Str(_) => "String",
            Value::Bool(_) => "Boolean",
            Value::Date(_) => "Date",
            Value::Time(_) => "Time",
            Value::DateTime(..) => "DateTime",
            Value::Array(_) => "Array",
            Value::Range { .. } => "Range",
            Value::Null => "Null",
        }
    }

    /// Whether this is the [`Value::Null`] value.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Numeric view of a Number/Currency.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) | Value::Currency(n) => Some(*n),
            _ => None,
        }
    }

    /// The default text form Crystal uses when an operand is coerced to text (`&`, bare `ToText`):
    /// numbers get grouped 2-decimal en-US formatting, dates/times the default patterns. Returns
    /// `None` for the non-coercible aggregate kinds (Array/Range).
    pub fn to_text_default(&self) -> Option<String> {
        Some(match self {
            Value::Str(s) => s.clone(),
            Value::Number(n) => format_number(*n, &NumberFormat::default()),
            Value::Currency(n) => format_currency(*n, &CurrencyFormat::default()),
            Value::Bool(b) => format_bool(*b, &BoolFormat::default()),
            Value::Date(d) => format_date(*d, &DateFormat::default()),
            Value::Time(t) => format_time(*t, &TimeFormat::default()),
            Value::DateTime(d, t) => {
                format_datetime(*d, *t, &DateFormat::default(), &TimeFormat::default())
            }
            Value::Null => String::new(),
            Value::Array(_) | Value::Range { .. } => return None,
        })
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_text_default() {
            Some(s) => f.write_str(&s),
            None => write!(f, "<{}>", self.type_name()),
        }
    }
}
