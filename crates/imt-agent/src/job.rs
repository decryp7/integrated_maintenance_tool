//! One run of one schedule: lock, disk check, run folder and manifest,
//! actions in order (a crashing action fails only itself), retention.

use crate::action::ActionExecutor;
use crate::clock::Clock;
use crate::config::{AgentConfig, Schedule};
use crate::disk::{check_free, DiskProbe};
use crate::lock::{LockError, ProcessProbe, RunLock};
use crate::log::AgentLog;
use crate::run::{create_run_dir, dir_size, write_manifest, ActionRecord, ActionStatus, RunManifest, RunReason, RunStatus};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Everything a run needs from the outside world.
#[derive(Clone)]
pub struct JobDeps {
    pub executor: Arc<dyn ActionExecutor>,
    pub probe: Arc<dyn ProcessProbe>,
    pub disk: Arc<dyn DiskProbe>,
    pub clock: Arc<dyn Clock>,
    pub log: Arc<AgentLog>,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// It ran; its folder id and final status.
    Ran { id: String, status: RunStatus },
    /// It did not run, and why (overlap, low disk space, folder not writable).
    Skipped(String),
}

/// Runs `schedule` now.
pub fn run_schedule(cfg: &AgentConfig, schedule: &Schedule, reason: RunReason, deps: &JobDeps, stop: &AtomicBool) -> Outcome {
    let log = &deps.log;
    let name = Value::from(schedule.name.as_str());
    let started = deps.clock.now();
    let _lock = match RunLock::acquire(
        &cfg.state_dir.join("locks"),
        &schedule.name,
        started.timestamp_millis(),
        &*deps.probe,
    ) {
        Ok(l) => l,
        Err(e) => {
            let why = e.to_string();
            match e {
                LockError::Held { .. } => log.info(
                    "Run skipped: the previous run is still going",
                    &[("Schedule", name), ("Reason", Value::from(why.clone()))],
                ),
                LockError::Io(..) => log.error(
                    "Run skipped: cannot take the run lock",
                    &[("Schedule", name), ("Reason", Value::from(why.clone()))],
                ),
            }
            return Outcome::Skipped(why);
        }
    };
    if let Err(why) = check_free(&*deps.disk, &cfg.output_dir, cfg.min_free_bytes) {
        log.warn(
            "Run skipped: not enough free disk space",
            &[("Schedule", name), ("Reason", Value::from(why.clone()))],
        );
        return Outcome::Skipped(why);
    }
    let (id, dir) = match create_run_dir(&cfg.output_dir, &schedule.name, started) {
        Ok(x) => x,
        Err(e) => {
            let why = format!("cannot create a run folder in {}: {e}", cfg.output_dir.display());
            log.error(
                "Run skipped: cannot create its folder",
                &[("Schedule", name), ("Reason", Value::from(why.clone()))],
            );
            return Outcome::Skipped(why);
        }
    };
    log.info(
        "Run started",
        &[
            ("Schedule", name.clone()),
            ("RunId", Value::from(id.as_str())),
            ("Reason", Value::from(reason.label())),
        ],
    );
    let mut manifest = RunManifest::start(&id, &schedule.name, reason, started);
    let save = |m: &RunManifest| {
        if let Err(e) = write_manifest(&dir, m) {
            log.error(
                "Cannot write the run manifest",
                &[("RunId", Value::from(id.as_str())), ("Error", Value::from(e.to_string()))],
            );
        }
    };
    save(&manifest);
    for action in &schedule.actions {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let rec = imt_core::guard::catch("imt-agent", || Ok(deps.executor.execute(action, &dir, stop)))
            .unwrap_or_else(|e| ActionRecord::new(action.kind(), ActionStatus::Failed, Some(format!("{e:#} (please report this)"))));
        let level_failed = rec.status == ActionStatus::Failed;
        let props = [
            ("Schedule", name.clone()),
            ("Action", Value::from(rec.kind.as_str())),
            ("Status", serde_json::to_value(rec.status).unwrap_or_default()),
            ("Detail", Value::from(rec.message.clone().unwrap_or_default())),
        ];
        if level_failed {
            log.warn("Action failed", &props);
        } else {
            log.info("Action finished", &props);
        }
        manifest.actions.push(rec);
        save(&manifest);
    }
    let stopped = stop.load(Ordering::Relaxed) && manifest.actions.len() < schedule.actions.len();
    let (deleted, errors) = crate::retention::apply(&cfg.output_dir, &cfg.retention, deps.clock.now().timestamp_millis(), &id);
    for e in &errors {
        log.warn("Retention could not delete a run", &[("Error", Value::from(e.as_str()))]);
    }
    if !deleted.is_empty() {
        log.info(
            "Old runs deleted",
            &[("Count", Value::from(deleted.len())), ("Runs", Value::from(deleted))],
        );
    }
    manifest.finish(deps.clock.now(), stopped, dir_size(&dir));
    save(&manifest);
    log.info(
        "Run finished",
        &[
            ("Schedule", name),
            ("RunId", Value::from(id.as_str())),
            ("Status", serde_json::to_value(manifest.status).unwrap_or_default()),
            ("DurationMs", Value::from(manifest.duration_ms)),
        ],
    );
    Outcome::Ran {
        id,
        status: manifest.status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Action, AgentConfigFile, When};
    use crate::run::read_manifest;
    use crate::schedule::Timer;
    use crate::testing::{AliveProbe as Probe, FakeClock, FakeExecutor, FixedDisk as Disk, MemorySink};
    use std::path::Path;
    use std::time::Duration;

    fn setup(out: &Path, free: Option<u64>, panic_on: Option<&'static str>) -> (AgentConfig, JobDeps, MemorySink, Arc<FakeExecutor>) {
        let mut file = AgentConfigFile::starter(&out.to_string_lossy());
        file.retention.max_runs = Some(2);
        let mut cfg = file.validate(out).unwrap();
        cfg.schedules[0].actions.push(Action::CopyLogs(imt_collect::LogSpec {
            path: "/x".into(),
            include: vec![],
            max_age_days: None,
            max_total_mb: None,
        }));
        let sink = MemorySink::default();
        let exec = Arc::new(FakeExecutor::new(panic_on));
        let deps = JobDeps {
            executor: exec.clone(),
            probe: Arc::new(Probe),
            disk: Arc::new(Disk(free)),
            clock: FakeClock::at(2026, 10, 17, 2, 0),
            log: Arc::new(AgentLog::with_sink(Box::new(sink.clone()), crate::config::LogLevel::Debug, false)),
        };
        (cfg, deps, sink, exec)
    }

    #[test]
    fn a_run_writes_its_folder_manifest_and_log() {
        let out = tempfile::tempdir().unwrap();
        let (cfg, deps, sink, _) = setup(out.path(), None, None);
        let s = &cfg.schedules[0];
        assert!(matches!(s.when, When::Timer(Timer::Cron(_))));
        let outcome = run_schedule(&cfg, s, RunReason::Manual, &deps, &AtomicBool::new(false));
        let Outcome::Ran { id, status } = outcome else {
            panic!("{outcome:?}")
        };
        assert_eq!(id, "20261017-020000-nightly");
        assert_eq!(status, RunStatus::Ok);
        let m = read_manifest(&out.path().join(&id)).unwrap();
        assert_eq!(m.actions.len(), 2);
        // Everything but the final manifest itself.
        assert!(m.total_bytes > 0 && m.total_bytes <= dir_size(&out.path().join(&id)));
        assert!(m.finished_at.is_some());
        let msgs = sink.messages();
        assert!(
            msgs.contains(&"Run started".to_string()) && msgs.contains(&"Run finished".to_string()),
            "{msgs:?}"
        );
        // The lock is released afterwards.
        assert!(!cfg.state_dir.join("locks/nightly.lock").exists());
    }

    #[test]
    fn a_crashing_action_fails_alone() {
        let out = tempfile::tempdir().unwrap();
        let (cfg, deps, sink, exec) = setup(out.path(), None, Some("collect"));
        let Outcome::Ran { id, status } = run_schedule(&cfg, &cfg.schedules[0], RunReason::Manual, &deps, &AtomicBool::new(false)) else {
            panic!()
        };
        assert_eq!(status, RunStatus::Partial);
        assert_eq!(exec.ran.lock().unwrap().as_slice(), ["collect", "copyLogs"]);
        let m = read_manifest(&out.path().join(id)).unwrap();
        assert_eq!(m.actions[0].status, ActionStatus::Failed);
        assert!(m.actions[0].message.as_deref().unwrap().contains("executor bug"));
        assert!(sink.messages().contains(&"Action failed".to_string()));
    }

    #[test]
    fn skipped_when_locked_low_on_disk_or_unwritable() {
        let out = tempfile::tempdir().unwrap();
        let (cfg, deps, sink, _) = setup(out.path(), Some(1), None);
        let s = &cfg.schedules[0];
        let Outcome::Skipped(why) = run_schedule(&cfg, s, RunReason::Manual, &deps, &AtomicBool::new(false)) else {
            panic!()
        };
        assert!(why.contains("MB free"), "{why}");
        assert!(sink.messages().iter().any(|m| m.contains("not enough free disk space")));

        let (cfg, deps, _, exec) = setup(out.path(), None, None);
        let held = RunLock::acquire(&cfg.state_dir.join("locks"), "nightly", 0, &Probe).unwrap();
        let Outcome::Skipped(why) = run_schedule(&cfg, s, RunReason::Manual, &deps, &AtomicBool::new(false)) else {
            panic!()
        };
        assert!(why.contains("already running"));
        assert!(exec.ran.lock().unwrap().is_empty());
        drop(held);

        // The output folder is a file.
        let file = out.path().join("file");
        std::fs::write(&file, "x").unwrap();
        let (mut cfg, deps, _, _) = setup(out.path(), None, None);
        cfg.output_dir = file.clone();
        let Outcome::Skipped(why) = run_schedule(&cfg, s, RunReason::Manual, &deps, &AtomicBool::new(false)) else {
            panic!()
        };
        assert!(why.contains("cannot create a run folder"));
        // The lock folder cannot be created.
        cfg.state_dir = file.join("state");
        let Outcome::Skipped(why) = run_schedule(&cfg, s, RunReason::Manual, &deps, &AtomicBool::new(false)) else {
            panic!()
        };
        assert!(why.contains("lock"));
    }

    #[test]
    fn stop_before_actions_and_retention() {
        let out = tempfile::tempdir().unwrap();
        let (cfg, deps, sink, exec) = setup(out.path(), None, None);
        let s = &cfg.schedules[0];
        let Outcome::Ran { status, .. } = run_schedule(&cfg, s, RunReason::Manual, &deps, &AtomicBool::new(true)) else {
            panic!()
        };
        assert_eq!(status, RunStatus::Stopped);
        assert!(exec.ran.lock().unwrap().is_empty());
        // maxRuns = 2: the third run deletes the oldest.
        let clock = FakeClock::at(2026, 10, 17, 3, 0);
        let deps = JobDeps {
            clock: clock.clone(),
            ..deps
        };
        for _ in 0..2 {
            clock.advance(Duration::from_secs(60));
            run_schedule(&cfg, s, RunReason::Manual, &deps, &AtomicBool::new(false));
        }
        assert!(sink.messages().contains(&"Old runs deleted".to_string()), "{:?}", sink.messages());
        assert_eq!(crate::run::list_runs(out.path()).len(), 2);
    }
}
