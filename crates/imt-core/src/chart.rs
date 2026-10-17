//! Chart definitions and time-bucketed aggregation.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::*;

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
