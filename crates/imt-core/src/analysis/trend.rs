//! Sustained growth of memory-like series (leaks): private bytes, working set,
//! handle counts. A least-squares line over time must fit well (R²), span long
//! enough and grow by a meaningful amount, so sawtooth garbage-collection
//! patterns and noisy flat series are not reported.

use super::anomaly::{Anomaly, AnomalyKind};
use super::util::{fmt_duration, fmt_num};
use crate::row::RowRef;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MB: f64 = 1_048_576.0;
const MAX_SAMPLES: usize = 5;
/// Fewer points than this cannot establish a trend.
const MIN_POINTS: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TrendConfig {
    /// Numeric fields holding megabytes (`...Mb`) or counts (`Handle...`) to examine per instance.
    pub fields: Vec<String>,
    /// PerfMon long-format counters (`Counter`/`Value` records, in bytes or counts) to examine.
    pub counters: Vec<String>,
    /// Shortest span worth judging (ms).
    pub min_duration: i64,
    /// Minimum R² of the linear fit.
    pub min_r2: f64,
    /// Minimum growth relative to the fitted start value (0.2 = 20 %).
    pub min_growth_ratio: f64,
    /// Minimum absolute growth of byte series.
    pub min_growth_bytes: f64,
    /// Minimum absolute growth of count series (handles).
    pub min_growth_count: f64,
}

impl Default for TrendConfig {
    fn default() -> Self {
        TrendConfig {
            fields: ["PrivateMb", "WorkingSetMb", "Handles", "HandleCount"].map(String::from).to_vec(),
            counters: ["Private Bytes", "Working Set - Private", "Handle Count", "Virtual Bytes"]
                .map(String::from)
                .to_vec(),
            min_duration: 10 * 60_000,
            min_r2: 0.8,
            min_growth_ratio: 0.2,
            min_growth_bytes: 50.0 * MB,
            min_growth_count: 500.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Unit {
    Bytes,
    Count,
}

/// One memory-like series of an instance (a growth field) or of a process (a PerfMon counter).
#[derive(Debug, Clone, PartialEq)]
pub struct GrowthSeries {
    /// The instance key (fields) or "machine · process" (counters, see [`counter_place`]).
    pub place: String,
    /// The field or counter name.
    pub name: String,
    /// A PerfMon counter (values in bytes or counts) rather than a growth field (megabytes or counts).
    pub counter: bool,
    /// (timestamp ms, value as logged, record id), sorted by timestamp.
    pub points: Vec<(i64, f64, u32)>,
}

impl GrowthSeries {
    /// The unit, and the factor that turns logged values into it.
    fn unit(&self) -> (Unit, f64) {
        if self.counter {
            (if self.name.ends_with("Count") { Unit::Count } else { Unit::Bytes }, 1.0)
        } else if self.name.to_ascii_lowercase().ends_with("mb") {
            (Unit::Bytes, MB)
        } else {
            (Unit::Count, 1.0)
        }
    }
}

/// Instances as `(key, events sorted or not)`; see [`super::detect`].
pub fn memory_growth<'a, R: RowRef + 'a>(
    instances: impl IntoIterator<Item = (&'a str, &'a [(i64, R)])>,
    cfg: &TrendConfig,
) -> Vec<Anomaly> {
    growth_series(instances, cfg)
        .into_iter()
        .filter_map(|s| judge_series(s, cfg))
        .collect()
}

/// The growth series of the instances' events, ordered by place and name, points sorted by time.
pub(crate) fn growth_series<'a, R: RowRef + 'a>(
    instances: impl IntoIterator<Item = (&'a str, &'a [(i64, R)])>,
    cfg: &TrendConfig,
) -> Vec<GrowthSeries> {
    let mut series: BTreeMap<(String, String), GrowthSeries> = BTreeMap::new();
    for (key, events) in instances {
        for (ts, rec) in events {
            collect_fields(&mut series, key, *ts, rec, cfg);
            collect_counter(&mut series, *ts, rec, cfg);
        }
    }
    series
        .into_values()
        .map(|mut s| {
            s.points.sort_by_key(|p| p.0);
            s
        })
        .collect()
}

fn record_id(rec: &dyn RowRef) -> u32 {
    u32::try_from(rec.id()).unwrap_or(u32::MAX)
}

fn push(series: &mut BTreeMap<(String, String), GrowthSeries>, place: String, name: &str, counter: bool, point: (i64, f64, u32)) {
    series
        .entry((place, name.to_string()))
        .or_insert_with_key(|(place, name)| GrowthSeries {
            place: place.clone(),
            name: name.clone(),
            counter,
            points: Vec::new(),
        })
        .points
        .push(point);
}

fn collect_fields(series: &mut BTreeMap<(String, String), GrowthSeries>, key: &str, ts: i64, rec: &dyn RowRef, cfg: &TrendConfig) {
    for field in &cfg.fields {
        let Some(v) = rec.number_of(field) else { continue };
        push(series, key.to_string(), field, false, (ts, v, record_id(rec)));
    }
}

/// Where a PerfMon counter series belongs: "machine · process", with "#index" for one of several
/// processes of the same name.
pub fn counter_place(machine: &str, instance: &str, index: Option<f64>) -> String {
    match index {
        Some(n) => format!("{machine} · {instance}#{n}"),
        None => format!("{machine} · {instance}"),
    }
}

fn collect_counter(series: &mut BTreeMap<(String, String), GrowthSeries>, ts: i64, rec: &dyn RowRef, cfg: &TrendConfig) {
    let text = |name: &str| rec.text_of(name);
    let (Some(counter), Some(value)) = (text("Counter"), rec.number_of("Value")) else {
        return;
    };
    let counter = counter.as_ref();
    if !cfg.counters.iter().any(|c| c == counter) {
        return;
    }
    let machine = text("MachineName").unwrap_or_default();
    let instance = text("ProcessName").or_else(|| text("Instance")).unwrap_or_default();
    let place = counter_place(&machine, &instance, rec.number_of("InstanceIndex"));
    push(series, place, counter, true, (ts, value, record_id(rec)));
}

struct Fit {
    slope_per_min: f64,
    intercept: f64,
    r2: f64,
}

fn fit(points: &[(i64, f64, u32)]) -> Option<Fit> {
    let t0 = points[0].0;
    let n = points.len() as f64;
    let xs: Vec<f64> = points.iter().map(|p| (p.0 - t0) as f64 / 60_000.0).collect();
    let mean_x = xs.iter().sum::<f64>() / n;
    let mean_y = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = xs.iter().map(|x| (x - mean_x).powi(2)).sum();
    let syy: f64 = points.iter().map(|p| (p.1 - mean_y).powi(2)).sum();
    let sxy: f64 = xs.iter().zip(points).map(|(x, p)| (x - mean_x) * (p.1 - mean_y)).sum();
    if sxx <= 0.0 || syy <= 0.0 {
        return None;
    }
    let slope = sxy / sxx;
    Some(Fit {
        slope_per_min: slope,
        intercept: mean_y - slope * mean_x,
        r2: sxy * sxy / (sxx * syy),
    })
}

/// A [`AnomalyKind::MemoryGrowth`] when the series grows steadily.
pub fn judge_series(mut s: GrowthSeries, cfg: &TrendConfig) -> Option<Anomaly> {
    let (unit, factor) = s.unit();
    if factor != 1.0 {
        for p in &mut s.points {
            p.1 *= factor;
        }
    }
    let (place, name) = (&s.place, &s.name);
    let pts = &s.points;
    if pts.len() < MIN_POINTS {
        return None;
    }
    let (from, to) = (pts[0].0, pts[pts.len() - 1].0);
    if to - from < cfg.min_duration {
        return None;
    }
    let f = fit(pts)?;
    let minutes = (to - from) as f64 / 60_000.0;
    let growth = f.slope_per_min * minutes;
    let floor = if unit == Unit::Bytes {
        cfg.min_growth_bytes
    } else {
        cfg.min_growth_count
    };
    if f.slope_per_min <= 0.0 || f.r2 < cfg.min_r2 || growth < floor || growth < cfg.min_growth_ratio * f.intercept.max(0.0) {
        return None;
    }
    let amount = |v: f64| match unit {
        Unit::Bytes => fmt_bytes(v),
        Unit::Count => fmt_num(v),
    };
    let step = (pts.len() / MAX_SAMPLES).max(1);
    let mut ids: Vec<u32> = pts.iter().step_by(step).map(|p| p.2).take(MAX_SAMPLES).collect();
    let last = pts[pts.len() - 1].2;
    if !ids.contains(&last) {
        ids.pop();
        ids.push(last);
    }
    Some(Anomaly {
        kind: AnomalyKind::MemoryGrowth,
        instance: Some(place.to_string()),
        from,
        to,
        score: if f.intercept > 0.0 { growth / f.intercept } else { growth / floor },
        message: format!(
            "{name} on {place} grew {} over {} (~{}/min, R² {:.2})",
            amount(growth),
            fmt_duration(to - from),
            amount(f.slope_per_min),
            f.r2
        ),
        sample_ids: ids,
    })
}

/// "1.8 GB", "512 MB", "40 KB", "12 bytes" (the shared rules of `imt_plugin_api::units`).
fn fmt_bytes(v: f64) -> String {
    let text = imt_plugin_api::units::bytes(v.abs().round() as u64);
    if v < 0.0 {
        format!("-{text}")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(fmt_bytes(2.0 * MB * 1024.0), "2 GB");
        assert_eq!(fmt_bytes(18.0 * MB), "18 MB");
        assert_eq!(fmt_bytes(2048.0), "2 KB");
        assert_eq!(fmt_bytes(12.0), "12 bytes");
        assert_eq!(fmt_bytes(-2048.0), "-2 KB", "a shrinking series keeps its sign");
    }

    fn sample(id: u32, minute: i64, working_set_mb: f64) -> (i64, crate::model::Record) {
        let mut r = crate::model::Record::new(id);
        r.id = id;
        r.fields.insert("WorkingSetMb".into(), serde_json::json!(working_set_mb));
        (minute * 60_000, r)
    }

    #[test]
    fn steady_growth_over_time_is_reported_per_instance_and_flat_series_are_not() {
        let rows = |f: &dyn Fn(i64) -> f64| -> Vec<(i64, crate::model::Record)> { (0..20).map(|m| sample(m as u32, m, f(m))).collect() };
        let leaking = rows(&|m| 300.0 + 30.0 * m as f64);
        let flat = rows(&|m| 300.0 + (m % 2) as f64);
        let found = memory_growth([("WEB01", leaking.as_slice()), ("WEB02", flat.as_slice())], &TrendConfig::default());
        assert_eq!(found.len(), 1, "{found:?}");
        let a = &found[0];
        assert_eq!((a.kind, a.instance.as_deref()), (AnomalyKind::MemoryGrowth, Some("WEB01")));
        assert!(a.message.contains("WorkingSetMb on WEB01 grew"), "{}", a.message);
        assert!(a.sample_ids.contains(&19), "the last point is always a sample: {:?}", a.sample_ids);
        assert!(a.sample_ids.len() <= MAX_SAMPLES);
        // Too short a span, or too few points, is no trend.
        let brief: Vec<_> = (0..8).map(|m| sample(m as u32, m, 300.0 + 100.0 * m as f64)).collect();
        assert!(memory_growth([("WEB01", brief.as_slice())], &TrendConfig::default()).is_empty());
        let few: Vec<_> = (0..4).map(|m| sample(m as u32, m * 10, 300.0 + 100.0 * m as f64)).collect();
        assert!(memory_growth([("WEB01", few.as_slice())], &TrendConfig::default()).is_empty());
        // A shrinking series is never a leak.
        let shrinking = rows(&|m| 900.0 - 30.0 * m as f64);
        assert!(memory_growth([("WEB01", shrinking.as_slice())], &TrendConfig::default()).is_empty());
    }

    #[test]
    fn fit_needs_variance() {
        assert!(fit(&[(0, 1.0, 0), (60_000, 1.0, 1)]).is_none());
        assert!(fit(&[(0, 1.0, 0), (0, 2.0, 1)]).is_none());
        let f = fit(&[(0, 1.0, 0), (60_000, 3.0, 1), (120_000, 5.0, 2)]).unwrap();
        assert!((f.slope_per_min - 2.0).abs() < 1e-9 && (f.r2 - 1.0).abs() < 1e-9 && (f.intercept - 1.0).abs() < 1e-9);
    }
}
