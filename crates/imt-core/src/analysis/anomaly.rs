//! Finds the moments in a set of logs that deserve a look: error bursts,
//! instances that fell silent or crashed, slow outliers and rate shifts.

use super::trend::{memory_growth, TrendConfig};
use super::util::{fmt_clock, fmt_duration, fmt_num, mad, median, median_sorted, percentile_sorted, sorted, truncate};
use crate::chart::nice_bucket;
use crate::model::{Level, Record};
use crate::query::GROUP_SEPARATOR;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_SAMPLES: usize = 5;
/// Instances with fewer events than this are too sparse to call "steady".
const MIN_STEADY_EVENTS: usize = 10;
/// Buckets of history needed before the overall rate can be judged.
const RATE_HISTORY: usize = 5;
const RATE_WINDOW: usize = 10;
const RATE_FACTOR: f64 = 5.0;
const MIN_LATENCY_VALUES: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnomalyKind {
    ErrorSpike,
    Silence,
    Crash,
    SlowOutlier,
    RateChange,
    MemoryGrowth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AnomalyConfig {
    /// Robust z-score an error bucket must reach.
    pub z: f64,
    /// Minimum errors in a bucket to call it a spike.
    pub min_count: u32,
    /// Shortest silence worth reporting (ms).
    pub min_gap: i64,
    /// An exception this close to an instance's last event counts as its crash (ms).
    pub crash_window: i64,
    /// How soon after a crash a new instance on the same machine is a restart (ms).
    pub restart_window: i64,
    /// Numeric fields examined for slow outliers.
    pub latency_fields: Vec<String>,
    /// Target number of time buckets over the whole range.
    pub buckets: usize,
    /// Thresholds for [`AnomalyKind::MemoryGrowth`].
    pub growth: TrendConfig,
}

impl Default for AnomalyConfig {
    fn default() -> Self {
        AnomalyConfig {
            z: 4.0,
            min_count: 5,
            min_gap: 30_000,
            crash_window: 5_000,
            restart_window: 120_000,
            latency_fields: ["ElapsedMs", "Elapsed", "Duration", "DurationMs"].map(String::from).to_vec(),
            buckets: 200,
            growth: TrendConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Anomaly {
    pub kind: AnomalyKind,
    /// Instance key; `None` when the finding spans all instances.
    pub instance: Option<String>,
    pub from: i64,
    pub to: i64,
    pub score: f64,
    pub message: String,
    /// Up to five event ids that show the problem.
    pub sample_ids: Vec<u32>,
}

type Events<'a> = Vec<(i64, &'a Record)>;

struct Ctx<'a> {
    instances: BTreeMap<String, Events<'a>>,
    start: i64,
    end: i64,
    bucket: i64,
    buckets: usize,
}

impl Ctx<'_> {
    fn index(&self, ts: i64) -> usize {
        (((ts - self.start) / self.bucket) as usize).min(self.buckets - 1)
    }

    fn span_of(&self, first: usize, last: usize) -> (i64, i64) {
        let from = self.start + first as i64 * self.bucket;
        let to = (self.start + (last as i64 + 1) * self.bucket).min(self.end).max(from);
        (from, to)
    }

    fn per_minute(&self, count: f64) -> String {
        fmt_num(count * 60_000.0 / self.bucket as f64)
    }
}

/// Runs every detector over `(event, instance key)` pairs. Events without a
/// timestamp are ignored. Results are ordered by start time, then score.
pub fn detect<'a>(records: impl IntoIterator<Item = (&'a Record, &'a str)>, cfg: &AnomalyConfig) -> Vec<Anomaly> {
    let mut instances: BTreeMap<String, Events<'a>> = BTreeMap::new();
    for (rec, key) in records {
        let Some(ts) = rec.ts else { continue };
        match instances.get_mut(key) {
            Some(v) => v.push((ts, rec)),
            None => {
                instances.insert(key.to_string(), vec![(ts, rec)]);
            }
        }
    }
    for v in instances.values_mut() {
        v.sort_by_key(|e| e.0);
    }
    let bounds = instances.values().fold(None, |acc: Option<(i64, i64)>, v| {
        let (lo, hi) = (v[0].0, v[v.len() - 1].0);
        Some(acc.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))))
    });
    let Some((start, end)) = bounds else { return Vec::new() };
    let bucket = nice_bucket(end - start, cfg.buckets);
    let buckets = ((end - start) / bucket) as usize + 1;
    let ctx = Ctx {
        instances,
        start,
        end,
        bucket,
        buckets,
    };
    let mut out = error_spikes(&ctx, cfg);
    out.extend(silences(&ctx, cfg));
    out.extend(crashes(&ctx, cfg));
    out.extend(slow_outliers(&ctx, cfg));
    out.extend(rate_changes(&ctx));
    out.extend(memory_growth(
        ctx.instances.iter().map(|(k, v)| (k.as_str(), v.as_slice())),
        &cfg.growth,
    ));
    out.sort_by(|a, b| a.from.cmp(&b.from).then(b.score.total_cmp(&a.score)));
    out
}

/// Consecutive flagged entries as `(first, last, max value)`.
fn runs(flags: &[Option<f64>]) -> Vec<(usize, usize, f64)> {
    let mut out: Vec<(usize, usize, f64)> = Vec::new();
    for (i, f) in flags.iter().enumerate() {
        let Some(v) = *f else { continue };
        match out.last_mut() {
            Some(last) if last.1 + 1 == i => {
                last.1 = i;
                last.2 = last.2.max(v);
            }
            _ => out.push((i, i, v)),
        }
    }
    out
}

fn is_error(r: &Record) -> bool {
    r.level >= Level::Error
}

fn error_spikes(ctx: &Ctx, cfg: &AnomalyConfig) -> Vec<Anomaly> {
    let mut out = Vec::new();
    for (key, events) in &ctx.instances {
        let (first, last) = (ctx.index(events[0].0), ctx.index(events[events.len() - 1].0));
        let mut counts = vec![0f64; ctx.buckets];
        for (ts, _) in events.iter().filter(|e| is_error(e.1)) {
            counts[ctx.index(*ts)] += 1.0;
        }
        let life = &counts[first..=last];
        let med = median(life);
        let scale = (1.4826 * mad(life, med)).max(1.0);
        let flags: Vec<Option<f64>> = counts
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let z = (c - med) / scale;
                (i >= first && i <= last && z >= cfg.z && *c >= f64::from(cfg.min_count)).then_some(z)
            })
            .collect();
        for (a, b, z) in runs(&flags) {
            let (from, to) = ctx.span_of(a, b);
            let peak = counts[a..=b].iter().copied().fold(0.0, f64::max);
            out.push(Anomaly {
                kind: AnomalyKind::ErrorSpike,
                instance: Some(key.clone()),
                from,
                to,
                score: z,
                message: format!(
                    "Error rate on {key} rose to {}/min (baseline {}/min)",
                    ctx.per_minute(peak),
                    ctx.per_minute(med)
                ),
                sample_ids: events
                    .iter()
                    .filter(|(ts, r)| is_error(r) && *ts >= from && *ts <= to)
                    .map(|(_, r)| r.id)
                    .take(MAX_SAMPLES)
                    .collect(),
            });
        }
    }
    out
}

/// The event that explains why an instance ended, if it looks like a crash: a
/// fatal event or an error with an exception close to the instance's last event.
fn crash_trigger<'a>(events: &Events<'a>, end: i64, cfg: &AnomalyConfig) -> Option<(i64, &'a Record)> {
    let last_ts = events[events.len() - 1].0;
    if last_ts >= end {
        return None;
    }
    events
        .iter()
        .rev()
        .take_while(|(ts, _)| *ts >= last_ts - cfg.crash_window)
        .find(|(_, r)| r.level == Level::Fatal || (r.exception.is_some() && is_error(r)))
        .copied()
}

fn silences(ctx: &Ctx, cfg: &AnomalyConfig) -> Vec<Anomaly> {
    let mut out = Vec::new();
    for (key, events) in &ctx.instances {
        if events.len() < MIN_STEADY_EVENTS {
            continue;
        }
        let gaps: Vec<f64> = events.windows(2).map(|w| (w[1].0 - w[0].0) as f64).collect();
        let threshold = (cfg.min_gap as f64).max(10.0 * median_sorted(&sorted(&gaps)));
        let mut report = |from: i64, to: i64| {
            let gap = to - from;
            out.push(Anomaly {
                kind: AnomalyKind::Silence,
                instance: Some(key.clone()),
                from,
                to,
                score: gap as f64 / threshold,
                message: format!("{key} went silent at {} (no events for {})", fmt_clock(from), fmt_duration(gap)),
                sample_ids: Vec::new(),
            });
        };
        for w in events.windows(2) {
            if (w[1].0 - w[0].0) as f64 >= threshold {
                report(w[0].0, w[1].0);
            }
        }
        let (last_ts, last) = events[events.len() - 1];
        let stopped = {
            let m = last.message.to_ascii_lowercase();
            m.contains("stop") || m.contains("shutting down")
        };
        if (ctx.end - last_ts) as f64 >= threshold && !stopped && crash_trigger(events, ctx.end, cfg).is_none() {
            report(last_ts, ctx.end);
        }
    }
    out
}

/// The key without its last component (usually the process id): what a
/// restarted process has in common with the one it replaces.
fn machine(key: &str) -> &str {
    key.rsplit_once(GROUP_SEPARATOR).map_or(key, |(head, _)| head)
}

fn crashes(ctx: &Ctx, cfg: &AnomalyConfig) -> Vec<Anomaly> {
    let mut out = Vec::new();
    for (key, events) in &ctx.instances {
        let Some((crash_ts, trigger)) = crash_trigger(events, ctx.end, cfg) else {
            continue;
        };
        let last = events[events.len() - 1].1;
        let restart = ctx
            .instances
            .iter()
            .filter(|(k, _)| *k != key && machine(k) == machine(key))
            .map(|(k, v)| (k, v[0].0))
            .filter(|(_, first)| *first >= crash_ts && *first - crash_ts <= cfg.restart_window)
            .min_by_key(|(_, first)| *first);
        let mut message = format!(
            "{key} crashed at {} ({} \"{}\")",
            fmt_clock(crash_ts),
            trigger.level.as_str(),
            truncate(&trigger.message, 80)
        );
        if let Some((next, first)) = restart {
            message.push_str(&format!("; restarted as {next} {} later", fmt_duration(first - crash_ts)));
        }
        let mut ids: Vec<u32> = vec![trigger.id, last.id];
        ids.extend(events.iter().rev().filter(|e| is_error(e.1)).map(|e| e.1.id));
        let mut seen = Vec::new();
        ids.retain(|id| {
            let fresh = !seen.contains(id);
            seen.push(*id);
            fresh
        });
        ids.truncate(MAX_SAMPLES);
        out.push(Anomaly {
            kind: AnomalyKind::Crash,
            instance: Some(key.clone()),
            from: crash_ts,
            to: restart.map_or(crash_ts, |(_, first)| first),
            score: if trigger.level == Level::Fatal { 10.0 } else { 5.0 },
            message,
            sample_ids: ids,
        });
    }
    out
}

fn slow_outliers(ctx: &Ctx, cfg: &AnomalyConfig) -> Vec<Anomaly> {
    let mut out = Vec::new();
    for field in &cfg.latency_fields {
        let mut samples: Vec<(i64, f64, u32, &str)> = Vec::new();
        for (key, events) in &ctx.instances {
            for (ts, r) in events {
                if let Some(v) = r.field_number(field) {
                    samples.push((*ts, v, r.id, key));
                }
            }
        }
        if samples.len() < MIN_LATENCY_VALUES {
            continue;
        }
        let values = sorted(&samples.iter().map(|s| s.1).collect::<Vec<_>>());
        let (med, p99) = (median_sorted(&values), percentile_sorted(&values, 99.0));
        let mut by_bucket: BTreeMap<usize, Vec<&(i64, f64, u32, &str)>> = BTreeMap::new();
        for s in samples.iter().filter(|s| s.1 > p99 && s.1 >= 3.0 * med) {
            by_bucket.entry(ctx.index(s.0)).or_default().push(s);
        }
        for (idx, group) in by_bucket {
            let (from, to) = ctx.span_of(idx, idx);
            let peak = group.iter().map(|s| s.1).fold(f64::MIN, f64::max);
            let first_instance = group[0].3;
            let single = group.iter().all(|s| s.3 == first_instance);
            let place = if single { first_instance } else { "several instances" };
            out.push(Anomaly {
                kind: AnomalyKind::SlowOutlier,
                instance: single.then(|| first_instance.to_string()),
                from,
                to,
                score: peak / med.max(1.0),
                message: format!(
                    "Slow {field} on {place}: {} values up to {} (typical {}, p99 {})",
                    group.len(),
                    fmt_num(peak),
                    fmt_num(med),
                    fmt_num(p99)
                ),
                sample_ids: group.iter().map(|s| s.2).take(MAX_SAMPLES).collect(),
            });
        }
    }
    out
}

fn rate_changes(ctx: &Ctx) -> Vec<Anomaly> {
    let mut counts = vec![0f64; ctx.buckets];
    let mut ids: Vec<Vec<u32>> = vec![Vec::new(); ctx.buckets];
    for (ts, r) in ctx.instances.values().flatten() {
        let i = ctx.index(*ts);
        counts[i] += 1.0;
        if ids[i].len() < MAX_SAMPLES {
            ids[i].push(r.id);
        }
    }
    let mut up = vec![None; ctx.buckets];
    let mut down = vec![None; ctx.buckets];
    let mut baselines = vec![0.0; ctx.buckets];
    for i in RATE_HISTORY..ctx.buckets {
        let med = median(&counts[i.saturating_sub(RATE_WINDOW)..i]);
        baselines[i] = med;
        if med > 0.0 && counts[i] >= RATE_FACTOR * med {
            up[i] = Some(counts[i] / med);
        } else if med > 0.0 && counts[i] * RATE_FACTOR <= med {
            down[i] = Some(med / counts[i].max(0.5));
        }
    }
    let mut out = Vec::new();
    for (flags, verb) in [(&up, "rose"), (&down, "dropped")] {
        for (a, b, score) in runs(flags) {
            let (from, to) = ctx.span_of(a, b);
            let peak_idx = (a..=b).max_by(|x, y| ratio(&counts, &baselines, *x).total_cmp(&ratio(&counts, &baselines, *y)));
            let i = peak_idx.unwrap_or(a);
            out.push(Anomaly {
                kind: AnomalyKind::RateChange,
                instance: None,
                from,
                to,
                score,
                message: format!(
                    "Overall event rate {verb} to {}/min (recent typical {}/min)",
                    ctx.per_minute(counts[i]),
                    ctx.per_minute(baselines[i])
                ),
                sample_ids: ids[a..=b].iter().flatten().copied().take(MAX_SAMPLES).collect(),
            });
        }
    }
    out
}

fn ratio(counts: &[f64], baselines: &[f64], i: usize) -> f64 {
    let (c, b) = (counts[i], baselines[i]);
    (c / b).max(b / c.max(0.5))
}
