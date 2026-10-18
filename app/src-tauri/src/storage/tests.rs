use super::*;
use crate::storage::walk::tests::write;
use crate::test_support::{test_app_with_commands, window, TempDir};
use crate::tools::tests::ipc;
use serde_json::json;

fn log_folders(d: &std::path::Path) -> Folders<'static> {
    Folders {
        bundles: Some(d.join("bundles")),
        webview: None,
        data: None,
        log: Some(d.join("logs")),
        active_log: Some(d.join("logs").join("imt.2026-10-18.log")),
        runtime_tag: "b200".into(),
        session_start: None,
        main_instance: true,
        previous_crash: false,
        browser_open: false,
        ai: None,
    }
}

#[test]
fn usage_sums_the_categories_and_reports_a_cap() {
    let d = TempDir::new("storage-usage");
    write(&d.0.join("logs").join("imt.2026-10-17.log"), 40);
    write(&d.0.join("logs").join("snapshots").join("a.png"), 2);
    let cats = plan::categories(&log_folders(&d.0));
    let u = usage_of(&cats, &mut Budget::standard());
    assert_eq!(u.total_bytes, 42);
    assert!(!u.capped);
    let logs = u.categories.iter().find(|c| c.id == CategoryId::Logs).unwrap();
    assert_eq!((logs.bytes, logs.clearable_bytes, logs.exists), (40, 40, true));
    assert!(usage_of(&cats, &mut Budget::new(1, Duration::from_secs(60))).capped);
    let json = serde_json::to_value(&u).unwrap();
    assert_eq!(json["categories"][0]["id"], "bundle-extracts");
    assert_eq!(json["categories"][0]["kind"], "cache");
    assert!(json["categories"][0].get("clearableBytes").is_some());
}

#[test]
fn clearing_several_categories_reports_each_with_progress_and_stops_when_cancelled() {
    let d = TempDir::new("storage-clear-all");
    write(&d.0.join("logs").join("imt.2026-10-16.log"), 10);
    write(&d.0.join("logs").join("imt.2026-10-17.log"), 20);
    write(&d.0.join("logs").join("snapshots").join("a.png"), 5);
    let cats = plan::categories(&log_folders(&d.0));
    let ids = [CategoryId::Logs, CategoryId::ErrorSnapshots, CategoryId::Stores];
    let mut seen = Vec::new();
    let r = clear_all(&cats, &ids, None, &AtomicBool::new(false), &mut |f| seen.push(f));
    assert_eq!(r.freed_bytes, 35);
    assert!(!r.cancelled);
    assert_eq!(r.categories.iter().map(|c| c.id).collect::<Vec<_>>(), ids);
    assert_eq!(
        r.categories[2],
        CategoryClearResult::new(CategoryId::Stores),
        "absent categories have nothing to clear"
    );
    assert_eq!(seen.last().copied(), Some(1.0));
    assert!(seen.windows(2).all(|w| w[0] <= w[1]));

    write(&d.0.join("logs").join("imt.2026-10-15.log"), 10);
    let cats = plan::categories(&log_folders(&d.0));
    let r = clear_all(&cats, &ids, None, &AtomicBool::new(true), &mut |_| {});
    assert!(r.cancelled);
    assert_eq!(r.freed_bytes, 0);
    assert!(d.0.join("logs").join("imt.2026-10-15.log").is_file());
}

#[test]
fn one_clear_runs_at_a_time_as_a_stoppable_background_task() {
    let app = test_app_with_commands(None);
    let job = Job::begin(app.handle()).unwrap();
    assert!(Job::begin(app.handle()).err().is_some_and(|e| e.contains("already clearing")));
    let tasks = app.state::<BackgroundTasks>();
    let task = tasks.list().into_iter().find(|t| t.id == TASK_ID).unwrap();
    assert!(task.stoppable);
    job.progress(0.5);
    assert_eq!(tasks.list().into_iter().find(|t| t.id == TASK_ID).unwrap().progress, Some(0.5));
    tasks.stop(TASK_ID).unwrap();
    assert!(job.cancel.load(Ordering::Relaxed), "stopping the task cancels the clear");
    drop(job);
    assert!(tasks.list().iter().all(|t| t.id != TASK_ID));
    // The slot is free again.
    drop(Job::begin(app.handle()).unwrap());
}

#[test]
fn commands_validate_what_the_ui_sends() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let err = |v: Result<serde_json::Value, serde_json::Value>| v.unwrap_err().as_str().unwrap_or_default().to_string();
    assert!(err(ipc(&main, "storage_clear", json!({ "ids": [] }))).contains("choose what to clear"));
    assert!(err(ipc(&main, "storage_clear", json!({ "ids": ["../../etc"] }))).contains("unknown storage category"));
    assert!(ipc(&main, "storage_clear", json!({ "ids": "logs" })).is_err());
    assert!(err(ipc(&main, "storage_reveal", json!({ "id": "C:\\Windows" }))).contains("unknown storage category"));
    // Stores are never cleared from here (and are absent unless their folder exists).
    let r = ipc(&main, "storage_clear", json!({ "ids": ["stores"] })).unwrap();
    assert_eq!(r["freedBytes"], 0);
    assert_eq!(r["categories"][0]["id"], "stores");
}

#[test]
fn usage_lists_every_category_through_ipc() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let u = ipc(&main, "storage_usage", json!({})).unwrap();
    let ids: Vec<_> = u["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    for id in &CategoryId::ALL[..8] {
        assert!(ids.contains(&id.as_str().to_string()), "{id:?} in {ids:?}");
    }
    // No AI engine is managed in this test app: its categories cannot be cleared.
    let models = u["categories"].as_array().unwrap().iter().find(|c| c["id"] == "ai-models").unwrap();
    assert_eq!(models["clearable"], false);
    // Without the single-instance flag the sessions belong to another instance.
    let sessions = u["categories"].as_array().unwrap().iter().find(|c| c["id"] == "sessions").unwrap();
    assert_eq!(sessions["clearable"], false);
}
