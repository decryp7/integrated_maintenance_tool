//! Live tail in the shell: the `notify` adapter for the engine's watcher port
//! ([`WatcherFactory`]), the running session, and the live tail commands.
//! The engine decides everything else (what changed, rotation, coalescing,
//! polling fallback, alerts); this module only wires it to the OS and the UI.

use crate::{err, AppState, CmdResult, State, CHANGED_EVENT};
use imt_core::workspace::{
    FileWatcher, LiveTail, OpenOptions, TailOptions, TailReport, WatchMode, WatchPreference, WatchSettings, WatchSignal, WatchSink,
    WatcherFactory,
};
use notify_debouncer_mini::notify;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Event carrying one [`imt_core::workspace::LiveAlert`].
pub(crate) const ALERT_EVENT: &str = "imt://live-alert";
/// Polling interval when none is given.
const DEFAULT_POLL_MS: u64 = 2_000;
/// Change events reach the UI at most about four times per second.
const MIN_EVENT_INTERVAL_MS: u64 = 250;

/// Options of `set_live` (all optional; camelCase on the wire).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct LiveOptions {
    /// auto (events, polling for network paths and after errors) | events | polling.
    pub watch: WatchPreference,
    pub poll_interval_ms: Option<u64>,
    #[serde(flatten)]
    pub tail: TailOptions,
}

/// The running live session, if any.
#[derive(Default)]
pub(crate) struct LiveSlot(Mutex<Option<LiveTail>>);

impl LiveSlot {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<LiveTail>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[cfg(test)]
    pub(crate) fn is_running(&self) -> bool {
        self.lock().is_some()
    }
}

fn slot<R: Runtime>(app: &AppHandle<R>) -> State<'_, LiveSlot> {
    if app.try_state::<LiveSlot>().is_none() {
        app.manage(LiveSlot::default());
    }
    app.state::<LiveSlot>()
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
            Err(e) => sink(WatchSignal::Error(e.to_string())),
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

/// Watcher settings from the user's debounce setting and the request.
fn watch_settings(debounce: Duration, options: &LiveOptions) -> WatchSettings {
    let debounce = debounce.as_millis().clamp(1, 10_000) as u64;
    WatchSettings {
        preference: options.watch,
        poll_interval_ms: options.poll_interval_ms.unwrap_or(DEFAULT_POLL_MS).clamp(100, 600_000),
        quiet_ms: debounce.min(100),
        max_delay_ms: debounce,
        min_interval_ms: debounce.max(MIN_EVENT_INTERVAL_MS),
        tick_ms: 1_000,
    }
}

/// Tells the UI what a refresh changed, and raises its alerts.
pub(crate) fn emit_report<R: Runtime>(app: &AppHandle<R>, report: &TailReport) {
    if report.is_empty() {
        return;
    }
    let _ = app.emit(CHANGED_EVENT, report);
    for alert in &report.alerts {
        log::info!(Kind = alert.kind.as_str(); "Live alert");
        let _ = app.emit(ALERT_EVENT, alert);
    }
}

/// Turns live tail on (replacing a running session) or off; returns how many
/// folders are watched.
pub(crate) fn set<R: Runtime>(app: &AppHandle<R>, state: &AppState, enabled: bool, options: LiveOptions) -> CmdResult<usize> {
    let slot = slot(app);
    let mut running = slot.lock();
    // Stops the previous session (its worker finishes the batch in flight).
    *running = None;
    if !enabled {
        return Ok(0);
    }
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
    let live = LiveTail::start(
        &roots,
        settings,
        Arc::new(NotifyFactory),
        Box::new(move |paths| {
            let report = crate::apply_changes(&handle.state::<AppState>(), &paths);
            emit_report(&handle, &report);
        }),
        // Paths stay out of the logs; the reason is enough to diagnose.
        Box::new(|_root, why| log::warn!(Reason = why; "Live tail falls back to polling")),
    );
    let watched = live.roots().iter().filter(|r| r.mode != WatchMode::Off).count();
    let polling = live.roots().iter().filter(|r| r.mode == WatchMode::Polling).count();
    *running = Some(live);
    log::info!(Watched = watched as u64, Polling = polling as u64; "Live tail on");
    Ok(watched)
}

/// Per-file live tail state (offset, size, lines/s, rotation...) and totals,
/// with how each file is watched.
#[tauri::command]
pub(crate) async fn tail_status<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>) -> CmdResult<Value> {
    let status = crate::read(&state, |ws| Ok(ws.tail_status())).await?;
    let slot = slot(&app);
    let running = slot.lock();
    let status = match running.as_ref() {
        Some(live) => status.with_modes(|p| live.mode_of(p)),
        None => status,
    };
    serde_json::to_value(status).map_err(err)
}

/// Opens files and folders; with `startAtEnd` text files are tailed from
/// their current end (only lines written from now on are shown).
#[tauri::command]
pub(crate) async fn open_paths_live<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    paths: Vec<String>,
    start_at_end: Option<bool>,
) -> CmdResult<Value> {
    let options = OpenOptions {
        start_at_end: start_at_end.unwrap_or(false),
    };
    crate::write_reporting(&app, &state, move |ws| {
        ws.open_with(&paths.into_iter().map(PathBuf::from).collect::<Vec<_>>(), &options)?;
        let summary = ws.summary();
        log::info!(Sources = summary.sources.len() as u64, StartAtEnd = options.start_at_end; "Workspace opened for live tail");
        serde_json::to_value(summary).map_err(anyhow::Error::from)
    })
    .await
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
}
