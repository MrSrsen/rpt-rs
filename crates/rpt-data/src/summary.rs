//! One incremental summary reducer shared by the batch group aggregate ([`crate::pipeline`]), the
//! running totals ([`crate::running_total`]), and the cross-tab pivot cells (`rpt-layout`). Folding
//! the same way everywhere keeps those three paths from silently diverging (they historically differed
//! on `WeightedAvg`, `DistinctCount`, currency, and empty min/max).
//!
//! Parameterized / order-statistic operations (percentile, median, Nth, variance, standard deviation)
//! need the full value set at once, so [`SummaryAccumulator::value`] returns `None` for them and the
//! batch aggregate computes them directly.

use crate::value_order::{compare_values, value_key};
use crystal_formula::eval::Value;
use rpt_model::SummaryOperation;
use std::collections::HashSet;

/// Folds values one at a time and resolves the running result for a [`SummaryOperation`].
#[derive(Debug, Default, Clone)]
pub struct SummaryAccumulator {
    /// Non-null values folded.
    count: u64,
    /// Numeric values folded (the denominator for `Average`).
    numeric_count: u64,
    sum: f64,
    /// Any folded value was a `Currency`, so numeric results carry currency.
    currency: bool,
    min: Option<Value>,
    max: Option<Value>,
    distinct: HashSet<String>,
    last: Option<Value>,
}

impl SummaryAccumulator {
    /// A fresh accumulator with no folded values.
    pub fn new() -> SummaryAccumulator {
        SummaryAccumulator::default()
    }

    /// Fold one value. Nulls are ignored (they count toward no operation).
    pub fn fold(&mut self, v: &Value) {
        if v.is_null() {
            return;
        }
        self.count += 1;
        if let Some(n) = v.as_number() {
            self.numeric_count += 1;
            self.sum += n;
            if matches!(v, Value::Currency(_)) {
                self.currency = true;
            }
        }
        self.distinct.insert(value_key(v));
        self.min = Some(match &self.min {
            Some(m) if compare_values(m, v).is_le() => m.clone(),
            _ => v.clone(),
        });
        self.max = Some(match &self.max {
            Some(m) if compare_values(m, v).is_ge() => m.clone(),
            _ => v.clone(),
        });
        self.last = Some(v.clone());
    }

    /// The result for `op`, or `None` for a parameterized / order-statistic op the batch aggregate
    /// must compute over the whole value set.
    pub fn value(&self, op: SummaryOperation) -> Option<Value> {
        use SummaryOperation as Op;
        let numeric = |n: f64| {
            if self.currency {
                Value::Currency(n)
            } else {
                Value::Number(n)
            }
        };
        let v = match op {
            Op::Count => Value::Number(self.count as f64),
            Op::DistinctCount => Value::Number(self.distinct.len() as f64),
            Op::Sum => {
                if self.numeric_count == 0 {
                    Value::Null
                } else {
                    numeric(self.sum)
                }
            }
            Op::Average => {
                if self.numeric_count == 0 {
                    Value::Null
                } else {
                    numeric(self.sum / self.numeric_count as f64)
                }
            }
            Op::Maximum => self.max.clone().unwrap_or(Value::Null),
            Op::Minimum => self.min.clone().unwrap_or(Value::Null),
            // Two-field summaries: WeightedAvg needs a weight field, Correlation/Covariance a paired
            // field, none of which the single-field summary model carries. They resolve to `Null`
            // (unavailable) rather than silently degrading to a different, plausible-but-wrong
            // operation. Distinct from the batch-only ops below, which return `None` so the batch
            // aggregate computes them from the whole value set.
            Op::WeightedAvg | Op::Correlation | Op::Covariance => Value::Null,
            _ => return None,
        };
        Some(v)
    }

    /// The last non-null value folded (the running-total fallback for an op [`value`](Self::value)
    /// does not compute incrementally).
    pub fn last(&self) -> Value {
        self.last.clone().unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crystal_formula::eval::Date;

    fn fold_all(vals: &[Value]) -> SummaryAccumulator {
        let mut a = SummaryAccumulator::new();
        for v in vals {
            a.fold(v);
        }
        a
    }

    #[test]
    fn counts_distincts_and_ignores_nulls() {
        let a = fold_all(&[
            Value::Number(1.0),
            Value::Null,
            Value::Number(1.0),
            Value::Number(2.0),
        ]);
        assert_eq!(a.value(SummaryOperation::Count), Some(Value::Number(3.0)));
        assert_eq!(
            a.value(SummaryOperation::DistinctCount),
            Some(Value::Number(2.0))
        );
    }

    #[test]
    fn sum_and_average_over_numeric_count() {
        let a = fold_all(&[Value::Number(2.0), Value::Number(4.0), Value::Number(6.0)]);
        assert_eq!(a.value(SummaryOperation::Sum), Some(Value::Number(12.0)));
        assert_eq!(a.value(SummaryOperation::Average), Some(Value::Number(4.0)));
    }

    #[test]
    fn two_field_ops_are_unavailable_not_average() {
        // WeightedAvg / Correlation / Covariance need a second field the single-field summary model
        // does not carry, so they resolve to `Null` (unavailable) — never a silent fall-back to the
        // plain Average (4.0) that the same values would otherwise produce.
        let a = fold_all(&[Value::Number(2.0), Value::Number(4.0), Value::Number(6.0)]);
        assert_eq!(a.value(SummaryOperation::WeightedAvg), Some(Value::Null));
        assert_eq!(a.value(SummaryOperation::Correlation), Some(Value::Null));
        assert_eq!(a.value(SummaryOperation::Covariance), Some(Value::Null));
    }

    #[test]
    fn currency_is_preserved() {
        let a = fold_all(&[Value::Currency(10.0), Value::Currency(20.0)]);
        assert_eq!(a.value(SummaryOperation::Sum), Some(Value::Currency(30.0)));
    }

    #[test]
    fn min_max_are_typed_and_null_when_empty() {
        let a = fold_all(&[
            Value::Date(Date::new(2024, 2, 1)),
            Value::Date(Date::new(2024, 12, 1)),
            Value::Date(Date::new(2024, 6, 1)),
        ]);
        assert_eq!(
            a.value(SummaryOperation::Maximum),
            Some(Value::Date(Date::new(2024, 12, 1)))
        );
        assert_eq!(
            a.value(SummaryOperation::Minimum),
            Some(Value::Date(Date::new(2024, 2, 1)))
        );
        let empty = SummaryAccumulator::new();
        assert_eq!(empty.value(SummaryOperation::Maximum), Some(Value::Null));
    }

    #[test]
    fn parameterized_ops_defer_to_the_batch() {
        let a = fold_all(&[Value::Number(1.0), Value::Number(2.0)]);
        assert_eq!(a.value(SummaryOperation::Median), None);
        assert_eq!(a.value(SummaryOperation::Percentile), None);
    }
}
