//! Live tail in the shell: the `notify` adapter for the engine's watcher port
//! ([`WatcherFactory`]), each window's running session, and the live tail
//! commands. The watcher coalesces changes; the store reads what was appended
//! (`Ingest::append_tail`: new segments, the full-text index follows); this
//! module wires it to the OS, the owning window and the background task registry.
//! Alerts and the per-file tail status come from the window's [`LiveSession`] over the store's ports; an event held back
//! at the end of a file (plain text events may continue on the next lines) is written by a follow-up refresh once the
//! file has been idle for `TailOptions::carry_flush_ms`.

use crate::background::{BackgroundTasks, TaskKind, TaskSpec};
use crate::notify::{notify, NotificationKind};
use crate::windows::{emit_to_window, CurrentWindow, WindowState, Windows};
use crate::{CmdError, CmdResult};
use debrief_core::live::LiveSession;
use debrief_core::workspace::{
    AlertTarget, FileWatcher, LiveTail, StopToken, TailOptions, TailReport, TailStatus, WatchMode, WatchPreference, WatchSettings,
    WatchSignal, WatchSink, WatcherFactory,
};
use notify_debouncer_mini::notify;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};

/// Event telling a window that live tail changed its workspace.
pub(crate) const CHANGED_EVENT: &str = "debrief://workspace-changed";
/// Event carrying one [`debrief_core::workspace::LiveAlert`].
pub(crate) const ALERT_EVENT: &str = "debrief://live-alert";
/// Event telling a window that its live tail was stopped from outside (the tray).
pub(crate) const STOPPED_EVENT: &str = "debrief://live-stopped";
/// Polling interval when none is given.
const DEFAULT_POLL_MS: u64 = 2_000;
/// Change events reach the UI at most about four times per second.
const MIN_EVENT_INTERVAL_MS: u64 = 250;

/// Polling interval accepted (ms); others are clamped into it.
const POLL_RANGE: std::ops::RangeInclusive<u64> = 100..=600_000;

/// Options of `set_live` (all optional; camelCase on the wire).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct LiveOptions {
    /// auto (events, polling for network paths and after errors) | events | polling.
    pub watch: WatchPreference,
    pub poll_interval_ms: Option<u64>,
    #[serde(flatten)]
    pub tail: TailOptions,
}

impl LiveOptions {
    /// The options in effect: defaults filled in and every value clamped to its range.
    pub(crate) fn resolved(self) -> LiveOptions {
        LiveOptions {
            watch: self.watch,
            poll_interval_ms: Some(
                self.poll_interval_ms
                    .unwrap_or(DEFAULT_POLL_MS)
                    .clamp(*POLL_RANGE.start(), *POLL_RANGE.end()),
            ),
            tail: self.tail.clamped(),
        }
    }
}

/// Reply of `live_state`: whether the window's live tail runs, and its options.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LiveState {
    pub enabled: bool,
    pub options: LiveOptions,
}

/// A window's running live session, if any, and the options it was last started with.
#[derive(Default)]
pub(crate) struct LiveSlot {
    running: Mutex<Option<LiveTail>>,
    options: Mutex<Option<LiveOptions>>,
    /// Alert state, mutes and file rates; outlives stopping and starting live tail.
    session: Mutex<LiveSession>,
    /// A refresh left an event held back: ask again once the file is idle.
    flush_wanted: AtomicBool,
    /// A follow-up refresh is waiting for the idle time.
    flush_scheduled: AtomicBool,
    /// The annotators' pending cursor (first new event since they last ran) and when they last ran.
    annotating: Mutex<(Option<debrief_core::plugin::annotate::RecordCursor>, Option<std::time::Instant>)>,
}

impl LiveSlot {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<LiveTail>> {
        crate::sync::lock(&self.running)
    }

    fn options(&self) -> std::sync::MutexGuard<'_, Option<LiveOptions>> {
        crate::sync::lock(&self.options)
    }

    fn session(&self) -> std::sync::MutexGuard<'_, LiveSession> {
        crate::sync::lock(&self.session)
    }

    /// Whether live tail runs and the options in effect (defaults until it was first started).
    pub(crate) fn state(&self) -> LiveState {
        LiveState {
            enabled: self.lock().is_some(),
            options: self.options().clone().unwrap_or_else(|| LiveOptions::default().resolved()),
        }
    }

    #[cfg(test)]
    pub(crate) fn is_running(&self) -> bool {
        self.lock().is_some()
    }
}

/// Id of a window's live tail in the background task registry.
pub(crate) fn task_id(label: &str) -> String {
    format!("live:{label}")
}

/// `notify` watchers behind the engine's port.
struct NotifyFactory;

struct Notified<W>(W);

impl<W: notify::Watcher + Send> FileWatcher for Notified<W> {
    fn watch(&mut self, path: &Path, recursive: bool) -> anyhow::Result<()> {
        let mode = if recursive {
            notify::RecursiveMode::Recursive
        } else {
            notify::RecursiveMode::NonRecursive
        };
        self.0.watch(path, mode).map_err(anyhow::Error::from)
    }
}

impl WatcherFactory for NotifyFactory {
    fn create(&self, mode: WatchMode, poll: Duration, sink: WatchSink) -> anyhow::Result<Box<dyn FileWatcher>> {
        let handler = move |res: notify::Result<notify::Event>| match res {
            // Opening or reading a file changes nothing.
            Ok(ev) if matches!(ev.kind, notify::EventKind::Access(_)) => {}
            Ok(ev) => sink(WatchSignal::Paths(ev.paths)),
            Err(e) => {
                log_watcher_error(&e, std::time::Instant::now());
                sink(WatchSignal::Error(e.to_string()))
            }
        };
        Ok(match mode {
            WatchMode::Polling => Box::new(Notified(notify::PollWatcher::new(
                handler,
                notify::Config::default().with_poll_interval(poll),
            )?)),
            WatchMode::Events | WatchMode::Off => Box::new(Notified(notify::recommended_watcher(handler)?)),
        })
    }
}

/// File watcher errors are logged at most once per this interval (a broken watcher reports continuously).
const WATCHER_ERROR_INTERVAL: Duration = Duration::from_secs(10);

/// Logs a file watcher error: its kind only (the message may name paths), throttled, with how many were
/// left out since the last entry. Returns whether it logged.
fn log_watcher_error(e: &notify::Error, now: std::time::Instant) -> bool {
    static LAST: std::sync::Mutex<(Option<std::time::Instant>, u64)> = std::sync::Mutex::new((None, 0));
    let mut last = crate::sync::lock(&LAST);
    if last.0.is_some_and(|t| now.duration_since(t) < WATCHER_ERROR_INTERVAL) {
        last.1 += 1;
        return false;
    }
    let suppressed = std::mem::take(&mut last.1);
    last.0 = Some(now);
    drop(last);
    let kind = match &e.kind {
        notify::ErrorKind::Io(io) => format!("Io({:?})", io.kind()),
        other => format!("{other:?}").chars().take(80).collect(),
    };
    log::warn!(ErrorKind = kind.as_str(), Suppressed = suppressed; "File watcher error; live tail may fall back to polling");
    true
}

/// Watcher settings from the user's debounce setting and the request.
fn watch_settings(debounce: Duration, options: &LiveOptions) -> WatchSettings {
    let debounce = debounce.as_millis().clamp(1, 10_000) as u64;
    WatchSettings {
        preference: options.watch,
        poll_interval_ms: options
            .poll_interval_ms
            .unwrap_or(DEFAULT_POLL_MS)
            .clamp(*POLL_RANGE.start(), *POLL_RANGE.end()),
        quiet_ms: debounce.min(100),
        max_delay_ms: debounce,
        min_interval_ms: debounce.max(MIN_EVENT_INTERVAL_MS),
        tick_ms: 1_000,
    }
}

/// Tells the owning window what a refresh changed, and raises its alerts
/// (also as a desktop notification while the window is hidden or minimised).
pub(crate) fn emit_report<R: Runtime>(app: &AppHandle<R>, label: &str, report: &TailReport) {
    if report.is_empty() {
        return;
    }
    emit_to_window(app, label, CHANGED_EVENT, report);
    if report.alerts.is_empty() {
        return;
    }
    for alert in &report.alerts {
        log::info!(Kind = alert.kind.as_str(); "Live alert");
        emit_to_window(app, label, ALERT_EVENT, alert);
        crate::notification_centre::post(app, Some(label), alert_notice(alert), None);
    }
    let body = match report.alerts.len() {
        1 => "A live tail alert was raised. Open Debrief to see it.".to_string(),
        n => format!("{n} live tail alerts were raised. Open Debrief to see them."),
    };
    // Window getters wait for the main thread: they run there, never on this worker.
    let label = label.to_string();
    crate::ui_dispatch::run(app, move |app| {
        let seen = app
            .get_window(&label)
            .is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false));
        notify(app, NotificationKind::LiveAlert, seen, "Debrief live alert", &body);
    });
}

/// A live alert as a notification-centre message: repeats of the same kind on the same instance (or rule)
/// merge into one with a count. (Muted alerts never get here; the OS notification is raised below.)
pub(crate) fn alert_notice(alert: &debrief_core::workspace::LiveAlert) -> crate::notification_centre::centre::NewNotice {
    use crate::notification_centre::centre::{NewNotice, NoticeAction, NoticeScope, NoticeSeverity, NoticeSource};
    let what = match alert.kind.as_str() {
        "errorSpike" => "Error spike".to_string(),
        "silence" => "Silence".into(),
        "crash" => "Crash".into(),
        "slowOutlier" => "Slow outlier".into(),
        "rateChange" => "Rate change".into(),
        "memoryGrowth" => "Memory growth".into(),
        "rule" => alert.rule.clone().unwrap_or_else(|| "Alert rule".into()),
        other => other.to_string(),
    };
    let who = alert.rule.clone().or_else(|| alert.instance.clone()).unwrap_or_default();
    let title = match &alert.instance {
        Some(i) if !i.is_empty() => format!("{what} on {i}"),
        _ => what,
    };
    let severity = match alert.severity {
        debrief_core::api::Severity::Critical => NoticeSeverity::Error,
        debrief_core::api::Severity::Warning => NoticeSeverity::Warning,
        debrief_core::api::Severity::Info => NoticeSeverity::Info,
    };
    NewNotice::new(
        format!("live:{}:{who}", alert.kind),
        NoticeSource::LiveAlert,
        severity,
        title,
        NoticeScope::Project,
    )
    .body(alert.message.clone())
    .primary(NoticeAction::ShowAlert {
        alert_id: alert.id.clone(),
    })
}

/// Turns a window's live tail on (replacing a running session) or off;
/// returns how many folders are watched. Never waits for a batch in flight: stopping a session only
/// sets its stop token (see [`LiveTail`]), and the workspace is touched without holding the session lock.
pub(crate) fn set<R: Runtime>(app: &AppHandle<R>, state: &Arc<WindowState>, enabled: bool, options: LiveOptions) -> CmdResult<usize> {
    let label = state.label().to_string();
    // Stops the previous session; its batch in flight, if any, sees the stop token and applies nothing.
    let previous = state.live.lock().take();
    drop(previous);
    if !enabled {
        if let Some(tasks) = app.try_state::<BackgroundTasks>() {
            tasks.remove(&task_id(&label));
        }
        return Ok(0);
    }
    let options = options.resolved();
    *state.live.options() = Some(options.clone());
    // The opened roots, as the store remembers them for refresh.
    let store = state.engine.store()?;
    // Alerts count the events newer than the store's newest one from now on.
    if let Err(e) = state.live.session().start(&*store) {
        log::warn!("live alerts could not start: {e}");
    }
    let saved = store
        .save_project(&debrief_core::package::ProjectManifest::default())
        .map_err(CmdError::from)?;
    let roots: Vec<PathBuf> = saved
        .roots
        .iter()
        .map(|r| PathBuf::from(r.split(debrief_core::archive::ENTRY_SEPARATOR).next().unwrap_or(r)))
        .collect();
    let settings = watch_settings(crate::settings::live_tail_debounce(app), &options);
    let handle = app.clone();
    // Weak: the session lives inside the window state and must not keep it alive.
    let weak: Weak<WindowState> = Arc::downgrade(state);
    let owner = label.clone();
    let stop = StopToken::default();
    let batch_stop = stop.clone();
    let live = LiveTail::start_with(
        &roots,
        settings,
        Arc::new(NotifyFactory),
        Box::new(move |paths| {
            if batch_stop.is_stopped() {
                return;
            }
            let Some(state) = weak.upgrade() else { return };
            if let Some(report) = apply_unless_stopped(&state, &paths, &batch_stop) {
                emit_report(&handle, &owner, &report);
            }
            if state.live.flush_wanted.load(Ordering::SeqCst) {
                schedule_flush(&handle, &weak, &owner, &batch_stop);
            }
            release_off_worker(state);
        }),
        // Paths stay out of the logs; the reason is enough to diagnose.
        Box::new(|_root, why| log::warn!(Reason = why; "Live tail falls back to polling")),
        stop,
    );
    let watched = live.roots().iter().filter(|r| r.mode != WatchMode::Off).count();
    let polling = live.roots().iter().filter(|r| r.mode == WatchMode::Polling).count();
    let replaced = state.live.lock().replace(live);
    drop(replaced);
    register_task(app, &label);
    log::info!(Watched = watched as u64, Polling = polling as u64; "Live tail on");
    Ok(watched)
}

/// Lists the window's live tail as background work that the tray can stop.
fn register_task<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let Some(tasks) = app.try_state::<BackgroundTasks>() else { return };
    let title = app
        .try_state::<Windows>()
        .and_then(|w| w.get(label))
        .map(|w| w.title())
        .unwrap_or_default();
    let spec = TaskSpec {
        id: task_id(label),
        label: if title.is_empty() {
            "Live tail".into()
        } else {
            format!("Live tail — {title}")
        },
        kind: TaskKind::LiveTail,
        window: Some(label.to_string()),
    };
    let handle = app.clone();
    let owner = label.to_string();
    let stop = Arc::new(move || {
        stop(&handle, &owner);
        Ok(())
    });
    if let Err(e) = tasks.register(spec, Some(stop)) {
        log::warn!("could not list live tail as background work: {e:#}");
    }
}

/// Stops a window's live tail from outside its UI and tells the window.
pub(crate) fn stop<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(state) = app.try_state::<Windows>().and_then(|w| w.get(label)) {
        *state.live.lock() = None;
        log::info!("Live tail stopped from the tray");
    }
    emit_to_window(app, label, STOPPED_EVENT, ());
}

/// [`apply_changes`] unless the session stopped. `None`: stopped, nothing applied.
fn apply_unless_stopped(state: &WindowState, paths: &[PathBuf], stop: &StopToken) -> Option<TailReport> {
    if stop.is_stopped() {
        return None;
    }
    Some(apply_changes(state, paths))
}

/// Lets go of the window state from the live tail worker: when the window closed meanwhile, this is the
/// last reference, and the workspace is dropped on a thread of its own rather than on the worker.
fn release_off_worker(state: Arc<WindowState>) {
    if Arc::strong_count(&state) > 1 {
        return;
    }
    // If no thread can start, the state is dropped here (dropping a session never blocks).
    let _ = std::thread::Builder::new()
        .name("debrief-window-drop".into())
        .spawn(move || drop(state));
}

/// The engine-side tail options in effect for a window.
fn tail_options(state: &WindowState) -> TailOptions {
    state
        .live
        .options()
        .clone()
        .unwrap_or_else(|| LiveOptions::default().resolved())
        .tail
}

/// Reads what was appended to `paths` (the store writes new segments) and evaluates the live alerts; a failure is
/// logged and reported as no change.
pub(crate) fn apply_changes(state: &WindowState, paths: &[PathBuf]) -> TailReport {
    let options = tail_options(state);
    let done = state.engine.store().map_err(|e| e.message).and_then(|store| {
        let appended = store.append_tail(paths).map_err(|e| e.to_string())?;
        let report = state.live.session().after_refresh(&*store, &appended, &options);
        let annotating_later = {
            let mut a = crate::sync::lock(&state.live.annotating);
            let (pending, last) = &mut *a;
            crate::engine::annotate::after_tail(state, &*store, pending, last, appended.first_new, std::time::Instant::now())
        };
        // Held events and throttled annotators are both picked up by asking again after the wait.
        state
            .live
            .flush_wanted
            .store(!appended.held.is_empty() || annotating_later, Ordering::SeqCst);
        Ok(report)
    });
    match done {
        Ok(report) => report,
        Err(e) => {
            log::warn!(Files = paths.len() as u64; "Live tail refresh failed: {}", crate::activity::privacy::redact(&e));
            TailReport::default()
        }
    }
}

/// After `carry_flush_ms` (the tail option) asks the store again, with no paths, for the events it held back at the end of idle files
/// (once per wait, however many refreshes asked); repeats while it still holds some. Stops with the session.
fn schedule_flush<R: Runtime>(app: &AppHandle<R>, state: &Weak<WindowState>, label: &str, stop: &StopToken) {
    let Some(strong) = state.upgrade() else { return };
    if strong.live.flush_scheduled.swap(true, Ordering::SeqCst) {
        return;
    }
    drop(strong);
    let (app, state, label, stop) = (app.clone(), state.clone(), label.to_string(), stop.clone());
    let spawned = std::thread::Builder::new().name("debrief-live-flush".into()).spawn(move || loop {
        let Some(win) = state.upgrade() else { return };
        let wait = Duration::from_millis(tail_options(&win).carry_flush_ms);
        drop(win);
        std::thread::sleep(wait + Duration::from_millis(100));
        let Some(win) = state.upgrade() else { return };
        if stop.is_stopped() {
            win.live.flush_scheduled.store(false, Ordering::SeqCst);
            return;
        }
        if let Some(report) = apply_unless_stopped(&win, &[], &stop) {
            emit_report(&app, &label, &report);
        }
        if !win.live.flush_wanted.load(Ordering::SeqCst) {
            win.live.flush_scheduled.store(false, Ordering::SeqCst);
            // A refresh may have asked in between.
            if !win.live.flush_wanted.load(Ordering::SeqCst) || win.live.flush_scheduled.swap(true, Ordering::SeqCst) {
                return;
            }
        }
    });
    if let Err(e) = spawned {
        log::warn!("could not schedule the follow-up live refresh: {e}");
    }
}

/// Per-file live tail state (offset, size, lines/s, rotation...) and totals,
/// with how each file is watched.
#[tauri::command]
pub(crate) async fn tail_status(state: CurrentWindow) -> CmdResult<TailStatus> {
    let win = state.arc().clone();
    crate::engine::with_store(&state, move |s| {
        let running = win.live.lock();
        let mode_of = |p: &Path| running.as_ref().map_or(WatchMode::Off, |l| l.mode_of(p));
        win.live.session().status(s, mode_of)
    })
    .await
}

/// Whether the calling window's live tail runs, and the options in effect
/// (as last given to `set_live`, with defaults filled in and values clamped).
/// The UI restores its live controls from it (e.g. after a webview reload).
#[tauri::command]
pub(crate) async fn live_state(state: CurrentWindow) -> LiveState {
    state.live.state()
}

/// Mutes the calling window's live alerts of one kind or rule until
/// `untilMs` (wall clock, at most a day ahead); they are not raised meanwhile.
#[tauri::command]
pub(crate) async fn mute_alerts(state: CurrentWindow, target: AlertTarget, until_ms: i64) -> CmdResult<()> {
    state
        .live
        .session()
        .mute(target, until_ms)
        .map_err(|e| CmdError::invalid(e.to_string()))
}

/// Lifts a mute set with `mute_alerts`.
#[tauri::command]
pub(crate) async fn unmute_alerts(state: CurrentWindow, target: AlertTarget) -> CmdResult<()> {
    state.live.session().unmute(&target);
    Ok(())
}

/// Opens files and folders for live tail. With `startAtEnd` only the lines written from now on are read (files that
/// cannot be followed from the middle, such as archives and JSON documents, are read in full).
#[tauri::command]
pub(crate) async fn open_paths_live<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    paths: Vec<String>,
    start_at_end: Option<bool>,
) -> CmdResult<u64> {
    let paths: Vec<String> = crate::workspace_commands::open_request(&app, state.label(), paths)?
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let (recorder, recorded) = (app.clone(), paths.clone());
    let at_end = start_at_end.unwrap_or(false);
    // An import job does the opening and the command returns its id at once.
    crate::workspace_commands::start_open_at(&app, &state, paths, at_end, move |summary| {
        log::info!(Sources = summary.sources.len() as u64, StartAtEnd = at_end; "Workspace opened for live tail");
        crate::recent::opened_files(&recorder, &recorded);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Json as _;
    use crate::test_support::Now as _;

    #[test]
    fn options_and_settings() {
        let o: LiveOptions = serde_json::from_value(serde_json::json!({
            "watch": "polling",
            "pollIntervalMs": 5,
            "deletedFiles": "drop",
            "carryFlushMs": 50
        }))
        .unwrap();
        assert_eq!(o.watch, WatchPreference::Polling);
        assert_eq!(o.tail.carry_flush_ms, 50);
        let s = watch_settings(Duration::from_millis(750), &o);
        assert_eq!(
            (s.poll_interval_ms, s.quiet_ms, s.max_delay_ms, s.min_interval_ms),
            (100, 100, 750, 750)
        );
        let s = watch_settings(Duration::from_millis(100), &LiveOptions::default());
        assert_eq!(
            (s.poll_interval_ms, s.max_delay_ms, s.min_interval_ms),
            (DEFAULT_POLL_MS, 100, MIN_EVENT_INTERVAL_MS)
        );
    }

    #[test]
    fn notify_factory_creates_both_kinds() {
        let dir = std::env::temp_dir();
        let sink: WatchSink = Arc::new(|_| {});
        for mode in [WatchMode::Events, WatchMode::Polling] {
            let mut w = NotifyFactory.create(mode, Duration::from_millis(100), sink.clone()).unwrap();
            w.watch(&dir, false).unwrap();
        }
    }

    use crate::test_support::{block, main_window, test_app, window, TempDir};
    use crate::workspace_commands::{open_paths, set_live, workspace_summary};
    use std::sync::Mutex as StdMutex;
    use tauri::{Listener, Manager};

    /// An app whose `label` window has `dir` opened.
    fn opened(app: &tauri::App<tauri::test::MockRuntime>, label: &str, dir: &Path) -> CurrentWindow {
        let win = CurrentWindow::of(app.handle(), label).unwrap();
        crate::paths::allow_under(app.handle(), label, dir);
        crate::test_support::finished(
            app,
            block(open_paths(
                app.handle().clone(),
                win.clone(),
                vec![dir.to_string_lossy().into_owned()],
            )),
        )
        .unwrap()
        .json();
        win
    }

    #[test]
    fn live_tail_per_window() {
        let dir = TempDir::new("live");
        std::fs::write(dir.0.join("a.log"), "2026-10-14 10:00:00 INFO first\n").unwrap();
        let app = test_app(None);
        let st = || main_window(&app);
        opened(&app, "main", &dir.0);
        assert_eq!(block(set_live(app.handle().clone(), st(), true, None)).unwrap(), 1);
        assert!(st().live.is_running());
        // Listed as background work for this window.
        let tasks = app.state::<BackgroundTasks>().list();
        assert_eq!((tasks[0].id.as_str(), tasks[0].window.as_deref()), ("live:main", Some("main")));
        std::fs::write(
            dir.0.join("a.log"),
            "2026-10-14 10:00:00 INFO first\n2026-10-14 10:00:01 INFO second\n",
        )
        .unwrap();
        let report = apply_changes(&st(), &[dir.0.join("a.log")]);
        // The running watcher may pick the change up first; then this refresh finds nothing new.
        assert!([1, 0].contains(&report.appended), "{report:?}");
        emit_report(app.handle(), "main", &report);
        emit_report(app.handle(), "main", &TailReport::default());
        // Generous: a loaded CI runner can stall this test for many seconds.
        let deadline = std::time::Instant::now() + Duration::from_secs(90);
        while crate::test_support::store_summary(&st()).total < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(block(workspace_summary(st())).unwrap().json()["total"], 2);
        // The store lists the followed file with where it was last read; the session's watcher mode is shown.
        let status = block(tail_status(st())).unwrap();
        assert_eq!(status.files.len(), 1);
        assert_eq!((status.files[0].name.as_str(), status.totals.files), ("a.log", 1));
        assert!(status.files[0].size > 0 && status.files[0].offset <= status.files[0].size);
        assert_eq!(block(set_live(app.handle().clone(), st(), false, None)).unwrap(), 0);
        assert!(!st().live.is_running());
        assert!(app.state::<BackgroundTasks>().list().is_empty());

        // Opening for live tail at the end reads nothing of what is there; the file is followed from now on.
        let other = TempDir::new("live-end");
        std::fs::write(other.0.join("b.log"), "2026-10-14 10:00:00 INFO old\n").unwrap();
        crate::paths::allow_under(app.handle(), st().label(), &other.0);
        let summary = crate::test_support::finished(
            &app,
            crate::test_support::returns_fast(
                "open_paths_live",
                open_paths_live(app.handle().clone(), st(), vec![other.0.to_string_lossy().into_owned()], Some(true)),
            ),
        )
        .unwrap()
        .json();
        assert_eq!(summary["total"], 2, "the old line of b.log was not read");
        assert_eq!(summary["sources"].as_array().unwrap().len(), 2);
        let b = other.0.join("b.log");
        std::fs::write(&b, "2026-10-14 10:00:00 INFO old\n2026-10-14 10:00:05 ERROR new\n").unwrap();
        let report = apply_changes(&st(), std::slice::from_ref(&b));
        assert_eq!(report.appended, 0, "the last line is held back until the file is idle: {report:?}");
        assert!(st().live.flush_wanted.load(Ordering::SeqCst));
        std::thread::sleep(Duration::from_millis(
            debrief_core::workspace::TailOptions::default().carry_flush_ms + 200,
        ));
        let report = apply_changes(&st(), &[]);
        assert_eq!(report.appended, 1, "{report:?}");
        assert!(!st().live.flush_wanted.load(Ordering::SeqCst));
        assert_eq!(crate::test_support::store_summary(&st()).total, 3);
        // A refresh that changes nothing is reported as no change.
        assert!(apply_changes(&st(), &[]).is_empty());
    }

    /// Annotators run at most once a second while live tail appends. A refresh that has to wait for that keeps what
    /// arrived pending and asks for a follow-up (`flush_wanted`); the follow-up runs them and stops asking.
    #[test]
    fn throttled_annotators_ask_for_a_follow_up_refresh_that_runs_them() {
        use crate::engine::annotate::{tests::recording_registry, TAIL_EVERY};
        let (registry, recorder) = recording_registry();
        let win = crate::windows::tests::window_state("live-annotate", registry);
        let dir = TempDir::new("live-annotate");
        let log = dir.0.join("a.log");
        let line = |s: u32| format!("2026-10-14 10:00:{s:02} INFO event {s}\n");
        std::fs::write(&log, line(0)).unwrap();
        crate::test_support::import(&win, std::slice::from_ref(&log));
        let runs = || recorder.0.lock().unwrap().len();
        let pending = || crate::sync::lock(&win.live.annotating).0.is_some();
        let grow = |lines: &[u32]| {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
            for s in lines {
                f.write_all(line(*s).as_bytes()).unwrap();
            }
        };
        // The newest event of a refresh is held back until the file is idle, so each refresh writes the one before.
        grow(&[1, 2]);
        let started = std::time::Instant::now();
        let first = apply_changes(&win, std::slice::from_ref(&log));
        assert_eq!(first.appended, 1, "{first:?}");
        assert_eq!(runs(), 1, "the first refresh runs the annotators at once");
        assert!(!pending());

        // Another refresh right away is throttled: its events wait, and a follow-up is wanted.
        grow(&[3]);
        let second = apply_changes(&win, std::slice::from_ref(&log));
        assert_eq!(second.appended, 1, "{second:?}");
        if started.elapsed() >= TAIL_EVERY {
            // The machine stalled for longer than the throttle window between the two refreshes, so the second was
            // not "right away"; the throttle itself is checked with explicit times in `engine::annotate`.
            eprintln!("skipped: {:?} passed between two refreshes", started.elapsed());
            return;
        }
        assert_eq!(runs(), 1, "throttled");
        assert!(pending(), "what arrived waits for the next run");
        assert!(win.live.flush_wanted.load(Ordering::SeqCst), "the follow-up is wanted");

        // The follow-up (no paths) once the window is over writes the held event, runs the annotators and stops asking.
        std::thread::sleep(TAIL_EVERY.max(Duration::from_millis(tail_options(&win).carry_flush_ms)) + Duration::from_millis(200));
        let again = apply_changes(&win, &[]);
        assert_eq!(again.appended, 1, "{again:?}");
        assert_eq!(runs(), 2, "the waiting events were annotated by the follow-up");
        assert!(!pending());
        assert!(!win.live.flush_wanted.load(Ordering::SeqCst), "nothing is left to follow up");
    }

    /// The tray (main thread) and `set_live` stop a session at once; a stopped session applies nothing.
    #[test]
    fn stopping_returns_at_once_and_a_stopped_session_applies_nothing() {
        let dir = TempDir::new("live-stop");
        let log = dir.0.join("a.log");
        std::fs::write(&log, "2026-10-14 10:00:00 INFO first\n").unwrap();
        let app = test_app(None);
        let state = opened(&app, "main", &dir.0);
        block(set_live(app.handle().clone(), state.clone(), true, None)).unwrap();
        let started = std::time::Instant::now();
        stop(app.handle(), "main");
        assert!(started.elapsed() < Duration::from_secs(1), "the tray did not wait");
        assert!(!state.live.is_running());
        let token = StopToken::default();
        token.stop();
        std::fs::write(&log, "2026-10-14 10:00:00 INFO first\n2026-10-14 10:00:01 INFO second\n").unwrap();
        assert!(apply_unless_stopped(&state, std::slice::from_ref(&log), &token).is_none());
        assert_eq!(
            crate::test_support::store_summary(&state).total,
            1,
            "a stopped session applies nothing"
        );
        // Again through the command, with a session restarted.
        block(set_live(app.handle().clone(), state.clone(), true, None)).unwrap();
        assert_eq!(block(set_live(app.handle().clone(), state.clone(), false, None)).unwrap(), 0);
    }

    /// Events go to the owning window only, and the tray can stop a window's live tail.
    #[test]
    fn events_reach_the_owning_window_only_and_the_tray_stops_live_tail() {
        let dir = TempDir::new("live-route");
        std::fs::write(dir.0.join("a.log"), "2026-10-14 10:00:00 INFO first\n").unwrap();
        let app = test_app(None);
        let a = window(&app, "main");
        let b = window(&app, "w-2");
        let heard = Arc::new(StdMutex::new(Vec::<String>::new()));
        for (w, name) in [(&a, "main"), (&b, "w-2")] {
            for event in [CHANGED_EVENT, STOPPED_EVENT, crate::snapshots::ERROR_EVENT] {
                let h = heard.clone();
                w.listen(event, move |_| h.lock().unwrap().push(format!("{name} {event}")));
            }
        }
        let main = opened(&app, "main", &dir.0);
        block(set_live(app.handle().clone(), main.clone(), true, None)).unwrap();
        std::fs::write(
            dir.0.join("a.log"),
            "2026-10-14 10:00:00 INFO first\n2026-10-14 10:00:01 INFO second\n",
        )
        .unwrap();
        // What the tail applied (the running watcher may take the change first, so the report is made here):
        // this test is about where events go.
        let report = TailReport {
            appended: 2,
            ..TailReport::default()
        };
        emit_report(app.handle(), "main", &report);
        crate::snapshots::emit_engine_error(
            app.handle(),
            "w-2",
            &crate::snapshots::EngineError {
                kind: crate::snapshots::EngineErrorKind::ImportCrash,
                message: "m".into(),
                plugin: None,
            },
        );
        app.state::<BackgroundTasks>().stop("live:main").unwrap();
        assert!(!main.live.is_running());
        assert!(app.state::<BackgroundTasks>().list().is_empty());
        assert_eq!(
            *heard.lock().unwrap(),
            [
                format!("main {CHANGED_EVENT}"),
                format!("w-2 {}", crate::snapshots::ERROR_EVENT),
                format!("main {STOPPED_EVENT}"),
            ]
        );
        // Closing the window frees it; its live tail ends with it.
        let weak = Arc::downgrade(main.arc());
        drop(main);
        crate::window_manager::destroyed(app.handle(), "main");
        // A batch the watcher had in flight (the store re-reading the file) finishes, then lets go.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while weak.upgrade().is_some() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(weak.upgrade().is_none());
    }

    /// `live_state` reflects the window's live tail and its resolved options, across stop and restart.
    #[test]
    fn live_state_reports_enabled_and_resolved_options() {
        let dir = TempDir::new("live-state");
        std::fs::write(dir.0.join("a.log"), "2026-10-14 10:00:00 INFO first\n").unwrap();
        let app = test_app(None);
        let st = || main_window(&app);
        // Never started: off, with the defaults resolved.
        let fresh = live_state(st()).now();
        assert!(!fresh.enabled);
        assert_eq!(fresh.options.poll_interval_ms, Some(DEFAULT_POLL_MS));
        assert_eq!(fresh.options.tail, TailOptions::default());
        let json = serde_json::to_value(&fresh).unwrap();
        assert_eq!(json["options"]["detectEveryMs"], 1000);
        assert_eq!(json["options"]["watch"], "auto");
        assert_eq!(json["enabled"], false);

        opened(&app, "main", &dir.0);
        let options: LiveOptions = serde_json::from_value(serde_json::json!({
            "watch": "polling", "pollIntervalMs": 5, "detectEveryMs": 999_999_999u64, "carryFlushMs": 300
        }))
        .unwrap();
        block(set_live(app.handle().clone(), st(), true, Some(options))).unwrap();
        let on = live_state(st()).now();
        assert!(on.enabled);
        assert_eq!(on.options.watch, WatchPreference::Polling);
        assert_eq!(on.options.poll_interval_ms, Some(100));
        assert_eq!(on.options.tail.detect_every_ms, debrief_core::workspace::MAX_DETECT_EVERY_MS);
        assert_eq!(on.options.tail.carry_flush_ms, 300);
        // Off keeps the options for the next start.
        block(set_live(app.handle().clone(), st(), false, None)).unwrap();
        let off = live_state(st()).now();
        assert!(!off.enabled);
        assert_eq!(off.options, on.options);
        // Stopped from the tray: off.
        block(set_live(app.handle().clone(), st(), true, None)).unwrap();
        assert!(live_state(st()).now().enabled);
        assert_eq!(live_state(st()).now().options.poll_interval_ms, Some(DEFAULT_POLL_MS));
        stop(app.handle(), "main");
        assert!(!live_state(st()).now().enabled);
    }

    /// Mutes belong to the window's live session: they validate like the engine's, and unmuting lifts them.
    #[test]
    fn mute_and_unmute_alerts() {
        let app = test_app(None);
        let st = || main_window(&app);
        let rule = AlertTarget::Rule("payment errors".into());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let e = block(mute_alerts(st(), rule.clone(), 0)).unwrap_err();
        assert_eq!(e.code, crate::ErrorCode::Invalid);
        assert!(e.message.contains("future"), "{}", e.message);
        let e = block(mute_alerts(st(), rule.clone(), now + 3 * 24 * 3_600_000)).unwrap_err();
        assert!(e.message.contains("24 hours"), "{}", e.message);
        block(mute_alerts(st(), rule.clone(), now + 60_000)).unwrap();
        assert!(
            st().live.session().mute(rule.clone(), now + 120_000).is_ok(),
            "a second mute replaces the first"
        );
        block(unmute_alerts(st(), rule)).unwrap();
        // The wire format names exactly one target.
        assert!(serde_json::from_value::<AlertTarget>(serde_json::json!({ "kind": "crash" })).is_ok());
        assert!(serde_json::from_value::<AlertTarget>(serde_json::json!({ "kind": "crash", "rule": "x" })).is_err());
    }

    /// Smoke test with the real (polling) watcher: an appended line arrives on its own.
    #[test]
    fn live_tail_watcher_smoke() {
        let dir = TempDir::new("live-smoke");
        let file = dir.0.join("a.log");
        std::fs::write(&file, "2026-10-14 10:00:00 INFO first\n").unwrap();
        let app = test_app(None);
        let st = || main_window(&app);
        opened(&app, "main", &dir.0);
        let options: LiveOptions = serde_json::from_value(serde_json::json!({ "watch": "polling", "pollIntervalMs": 100 })).unwrap();
        assert_eq!(block(set_live(app.handle().clone(), st(), true, Some(options))).unwrap(), 1);
        std::thread::sleep(Duration::from_millis(300));
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        std::io::Write::write_all(&mut f, b"2026-10-14 10:00:01 INFO second\n").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while crate::test_support::store_summary(&st()).total < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(crate::test_support::store_summary(&st()).total, 2);
        block(set_live(app.handle().clone(), st(), false, None)).unwrap();
    }

    #[test]
    fn watcher_errors_are_logged_by_kind_and_throttled() {
        let _g = crate::activity::test_capture();
        let t0 = std::time::Instant::now() + Duration::from_secs(3600);
        let e = notify::Error::path_not_found().add_path(PathBuf::from("/secret/dir/a.log"));
        assert!(log_watcher_error(&e, t0));
        assert!(!log_watcher_error(&e, t0 + Duration::from_secs(1)));
        let io = notify::Error::io(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "/secret/dir"));
        assert!(log_watcher_error(&io, t0 + WATCHER_ERROR_INTERVAL * 2));
        let logged: Vec<_> = crate::activity::captured()
            .into_iter()
            .filter(|c| c.message.starts_with("File watcher error"))
            .collect();
        assert_eq!(logged.len(), 2);
        assert_eq!(
            (logged[0].level, logged[0].props["ErrorKind"].as_str()),
            (log::Level::Warn, "PathNotFound")
        );
        assert_eq!(
            (logged[1].props["ErrorKind"].as_str(), logged[1].props["Suppressed"].as_str()),
            ("Io(PermissionDenied)", "1")
        );
        assert!(logged.iter().all(|c| !format!("{:?}", c.props).contains("secret")));
    }

    #[test]
    fn live_alerts_become_messages_that_merge_per_kind_and_instance() {
        let alert = |kind: &str, instance: Option<&str>, rule: Option<&str>| debrief_core::workspace::LiveAlert {
            id: format!("{kind}:x:1"),
            kind: kind.into(),
            rule: rule.map(String::from),
            instance: instance.map(String::from),
            severity: debrief_core::api::Severity::Critical,
            from: 0,
            to: 1,
            message: "12 errors in a minute".into(),
            evidence_query: Default::default(),
        };
        let n = alert_notice(&alert("errorSpike", Some("WEB02"), None));
        assert_eq!(n.title, "Error spike on WEB02");
        assert_eq!(n.cause, "live:errorSpike:WEB02");
        assert_eq!(n.severity, crate::notification_centre::centre::NoticeSeverity::Error);
        let r = alert_notice(&alert("rule", None, Some("payment errors")));
        assert_eq!((r.title.as_str(), r.cause.as_str()), ("payment errors", "live:rule:payment errors"));
        for k in ["silence", "crash", "slowOutlier", "rateChange", "memoryGrowth", "other"] {
            assert!(!alert_notice(&alert(k, None, None)).title.is_empty());
        }
        let app = crate::test_support::test_app(None);
        let a = alert("errorSpike", Some("WEB02"), None);
        let report = TailReport {
            alerts: vec![a.clone(), a],
            ..TailReport::default()
        };
        emit_report(app.handle(), "main", &report);
        let list = app.state::<crate::notification_centre::NotificationCentre>().snapshot("main");
        assert_eq!(
            (list.notices.len(), list.notices[0].count),
            (1, 2),
            "merged: ERROR spike on WEB02 (×2)"
        );
    }
}
