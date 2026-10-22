//! Live tail over a store (the ports): the live alerts raised after each refresh, alert mutes, and the per-file
//! tail status with lines per second. A [`LiveSession`] is one window's; the store reads the files
//! ([`crate::ports::Ingest::append_tail`]) and answers the queries the alerts are made of:
//!
//! - **alert rules** (`[[live.rule]]`): the events matching the rule's condition within its window, counted by the
//!   store, among the events newer than the newest one when the session started (so history opened with the files
//!   does not alert);
//! - **silence**: instances (the profile's group-by key) with a few recent events whose last event crossed
//!   `silence_after` since the previous evaluation (from the timeline's lanes over the recent window);
//! - **detectors** (error spike, crash, slow outlier, rate change, memory growth): the store's anomaly analysis
//!   over the recent window, throttled to `detect_every_ms`, reporting only findings that end after the previous
//!   run.
//!
//! An alert is raised at most once per cooldown for a kind and instance (or rule) and not at all while its kind or
//! rule is muted ([`AlertMutes`]). "Now" is the newest event's time, or the wall clock with `[live] clock = "wall"`.
//! (The in-memory workspace computes the same alerts from its records; this is the store's version of it.)

use crate::analysis::AnomalyKind;
use crate::api::{QueryRequest, Severity, TimelineRequest};
use crate::batch::SourceId;
use crate::ports::{Analysis, Ingest, PageQuery, Profiles, Query as QueryPort, StoreResult, Tail, TailAppend, TailFile, TailRead};
use crate::query::Query;
use crate::workspace::{
    group_of, AlertMutes, AlertTarget, DeletedFiles, LiveAlert, LiveRules, TailFileChange, TailFileState, TailFileStatus, TailOptions,
    TailReport, TailStatus, TailTotals, WatchMode,
};
use anyhow::Result;
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::time::{Duration, Instant};

/// Cooldown entries kept before old ones are pruned.
const MAX_COOLDOWNS: usize = 10_000;
/// An instance needs this many recent events before its silence is worth reporting.
const MIN_EVENTS_FOR_SILENCE: usize = 3;
/// Instances looked at for silence.
const SILENCE_LANES: usize = 2_000;
/// Window of the lines-per-second rate.
const RATE_WINDOW: Duration = Duration::from_secs(10);
const RATE_SAMPLES: usize = 64;

/// The store ports a live session reads.
pub trait LivePorts: QueryPort + Analysis + Profiles + Tail + Ingest {}

impl<T: QueryPort + Analysis + Profiles + Tail + Ingest + ?Sized> LivePorts for T {}

/// Lines per second over the last seconds.
#[derive(Debug, Clone, Default)]
struct Rate {
    samples: VecDeque<(Instant, u64)>,
}

impl Rate {
    fn add(&mut self, now: Instant, lines: u64) {
        self.samples.push_back((now, lines));
        if self.samples.len() > RATE_SAMPLES {
            self.samples.pop_front();
        }
    }

    fn per_sec(&self, now: Instant) -> f64 {
        let lines: u64 = self
            .samples
            .iter()
            .filter(|(t, _)| now.duration_since(*t) <= RATE_WINDOW)
            .map(|(_, n)| n)
            .sum();
        lines as f64 / RATE_WINDOW.as_secs_f64()
    }
}

/// What the session last saw of a file.
#[derive(Debug, Clone, Default)]
struct FileSeen {
    lines: u64,
    rate: Rate,
}

/// One window's live tail: alert state, mutes and file rates. Mutes and cooldowns outlive stopping and starting
/// live tail; [`LiveSession::start`] only resets where new events begin.
pub struct LiveSession {
    mutes: AlertMutes,
    cooldowns: HashMap<(String, Option<String>), i64>,
    /// The newest event's time when the session started: rules count only events after it.
    baseline: Option<i64>,
    prev_now: Option<i64>,
    prev_detect: Option<i64>,
    last_detect_at: Option<Instant>,
    files: HashMap<u32, FileSeen>,
}

impl Default for LiveSession {
    fn default() -> Self {
        LiveSession::with_mutes(AlertMutes::default())
    }
}

/// The wall clock in epoch ms.
fn wall_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn kind_severity(kind: AnomalyKind) -> Severity {
    match kind {
        AnomalyKind::Crash | AnomalyKind::ErrorSpike => Severity::Critical,
        AnomalyKind::Silence | AnomalyKind::SlowOutlier | AnomalyKind::MemoryGrowth => Severity::Warning,
        AnomalyKind::RateChange => Severity::Info,
    }
}

impl LiveSession {
    /// A session whose mutes are timed with their own clock (tests pass a fake one).
    pub fn with_mutes(mutes: AlertMutes) -> LiveSession {
        LiveSession {
            mutes,
            cooldowns: HashMap::new(),
            baseline: None,
            prev_now: None,
            prev_detect: None,
            last_detect_at: None,
            files: HashMap::new(),
        }
    }

    /// Live tail starts (again): only events newer than the store's newest one count for alert rules.
    pub fn start(&mut self, store: &(impl LivePorts + ?Sized)) -> StoreResult<()> {
        self.baseline = store.summary()?.to;
        self.prev_now = None;
        self.prev_detect = None;
        self.last_detect_at = None;
        Ok(())
    }

    /// Mutes alerts of one kind or rule until `until_ms` (wall clock, at most a day ahead).
    pub fn mute(&mut self, target: AlertTarget, until_ms: i64) -> Result<()> {
        self.mutes.mute(target, until_ms)
    }

    /// Lifts a mute; true when there was one.
    pub fn unmute(&mut self, target: &AlertTarget) -> bool {
        self.mutes.unmute(target)
    }

    /// What a refresh changed, with the alerts it raised: `appended` is what the store read.
    pub fn after_refresh(&mut self, store: &(impl LivePorts + ?Sized), appended: &TailAppend, options: &TailOptions) -> TailReport {
        let started = Instant::now();
        let mut report = TailReport {
            appended: appended.appended,
            sources_changed: appended.sources_changed.clone(),
            added: appended.added.clone(),
            truncated: appended.truncated.clone(),
            reimported: appended.reimported.clone(),
            deleted: appended.deleted.clone(),
            ..TailReport::default()
        };
        let states = [
            (&appended.added, TailFileState::Tailing),
            (&appended.truncated, TailFileState::Truncated),
            (&appended.reimported, TailFileState::Whole),
            (&appended.deleted, TailFileState::Deleted),
        ];
        for (ids, state) in states {
            report.file_states.extend(ids.iter().map(|id| TailFileChange {
                source_id: *id,
                state: state.clone(),
            }));
        }
        // Files that disappeared lose their source when the option says so (the default keeps the history).
        if options.deleted_files == DeletedFiles::Drop {
            for id in &appended.deleted {
                if let Err(e) = store.remove_source(SourceId(*id)) {
                    log::warn!("the source {id} of a deleted file could not be removed: {e}");
                }
            }
        }
        if !appended.is_empty() {
            if let Ok(files) = store.tail_files() {
                self.observe(&files, Instant::now());
            }
        }
        match self.alerts(store, appended, options) {
            Ok((alerts, newest)) => {
                report.newest_ts = newest;
                report.alerts_raised = alerts.len();
                report.alerts = alerts;
            }
            Err(e) => log::warn!("live alerts could not be evaluated: {e}"),
        }
        report.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        report
    }

    /// Per-file live state (watched as `mode_of` says) and totals.
    pub fn status(&mut self, store: &(impl LivePorts + ?Sized), mode_of: impl Fn(&Path) -> WatchMode) -> StoreResult<TailStatus> {
        let files = store.tail_files()?;
        let now = Instant::now();
        self.observe(&files, now);
        let records = store.summary()?.total;
        let files: Vec<TailFileStatus> = files
            .iter()
            .map(|f| {
                let file = f.path.split(crate::archive::ENTRY_SEPARATOR).next().unwrap_or(&f.path);
                let state = if !Path::new(file).exists() {
                    TailFileState::Deleted
                } else if f.read == TailRead::Whole {
                    TailFileState::Whole
                } else {
                    TailFileState::Tailing
                };
                TailFileStatus {
                    source_id: f.source,
                    path: f.path.clone(),
                    name: f.name.clone(),
                    group: group_of(&f.name),
                    mode: mode_of(Path::new(file)),
                    offset: f.offset,
                    size: f.size.max(f.offset),
                    lines_per_sec: self.files.get(&f.source).map_or(0.0, |s| s.rate.per_sec(now)),
                    last_event: f.changed_ms,
                    state,
                }
            })
            .collect();
        let totals = TailTotals {
            files: files.len(),
            tailing: files.iter().filter(|f| f.state == TailFileState::Tailing).count(),
            errors: files.iter().filter(|f| matches!(f.state, TailFileState::Error { .. })).count(),
            bytes: files.iter().map(|f| f.offset).sum(),
            lines_per_sec: files.iter().map(|f| f.lines_per_sec).sum(),
            records,
        };
        Ok(TailStatus { files, totals })
    }

    /// Adds the lines each file grew by to its rate.
    fn observe(&mut self, files: &[TailFile], now: Instant) {
        let live: std::collections::HashSet<u32> = files.iter().map(|f| f.source).collect();
        self.files.retain(|id, _| live.contains(id));
        for f in files {
            match self.files.get_mut(&f.source) {
                Some(seen) => {
                    if f.lines > seen.lines {
                        seen.rate.add(now, f.lines - seen.lines);
                    }
                    seen.lines = f.lines;
                }
                None => {
                    self.files.insert(
                        f.source,
                        FileSeen {
                            lines: f.lines,
                            rate: Rate::default(),
                        },
                    );
                }
            }
        }
    }

    /// Records the alert unless its kind (or rule) is muted or one for the same key fired within the cooldown. A
    /// muted alert does not start the cooldown.
    fn cooled(&mut self, kind: &str, who: Option<&str>, now: i64, cooldown: i64) -> bool {
        if self.mutes.covers(kind, who) {
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
            let mut times: Vec<i64> = self.cooldowns.values().copied().collect();
            times.sort_unstable();
            let cutoff = times[times.len() / 2];
            self.cooldowns.retain(|_, last| *last > cutoff);
        }
        self.cooldowns.insert(key, now);
        true
    }

    /// The alerts of this refresh and the newest event's time.
    fn alerts(
        &mut self,
        store: &(impl LivePorts + ?Sized),
        appended: &TailAppend,
        options: &TailOptions,
    ) -> Result<(Vec<LiveAlert>, Option<i64>)> {
        let summary = store.summary()?;
        let rules = store.active_profile()?.live.compile()?;
        let now = if rules.wall_clock {
            wall_ms()
        } else {
            match summary.to {
                Some(t) => t,
                None => return Ok((Vec::new(), None)),
            }
        };
        let mut out = Vec::new();
        if !appended.is_empty() {
            self.rules(store, &rules, now, &mut out)?;
        }
        if rules.kinds.contains(&AnomalyKind::Silence) && self.prev_now.is_some_and(|p| p != now) {
            self.silences(store, &rules, now, &summary.default_group_by, &mut out)?;
        }
        if appended.appended > 0 && self.detect_due(options.detect_every_ms) {
            self.detectors(store, &rules, now, &mut out)?;
        }
        self.prev_now = Some(now);
        for a in &mut out {
            let who = a.rule.as_deref().or(a.instance.as_deref()).unwrap_or("");
            a.id = format!("{}:{who}:{}", a.kind, a.from.div_euclid(rules.cooldown.max(1)));
        }
        Ok((out, summary.to))
    }

    fn detect_due(&mut self, every_ms: u64) -> bool {
        let due = self.last_detect_at.is_none_or(|t| t.elapsed() >= Duration::from_millis(every_ms));
        if due {
            self.last_detect_at = Some(Instant::now());
        }
        due
    }

    fn rules(&mut self, store: &(impl LivePorts + ?Sized), rules: &LiveRules, now: i64, out: &mut Vec<LiveAlert>) -> Result<()> {
        for rule in &rules.rules {
            let since = now.saturating_sub(rule.window);
            let from = match self.baseline {
                Some(b) => since.max(b.saturating_add(1)),
                None => since,
            };
            let query = Query {
                ql: rule.query.ql.clone(),
                from: Some(from),
                to: Some(now),
                ..Query::default()
            };
            let page = store.page(&PageQuery {
                query,
                offset: 0,
                limit: 1,
                descending: false,
                anchor: None,
            })?;
            if page.total < rule.threshold {
                continue;
            }
            let first = page.rows.first().and_then(|r| r.ts).unwrap_or(from);
            if !self.cooled("rule", Some(&rule.name), now, rules.cooldown) {
                continue;
            }
            out.push(LiveAlert {
                id: String::new(),
                kind: "rule".into(),
                rule: Some(rule.name.clone()),
                instance: None,
                severity: rule.severity,
                from: first,
                to: now,
                message: format!(
                    "{}: {} matching events in the last {}",
                    rule.name,
                    page.total,
                    crate::analysis::util::fmt_duration(rule.window)
                ),
                evidence_query: Query {
                    ql: rule.query.ql.clone(),
                    from: Some(first),
                    to: Some(now),
                    ..Query::default()
                },
            });
        }
        Ok(())
    }

    fn silences(
        &mut self,
        store: &(impl LivePorts + ?Sized),
        rules: &LiveRules,
        now: i64,
        group_by: &[String],
        out: &mut Vec<LiveAlert>,
    ) -> Result<()> {
        let Some(prev) = self.prev_now else { return Ok(()) };
        let timeline = store.timeline(&TimelineRequest {
            query: Query {
                from: Some(now.saturating_sub(rules.silence_after.saturating_add(rules.window))),
                to: Some(now),
                group_by: group_by.to_vec(),
                ..Query::default()
            },
            buckets: 1,
            always_show_lifecycle: false,
        })?;
        let mut silent: Vec<(String, i64)> = timeline
            .lanes
            .into_iter()
            .take(SILENCE_LANES)
            .filter(|l| l.total >= MIN_EVENTS_FOR_SILENCE)
            .filter(|l| now - l.last >= rules.silence_after && prev - l.last < rules.silence_after)
            .map(|l| (l.key, l.last))
            .collect();
        silent.sort();
        for (key, last) in silent {
            if !self.cooled("silence", Some(&key), now, rules.cooldown) {
                continue;
            }
            out.push(LiveAlert {
                id: String::new(),
                kind: "silence".into(),
                rule: None,
                instance: Some(key.clone()),
                severity: kind_severity(AnomalyKind::Silence),
                from: last,
                to: now,
                message: format!("{key} has not logged for {}", crate::analysis::util::fmt_duration(now - last)),
                evidence_query: Query {
                    from: Some(last - rules.window),
                    to: Some(now),
                    groups: vec![key],
                    ..Query::default()
                },
            });
        }
        Ok(())
    }

    fn detectors(&mut self, store: &(impl LivePorts + ?Sized), rules: &LiveRules, now: i64, out: &mut Vec<LiveAlert>) -> Result<()> {
        let found = store.anomalies(&QueryRequest {
            query: Query {
                from: Some(now.saturating_sub(rules.window)),
                to: Some(now),
                ..Query::default()
            },
        })?;
        let prev = self.prev_detect.replace(now);
        for a in found.result {
            // Silence has its own live detector; older findings were reported (or not) before.
            if a.kind == AnomalyKind::Silence || !rules.kinds.contains(&a.kind) || prev.is_some_and(|p| a.to < p) {
                continue;
            }
            let kind = crate::workspace::kind_name(a.kind);
            if !self.cooled(kind, a.instance.as_deref(), now, rules.cooldown) {
                continue;
            }
            let levels = match a.kind {
                AnomalyKind::ErrorSpike | AnomalyKind::Crash => vec![crate::model::Level::Error, crate::model::Level::Fatal],
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rate_keeps_a_bounded_history_and_counts_only_its_window() {
        let t0 = Instant::now();
        let mut rate = Rate::default();
        for i in 0..(RATE_SAMPLES as u32 + 20) {
            rate.add(t0, u64::from(i % 2) + 1);
        }
        assert_eq!(rate.samples.len(), RATE_SAMPLES, "the oldest samples are dropped");
        assert!(rate.per_sec(t0) > 0.0);
        // Everything is outside the window a while later.
        assert_eq!(rate.per_sec(t0 + RATE_WINDOW + Duration::from_secs(1)), 0.0);
    }

    #[test]
    fn a_cooldown_blocks_repeats_until_it_passes_and_a_mute_does_not_start_one() {
        let mut s = LiveSession::default();
        assert!(s.cooled("rule", Some("a"), 1_000, 500));
        assert!(!s.cooled("rule", Some("a"), 1_400, 500), "within the cooldown");
        assert!(s.cooled("rule", Some("b"), 1_400, 500), "another rule is independent");
        assert!(s.cooled("rule", Some("a"), 1_500, 500), "after the cooldown");
        // A clock that went backwards does not block a new alert forever.
        assert!(s.cooled("rule", Some("a"), 100, 500));
        s.mute(AlertTarget::Kind(AnomalyKind::Silence), wall_ms() + 60_000).unwrap();
        assert!(!s.cooled("silence", Some("x"), 5_000, 500));
        assert!(s.unmute(&AlertTarget::Kind(AnomalyKind::Silence)));
        assert!(
            s.cooled("silence", Some("x"), 5_000, 500),
            "the muted attempt left no cooldown behind"
        );
    }

    #[test]
    fn cooldown_memory_is_bounded() {
        let mut s = LiveSession::default();
        let cooldown = 1_000_000;
        for i in 0..MAX_COOLDOWNS as i64 {
            assert!(s.cooled("rule", Some(&format!("r{i}")), i, cooldown));
        }
        assert_eq!(s.cooldowns.len(), MAX_COOLDOWNS);
        // Everything is still cooling: the next one prunes the older half instead of growing without bound.
        assert!(s.cooled("rule", Some("fresh"), MAX_COOLDOWNS as i64, cooldown));
        assert!(s.cooldowns.len() <= MAX_COOLDOWNS / 2 + 2, "{}", s.cooldowns.len());
        assert!(s.cooldowns.contains_key(&("rule".to_string(), Some("fresh".to_string()))));
        // Entries whose cooldown has passed are pruned first.
        let mut s = LiveSession::default();
        for i in 0..MAX_COOLDOWNS as i64 {
            s.cooled("rule", Some(&format!("r{i}")), i, 10);
        }
        assert!(s.cooled("rule", Some("fresh"), 1_000_000, 10));
        assert_eq!(s.cooldowns.len(), 1);
    }

    #[test]
    fn severities_follow_the_kind_of_finding() {
        assert_eq!(kind_severity(AnomalyKind::Crash), Severity::Critical);
        assert_eq!(kind_severity(AnomalyKind::ErrorSpike), Severity::Critical);
        assert_eq!(kind_severity(AnomalyKind::Silence), Severity::Warning);
        assert_eq!(kind_severity(AnomalyKind::SlowOutlier), Severity::Warning);
        assert_eq!(kind_severity(AnomalyKind::MemoryGrowth), Severity::Warning);
        assert_eq!(kind_severity(AnomalyKind::RateChange), Severity::Info);
    }

    #[test]
    fn the_wall_clock_is_the_current_time() {
        let before = chrono::Utc::now().timestamp_millis();
        let now = wall_ms();
        assert!(now >= before && now - before < 5_000);
    }
}
