//! Distribution of one field over a set of events.

use super::util::{percentile_sorted, sorted};
use crate::row::RowRef;
use serde::Serialize;
use std::collections::HashMap;

/// Distinct values tracked before further new values are ignored.
pub const MAX_DISTINCT: usize = 10_000;
/// Most frequent values listed.
pub const TOP: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueCount {
    pub value: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistogramBin {
    pub from: f64,
    pub to: f64,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NumericStats {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub stddev: f64,
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
    pub histogram: Vec<HistogramBin>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldStats {
    /// Events that carry the field.
    pub count: u64,
    /// Events without it.
    pub missing: u64,
    /// Distinct values, capped at [`MAX_DISTINCT`].
    pub distinct: usize,
    /// Present only when every value is a number.
    pub numeric: Option<NumericStats>,
    pub top: Vec<ValueCount>,
}

/// Statistics of `field` (a field name or `@pseudo` field; `@source` is empty
/// here because no source names are available).
pub fn field_stats<R: RowRef>(records: impl IntoIterator<Item = R>, field: &str, buckets: usize) -> FieldStats {
    let (mut count, mut missing) = (0u64, 0u64);
    let mut values: HashMap<String, u64> = HashMap::new();
    let mut numbers: Vec<f64> = Vec::new();
    for r in records {
        let Some(text) = r.text_of(field) else {
            missing += 1;
            continue;
        };
        count += 1;
        if let Some(n) = r.number_of(field) {
            numbers.push(n);
        }
        if let Some(c) = values.get_mut(text.as_ref()) {
            *c += 1;
        } else if values.len() < MAX_DISTINCT {
            values.insert(text.into_owned(), 1);
        }
    }
    let numeric = (count > 0 && numbers.len() as u64 == count).then(|| numeric_stats(&numbers, buckets));
    let distinct = values.len();
    let mut top: Vec<ValueCount> = values.into_iter().map(|(value, count)| ValueCount { value, count }).collect();
    top.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
    top.truncate(TOP);
    FieldStats {
        count,
        missing,
        distinct,
        numeric,
        top,
    }
}

fn numeric_stats(numbers: &[f64], buckets: usize) -> NumericStats {
    let v = sorted(numbers);
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    NumericStats {
        min: v[0],
        max: v[v.len() - 1],
        mean,
        stddev: var.sqrt(),
        p50: percentile_sorted(&v, 50.0),
        p90: percentile_sorted(&v, 90.0),
        p95: percentile_sorted(&v, 95.0),
        p99: percentile_sorted(&v, 99.0),
        histogram: histogram(&v, buckets.max(1)),
    }
}

/// Smallest 1/2/5 x 10^k step that is at least `raw`.
fn nice_step(raw: f64) -> f64 {
    let mag = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .iter()
        .map(|m| m * mag)
        .find(|s| *s >= raw * (1.0 - 1e-12))
        .unwrap_or(10.0 * mag)
}

/// The bins of a histogram over `min..=max` with round edges and at most `buckets` bins: `(start, step,
/// bins)`, or `None` when all values are equal (one bin). A value `x` falls in bin
/// `((x - start) / step) as usize` (at most `bins - 1`). Shared with stores that count bins in SQL.
pub fn bin_layout(min: f64, max: f64, buckets: usize) -> Option<(f64, f64, usize)> {
    if max == min {
        return None;
    }
    let buckets = buckets.max(1);
    let mut step = nice_step((max - min) / buckets as f64);
    loop {
        let start = (min / step).floor() * step;
        let bins = ((max - start) / step).floor() as usize + 1;
        if bins <= buckets {
            return Some((start, step, bins));
        }
        step = nice_step(step * 1.01);
    }
}

/// Histogram of a sorted slice with round bin edges and at most `buckets` bins.
fn histogram(v: &[f64], buckets: usize) -> Vec<HistogramBin> {
    let (min, max) = (v[0], v[v.len() - 1]);
    let Some((start, step, bins)) = bin_layout(min, max, buckets) else {
        return vec![HistogramBin {
            from: min,
            to: max,
            count: v.len() as u64,
        }];
    };
    let mut out: Vec<HistogramBin> = (0..bins)
        .map(|i| HistogramBin {
            from: start + i as f64 * step,
            to: start + (i + 1) as f64 * step,
            count: 0,
        })
        .collect();
    for x in v {
        let i = (((x - start) / step) as usize).min(bins - 1);
        out[i].count += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::bin_layout;

    #[test]
    fn bins_have_round_edges_and_cover_the_range() {
        assert_eq!(bin_layout(3.0, 3.0, 10), None);
        assert_eq!(bin_layout(0.0, 95.0, 10), Some((0.0, 10.0, 10)));
        let (start, step, bins) = bin_layout(13.0, 987.0, 20).unwrap();
        assert!(
            start <= 13.0 && start + step * bins as f64 > 987.0 && bins <= 20,
            "{start} {step} {bins}"
        );
        // Zero buckets ask for one.
        assert_eq!(bin_layout(0.0, 1.0, 0).map(|b| b.2), Some(1));
    }
}
