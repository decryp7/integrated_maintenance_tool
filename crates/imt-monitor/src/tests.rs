use super::*;
use crate::config::{Config, CpuMode, Targets};
use crate::counters::{CounterPath, CounterValue};
use crate::source::{ProcSample, ProcessSource, SysinfoSource};
use imt_core::export::ClefFileSink;
use imt_core::model::Level;
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

const T0: i64 = 1_760_000_000_000;

fn sample(pid: u32, name: &str, uptime: u64) -> ProcSample {
    ProcSample {
        pid,
        name: name.into(),
        exe: Some(format!("C:/apps/{name}")),
        cpu_core_percent: Some(100.0),
        working_set: 100 * 1024 * 1024,
        virtual_bytes: 200 * 1024 * 1024,
        uptime_sec: uptime,
        ..Default::default()
    }
}

fn tracker() -> Tracker {
    Tracker::new(CpuMode::Machine, 4, "HOST")
}

fn msgs(r: &[Record]) -> Vec<&str> {
    r.iter().map(|r| r.message.as_str()).collect()
}

#[test]
fn first_sight_is_not_a_start_then_steady_sampling() {
    let mut t = tracker();
    let r = t.tick(T0, &[sample(1, "svc.exe", 500)]);
    assert_eq!(msgs(&r)[0], "Now monitoring svc.exe (1), running for 500s");
    assert_eq!(r[0].fields["AlreadyRunning"], true);
    assert_eq!(r[1].message, "Process stats svc.exe CPU 25.0% · WorkingSet 100.0 MB");
    let f = &r[1].fields;
    assert_eq!(f["ProcessName"], "svc.exe");
    assert_eq!(f["ProcessId"], 1);
    assert_eq!(f["MachineName"], "HOST");
    assert_eq!(f["SourceContext"], "IMT.Monitor");
    assert_eq!(f["Exe"], "C:/apps/svc.exe");
    assert_eq!(
        (f["CpuPercent"].as_f64(), f["WorkingSetMb"].as_f64(), f["VirtualMb"].as_f64()),
        (Some(25.0), Some(100.0), Some(200.0))
    );
    assert!(f.get("PrivateMb").is_none() && f.get("Threads").is_none() && f.get("Handles").is_none());
    assert!(f.get("ReadBytesPerSec").is_none(), "no rate on the first tick");
    assert_eq!(r[1].level, Level::Information);
    assert!(t.is_tracked(1) && !t.is_tracked(2));
    let r = t.tick(T0 + 2000, &[sample(1, "svc.exe", 502)]);
    assert_eq!(r.len(), 1, "steady state: stats only");
    assert_eq!(r[0].fields["UptimeSec"], 502);
}

#[test]
fn optional_fields_and_io_rates() {
    let mut t = tracker();
    t.tick(T0, &[]);
    let mut s = sample(1, "a", 5);
    s.private_bytes = Some(50 * 1024 * 1024);
    s.threads = Some(12);
    s.handles = Some(340);
    s.read_bytes = Some(4000);
    s.write_bytes = Some(1000);
    let r = t.tick(T0 + 2000, &[s]);
    let f = &r[1].fields;
    assert_eq!(
        (f["PrivateMb"].as_f64(), f["Threads"].as_u64(), f["Handles"].as_u64()),
        (Some(50.0), Some(12), Some(340))
    );
    assert_eq!(
        (f["ReadBytesPerSec"].as_u64(), f["WriteBytesPerSec"].as_u64()),
        (Some(2000), Some(500))
    );
}

#[test]
fn no_stats_without_cpu_baseline() {
    let mut t = tracker();
    let mut s = sample(1, "a", 5);
    s.cpu_core_percent = None;
    let r = t.tick(T0, &[s]);
    assert_eq!(r.len(), 1);
}

#[test]
fn cpu_machine_vs_core() {
    let mut s = sample(1, "a", 5);
    s.cpu_core_percent = Some(200.0);
    let machine = Tracker::new(CpuMode::Machine, 4, "H").tick(T0, std::slice::from_ref(&s));
    assert_eq!(machine[1].fields["CpuPercent"].as_f64(), Some(50.0));
    let core = Tracker::new(CpuMode::Core, 4, "H").tick(T0, std::slice::from_ref(&s));
    assert_eq!(core[1].fields["CpuPercent"].as_f64(), Some(200.0));
    s.cpu_core_percent = Some(800.0);
    let capped = Tracker::new(CpuMode::Machine, 4, "H").tick(T0, std::slice::from_ref(&s));
    assert_eq!(capped[1].fields["CpuPercent"].as_f64(), Some(100.0));
    s.cpu_core_percent = Some(33.33);
    let r = Tracker::new(CpuMode::Core, 0, "H").tick(T0, &[s]);
    assert_eq!(r[1].fields["CpuPercent"].as_f64(), Some(33.3));
}

#[test]
fn start_and_exit_after_long_life_are_informational() {
    let mut t = tracker();
    t.tick(T0, &[]);
    let r = t.tick(T0 + 2000, &[sample(7, "svc.exe", 0)]);
    assert_eq!(r[0].message, "Application started: svc.exe (7)");
    assert!(r[0].fields.get("Restarted").is_none());
    t.tick(T0 + 4000, &[sample(7, "svc.exe", 200)]);
    assert!(t.tick(T0 + 6000, &[]).is_empty(), "exit is held back for the restart window");
    assert!(!t.is_tracked(7));
    let r = t.tick(T0 + 16_000, &[]);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].message, "Application exited: svc.exe (7) after 200s");
    assert_eq!(r[0].level, Level::Information);
    assert_eq!(r[0].ts, Some(T0 + 6000), "stamped when the exit was noticed");
    assert!(r[0].fields.get("RestartedAs").is_none());
}

#[test]
fn short_lived_process_exit_is_a_warning() {
    let mut t = tracker();
    t.tick(T0, &[sample(7, "svc.exe", 5)]);
    t.tick(T0 + 2000, &[]);
    let r = t.finish(T0 + 3000);
    assert_eq!(r[0].level, Level::Warning);
    assert!(t.finish(T0 + 4000).is_empty());
}

#[test]
fn quick_restart_links_exit_and_start() {
    let mut t = tracker();
    t.tick(T0, &[sample(7, "Svc.exe", 900)]);
    // Same sample: old pid gone, new pid with the same name (different case) appears.
    let r = t.tick(T0 + 2000, &[sample(8, "svc.EXE", 0)]);
    let start = r.iter().find(|r| r.message.starts_with("Application started")).unwrap();
    assert_eq!(start.fields["Restarted"], true);
    let exit = r.iter().find(|r| r.message.starts_with("Application exited")).unwrap();
    assert_eq!(exit.level, Level::Warning, "restart makes a long-lived exit a warning");
    assert_eq!(exit.fields["RestartedAs"], 8);
    assert_eq!(exit.message, "Application exited: Svc.exe (7) after 900s");
}

#[test]
fn restart_in_a_later_tick_within_window_and_outside_it() {
    let mut t = tracker();
    t.tick(T0, &[sample(7, "svc", 900)]);
    t.tick(T0 + 2000, &[]);
    let r = t.tick(T0 + 8000, &[sample(9, "svc", 0)]);
    assert_eq!(r.iter().filter(|r| r.fields.get("RestartedAs").is_some()).count(), 1);
    assert!(r.iter().any(|r| r.fields.get("Restarted").is_some()));
    // After the window the new process is not a restart and the exit was already flushed.
    let mut t = tracker();
    t.tick(T0, &[sample(7, "svc", 900)]);
    t.tick(T0 + 2000, &[]);
    let out = t.tick(T0 + 13_000, &[sample(9, "svc", 0)]);
    assert!(out.iter().all(|r| r.fields.get("Restarted").is_none()));
    assert!(out
        .iter()
        .any(|r| r.message.starts_with("Application exited") && r.fields.get("RestartedAs").is_none()));
    // A second new process of the same name does not reuse an already matched exit.
    let mut t = tracker();
    t.tick(T0, &[sample(7, "svc", 900)]);
    let r = t.tick(T0 + 2000, &[sample(8, "svc", 0), sample(9, "svc", 0)]);
    assert_eq!(r.iter().filter(|r| r.fields.get("Restarted").is_some()).count(), 1);
}

#[test]
fn access_denied_warns_once_per_process() {
    let mut t = tracker();
    let denied = ProcSample {
        pid: 4,
        name: "lsass.exe".into(),
        access_denied: true,
        ..Default::default()
    };
    let r = t.tick(T0, std::slice::from_ref(&denied));
    assert_eq!(msgs(&r), ["Access denied reading lsass.exe (4); run the monitor elevated"]);
    assert_eq!(r[0].level, Level::Warning);
    assert_eq!(r[0].fields["AccessDenied"], true);
    assert!(t.tick(T0 + 2000, std::slice::from_ref(&denied)).is_empty());
    assert!(!t.is_tracked(4));
    // After the process disappears and returns, it is reported again.
    t.tick(T0 + 4000, &[]);
    assert_eq!(t.tick(T0 + 6000, &[denied]).len(), 1);
}

// ---------------------------------------------------------------- config

fn cfg(v: serde_json::Value) -> Result<Config> {
    Config::from_json(&v)
}

fn err(v: serde_json::Value) -> String {
    format!("{:#}", cfg(v).unwrap_err())
}

#[test]
fn config_defaults_and_values() {
    let c = cfg(json!({"targets": [{"pid": 5}]})).unwrap();
    assert_eq!(
        (c.interval_ms, c.cpu, c.include_children, c.counters.len()),
        (2000, CpuMode::Machine, false, 0)
    );
    assert_eq!(c.interval(), Duration::from_secs(2));
    let c = cfg(
        json!({"targets": [{"name": "a"}], "intervalMs": 250, "cpu": "core", "includeChildren": true,
        "counters": ["\\Processor(_Total)\\% Processor Time"]}),
    )
    .unwrap();
    assert_eq!(
        (c.interval_ms, c.cpu, c.include_children, c.counters.len()),
        (250, CpuMode::Core, true, 1)
    );
    // Counters alone are a valid config.
    assert!(cfg(json!({"counters": ["\\Memory\\Available MBytes"]})).is_ok());
}

#[test]
fn config_validation_errors() {
    assert!(err(json!({})).contains("at least one"));
    assert!(err(json!({"targets": [{"pid": 1}], "intervalMs": 100})).contains("at least 250"));
    assert!(err(json!({"targets": [{}]})).contains("targets[0]: specify exactly one"));
    assert!(err(json!({"targets": [{"name": "a", "pid": 1}]})).contains("exactly one"));
    assert!(err(json!({"targets": [{"pid": 1}, {"name": " "}]})).contains("targets[1]: \"name\" must not be empty"));
    assert!(err(json!({"targets": [{"path": ""}]})).contains("\"path\" must not be empty"));
    assert!(err(json!({"targets": [{"pid": 0}]})).contains("greater than 0"));
    assert!(err(json!({"targets": [{"name": "[a"}]})).contains("bad name pattern"));
    assert!(err(json!({"targets": [{"path": "[a"}]})).contains("bad path pattern"));
    assert!(err(json!({"targets": [{"pid": 1}], "cpu": "half"})).contains("invalid monitor config"));
    assert!(err(json!({"targets": [{"pid": 1}], "bogus": 1})).contains("unknown field"));
    assert!(err(json!({"targets": [{"pid": 1}], "counters": ["nope"]})).contains("counters[0]"));
    assert!(err(json!("text")).contains("invalid monitor config"));
}

#[test]
fn glob_matching() {
    let t = cfg(json!({"targets": [{"name": "OrderService*"}, {"path": "C:\\Apps\\Tool\\*.exe"}, {"pid": 99}]}))
        .unwrap()
        .targets;
    assert!(t.matches(1, "OrderService.exe", None));
    assert!(t.matches(1, "orderservice-worker", None));
    assert!(!t.matches(1, "Other.exe", None));
    assert!(t.matches(1, "x", Some("c:/apps/tool/run.EXE")));
    assert!(t.matches(1, "x", Some("C:\\Apps\\Tool\\run.exe")));
    assert!(!t.matches(1, "x", Some("C:/Apps/Else/run.exe")));
    assert!(!t.matches(1, "x", None));
    assert!(t.matches(99, "x", None) && !t.matches(98, "x", None));
    // `.exe` is optional in a name pattern.
    let t = cfg(json!({"targets": [{"name": "svc"}]})).unwrap().targets;
    assert!(t.matches(1, "svc.exe", None) && t.matches(1, "SVC.EXE", None) && t.matches(1, "svc", None));
    // Linux cuts process names to 15 characters; the executable's file name still matches.
    let t = cfg(json!({"targets": [{"name": "order-service-worker"}]})).unwrap().targets;
    assert!(t.matches(1, "order-service-w", Some("/opt/shop/bin/order-service-worker")));
    assert!(t.matches(1, "order-service-w", Some(r"C:\Shop\order-service-worker.exe")));
    assert!(!t.matches(1, "order-service-w", None) && !t.matches(1, "order-service-w", Some("/opt/shop/bin/")));
    assert!(!t.matches(1, "svc2.exe", None));
    assert_eq!(t.only_pids(), None);
    assert_eq!(
        cfg(json!({"targets": [{"pid": 3}, {"pid": 4}]})).unwrap().targets.only_pids(),
        Some(vec![3, 4])
    );
    assert!(Targets::default().is_empty());
}

// ------------------------------------------------------------- run loop

struct FakeSource {
    script: Vec<Vec<ProcSample>>,
    calls: usize,
}
impl ProcessSource for FakeSource {
    fn cpu_count(&self) -> usize {
        2
    }
    fn host_name(&self) -> String {
        "HOST".into()
    }
    fn sample(&mut self, _: &Targets, _: bool) -> Vec<ProcSample> {
        let s = self.script.get(self.calls).cloned().unwrap_or_default();
        self.calls += 1;
        s
    }
}

/// Collects records and sets `stop` after `limit` writes.
struct StopSink<'a> {
    records: Mutex<Vec<Record>>,
    writes: AtomicUsize,
    limit: usize,
    stop: &'a AtomicBool,
}
impl EventSink for StopSink<'_> {
    fn write(&self, r: &[Record]) -> Result<()> {
        self.records.lock().unwrap().extend_from_slice(r);
        if self.writes.fetch_add(1, Ordering::SeqCst) + 1 >= self.limit {
            self.stop.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}

fn clock() -> impl Fn() -> i64 {
    let n = AtomicUsize::new(0);
    move || T0 + n.fetch_add(2000, Ordering::SeqCst) as i64
}

fn base_cfg() -> Config {
    cfg(json!({"targets": [{"name": "svc*"}]})).unwrap()
}

#[test]
fn run_loop_samples_until_stopped_and_flushes_exits() {
    let stop = AtomicBool::new(false);
    let sink = StopSink {
        records: Mutex::new(vec![]),
        writes: AtomicUsize::new(0),
        limit: 2,
        stop: &stop,
    };
    let mut src = FakeSource {
        script: vec![vec![sample(1, "svc", 100), sample(2, "svc2", 5)], vec![sample(2, "svc2", 7)]],
        calls: 0,
    };
    run_loop(
        &base_cfg(),
        &mut src,
        Probes::default(),
        Duration::from_millis(5),
        &sink,
        &stop,
        &clock(),
    )
    .unwrap();
    let recs = sink.records.lock().unwrap();
    let m = msgs(&recs);
    assert!(m[0].starts_with("Now monitoring") && m[1].starts_with("Process stats"));
    assert_eq!(m.iter().filter(|m| m.starts_with("Process stats")).count(), 3);
    assert_eq!(
        *m.last().unwrap(),
        "Application exited: svc (1) after 100s",
        "pending exit flushed on stop"
    );
    assert_eq!(src.calls, 2);
}

#[test]
fn run_loop_does_not_sample_when_already_stopped() {
    let stop = AtomicBool::new(true);
    let sink = StopSink {
        records: Mutex::new(vec![]),
        writes: AtomicUsize::new(0),
        limit: 1,
        stop: &stop,
    };
    let mut src = FakeSource { script: vec![], calls: 0 };
    run_loop(
        &base_cfg(),
        &mut src,
        Probes::default(),
        Duration::from_millis(5),
        &sink,
        &stop,
        &clock(),
    )
    .unwrap();
    assert_eq!(src.calls, 0);
    assert!(sink.records.lock().unwrap().is_empty());
}

#[test]
fn run_loop_stops_during_sleep() {
    let stop = AtomicBool::new(false);
    let sink = StopSink {
        records: Mutex::new(vec![]),
        writes: AtomicUsize::new(0),
        limit: 1,
        stop: &stop,
    };
    let mut src = FakeSource {
        script: vec![vec![sample(1, "svc", 1)]],
        calls: 0,
    };
    let t = Instant::now();
    run_loop(
        &base_cfg(),
        &mut src,
        Probes::default(),
        Duration::from_secs(30),
        &sink,
        &stop,
        &clock(),
    )
    .unwrap();
    assert!(t.elapsed() < Duration::from_secs(5));
}

struct FailingSink;
impl EventSink for FailingSink {
    fn write(&self, _: &[Record]) -> Result<()> {
        anyhow::bail!("disk full")
    }
}

#[test]
fn run_loop_propagates_sink_errors() {
    let stop = AtomicBool::new(false);
    let mut src = FakeSource {
        script: vec![vec![sample(1, "svc", 1)]],
        calls: 0,
    };
    let e = run_loop(
        &base_cfg(),
        &mut src,
        Probes::default(),
        Duration::from_millis(5),
        &FailingSink,
        &stop,
        &clock(),
    )
    .unwrap_err();
    assert!(e.to_string().contains("disk full"));
    // Also when the failure is the counter warning.
    let mut src = FakeSource { script: vec![], calls: 0 };
    assert!(run_loop(
        &base_cfg(),
        &mut src,
        Probes {
            counters: Some(Err(anyhow::anyhow!("x"))),
            ..Default::default()
        },
        Duration::from_millis(5),
        &FailingSink,
        &stop,
        &clock()
    )
    .is_err());
}

struct FakeCounters {
    fail_after: usize,
    calls: usize,
}
impl CounterSource for FakeCounters {
    fn collect(&mut self) -> Result<Vec<CounterValue>> {
        self.calls += 1;
        if self.calls > self.fail_after {
            anyhow::bail!("pdh broke");
        }
        let p = |s: &str| CounterPath::parse(s).unwrap();
        Ok(vec![
            CounterValue {
                path: p("\\Process(svc)\\ID Process"),
                value: 1.0,
            },
            CounterValue {
                path: p("\\Process(svc)\\Thread Count"),
                value: 9.0,
            },
            CounterValue {
                path: p("\\Process(other)\\ID Process"),
                value: 2.0,
            },
            CounterValue {
                path: p("\\Process(other)\\Thread Count"),
                value: 3.0,
            },
        ])
    }
}

#[test]
fn run_loop_emits_counters_for_tracked_processes_and_survives_failure() {
    let mut c = base_cfg();
    c.counters = vec![CounterPath::parse("\\Process(*)\\Thread Count").unwrap()];
    let stop = AtomicBool::new(false);
    let sink = StopSink {
        records: Mutex::new(vec![]),
        writes: AtomicUsize::new(0),
        limit: 2,
        stop: &stop,
    };
    let mut src = FakeSource {
        script: vec![vec![sample(1, "svc", 100)], vec![sample(1, "svc", 102)]],
        calls: 0,
    };
    let counters: Box<dyn CounterSource> = Box::new(FakeCounters { fail_after: 1, calls: 0 });
    run_loop(
        &c,
        &mut src,
        Probes {
            counters: Some(Ok(counters)),
            ..Default::default()
        },
        Duration::from_millis(5),
        &sink,
        &stop,
        &clock(),
    )
    .unwrap();
    let recs = sink.records.lock().unwrap();
    let m = msgs(&recs);
    assert_eq!(
        m.iter().filter(|m| m.starts_with("Counter ")).count(),
        1,
        "only the tracked process, ID Process helper hidden"
    );
    assert!(m.contains(&"Counter \\Process(svc)\\Thread Count = 9"));
    let w = recs
        .iter()
        .find(|r| r.message.starts_with("Performance counter collection failed"))
        .unwrap();
    assert_eq!(w.level, Level::Warning);
}

#[test]
fn run_loop_warns_once_when_counters_cannot_open() {
    let stop = AtomicBool::new(false);
    let sink = StopSink {
        records: Mutex::new(vec![]),
        writes: AtomicUsize::new(0),
        limit: 1,
        stop: &stop,
    };
    let mut src = FakeSource { script: vec![], calls: 0 };
    run_loop(
        &base_cfg(),
        &mut src,
        Probes {
            counters: Some(Err(anyhow::anyhow!("not here"))),
            ..Default::default()
        },
        Duration::from_millis(5),
        &sink,
        &stop,
        &clock(),
    )
    .unwrap();
    let recs = sink.records.lock().unwrap();
    assert_eq!(recs[0].message, "Performance counters unavailable: not here");
    assert_eq!(recs[0].level, Level::Warning);
}

// -------------------------------------------------------- collector/plugin

#[test]
fn the_collector_names_itself() {
    assert_eq!((ProcessMonitor.id(), ProcessMonitor.name()), ("process-monitor", "Process monitor"));
}

#[test]
fn collector_rejects_bad_config() {
    let stop = AtomicBool::new(true);
    let sink = StopSink {
        records: Mutex::new(vec![]),
        writes: AtomicUsize::new(0),
        limit: 1,
        stop: &stop,
    };
    let e = ProcessMonitor
        .run(&json!({"targets": [{"pid": 1}], "intervalMs": 1}), &sink, &stop)
        .unwrap_err();
    assert!(e.to_string().contains("intervalMs"));
}

#[test]
fn real_sysinfo_monitors_the_current_process() {
    let me = std::process::id();
    let stop = AtomicBool::new(false);
    let sink = StopSink {
        records: Mutex::new(vec![]),
        writes: AtomicUsize::new(0),
        limit: 2,
        stop: &stop,
    };
    ProcessMonitor
        .run(&json!({"targets": [{"pid": me}], "intervalMs": 250}), &sink, &stop)
        .unwrap();
    let recs = sink.records.lock().unwrap();
    let stats: Vec<_> = recs.iter().filter(|r| r.message.starts_with("Process stats")).collect();
    assert!(!stats.is_empty(), "{:?}", msgs(&recs));
    let f = &stats[0].fields;
    assert_eq!(f["ProcessId"], me);
    assert!(f["WorkingSetMb"].as_f64().unwrap() > 0.0);
    assert!(f.get("CpuPercent").is_some());
    assert!(recs[0].fields.get("AlreadyRunning").is_some());
}

#[test]
fn sysinfo_source_by_name_children_and_missing_pid() {
    let mut s = SysinfoSource::default();
    assert!(s.cpu_count() >= 1 && !s.host_name().is_empty());
    let me = std::process::id();
    // Name target: the test binary's own name (without extension) matches via glob.
    let exe = std::env::current_exe().unwrap();
    let name = exe.file_name().unwrap().to_string_lossy().into_owned();
    let t = cfg(json!({"targets": [{"name": name}]})).unwrap().targets;
    let first = s.sample(&t, true);
    let mine = first.iter().find(|p| p.pid == me).expect("own process found by name");
    assert!(mine.cpu_core_percent.is_none(), "no CPU baseline on first sight");
    assert!(!mine.access_denied);
    std::thread::sleep(Duration::from_millis(300));
    let second = s.sample(&t, true);
    assert!(second.iter().find(|p| p.pid == me).unwrap().cpu_core_percent.is_some());
    // Path target.
    let t = cfg(json!({"targets": [{"path": exe.to_string_lossy().replace('\\', "/")}]}))
        .unwrap()
        .targets;
    assert!(s.sample(&t, false).iter().any(|p| p.pid == me));
    // A PID that does not exist yields nothing.
    let t = cfg(json!({"targets": [{"pid": 4_000_000}]})).unwrap().targets;
    assert!(s.sample(&t, false).is_empty());
}

// ------------------------------------------------------------ end to end

#[test]
fn clef_files_open_in_imt_with_lanes_and_lifecycle_markers() {
    use imt_core::api::TimelineRequest;
    use imt_core::{Profile, Workspace};
    let dir = tempfile::tempdir().unwrap();
    let sink = ClefFileSink::new(dir.path(), "process-monitor", 10 * 1024 * 1024, 10).unwrap();
    let mut t = Tracker::new(CpuMode::Machine, 4, "BUILD01");
    let mut at = T0;
    let mut step = |t: &mut Tracker, s: &[ProcSample]| {
        at += 2000;
        sink.write(&t.tick(at, s)).unwrap();
    };
    step(&mut t, &[sample(100, "OrderService.exe", 50)]);
    step(&mut t, &[sample(100, "OrderService.exe", 52), sample(300, "Worker.exe", 0)]);
    step(&mut t, &[sample(100, "OrderService.exe", 54), sample(300, "Worker.exe", 2)]);
    // OrderService crashes after 4 s and is restarted as 101; Worker keeps running and later exits cleanly.
    step(&mut t, &[sample(101, "OrderService.exe", 0), sample(300, "Worker.exe", 4)]);
    step(&mut t, &[sample(101, "OrderService.exe", 2), sample(300, "Worker.exe", 6)]);
    for _ in 0..6 {
        step(&mut t, &[sample(101, "OrderService.exe", 4)]);
    }
    sink.write(&t.finish(at + 1000)).unwrap();

    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[dir.path().to_path_buf()]).unwrap();
    let tl = ws
        .timeline(&TimelineRequest {
            query: Default::default(),
            buckets: 50,
            always_show_lifecycle: true,
        })
        .unwrap();
    let lanes: Vec<&str> = tl.lanes.iter().map(|l| l.key.as_str()).collect();
    for pid in ["100", "101", "300"] {
        assert!(
            lanes.iter().any(|l| l.ends_with(&format!(" · {pid}"))),
            "lane for {pid} in {lanes:?}"
        );
    }
    let kinds = |pid: &str| -> Vec<String> {
        tl.lanes
            .iter()
            .filter(|l| l.key.ends_with(&format!(" · {pid}")))
            .flat_map(|l| l.markers.iter().map(|m| m.kind.clone()))
            .collect()
    };
    assert!(
        kinds("101").contains(&"start".to_string()),
        "restarted process shows a start marker: {:?}",
        kinds("101")
    );
    assert!(
        kinds("100").contains(&"stop".to_string()),
        "crashed process shows a stop marker: {:?}",
        kinds("100")
    );
    assert!(kinds("300").contains(&"start".to_string()));
    assert!(kinds("300").contains(&"stop".to_string()));
    assert!(
        !kinds("100").contains(&"start".to_string()),
        "already-running processes get no start marker"
    );
    let s = ws.summary();
    assert!(s.total > 10);
}

// ------------------------------------------------------------ .NET runtime counters

#[test]
fn config_dotnet_flag() {
    assert!(!cfg(json!({"targets": [{"pid": 1}]})).unwrap().dotnet);
    assert!(cfg(json!({"targets": [{"pid": 1}], "dotnet": true})).unwrap().dotnet);
    assert!(err(json!({"targets": [{"pid": 1}], "dotnet": "yes"})).contains("invalid monitor config"));
}

/// Pid 1 is a .NET Framework process (instance `svc`), pid 2 runs .NET Core.
struct FakeDotnet {
    fail_after: usize,
    calls: usize,
}
impl DotnetSource for FakeDotnet {
    fn collect(&mut self) -> Result<Vec<dotnet::ClrValue>> {
        use dotnet::{ClrCounter, ClrValue};
        self.calls += 1;
        if self.calls > self.fail_after {
            anyhow::bail!("clr broke");
        }
        let v = |counter, value| ClrValue {
            counter,
            instance: "svc".into(),
            value,
        };
        Ok(vec![
            v(ClrCounter::ProcessId, 1.0),
            v(ClrCounter::HeapBytes, 2048.0),
            v(ClrCounter::Exceptions, 10.0 * self.calls as f64),
        ])
    }
    fn runtime(&mut self, pid: u32) -> Option<dotnet::Runtime> {
        (pid == 2).then_some(dotnet::Runtime::Core)
    }
}

fn dotnet_probes(source: FakeDotnet, problems: Vec<String>) -> Probes {
    Probes {
        dotnet: Some(Ok(dotnet::Opened {
            source: Box::new(source),
            problems,
        })),
        ..Default::default()
    }
}

#[test]
fn run_loop_adds_dotnet_fields_to_stats_and_survives_failure() {
    let stop = AtomicBool::new(false);
    let sink = StopSink {
        records: Mutex::new(vec![]),
        writes: AtomicUsize::new(0),
        limit: 4,
        stop: &stop,
    };
    let tick = vec![sample(1, "svc", 100), sample(2, "svc-core", 100), sample(3, "svc-native", 100)];
    let mut src = FakeSource {
        script: vec![tick.clone(), tick.clone(), tick],
        calls: 0,
    };
    let probes = dotnet_probes(FakeDotnet { fail_after: 2, calls: 0 }, vec!["skipped: x".into()]);
    run_loop(&base_cfg(), &mut src, probes, Duration::from_millis(5), &sink, &stop, &clock()).unwrap();
    let recs = sink.records.lock().unwrap();
    assert_eq!(recs[0].message, "skipped: x");
    assert_eq!(recs[0].level, Level::Warning);
    let stats = |pid: u32| -> Vec<&Record> {
        recs.iter()
            .filter(|r| r.message.starts_with("Process stats") && r.fields["ProcessId"] == pid)
            .collect()
    };
    let fw = stats(1);
    assert_eq!(fw.len(), 3);
    assert_eq!(fw[0].fields["DotnetRuntime"], "framework");
    assert_eq!(fw[0].fields["DotnetHeapBytes"], 2048);
    assert!(fw[0].fields.get("DotnetExceptionsPerSec").is_none());
    assert_eq!(fw[1].fields["DotnetExceptionsPerSec"], 5.0, "10 exceptions in 2 s");
    assert!(fw[2].fields.get("DotnetRuntime").is_none(), "collection stopped after the failure");
    assert_eq!(stats(2)[0].fields["DotnetRuntime"], "core");
    assert!(stats(3).iter().all(|r| r.fields.get("DotnetRuntime").is_none()));
    assert_eq!(recs.iter().filter(|r| r.message.contains("EventPipe")).count(), 1);
    let failed: Vec<_> = recs
        .iter()
        .filter(|r| r.message.starts_with(".NET runtime counter collection failed"))
        .collect();
    assert_eq!(failed.len(), 1);
    assert!(failed[0].message.contains("clr broke"));
}

#[test]
fn run_loop_warns_once_when_dotnet_cannot_open() {
    let stop = AtomicBool::new(false);
    let sink = StopSink {
        records: Mutex::new(vec![]),
        writes: AtomicUsize::new(0),
        limit: 2,
        stop: &stop,
    };
    let mut src = FakeSource {
        script: vec![vec![sample(1, "svc", 1)]],
        calls: 0,
    };
    let probes = Probes {
        dotnet: Some(Err(anyhow::anyhow!("no pdh"))),
        ..Default::default()
    };
    run_loop(&base_cfg(), &mut src, probes, Duration::from_millis(5), &sink, &stop, &clock()).unwrap();
    let recs = sink.records.lock().unwrap();
    assert_eq!(recs[0].message, ".NET runtime counters unavailable: no pdh");
    assert_eq!(recs.iter().filter(|r| r.message.contains("unavailable")).count(), 1);
    // Sink failures while reporting open problems propagate.
    let mut src = FakeSource { script: vec![], calls: 0 };
    let probes = dotnet_probes(FakeDotnet { fail_after: 9, calls: 0 }, vec!["p".into()]);
    assert!(run_loop(
        &base_cfg(),
        &mut src,
        probes,
        Duration::from_millis(5),
        &FailingSink,
        &stop,
        &clock()
    )
    .is_err());
    let probes = Probes {
        dotnet: Some(Err(anyhow::anyhow!("x"))),
        ..Default::default()
    };
    assert!(run_loop(
        &base_cfg(),
        &mut src,
        probes,
        Duration::from_millis(5),
        &FailingSink,
        &stop,
        &clock()
    )
    .is_err());
}

#[test]
fn collector_opens_the_platform_dotnet_source() {
    // The real source opens (Windows) or reports the platform limit once (elsewhere).
    let me = std::process::id();
    let stop = AtomicBool::new(false);
    let sink = StopSink {
        records: Mutex::new(vec![]),
        writes: AtomicUsize::new(0),
        limit: 3,
        stop: &stop,
    };
    ProcessMonitor
        .run(&json!({"targets": [{"pid": me}], "intervalMs": 250, "dotnet": true}), &sink, &stop)
        .unwrap();
    let recs = sink.records.lock().unwrap();
    // This test binary is native: it never gets .NET fields.
    assert!(recs.iter().all(|r| r.fields.get("DotnetRuntime").is_none()));
    if cfg!(not(windows)) {
        assert!(recs[0].message.contains("only available on Windows"));
    }
}
