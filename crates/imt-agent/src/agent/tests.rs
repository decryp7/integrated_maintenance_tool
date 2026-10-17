use super::*;
use crate::config::{AgentConfigFile, LogLevel};
use crate::log::AgentLog;
use crate::testing::{AliveProbe, FakeClock, FakeExecutor, FixedDisk, MemorySink};
use imt_monitor::config::Targets;
use imt_monitor::source::ProcSample;
use std::path::Path;
use std::time::Duration;

struct HighSampler;
impl Sampler for HighSampler {
    fn sample(&mut self, _: &Targets) -> Vec<ProcSample> {
        vec![ProcSample {
            pid: 7,
            name: "svc".into(),
            private_bytes: Some(4 << 30),
            ..ProcSample::default()
        }]
    }
    fn cpu_count(&self) -> usize {
        1
    }
}

struct Env {
    clock: Arc<FakeClock>,
    sink: MemorySink,
    exec: Arc<FakeExecutor>,
    deps: Deps,
}

fn env() -> Env {
    let clock = FakeClock::at(2026, 10, 17, 1, 59);
    let sink = MemorySink::default();
    let exec = Arc::new(FakeExecutor::new(None));
    let deps = Deps {
        job: JobDeps {
            executor: exec.clone(),
            probe: Arc::new(AliveProbe),
            disk: Arc::new(FixedDisk(None)),
            clock: clock.clone(),
            log: Arc::new(AgentLog::with_sink(Box::new(sink.clone()), LogLevel::Debug, false)),
        },
        sampler: Arc::new(|| Box::new(HighSampler)),
    };
    Env { clock, sink, exec, deps }
}

fn config(out: &Path, extra: &str) -> AgentConfig {
    let text = format!(
        r#"
version = 1
[agent]
outputDir = "{}"
timezone = "utc"
[[schedules]]
name = "nightly"
cron = "0 2 * * *"
  [[schedules.actions]]
  kind = "collect"
[[schedules]]
name = "off"
enabled = false
every = "1h"
  [[schedules.actions]]
  kind = "collect"
{extra}"#,
        out.display().to_string().replace('\\', "/")
    );
    AgentConfigFile::parse(&text).unwrap().validate(out).unwrap()
}

fn wait_idle(agent: &mut Agent) {
    for _ in 0..500 {
        agent.reap(false);
        if agent.running.is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("runs did not finish");
}

#[test]
fn timers_fire_when_due_and_state_is_recorded() {
    let out = tempfile::tempdir().unwrap();
    let e = env();
    let mut agent = Agent::new(config(out.path(), ""), e.deps.clone(), Arc::new(AtomicBool::new(false)));
    agent.tick();
    assert!(agent.running.is_empty(), "not 02:00 yet");
    e.clock.advance(Duration::from_secs(60));
    agent.tick();
    assert!(agent.running.contains_key("nightly"));
    // While it runs it is not started again.
    assert!(agent.due().is_empty());
    wait_idle(&mut agent);
    let st = agent.state().get("nightly");
    assert_eq!(st.last_status, Some(RunStatus::Ok));
    assert_eq!(st.last_run_id.as_deref(), Some("20261017-020000-nightly"));
    assert_eq!(
        e.exec.ran.lock().unwrap().as_slice(),
        ["collect"],
        "the disabled schedule never ran"
    );
    // Not due again until tomorrow; state survives a restart.
    e.clock.advance(Duration::from_secs(3600));
    assert!(agent.due().is_empty());
    let again = Agent::new(config(out.path(), ""), e.deps.clone(), Arc::new(AtomicBool::new(false)));
    assert_eq!(again.state().get("nightly").last_status, Some(RunStatus::Ok));
    assert_eq!(again.anchors["nightly"].timestamp_millis(), st.last_run_ms.unwrap());
}

#[test]
fn corrupt_state_is_logged_and_replaced() {
    let out = tempfile::tempdir().unwrap();
    let e = env();
    let cfg = config(out.path(), "");
    std::fs::create_dir_all(&cfg.state_dir).unwrap();
    std::fs::write(cfg.state_dir.join("state.json"), "garbage").unwrap();
    let agent = Agent::new(cfg, e.deps.clone(), Arc::new(AtomicBool::new(false)));
    assert!(agent.state().schedules.is_empty());
    assert!(e.sink.messages().iter().any(|m| m.contains("state was unusable")));
}

const TRIGGERS: &str = r#"
[[schedules]]
name = "spike"
  [schedules.trigger]
  kind = "process"
  process = "svc"
  metric = "privateBytes"
  above = 100
  [[schedules.actions]]
  kind = "collect"
[[schedules]]
name = "errors"
  [schedules.trigger]
  kind = "log"
  path = "missing.log"
  ql = "level >= Error"
  [[schedules.actions]]
  kind = "collect"
"#;

#[test]
fn triggers_start_runs() {
    let out = tempfile::tempdir().unwrap();
    let e = env();
    let mut agent = Agent::new(config(out.path(), TRIGGERS), e.deps.clone(), Arc::new(AtomicBool::new(false)));
    let due = agent.due();
    assert_eq!(due.len(), 1, "{due:?}");
    let (i, RunReason::Trigger { detail }) = &due[0] else {
        panic!("{due:?}")
    };
    assert_eq!(agent.cfg.schedules[*i].name, "spike");
    assert!(detail.contains("svc (7)"), "{detail}");
    agent.start(*i, due[0].1.clone());
    wait_idle(&mut agent);
    assert_eq!(agent.state().get("spike").last_status, Some(RunStatus::Ok));
}

#[test]
fn trigger_errors_are_logged() {
    let out = tempfile::tempdir().unwrap();
    let e = env();
    let mut cfg = config(out.path(), TRIGGERS);
    // A log trigger whose query fails (built in code, bypassing validation) only logs.
    let log = out.path().join("app.log");
    std::fs::write(&log, "2026-10-17 10:00:00.000 [INF] started\n").unwrap();
    if let When::Trigger(TriggerSpec::Log(l)) = &mut cfg.schedules[3].when {
        l.path = log;
        l.ql = "level >= ".into();
    }
    // A process trigger whose pattern cannot compile is disabled with an error.
    if let When::Trigger(TriggerSpec::Process(p)) = &mut cfg.schedules[2].when {
        p.process = "a[".into();
    }
    let mut agent = Agent::new(cfg, e.deps.clone(), Arc::new(AtomicBool::new(false)));
    assert!(e.sink.messages().contains(&"Trigger disabled".to_string()));
    for _ in 0..2 {
        agent.due();
        e.clock.advance(Duration::from_secs(10));
    }
    assert!(
        e.sink.messages().contains(&"Trigger check failed".to_string()),
        "{:?}",
        e.sink.messages()
    );
}

#[test]
fn run_loop_stops_and_waits() {
    let out = tempfile::tempdir().unwrap();
    let e = env();
    let stop = Arc::new(AtomicBool::new(false));
    let mut agent = Agent::new(config(out.path(), ""), e.deps.clone(), stop.clone());
    let flag = stop.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        flag.store(true, Ordering::Relaxed);
    });
    agent.run();
    t.join().unwrap();
    assert!(agent.running.is_empty());
    let msgs = e.sink.messages();
    assert_eq!(msgs.first().map(String::as_str), Some("Agent started"));
    assert_eq!(msgs.last().map(String::as_str), Some("Agent stopped"));
    assert!(msgs.contains(&"Run finished".to_string()), "the fake clock passed 02:00");
}

#[test]
fn run_once_and_exit_codes() {
    let out = tempfile::tempdir().unwrap();
    let e = env();
    let mut agent = Agent::new(config(out.path(), ""), e.deps.clone(), Arc::new(AtomicBool::new(false)));
    let all = agent.run_once(None).unwrap();
    assert_eq!(all.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["nightly"]);
    assert_eq!(Agent::exit_code(&all), 0);
    let off = agent.run_once(Some("off")).unwrap();
    assert_eq!(off.len(), 1, "a disabled schedule runs when named");
    assert!(agent.run_once(Some("nope")).unwrap_err().contains("no schedule called"));
    assert_eq!(Agent::exit_code(&[("x".into(), Outcome::Skipped("busy".into()))]), 1);
    // Stopped: nothing more runs.
    let mut stopped = Agent::new(config(out.path(), ""), e.deps, Arc::new(AtomicBool::new(true)));
    assert!(stopped.run_once(None).unwrap().is_empty());
}

#[test]
fn state_save_failures_are_logged() {
    let out = tempfile::tempdir().unwrap();
    let e = env();
    let mut cfg = config(out.path(), "");
    let file = out.path().join("file");
    std::fs::write(&file, "x").unwrap();
    cfg.state_dir = file.join("state");
    let mut agent = Agent::new(cfg, e.deps.clone(), Arc::new(AtomicBool::new(false)));
    agent.run_once(Some("nightly")).unwrap();
    assert!(e.sink.messages().contains(&"Cannot save the agent state".to_string()));
}

#[test]
fn system_deps_build() {
    let out = tempfile::tempdir().unwrap();
    let log = Arc::new(AgentLog::with_sink(Box::new(MemorySink::default()), LogLevel::Error, false));
    let d = Deps::system(log, &config(out.path(), ""));
    assert!((d.sampler)().cpu_count() >= 1);
}
