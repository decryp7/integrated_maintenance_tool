//! Live detectors: after each appended batch the anomaly detectors run on the
//! recent window only, instances that stop logging are reported, and user
//! alert rules (a query-language condition, N matches in T) are counted
//! incrementally. New findings become [`LiveAlert`]s, deduplicated with a
//! cooldown per kind and instance (or rule); muted kinds and rules
//! ([`AlertMutes`]) are not raised at all.

use super::mutes::AlertMutes;
use super::types::LiveAlert;
use crate::analysis::{self, AnomalyConfig, AnomalyKind};
use crate::api::Severity;
use crate::model::{Level, Record};
use crate::query::{group_key, CompiledQuery, Query};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// Most recent records the live detectors look at per evaluation.
pub const MAX_LIVE_ANALYSED: usize = 100_000;
/// Timestamps remembered per alert rule (its threshold is capped to this).
const MAX_RULE_HITS: usize = 10_000;
/// Cooldown entries kept before old ones are pruned.
const MAX_COOLDOWNS: usize = 10_000;
/// An instance needs this many events before its silence is worth reporting.
const MIN_EVENTS_FOR_SILENCE: usize = 3;

/// `[live]` in the profile: live detectors and alert rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveCfg {
    /// Detectors raising live alerts: error_spike, silence, crash,
    /// slow_outlier, rate_change, memory_growth.
    pub alerts: Vec<String>,
    /// An instance silent this long is reported ("60s").
    pub silence_after: String,
    /// The same kind of alert for the same instance is raised at most once per cooldown.
    pub cooldown: String,
    /// Recent window the detectors evaluate after each batch.
    pub window: String,
    /// "Now" for live windows and silence: "data" (newest record) or "wall" (system clock).
    pub clock: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rule: Vec<AlertRuleCfg>,
}

impl Default for LiveCfg {
    fn default() -> Self {
        LiveCfg {
            alerts: ["error_spike", "silence", "crash", "slow_outlier", "rate_change", "memory_growth"]
                .map(String::from)
                .to_vec(),
            silence_after: "60s".into(),
            cooldown: "5m".into(),
            window: "5m".into(),
            clock: "data".into(),
            rule: Vec::new(),
        }
    }
}

/// `[[live.rule]]`: alert when `when` matches at least `threshold` events within `window`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertRuleCfg {
    pub name: String,
    /// A query-language condition, e.g. `level = "Error" and SourceContext = "Payment*"`.
    pub when: String,
    pub threshold: u32,
    pub window: String,
    /// critical, warning or info.
    pub severity: String,
}

impl Default for AlertRuleCfg {
    fn default() -> Self {
        AlertRuleCfg {
            name: String::new(),
            when: String::new(),
            threshold: 1,
            window: "1m".into(),
            severity: "warning".into(),
        }
    }
}

/// [`LiveCfg`] parsed and validated.
#[derive(Debug, Clone)]
pub struct LiveRules {
    pub kinds: Vec<AnomalyKind>,
    pub silence_after: i64,
    pub cooldown: i64,
    pub window: i64,
    pub wall_clock: bool,
    pub rules: Vec<AlertRule>,
}

#[derive(Debug, Clone)]
pub struct AlertRule {
    pub name: String,
    pub query: Query,
    pub threshold: usize,
    pub window: i64,
    pub severity: Severity,
}

impl LiveCfg {
    /// Parses durations, detector names and rule conditions; fails with a message naming the bad key.
    pub fn compile(&self) -> Result<LiveRules> {
        let kinds = self
            .alerts
            .iter()
            .map(|a| {
                kind_of(a).ok_or_else(|| {
                    anyhow!(
                        "live.alerts: unknown detector \"{a}\" (use error_spike, silence, crash, slow_outlier, rate_change, memory_growth)"
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let wall_clock = match self.clock.to_ascii_lowercase().as_str() {
            "data" | "" => false,
            "wall" => true,
            other => return Err(anyhow!("live.clock must be \"data\" or \"wall\", not \"{other}\"")),
        };
        let rules = self
            .rule
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let name = if r.name.trim().is_empty() {
                    format!("rule {}", i + 1)
                } else {
                    r.name.clone()
                };
                crate::ql::parse(&r.when).map_err(|e| anyhow!("live.rule \"{name}\": {e}"))?;
                if r.when.trim().is_empty() {
                    return Err(anyhow!("live.rule \"{name}\": `when` is empty"));
                }
                Ok(AlertRule {
                    query: Query {
                        ql: r.when.clone(),
                        ..Query::default()
                    },
                    threshold: (r.threshold.max(1) as usize).min(MAX_RULE_HITS),
                    window: duration_ms(&r.window).with_context(|| format!("live.rule \"{name}\".window"))?,
                    severity: severity_of(&r.severity)
                        .ok_or_else(|| anyhow!("live.rule \"{name}\".severity must be critical, warning or info"))?,
                    name,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(LiveRules {
            kinds,
            silence_after: duration_ms(&self.silence_after).context("live.silence_after")?,
            cooldown: duration_ms(&self.cooldown).context("live.cooldown")?,
            window: duration_ms(&self.window).context("live.window")?,
            wall_clock,
            rules,
        })
    }
}

fn kind_of(name: &str) -> Option<AnomalyKind> {
    let n: String = name
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    Some(match n.as_str() {
        "errorspike" => AnomalyKind::ErrorSpike,
        "silence" => AnomalyKind::Silence,
        "crash" => AnomalyKind::Crash,
        "slowoutlier" => AnomalyKind::SlowOutlier,
        "ratechange" => AnomalyKind::RateChange,
        "memorygrowth" => AnomalyKind::MemoryGrowth,
        _ => return None,
    })
}

/// The wire name of a kind (`errorSpike`, ...).
pub fn kind_name(kind: AnomalyKind) -> &'static str {
    match kind {
        AnomalyKind::ErrorSpike => "errorSpike",
        AnomalyKind::Silence => "silence",
        AnomalyKind::Crash => "crash",
        AnomalyKind::SlowOutlier => "slowOutlier",
        AnomalyKind::RateChange => "rateChange",
        AnomalyKind::MemoryGrowth => "memoryGrowth",
    }
}

fn severity_of(s: &str) -> Option<Severity> {
    Some(match s.to_ascii_lowercase().as_str() {
        "critical" | "error" => Severity::Critical,
        "warning" | "warn" | "" => Severity::Warning,
        "info" => Severity::Info,
        _ => return None,
    })
}

fn kind_severity(kind: AnomalyKind) -> Severity {
    match kind {
        AnomalyKind::Crash | AnomalyKind::ErrorSpike => Severity::Critical,
        AnomalyKind::Silence | AnomalyKind::SlowOutlier | AnomalyKind::MemoryGrowth => Severity::Warning,
        AnomalyKind::RateChange => Severity::Info,
    }
}

/// "60s", "5m", "1.5h", "250ms", "2d" or a bare number of milliseconds.
pub fn duration_ms(text: &str) -> Result<i64> {
    let t = text.trim();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (num, unit) = (&t[..split], t[split..].trim());
    let n: f64 = num
        .parse()
        .map_err(|_| anyhow!("\"{text}\" is not a duration (e.g. \"60s\", \"5m\", \"1h\")"))?;
    let factor = match unit.to_ascii_lowercase().as_str() {
        "" | "ms" => 1.0,
        "s" | "sec" | "secs" => 1_000.0,
        "m" | "min" | "mins" => 60_000.0,
        "h" | "hr" | "hours" => 3_600_000.0,
        "d" | "days" => 86_400_000.0,
        _ => return Err(anyhow!("\"{text}\": unknown unit \"{unit}\" (use ms, s, m, h or d)")),
    };
    let ms = n * factor;
    if !(1.0..=(i64::MAX / 4) as f64).contains(&ms) {
        return Err(anyhow!("\"{text}\" must be a positive duration"));
    }
    Ok(ms as i64)
}

/// The stable id of an alert (see [`LiveAlert::id`]).
fn alert_id(a: &LiveAlert, cooldown: i64) -> String {
    let who = a.rule.as_deref().or(a.instance.as_deref()).unwrap_or("");
    format!("{}:{who}:{}", a.kind, a.from.div_euclid(cooldown.max(1)))
}

#[derive(Debug, Clone, Copy, Default)]
pub struct InstanceSeen {
    pub last_ts: i64,
    pub count: usize,
}

/// What the live detectors remember between batches.
#[derive(Default)]
pub struct AlertState {
    cooldowns: HashMap<(String, Option<String>), i64>,
    rule_hits: Vec<VecDeque<i64>>,
    /// "Now" at the previous evaluation.
    prev_now: Option<i64>,
    /// "Now" at the previous detector run.
    prev_detect: Option<i64>,
    last_detect_at: Option<std::time::Instant>,
}

/// Inputs of one evaluation.
pub struct Batch<'a> {
    pub records: &'a [Record],
    /// Positions (ids) of the records this batch added.
    pub inserted: &'a [u32],
    pub names: &'a dyn Fn(u32) -> &'a str,
    pub group_by: &'a [String],
    pub marker_kinds: &'a [String],
    pub instances: &'a HashMap<String, InstanceSeen>,
    pub mutes: &'a AlertMutes,
    pub now: i64,
    pub detect: bool,
}

impl AlertState {
    /// Whether the detectors are due (they are throttled to every `every_ms`).
    pub fn detect_due(&mut self, every_ms: u64) -> bool {
        let due = self
            .last_detect_at
            .is_none_or(|t| t.elapsed() >= std::time::Duration::from_millis(every_ms));
        if due {
            self.last_detect_at = Some(std::time::Instant::now());
        }
        due
    }

    /// New alerts for this batch.
    pub fn evaluate(&mut self, rules: &LiveRules, b: &Batch) -> Vec<LiveAlert> {
        let mut out = Vec::new();
        self.rules(rules, b, &mut out);
        if rules.kinds.contains(&AnomalyKind::Silence) {
            self.silences(rules, b, &mut out);
        }
        if b.detect {
            self.detectors(rules, b, &mut out);
        }
        self.prev_now = Some(b.now);
        for a in &mut out {
            a.id = alert_id(a, rules.cooldown);
        }
        out
    }

    /// Records the alert unless its kind (or rule) is muted or one for the
    /// same key fired within the cooldown. A muted alert does not start the cooldown.
    fn cooled(&mut self, mutes: &AlertMutes, kind: &str, who: Option<&str>, now: i64, cooldown: i64) -> bool {
        if mutes.covers(kind, who) {
            return false;
        }
        let key = (kind.to_string(), who.map(str::to_string));
        if self.cooldowns.get(&key).is_some_and(|last| now - last < cooldown && now >= *last) {
            return false;
        }
        if self.cooldowns.len() >= MAX_COOLDOWNS {
            self.cooldowns.retain(|_, last| now - *last < cooldown);
        }
        if self.cooldowns.len() >= MAX_COOLDOWNS {
            // Still full of live entries: keep the newest half.
            let mut times: Vec<i64> = self.cooldowns.values().copied().collect();
            times.sort_unstable();
            let cutoff = times[times.len() / 2];
            self.cooldowns.retain(|_, last| *last > cutoff);
        }
        self.cooldowns.insert(key, now);
        true
    }

    fn rules(&mut self, rules: &LiveRules, b: &Batch, out: &mut Vec<LiveAlert>) {
        self.rule_hits.resize_with(rules.rules.len(), VecDeque::new);
        for (i, rule) in rules.rules.iter().enumerate() {
            let Ok(cq) = CompiledQuery::new(&rule.query, b.group_by, b.marker_kinds) else {
                continue;
            };
            let hits = &mut self.rule_hits[i];
            for &id in b.inserted {
                let Some(r) = b.records.get(id as usize) else { continue };
                if cq.matches(r, (b.names)(r.source)) {
                    hits.push_back(r.ts.unwrap_or(b.now));
                    if hits.len() > MAX_RULE_HITS {
                        hits.pop_front();
                    }
                }
            }
            let since = b.now - rule.window;
            while hits.front().is_some_and(|t| *t < since) {
                hits.pop_front();
            }
            if hits.len() >= rule.threshold {
                let from = hits.front().copied().unwrap_or(since);
                let n = hits.len();
                if self.cooled(b.mutes, "rule", Some(&rule.name), b.now, rules.cooldown) {
                    out.push(LiveAlert {
                        id: String::new(),
                        kind: "rule".into(),
                        rule: Some(rule.name.clone()),
                        instance: None,
                        severity: rule.severity,
                        from,
                        to: b.now,
                        message: format!(
                            "{}: {n} matching events in the last {}",
                            rule.name,
                            analysis::util::fmt_duration(rule.window)
                        ),
                        evidence_query: Query {
                            ql: rule.query.ql.clone(),
                            from: Some(from),
                            to: Some(b.now),
                            ..Query::default()
                        },
                    });
                }
            }
        }
    }

    fn silences(&mut self, rules: &LiveRules, b: &Batch, out: &mut Vec<LiveAlert>) {
        let Some(prev) = self.prev_now else { return };
        let mut silent: Vec<(&String, &InstanceSeen)> = b
            .instances
            .iter()
            .filter(|(_, s)| s.count >= MIN_EVENTS_FOR_SILENCE)
            // Crossed the threshold since the previous evaluation.
            .filter(|(_, s)| b.now - s.last_ts >= rules.silence_after && prev - s.last_ts < rules.silence_after)
            .collect();
        silent.sort_by(|a, b| a.0.cmp(b.0));
        for (key, seen) in silent {
            if self.cooled(b.mutes, "silence", Some(key), b.now, rules.cooldown) {
                out.push(LiveAlert {
                    id: String::new(),
                    kind: "silence".into(),
                    rule: None,
                    instance: Some(key.clone()),
                    severity: kind_severity(AnomalyKind::Silence),
                    from: seen.last_ts,
                    to: b.now,
                    message: format!("{key} has not logged for {}", analysis::util::fmt_duration(b.now - seen.last_ts)),
                    evidence_query: Query {
                        from: Some(seen.last_ts - rules.window),
                        to: Some(b.now),
                        groups: vec![key.clone()],
                        ..Query::default()
                    },
                });
            }
        }
    }

    fn detectors(&mut self, rules: &LiveRules, b: &Batch, out: &mut Vec<LiveAlert>) {
        let since = b.now - rules.window;
        let end = b.records.partition_point(|r| r.ts.is_some());
        let start = b.records[..end].partition_point(|r| r.ts.is_some_and(|t| t < since));
        let start = start.max(end.saturating_sub(MAX_LIVE_ANALYSED));
        let window = &b.records[start..end];
        let keys: Vec<String> = window.iter().map(|r| group_key(b.group_by, r, (b.names)(r.source))).collect();
        let found = analysis::detect(window.iter().zip(keys.iter().map(String::as_str)), &AnomalyConfig::default());
        let prev = self.prev_detect.replace(b.now);
        for a in found {
            // Silence has its own live detector; old findings were reported (or not) before.
            if a.kind == AnomalyKind::Silence || !rules.kinds.contains(&a.kind) || prev.is_some_and(|p| a.to < p) {
                continue;
            }
            let kind = kind_name(a.kind);
            if !self.cooled(b.mutes, kind, a.instance.as_deref(), b.now, rules.cooldown) {
                continue;
            }
            let levels = match a.kind {
                AnomalyKind::ErrorSpike | AnomalyKind::Crash => vec![Level::Error, Level::Fatal],
                _ => Vec::new(),
            };
            out.push(LiveAlert {
                id: String::new(),
                kind: kind.into(),
                rule: None,
                severity: kind_severity(a.kind),
                from: a.from,
                to: a.to,
                message: a.message.clone(),
                evidence_query: Query {
                    from: Some(a.from),
                    to: Some(a.to),
                    groups: a.instance.iter().cloned().collect(),
                    levels,
                    ..Query::default()
                },
                instance: a.instance,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config() {
        let rules = LiveCfg::default().compile().unwrap();
        assert_eq!(rules.kinds.len(), 6);
        assert_eq!((rules.silence_after, rules.cooldown, rules.window), (60_000, 300_000, 300_000));
        assert!(!rules.wall_clock);
        let mut cfg = LiveCfg {
            clock: "wall".into(),
            alerts: vec!["errorSpike".into(), "Memory-Growth".into()],
            ..LiveCfg::default()
        };
        cfg.rule.push(AlertRuleCfg {
            when: "level = \"Error\"".into(),
            threshold: 0,
            severity: "critical".into(),
            ..AlertRuleCfg::default()
        });
        let rules = cfg.compile().unwrap();
        assert!(rules.wall_clock);
        assert_eq!(rules.rules[0].name, "rule 1");
        assert_eq!(rules.rules[0].threshold, 1);
        assert_eq!(rules.rules[0].severity, Severity::Critical);
        for bad in [
            LiveCfg {
                alerts: vec!["nope".into()],
                ..LiveCfg::default()
            },
            LiveCfg {
                clock: "moon".into(),
                ..LiveCfg::default()
            },
            LiveCfg {
                cooldown: "5 parsecs".into(),
                ..LiveCfg::default()
            },
            LiveCfg {
                rule: vec![AlertRuleCfg::default()],
                ..LiveCfg::default()
            },
            LiveCfg {
                rule: vec![AlertRuleCfg {
                    when: "level >".into(),
                    ..AlertRuleCfg::default()
                }],
                ..LiveCfg::default()
            },
            LiveCfg {
                rule: vec![AlertRuleCfg {
                    when: "level = \"Error\"".into(),
                    severity: "meh".into(),
                    ..AlertRuleCfg::default()
                }],
                ..LiveCfg::default()
            },
        ] {
            assert!(bad.compile().is_err());
        }
    }

    #[test]
    fn durations() {
        assert_eq!(duration_ms("60s").unwrap(), 60_000);
        assert_eq!(duration_ms("1.5h").unwrap(), 5_400_000);
        assert_eq!(duration_ms("250").unwrap(), 250);
        assert_eq!(duration_ms("2d").unwrap(), 172_800_000);
        assert_eq!(duration_ms("5 min").unwrap(), 300_000);
        assert!(duration_ms("0s").is_err());
        assert!(duration_ms("x").is_err());
        assert!(duration_ms("5y").is_err());
        for (k, n) in [
            (AnomalyKind::ErrorSpike, "errorSpike"),
            (AnomalyKind::Silence, "silence"),
            (AnomalyKind::Crash, "crash"),
            (AnomalyKind::SlowOutlier, "slowOutlier"),
            (AnomalyKind::RateChange, "rateChange"),
            (AnomalyKind::MemoryGrowth, "memoryGrowth"),
        ] {
            assert_eq!(kind_name(k), n);
            assert_eq!(kind_of(n), Some(k));
        }
        assert_eq!(severity_of("info"), Some(Severity::Info));
        assert_eq!(kind_severity(AnomalyKind::RateChange), Severity::Info);
    }

    #[test]
    fn cooldown_and_throttle() {
        let mut s = AlertState::default();
        let m = AlertMutes::default();
        assert!(s.cooled(&m, "crash", Some("a"), 1_000, 500));
        assert!(!s.cooled(&m, "crash", Some("a"), 1_200, 500));
        assert!(s.cooled(&m, "crash", Some("b"), 1_200, 500));
        assert!(s.cooled(&m, "crash", Some("a"), 1_600, 500));
        // The cooldown table stays bounded.
        for i in 0..MAX_COOLDOWNS + 1 {
            s.cooled(&m, "rule", Some(&i.to_string()), 10_000, 500);
        }
        assert!(s.cooldowns.len() <= MAX_COOLDOWNS);
        for i in 0..MAX_COOLDOWNS {
            s.cooled(&m, "rule", Some(&i.to_string()), 10_000 + i as i64, 500_000);
        }
        assert!(s.cooldowns.len() <= MAX_COOLDOWNS);
        assert!(s.cooled(&m, "rule", Some("late"), 900_000, 500));
        assert!(s.cooldowns.len() < MAX_COOLDOWNS);
        assert!(s.detect_due(60_000));
        assert!(!s.detect_due(60_000));
        assert!(s.detect_due(0));
    }
}
