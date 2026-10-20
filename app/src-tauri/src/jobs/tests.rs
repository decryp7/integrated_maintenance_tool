//! The jobs commands through the real IPC path (`tauri::test`), with the test app's manual clock: the
//! pane's list and events, the tray mirror, Cancel, Retry, View (results, revealing files), the history
//! saved with the project, window ownership, and messages for finished jobs.

use super::*;
use crate::test_support::{samples, test_app_with_commands, window, TempDir, TestClock};
use crate::tools::tests::ipc;
use serde_json::json;
use std::sync::mpsc;
use tauri::test::MockRuntime;
use tauri::Listener;

fn clock(app: &tauri::App<MockRuntime>) -> Arc<ManualClock> {
    app.state::<TestClock>().0.clone()
}

fn jobs(app: &tauri::App<MockRuntime>) -> Jobs {
    app.state::<Jobs>().inner().clone()
}

type Release = mpsc::Sender<Result<Option<JobResult>, String>>;

/// An export job of `window` that runs until the test releases it with a result.
fn gated_export(app: &tauri::App<MockRuntime>, window: &str, title: &str) -> (JobId, Release) {
    let (tx, rx) = mpsc::channel::<Result<Option<JobResult>, String>>();
    let rx = Mutex::new(rx);
    let spec = JobSpec::new(JobKind::Export, title)
        .window(window)
        .retry_with(json!({"op": "export"}));
    let id = spawn(
        app.handle(),
        spec,
        Controls::default(),
        move |ctx: &JobCtx| -> Result<Done<()>, String> {
            loop {
                if ctx.is_cancelled() {
                    return Err("cancelled".into());
                }
                match rx.lock().unwrap().recv_timeout(std::time::Duration::from_millis(5)) {
                    Ok(Ok(r)) => {
                        let d = Done::new(());
                        return Ok(match r {
                            Some(r) => d.with_result(r),
                            None => d,
                        });
                    }
                    Ok(Err(e)) => return Err(e),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => return Err("gone".into()),
                }
            }
        },
    )
    .unwrap();
    (id, tx)
}

fn listed(main: &tauri::WebviewWindow<MockRuntime>) -> Vec<Value> {
    ipc(main, "jobs_list", json!({})).unwrap()["jobs"].as_array().unwrap().clone()
}

#[test]
fn the_pane_lists_running_jobs_after_two_seconds_and_the_tray_can_stop_them() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let events = Arc::new(Mutex::new(Vec::<Value>::new()));
    let e = events.clone();
    main.listen(JOBS_EVENT, move |ev| {
        e.lock().unwrap().push(serde_json::from_str(ev.payload()).unwrap())
    });
    let (id, _release) = gated_export(&app, "main", "Export report r.pdf");
    wait_until("running", || jobs(&app).state_of(id) == Some(JobState::Running));
    assert!(listed(&main).is_empty(), "not before 2 s");
    clock(&app).advance(queue::VISIBLE_AFTER_MS);
    let list = listed(&main);
    assert_eq!(list[0]["title"], "Export report r.pdf");
    assert_eq!(
        (list[0]["state"].as_str(), list[0]["cancellable"].as_bool()),
        (Some("running"), Some(true))
    );
    // The ticker tells the window, and the tray lists the job with Stop.
    wait_until("an event with the job", || {
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e["jobs"].as_array().is_some_and(|j| !j.is_empty()))
    });
    let task = format!("{TASK_PREFIX}{id}");
    wait_until("the tray entry", || {
        ipc(&main, "background_tasks", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == task.as_str() && t["kind"] == "job")
    });
    ipc(&main, "stop_background_task", json!({"id": task})).unwrap();
    wait_until("cancelled", || jobs(&app).state_of(id) == Some(JobState::Cancelled));
    wait_until("the tray entry to go", || {
        ipc(&main, "background_tasks", json!({})).unwrap().as_array().unwrap().is_empty()
    });
    let list = listed(&main);
    assert_eq!(
        (list[0]["state"].as_str(), list[0]["retryable"].as_bool()),
        (Some("cancelled"), Some(true))
    );
    assert_eq!(ipc(&main, "jobs_clear_finished", json!({})).unwrap(), 1);
    assert!(listed(&main).is_empty());
}

#[test]
fn long_jobs_post_a_message_and_view_opens_their_result() {
    let tmp = TempDir::new("jobs-view");
    let file = tmp.0.join("report.html");
    std::fs::write(&file, "<html></html>").unwrap();
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let (id, release) = gated_export(&app, "main", "Export report report.html");
    wait_until("running", || jobs(&app).state_of(id) == Some(JobState::Running));
    clock(&app).advance(queue::ATTENTION_AFTER_MS);
    release
        .send(Ok(Some(JobResult::File {
            path: file.display().to_string(),
        })))
        .unwrap();
    wait_until("done", || jobs(&app).state_of(id) == Some(JobState::Done));
    let job = listed(&main)[0].clone();
    assert_eq!((job["unseen"].as_bool(), job["resultType"].as_str()), (Some(true), Some("file")));
    // The message is posted right after the job ends.
    wait_until("the message", || {
        ipc(&main, "notifications_list", json!({})).unwrap()["unread"] == 1
    });
    let notices = ipc(&main, "notifications_list", json!({})).unwrap();
    let n = &notices["notices"][0];
    assert_eq!((n["source"].as_str(), n["severity"].as_str()), (Some("job"), Some("success")));
    assert_eq!(n["primary"], json!({"type": "showInFolder", "jobId": id.0}));
    assert_eq!(n["title"], "Finished: Export report report.html");
    // View: the file is revealed (only the one the job wrote) and the job counts as seen.
    assert_eq!(ipc(&main, "jobs_reveal", json!({"id": id.0})).unwrap(), file.display().to_string());
    assert_eq!(listed(&main)[0]["unseen"], false);
    assert_eq!(ipc(&main, "jobs_result", json!({"id": id.0})).unwrap()["type"], "file");
    std::fs::remove_file(&file).unwrap();
    let gone = ipc(&main, "jobs_reveal", json!({"id": id.0})).unwrap_err();
    assert!(gone.as_str().unwrap().contains("no longer exists"));
    assert!(ipc(&main, "jobs_reveal", json!({"id": 999})).is_err());
}

#[test]
fn a_dialog_that_follows_a_job_gets_its_outcome_and_no_message_is_posted() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let (id, release) = gated_export(&app, "main", "Export followed.csv");
    clock(&app).advance(queue::ATTENTION_AFTER_MS);
    // The dialog waits for it over IPC (on its own thread, as the UI's promise).
    let w = main.clone();
    let waiter = std::thread::spawn(move || ipc(&w, "jobs_wait", json!({ "id": id.0 })).unwrap());
    wait_until("the dialog to wait", || jobs(&app).waited(id));
    release.send(Ok(Some(JobResult::File { path: "x.csv".into() }))).unwrap();
    let outcome = waiter.join().unwrap();
    assert_eq!(
        (outcome["state"].as_str(), outcome["result"]["type"].as_str()),
        (Some("done"), Some("file"))
    );
    // It showed the outcome: nothing unseen, no message.
    assert_eq!(listed(&main)[0]["unseen"], false);
    assert_eq!(ipc(&main, "notifications_list", json!({})).unwrap()["unread"], 0);

    // A dialog that closes stops following: the job then tells the user itself.
    let (id, release) = gated_export(&app, "main", "Export left.csv");
    let w = main.clone();
    let waiter = std::thread::spawn(move || ipc(&w, "jobs_wait", json!({ "id": id.0 })).unwrap());
    wait_until("the dialog to wait", || jobs(&app).waited(id));
    ipc(&main, "jobs_unwait", json!({ "id": id.0 })).unwrap();
    assert_eq!(waiter.join().unwrap()["state"], "running");
    clock(&app).advance(queue::ATTENTION_AFTER_MS);
    release.send(Ok(Some(JobResult::File { path: "y.csv".into() }))).unwrap();
    wait_until("the message", || {
        ipc(&main, "notifications_list", json!({})).unwrap()["unread"] == 1
    });
    assert!(ipc(&main, "jobs_unwait", json!({ "id": 999 })).is_err(), "unknown jobs are refused");
}

#[test]
fn failures_always_ask_for_attention_with_retry() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let (id, release) = gated_export(&app, "main", "Export events to x.csv");
    wait_until("running", || jobs(&app).state_of(id) == Some(JobState::Running));
    release.send(Err("the disk is full".into())).unwrap();
    wait_until("failed", || jobs(&app).state_of(id) == Some(JobState::Failed));
    // The message is posted right after the job ends.
    wait_until("the message", || {
        ipc(&main, "notifications_list", json!({})).unwrap()["unread"] == 1
    });
    let n = ipc(&main, "notifications_list", json!({})).unwrap()["notices"][0].clone();
    assert_eq!(
        (n["severity"].as_str(), n["body"].as_str()),
        (Some("error"), Some("the disk is full"))
    );
    assert_eq!(n["primary"], json!({"type": "retryJob", "jobId": id.0}));
    assert_eq!(n["secondary"], json!({"type": "showJobs"}));
    // Exports have no retry handler: a clear message.
    let e = ipc(&main, "jobs_retry", json!({"id": id.0})).unwrap_err();
    assert!(e.as_str().unwrap().contains("cannot be retried"), "{e}");
    assert!(ipc(&main, "jobs_move", json!({"id": id.0, "up": true})).is_err());
    assert_eq!(ipc(&main, "jobs_mark_seen", json!({"id": id.0})).unwrap(), true);
}

#[test]
fn a_window_acts_only_on_its_own_and_app_wide_jobs() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let other = window(&app, "w-2");
    crate::windows::CurrentWindow::of(app.handle(), "w-2").unwrap();
    let (id, _release) = gated_export(&app, "w-2", "theirs");
    wait_until("running", || jobs(&app).state_of(id) == Some(JobState::Running));
    clock(&app).advance(queue::VISIBLE_AFTER_MS);
    assert!(listed(&main).is_empty());
    assert_eq!(listed(&other).len(), 1);
    for cmd in ["jobs_cancel", "jobs_retry", "jobs_mark_seen", "jobs_result", "jobs_reveal"] {
        let e = ipc(&main, cmd, json!({"id": id.0})).unwrap_err();
        assert!(e.as_str().unwrap().contains("another window"), "{cmd}: {e}");
    }
    let e = ipc(&main, "jobs_move", json!({"id": id.0, "up": false})).unwrap_err();
    assert!(e.as_str().unwrap().contains("another window"));
    // Closing the window stops its jobs and drops its history.
    forget_window(app.handle(), "w-2");
    wait_until("stopped", || jobs(&app).find_active(|_| true).is_empty());
}

#[test]
fn the_history_round_trips_through_the_project_and_unfinished_jobs_come_back_interrupted() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let (id, release) = gated_export(&app, "main", "kept");
    wait_until("running", || jobs(&app).state_of(id) == Some(JobState::Running));
    clock(&app).advance(queue::VISIBLE_AFTER_MS);
    release.send(Ok(None)).unwrap();
    wait_until("done", || jobs(&app).state_of(id) == Some(JobState::Done));
    let mut record = ipc(&main, "jobs_record", json!({})).unwrap();
    assert_eq!((record["version"].as_u64(), record["jobs"].as_array().unwrap().len()), (Some(1), 1));
    // A session saved while an AI request ran (IMT crashed): it comes back interrupted, with Retry.
    record["jobs"].as_array_mut().unwrap().push(json!({
        "id": 77, "kind": "ai", "title": "Summarize a time range", "source": {"window": "main"}, "state": "running",
        "queuedAt": 1, "startedAt": 2, "cancellable": true, "request": {"op": "summarize", "from": 0, "to": 1}
    }));
    let restored = ipc(&main, "jobs_restore", json!({"record": record})).unwrap();
    assert_eq!(restored["warning"], Value::Null);
    let all = restored["snapshot"]["jobs"].as_array().unwrap().clone();
    let interrupted = all.iter().find(|j| j["state"] == "interrupted").expect("interrupted").clone();
    assert!(interrupted["retryable"].as_bool().unwrap() && interrupted["unseen"].as_bool().unwrap());
    // A damaged record restores an empty history, with the reason.
    let bad = ipc(&main, "jobs_restore", json!({"record": {"version": 1, "jobs": "nope"}})).unwrap();
    assert!(bad["warning"].as_str().unwrap().contains("damaged"));
    assert!(bad["snapshot"]["jobs"].as_array().unwrap().is_empty());
    assert_eq!(ipc(&main, "jobs_restore", json!({})).unwrap()["warning"], Value::Null);
}

fn finished_jobs(app: &tauri::App<MockRuntime>) -> Arc<Mutex<Vec<model::Job>>> {
    let ended = Arc::new(Mutex::new(Vec::new()));
    let e = ended.clone();
    jobs(app).subscribe(move |ev| {
        if let JobsEvent::Finished(f) = ev {
            e.lock().unwrap().push(f.job.clone());
        }
    });
    ended
}

#[test]
fn imports_are_import_jobs_with_retry_through_the_window() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let ended = finished_jobs(&app);
    let dir = samples().join("serilog-clef");
    ipc(&main, "open_paths", json!({ "paths": [dir.to_string_lossy()] })).unwrap();
    wait_until("the import job", || !ended.lock().unwrap().is_empty());
    let job = ended.lock().unwrap()[0].clone();
    assert_eq!(
        (job.kind, job.state(), job.result.clone()),
        (JobKind::Import, JobState::Done, Some(JobResult::Workspace))
    );
    assert_eq!(job.title, "Open serilog-clef");
    // Retry hands the paths back to the window (as after a launch).
    assert_eq!(crate::workspace_commands::retry_import(app.handle(), &job).unwrap(), None);
    let pending = ipc(&main, "take_pending_open", json!({})).unwrap();
    assert_eq!(pending.as_array().unwrap().len(), 1);
    let mut gone = job.clone();
    gone.request = Some(json!({"op": "open", "paths": ["/no/such/file.log"]}));
    assert!(crate::workspace_commands::retry_import(app.handle(), &gone)
        .unwrap_err()
        .contains("exist"));
    gone.request = None;
    assert!(crate::workspace_commands::retry_import(app.handle(), &gone).is_err());
}

#[test]
fn analyses_and_queries_are_analysis_jobs() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    ipc(
        &main,
        "open_paths",
        json!({ "paths": [samples().join("serilog-clef").to_string_lossy()] }),
    )
    .unwrap();
    let ended = finished_jobs(&app);
    let r = ipc(&main, "query_anomalies", json!({"request": {"query": {"ql": "level >= warning"}}})).unwrap();
    assert!(r.is_object() || r.is_array());
    wait_until("the analysis job", || !ended.lock().unwrap().is_empty());
    let job = ended.lock().unwrap()[0].clone();
    assert_eq!(job.kind, JobKind::Analysis);
    assert_eq!(
        job.source.link,
        Some(JobLink::Query {
            ql: "level >= warning".into()
        })
    );
    assert_eq!(
        job.result,
        Some(JobResult::View {
            view: "anomalies".into(),
            ql: Some("level >= warning".into())
        })
    );
}

#[test]
fn without_the_service_work_runs_directly_and_errors_convert() {
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let v: Result<u8, String> =
        tauri::async_runtime::block_on(run(app.handle(), JobSpec::new(JobKind::Export, "x"), Controls::default(), |_| {
            Ok(Done::new(4))
        }));
    assert_eq!(v, Ok(4));
    let none = spawn(app.handle(), JobSpec::new(JobKind::Export, "x"), Controls::default(), |_| {
        Ok::<_, String>(Done::new(()))
    });
    assert!(none.is_err());
    forget_window(app.handle(), "main");
    shutdown(app.handle());
    assert_eq!(job_title(JobKind::Ai, JobState::Interrupted, "t"), "Interrupted: t");
}

/// Listing jobs as background tasks notifies the tray and the windows; that must never happen while the
/// mirror's own lock is held (the tray's getters wait for the main thread, which may want the lock).
#[test]
fn mirroring_never_notifies_under_its_lock() {
    let app = test_app_with_commands(None);
    let _main = window(&app, "main");
    let (id, release) = gated_export(&app, "main", "Export a");
    wait_until("running", || jobs(&app).state_of(id) == Some(JobState::Running));
    clock(&app).advance(queue::VISIBLE_AFTER_MS);
    let mirrored = Arc::new(Mutex::new(std::collections::HashSet::<String>::new()));
    let (probe, notified, under_lock) = (
        mirrored.clone(),
        Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    let (n, held) = (notified.clone(), under_lock.clone());
    app.state::<BackgroundTasks>().subscribe(move |_| {
        n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if probe.try_lock().is_err() {
            held.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    });
    super::mirror(app.handle(), &jobs(&app), &mirrored);
    assert!(notified.load(std::sync::atomic::Ordering::SeqCst) > 0, "the job was listed");
    assert!(
        !under_lock.load(std::sync::atomic::Ordering::SeqCst),
        "listeners ran under the mirror's lock"
    );
    assert_eq!(mirrored.lock().unwrap().len(), 1);
    release.send(Ok(None)).unwrap();
}
