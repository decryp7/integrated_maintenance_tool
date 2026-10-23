//! Chart definitions and time-bucketed aggregation.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartDef {
    pub id: String,
    pub title: String,
    pub y: String,
    #[serde(default = "auto")]
    pub bucket: String,
    #[serde(default)]
    pub split_by: Vec<String>,
    #[serde(rename = "type", default = "line")]
    pub kind: String,
    #[serde(default)]
    pub unit: String,
}

fn auto() -> String {
    "auto".into()
}
fn line() -> String {
    "line".into()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Agg {
    Count,
    Avg,
    Min,
    Max,
    Sum,
    Last,
}

/// Parses "count()", "avg(cpu)", ... into the aggregate and its field.
pub fn parse_y(y: &str) -> Result<(Agg, Option<String>)> {
    let y = y.trim();
    let (name, arg) = y
        .strip_suffix(')')
        .and_then(|s| s.split_once('('))
        .ok_or_else(|| anyhow!("expected count(), avg(field), min(field), max(field), sum(field) or last(field), got \"{y}\""))?;
    let agg = match name.trim().to_ascii_lowercase().as_str() {
        "count" => Agg::Count,
        "avg" | "mean" => Agg::Avg,
        "min" => Agg::Min,
        "max" => Agg::Max,
        "sum" => Agg::Sum,
        "last" => Agg::Last,
        other => return Err(anyhow!("unknown aggregate \"{other}\"")),
    };
    let arg = arg.trim();
    match (agg, arg.is_empty()) {
        (Agg::Count, _) => Ok((agg, None)),
        (_, true) => Err(anyhow!("{name}() needs a field, e.g. {name}(cpu)")),
        (_, false) => Ok((agg, Some(arg.to_string()))),
    }
}

/// "auto" -> None, otherwise a duration like 30s, 1m, 5m, 1h, 1d in ms.
pub fn parse_bucket(b: &str) -> Result<Option<i64>> {
    let b = b.trim();
    if b.is_empty() || b.eq_ignore_ascii_case("auto") {
        return Ok(None);
    }
    let idx = b
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| anyhow!("bucket \"{b}\" needs a unit (s, m, h, d)"))?;
    let n: i64 = b[..idx].parse().map_err(|_| anyhow!("invalid bucket \"{b}\""))?;
    let unit = match &b[idx..] {
        "ms" => 1,
        "s" => 1000,
        "m" | "min" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        other => return Err(anyhow!("unknown bucket unit \"{other}\"")),
    };
    if n <= 0 {
        return Err(anyhow!("bucket must be positive"));
    }
    Ok(Some(n * unit))
}

/// Picks a round bucket size so the range splits into at most `max` buckets.
pub fn nice_bucket(range_ms: i64, max: usize) -> i64 {
    const STEPS: [i64; 22] = [
        1, 5, 10, 50, 100, 250, 500, 1000, 2000, 5000, 10_000, 15_000, 30_000, 60_000, 120_000, 300_000, 600_000, 900_000, 1_800_000,
        3_600_000, 10_800_000, 86_400_000,
    ];
    let target = (range_ms.max(1) as f64 / max.max(1) as f64).ceil() as i64;
    STEPS
        .iter()
        .copied()
        .find(|s| *s >= target)
        .unwrap_or_else(|| (target / 86_400_000 + 1) * 86_400_000)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Acc {
    pub count: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub last: f64,
}

impl Acc {
    pub fn add(&mut self, v: f64) {
        if self.count == 0 {
            self.min = v;
            self.max = v;
        } else {
            self.min = self.min.min(v);
            self.max = self.max.max(v);
        }
        self.count += 1;
        self.sum += v;
        self.last = v;
    }

    pub fn merge(&mut self, o: &Acc) {
        if o.count == 0 {
            return;
        }
        if self.count == 0 {
            *self = *o;
            return;
        }
        self.count += o.count;
        self.sum += o.sum;
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
        self.last = o.last;
    }

    pub fn value(&self, agg: Agg) -> Option<f64> {
        if self.count == 0 {
            return None;
        }
        Some(match agg {
            Agg::Count => self.count as f64,
            Agg::Avg => self.sum / self.count as f64,
            Agg::Min => self.min,
            Agg::Max => self.max,
            Agg::Sum => self.sum,
            Agg::Last => self.last,
        })
    }
}

/// The profile's charts (`[[chart]]`), with ids (`chart-N` when the profile names none).
pub fn chart_defs(profile: &crate::profile::Profile) -> Vec<ChartDef> {
    profile
        .chart
        .iter()
        .enumerate()
        .map(|(i, c)| ChartDef {
            id: c.id.clone().unwrap_or_else(|| format!("chart-{}", i + 1)),
            title: c.title.clone(),
            y: c.y.clone(),
            bucket: c.bucket.clone(),
            split_by: c.split_by.clone(),
            kind: c.kind.clone(),
            unit: c.unit.clone(),
        })
        .collect()
}

/// Series kept per chart; the rest fold into "Other".
pub const MAX_SERIES: usize = 8;

/// The time buckets of a chart over `from..=to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Buckets {
    pub start: i64,
    pub bucket_ms: i64,
    pub n: usize,
}

impl Buckets {
    /// Buckets of `def` (its fixed bucket, kept to at most 5,000 buckets, or a nice one for `buckets`).
    pub fn of(def: &ChartDef, from: i64, to: i64, buckets: usize) -> Result<Buckets> {
        let span = (to - from).max(1);
        let bucket_ms = match parse_bucket(&def.bucket)? {
            Some(b) => b.max(span / 5000),
            None => nice_bucket(span, buckets.clamp(10, 2000)),
        };
        let start = from - from.rem_euclid(bucket_ms);
        Ok(Buckets {
            start,
            bucket_ms,
            n: ((to - start) / bucket_ms + 1) as usize,
        })
    }

    fn index(&self, ts: i64) -> Option<usize> {
        let b = (ts - self.start) / self.bucket_ms;
        (b >= 0 && (b as usize) < self.n).then_some(b as usize)
    }
}

/// Aggregates the values of a chart per series and bucket.
pub struct SeriesBuilder {
    buckets: Buckets,
    series: std::collections::HashMap<String, Vec<Acc>>,
    totals: std::collections::HashMap<String, Acc>,
}

impl SeriesBuilder {
    pub fn new(buckets: Buckets) -> SeriesBuilder {
        SeriesBuilder {
            buckets,
            series: Default::default(),
            totals: Default::default(),
        }
    }

    /// Adds one value of series `key` at `ts` (outside the buckets: ignored).
    pub fn add(&mut self, ts: i64, value: f64, key: String) {
        let Some(b) = self.buckets.index(ts) else { return };
        let n = self.buckets.n;
        self.series.entry(key.clone()).or_insert_with(|| vec![Acc::default(); n])[b].add(value);
        self.totals.entry(key).or_default().add(value);
    }

    /// Merges an aggregate computed elsewhere (e.g. by SQL) into bucket `bucket` of series `key`.
    pub fn add_acc(&mut self, bucket: usize, key: String, acc: &Acc) {
        if bucket >= self.buckets.n {
            return;
        }
        let n = self.buckets.n;
        self.series.entry(key.clone()).or_insert_with(|| vec![Acc::default(); n])[bucket].merge(acc);
        self.totals.entry(key).or_default().merge(acc);
    }

    /// The biggest series (levels in severity order when split by `@level`), the rest as "Other".
    pub fn finish(mut self, def: &ChartDef, agg: Agg) -> Vec<crate::api::ChartSeries> {
        let n = self.buckets.n;
        let mut keys: Vec<String> = self.series.keys().cloned().collect();
        if def.split_by.len() == 1 && def.split_by[0] == "@level" {
            // By level; values that are not levels after them, by name (not in the map's order).
            keys.sort_by(|a, b| {
                let rank = |k: &str| crate::model::Level::parse(k).map_or(99, |l| l.index());
                rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
            });
        } else {
            let totals = &self.totals;
            keys.sort_by(|a, b| totals[b].count.cmp(&totals[a].count).then_with(|| a.cmp(b)));
        }
        let total_series = self.totals.len();
        let mut out = Vec::new();
        let mut other: Option<Vec<Acc>> = None;
        for (i, k) in keys.into_iter().enumerate() {
            let Some(accs) = self.series.remove(&k) else { continue };
            if i < MAX_SERIES - 1 || total_series <= MAX_SERIES {
                out.push((k, accs));
            } else {
                let o = other.get_or_insert_with(|| vec![Acc::default(); n]);
                for (a, b) in o.iter_mut().zip(&accs) {
                    a.merge(b);
                }
            }
        }
        if let Some(o) = other {
            out.push(("Other".into(), o));
        }
        let Buckets { start, bucket_ms, .. } = self.buckets;
        out.into_iter()
            .map(|(key, accs)| crate::api::ChartSeries {
                key,
                points: accs
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        (
                            start + i as i64 * bucket_ms,
                            a.value(agg).or(if agg == Agg::Count { Some(0.0) } else { None }),
                        )
                    })
                    .collect(),
            })
            .collect()
    }
}

/// The value a chart aggregates for one event: 1 for `count()`, else the field's number (`None`: the
/// event is skipped).
pub fn chart_value(row: &dyn crate::row::RowRef, field: Option<&str>) -> Option<f64> {
    match field {
        None => Some(1.0),
        Some(f) => row.number_of(f),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_computed_elsewhere_merge_into_their_bucket_and_out_of_range_ones_are_dropped() {
        let def = ChartDef {
            id: "c".into(),
            title: "t".into(),
            y: "sum(Took)".into(),
            bucket: "1s".into(),
            split_by: vec!["Host".into()],
            kind: "line".into(),
            unit: String::new(),
        };
        let buckets = Buckets {
            start: 0,
            bucket_ms: 1_000,
            n: 3,
        };
        let mut b = SeriesBuilder::new(buckets);
        let mut acc = Acc::default();
        acc.add(2.0);
        acc.add(4.0);
        b.add_acc(1, "web".into(), &acc);
        b.add_acc(1, "web".into(), &acc);
        b.add_acc(3, "web".into(), &acc); // past the last bucket
        b.add_acc(usize::MAX, "db".into(), &acc);
        b.add(1_500, 1.0, "web".into());
        b.add(9_999, 1.0, "web".into()); // outside the buckets: ignored
        let series = b.finish(&def, Agg::Sum);
        assert_eq!(series.len(), 1, "series that only had out-of-range values do not exist: {series:?}");
        assert_eq!(series[0].key, "web");
        assert_eq!(series[0].points.iter().map(|p| p.1).collect::<Vec<_>>(), [None, Some(13.0), None]);
    }

    #[test]
    fn level_series_are_ordered_by_level_then_name() {
        let def = ChartDef {
            id: "c".into(),
            title: "t".into(),
            y: "count()".into(),
            bucket: "1s".into(),
            split_by: vec!["@level".into()],
            kind: "line".into(),
            unit: String::new(),
        };
        let buckets = Buckets::of(&def, 0, 1_000, 10).unwrap();
        // Many keys, so the map's own order would show.
        let names: Vec<String> = (0..6).map(|i| format!("x{i}")).collect();
        for _ in 0..3 {
            let mut b = SeriesBuilder::new(buckets);
            for k in names.iter().rev().cloned().chain(["Error".to_string(), "Information".to_string()]) {
                b.add(0, 1.0, k);
            }
            let keys: Vec<String> = b.finish(&def, Agg::Count).into_iter().map(|s| s.key).collect();
            assert_eq!(keys[..2], ["Information".to_string(), "Error".to_string()]);
            let mut rest = keys[2..].to_vec();
            rest.sort();
            assert_eq!(keys[2..], rest[..], "values that are not levels come by name");
        }
    }

    #[test]
    fn parses() {
        assert_eq!(parse_y("count()").unwrap(), (Agg::Count, None));
        assert_eq!(parse_y("avg(cpu)").unwrap(), (Agg::Avg, Some("cpu".into())));
        assert!(parse_y("avg()").is_err());
        assert_eq!(parse_bucket("5m").unwrap(), Some(300_000));
        assert_eq!(parse_bucket("auto").unwrap(), None);
        assert_eq!(nice_bucket(3 * 3_600_000, 200), 60_000);
    }

    #[test]
    fn parse_errors_and_units() {
        assert_eq!(parse_y("sum(x)").unwrap().0, Agg::Sum);
        assert_eq!(parse_y("last(x)").unwrap().0, Agg::Last);
        assert_eq!(parse_y("mean(x)").unwrap().0, Agg::Avg);
        assert_eq!(parse_y("min(x)").unwrap().0, Agg::Min);
        assert_eq!(parse_y("MAX( x )").unwrap(), (Agg::Max, Some("x".into())));
        assert!(parse_y("median(x)").is_err());
        assert!(parse_y("cpu").is_err());
        assert_eq!(parse_bucket("250ms").unwrap(), Some(250));
        assert_eq!(parse_bucket("30s").unwrap(), Some(30_000));
        assert_eq!(parse_bucket("2min").unwrap(), Some(120_000));
        assert_eq!(parse_bucket("1h").unwrap(), Some(3_600_000));
        assert_eq!(parse_bucket("1d").unwrap(), Some(86_400_000));
        assert_eq!(parse_bucket("").unwrap(), None);
        assert!(parse_bucket("5").is_err());
        assert!(parse_bucket("5w").is_err());
        assert!(parse_bucket("0s").is_err());
        assert_eq!(nice_bucket(10, 100), 1);
        assert_eq!(nice_bucket(400 * 86_400_000, 10), 41 * 86_400_000);
    }

    #[test]
    fn accumulates() {
        let mut a = Acc::default();
        assert_eq!(a.value(Agg::Avg), None);
        for v in [3.0, 1.0, 2.0] {
            a.add(v);
        }
        assert_eq!(a.value(Agg::Count), Some(3.0));
        assert_eq!(a.value(Agg::Avg), Some(2.0));
        assert_eq!(a.value(Agg::Min), Some(1.0));
        assert_eq!(a.value(Agg::Max), Some(3.0));
        assert_eq!(a.value(Agg::Sum), Some(6.0));
        assert_eq!(a.value(Agg::Last), Some(2.0));

        let mut b = Acc::default();
        b.merge(&Acc::default());
        assert_eq!(b.count, 0);
        b.merge(&a);
        assert_eq!(b.value(Agg::Sum), Some(6.0));
        let mut c = Acc::default();
        c.add(10.0);
        c.add(-1.0);
        b.merge(&c);
        assert_eq!(b.value(Agg::Count), Some(5.0));
        assert_eq!(b.value(Agg::Min), Some(-1.0));
        assert_eq!(b.value(Agg::Max), Some(10.0));
        assert_eq!(b.value(Agg::Last), Some(-1.0));
    }

    #[test]
    fn chart_def_defaults() {
        let d: ChartDef = serde_json::from_str(r#"{"id":"a","title":"A","y":"count()"}"#).unwrap();
        assert_eq!(d.bucket, "auto");
        assert_eq!(d.kind, "line");
        assert!(d.split_by.is_empty());
    }
}
