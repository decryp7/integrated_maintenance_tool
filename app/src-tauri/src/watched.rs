//! Watched folders in the app (ADR 0048): a window can watch folders; files that settle in them are imported into
//! the window's open project (as ordinary import jobs, so the Jobs pane shows them) and a known issue found in
//! the new data is announced once in the notification centre (and on the desktop). The decisions are
//! `debrief_core::watch::WatchState`'s, unchanged; this module owns the folder list, the polling thread and the
//! commands. The list is kept in the window and saved with the project by the UI.

use crate::background::{BackgroundTasks, TaskKind, TaskSpec};
use crate::engine::with_store;
use crate::jobs::{JobKind, JobSpec};
use crate::notification_centre::centre::{NewNotice, NoticeScope, NoticeSeverity, NoticeSource};
use crate::notify::NotificationKind;
use crate::windows::{emit_to_window, CurrentWindow};
use crate::{CmdError, CmdResult};
use debrief_core::known_issues::{match_known_issues, KnowledgeText};
use debrief_core::watch::{scan_folder, FileStamp, Skipped, WatchConfig, WatchState};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Runtime};

/// Event telling a window that watched folders imported something (the UI refreshes its summary).
pub(crate) const IMPORTED_EVENT: &str = "debrief://watched-imported";
/// Folders one window may watch.
const MAX_FOLDERS: usize = 8;
/// Time between scans.
const POLL: Duration = Duration::from_secs(5);

/// A watched folder as the UI sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WatchedFolderInfo {
    pub path: String,
    pub enabled: bool,
    /// Files imported since the folder was added.
    pub imported_files: usize,
    /// Files waiting for their quiet time or a retry.
    pub pending: usize,
    /// The last thing worth saying (a file given up on, a file too large), or empty.
    pub message: String,
}

#[derive(Debug, Clone)]
struct Folder {
    path: PathBuf,
    enabled: bool,
    imported_files: usize,
    pending: usize,
    message: String,
}

/// A window's watched folders and the polling thread that serves them.
#[derive(Default)]
pub(crate) struct WatchedSlot {
    folders: Mutex<Vec<Folder>>,
    /// The running poller's stop flag.
    running: Mutex<Option<Arc<AtomicBool>>>,
}

impl WatchedSlot {
    fn folders(&self) -> std::sync::MutexGuard<'_, Vec<Folder>> {
        crate::sync::lock(&self.folders)
    }

    pub(crate) fn infos(&self) -> Vec<WatchedFolderInfo> {
        self.folders()
            .iter()
            .map(|f| WatchedFolderInfo {
                path: f.path.display().to_string(),
                enabled: f.enabled,
                imported_files: f.imported_files,
                pending: f.pending,
                message: f.message.clone(),
            })
            .collect()
    }

    fn any_enabled(&self) -> bool {
        self.folders().iter().any(|f| f.enabled)
    }

    /// Adds a folder (a repeat is ignored); refuses more than [`MAX_FOLDERS`].
    pub(crate) fn add(&self, path: PathBuf) -> CmdResult<()> {
        let mut folders = self.folders();
        if folders.iter().any(|f| f.path == path) {
            return Ok(());
        }
        if folders.len() >= MAX_FOLDERS {
            return Err(CmdError::invalid(format!(
                "at most {MAX_FOLDERS} folders can be watched in a window; remove one first"
            )));
        }
        folders.push(Folder {
            path,
            enabled: true,
            imported_files: 0,
            pending: 0,
            message: String::new(),
        });
        Ok(())
    }

    pub(crate) fn remove(&self, path: &Path) {
        self.folders().retain(|f| f.path != path);
    }

    pub(crate) fn set_enabled(&self, path: &Path, enabled: bool) {
        if let Some(f) = self.folders().iter_mut().find(|f| f.path == path) {
            f.enabled = enabled;
        }
    }

    /// Stops the poller (if any).
    pub(crate) fn stop(&self) {
        if let Some(flag) = crate::sync::lock(&self.running).take() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    fn note(&self, path: &Path, edit: impl FnOnce(&mut Folder)) {
        if let Some(f) = self.folders().iter_mut().find(|f| f.path == path) {
            edit(f);
        }
    }
}

fn task_id(label: &str) -> String {
    format!("watch:{label}")
}

/// Starts the poller when a folder is enabled and none runs; stops it when none is.
fn sync_poller<R: Runtime>(app: &AppHandle<R>, win: &CurrentWindow) {
    let slot = &win.watched;
    let label = win.label().to_string();
    if !slot.any_enabled() {
        slot.stop();
        if let Some(tasks) = tauri::Manager::try_state::<BackgroundTasks>(app) {
            tasks.remove(&task_id(&label));
        }
        return;
    }
    let mut running = crate::sync::lock(&slot.running);
    if running.as_ref().is_some_and(|f| !f.load(Ordering::Relaxed)) {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    *running = Some(stop.clone());
    drop(running);
    if let Some(tasks) = tauri::Manager::try_state::<BackgroundTasks>(app) {
        let flag = stop.clone();
        let _ = tasks.register(
            TaskSpec {
                id: task_id(&label),
                label: "Watching folders for new files".into(),
                kind: TaskKind::Other,
                window: Some(label.clone()),
            },
            Some(Arc::new(move || {
                flag.store(true, Ordering::Relaxed);
                Ok(())
            })),
        );
    }
    request_notification_permission(app);
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("debrief-watch-folders".into())
        .spawn(move || poll(app, label, stop));
    if let Err(e) = spawned {
        log::warn!("could not start the folder watcher: {e}");
    }
}

/// Asks the OS once for permission to show notifications, so a known issue found while the window is in the
/// background can reach the user (a refusal only means the notification centre is used).
#[cfg(not(test))]
fn request_notification_permission<R: Runtime>(app: &AppHandle<R>) {
    use tauri_plugin_notification::{NotificationExt, PermissionState};
    let n = app.notification();
    if matches!(
        n.permission_state(),
        Ok(PermissionState::Prompt) | Ok(PermissionState::PromptWithRationale)
    ) {
        if let Err(e) = n.request_permission() {
            log::warn!("notification permission was not requested: {e}");
        }
    }
}

#[cfg(test)]
fn request_notification_permission<R: Runtime>(_app: &AppHandle<R>) {}

/// The poller: every [`POLL`], scan each enabled folder, import what settled, announce new known issues.
fn poll<R: Runtime>(app: AppHandle<R>, label: String, stop: Arc<AtomicBool>) {
    let started = Instant::now();
    let mut states: HashMap<PathBuf, WatchState> = HashMap::new();
    while !stop.load(Ordering::Relaxed) {
        let Ok(win) = CurrentWindow::of(&app, &label) else { break };
        let now = started.elapsed().as_millis() as u64;
        let enabled: Vec<PathBuf> = win.watched.folders().iter().filter(|f| f.enabled).map(|f| f.path.clone()).collect();
        for dir in &enabled {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let state = states.entry(dir.clone()).or_insert_with(|| WatchState::new(WatchConfig::default()));
            step(&app, &win, dir, state, now);
        }
        let until = Instant::now() + POLL;
        while Instant::now() < until && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    if let Some(tasks) = tauri::Manager::try_state::<BackgroundTasks>(&app) {
        tasks.remove(&task_id(&label));
    }
}

/// One scan of one folder.
fn step<R: Runtime>(app: &AppHandle<R>, win: &CurrentWindow, dir: &Path, state: &mut WatchState, now: u64) {
    let compiled = win.host.compiled();
    let (scan, _cut) = scan_folder(dir, &|rel: &Path| compiled.include.is_match(rel) && !compiled.exclude.is_match(rel));
    let ready = state.observe(&scan, now);
    win.watched.note(dir, |f| f.pending = state.pending_len());
    for (path, why) in &ready.skipped {
        let text = match why {
            Skipped::TooLarge { size } => format!("{} is {} bytes, too large to import automatically", file_name(path), size),
            Skipped::GaveUp { reason } => format!("gave up on {}: {reason}", file_name(path)),
        };
        win.watched.note(dir, |f| f.message = text);
    }
    if !state.flush_due(&ready, now) {
        // More files may still be settling: they go together in one job.
        let waiting = ready.new.len() + ready.changed.len();
        if waiting > 0 {
            win.watched.note(dir, |f| {
                f.message = format!("{waiting} file{} ready, waiting for more", if waiting == 1 { "" } else { "s" })
            });
        }
        return;
    }
    let stamps: HashMap<&Path, &FileStamp> = scan.iter().map(|f| (f.path.as_path(), f)).collect();
    let mut imported = 0usize;
    // New files open together and changed files refresh together: one import job (one position rebuild) each.
    for (files, refresh) in [(&ready.new, false), (&ready.changed, true)] {
        let files: Vec<PathBuf> = files.iter().filter(|p| stamps.contains_key(p.as_path())).cloned().collect();
        if files.is_empty() {
            continue;
        }
        match import_batch(app, win, dir, &files, refresh) {
            Ok(failed) => {
                for path in &files {
                    match failed.iter().find(|(p, _)| p == path || p.file_name() == path.file_name()) {
                        None => {
                            state.imported(stamps[path.as_path()]);
                            imported += 1;
                        }
                        Some((_, reason)) => give_up_or_retry(win, dir, state, path, reason, now),
                    }
                }
            }
            // Another import is running in the window: the files stay ready and are tried on the next scan.
            Err(e) if e.to_string().to_ascii_lowercase().contains("busy") => {}
            Err(e) => {
                for path in &files {
                    give_up_or_retry(win, dir, state, path, &e.to_string(), now);
                }
            }
        }
    }
    if imported > 0 {
        win.watched.note(dir, |f| f.imported_files += imported);
        emit_to_window(app, win.label(), IMPORTED_EVENT, imported);
        announce_known_issues(app, win, dir, state);
    }
}

fn give_up_or_retry(win: &CurrentWindow, dir: &Path, state: &mut WatchState, path: &Path, reason: &str, now: u64) {
    if let Some(Skipped::GaveUp { reason }) = state.failed(path, reason, now) {
        win.watched
            .note(dir, |f| f.message = format!("gave up on {}: {reason}", file_name(path)));
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// The batches imported so far, as (folder, files), for tests.
#[cfg(test)]
static BATCHES: Mutex<Vec<(PathBuf, usize)>> = Mutex::new(Vec::new());

/// Imports `files` of the watched folder `dir` as ONE import job of the window (opening them, or refreshing them
/// when they changed). The job shows the files' progress and has Cancel like any import. Returns the files that were
/// not recognised with the reason (the rest were imported).
fn import_batch<R: Runtime>(
    app: &AppHandle<R>,
    win: &CurrentWindow,
    dir: &Path,
    files: &[PathBuf],
    refresh: bool,
) -> CmdResult<Vec<(PathBuf, String)>> {
    #[cfg(test)]
    BATCHES
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push((dir.to_path_buf(), files.len()));
    let before = crate::engine_errors::HealthSnapshot::new(&win.host.registry.plugins(), &[]);
    let paths = files.to_vec();
    let spec = JobSpec::new(JobKind::Import, batch_title(dir, files)).window(win.label());
    let report = tauri::async_runtime::block_on(crate::engine::import::run(
        app,
        win,
        spec,
        Box::new(move |s| if refresh { s.refresh(&paths) } else { s.open(&paths) }),
    ))?;
    crate::workspace_commands::after_import(app, win, &before, &report);
    Ok(report
        .unrecognized
        .iter()
        .map(|u| (PathBuf::from(&u.path), u.reason.clone()))
        .collect())
}

/// The job's title: the file for one, "N files from <folder>" for a batch.
pub(crate) fn batch_title(dir: &Path, files: &[PathBuf]) -> String {
    match files {
        [one] => format!("Import {} (watched folder)", file_name(one)),
        many => format!("Import {} files from {} (watched folder)", many.len(), file_name(dir)),
    }
}

/// Tells the user about known issues in the data that were not announced before (or grew).
fn announce_known_issues<R: Runtime>(app: &AppHandle<R>, win: &CurrentWindow, dir: &Path, state: &mut WatchState) {
    let texts = crate::investigate::knowledge_texts(app, win.arc());
    if texts.is_empty() {
        return;
    }
    let report = tauri::async_runtime::block_on(with_store(win, move |s| {
        let files: Vec<KnowledgeText<'_>> = texts.iter().map(|(name, text)| KnowledgeText { name, text }).collect();
        match_known_issues(s, &files)
    }));
    let Ok(report) = report else { return };
    for m in state.fresh_matches(&report) {
        let notice = NewNotice::new(
            format!("watched-issue:{}", m.issue.id),
            NoticeSource::Workspace,
            NoticeSeverity::Warning,
            format!("Known issue found: {}", m.issue.title),
            NoticeScope::Project,
        )
        .body(format!("{} events in files from {}. {}", m.count, file_name(dir), m.issue.fix));
        crate::notification_centre::post(app, Some(win.label()), notice, Some(NotificationKind::Attention));
    }
}

/// The calling window's watched folders.
#[tauri::command]
pub(crate) async fn watched_folders(state: CurrentWindow) -> CmdResult<Vec<WatchedFolderInfo>> {
    Ok(state.watched.infos())
}

/// Starts watching a folder the user chose.
#[tauri::command]
pub(crate) async fn add_watched_folder<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    path: String,
) -> CmdResult<Vec<WatchedFolderInfo>> {
    let granted = crate::workspace_commands::open_request(&app, state.label(), vec![path])?;
    let Some(dir) = granted.into_iter().next() else {
        return Err(CmdError::invalid("no folder given"));
    };
    if !dir.is_dir() {
        return Err(CmdError::invalid(format!("{} is not a folder", dir.display())));
    }
    state.watched.add(dir)?;
    sync_poller(&app, &state);
    Ok(state.watched.infos())
}

/// Stops watching a folder (what it imported stays).
#[tauri::command]
pub(crate) async fn remove_watched_folder<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    path: String,
) -> CmdResult<Vec<WatchedFolderInfo>> {
    state.watched.remove(Path::new(&path));
    sync_poller(&app, &state);
    Ok(state.watched.infos())
}

/// Pauses or resumes watching a folder.
#[tauri::command]
pub(crate) async fn set_watched_folder_enabled<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    path: String,
    enabled: bool,
) -> CmdResult<Vec<WatchedFolderInfo>> {
    state.watched.set_enabled(Path::new(&path), enabled);
    sync_poller(&app, &state);
    Ok(state.watched.infos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_list_is_bounded_and_deduplicated() {
        let slot = WatchedSlot::default();
        slot.add(PathBuf::from("/a")).unwrap();
        slot.add(PathBuf::from("/a")).unwrap();
        assert_eq!(slot.infos().len(), 1);
        for i in 0..MAX_FOLDERS {
            let _ = slot.add(PathBuf::from(format!("/f{i}")));
        }
        assert_eq!(slot.infos().len(), MAX_FOLDERS);
        assert!(slot.add(PathBuf::from("/one-too-many")).is_err());
        slot.set_enabled(Path::new("/a"), false);
        assert!(!slot.infos()[0].enabled);
        slot.remove(Path::new("/a"));
        assert!(slot.infos().iter().all(|f| f.path != "/a"));
        slot.stop();
    }

    #[test]
    fn any_enabled_follows_the_switches() {
        let slot = WatchedSlot::default();
        assert!(!slot.any_enabled());
        slot.add(PathBuf::from("/a")).unwrap();
        assert!(slot.any_enabled());
        slot.set_enabled(Path::new("/a"), false);
        assert!(!slot.any_enabled());
    }

    #[test]
    fn the_commands_watch_a_folder_import_what_settles_and_apply_an_import_window() {
        use crate::test_support::{test_app_with_commands, window, TempDir};
        use crate::tools::tests::ipc;
        use serde_json::json;
        let app = test_app_with_commands(None);
        let main = window(&app, crate::windows::MAIN);
        let dir = TempDir::new("watched");
        crate::paths::allow_under(app.handle(), crate::windows::MAIN, &dir.0);
        let log = |h: u32| format!("2026-10-14 {h:02}:00:00.000 +00:00 [INF] tick {h}\n");
        std::fs::write(dir.0.join("a.log"), format!("{}{}", log(1), log(2))).unwrap();

        // A backwards import window is refused with a message; a good one is accepted and cleared again.
        let e = ipc(&main, "set_import_window", json!({ "from": 5, "to": 1 })).unwrap_err();
        assert!(e["message"].as_str().unwrap().contains("before"), "{e}");
        ipc(&main, "set_import_window", json!({ "from": null, "to": null })).unwrap();

        // Adding the folder lists it; a file path is refused; an unknown path is refused.
        let listed = ipc(&main, "add_watched_folder", json!({ "path": dir.0.to_string_lossy() })).unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert_eq!(listed[0]["enabled"], true);
        let e = ipc(
            &main,
            "add_watched_folder",
            json!({ "path": dir.0.join("a.log").to_string_lossy() }),
        )
        .unwrap_err();
        assert!(e["message"].as_str().unwrap().contains("not a folder"), "{e}");
        assert!(ipc(&main, "add_watched_folder", json!({ "path": "/definitely/not/granted" })).is_err());

        // The poller imports the settled file into the window's store.
        let started = Instant::now();
        let imported = loop {
            let now = ipc(&main, "watched_folders", json!({})).unwrap();
            if now[0]["importedFiles"].as_u64().unwrap_or(0) > 0 || started.elapsed() > Duration::from_secs(40) {
                break now[0]["importedFiles"].as_u64().unwrap_or(0);
            }
            std::thread::sleep(Duration::from_millis(300));
        };
        assert_eq!(imported, 1, "the file settled and was imported");
        let summary = ipc(&main, "workspace_summary", json!({})).unwrap();
        assert_eq!(summary["total"], 2);

        // Pausing and removing stop the watching (by the path the listing gave: the engine lists canonical paths).
        let listed_path = listed[0]["path"].as_str().unwrap().to_string();
        let paused = ipc(
            &main,
            "set_watched_folder_enabled",
            json!({ "path": listed_path, "enabled": false }),
        )
        .unwrap();
        assert_eq!(paused[0]["enabled"], false);
        let gone = ipc(&main, "remove_watched_folder", json!({ "path": listed_path })).unwrap();
        assert!(gone.as_array().unwrap().is_empty());
    }

    /// Files that settle together are imported by ONE job (one position rebuild), not one job per file.
    #[test]
    fn files_that_settle_together_are_imported_by_one_job() {
        use crate::test_support::{test_app_with_commands, window, TempDir};
        use crate::tools::tests::ipc;
        use serde_json::json;
        let app = test_app_with_commands(None);
        let main = window(&app, crate::windows::MAIN);
        let dir = TempDir::new("watched-batch");
        crate::paths::allow_under(app.handle(), crate::windows::MAIN, &dir.0);
        for n in 0..6 {
            std::fs::write(
                dir.0.join(format!("f{n}.log")),
                format!("2026-10-14 0{n}:00:00.000 +00:00 [INF] tick {n}\n"),
            )
            .unwrap();
        }
        ipc(&main, "add_watched_folder", json!({ "path": dir.0.to_string_lossy() })).unwrap();
        let started = Instant::now();
        loop {
            let now = ipc(&main, "watched_folders", json!({})).unwrap();
            if now[0]["importedFiles"].as_u64().unwrap_or(0) >= 6 || started.elapsed() > Duration::from_secs(60) {
                assert_eq!(now[0]["importedFiles"], 6);
                break;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        let mine: Vec<usize> = BATCHES
            .lock()
            .unwrap()
            .iter()
            .filter(|(d, _)| d.file_name() == dir.0.file_name())
            .map(|(_, n)| *n)
            .collect();
        assert_eq!(mine, [6], "one import job for the six files");
        assert_eq!(ipc(&main, "workspace_summary", json!({})).unwrap()["total"], 6);
    }

    #[test]
    fn a_batch_job_is_named_after_its_files() {
        let one = [PathBuf::from("/w/a.log")];
        assert_eq!(batch_title(Path::new("/w"), &one), "Import a.log (watched folder)");
        let two = [PathBuf::from("/w/a.log"), PathBuf::from("/w/b.log")];
        assert_eq!(batch_title(Path::new("/w"), &two), "Import 2 files from w (watched folder)");
    }
}
