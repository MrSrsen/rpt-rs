//! One canonical ordering and key over runtime [`Value`]s, shared by grouping/sort (`pipeline`),
//! the running-total min/max ([`running_total`](crate::running_total)), and the cross-tab pivot key
//! (`rpt-layout`). A single implementation avoids the divergence that once let a running `Max` over
//! dates rank by an unpadded `Debug` string (ranking Feb above Dec).

use crystal_formula::eval::Value;
use std::cmp::Ordering;

/// Total ordering over values for sort / group / running min-max: nulls sort first; temporal and
/// boolean values compare by their natural order; numbers compare numerically; only genuinely mixed
/// types fall back to the canonical key text.
pub fn compare_values(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        (Value::Str(x), Value::Str(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Date(x), Value::Date(y)) => x.cmp(y),
        (Value::Time(x), Value::Time(y)) => x.cmp(y),
        (Value::DateTime(dx, tx), Value::DateTime(dy, ty)) => (dx, tx).cmp(&(dy, ty)),
        _ => match (a.as_number(), b.as_number()) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            _ => value_key(a).cmp(&value_key(b)),
        },
    }
}

/// A stable, type-tagged string key for a value: distinct per type (so `Number(1.0)` and `Str("1")`
/// are different buckets) and order-preserving for temporal values via a fixed-width day/second
/// encoding. Used for group buckets, distinct/mode counting, running-total change detection, and the
/// cross-tab pivot key.
pub fn value_key(v: &Value) -> String {
    match v {
        Value::Str(s) => format!("s:{s}"),
        Value::Number(n) | Value::Currency(n) => format!("n:{n}"),
        Value::Bool(b) => format!("b:{b}"),
        Value::Date(d) => format!("d:{:+011}", d.to_days()),
        Value::Time(t) => format!("t:{:05}", t.to_seconds()),
        Value::DateTime(d, t) => format!("dt:{:+011}:{:05}", d.to_days(), t.to_seconds()),
        Value::Null => "null".to_string(),
        other => format!("o:{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crystal_formula::eval::{Date, Time};

    #[test]
    fn dates_order_chronologically_not_by_debug_string() {
        let feb = Value::Date(Date::new(2024, 2, 1));
        let dec = Value::Date(Date::new(2024, 12, 1));
        // The bug: a Debug/text key ranks "month: 12" < "month: 2", so Dec sorts before Feb.
        assert_eq!(compare_values(&dec, &feb), Ordering::Greater);
        assert_eq!(compare_values(&feb, &dec), Ordering::Less);
        // And the key text is itself order-preserving.
        assert!(value_key(&feb) < value_key(&dec));
    }

    #[test]
    fn datetime_orders_by_date_then_time() {
        let early = Value::DateTime(Date::new(2024, 1, 1), Time::new(8, 0, 0));
        let late = Value::DateTime(Date::new(2024, 1, 1), Time::new(17, 30, 0));
        let next_day = Value::DateTime(Date::new(2024, 1, 2), Time::new(0, 0, 0));
        assert_eq!(compare_values(&early, &late), Ordering::Less);
        assert_eq!(compare_values(&late, &next_day), Ordering::Less);
        assert!(value_key(&early) < value_key(&late));
        assert!(value_key(&late) < value_key(&next_day));
    }

    #[test]
    fn nulls_sort_first_and_types_key_distinctly() {
        assert_eq!(
            compare_values(&Value::Null, &Value::Number(1.0)),
            Ordering::Less
        );
        // A number and a string that look alike are distinct buckets.
        assert_ne!(
            value_key(&Value::Number(1.0)),
            value_key(&Value::Str("1".to_string()))
        );
    }
}
