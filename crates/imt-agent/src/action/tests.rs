use super::*;
use imt_collect::{CollectPlan, LogSpec};
use imt_collect::{CommandError, CommandOutput, CommandRunner};
use imt_plugin_tools::perfmon_control::{CollectorStatus, PerfmonController};
use std::sync::Arc;
use std::sync::Mutex;

fn exec(action: &Action, dir: &Path) -> ActionRecord {
    SystemExecutor {
        perfmon: false,
        ..SystemExecutor::default()
    }
    .execute(action, dir, &AtomicBool::new(false))
}

#[test]
fn collect_writes_a_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let plan = CollectPlan {
        name: "agent-test".into(),
        event_logs: vec![],
        ..CollectPlan::default()
    };
    let rec = exec(&Action::Collect(Box::new(plan)), dir.path());
    assert_eq!(rec.status, ActionStatus::Ok, "{rec:?}");
    assert!(rec.outputs[0].starts_with("collect/agent-test-") && rec.outputs[0].ends_with(".imtz"));
    assert!(rec.bytes > 0);
    // An invalid plan (built in code) fails the action, not the agent.
    let bad = CollectPlan {
        max_bundle_mb: 0,
        ..CollectPlan::default()
    };
    let rec = exec(&Action::Collect(Box::new(bad)), dir.path());
    assert_eq!(rec.status, ActionStatus::Failed);
    assert!(rec.message.unwrap().contains("maxBundleMb"));
}

#[test]
fn monitor_records_for_its_duration_or_until_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let settings = serde_json::json!({ "targets": [{ "pid": std::process::id() }], "intervalMs": 250 });
    let action = Action::Monitor {
        settings: settings.clone(),
        duration: Duration::from_millis(600),
    };
    let rec = exec(&action, dir.path());
    assert_eq!(rec.status, ActionStatus::Ok, "{rec:?}");
    assert_eq!(rec.outputs, ["monitor"]);
    let text = std::fs::read_to_string(dir.path().join("monitor/process-monitor.clef")).unwrap();
    assert!(text.contains("Process stats"), "{text}");
    assert!(rec.bytes > 0);

    let stop = AtomicBool::new(true);
    let long = Action::Monitor {
        settings,
        duration: Duration::from_secs(3600),
    };
    let rec = SystemExecutor::default().execute(&long, dir.path(), &stop);
    assert_eq!(rec.status, ActionStatus::Stopped);

    let bad = Action::Monitor {
        settings: serde_json::json!({ "targets": [] }),
        duration: Duration::from_secs(1),
    };
    assert_eq!(exec(&bad, dir.path()).status, ActionStatus::Failed);
    // The output folder cannot be created.
    let file = dir.path().join("file");
    std::fs::write(&file, "x").unwrap();
    assert_eq!(exec(&bad, &file).status, ActionStatus::Failed);
}

#[test]
fn copy_logs_within_limits() {
    let src = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(src.path().join("sub")).unwrap();
    std::fs::write(src.path().join("a.log"), "aaaa").unwrap();
    std::fs::write(src.path().join("sub/b.log"), "bb").unwrap();
    std::fs::write(src.path().join("c.bin"), "c").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let spec = LogSpec {
        path: src.path().to_string_lossy().into_owned(),
        include: vec!["*.log".into()],
        max_age_days: Some(1),
        max_total_mb: Some(1),
    };
    let rec = exec(&Action::CopyLogs(spec.clone()), dir.path());
    assert_eq!(rec.status, ActionStatus::Ok, "{rec:?}");
    assert_eq!(rec.bytes, 6);
    assert_eq!(std::fs::read_to_string(dir.path().join("logs/sub/b.log")).unwrap(), "bb");
    assert!(!dir.path().join("logs/c.bin").exists());
    assert!(rec.message.unwrap().starts_with("2 file(s) copied"));

    // Nothing to copy: skipped. A missing folder: failed with the reason.
    let none = LogSpec {
        include: vec!["*.nothing".into()],
        ..spec.clone()
    };
    assert_eq!(exec(&Action::CopyLogs(none), dir.path()).status, ActionStatus::Skipped);
    let missing = LogSpec {
        path: src.path().join("gone").to_string_lossy().into_owned(),
        ..spec.clone()
    };
    let rec = exec(&Action::CopyLogs(missing), dir.path());
    assert_eq!(rec.status, ActionStatus::Failed);
    assert!(rec.message.unwrap().contains("does not exist"));
    // Stopped before the first file.
    let rec = SystemExecutor::default().execute(&Action::CopyLogs(spec.clone()), dir.path(), &AtomicBool::new(true));
    assert_eq!(rec.status, ActionStatus::Stopped);
    // The destination cannot be written.
    let file = dir.path().join("file");
    std::fs::write(&file, "x").unwrap();
    let rec = exec(&Action::CopyLogs(spec), &file);
    assert_eq!(rec.status, ActionStatus::Failed);
}

#[test]
fn safe_relative_paths() {
    assert!(safe_rel("a/b.log").is_some());
    assert!(safe_rel("../x").is_none());
    assert!(safe_rel("/abs").is_none());
}

#[test]
fn perfmon_is_skipped_off_windows() {
    let dir = tempfile::tempdir().unwrap();
    let action = Action::Perfmon {
        collector: "IMT".into(),
        duration: Duration::from_secs(1),
    };
    let rec = exec(&action, dir.path());
    assert_eq!(rec.status, ActionStatus::Skipped);
    assert!(rec.message.unwrap().contains("only available on Windows"));
    if !cfg!(windows) {
        // The real controller refuses on this platform.
        let real = SystemExecutor {
            perfmon: true,
            ..SystemExecutor::default()
        };
        let rec = real.execute(&action, dir.path(), &AtomicBool::new(false));
        assert_eq!(rec.status, ActionStatus::Failed);
    }
}

/// A scripted `logman`.
struct FakeLogman {
    calls: Mutex<Vec<String>>,
    fail_on: Option<&'static str>,
}

#[test]
fn logman_runs_with_the_configured_timeout() {
    struct Timeouts(Mutex<Vec<Duration>>);
    impl CommandRunner for Timeouts {
        fn run(&self, _: &str, _: &[String], timeout: Duration) -> Result<CommandOutput, CommandError> {
            self.0.lock().unwrap().push(timeout);
            Err(CommandError::NotFound("logman".into()))
        }
    }
    let seen = Arc::new(Timeouts(Mutex::new(Vec::new())));
    let default = SystemExecutor::default();
    assert_eq!(default.logman_timeout, imt_plugin_tools::perfmon_control::DEFAULT_TIMEOUT);
    let exec = SystemExecutor::with_logman_timeout(Duration::from_secs(42));
    assert_eq!(exec.perfmon, default.perfmon);
    let _ = exec.perfmon_controller(seen.clone(), true).start("IMT");
    assert_eq!(*seen.0.lock().unwrap(), [Duration::from_secs(42)]);
}

impl CommandRunner for FakeLogman {
    fn run(&self, _program: &str, args: &[String], _timeout: Duration) -> Result<CommandOutput, CommandError> {
        self.calls.lock().unwrap().push(args[0].clone());
        if self.fail_on == Some(args[0].as_str()) {
            return Ok(CommandOutput {
                exit_code: Some(1),
                stdout: Vec::new(),
                stderr: "Data Collector Set was not found.".into(),
            });
        }
        let stdout = if args[0] == "query" { "Status: Stopped\n" } else { "" };
        Ok(CommandOutput {
            exit_code: Some(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: String::new(),
        })
    }
}

#[test]
fn perfmon_capture_with_a_fake_logman() {
    let src = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let fake = || FakeLogman {
        calls: Mutex::new(Vec::new()),
        fail_on: None,
    };
    let produced = src.path().join("IMT_000001.blg");
    let outputs = |_: &CollectorStatus| {
        std::fs::write(src.path().join("IMT_000001.blg"), "blg").unwrap();
        vec![produced.clone(), src.path().join("missing.blg")]
    };
    let c = PerfmonController::with_platform(Arc::new(fake()), true);
    let rec = perfmon_with(&c, "IMT", Duration::from_millis(30), dir.path(), &AtomicBool::new(false), &outputs);
    assert_eq!(rec.status, ActionStatus::Ok, "{rec:?}");
    assert_eq!(rec.outputs, ["perfmon/IMT_000001.blg"]);
    assert_eq!(rec.bytes, 3);

    // Nothing new: a note. Stopped early: status Stopped.
    let rec = perfmon_with(&c, "IMT", Duration::from_secs(60), dir.path(), &AtomicBool::new(true), &|_| {
        Vec::new()
    });
    assert_eq!(rec.status, ActionStatus::Stopped);
    assert!(rec.message.unwrap().contains("no new log file"));
    // The destination cannot be written.
    let file = dir.path().join("file");
    std::fs::write(&file, "x").unwrap();
    let rec = perfmon_with(&c, "IMT", Duration::from_millis(1), &file, &AtomicBool::new(false), &outputs);
    assert!(rec.message.unwrap().contains("cannot copy"));

    for step in ["start", "stop", "query"] {
        let c = PerfmonController::with_platform(
            Arc::new(FakeLogman {
                calls: Mutex::new(Vec::new()),
                fail_on: Some(step),
            }),
            true,
        );
        let rec = perfmon_with(&c, "IMT", Duration::from_millis(1), dir.path(), &AtomicBool::new(false), &outputs);
        assert_eq!(rec.status, ActionStatus::Failed, "{step}");
        assert!(rec.message.unwrap().contains("logman failed"), "{step}");
    }
}
