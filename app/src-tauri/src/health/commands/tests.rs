use super::*;
use crate::health::model::{RepairAction, ServiceHealth};
use crate::notification_centre::centre::{Centre, NoticeId};

fn snapshot_with(service: &str, state: ServiceState, cause: Option<Cause>, repair: Option<RepairAction>) -> (Snapshot, Change) {
    let snapshot = Snapshot {
        revision: 1,
        services: vec![ServiceHealth {
            service: service.into(),
            label: "the log store".into(),
            state,
            cause,
            repair,
        }],
    };
    let change = Change {
        service: service.into(),
        from: ServiceState::Ok,
        to: state,
    };
    (snapshot, change)
}

#[test]
fn an_outage_is_one_error_notification_with_the_services_cause_key_and_a_repair_button() {
    let (s, c) = snapshot_with(
        STORE,
        ServiceState::Down,
        Some(Cause::new("Debrief's log store stopped unexpectedly.").hint("Choose Try again.")),
        Some(RepairAction::Restart),
    );
    let n = notice_for(&s, &c).unwrap();
    assert_eq!(
        (n.cause.as_str(), n.severity, n.source),
        ("health:store", NoticeSeverity::Error, NoticeSource::Health)
    );
    assert_eq!(n.title, "Debrief's log store stopped unexpectedly.");
    assert_eq!(n.body, "Choose Try again.");
    assert_eq!(n.primary, Some(NoticeAction::RepairService { service: "store".into() }));
}

#[test]
fn the_same_outage_three_times_is_one_notification_and_ok_resolves_it() {
    let (s, c) = snapshot_with(STORE, ServiceState::Down, Some(Cause::new("stopped")), Some(RepairAction::Restart));
    let mut centre = Centre::new(10);
    for i in 0..3u64 {
        centre.post(notice_for(&s, &c).unwrap(), NoticeId(i + 1), 100 + i as i64);
    }
    assert_eq!(centre.notices().len(), 1, "same cause: updated in place");
    assert_eq!(centre.notices()[0].count, 3);
    assert!(centre.resolve_cause("health:store").is_some());
    assert!(centre.notices().is_empty());
}

#[test]
fn a_repair_in_progress_and_a_recovery_post_nothing() {
    let (s, c) = snapshot_with(
        STORE,
        ServiceState::Repairing,
        Some(Cause::new("stopped")),
        Some(RepairAction::Restart),
    );
    assert!(notice_for(&s, &c).is_none());
    let (s, c) = snapshot_with(STORE, ServiceState::Ok, None, None);
    assert!(notice_for(&s, &c).is_none());
}

#[test]
fn a_degraded_service_is_a_warning_without_a_button_when_nothing_repairs_it() {
    let (s, c) = snapshot_with(AI, ServiceState::Degraded, Some(Cause::new("slow")), None);
    let n = notice_for(&s, &c).unwrap();
    assert_eq!(n.severity, NoticeSeverity::Warning);
    assert!(n.primary.is_none());
}

#[test]
fn the_ai_states_map_to_health() {
    assert_eq!(ai_health(EngineStateDto::Disabled), AiHealth::Switched);
    assert_eq!(ai_health(EngineStateDto::Failed), AiHealth::LoadFailed);
    for ok in [
        EngineStateDto::Ready,
        EngineStateDto::Idle,
        EngineStateDto::Loading,
        EngineStateDto::NoModel,
        EngineStateDto::Unavailable,
    ] {
        assert_eq!(ai_health(ok), AiHealth::Fine);
    }
}

#[test]
fn the_engines_store_error_names_the_cause_and_reports_the_outage_once() {
    let seen = Arc::new(std::sync::Mutex::new(0usize));
    let s = seen.clone();
    let h = Health::new(Box::new(move |_, _| *s.lock().unwrap() += 1));
    hub::use_in_this_thread(h.clone());
    let store = debrief_core::ports::StoreError::Engine("the store worker exited (signal: 6)".into());
    let e: CmdError = crate::engine::store_error(store.clone());
    let again: CmdError = crate::engine::store_error(store);
    assert_eq!(e.cause, Some(crate::error::ServiceCause::Store));
    assert_eq!(serde_json::to_value(&e).unwrap()["cause"], "health:store");
    assert_eq!(e, again);
    assert_eq!(h.state(STORE), Some(ServiceState::Down));
    assert_eq!(*seen.lock().unwrap(), 1);
}

#[test]
fn an_error_names_a_service_outage_only_while_that_service_is_down() {
    let h = Health::new(Box::new(|_, _| {}));
    hub::use_in_this_thread(h.clone());
    let ai = || CmdError::from(debrief_ai::AiError::Engine("the worker exited".into()));
    assert_eq!(ai().cause, None, "the AI worker works: this error is the request's own");
    h.ai(AiHealth::Switched, Some("crashed"));
    assert_eq!(ai().cause, Some(crate::error::ServiceCause::Ai));
    assert_eq!(serde_json::to_value(ai()).unwrap()["cause"], "health:ai");
    // Another service's state does not matter, and codes that no service explains never carry a cause.
    assert_eq!(CmdError::from(debrief_ai::AiError::Browser("x".into())).cause, None);
    h.browser_missing("none");
    assert_eq!(
        CmdError::from(debrief_ai::AiError::Browser("x".into())).cause,
        Some(crate::error::ServiceCause::Browser)
    );
    assert_eq!(CmdError::from(debrief_ai::AiError::NoModel).cause, None);
    h.download_failed(1, "Download", "503");
    assert_eq!(
        CmdError::from(debrief_ai::AiError::Network("down".into())).cause,
        Some(crate::error::ServiceCause::Downloads)
    );
    h.plugin_quarantined("p", "p", "crashed");
    let tool = crate::tools::tests::missing_tool_error("x.y");
    assert_eq!(tool.cause, Some(crate::error::ServiceCause::Plugins));
}

/// The store worker binary built for this test run (`cargo build -p debrief-store-worker` into the same target folder, a
/// no-op when it is current). It is Debrief's own worker from the build tree: the installed app is never touched.
fn built_store_worker() -> std::path::PathBuf {
    let test_exe = std::env::current_exe().unwrap();
    let profile_dir = test_exe.parent().and_then(std::path::Path::parent).unwrap().to_path_buf();
    let mut build = debrief_plugin_api::process::quiet_command(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    build.args(["build", "-q", "-p", "debrief-store-worker"]);
    if profile_dir.file_name().is_some_and(|n| n == "release") {
        build.arg("--release");
    }
    let status = build.current_dir(env!("CARGO_MANIFEST_DIR")).status().expect("cargo runs");
    assert!(status.success(), "the store worker builds");
    let exe = profile_dir.join(format!("debrief-store-worker{}", std::env::consts::EXE_SUFFIX));
    assert!(exe.is_file(), "{} exists", exe.display());
    exe
}

fn process_is_alive(pid: u32) -> bool {
    let p = sysinfo::Pid::from_u32(pid);
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
    sys.process(p).is_some()
}

/// Kills the worker child this test started (its pid comes from the supervisor, never from a name).
fn kill_process(pid: u32) {
    let p = sysinfo::Pid::from_u32(pid);
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
    if let Some(process) = sys.process(p) {
        process.kill();
    }
}

/// Stops the worker even when an assertion fails: no process is left behind.
struct StopOnDrop(debrief_store_client::StoreWorker);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

#[test]
fn a_real_store_worker_killed_over_and_over_is_quarantined_then_repair_restarts_it() {
    use crate::test_support::test_app_with_commands;
    use debrief_ipc::supervisor::ChildState;
    let app = test_app_with_commands(None);
    let health = app.state::<Arc<Health>>().inner().clone();
    // A real worker process, started from this build tree and supervised like the app's.
    let tmp = crate::test_support::TempDir::new("health-real-worker");
    let mut cfg = debrief_store_client::WorkerConfig::new(built_store_worker());
    cfg.supervisor.max_crashes = 3;
    cfg.supervisor.backoff_base = std::time::Duration::from_millis(1);
    cfg.supervisor.backoff_max = std::time::Duration::from_millis(5);
    let worker = debrief_store_client::StoreWorker::new(cfg).expect("the worker is set up");
    let guard = StopOnDrop(worker.clone());
    app.state::<AppState>().performance.attach_worker(worker.clone());
    let _ = &tmp;

    // It runs, and the probe says so.
    worker.worker_status().expect("the worker starts and answers");
    let first_pid = worker.status().pid.expect("a running worker has a pid");
    assert!(process_is_alive(first_pid));
    probe(app.handle(), &health);
    assert_eq!(health.state(STORE), Some(ServiceState::Ok));

    // It is killed again and again until the supervisor stops restarting it. Each failing call reaches the engine's error
    // mapping, which reports the outage.
    for _ in 0..8 {
        if let Some(pid) = worker.status().pid {
            kill_process(pid);
        }
        if let Err(e) = worker.worker_status() {
            let _ = crate::engine::store_error(e);
        }
        if worker.status().state == ChildState::Quarantined {
            break;
        }
    }
    assert_eq!(worker.status().state, ChildState::Quarantined, "{:?}", worker.status());
    let err = worker.worker_status().expect_err("a quarantined worker refuses");
    let command_error = crate::engine::store_error(err);
    assert_eq!(command_error.cause, Some(crate::error::ServiceCause::Store));
    assert!(!process_is_alive(first_pid), "the killed worker is gone");

    // Service health: the store is down for one reason, the shell can repair it.
    probe(app.handle(), &health);
    let down = health.snapshot().services.into_iter().find(|s| s.service == STORE).unwrap();
    assert!(matches!(down.state, ServiceState::Down | ServiceState::Quarantined), "{down:?}");
    assert!(down.repair.is_some(), "{down:?}");
    let cause = down.cause.unwrap();
    assert!(cause.message.contains("log store"), "{cause:?}");

    // Repair, through the real repair (the shell's supervisor, not the store): it is running again.
    let id = health.begin_repair(STORE).expect("a repair is offered");
    assert_eq!(health.state(STORE), Some(ServiceState::Repairing));
    health.finish_repair(id);
    assert_eq!(health.state(STORE), Some(ServiceState::Ok), "{:?}", health.snapshot());
    assert_eq!(worker.status().state, ChildState::Running);
    let new_pid = worker.status().pid.expect("restarted");
    assert_ne!(new_pid, first_pid);
    assert!(process_is_alive(new_pid));
    worker.worker_status().expect("the worker answers again");

    // And no process is left behind.
    drop(guard);
    for _ in 0..100 {
        if !process_is_alive(new_pid) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("the store worker (pid {new_pid}) is still running after shutdown");
}

#[test]
fn a_browser_that_could_not_start_is_one_error_whose_button_opens_settings_web_browser() {
    let h = Health::new(Box::new(|_, _| {}));
    h.browser_lifecycle(hub::BrowserService::LaunchFailed);
    let snapshot = h.snapshot();
    let entry = snapshot.services.iter().find(|s| s.service == BROWSER).unwrap();
    assert_eq!((entry.state, entry.repair), (ServiceState::Down, Some(RepairAction::OpenSettings)));
    let change = Change {
        service: BROWSER.into(),
        from: ServiceState::Ok,
        to: ServiceState::Down,
    };
    let n = notice_for(&snapshot, &change).unwrap();
    assert_eq!(n.cause, "health:browser");
    assert_eq!(n.primary, Some(NoticeAction::OpenSettings { section: "browser".into() }));
    assert_eq!(hub::BROWSER_SETTINGS, crate::browser::SECTION);
    // The shell has nothing to run for it: the UI opens the section.
    assert!(h.begin_repair(BROWSER).is_err());
    assert_eq!(h.state(BROWSER), Some(ServiceState::Down));
    // No browser found at all: the failed start keeps the Install repair (downloading Chromium fixes it).
    h.browser_lifecycle(hub::BrowserService::Starting);
    h.browser_missing("no Chromium-based browser was found");
    h.browser_lifecycle(hub::BrowserService::LaunchFailed);
    let snapshot = h.snapshot();
    let entry = snapshot.services.iter().find(|s| s.service == BROWSER).unwrap();
    assert_eq!((entry.state, entry.repair), (ServiceState::Down, Some(RepairAction::Install)));
    // Starting again is not an outage; ready, closed or ended by the user is fine.
    h.browser_lifecycle(hub::BrowserService::Starting);
    assert_eq!(h.state(BROWSER), Some(ServiceState::Starting));
    h.browser_lifecycle(hub::BrowserService::Idle);
    assert_eq!(h.state(BROWSER), Some(ServiceState::Ok));
}
