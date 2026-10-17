//! Sustained growth of memory-like series (leaks): private bytes, working set,
//! handle counts. A least-squares line over time must fit well (R²), span long
//! enough and grow by a meaningful amount, so sawtooth garbage-collection
//! patterns and noisy flat series are not reported.

use super::anomaly::{Anomaly, AnomalyKind};
use super::util::{fmt_duration, fmt_num};
use crate::model::Record;
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

struct Series {
    unit: Unit,
    /// (timestamp ms, value in bytes or count, record id)
    points: Vec<(i64, f64, u32)>,
}

/// Instances as `(key, events sorted or not)`; see [`super::detect`].
pub fn memory_growth<'a>(instances: impl IntoIterator<Item = (&'a str, &'a [(i64, &'a Record)])>, cfg: &TrendConfig) -> Vec<Anomaly> {
    let mut series: BTreeMap<(String, String), Series> = BTreeMap::new();
    for (key, events) in instances {
        for (ts, rec) in events {
            collect_fields(&mut series, key, *ts, rec, cfg);
            collect_counter(&mut series, *ts, rec, cfg);
        }
    }
    let mut out = Vec::new();
    for ((place, name), mut s) in series {
        s.points.sort_by_key(|p| p.0);
        if let Some(a) = judge(&place, &name, &s, cfg) {
            out.push(a);
        }
    }
    out
}

fn collect_fields(series: &mut BTreeMap<(String, String), Series>, key: &str, ts: i64, rec: &Record, cfg: &TrendConfig) {
    for field in &cfg.fields {
        let Some(v) = rec.field_number(field) else { continue };
        let lower = field.to_ascii_lowercase();
        let (unit, value) = if lower.ends_with("mb") {
            (Unit::Bytes, v * MB)
        } else {
            (Unit::Count, v)
        };
        series
            .entry((key.to_string(), field.clone()))
            .or_insert_with(|| Series { unit, points: Vec::new() })
            .points
            .push((ts, value, rec.id));
    }
}

fn collect_counter(series: &mut BTreeMap<(String, String), Series>, ts: i64, rec: &Record, cfg: &TrendConfig) {
    let text = |name: &str| rec.fields.get(name).and_then(|v| v.as_str());
    let (Some(counter), Some(value)) = (text("Counter"), rec.field_number("Value")) else {
        return;
    };
    if !cfg.counters.iter().any(|c| c == counter) {
        return;
    }
    let machine = text("MachineName").unwrap_or("");
    let instance = text("ProcessName").or_else(|| text("Instance")).unwrap_or("");
    let place = match rec.field_number("InstanceIndex") {
        Some(n) => format!("{machine} · {instance}#{n}"),
        None => format!("{machine} · {instance}"),
    };
    let unit = if counter.ends_with("Count") { Unit::Count } else { Unit::Bytes };
    series
        .entry((place, counter.to_string()))
        .or_insert_with(|| Series { unit, points: Vec::new() })
        .points
        .push((ts, value, rec.id));
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

fn judge(place: &str, name: &str, s: &Series, cfg: &TrendConfig) -> Option<Anomaly> {
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
    let floor = if s.unit == Unit::Bytes {
        cfg.min_growth_bytes
    } else {
        cfg.min_growth_count
    };
    if f.slope_per_min <= 0.0 || f.r2 < cfg.min_r2 || growth < floor || growth < cfg.min_growth_ratio * f.intercept.max(0.0) {
        return None;
    }
    let amount = |v: f64| match s.unit {
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

/// "1.8 GB", "512 MB", "40 KB", "12 B".
fn fmt_bytes(v: f64) -> String {
    const KB: f64 = 1024.0;
    let a = v.abs();
    if a >= MB * 1024.0 {
        format!("{:.1} GB", v / (MB * 1024.0))
    } else if a >= MB {
        format!("{} MB", fmt_num(v / MB))
    } else if a >= KB {
        format!("{} KB", fmt_num(v / KB))
    } else {
        format!("{} B", fmt_num(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(fmt_bytes(2.0 * MB * 1024.0), "2.0 GB");
        assert_eq!(fmt_bytes(18.0 * MB), "18 MB");
        assert_eq!(fmt_bytes(2048.0), "2 KB");
        assert_eq!(fmt_bytes(12.0), "12 B");
    }

    #[test]
    fn fit_needs_variance() {
        assert!(fit(&[(0, 1.0, 0), (60_000, 1.0, 1)]).is_none());
        assert!(fit(&[(0, 1.0, 0), (0, 2.0, 1)]).is_none());
        let f = fit(&[(0, 1.0, 0), (60_000, 3.0, 1), (120_000, 5.0, 2)]).unwrap();
        assert!((f.slope_per_min - 2.0).abs() < 1e-9 && (f.r2 - 1.0).abs() < 1e-9 && (f.intercept - 1.0).abs() < 1e-9);
    }
}
