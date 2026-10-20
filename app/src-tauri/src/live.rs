//! Live tail in the shell: the `notify` adapter for the engine's watcher port
//! ([`WatcherFactory`]), each window's running session, and the live tail
//! commands. The engine decides everything else (what changed, rotation,
//! coalescing, polling fallback, alerts); this module only wires it to the OS,
//! the owning window and the background task registry.

use crate::background::{BackgroundTasks, TaskKind, TaskSpec};
use crate::notify::{notify, NotificationKind};
use crate::windows::{emit_to_window, read, write, write_reporting, CurrentWindow, WindowState, Windows};
use crate::{err, CmdResult};
use imt_core::workspace::{
    AlertTarget, FileWatcher, LiveTail, OpenOptions, StopToken, TailOptions, TailReport, WatchMode, WatchPreference, WatchSettings,
    WatchSignal, WatchSink, WatcherFactory,
};
use notify_debouncer_mini::notify;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};

/// Event telling a window that live tail changed its workspace.
pub(crate) const CHANGED_EVENT: &str = "imt://workspace-changed";
/// Event carrying one [`imt_core::workspace::LiveAlert`].
pub(crate) const ALERT_EVENT: &str = "imt://live-alert";
/// Event telling a window that its live tail was stopped from outside (the tray).
pub(crate) const STOPPED_EVENT: &str = "imt://live-stopped";
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
}

impl LiveSlot {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<LiveTail>> {
        self.running.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn options(&self) -> std::sync::MutexGuard<'_, Option<LiveOptions>> {
        self.options.lock().unwrap_or_else(|p| p.into_inner())
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
    let mut last = LAST.lock().unwrap_or_else(|p| p.into_inner());
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
    let seen = app
        .get_webview_window(label)
        .is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false));
    for alert in &report.alerts {
        log::info!(Kind = alert.kind.as_str(); "Live alert");
        emit_to_window(app, label, ALERT_EVENT, alert);
        crate::notification_centre::post(app, Some(label), alert_notice(alert), None);
    }
    let body = match report.alerts.len() {
        1 => "A live tail alert was raised. Open IMT to see it.".to_string(),
        n => format!("{n} live tail alerts were raised. Open IMT to see them."),
    };
    notify(app, NotificationKind::LiveAlert, seen, "IMT live alert", &body);
}

/// A live alert as a notification-centre message: repeats of the same kind on the same instance (or rule)
/// merge into one with a count. (Muted alerts never get here; the OS notification is raised below.)
pub(crate) fn alert_notice(alert: &imt_core::workspace::LiveAlert) -> crate::notification_centre::centre::NewNotice {
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
        imt_core::api::Severity::Critical => NoticeSeverity::Error,
        imt_core::api::Severity::Warning => NoticeSeverity::Warning,
        imt_core::api::Severity::Info => NoticeSeverity::Info,
    };
    NewNotice::new(NoticeSource::LiveAlert, severity, title, NoticeScope::Project)
        .body(alert.message.clone())
        .primary(NoticeAction::ShowAlert {
            alert_id: alert.id.clone(),
        })
        .dedupe(format!("live:{}:{who}", alert.kind))
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
    state.store_write().set_tail_options(options.tail);
    let roots: Vec<PathBuf> = state
        .store_read()
        .state()
        .roots
        .iter()
        .map(|r| PathBuf::from(r.split(imt_core::archive::ENTRY_SEPARATOR).next().unwrap_or(r)))
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

/// [`apply_changes`] unless the session stopped, checked again once the write lock is held (an import may
/// have held it for long). `None`: stopped, nothing applied.
fn apply_unless_stopped(state: &WindowState, paths: &[PathBuf], stop: &StopToken) -> Option<TailReport> {
    let mut ws = state.store_write();
    if stop.is_stopped() {
        return None;
    }
    Some(refresh(&mut **ws, paths))
}

/// Lets go of the window state from the live tail worker: when the window closed meanwhile, this is the
/// last reference, and the workspace is dropped on a thread of its own rather than on the worker.
fn release_off_worker(state: Arc<WindowState>) {
    if Arc::strong_count(&state) > 1 {
        return;
    }
    // If no thread can start, the state is dropped here (dropping a session never blocks).
    let _ = std::thread::Builder::new()
        .name("imt-window-drop".into())
        .spawn(move || drop(state));
}

/// Reads what changed in `paths`; a failure is logged and reported as no change.
#[cfg(test)]
pub(crate) fn apply_changes(state: &WindowState, paths: &[PathBuf]) -> TailReport {
    refresh(&mut **state.store_write(), paths)
}

fn refresh(ws: &mut dyn imt_core::LogStore, paths: &[PathBuf]) -> TailReport {
    ws.refresh_tail(paths).unwrap_or_else(|e| {
        log::warn!(Files = paths.len() as u64; "Live tail refresh failed: {}", crate::activity::privacy::redact(&format!("{e:#}")));
        TailReport::default()
    })
}

/// Per-file live tail state (offset, size, lines/s, rotation...) and totals,
/// with how each file is watched.
#[tauri::command]
pub(crate) async fn tail_status(state: CurrentWindow) -> CmdResult<Value> {
    let status = read(&state, |ws| Ok(ws.tail_status())).await?;
    let running = state.live.lock();
    let status = match running.as_ref() {
        Some(live) => status.with_modes(|p| live.mode_of(p)),
        None => status,
    };
    serde_json::to_value(status).map_err(err)
}

/// Whether the calling window's live tail runs, and the options in effect
/// (as last given to `set_live`, with defaults filled in and values clamped).
/// The UI restores its live controls from it (e.g. after a webview reload).
#[tauri::command]
pub(crate) fn live_state(state: CurrentWindow) -> LiveState {
    state.live.state()
}

/// Mutes the calling window's live alerts of one kind or rule until
/// `untilMs` (wall clock, at most a day ahead); they are not raised meanwhile.
#[tauri::command]
pub(crate) async fn mute_alerts(state: CurrentWindow, target: AlertTarget, until_ms: i64) -> CmdResult<()> {
    write(&state, move |ws| ws.mute_alerts(target, until_ms)).await
}

/// Lifts a mute set with `mute_alerts`.
#[tauri::command]
pub(crate) async fn unmute_alerts(state: CurrentWindow, target: AlertTarget) -> CmdResult<()> {
    write(&state, move |ws| {
        ws.unmute_alerts(&target);
        Ok(())
    })
    .await
}

/// Opens files and folders; with `startAtEnd` text files are tailed from
/// their current end (only lines written from now on are shown).
#[tauri::command]
pub(crate) async fn open_paths_live<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    paths: Vec<String>,
    start_at_end: Option<bool>,
) -> CmdResult<Value> {
    let options = OpenOptions {
        start_at_end: start_at_end.unwrap_or(false),
    };
    let paths = crate::workspace_commands::open_request(paths)?;
    let summary = write_reporting(&app, &state, move |ws| {
        ws.open_with(&paths, &options)?;
        let summary = ws.summary();
        log::info!(Sources = summary.sources.len() as u64, StartAtEnd = options.start_at_end; "Workspace opened for live tail");
        serde_json::to_value(summary).map_err(anyhow::Error::from)
    })
    .await?;
    crate::window_manager::import_finished(&app, state.label(), &summary);
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_and_settings() {
        let o: LiveOptions = serde_json::from_value(serde_json::json!({
            "watch": "polling",
            "pollIntervalMs": 5,
            "deletedFiles": "drop",
            "retention": { "maxRecords": 1000 },
            "carryFlushMs": 50
        }))
        .unwrap();
        assert_eq!(o.watch, WatchPreference::Polling);
        assert_eq!(o.tail.retention.max_records, Some(1000));
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
        block(open_paths(
            app.handle().clone(),
            win.clone(),
            vec![dir.to_string_lossy().into_owned()],
        ))
        .unwrap();
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
        assert!(
            [(1, 1), (0, 0)].contains(&(report.appended, report.sources_changed.len())),
            "{report:?}"
        );
        emit_report(app.handle(), "main", &report);
        emit_report(app.handle(), "main", &TailReport::default());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while st().store_read().summary().total < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(block(workspace_summary(st())).unwrap()["total"], 2);
        let status = block(tail_status(st())).unwrap();
        assert_eq!(status["files"][0]["mode"], "events");
        assert_eq!(status["totals"]["records"], 2);
        assert_eq!(block(set_live(app.handle().clone(), st(), false, None)).unwrap(), 0);
        assert!(!st().live.is_running());
        assert!(app.state::<BackgroundTasks>().list().is_empty());
        let status = block(tail_status(st())).unwrap();
        assert_eq!(status["files"][0]["mode"], "off");

        // Start at end: history skipped, new lines shown.
        let other = TempDir::new("live-end");
        std::fs::write(other.0.join("b.log"), "2026-10-14 10:00:00 INFO old\n").unwrap();
        let summary = block(open_paths_live(
            app.handle().clone(),
            st(),
            vec![other.0.to_string_lossy().into_owned()],
            Some(true),
        ))
        .unwrap();
        assert_eq!(summary["total"], 2);
        // A broken refresh is reported as no change.
        assert!(apply_changes(&st(), &[]).is_empty());
    }

    /// The tray (main thread) and `set_live` stop a session without waiting for its batch in flight, which
    /// may itself wait for an import holding the workspace; the stopped session then applies nothing.
    #[test]
    fn stopping_while_an_import_holds_the_workspace_returns_at_once() {
        let dir = TempDir::new("live-stop-locked");
        let log = dir.0.join("a.log");
        std::fs::write(&log, "2026-10-14 10:00:00 INFO first\n").unwrap();
        let app = test_app(None);
        let state = opened(&app, "main", &dir.0);
        block(set_live(app.handle().clone(), state.clone(), true, None)).unwrap();
        let import = state.store_write();
        std::fs::write(&log, "2026-10-14 10:00:00 INFO first\n2026-10-14 10:00:01 INFO second\n").unwrap();
        // Long enough for the watcher's batch to reach the workspace lock.
        std::thread::sleep(Duration::from_millis(600));
        let started = std::time::Instant::now();
        stop(app.handle(), "main");
        assert!(started.elapsed() < Duration::from_secs(1), "the tray did not wait");
        assert!(!state.live.is_running());
        drop(import);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(state.store_read().summary().total, 1, "a stopped session applies nothing");
        // Again through the command, with a session restarted.
        block(set_live(app.handle().clone(), state.clone(), true, None)).unwrap();
        let import = state.store_write();
        let started = std::time::Instant::now();
        let handle = app.handle().clone();
        let window = state.clone();
        let off = std::thread::spawn(move || block(set_live(handle, window, false, None)));
        while !off.is_finished() && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(off.is_finished(), "stopping returned while the workspace was held");
        drop(import);
        assert_eq!(off.join().unwrap().unwrap(), 0);
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
        let report = apply_changes(&main, &[dir.0.join("a.log")]);
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
        let fresh = live_state(st());
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
        let on = live_state(st());
        assert!(on.enabled);
        assert_eq!(on.options.watch, WatchPreference::Polling);
        assert_eq!(on.options.poll_interval_ms, Some(100));
        assert_eq!(on.options.tail.detect_every_ms, imt_core::workspace::MAX_DETECT_EVERY_MS);
        assert_eq!(on.options.tail.carry_flush_ms, 300);
        // Off keeps the options for the next start.
        block(set_live(app.handle().clone(), st(), false, None)).unwrap();
        let off = live_state(st());
        assert!(!off.enabled);
        assert_eq!(off.options, on.options);
        // Stopped from the tray: off.
        block(set_live(app.handle().clone(), st(), true, None)).unwrap();
        assert!(live_state(st()).enabled);
        assert_eq!(live_state(st()).options.poll_interval_ms, Some(DEFAULT_POLL_MS));
        stop(app.handle(), "main");
        assert!(!live_state(st()).enabled);
    }

    /// Alerts are muted in the window's engine; invalid mutes are refused with a message.
    #[test]
    fn mute_and_unmute_alerts() {
        let app = test_app(None);
        let st = || main_window(&app);
        let rule = AlertTarget::Rule("payment errors".into());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        block(mute_alerts(st(), rule.clone(), now + 60_000)).unwrap();
        let e = block(mute_alerts(st(), rule.clone(), now - 1)).unwrap_err();
        assert!(e.contains("future"), "{e}");
        let e = block(mute_alerts(st(), AlertTarget::Rule(String::new()), now + 60_000)).unwrap_err();
        assert!(e.contains("rule"), "{e}");
        block(unmute_alerts(st(), rule.clone())).unwrap();
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
        while st().store_read().summary().total < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(st().store_read().summary().total, 2);
        let status = block(tail_status(st())).unwrap();
        assert_eq!(status["files"][0]["mode"], "polling");
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
        let alert = |kind: &str, instance: Option<&str>, rule: Option<&str>| imt_core::workspace::LiveAlert {
            id: format!("{kind}:x:1"),
            kind: kind.into(),
            rule: rule.map(String::from),
            instance: instance.map(String::from),
            severity: imt_core::api::Severity::Critical,
            from: 0,
            to: 1,
            message: "12 errors in a minute".into(),
            evidence_query: Default::default(),
        };
        let n = alert_notice(&alert("errorSpike", Some("WEB02"), None));
        assert_eq!(n.title, "Error spike on WEB02");
        assert_eq!(n.dedupe_key.as_deref(), Some("live:errorSpike:WEB02"));
        assert_eq!(n.severity, crate::notification_centre::centre::NoticeSeverity::Error);
        let r = alert_notice(&alert("rule", None, Some("payment errors")));
        assert_eq!(
            (r.title.as_str(), r.dedupe_key.as_deref()),
            ("payment errors", Some("live:rule:payment errors"))
        );
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
