//! Analysis module: synthetic scenarios mirroring /samples plus the real samples.

use imt_core::analysis::{
    compare_windows, correlate, detect, field_stats, request_durations, spans, waterfall, Anomaly, AnomalyConfig, AnomalyKind,
    CompareConfig,
};
use imt_core::model::{Level, Record};
use imt_core::query::group_key;
use imt_core::{Profile, Workspace};
use serde_json::json;
use std::path::PathBuf;

/// 2026-10-14T09:00:00Z
const T0: i64 = 1_791_968_400_000;
const MIN: i64 = 60_000;
const SEC: i64 = 1000;

fn rec(id: u32, ts: Option<i64>, level: Level) -> Record {
    let mut r = Record::new(id);
    r.id = id;
    r.ts = ts;
    r.level = level;
    r.message = format!("event {id}");
    r
}

fn with(mut r: Record, name: &str, v: serde_json::Value) -> Record {
    r.fields.insert(name.into(), v);
    r
}

/// Keeps ids unique and pairs every record with its instance key.
struct Log {
    recs: Vec<(Record, String)>,
}

impl Log {
    fn new() -> Self {
        Log { recs: Vec::new() }
    }
    fn add(&mut self, key: &str, mut r: Record) {
        r.id = self.recs.len() as u32;
        self.recs.push((r, key.to_string()));
    }
    fn steady(&mut self, key: &str, from: i64, to: i64, every: i64) {
        let mut t = from;
        while t < to {
            self.add(key, rec(0, Some(t), Level::Information));
            t += every;
        }
    }
    fn detect(&self, cfg: &AnomalyConfig) -> Vec<Anomaly> {
        detect(self.recs.iter().map(|(r, k)| (r, k.as_str())), cfg)
    }
}

fn of_kind(v: &[Anomaly], kind: AnomalyKind) -> Vec<&Anomaly> {
    v.iter().filter(|a| a.kind == kind).collect()
}

/// WEB01 healthy for 3 h; WEB02 . 4120 climbs into errors and dies at 10:41:24,
/// WEB02 . 5532 starts 46 s later.
fn incident() -> Log {
    let mut log = Log::new();
    log.steady("WEB01 · 3312", T0, T0 + 180 * MIN, 3 * SEC);
    log.add("WEB01 · 3312", rec(0, Some(T0 + 180 * MIN), Level::Information));
    let crash = T0 + 101 * MIN + 24 * SEC;
    let mut t = T0;
    let mut n = 0;
    while t < crash {
        let ramp = t >= T0 + 90 * MIN && n % 3 == 0;
        let level = if ramp { Level::Error } else { Level::Information };
        log.add("WEB02 · 4120", rec(0, Some(t), level));
        t += 3 * SEC;
        n += 1;
    }
    let mut fatal = rec(0, Some(crash), Level::Fatal);
    fatal.message = "Host terminated unexpectedly".into();
    fatal.exception = Some("System.TimeoutException".into());
    log.add("WEB02 · 4120", fatal);
    log.steady("WEB02 · 5532", crash + 46 * SEC, T0 + 180 * MIN, 3 * SEC);
    log.steady("WEB02 · –", T0, T0 + 180 * MIN, MIN);
    log
}

#[test]
fn incident_scenario_finds_spike_and_crash_with_restart() {
    let log = incident();
    let found = log.detect(&AnomalyConfig::default());
    let crash = of_kind(&found, AnomalyKind::Crash);
    assert_eq!(crash.len(), 1, "{found:#?}");
    let c = crash[0];
    assert_eq!(c.instance.as_deref(), Some("WEB02 · 4120"));
    assert_eq!(c.from, T0 + 101 * MIN + 24 * SEC);
    assert_eq!(c.to, c.from + 46 * SEC);
    assert!(c.message.starts_with("WEB02 · 4120 crashed at 10:41:24"), "{}", c.message);
    assert!(c.message.contains("restarted as WEB02 · 5532 46 s later"), "{}", c.message);
    assert!(c.sample_ids.len() <= 5 && !c.sample_ids.is_empty());
    assert_eq!(c.score, 10.0);

    let spikes = of_kind(&found, AnomalyKind::ErrorSpike);
    assert_eq!(spikes.len(), 1, "{spikes:#?}");
    let s = spikes[0];
    assert_eq!(s.instance.as_deref(), Some("WEB02 · 4120"));
    assert!(s.to <= c.from + 60_000 && s.from < c.from);
    assert!(s.message.starts_with("Error rate on WEB02 · 4120 rose to "), "{}", s.message);
    assert!(s.message.ends_with("(baseline 0/min)"), "{}", s.message);
    assert_eq!(s.sample_ids.len(), 5);
    assert!(s.score >= 4.0);

    // The quiet servers stay quiet and the crashed one is not also "silent".
    assert!(of_kind(&found, AnomalyKind::Silence)
        .iter()
        .all(|a| a.instance.as_deref() != Some("WEB02 · 4120")));
    assert!(found.windows(2).all(|w| w[0].from <= w[1].from));
}

#[test]
fn empty_single_and_untimed_input() {
    let cfg = AnomalyConfig::default();
    assert!(detect(std::iter::empty::<(&imt_core::model::Record, &str)>(), &cfg).is_empty());

    let mut log = Log::new();
    log.add("A", rec(0, Some(T0), Level::Error));
    assert!(log.detect(&cfg).is_empty());

    let mut log = Log::new();
    for _ in 0..30 {
        log.add("A", rec(0, None, Level::Fatal));
    }
    assert!(log.detect(&cfg).is_empty());
}

#[test]
fn constant_series_has_no_spike_but_a_burst_over_zero_mad_does() {
    // Same error count in every bucket: MAD is zero and nothing stands out.
    let mut log = Log::new();
    for m in 0..60 {
        for k in 0..6 {
            log.add("A", rec(0, Some(T0 + m * MIN + k * SEC), Level::Error));
        }
    }
    let found = log.detect(&AnomalyConfig::default());
    assert!(of_kind(&found, AnomalyKind::ErrorSpike).is_empty(), "{found:#?}");

    // Silence everywhere except one minute with 30 errors (MAD = 0, median = 0).
    let mut log = Log::new();
    log.steady("A", T0, T0 + 60 * MIN, 5 * SEC);
    for k in 0..30 {
        log.add("A", rec(0, Some(T0 + 30 * MIN + k * SEC), Level::Error));
    }
    let found = log.detect(&AnomalyConfig::default());
    let spikes = of_kind(&found, AnomalyKind::ErrorSpike);
    assert_eq!(spikes.len(), 1);
    assert!(spikes[0].message.contains("rose to 60/min"), "{}", spikes[0].message);
    // Raising the bar removes it.
    let strict = AnomalyConfig {
        min_count: 100,
        ..AnomalyConfig::default()
    };
    assert!(of_kind(&log.detect(&strict), AnomalyKind::ErrorSpike).is_empty());
}

#[test]
fn adjacent_flagged_buckets_merge_and_distant_ones_do_not() {
    let mut log = Log::new();
    log.steady("A", T0, T0 + 120 * MIN, 10 * SEC);
    for m in [20, 21, 22, 80] {
        for k in 0..20 {
            log.add("A", rec(0, Some(T0 + m * MIN + k * SEC), Level::Error));
        }
    }
    let found = log.detect(&AnomalyConfig::default());
    let spikes = of_kind(&found, AnomalyKind::ErrorSpike);
    assert_eq!(spikes.len(), 2, "{spikes:#?}");
    assert_eq!(spikes[0].to - spikes[0].from, 3 * MIN);
}

#[test]
fn silence_in_the_middle_and_at_the_end() {
    let mut log = Log::new();
    log.steady("A", T0, T0 + 10 * MIN, SEC);
    log.steady("A", T0 + 10 * MIN + 46 * SEC, T0 + 20 * MIN, SEC);
    log.steady("B", T0, T0 + 20 * MIN, SEC);
    log.steady("C", T0, T0 + 5 * MIN, SEC);
    let found = log.detect(&AnomalyConfig::default());
    let silent = of_kind(&found, AnomalyKind::Silence);
    assert_eq!(silent.len(), 2, "{silent:#?}");
    assert_eq!(silent[1].instance.as_deref(), Some("A"));
    assert!(
        silent[1].message.starts_with("A went silent at 09:09:59 (no events for 47 s)"),
        "{}",
        silent[1].message
    );
    assert_eq!(silent[0].instance.as_deref(), Some("C"));
    assert!(silent[0].message.contains("no events for 15 min)"), "{}", silent[0].message);
    assert!(silent[0].score > 1.0);

    // An instance that says it stopped is not silent, a sparse one is not "steady".
    let mut log = Log::new();
    log.steady("A", T0, T0 + 5 * MIN, SEC);
    let mut bye = rec(0, Some(T0 + 5 * MIN), Level::Information);
    bye.message = "Application is shutting down...".into();
    log.add("A", bye);
    log.steady("B", T0, T0 + 20 * MIN, SEC);
    log.steady("S", T0, T0 + 3 * MIN, MIN);
    assert!(of_kind(&log.detect(&AnomalyConfig::default()), AnomalyKind::Silence).is_empty());
}

#[test]
fn crash_from_exception_without_restart() {
    let mut log = Log::new();
    log.steady("M1 · 1", T0, T0 + 10 * MIN, SEC);
    log.steady("M2 · 9", T0, T0 + 20 * MIN, SEC);
    log.steady("M2 · 10", T0 + 15 * MIN, T0 + 20 * MIN, SEC);
    // M1 . 1 ends with an exception 2 s before its last event, M2 . 9 with a plain event.
    let mut boom = rec(0, Some(T0 + 10 * MIN - 2 * SEC), Level::Error);
    boom.exception = Some("boom".into());
    log.add("M1 · 1", boom);
    log.add("M1 · 1", rec(0, Some(T0 + 10 * MIN), Level::Information));
    // Too old an exception, and a warning with an exception, do not count.
    let mut old = rec(0, Some(T0 + 5 * MIN), Level::Error);
    old.exception = Some("old".into());
    log.add("M3 · 1", old);
    log.steady("M3 · 1", T0 + 5 * MIN + SEC, T0 + 8 * MIN, SEC);
    let mut warn = rec(0, Some(T0 + 8 * MIN), Level::Warning);
    warn.exception = Some("meh".into());
    log.add("M3 · 1", warn);
    let found = log.detect(&AnomalyConfig::default());
    let crashes = of_kind(&found, AnomalyKind::Crash);
    assert_eq!(crashes.len(), 1, "{crashes:#?}");
    let c = crashes[0];
    assert_eq!(c.instance.as_deref(), Some("M1 · 1"));
    assert_eq!(c.score, 5.0);
    assert!(!c.message.contains("restarted"));
    assert_eq!(c.to, c.from);
    assert_eq!(c.sample_ids.len(), 2);
    // A later start on another machine, or beyond the window, is not a restart.
    let mut log = Log::new();
    let mut fatal = rec(0, Some(T0 + 10 * MIN), Level::Fatal);
    fatal.message = "x\nsecond line".repeat(20);
    log.add("A · 1", fatal);
    log.steady("A · 1", T0, T0 + 10 * MIN, SEC);
    log.steady("B · 2", T0 + 10 * MIN + 5 * SEC, T0 + 20 * MIN, SEC);
    log.steady("A · 2", T0 + 13 * MIN, T0 + 20 * MIN, SEC);
    let found = log.detect(&AnomalyConfig::default());
    let c = of_kind(&found, AnomalyKind::Crash);
    assert_eq!(c.len(), 1);
    assert!(!c[0].message.contains("restarted"), "{}", c[0].message);
    assert!(!c[0].message.contains("second line"));
    // The nearest of several restart candidates wins.
    log.steady("A · 3", T0 + 10 * MIN + 30 * SEC, T0 + 20 * MIN, SEC);
    log.steady("A · 4", T0 + 10 * MIN + 10 * SEC, T0 + 20 * MIN, SEC);
    let found = log.detect(&AnomalyConfig::default());
    assert!(of_kind(&found, AnomalyKind::Crash)[0]
        .message
        .contains("restarted as A · 4 10 s later"));
}

#[test]
fn slow_outliers_per_bucket() {
    let mut log = Log::new();
    for i in 0..400i64 {
        let slow = i == 300 || i == 301;
        let ms = if slow { 9000 } else { 100 + i % 7 };
        let key = if i == 301 { "B" } else { "A" };
        log.add(
            key,
            with(rec(0, Some(T0 + i * 10 * SEC), Level::Information), "ElapsedMs", json!(ms)),
        );
    }
    let found = log.detect(&AnomalyConfig::default());
    let slow = of_kind(&found, AnomalyKind::SlowOutlier);
    assert_eq!(slow.len(), 1, "{slow:#?}");
    let s = slow[0];
    assert_eq!(s.instance, None);
    assert!(
        s.message.starts_with("Slow ElapsedMs on several instances: 2 values up to 9000"),
        "{}",
        s.message
    );
    assert!(s.score > 50.0);

    // One instance only, and a zero median.
    let mut log = Log::new();
    for i in 0..100i64 {
        let ms = if i == 50 { 70 } else { 0 };
        log.add(
            "A",
            with(rec(0, Some(T0 + i * SEC), Level::Information), "Elapsed", json!(format!("{ms}"))),
        );
    }
    let found = log.detect(&AnomalyConfig::default());
    let slow = of_kind(&found, AnomalyKind::SlowOutlier);
    assert_eq!(slow.len(), 1);
    assert_eq!(slow[0].instance.as_deref(), Some("A"));
    assert_eq!(slow[0].score, 70.0);

    // Too few values, a constant series and unrelated field names find nothing.
    let mut log = Log::new();
    for i in 0..10 {
        log.add(
            "A",
            with(rec(0, Some(T0 + i * SEC), Level::Information), "ElapsedMs", json!(i * 1000)),
        );
    }
    for i in 0..50 {
        log.add("A", with(rec(0, Some(T0 + i * SEC), Level::Information), "Duration", json!(5)));
        log.add("A", with(rec(0, Some(T0 + i * SEC), Level::Information), "Other", json!(i * i)));
    }
    assert!(of_kind(&log.detect(&AnomalyConfig::default()), AnomalyKind::SlowOutlier).is_empty());
}

#[test]
fn rate_changes_up_and_down() {
    let mut log = Log::new();
    log.steady("A", T0, T0 + 40 * MIN, 2 * SEC);
    // Ten times the rate for two minutes, then nothing for three.
    log.steady("A", T0 + 20 * MIN, T0 + 21 * MIN, 200);
    log.steady("A", T0 + 40 * MIN, T0 + 41 * MIN, 5 * SEC);
    let mut quiet = Log::new();
    quiet.steady("A", T0, T0 + 20 * MIN, SEC);
    quiet.steady("A", T0 + 23 * MIN, T0 + 40 * MIN, SEC);
    let cfg = AnomalyConfig::default();
    let rate_up = of_kind(&log.detect(&cfg), AnomalyKind::RateChange)
        .iter()
        .map(|a| (*a).clone())
        .collect::<Vec<_>>();
    assert_eq!(rate_up.len(), 1, "{rate_up:#?}");
    assert!(
        rate_up[0].message.starts_with("Overall event rate rose to "),
        "{}",
        rate_up[0].message
    );
    assert_eq!(rate_up[0].instance, None);
    assert!(rate_up[0].score >= 5.0);
    assert_eq!(rate_up[0].sample_ids.len(), 5);

    let found = quiet.detect(&cfg);
    let down = of_kind(&found, AnomalyKind::RateChange);
    assert_eq!(down.len(), 1, "{down:#?}");
    assert!(down[0].message.starts_with("Overall event rate dropped to "), "{}", down[0].message);
    assert!(down[0].message.contains("(recent typical 60/min)"), "{}", down[0].message);
}

#[test]
fn anomalies_serialize_camel_case() {
    let found = incident().detect(&AnomalyConfig::default());
    let v = serde_json::to_value(&found[0]).unwrap();
    assert!(v.get("sampleIds").is_some() && v.get("kind").is_some());
    let kinds: Vec<_> = found.iter().map(|a| serde_json::to_value(a.kind).unwrap()).collect();
    assert!(kinds.contains(&json!("errorSpike")) && kinds.contains(&json!("crash")));
    let cfg: AnomalyConfig = serde_json::from_value(json!({"minCount": 9})).unwrap();
    assert_eq!(cfg.min_count, 9);
    assert_eq!(cfg.z, 4.0);
    assert_eq!(cfg.latency_fields.len(), 4);
}

// ---------------------------------------------------------------------------
// compare_windows

fn window(n: u32, errors: u32, status: &str, elapsed: f64, span_ms: i64) -> Vec<Record> {
    (0..n)
        .map(|i| {
            let level = if i < errors { Level::Error } else { Level::Information };
            let mut r = rec(i, Some(T0 + i as i64 * span_ms / n as i64), level);
            r.template = Some("HTTP responded".into());
            with(
                with(with(r, "StatusCode", json!(status)), "Elapsed", json!(elapsed + (i % 5) as f64)),
                "Note",
                json!(format!("n{i}")),
            )
        })
        .collect()
}

#[test]
fn compare_two_windows() {
    let a = window(60, 0, "200", 100.0, 60 * SEC);
    let mut b = window(120, 40, "500", 400.0, 60 * SEC);
    // A few 200s in B too, a value that only A has, and one that only B has.
    for r in b.iter_mut().take(30) {
        r.fields.insert("StatusCode".into(), json!("200"));
    }
    b[40].fields.insert("Region".into(), json!("eu"));
    let mut a = a;
    for r in a.iter_mut() {
        r.fields.insert("Region".into(), json!("us"));
    }
    b[41].fields.insert("Region".into(), json!("eu"));
    b[42].fields.insert("Region".into(), json!("eu"));
    b[43].fields.insert("Region".into(), json!("eu"));

    for r in a.iter_mut() {
        r.fields.insert("Size".into(), json!(10 + r.id));
    }
    for r in b.iter_mut() {
        r.fields.insert("Size".into(), json!(100 + r.id));
    }
    let cfg = CompareConfig::default();
    let c = compare_windows(a.iter(), b.iter(), &cfg);
    // Numeric text counts as a number (stores keep undeclared fields as text): the status codes too.
    assert_eq!(c.numeric_changes.len(), 3);
    assert!(c.numeric_changes.iter().any(|n| n.field == "Elapsed"));
    assert!(c.numeric_changes.iter().any(|n| n.field == "StatusCode"));
    assert_eq!(c.a.count, 60);
    assert_eq!(c.b.count, 120);
    assert_eq!(c.a.levels.len(), 6);
    assert_eq!(c.b.levels[Level::Error.index()].count, 40);
    assert!((c.b.levels[Level::Error.index()].per_minute - 40.0 * 60_000.0 / 59_500.0).abs() < 1e-9);
    assert!(c.b.per_minute > c.a.per_minute);

    let status = c
        .changed_fields
        .iter()
        .find(|f| f.field == "StatusCode")
        .expect("StatusCode shifted");
    assert!(status.distance > 0.5);
    assert!(status.top.len() <= cfg.top_values);
    assert!(status.top.iter().any(|t| t.value == "500" && t.share_a == 0.0));
    assert!(c.changed_fields[0].distance >= c.changed_fields.last().unwrap().distance);
    // High-cardinality fields (unique per event) are not compared as categories.
    assert!(c.changed_fields.iter().all(|f| f.field != "Note"));
    // The template did not change at all.
    assert!(c.changed_fields.iter().all(|f| f.field != "@template"));

    let elapsed = c.numeric_changes.iter().find(|n| n.field == "Elapsed").expect("Elapsed shifted");
    assert!(elapsed.b.median > elapsed.a.median * 3.0);
    assert!(elapsed.median_change_pct.unwrap() > 200.0);
    assert!(c.numeric_changes.iter().all(|n| n.field != "Note"));

    assert!(c.new_values.iter().any(|v| v.field == "StatusCode" && v.values[0].value == "500"));
    assert!(c.new_values.iter().any(|v| v.field == "Region" && v.values[0].count == 4));
    assert!(c.gone_values.iter().any(|v| v.field == "Region" && v.values[0].value == "us"));
    assert!(c.gone_values.iter().all(|v| v.field != "StatusCode"));

    assert!(
        c.summary[0].contains("2x the events per minute") || c.summary[0].contains("x the events"),
        "{:?}",
        c.summary
    );
    assert!(
        c.summary.iter().any(|s| s.starts_with("Errors rose from 0/min to ")),
        "{:?}",
        c.summary
    );
    assert!(c.summary.iter().any(|s| s.starts_with("StatusCode changed: ")), "{:?}", c.summary);
    assert!(c.summary.iter().any(|s| s.starts_with("Elapsed rose: median ")), "{:?}", c.summary);
    assert!(c.summary.iter().any(|s| s.starts_with("New in B: ")), "{:?}", c.summary);
    assert!(c.summary.iter().any(|s| s.starts_with("Only in A: Region")), "{:?}", c.summary);
    let v = serde_json::to_value(&c).unwrap();
    assert!(v["changedFields"].is_array() && v["a"]["perMinute"].is_number() && v["b"]["durationMs"].is_number());
}

#[test]
fn compare_edge_cases() {
    let cfg = CompareConfig::default();
    let empty: Vec<Record> = Vec::new();
    let some = window(10, 0, "200", 5.0, 10 * SEC);
    let c = compare_windows(empty.iter(), some.iter(), &cfg);
    assert_eq!(c.summary, vec!["One side is empty (A has 0 events, B has 10)".to_string()]);
    assert_eq!(c.a.duration_ms, 1000);
    assert!(c.a.levels.iter().all(|l| l.count == 0));
    let c = compare_windows(empty.iter(), empty.iter(), &cfg);
    assert_eq!(c.a.from, None);

    // Identical windows: similar rate, no errors, nothing changed.
    let c = compare_windows(some.iter(), some.iter(), &cfg);
    assert!(c.summary[0].starts_with("Event rate is similar"), "{:?}", c.summary);
    assert_eq!(c.summary.len(), 1);
    assert!(c.changed_fields.is_empty() && c.numeric_changes.is_empty() && c.new_values.is_empty());

    // Less of everything, errors falling, baseline-zero numerics, untimed events.
    let busy = window(100, 50, "500", 0.0, 10 * SEC);
    let calm = window(10, 0, "200", 9.0, 10 * SEC);
    let c = compare_windows(busy.iter(), calm.iter(), &cfg);
    assert!(c.summary[0].starts_with("B logs 0.11x the events"), "{:?}", c.summary);
    assert!(c.summary.iter().any(|s| s.starts_with("Errors fell from ")));
    let e = c.numeric_changes.iter().find(|n| n.field == "Elapsed").unwrap();
    assert_eq!(e.median_change_pct, Some(c.numeric_changes[0].median_change_pct.unwrap()));
    let zero_to = window(10, 0, "200", 0.0, 10 * SEC);
    let zero_base: Vec<Record> = zero_to.iter().cloned().map(|r| with(r, "Elapsed", json!(0))).collect();
    let c = compare_windows(zero_base.iter(), calm.iter(), &cfg);
    let e = c.numeric_changes.iter().find(|n| n.field == "Elapsed").unwrap();
    assert_eq!(e.median_change_pct, None);
    assert!(c.summary.iter().any(|s| s.starts_with("Elapsed rose")));
    let c = compare_windows(calm.iter(), zero_base.iter(), &cfg);
    assert!(c.summary.iter().any(|s| s.starts_with("Elapsed fell")), "{:?}", c.summary);
    let mut untimed = rec(1, None, Level::Warning);
    untimed.fields.insert("x".into(), json!(1));
    let c = compare_windows([&untimed], [&untimed], &cfg);
    assert_eq!(c.a.from, None);
    assert_eq!(c.a.levels[Level::Warning.index()].count, 1);
    // Stable numeric values on both sides (zero change) and a field missing from B.
    let flat: Vec<Record> = (0..10).map(|i| with(rec(i, Some(T0), Level::Information), "k", json!(5))).collect();
    let other: Vec<Record> = (0..10).map(|i| with(rec(i, Some(T0), Level::Information), "j", json!(5))).collect();
    let c = compare_windows(flat.iter(), flat.iter(), &cfg);
    assert!(c.numeric_changes.is_empty());
    let c = compare_windows(flat.iter(), other.iter(), &cfg);
    assert_eq!(c.changed_fields.len(), 2);
    assert!(c.new_values.is_empty() && c.gone_values.is_empty());
    // Fewer than five numeric values per side are not compared.
    let few: Vec<Record> = (0..3)
        .map(|i| with(rec(i, Some(T0), Level::Information), "k", json!(500)))
        .collect();
    assert!(compare_windows(flat.iter(), few.iter(), &cfg).numeric_changes.is_empty());
    // The section limits apply.
    let tight = CompareConfig {
        max_fields: 1,
        ..CompareConfig::default()
    };
    let c = compare_windows(busy.iter(), calm.iter(), &tight);
    assert!(c.changed_fields.len() <= 1 && c.numeric_changes.len() <= 1);
}

// ---------------------------------------------------------------------------
// field_stats

#[test]
fn numeric_field_stats() {
    let recs: Vec<Record> = (1..=100)
        .map(|i| with(rec(i, Some(T0), Level::Information), "ms", json!(i)))
        .collect();
    let mut all = recs.clone();
    all.push(rec(500, Some(T0), Level::Information));
    let s = field_stats(all.iter(), "ms", 10);
    assert_eq!((s.count, s.missing, s.distinct), (100, 1, 100));
    let n = s.numeric.expect("numeric");
    assert_eq!((n.min, n.max, n.mean), (1.0, 100.0, 50.5));
    assert!((n.stddev - 28.866).abs() < 0.01);
    assert_eq!(n.p50, 50.5);
    assert!((n.p90 - 90.1).abs() < 1e-9 && (n.p95 - 95.05).abs() < 1e-9 && (n.p99 - 99.01).abs() < 1e-9);
    assert!(n.histogram.len() <= 10 && n.histogram.iter().map(|b| b.count).sum::<u64>() == 100);
    assert_eq!(n.histogram[0].from, 0.0);
    assert!(n.histogram.windows(2).all(|w| w[0].to == w[1].from));
    assert_eq!(s.top.len(), 20);
    assert_eq!(s.top[0].count, 1);
    assert_eq!(s.top[0].value, "1");

    // Value on an exact bin edge lands in the last bin; one bucket forces a coarser step.
    let two: Vec<Record> = [0.5, 1.5].iter().map(|v| with(rec(1, None, Level::Debug), "v", json!(v))).collect();
    let n = field_stats(two.iter(), "v", 1).numeric.unwrap();
    assert_eq!(n.histogram.len(), 1);
    assert_eq!(n.histogram[0].count, 2);
    let n = field_stats(two.iter(), "v", 0).numeric.unwrap();
    assert_eq!(n.histogram.len(), 1);
    let n = field_stats(two.iter(), "v", 100).numeric.unwrap();
    assert_eq!(n.histogram[0].from, 0.5);
    assert!((1.5..1.6).contains(&n.histogram.last().unwrap().to));
    // Small fractional ranges and string numbers.
    let tiny: Vec<Record> = ["0.0012", "0.0034", "0.0099"]
        .iter()
        .map(|v| with(rec(1, None, Level::Debug), "v", json!(v)))
        .collect();
    let n = field_stats(tiny.iter(), "v", 4).numeric.unwrap();
    assert!(n.histogram.len() <= 4 && n.histogram.iter().map(|b| b.count).sum::<u64>() == 3);
}

#[test]
fn field_stats_edge_cases() {
    let s = field_stats(std::iter::empty::<&Record>(), "x", 10);
    assert_eq!((s.count, s.missing, s.distinct), (0, 0, 0));
    assert!(s.numeric.is_none() && s.top.is_empty());

    // All-equal numeric field: one degenerate bin, zero deviation.
    let same: Vec<Record> = (0..5).map(|i| with(rec(i, None, Level::Information), "v", json!(7))).collect();
    let n = field_stats(same.iter(), "v", 10).numeric.unwrap();
    assert_eq!((n.min, n.max, n.stddev, n.p99), (7.0, 7.0, 0.0, 7.0));
    assert_eq!(n.histogram.len(), 1);
    assert_eq!((n.histogram[0].from, n.histogram[0].to, n.histogram[0].count), (7.0, 7.0, 5));

    // Mixed text and numbers is not numeric; ties in `top` sort by value.
    let mixed = [
        with(rec(1, None, Level::Debug), "v", json!(1)),
        with(rec(2, None, Level::Debug), "v", json!("b")),
        with(rec(3, None, Level::Debug), "v", json!("a")),
    ];
    let s = field_stats(mixed.iter(), "v", 5);
    assert!(s.numeric.is_none());
    assert_eq!(s.top.iter().map(|t| t.value.as_str()).collect::<Vec<_>>(), ["1", "a", "b"]);

    // Pseudo fields work; distinct values are capped.
    let s = field_stats(mixed.iter(), "@level", 5);
    assert_eq!(s.top[0].value, "Debug");
    assert_eq!(s.top[0].count, 3);
    let many: Vec<Record> = (0..10_100)
        .map(|i| with(rec(i, None, Level::Debug), "id", json!(format!("v{i}"))))
        .collect();
    let s = field_stats(many.iter(), "id", 5);
    assert_eq!((s.count, s.distinct), (10_100, 10_000));
    assert!(s.numeric.is_none());
    let s = field_stats(many.iter(), "@line", 5);
    assert!(s.numeric.is_some());
    let v = serde_json::to_value(&s).unwrap();
    assert!(v["top"][0]["value"].is_string() && v["numeric"]["histogram"][0]["from"].is_number());
}

// ---------------------------------------------------------------------------
// trace

fn span_rec(id: u32, ts: i64, trace: &str, span: &str, parent: Option<&str>, level: Level) -> Record {
    let mut r = with(with(rec(id, Some(ts), level), "TraceId", json!(trace)), "SpanId", json!(span));
    if let Some(p) = parent {
        r.fields.insert("ParentSpanId".into(), json!(p));
    }
    r
}

#[test]
fn correlate_orders_by_time() {
    let recs = [
        with(rec(3, Some(T0 + 30), Level::Information), "CorrelationId", json!("abc")),
        with(rec(1, Some(T0 + 10), Level::Information), "CorrelationId", json!("abc")),
        with(rec(2, Some(T0 + 20), Level::Information), "CorrelationId", json!("zzz")),
        with(rec(4, None, Level::Information), "CorrelationId", json!("abc")),
        rec(5, Some(T0), Level::Information),
    ];
    assert_eq!(correlate(recs.iter(), "CorrelationId", "abc"), vec![1, 3, 4]);
    assert!(correlate(recs.iter(), "CorrelationId", "nope").is_empty());
    assert!(correlate(recs.iter(), "Missing", "abc").is_empty());
}

#[test]
fn correlate_orders_events_at_the_same_time_by_source_and_line_not_by_id() {
    // Ids follow the import order, which parallel imports and other stores do not share: ties go by source and line.
    let at = |id: u32, source: u32, line: u32| {
        let mut r = with(rec(id, Some(T0), Level::Information), "MachineName", json!("WEB02"));
        r.source = source;
        r.line = line;
        r
    };
    let recs = [at(1, 1, 5), at(2, 0, 9), at(3, 1, 2), at(4, 0, 3)];
    assert_eq!(correlate(recs.iter(), "MachineName", "WEB02"), vec![4, 2, 3, 1]);
}

#[test]
fn spans_and_waterfall() {
    let mut recs = vec![
        span_rec(1, T0, "t1", "root", None, Level::Information),
        span_rec(2, T0 + 10, "t1", "db", Some("root"), Level::Information),
        span_rec(3, T0 + 12, "t1", "db", None, Level::Error),
        span_rec(4, T0 + 30, "t1", "http", Some("root"), Level::Information),
        span_rec(5, T0 + 31, "t1", "child", Some("http"), Level::Warning),
        span_rec(6, T0 + 100, "t1", "root", None, Level::Information),
        span_rec(7, T0 + 5, "t1", "orphan", Some("gone"), Level::Information),
        span_rec(8, T0, "t2", "other", None, Level::Information),
        rec(9, Some(T0), Level::Information),
        span_rec(10, T0, "t1", "undated", None, Level::Information),
        span_rec(11, T0 + 5, "t1", "orphan", Some("gone"), Level::Information),
    ];
    recs[9].ts = None;
    recs[0].fields.insert("RequestPath".into(), json!("/api/orders"));
    recs[0].fields.insert("ElapsedMs".into(), json!(95));
    recs[5].fields.insert("Elapsed".into(), json!(98.5));
    recs[1].fields.insert("SourceContext".into(), json!("Db.Client"));
    recs[3].template = Some("Calling {Service}".into());
    recs[4].message = "m".repeat(200);
    recs[6].fields.insert("@tr".into(), json!("t3"));
    recs[6].fields.remove("TraceId");
    recs[6].fields.insert("@sp".into(), json!("s"));
    recs[6].fields.remove("SpanId");
    recs[7].fields.insert("TraceId".into(), json!(""));
    recs[7].fields.insert("trace_id".into(), json!("t2"));

    let all = spans(recs.iter());
    assert_eq!(all.iter().filter(|s| s.trace_id == "t1").count(), 5);
    let root = all.iter().find(|s| s.span_id == "root").unwrap();
    assert_eq!(
        (root.name.as_str(), root.start, root.duration_ms, root.events),
        ("/api/orders", T0, 98.5, 2)
    );
    let db = all.iter().find(|s| s.span_id == "db").unwrap();
    assert_eq!(
        (db.name.as_str(), db.duration_ms, db.errors, db.parent.as_deref()),
        ("Db.Client", 2.0, 1, Some("root"))
    );
    assert_eq!(all.iter().find(|s| s.span_id == "http").unwrap().name, "Calling {Service}");
    assert_eq!(all.iter().find(|s| s.span_id == "child").unwrap().name.chars().count(), 81);
    assert_eq!(all.iter().find(|s| s.trace_id == "t3").unwrap().span_id, "s");
    assert!(all.iter().any(|s| s.trace_id == "t2" && s.span_id == "other"));
    assert!(all.windows(2).all(|w| w[0].start <= w[1].start));

    let rows = waterfall(&all, "t1");
    let shape: Vec<(&str, u32)> = rows.iter().map(|r| (r.span_id.as_str(), r.depth)).collect();
    assert_eq!(shape, vec![("root", 0), ("db", 1), ("http", 1), ("child", 2), ("orphan", 0)]);
    assert_eq!(rows[1].start_offset_ms, 10);
    assert_eq!(rows[1].name, "Db.Client");
    assert_eq!(rows[1].errors, 1);
    assert_eq!(rows[3].parent.as_deref(), Some("http"));
    assert!(waterfall(&all, "missing").is_empty());
    let v = serde_json::to_value(&rows[0]).unwrap();
    assert!(v["startOffsetMs"].is_number() && v["spanId"].is_string());
}

fn plain_span(id: &str, parent: Option<&str>, start: i64) -> imt_core::analysis::Span {
    imt_core::analysis::Span {
        trace_id: "t".into(),
        span_id: id.into(),
        parent: parent.map(String::from),
        name: id.into(),
        start,
        duration_ms: 1.0,
        events: 1,
        errors: 0,
        first_id: 0,
    }
}

#[test]
fn waterfall_survives_cycles_and_self_parents() {
    let spans = vec![
        plain_span("r", None, 0),
        plain_span("a", Some("b"), 1),
        plain_span("b", Some("a"), 2),
        plain_span("c", Some("c"), 3),
        plain_span("d", Some("a"), 4),
    ];
    let mut spans = spans;
    spans.push(plain_span("tie", None, 0));
    let rows = waterfall(&spans, "t");
    let mut ids: Vec<&str> = rows.iter().map(|r| r.span_id.as_str()).collect();
    assert_eq!(ids[..2], ["r", "tie"]);
    ids.sort();
    assert_eq!(ids, ["a", "b", "c", "d", "r", "tie"]);
    // Each span appears once with a bounded depth.
    assert!(rows.iter().all(|r| r.depth < 5));
}

#[test]
fn request_durations_group_by_id() {
    let mut recs = [
        with(rec(1, Some(T0 + 100), Level::Information), "RequestId", json!("r1")),
        with(rec(2, Some(T0 + 400), Level::Error), "RequestId", json!("r1")),
        with(rec(3, Some(T0 + 50), Level::Information), "CorrelationId", json!("r2")),
        with(rec(4, Some(T0 + 10), Level::Fatal), "CorrelationId", json!("r1")),
        with(rec(5, None, Level::Information), "RequestId", json!("r9")),
        rec(6, Some(T0), Level::Information),
    ];
    recs[2].fields.insert("RequestId".into(), json!(""));
    let keyed: Vec<(&Record, &str)> = recs.iter().map(|r| (r, if r.id == 3 { "B" } else { "A" })).collect();
    let out = request_durations(keyed.iter().copied(), "RequestId | CorrelationId");
    assert_eq!(out.len(), 2);
    assert_eq!(
        (out[0].id.as_str(), out[0].first, out[0].last, out[0].events, out[0].errors),
        ("r1", T0 + 10, T0 + 400, 3, 2)
    );
    assert_eq!((out[1].id.as_str(), out[1].instance.as_str()), ("r2", "B"));
    assert!(request_durations(keyed.iter().copied(), "|").is_empty());
    let v = serde_json::to_value(&out[0]).unwrap();
    assert!(v["first"].is_number() && v["instance"] == "A");
}

// ---------------------------------------------------------------------------
// The real samples

#[test]
fn detect_on_the_generated_samples() {
    // The real .NET library output lies outside the scenario (its own times and zones).
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| !p.ends_with("real-libraries"))
        .collect();
    paths.sort();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&paths).unwrap();
    let summary = ws.summary();
    let names: std::collections::HashMap<u32, &str> = summary.sources.iter().map(|s| (s.id, s.name.as_str())).collect();
    let group_by = summary.default_group_by.clone();
    let keys: Vec<String> = ws
        .records()
        .iter()
        .map(|r| group_key(&group_by, r, names.get(&r.source).copied().unwrap_or("")))
        .collect();
    let found = detect(ws.records().iter().zip(keys.iter().map(String::as_str)), &AnomalyConfig::default());

    let crash = found
        .iter()
        .find(|a| a.kind == AnomalyKind::Crash && a.instance.as_deref().is_some_and(|i| i.contains("WEB02") && i.contains("4120")))
        .unwrap_or_else(|| panic!("no WEB02 crash in {found:#?}"));
    assert!(crash.message.contains("crashed at 10:41:24"), "{}", crash.message);
    assert!(
        crash.message.contains("restarted as OrderService · WEB02 · 5532 46 s later"),
        "{}",
        crash.message
    );
    let spike = found
        .iter()
        .find(|a| a.kind == AnomalyKind::ErrorSpike && a.instance.as_deref() == crash.instance.as_deref())
        .unwrap_or_else(|| panic!("no error spike in {found:#?}"));
    assert!(spike.from < crash.from, "{} !< {}", spike.from, crash.from);
    assert!(spike.message.starts_with("Error rate on OrderService · WEB02 · 4120 rose to "));
    // Besides WEB02, the billing log stopping after an error and the shop app crashing on the QA phone (logcat,
    // pid 4211), only the diagnostics sample's child processes crash (a plugin crashing repeatedly, the AI runtime;
    // one lane per process id, ADR 0020).
    let crashes: Vec<&str> = found
        .iter()
        .filter(|a| a.kind == AnomalyKind::Crash)
        .filter_map(|a| a.instance.as_deref())
        .collect();
    assert!(crashes.iter().any(|i| i.ends_with("logcat-20261014.txt · 4211")), "{crashes:#?}");
    assert!(
        crashes.iter().filter(|i| !i.starts_with("imt-diagnostics-sample.zip/")).count() <= 3,
        "{crashes:#?}"
    );
}

#[test]
fn ties_sort_by_name() {
    let recs: Vec<Record> = ["b", "a"]
        .iter()
        .map(|id| with(rec(1, Some(T0), Level::Information), "RequestId", json!(id)))
        .collect();
    let out = request_durations(recs.iter().map(|r| (r, "I")), "RequestId");
    assert_eq!(out.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);

    // Two numeric fields that moved by the same relative amount.
    let a: Vec<Record> = (0..6)
        .map(|i| with(with(rec(i, Some(T0), Level::Debug), "x", json!(10)), "y", json!(10)))
        .collect();
    let b: Vec<Record> = (0..6)
        .map(|i| with(with(rec(i, Some(T0), Level::Debug), "x", json!(20)), "y", json!(20)))
        .collect();
    let c = compare_windows(a.iter(), b.iter(), &CompareConfig::default());
    assert_eq!(c.numeric_changes.iter().map(|n| n.field.as_str()).collect::<Vec<_>>(), ["x", "y"]);
}

#[test]
fn restart_must_share_the_machine_not_just_the_application() {
    let mut log = Log::new();
    let mut fatal = rec(0, Some(T0 + 10 * MIN), Level::Fatal);
    fatal.message = "dead".into();
    log.steady("App · WEB02 · 1", T0, T0 + 10 * MIN, SEC);
    log.add("App · WEB02 · 1", fatal);
    log.steady("App · WEB01 · 7", T0, T0 + 20 * MIN, SEC);
    log.steady("App · WEB01 · 8", T0 + 10 * MIN + 2 * SEC, T0 + 20 * MIN, SEC);
    log.steady("App · WEB02 · 2", T0 + 10 * MIN + 40 * SEC, T0 + 20 * MIN, SEC);
    let found = log.detect(&AnomalyConfig::default());
    let c = of_kind(&found, AnomalyKind::Crash);
    assert!(c[0].message.contains("restarted as App · WEB02 · 2 40 s later"), "{}", c[0].message);
}

#[test]
fn compare_bounds_the_values_it_tracks() {
    // Regression (engine review finding 13): compare counted every distinct value of every field.
    let cfg = CompareConfig::default();
    let side = |offset: u32| -> Vec<Record> {
        (0..200)
            .map(|i| {
                let r = rec(i + offset, Some(T0 + i as i64 * SEC), Level::Information);
                with(
                    with(r, "Unique", json!(format!("u{}", i + offset))),
                    "Mode",
                    json!(if offset == 0 { "a" } else { "b" }),
                )
            })
            .collect()
    };
    let (a, b) = (side(0), side(1000));
    let c = compare_windows(a.iter(), b.iter(), &cfg);
    // High-cardinality fields are not compared; a low-cardinality shift is.
    assert!(c.changed_fields.iter().all(|f| f.field != "Unique"));
    assert!(c.new_values.iter().all(|f| f.field != "Unique"));
    assert!(c.changed_fields.iter().any(|f| f.field == "Mode"));
}

#[test]
fn waterfall_of_a_long_chain_is_linear() {
    // Regression (engine review finding 19): roots.contains and the root walk were quadratic.
    let n = 20_000u32;
    let recs: Vec<Record> = (0..n)
        .map(|i| {
            let r = rec(i, Some(T0 + i as i64), Level::Information);
            let r = with(with(r, "TraceId", json!("t")), "SpanId", json!(format!("s{i}")));
            if i > 0 {
                with(r, "ParentSpanId", json!(format!("s{}", i - 1)))
            } else {
                r
            }
        })
        .collect();
    let spans = imt_core::analysis::spans(recs.iter());
    let started = std::time::Instant::now();
    let rows = imt_core::analysis::waterfall(&spans, "t");
    assert_eq!(rows.len(), n as usize);
    assert_eq!(rows.last().unwrap().depth, n - 1);
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
}
