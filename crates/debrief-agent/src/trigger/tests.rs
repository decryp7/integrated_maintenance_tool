use super::*;
use std::collections::VecDeque;
use std::io::Write;
use std::time::Duration;

/// Plays back scripted sample batches (the last one repeats).
struct FakeSampler(VecDeque<Vec<ProcSample>>);

impl Sampler for FakeSampler {
    fn sample(&mut self, _: &Targets) -> Vec<ProcSample> {
        if self.0.len() > 1 {
            self.0.pop_front().unwrap_or_default()
        } else {
            self.0.front().cloned().unwrap_or_default()
        }
    }
    fn cpu_count(&self) -> usize {
        4
    }
}

fn proc(pid: u32, private_mb: u64) -> ProcSample {
    ProcSample {
        pid,
        name: "OrderService".into(),
        private_bytes: Some(private_mb << 20),
        working_set: 10 << 20,
        virtual_bytes: 20 << 20,
        cpu_core_percent: Some(200.0),
        threads: Some(12),
        handles: Some(300),
        ..ProcSample::default()
    }
}

fn spec(sustain: u64, cooldown: u64) -> ProcessTrigger {
    ProcessTrigger {
        process: "OrderService*".into(),
        metric: Metric::PrivateBytes,
        above: 1000.0,
        sustain: Duration::from_secs(sustain),
        sample: Duration::from_secs(5),
        cooldown: Duration::from_secs(cooldown),
    }
}

#[test]
fn metric_values_in_config_units() {
    let s = proc(1, 1536);
    assert_eq!(metric_value(&s, Metric::PrivateBytes, 4), Some(1536.0));
    assert_eq!(metric_value(&s, Metric::WorkingSet, 4), Some(10.0));
    assert_eq!(metric_value(&s, Metric::VirtualBytes, 4), Some(20.0));
    assert_eq!(metric_value(&s, Metric::Cpu, 4), Some(50.0));
    assert_eq!(metric_value(&s, Metric::Cpu, 0), Some(200.0));
    assert_eq!(metric_value(&s, Metric::Threads, 4), Some(12.0));
    assert_eq!(metric_value(&s, Metric::Handles, 4), Some(300.0));
    assert_eq!(metric_value(&ProcSample::default(), Metric::PrivateBytes, 4), None);
}

#[test]
fn threshold_needs_sustain_rearm_and_cooldown() {
    let mut t = Threshold::new(10.0, 30_000, 60_000);
    assert!(!t.observe(0, Some(11.0)));
    assert!(!t.observe(20_000, Some(12.0)));
    assert!(t.observe(30_000, Some(12.0)), "fires after 30 s above");
    assert!(!t.observe(40_000, Some(50.0)), "still high: not again");
    assert!(!t.observe(45_000, Some(5.0)), "drop re-arms");
    assert!(!t.observe(50_000, Some(20.0)));
    assert!(!t.observe(85_000, Some(20.0)), "sustained but within the cooldown");
    assert!(t.observe(90_000, Some(20.0)), "cooldown over");
    // A gap in measurements resets the sustain timer.
    let mut t = Threshold::new(10.0, 10_000, 0);
    assert!(!t.observe(0, Some(11.0)));
    assert!(!t.observe(5_000, None));
    assert!(!t.observe(6_000, Some(11.0)));
    assert!(t.observe(16_000, Some(11.0)));
    // Zero sustain fires at once; equal to the threshold is not above it.
    let mut t = Threshold::new(10.0, 0, 0);
    assert!(!t.observe(0, Some(10.0)));
    assert!(t.observe(1, Some(10.5)));
}

#[test]
fn process_watch_with_a_fake_sampler() {
    let batches = VecDeque::from(vec![
        vec![proc(1, 500)],
        vec![proc(1, 1200), proc(2, 1500)],
        vec![proc(1, 1200), proc(2, 1600)],
        vec![proc(2, 1700)],
    ]);
    let mut w = ProcessWatch::new(spec(5, 0), Box::new(FakeSampler(batches))).unwrap();
    assert_eq!(w.poll(0), None);
    assert_eq!(w.poll(1_000), None, "not yet time to sample");
    assert_eq!(w.poll(5_000), None, "above, sustain starts");
    let fired = w.poll(10_000).expect("fires after 5 s above");
    assert!(fired.contains("OrderService (2)") && fired.contains("1600.0MB > 1000MB"), "{fired}");
    assert_eq!(w.poll(15_000), None, "stays high: no repeat");
    // No processes at all: nothing measured.
    let mut w = ProcessWatch::new(spec(0, 0), Box::new(FakeSampler(VecDeque::new()))).unwrap();
    assert_eq!(w.poll(0), None);
    assert!(ProcessWatch::new(
        ProcessTrigger {
            process: "a[".into(),
            ..spec(0, 0)
        },
        Box::new(FakeSampler(VecDeque::new()))
    )
    .is_err());
}

#[test]
fn system_sampler_sees_this_process() {
    let mut s = SystemSampler::default();
    let cfg = debrief_monitor::config::Config::from_json(&serde_json::json!({ "targets": [{ "pid": std::process::id() }] })).unwrap();
    let samples = s.sample(&cfg.targets);
    assert_eq!(samples.len(), 1);
    assert!(s.cpu_count() >= 1);
}

fn log_spec(path: PathBuf, cooldown: u64) -> LogTrigger {
    LogTrigger {
        path,
        ql: "level >= Error".into(),
        poll: Duration::from_secs(1),
        cooldown: Duration::from_secs(cooldown),
    }
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

const LINE: &str = "2026-10-17 10:00:00.000 [INF] started\n";

#[test]
fn log_watch_fires_on_new_matching_lines_only() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("app.log");
    let mut w = LogWatch::new(log_spec(log.clone(), 0));
    assert_eq!(w.poll(0).unwrap(), None, "missing file: waits");
    // History (an old error) is skipped: the file is tailed from its end.
    append(&log, LINE);
    append(&log, "2026-10-17 10:00:01.000 [ERR] old failure\n");
    assert_eq!(w.poll(1_000).unwrap(), None);
    assert_eq!(w.poll(1_500).unwrap(), None, "within the poll interval");
    append(&log, "2026-10-17 10:00:02.000 [INF] fine\n");
    assert_eq!(w.poll(2_000).unwrap(), None, "no match");
    append(&log, "2026-10-17 10:00:03.000 [ERR] OutOfMemoryException\n");
    let fired = w.poll(3_000).unwrap().expect("fires");
    assert!(fired.contains("OutOfMemoryException"), "{fired}");
    assert_eq!(w.poll(4_000).unwrap(), None, "nothing new");
}

#[test]
fn log_watch_cooldown_and_folders() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("svc.log");
    append(&log, LINE);
    let mut w = LogWatch::new(log_spec(dir.path().to_path_buf(), 60));
    assert_eq!(w.poll(0).unwrap(), None);
    append(&log, "2026-10-17 10:00:03.000 [ERR] first\n");
    assert!(w.poll(1_000).unwrap().is_some());
    append(&log, "2026-10-17 10:00:04.000 [ERR] second\n");
    assert_eq!(w.poll(2_000).unwrap(), None, "cooling down");
    append(&log, "2026-10-17 10:00:05.000 [ERR] third\n");
    assert!(w.poll(70_000).unwrap().is_some());
    assert!(LogWatch::files(&dir.path().join("none")).is_empty());
}
