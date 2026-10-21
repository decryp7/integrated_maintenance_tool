//! Finds the moments in a set of logs that deserve a look: error bursts,
//! instances that fell silent or crashed, slow outliers and rate shifts.
//!
//! The detectors read what they need through [`AnomalyInputs`] (per-instance counts, gaps, endings, value
//! ranks and a few sample ids), so a store can answer with aggregates computed where the data is instead of
//! holding every event; [`detect`] answers them from events in memory ([`super::anomaly_events`]).

use super::anomaly_events::{EventInputs, SlimNames};
use super::trend::{judge_series, GrowthSeries, TrendConfig};
use super::util::{fmt_clock, fmt_duration, fmt_num, mad, median, truncate};
use crate::chart::nice_bucket;
use crate::model::Level;
use crate::query::GROUP_SEPARATOR;
use crate::row::RowRef;
use serde::{Deserialize, Serialize};

/// Sample ids per finding.
pub const MAX_SAMPLES: usize = 5;
/// Instances with fewer events than this are too sparse to call "steady".
pub const MIN_STEADY_EVENTS: u64 = 10;
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

/// An event's id as the DTOs carry it (saturating until 6c widens them).
pub fn sample_id(id: u64) -> u32 {
    u32::try_from(id).unwrap_or(u32::MAX)
}

/// Whether a message says its instance stopped on purpose (so its last event is no silence).
pub fn says_stopped(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("stop") || m.contains("shutting down")
}

/// Whether an event can explain a crash: fatal, or an error with an exception.
pub fn is_crash_trigger(level: Level, has_exception: bool) -> bool {
    level == Level::Fatal || (has_exception && level >= Level::Error)
}

/// An instance (events with the same instance key) with timed events.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub key: String,
    /// Its timed events.
    pub events: u64,
    /// Its first and last timestamps.
    pub first: i64,
    pub last: i64,
}

/// The time buckets of one detection: [`AnomalyConfig::buckets`] "nice" buckets over every instance's events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub start: i64,
    pub end: i64,
    /// Bucket width (ms).
    pub bucket: i64,
    pub buckets: usize,
}

impl Layout {
    fn over(instances: &[Instance], buckets: usize) -> Option<Layout> {
        let start = instances.iter().map(|i| i.first).min()?;
        let end = instances.iter().map(|i| i.last).max()?;
        let bucket = nice_bucket(end - start, buckets);
        Some(Layout {
            start,
            end,
            bucket,
            buckets: ((end - start) / bucket) as usize + 1,
        })
    }

    /// The bucket of a timestamp within the layout.
    pub fn index(&self, ts: i64) -> usize {
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

/// The events and error events (level error or above) of one instance in one bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BucketCount {
    /// Index into [`AnomalyInputs::instances`].
    pub instance: usize,
    pub bucket: usize,
    pub events: u64,
    pub errors: u64,
}

/// A time range of one instance (both ends included).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceRange {
    pub instance: usize,
    pub from: i64,
    pub to: i64,
}

/// The gaps between an instance's consecutive events.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Gaps {
    /// The median gap (ms).
    pub median: f64,
    /// The long gaps as `(from, to)`, in time order.
    pub long: Vec<(i64, i64)>,
}

/// The median of `n` values fed in ascending order as `(value, count)` runs, as
/// [`super::util::median_sorted`] takes it (0 without values), in constant memory.
#[derive(Debug, Clone)]
pub struct CountedMedian {
    lo: u64,
    hi: u64,
    seen: u64,
    a: Option<f64>,
    b: Option<f64>,
}

impl CountedMedian {
    pub fn new(n: u64) -> CountedMedian {
        let (lo, hi) = match n {
            0 => (u64::MAX, u64::MAX),
            n if n % 2 == 1 => (n / 2, n / 2),
            n => (n / 2 - 1, n / 2),
        };
        CountedMedian {
            lo,
            hi,
            seen: 0,
            a: None,
            b: None,
        }
    }

    pub fn push(&mut self, value: f64, count: u64) {
        self.seen = self.seen.saturating_add(count);
        if self.a.is_none() && self.seen > self.lo {
            self.a = Some(value);
        }
        if self.b.is_none() && self.seen > self.hi {
            self.b = Some(value);
        }
    }

    pub fn median(&self) -> f64 {
        match (self.a, self.b) {
            (Some(a), Some(_)) if self.lo == self.hi => a,
            (Some(a), Some(b)) => (a + b) / 2.0,
            _ => 0.0,
        }
    }
}

/// The median of `n` sorted values given as `(value, count)` runs (see [`CountedMedian`]).
pub fn median_of_counts(n: u64, runs: impl IntoIterator<Item = (f64, u64)>) -> f64 {
    let mut m = CountedMedian::new(n);
    for (v, c) in runs {
        m.push(v, c);
    }
    m.median()
}

/// How an instance's events end.
#[derive(Debug, Clone, PartialEq)]
pub struct Ending {
    /// Its last event (in time order).
    pub last_id: u32,
    /// Whether the last event's message says it stopped ([`says_stopped`]).
    pub stopped: bool,
    /// The last crash trigger ([`is_crash_trigger`]) within the crash window before the last event.
    pub trigger: Option<Trigger>,
}

/// The event that may explain why an instance ended.
#[derive(Debug, Clone, PartialEq)]
pub struct Trigger {
    pub ts: i64,
    pub id: u32,
    pub level: Level,
    pub message: String,
}

/// The outlying values of one field in one bucket.
#[derive(Debug, Clone, PartialEq)]
pub struct OutlierBucket {
    pub bucket: usize,
    pub count: usize,
    /// The largest value.
    pub peak: f64,
    /// The first instance (in key order) with an outlier here, and whether it is the only one.
    pub first_instance: usize,
    pub single: bool,
    /// The first ids, in instance order, then time order.
    pub ids: Vec<u32>,
}

/// What the detectors read of the events (the matching events with a timestamp, grouped into instances).
/// "Time order" within an instance is the order of timestamps, then of storage. Implementations answer from
/// events in memory or from a database; they must give the same answers.
pub trait AnomalyInputs {
    type Error;

    /// The instances, sorted by key.
    fn instances(&mut self) -> Result<Vec<Instance>, Self::Error>;

    /// The non-zero counts per instance and bucket of `layout`.
    fn bucket_counts(&mut self, layout: &Layout) -> Result<Vec<BucketCount>, Self::Error>;

    /// For each range, the ids of its instance's first `n` error events in it, in time order.
    fn error_ids(&mut self, ranges: &[InstanceRange], n: usize) -> Result<Vec<Vec<u32>>, Self::Error>;

    /// For each of `instances` (indexes), the gaps between its consecutive events: their median and those of
    /// at least `at_least` ms.
    fn gaps(&mut self, instances: &[usize], at_least: f64) -> Result<Vec<Gaps>, Self::Error>;

    /// How each instance ends, crash triggers looked for within `crash_window` ms before its last event.
    fn endings(&mut self, crash_window: i64) -> Result<Vec<Ending>, Self::Error>;

    /// For each of `instances`, the ids of its last `n` error events, newest first.
    fn latest_error_ids(&mut self, instances: &[usize], n: usize) -> Result<Vec<Vec<u32>>, Self::Error>;

    /// How many events have a number in `field`.
    fn value_count(&mut self, field: &str) -> Result<usize, Self::Error>;

    /// The values of `field` at the `ranks` (0-based, ascending) of all its values sorted.
    fn values_at(&mut self, field: &str, ranks: &[usize]) -> Result<Vec<f64>, Self::Error>;

    /// The events whose `field` is above `above` and at least `at_least`, per bucket (ascending).
    fn outliers(&mut self, field: &str, above: f64, at_least: f64, layout: &Layout) -> Result<Vec<OutlierBucket>, Self::Error>;

    /// For each run of buckets `(first, last)`, the first `n` event ids in bucket, instance, then time order.
    fn bucket_ids(&mut self, layout: &Layout, runs: &[(usize, usize)], n: usize) -> Result<Vec<Vec<u32>>, Self::Error>;

    /// Calls `f` with each growth series ([`TrendConfig`]: the instances' growth fields and the PerfMon
    /// counters), its points in time order (then instance order for counters).
    fn growth_series(&mut self, cfg: &TrendConfig, f: &mut dyn FnMut(GrowthSeries)) -> Result<(), Self::Error>;
}

/// Runs every detector over `(event, instance key)` pairs. Events without a
/// timestamp are ignored. Results are ordered by start time, then score. Only a few fields of each event
/// are kept while detecting.
pub fn detect<R: RowRef, K: AsRef<str>>(records: impl IntoIterator<Item = (R, K)>, cfg: &AnomalyConfig) -> Vec<Anomaly> {
    let names = SlimNames::of(cfg);
    let mut inputs = EventInputs::new(records, &names);
    match detect_with(&mut inputs, cfg) {
        Ok(found) => found,
        Err(never) => match never {},
    }
}

/// Runs every detector over what `inputs` answers (see [`detect`]).
pub fn detect_with<I: AnomalyInputs + ?Sized>(inputs: &mut I, cfg: &AnomalyConfig) -> Result<Vec<Anomaly>, I::Error> {
    let instances = inputs.instances()?;
    let Some(layout) = Layout::over(&instances, cfg.buckets) else {
        return Ok(Vec::new());
    };
    let counts = inputs.bucket_counts(&layout)?;
    let mut out = error_spikes(inputs, &layout, &instances, &counts, cfg)?;
    let endings = inputs.endings(cfg.crash_window)?;
    out.extend(silences(inputs, &layout, &instances, &endings, cfg)?);
    out.extend(crashes(inputs, &layout, &instances, &endings, cfg)?);
    out.extend(slow_outliers(inputs, &layout, &instances, cfg)?);
    out.extend(rate_changes(inputs, &layout, &counts)?);
    let mut growth: Vec<((String, String), Anomaly)> = Vec::new();
    inputs.growth_series(&cfg.growth, &mut |s| {
        let key = (s.place.clone(), s.name.clone());
        if let Some(a) = judge_series(s, &cfg.growth) {
            growth.push((key, a));
        }
    })?;
    growth.sort_by(|a, b| a.0.cmp(&b.0));
    out.extend(growth.into_iter().map(|(_, a)| a));
    out.sort_by(|a, b| a.from.cmp(&b.from).then(b.score.total_cmp(&a.score)));
    Ok(out)
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

fn error_spikes<I: AnomalyInputs + ?Sized>(
    inputs: &mut I,
    layout: &Layout,
    instances: &[Instance],
    counts: &[BucketCount],
    cfg: &AnomalyConfig,
) -> Result<Vec<Anomaly>, I::Error> {
    let mut errors: Vec<Vec<(usize, u64)>> = vec![Vec::new(); instances.len()];
    for c in counts.iter().filter(|c| c.errors > 0) {
        if let Some(e) = errors.get_mut(c.instance) {
            e.push((c.bucket, c.errors));
        }
    }
    // (instance, range, z, peak, median)
    let mut spikes: Vec<(usize, InstanceRange, f64, f64, f64)> = Vec::new();
    for (i, inst) in instances.iter().enumerate() {
        let (first, last) = (layout.index(inst.first), layout.index(inst.last));
        let mut counts = vec![0f64; layout.buckets];
        for &(b, n) in &errors[i] {
            counts[b.min(layout.buckets - 1)] += n as f64;
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
            let (from, to) = layout.span_of(a, b);
            let peak = counts[a..=b].iter().copied().fold(0.0, f64::max);
            spikes.push((i, InstanceRange { instance: i, from, to }, z, peak, med));
        }
    }
    let ranges: Vec<InstanceRange> = spikes.iter().map(|s| s.1).collect();
    let ids = inputs.error_ids(&ranges, MAX_SAMPLES)?;
    Ok(spikes
        .into_iter()
        .zip(ids.into_iter().chain(std::iter::repeat_with(Vec::new)))
        .map(|((i, range, z, peak, med), sample_ids)| {
            let key = &instances[i].key;
            Anomaly {
                kind: AnomalyKind::ErrorSpike,
                instance: Some(key.clone()),
                from: range.from,
                to: range.to,
                score: z,
                message: format!(
                    "Error rate on {key} rose to {}/min (baseline {}/min)",
                    layout.per_minute(peak),
                    layout.per_minute(med)
                ),
                sample_ids,
            }
        })
        .collect())
}

/// The event that explains why an instance ended, if it looks like a crash: a fatal event or an error with
/// an exception close to the instance's last event (which must not be the last of all events).
fn crash_trigger<'a>(inst: &Instance, ending: &'a Ending, end: i64) -> Option<&'a Trigger> {
    if inst.last >= end {
        return None;
    }
    ending.trigger.as_ref()
}

fn silences<I: AnomalyInputs + ?Sized>(
    inputs: &mut I,
    layout: &Layout,
    instances: &[Instance],
    endings: &[Ending],
    cfg: &AnomalyConfig,
) -> Result<Vec<Anomaly>, I::Error> {
    let steady: Vec<usize> = (0..instances.len()).filter(|&i| instances[i].events >= MIN_STEADY_EVENTS).collect();
    // The threshold is at least the minimum gap: those gaps are the candidates.
    let gaps = inputs.gaps(&steady, cfg.min_gap as f64)?;
    let mut out = Vec::new();
    for (&i, gaps) in steady.iter().zip(gaps) {
        let inst = &instances[i];
        let threshold = (cfg.min_gap as f64).max(10.0 * gaps.median);
        let key = &inst.key;
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
        for &(from, to) in &gaps.long {
            if (to - from) as f64 >= threshold {
                report(from, to);
            }
        }
        let Some(ending) = endings.get(i) else { continue };
        if (layout.end - inst.last) as f64 >= threshold && !ending.stopped && crash_trigger(inst, ending, layout.end).is_none() {
            report(inst.last, layout.end);
        }
    }
    Ok(out)
}

/// The key without its last component (usually the process id): what a
/// restarted process has in common with the one it replaces.
fn machine(key: &str) -> &str {
    key.rsplit_once(GROUP_SEPARATOR).map_or(key, |(head, _)| head)
}

fn crashes<I: AnomalyInputs + ?Sized>(
    inputs: &mut I,
    layout: &Layout,
    instances: &[Instance],
    endings: &[Ending],
    cfg: &AnomalyConfig,
) -> Result<Vec<Anomaly>, I::Error> {
    let crashed: Vec<(usize, &Trigger, u32)> = instances
        .iter()
        .zip(endings)
        .enumerate()
        .filter_map(|(i, (inst, e))| Some((i, crash_trigger(inst, e, layout.end)?, e.last_id)))
        .collect();
    let which: Vec<usize> = crashed.iter().map(|c| c.0).collect();
    // The trigger and the last event may be among the latest errors.
    let latest = inputs.latest_error_ids(&which, MAX_SAMPLES + 2)?;
    let mut out = Vec::new();
    for ((i, trigger, last_id), errors) in crashed.into_iter().zip(latest.into_iter().chain(std::iter::repeat_with(Vec::new))) {
        let key = &instances[i].key;
        let crash_ts = trigger.ts;
        let restart = instances
            .iter()
            .filter(|k| k.key != *key && machine(&k.key) == machine(key))
            .map(|k| (&k.key, k.first))
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
        let mut ids: Vec<u32> = vec![trigger.id, last_id];
        ids.extend(errors);
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
    Ok(out)
}

/// The ranks a percentile (`p` in 0..=100) of `n` sorted values interpolates between, and the weight of the
/// higher one (as [`super::util::percentile_sorted`] computes it).
fn percentile_ranks(n: usize, p: f64) -> (usize, usize, f64) {
    let rank = p / 100.0 * (n - 1) as f64;
    let (lo, hi) = (rank.floor() as usize, rank.ceil() as usize);
    (lo, hi, rank - lo as f64)
}

fn slow_outliers<I: AnomalyInputs + ?Sized>(
    inputs: &mut I,
    layout: &Layout,
    instances: &[Instance],
    cfg: &AnomalyConfig,
) -> Result<Vec<Anomaly>, I::Error> {
    let mut out = Vec::new();
    for field in &cfg.latency_fields {
        let n = inputs.value_count(field)?;
        if n < MIN_LATENCY_VALUES {
            continue;
        }
        // The median as `median_sorted` takes it, and the 99th percentile.
        let middle = if n % 2 == 1 { (n / 2, n / 2) } else { (n / 2 - 1, n / 2) };
        let (lo, hi, frac) = percentile_ranks(n, 99.0);
        let mut ranks = vec![middle.0, middle.1, lo, hi];
        ranks.sort_unstable();
        ranks.dedup();
        let values = inputs.values_at(field, &ranks)?;
        let at = |r: usize| {
            ranks
                .iter()
                .position(|x| *x == r)
                .and_then(|i| values.get(i))
                .copied()
                .unwrap_or(0.0)
        };
        let med = if middle.0 == middle.1 {
            at(middle.0)
        } else {
            (at(middle.0) + at(middle.1)) / 2.0
        };
        let p99 = at(lo) + (at(hi) - at(lo)) * frac;
        for group in inputs.outliers(field, p99, 3.0 * med, layout)? {
            let (from, to) = layout.span_of(group.bucket, group.bucket);
            let first_instance = instances.get(group.first_instance).map_or("", |i| i.key.as_str());
            let place = if group.single { first_instance } else { "several instances" };
            out.push(Anomaly {
                kind: AnomalyKind::SlowOutlier,
                instance: group.single.then(|| first_instance.to_string()),
                from,
                to,
                score: group.peak / med.max(1.0),
                message: format!(
                    "Slow {field} on {place}: {} values up to {} (typical {}, p99 {})",
                    group.count,
                    fmt_num(group.peak),
                    fmt_num(med),
                    fmt_num(p99)
                ),
                sample_ids: group.ids,
            });
        }
    }
    Ok(out)
}

fn rate_changes<I: AnomalyInputs + ?Sized>(inputs: &mut I, layout: &Layout, counts: &[BucketCount]) -> Result<Vec<Anomaly>, I::Error> {
    let mut totals = vec![0f64; layout.buckets];
    for c in counts {
        totals[c.bucket.min(layout.buckets - 1)] += c.events as f64;
    }
    let mut up = vec![None; layout.buckets];
    let mut down = vec![None; layout.buckets];
    let mut baselines = vec![0.0; layout.buckets];
    for i in RATE_HISTORY..layout.buckets {
        let med = median(&totals[i.saturating_sub(RATE_WINDOW)..i]);
        baselines[i] = med;
        if med > 0.0 && totals[i] >= RATE_FACTOR * med {
            up[i] = Some(totals[i] / med);
        } else if med > 0.0 && totals[i] * RATE_FACTOR <= med {
            down[i] = Some(med / totals[i].max(0.5));
        }
    }
    let mut found: Vec<(usize, usize, f64, &str)> = Vec::new();
    for (flags, verb) in [(&up, "rose"), (&down, "dropped")] {
        found.extend(runs(flags).into_iter().map(|(a, b, score)| (a, b, score, verb)));
    }
    let spans: Vec<(usize, usize)> = found.iter().map(|f| (f.0, f.1)).collect();
    let ids = inputs.bucket_ids(layout, &spans, MAX_SAMPLES)?;
    Ok(found
        .into_iter()
        .zip(ids.into_iter().chain(std::iter::repeat_with(Vec::new)))
        .map(|((a, b, score, verb), sample_ids)| {
            let (from, to) = layout.span_of(a, b);
            let peak_idx = (a..=b).max_by(|x, y| ratio(&totals, &baselines, *x).total_cmp(&ratio(&totals, &baselines, *y)));
            let i = peak_idx.unwrap_or(a);
            Anomaly {
                kind: AnomalyKind::RateChange,
                instance: None,
                from,
                to,
                score,
                message: format!(
                    "Overall event rate {verb} to {}/min (recent typical {}/min)",
                    layout.per_minute(totals[i]),
                    layout.per_minute(baselines[i])
                ),
                sample_ids,
            }
        })
        .collect())
}

fn ratio(counts: &[f64], baselines: &[f64], i: usize) -> f64 {
    let (c, b) = (counts[i], baselines[i]);
    (c / b).max(b / c.max(0.5))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::util::{median_sorted, percentile_sorted};

    #[test]
    fn percentile_ranks_interpolate_as_percentile_sorted() {
        for n in [1usize, 2, 3, 20, 101, 1000] {
            let v: Vec<f64> = (0..n).map(|i| (i * i) as f64 / 7.0).collect();
            let (lo, hi, frac) = percentile_ranks(n, 99.0);
            assert_eq!(v[lo] + (v[hi] - v[lo]) * frac, percentile_sorted(&v, 99.0), "n = {n}");
            let mid = if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 };
            assert_eq!(mid, median_sorted(&v));
        }
    }

    #[test]
    fn medians_of_counted_values_are_those_of_the_values() {
        for values in [
            vec![],
            vec![4.0],
            vec![1.0, 1.0, 2.0, 9.0],
            vec![1.0, 2.0, 2.0, 2.0, 3.0, 8.0, 8.0],
            vec![5.0, 7.0],
        ] {
            let mut runs: Vec<(f64, u64)> = Vec::new();
            for v in &values {
                match runs.last_mut() {
                    Some(r) if r.0 == *v => r.1 += 1,
                    _ => runs.push((*v, 1)),
                }
            }
            assert_eq!(median_of_counts(values.len() as u64, runs), median_sorted(&values), "{values:?}");
        }
        // Fewer values than said: no median.
        assert_eq!(median_of_counts(3, [(1.0, 1)]), 0.0);
    }

    #[test]
    fn helpers_say_what_ends_an_instance() {
        assert!(says_stopped("Service STOPPED") && says_stopped("shutting down now") && !says_stopped("started"));
        assert!(is_crash_trigger(Level::Fatal, false));
        assert!(is_crash_trigger(Level::Error, true) && !is_crash_trigger(Level::Error, false));
        assert!(!is_crash_trigger(Level::Warning, true));
        assert_eq!(sample_id(7), 7);
        assert_eq!(sample_id(u64::MAX), u32::MAX);
    }

    #[test]
    fn layouts_bucket_time_and_clamp_the_last_bucket() {
        let layout = Layout {
            start: 0,
            end: 100,
            bucket: 10,
            buckets: 11,
        };
        assert_eq!(layout.index(5), 0);
        assert_eq!(layout.index(1_000), 10);
        assert_eq!(layout.span_of(9, 10), (90, 100));
        assert_eq!(Layout::over(&[], 200), None);
    }
}
