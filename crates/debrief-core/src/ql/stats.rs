//! `stats`: grouping and aggregation over any [`Row`] source.

use super::error::QlError;
use super::expr::Row;
use super::parser::{AggFn, AggSpec, FieldRef};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Percentiles are exact, so every value is kept; beyond this many per group the query fails.
pub const MAX_PERCENTILE_VALUES: usize = 1_000_000;

enum Acc {
    Count(u64),
    Distinct(HashSet<String>),
    Num(NumAcc),
}

/// Aggregates over the numeric values of a field.
enum NumAcc {
    Sum(f64),
    Avg(f64, u64),
    Min(Option<f64>),
    Max(Option<f64>),
    Percentile(f64, Vec<f64>),
}

impl NumAcc {
    /// Returns false when a limit was exceeded.
    fn add(&mut self, v: f64) -> bool {
        match self {
            NumAcc::Sum(s) => *s += v,
            NumAcc::Avg(s, n) => {
                *s += v;
                *n += 1;
            }
            NumAcc::Min(m) => *m = Some(m.map_or(v, |c| c.min(v))),
            NumAcc::Max(m) => *m = Some(m.map_or(v, |c| c.max(v))),
            NumAcc::Percentile(_, vals) => {
                if vals.len() >= MAX_PERCENTILE_VALUES {
                    return false;
                }
                vals.push(v);
            }
        }
        true
    }

    fn finish(self) -> Value {
        match self {
            NumAcc::Sum(s) => json_number(s),
            NumAcc::Avg(_, 0) | NumAcc::Min(None) | NumAcc::Max(None) => Value::Null,
            NumAcc::Avg(s, n) => json_number(s / n as f64),
            NumAcc::Min(Some(v)) | NumAcc::Max(Some(v)) => json_number(v),
            NumAcc::Percentile(p, mut vals) => {
                if vals.is_empty() {
                    return Value::Null;
                }
                vals.sort_by(f64::total_cmp);
                let rank = ((p / 100.0) * vals.len() as f64).ceil() as usize;
                json_number(vals[rank.clamp(1, vals.len()) - 1])
            }
        }
    }
}

impl Acc {
    fn new(func: AggFn) -> Acc {
        match func {
            AggFn::Count => Acc::Count(0),
            AggFn::CountDistinct => Acc::Distinct(HashSet::new()),
            AggFn::Sum => Acc::Num(NumAcc::Sum(0.0)),
            AggFn::Avg => Acc::Num(NumAcc::Avg(0.0, 0)),
            AggFn::Min => Acc::Num(NumAcc::Min(None)),
            AggFn::Max => Acc::Num(NumAcc::Max(None)),
            AggFn::Percentile(p) => Acc::Num(NumAcc::Percentile(p, Vec::new())),
        }
    }

    /// Returns false when a limit was exceeded.
    fn feed(&mut self, row: &dyn Row, field: Option<&FieldRef>) -> bool {
        let key = field.map(|f| &f.key);
        match self {
            Acc::Count(n) => *n += 1,
            Acc::Distinct(set) => {
                if let Some(t) = key.and_then(|k| row.text(k)) {
                    set.insert(t.into_owned());
                }
            }
            Acc::Num(acc) => return key.and_then(|k| row.number(k)).is_none_or(|v| acc.add(v)),
        }
        true
    }

    fn finish(self) -> Value {
        match self {
            Acc::Count(n) => Value::from(n),
            Acc::Distinct(s) => Value::from(s.len()),
            Acc::Num(acc) => acc.finish(),
        }
    }
}

/// Whole numbers become JSON integers, everything else a float (NaN/inf become null): how `stats` writes
/// its numbers (stores that aggregate in SQL use it too).
pub fn json_number(f: f64) -> Value {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        Value::from(f as i64)
    } else {
        serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
    }
}

struct Group {
    keys: Vec<Value>,
    accs: Vec<Acc>,
}

/// Groups `rows` by `by` and evaluates `aggs`; returns column names and rows in first-seen group order.
pub fn aggregate<R: Row>(
    rows: impl Iterator<Item = R>,
    aggs: &[AggSpec],
    by: &[FieldRef],
) -> Result<(Vec<String>, Vec<Vec<Value>>), QlError> {
    let new_group = |keys: Vec<Value>| Group {
        keys,
        accs: aggs.iter().map(|a| Acc::new(a.func)).collect(),
    };
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut groups: Vec<Group> = Vec::new();
    for row in rows {
        let keys: Vec<Value> = by.iter().map(|f| row.value(&f.key).unwrap_or(Value::Null)).collect();
        let id = keys.iter().map(Value::to_string).collect::<Vec<_>>().join("\u{1f}");
        let slot = *index.entry(id).or_insert_with(|| {
            groups.push(new_group(keys));
            groups.len() - 1
        });
        for (acc, spec) in groups[slot].accs.iter_mut().zip(aggs) {
            if !acc.feed(&row, spec.field.as_ref()) {
                return Err(QlError::new(
                    format!("more than {MAX_PERCENTILE_VALUES} values for an exact percentile; narrow the filter first"),
                    spec.offset,
                    spec.len,
                ));
            }
        }
    }
    if by.is_empty() && groups.is_empty() {
        groups.push(new_group(Vec::new()));
    }
    let columns = by
        .iter()
        .map(|f| f.label.clone())
        .chain(aggs.iter().map(|a| a.column.clone()))
        .collect();
    let rows = groups
        .into_iter()
        .map(|g| g.keys.into_iter().chain(g.accs.into_iter().map(Acc::finish)).collect())
        .collect();
    Ok((columns, rows))
}
