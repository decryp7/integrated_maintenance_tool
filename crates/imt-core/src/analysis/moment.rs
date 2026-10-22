//! "What was happening between from and to": the Moment summary, the health-check findings and trace
//! summaries, built from streams of events so any store can feed them (pure, no store access).

use super::anomaly::{Anomaly, AnomalyKind};
use super::trace::Span;
use crate::api::{
    ActivityItem, Finding, FindingAction, FindingKind, MetricWindow, MomentInstance, MomentSummary, Severity, SourceInfo, TraceSummary,
    UnrecognizedFile, UserCount,
};
use crate::exceptions::ExceptionGroup;
use crate::model::Level;
use crate::patterns::{MinerConfig, PatternMiner};
use crate::profile::ActivityCfg;
use crate::row::RowRef;
use std::collections::{BTreeMap, HashMap};

const MAX_ACTIVITY: usize = 200;
/// Most traces summarised.
pub const MAX_TRACES: usize = 200;

/// An event's id as the DTOs carry it (saturating until 6c widens them).
fn sid(r: &dyn RowRef) -> u32 {
    u32::try_from(r.id()).unwrap_or(u32::MAX)
}

/// The first of `fields` with a non-empty value on `r`, with the field's name.
fn first_present_field<'f>(r: &dyn RowRef, fields: &'f [String]) -> Option<(&'f str, String)> {
    fields
        .iter()
        .find_map(|f| r.text_of(f).filter(|v| !v.is_empty()).map(|v| (f.as_str(), v.into_owned())))
}

struct InstanceAcc {
    events: usize,
    warnings: usize,
    errors: usize,
    first_error: Option<u32>,
    miner: PatternMiner,
}

/// Collects the Moment summary from the events of the window (in time order) with their instance keys.
pub struct MomentBuilder<'c> {
    activity_cfg: &'c ActivityCfg,
    total: usize,
    levels: BTreeMap<Level, usize>,
    instances: BTreeMap<String, InstanceAcc>,
    users: HashMap<(String, String), u64>,
    activity: Vec<ActivityItem>,
    metrics: MetricCollector<'c>,
    from: i64,
}

impl<'c> MomentBuilder<'c> {
    pub fn new(activity_cfg: &'c ActivityCfg, from: i64) -> MomentBuilder<'c> {
        MomentBuilder {
            activity_cfg,
            total: 0,
            levels: Level::ALL.iter().map(|l| (*l, 0)).collect(),
            instances: BTreeMap::new(),
            users: HashMap::new(),
            activity: Vec::new(),
            metrics: MetricCollector::new(&activity_cfg.metrics),
            from,
        }
    }

    pub fn add(&mut self, r: &dyn RowRef, key: &str) {
        self.total += 1;
        *self.levels.entry(r.level()).or_default() += 1;
        if !self.instances.contains_key(key) {
            self.instances.insert(
                key.to_string(),
                InstanceAcc {
                    events: 0,
                    warnings: 0,
                    errors: 0,
                    first_error: None,
                    miner: PatternMiner::new(MinerConfig::default()),
                },
            );
        }
        if let Some(inst) = self.instances.get_mut(key) {
            inst.events += 1;
            inst.warnings += usize::from(r.level() == Level::Warning);
            if r.level() >= Level::Error {
                inst.errors += 1;
                inst.first_error = inst.first_error.or(Some(sid(r)));
            }
            inst.miner.add(r);
        }
        let user = first_present_field(r, &self.activity_cfg.user).map(|(field, value)| {
            *self.users.entry((field.to_string(), value.clone())).or_default() += 1;
            value
        });
        let action = first_present_field(r, &self.activity_cfg.action).map(|(_, v)| v);
        if (user.is_some() || action.is_some()) && self.activity.len() < MAX_ACTIVITY {
            self.activity.push(ActivityItem {
                id: sid(r),
                ts: r.ts().unwrap_or(self.from),
                instance: key.to_string(),
                user,
                action: action.unwrap_or_else(|| r.message().chars().take(200).collect()),
                level: r.level(),
            });
        }
        self.metrics.add(r, key);
    }

    /// The summary; `before` holds the metrics of the window of the same length just before, and
    /// `anomalies` those overlapping the window.
    pub fn finish(self, to: i64, before: MetricCollector<'_>, anomalies: Vec<Anomaly>) -> MomentSummary {
        let mut instances: Vec<MomentInstance> = self
            .instances
            .into_iter()
            .map(|(key, acc)| {
                let mut patterns = acc.miner.clusters();
                patterns.truncate(3);
                MomentInstance {
                    key,
                    events: acc.events,
                    warnings: acc.warnings,
                    errors: acc.errors,
                    patterns,
                    first_error: acc.first_error,
                }
            })
            .collect();
        instances.sort_by(|a, b| b.errors.cmp(&a.errors).then_with(|| b.events.cmp(&a.events)));
        let mut users: Vec<UserCount> = self
            .users
            .into_iter()
            .map(|((field, value), count)| UserCount { field, value, count })
            .collect();
        users.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.value.cmp(&b.value))
                .then_with(|| a.field.cmp(&b.field))
        });
        users.truncate(20);
        MomentSummary {
            from: self.from,
            to,
            total: self.total,
            levels: self.levels,
            instances,
            metrics: self.metrics.windows(&before),
            users,
            activity: self.activity,
            anomalies,
        }
    }
}

/// Numeric series of one window: per instance and metric field, and PerfMon counters (one value per
/// record, `Counter`/`Value`).
pub struct MetricCollector<'f> {
    fields: &'f [String],
    series: BTreeMap<(String, String), MetricAcc>,
}

/// What a window keeps of one series: no value per event.
#[derive(Debug, Clone, Copy)]
struct MetricAcc {
    samples: usize,
    sum: f64,
    min: f64,
    max: f64,
    last: f64,
}

impl Default for MetricAcc {
    fn default() -> Self {
        MetricAcc {
            samples: 0,
            sum: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            last: 0.0,
        }
    }
}

impl MetricAcc {
    fn push(&mut self, v: f64) {
        self.samples += 1;
        self.sum += v;
        self.min = self.min.min(v);
        self.max = self.max.max(v);
        self.last = v;
    }

    fn avg(&self) -> f64 {
        self.sum / self.samples as f64
    }
}

impl<'f> MetricCollector<'f> {
    pub fn new(fields: &'f [String]) -> MetricCollector<'f> {
        MetricCollector {
            fields,
            series: BTreeMap::new(),
        }
    }

    pub fn add(&mut self, r: &dyn RowRef, key: &str) {
        if let (Some(counter), Some(value)) = (r.text_of("Counter"), r.number_of("Value")) {
            let machine = r.text_of("MachineName").map(|m| m.into_owned()).unwrap_or_default();
            let series = match r.text_of("Instance") {
                Some(i) if !i.is_empty() => format!("{machine} · {i}"),
                _ => machine,
            };
            self.series.entry((series, counter.into_owned())).or_default().push(value);
            return;
        }
        for f in self.fields {
            if let Some(v) = r.number_of(f) {
                self.series.entry((key.to_string(), f.clone())).or_default().push(v);
            }
        }
    }

    /// Adds `samples` values summing to `sum` to a series (a store that aggregated a window itself; the
    /// minimum, maximum and last value are not known, so this suits the window before, of which only the
    /// average is shown).
    pub fn add_summary(&mut self, series: String, metric: String, samples: usize, sum: f64) {
        if samples == 0 {
            return;
        }
        let acc = self.series.entry((series, metric)).or_default();
        acc.samples += samples;
        acc.sum += sum;
    }

    fn windows(self, before: &MetricCollector<'_>) -> Vec<MetricWindow> {
        let mut result: Vec<MetricWindow> = self
            .series
            .into_iter()
            .filter(|(_, acc)| acc.samples > 0)
            .map(|((series, metric), acc)| {
                let avg = acc.avg();
                let previous_avg = before.series.get(&(series.clone(), metric.clone())).map(MetricAcc::avg);
                let change_pct = previous_avg.filter(|p| *p != 0.0).map(|p| (avg - p) / p.abs() * 100.0);
                MetricWindow {
                    samples: acc.samples,
                    min: acc.min,
                    max: acc.max,
                    last: acc.last,
                    avg,
                    previous_avg,
                    change_pct,
                    series,
                    metric,
                }
            })
            .collect();
        result.sort_by(|a, b| {
            let ca = a.change_pct.map(f64::abs).unwrap_or(0.0);
            let cb = b.change_pct.map(f64::abs).unwrap_or(0.0);
            cb.total_cmp(&ca)
                .then_with(|| a.series.cmp(&b.series))
                .then_with(|| a.metric.cmp(&b.metric))
        });
        result
    }
}

/// The traces among `spans`, longest first (at most [`MAX_TRACES`]).
pub fn trace_summaries(spans: &[Span]) -> Vec<TraceSummary> {
    let mut by_trace: BTreeMap<&str, Vec<&Span>> = BTreeMap::new();
    for s in spans {
        by_trace.entry(s.trace_id.as_str()).or_default().push(s);
    }
    let mut out: Vec<TraceSummary> = by_trace
        .into_iter()
        .map(|(id, spans)| {
            let start = spans.iter().map(|s| s.start).min().unwrap_or(0);
            let end = spans.iter().map(|s| s.start as f64 + s.duration_ms).fold(start as f64, f64::max);
            let root = spans
                .iter()
                .find(|s| s.parent.is_none())
                .or(spans.first())
                .map(|s| s.name.clone())
                .unwrap_or_default();
            TraceSummary {
                trace_id: id.to_string(),
                spans: spans.len(),
                errors: spans.iter().map(|s| s.errors).sum(),
                start,
                duration_ms: end - start as f64,
                root,
            }
        })
        .collect();
    out.sort_by(|a, b| b.duration_ms.total_cmp(&a.duration_ms).then_with(|| a.trace_id.cmp(&b.trace_id)));
    out.truncate(MAX_TRACES);
    out
}

/// What the health check knows besides the anomalies and exceptions.
pub struct OverviewInputs<'a> {
    pub sources: &'a [SourceInfo],
    pub unrecognized: &'a [UnrecognizedFile],
    /// Quarantined plugins: name and last error.
    pub quarantined: Vec<(String, String)>,
}

/// "System.Net.Http.HttpRequestException" -> "HttpRequestException".
fn short_type(t: &str) -> &str {
    t.rsplit('.').next().unwrap_or(t)
}

/// Most anomalies the health check lists as findings; the rest are summarised by one "not shown" finding. A noisy
/// 1M-event log can hold thousands of anomalies, which no one reads and which froze the UI when all were rendered.
pub const MAX_ANOMALY_FINDINGS: usize = 100;

fn anomaly_severity(kind: AnomalyKind) -> Severity {
    match kind {
        AnomalyKind::Crash => Severity::Critical,
        AnomalyKind::ErrorSpike | AnomalyKind::MemoryGrowth | AnomalyKind::Silence => Severity::Warning,
        AnomalyKind::SlowOutlier | AnomalyKind::RateChange => Severity::Info,
    }
}

/// The anomalies worth listing, at most [`MAX_ANOMALY_FINDINGS`]: most severe kinds first, then the highest score, then
/// the earliest (so the order is stable). Also the number left out.
fn ranked_anomalies(anomalies: &[Anomaly]) -> (Vec<&Anomaly>, usize) {
    let mut ranked: Vec<&Anomaly> = anomalies.iter().collect();
    // Selecting the top N is O(n) on average; only those N are then fully ordered.
    let key = |a: &&Anomaly, b: &&Anomaly| {
        anomaly_severity(a.kind)
            .cmp(&anomaly_severity(b.kind))
            .then_with(|| b.score.total_cmp(&a.score))
            .then_with(|| a.from.cmp(&b.from))
            .then_with(|| a.message.cmp(&b.message))
    };
    if ranked.len() > MAX_ANOMALY_FINDINGS {
        ranked.select_nth_unstable_by(MAX_ANOMALY_FINDINGS, key);
        ranked.truncate(MAX_ANOMALY_FINDINGS);
    }
    ranked.sort_by(key);
    (ranked, anomalies.len().saturating_sub(MAX_ANOMALY_FINDINGS))
}

/// The automatic health check: what went wrong and what needs attention, most severe first, each with a
/// suggested action. At most [`MAX_ANOMALY_FINDINGS`] anomalies are listed (the highest ranked); a final finding says
/// how many were left out.
pub fn findings(anomalies: &[Anomaly], groups: &[ExceptionGroup], inputs: &OverviewInputs<'_>) -> Vec<Finding> {
    let mut out = Vec::new();
    let (listed, left_out) = ranked_anomalies(anomalies);
    for a in listed {
        let severity = anomaly_severity(a.kind);
        let (kind, title) = match a.kind {
            AnomalyKind::Crash => (FindingKind::Crash, "Crash"),
            AnomalyKind::ErrorSpike => (FindingKind::ErrorSpike, "Error spike"),
            AnomalyKind::MemoryGrowth => (FindingKind::MemoryGrowth, "Memory growth"),
            AnomalyKind::Silence => (FindingKind::Silence, "Went silent"),
            AnomalyKind::SlowOutlier => (FindingKind::SlowOutlier, "Slow outliers"),
            AnomalyKind::RateChange => (FindingKind::RateChange, "Rate change"),
        };
        out.push(Finding {
            kind,
            severity,
            title: title.into(),
            detail: a.message.clone(),
            action: Some(FindingAction::Zoom {
                from: a.from,
                to: a.to,
                record_id: a.sample_ids.first().copied(),
            }),
        });
    }
    if left_out > 0 {
        out.push(Finding {
            kind: FindingKind::NotShown,
            severity: Severity::Info,
            title: "More anomalies not listed".into(),
            detail: format!("{left_out} lower-ranked anomalies are not listed; narrow the time range or filters to see them"),
            action: None,
        });
    }
    for g in groups.iter().filter(|g| g.count >= 5).take(5) {
        let instances = g.instances.len();
        out.push(Finding {
            kind: FindingKind::Exception,
            severity: Severity::Warning,
            title: format!("{} ×{}", short_type(&g.type_name), g.count),
            detail: format!(
                "{} on {} instance{}{}",
                g.message.chars().take(160).collect::<String>(),
                instances,
                if instances == 1 { "" } else { "s" },
                g.top_frame.as_deref().map(|f| format!(", at {f}")).unwrap_or_default()
            ),
            action: Some(FindingAction::Exception {
                fingerprint: g.fingerprint.clone(),
                record_id: g.sample_ids.first().copied(),
            }),
        });
    }
    for s in inputs.sources {
        let lines = s.records + s.skipped;
        if s.skipped > 0 && s.skipped * 5 > lines {
            out.push(Finding {
                kind: FindingKind::UnreadLines,
                severity: Severity::Warning,
                title: "Lines not understood".into(),
                detail: format!(
                    "{}: {}% of the lines could not be read as events",
                    s.name,
                    s.skipped * 100 / lines.max(1)
                ),
                action: Some(FindingAction::SetupFormat { path: s.path.clone() }),
            });
        }
        if s.from.is_none() && s.records > 0 {
            out.push(Finding {
                kind: FindingKind::NoTimestamps,
                severity: Severity::Info,
                title: "No timestamps".into(),
                detail: format!("{} has no recognisable timestamps, so it is missing from the timeline", s.name),
                action: Some(FindingAction::SetupFormat { path: s.path.clone() }),
            });
        }
    }
    for u in inputs.unrecognized {
        out.push(Finding {
            kind: FindingKind::FileNotOpened,
            severity: Severity::Info,
            title: "File not opened".into(),
            detail: format!("{}: {}", u.name, u.reason),
            action: Some(FindingAction::SetupFormat { path: u.path.clone() }),
        });
    }
    let dropped: usize = inputs.sources.iter().map(|s| s.dropped).sum();
    if dropped > 0 {
        out.push(Finding {
            kind: FindingKind::Dropped,
            severity: Severity::Info,
            title: "Events removed by the pipeline".into(),
            detail: format!("{dropped} events were dropped by the profile's cleaning steps (your files are unchanged)"),
            action: Some(FindingAction::OpenPanel { panel: "pipeline".into() }),
        });
    }
    for (name, error) in &inputs.quarantined {
        out.push(Finding {
            kind: FindingKind::PluginQuarantined,
            severity: Severity::Warning,
            title: "Plugin disabled after crashes".into(),
            detail: format!("{name}: {error}"),
            action: Some(FindingAction::OpenPanel { panel: "plugins".into() }),
        });
    }
    out.sort_by_key(|f| f.severity);
    out
}

/// The queries of a Moment from `base` (without a live window): the window itself, the window of the
/// same length just before it (for metric changes) and all time (for anomalies).
pub fn moment_queries(base: &crate::query::Query, from: i64, to: i64) -> [crate::query::Query; 3] {
    use crate::query::Query;
    let span = (to - from).max(1);
    [
        Query {
            from: Some(from),
            to: Some(to),
            ..base.clone()
        },
        Query {
            from: Some(from - span),
            to: Some(from - 1),
            ..base.clone()
        },
        Query {
            from: None,
            to: None,
            ..base.clone()
        },
    ]
}
