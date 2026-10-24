//! The start-up contract's Rust half (ADR 0058): every Debrief webview reports `window_ready` once it has painted
//! its frame; a watchdog reloads a window that did not within [`READY_TIMEOUT`] and, if the reload does not help
//! either, shows a native "not responding" dialog (Reload / Restart in Safe Mode / Keep Waiting). A window whose
//! web content process terminated is reloaded by `window_manager::web_content_terminated` and marked here, so the
//! reloaded page can tell the user calmly what happened.
//!
//! Why: the webview's content process is a separate process that can be paused or throttled for seconds (CPU
//! limiters such as App Tamer stop it), killed, or can lose its first IPC message; the window then stayed blank
//! with no way out but quitting. The decisions are pure ([`Watchdog`]); the ticker thread only applies them.
//!
//! Each page also keeps its UI state in memory here (`ui_checkpoint`), so a page Debrief reloads gets its query,
//! filters and selection back with its `window_ready` reply.
//!
//! Commands: `window_ready`, `window_boot_error`, `ui_checkpoint`.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Runtime};

/// How long a page may take from starting to load until it reports ready.
pub const READY_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the ticker looks at the windows.
const TICK: Duration = Duration::from_millis(500);
/// A gap between two ticks longer than this means Debrief itself was paused (suspended, stopped by a CPU limiter):
/// the waiting windows' clocks restart instead of all timing out at once on resume.
const PAUSE_GAP: Duration = Duration::from_secs(3);
/// Longest boot error message logged (the rest is cut).
const MAX_ERROR_LEN: usize = 500;

/// Why a window was reloaded by Debrief (told to the reloaded page once).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Recovery {
    /// The page did not report ready in time.
    Unresponsive,
    /// The web content process terminated (crashed, killed, out of memory). Reported on macOS only (Tauri 2).
    #[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
    Terminated,
}

/// The reply to `window_ready`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyReply {
    /// Set when Debrief reloaded this window before this page loaded.
    pub recovered: Option<Recovery>,
    /// With `recovered`: the window's last UI state checkpoint (query, filters, selection, …) for the reloaded page
    /// to apply once its workspace is loaded. The engine kept the data; this is the rest.
    pub ui: Option<serde_json::Value>,
}

/// What the ticker must do for a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Reload the webview (first missed deadline).
    Reload(String),
    /// Show the "not responding" dialog (the reload did not help).
    Recover(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Loading: must report ready before `since + READY_TIMEOUT`.
    Waiting,
    Ready,
    /// The dialog is open; nothing more until the user answers.
    Asking,
}

#[derive(Debug, Clone)]
struct Entry {
    phase: Phase,
    since: Instant,
    /// Reloads the watchdog did for this window since it was last ready.
    reloads: u8,
    recovered: Option<Recovery>,
}

/// The per-window start-up state (pure: the caller passes the time).
#[derive(Debug, Default)]
pub struct Watchdog {
    windows: HashMap<String, Entry>,
    last_tick: Option<Instant>,
}

impl Watchdog {
    /// A page started loading in window `label` (first load or reload).
    pub fn page_started(&mut self, label: &str, now: Instant) {
        let e = self.windows.entry(label.to_string()).or_insert(Entry {
            phase: Phase::Waiting,
            since: now,
            reloads: 0,
            recovered: None,
        });
        e.phase = Phase::Waiting;
        e.since = now;
    }

    /// The page reported ready: returns (and forgets) why Debrief reloaded it, if it did.
    pub fn ready(&mut self, label: &str) -> Option<Recovery> {
        let e = self.windows.get_mut(label)?;
        e.phase = Phase::Ready;
        e.reloads = 0;
        e.recovered.take()
    }

    /// The window's web content process terminated (it is being reloaded).
    #[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
    pub fn terminated(&mut self, label: &str, now: Instant) {
        self.page_started(label, now);
        if let Some(e) = self.windows.get_mut(label) {
            e.recovered = Some(Recovery::Terminated);
        }
    }

    /// The user chose to keep waiting: the window gets another full timeout before the dialog comes back.
    pub fn keep_waiting(&mut self, label: &str, now: Instant) {
        if let Some(e) = self.windows.get_mut(label) {
            e.phase = Phase::Waiting;
            e.since = now;
        }
    }

    /// Forgets a closed window.
    pub fn closed(&mut self, label: &str) {
        self.windows.remove(label);
    }

    /// Whether window `label` reported ready.
    #[cfg(test)]
    pub fn is_ready(&self, label: &str) -> bool {
        self.windows.get(label).is_some_and(|e| e.phase == Phase::Ready)
    }

    /// The windows whose deadline passed at `now`, with what to do (and records it). A long gap since the last tick
    /// means Debrief itself was paused: the waiting windows' clocks restart rather than fire.
    pub fn due(&mut self, now: Instant) -> Vec<Action> {
        let paused = self.last_tick.is_some_and(|t| now.saturating_duration_since(t) > PAUSE_GAP);
        self.last_tick = Some(now);
        let mut out = Vec::new();
        for (label, e) in &mut self.windows {
            if e.phase != Phase::Waiting {
                continue;
            }
            if paused {
                e.since = now;
                continue;
            }
            if now.saturating_duration_since(e.since) < READY_TIMEOUT {
                continue;
            }
            if e.reloads == 0 {
                e.reloads = 1;
                e.since = now;
                e.recovered = Some(Recovery::Unresponsive);
                out.push(Action::Reload(label.clone()));
            } else {
                e.phase = Phase::Asking;
                out.push(Action::Recover(label.clone()));
            }
        }
        out.sort_by(|a, b| action_label(a).cmp(action_label(b)));
        out
    }
}

fn action_label(a: &Action) -> &str {
    match a {
        Action::Reload(l) | Action::Recover(l) => l,
    }
}

/// The latest UI state of each window (memory only), sent by the page as it changes: what a page reloaded by
/// Debrief restores. The autosaved session on disk is for restarts; this is for reloads within the run.
#[derive(Debug, Default)]
pub struct Checkpoints(HashMap<String, serde_json::Value>);

impl Checkpoints {
    pub fn put(&mut self, label: &str, ui: serde_json::Value) {
        self.0.insert(label.to_string(), ui);
    }

    /// The window's last checkpoint (kept: a second reload restores it too).
    pub fn get(&self, label: &str) -> Option<serde_json::Value> {
        self.0.get(label).cloned()
    }

    pub fn closed(&mut self, label: &str) {
        self.0.remove(label);
    }
}

/// What `window_ready` answers: the recovery reason and, with it, the UI state to restore.
pub fn ready_reply(dog: &mut Watchdog, checkpoints: &Checkpoints, label: &str) -> ReadyReply {
    let recovered = dog.ready(label);
    let ui = recovered.and_then(|_| checkpoints.get(label));
    ReadyReply { recovered, ui }
}

/// The app-wide watchdog and checkpoints (managed state).
#[derive(Default)]
pub struct StartupWatchdog(Mutex<Watchdog>, Mutex<Checkpoints>);

impl StartupWatchdog {
    pub fn with<T>(&self, f: impl FnOnce(&mut Watchdog) -> T) -> T {
        f(&mut crate::sync::lock(&self.0))
    }

    pub fn checkpoints<T>(&self, f: impl FnOnce(&mut Checkpoints) -> T) -> T {
        f(&mut crate::sync::lock(&self.1))
    }
}

/// Whether a webview is one of Debrief's own pages (main, workspace, pop-out and dialog windows): not the
/// internal browser's page (another webview in a window) and not the drag ghost.
pub fn watched(webview_label: &str, window_label: &str) -> bool {
    webview_label == window_label && webview_label != crate::drag_ghost::GHOST_LABEL
}

/// `Builder::on_page_load`: arms the watchdog when one of Debrief's pages starts loading.
pub fn on_page_load<R: Runtime>(webview: &tauri::Webview<R>, payload: &tauri::webview::PageLoadPayload<'_>) {
    if payload.event() != tauri::webview::PageLoadEvent::Started || !watched(webview.label(), webview.window().label()) {
        return;
    }
    if let Some(w) = webview.try_state::<StartupWatchdog>() {
        w.with(|w| w.page_started(webview.label(), Instant::now()));
    }
}

/// A web content process terminated: the reloaded page will hear why.
#[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
pub fn note_terminated<R: Runtime>(webview: &tauri::Webview<R>) {
    if let Some(w) = webview.try_state::<StartupWatchdog>() {
        w.with(|w| w.terminated(webview.label(), Instant::now()));
    }
}

/// Starts the ticker thread (background; never the main thread). It ends with the process.
pub fn start<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("startup-watchdog".into()).spawn(move || loop {
        std::thread::sleep(TICK);
        if app.try_state::<crate::windows::Windows>().is_some_and(|w| w.exiting()) {
            return;
        }
        let Some(dog) = app.try_state::<StartupWatchdog>() else { continue };
        // Windows that are gone are forgotten.
        let gone: Vec<String> = dog.with(|w| w.windows.keys().filter(|l| app.get_webview(l).is_none()).cloned().collect());
        dog.checkpoints(|c| gone.iter().for_each(|l| c.closed(l)));
        let actions = dog.with(|w| {
            gone.iter().for_each(|l| w.closed(l));
            w.due(Instant::now())
        });
        for action in actions {
            apply(&app, action);
        }
    });
    if let Err(e) = spawned {
        log::error!("The start-up watchdog could not start: {e}");
    }
}

fn apply<R: Runtime>(app: &AppHandle<R>, action: Action) {
    match action {
        Action::Reload(label) => {
            log::error!(Window = label.as_str(), TimeoutS = READY_TIMEOUT.as_secs(); "Window {label} did not finish starting in time; reloading it");
            crate::ui_dispatch::run(app, move |app| {
                if let Some(w) = app.get_webview(&label) {
                    if let Err(e) = w.reload() {
                        log::error!(Window = label.as_str(); "Could not reload window {label}: {e}");
                    }
                }
            });
        }
        Action::Recover(label) => {
            log::error!(Window = label.as_str(); "Window {label} did not start after a reload; asking the user");
            crate::ui_dispatch::run(app, move |app| ask(app, label));
        }
    }
}

/// What the user chose in the "not responding" dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Reload,
    SafeMode,
    Wait,
}

pub const RELOAD_LABEL: &str = "Reload";
pub const SAFE_MODE_LABEL: &str = "Restart in Safe Mode";
pub const WAIT_LABEL: &str = "Keep Waiting";

/// The dialog's answer as a choice (custom labels on macOS and Windows; Yes/No/Cancel elsewhere).
pub fn choice_of(result: &tauri_plugin_dialog::MessageDialogResult) -> Choice {
    use tauri_plugin_dialog::MessageDialogResult as R;
    match result {
        R::Yes | R::Ok => Choice::Reload,
        R::No => Choice::SafeMode,
        R::Custom(s) if s == RELOAD_LABEL => Choice::Reload,
        R::Custom(s) if s == SAFE_MODE_LABEL => Choice::SafeMode,
        _ => Choice::Wait,
    }
}

fn ask<R: Runtime>(app: &AppHandle<R>, label: String) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let mut dialog = app
        .dialog()
        .message(
            "This Debrief window did not finish starting, even after a reload. Another app may be pausing Debrief \
             (for example a CPU limiter), or the window's content keeps failing. Your open files are kept.",
        )
        .title("Debrief is not responding")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::YesNoCancelCustom(
            RELOAD_LABEL.into(),
            SAFE_MODE_LABEL.into(),
            WAIT_LABEL.into(),
        ));
    if let Some(w) = app.get_window(&label) {
        dialog = dialog.parent(&w);
    }
    let app = app.clone();
    dialog.show_with_result(move |result| {
        let choice = choice_of(&result);
        log::info!(Window = label.as_str(), Choice = format!("{choice:?}"); "Not-responding dialog answered");
        match choice {
            Choice::Reload => {
                if let Some(d) = app.try_state::<StartupWatchdog>() {
                    d.with(|w| w.keep_waiting(&label, Instant::now()));
                }
                crate::ui_dispatch::run(&app, move |app| {
                    if let Some(w) = app.get_webview(&label) {
                        let _ = w.reload();
                    }
                });
            }
            Choice::SafeMode => {
                let session = app.try_state::<crate::lifecycle::Lifecycle>().and_then(|l| l.session_path(&label));
                if let Err(e) = crate::lifecycle::relaunch(&app, true, session.as_deref()) {
                    log::error!("Could not restart in safe mode: {e:#}");
                }
            }
            Choice::Wait => {
                if let Some(d) = app.try_state::<StartupWatchdog>() {
                    d.with(|w| w.keep_waiting(&label, Instant::now()));
                }
            }
        }
    });
}

/// The calling page has painted its frame (the start-up contract's "ready"). Tells it whether Debrief reloaded it.
#[tauri::command]
pub async fn window_ready<R: Runtime>(webview: tauri::Webview<R>, dog: tauri::State<'_, StartupWatchdog>) -> crate::CmdResult<ReadyReply> {
    let label = webview.label().to_string();
    let reply = dog.checkpoints(|c| dog.with(|w| ready_reply(w, c, &label)));
    log::info!(Window = label.as_str(), Recovered = format!("{:?}", reply.recovered), RestoresUi = reply.ui.is_some(); "Window {label} ready");
    Ok(reply)
}

/// The calling window's current UI state (memory only; debounced by the UI). Restored if Debrief reloads the page.
#[tauri::command]
pub async fn ui_checkpoint<R: Runtime>(
    webview: tauri::Webview<R>,
    dog: tauri::State<'_, StartupWatchdog>,
    ui: serde_json::Value,
) -> crate::CmdResult<()> {
    if !ui.is_object() {
        return Err("the UI state must be an object".into());
    }
    dog.checkpoints(|c| c.put(webview.label(), ui));
    Ok(())
}

/// One line of a boot error for the log: no control characters, at most [`MAX_ERROR_LEN`] characters.
pub fn clean_error(text: &str) -> String {
    let mut out: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_ERROR_LEN)
        .collect();
    if text.chars().count() > MAX_ERROR_LEN {
        out.push('…');
    }
    out
}

/// An uncaught error or unhandled rejection in a page before (or while) it started, logged with its window.
/// `kind` is `error`, `rejection` or `bootstrap`; the message is cut to one bounded line (no file contents: the UI
/// sends the error's message and first stack frame only).
#[tauri::command]
pub async fn window_boot_error<R: Runtime>(webview: tauri::Webview<R>, kind: String, message: String) {
    let kind = match kind.as_str() {
        "error" | "rejection" | "bootstrap" => kind,
        _ => "other".into(),
    };
    log::error!(Window = webview.label(), Kind = kind.as_str(); "Uncaught {kind} in window {}: {}", webview.label(), clean_error(&message));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    /// Ticks every 500 ms from `from` to `to` (ms after t0), collecting the actions.
    fn run(w: &mut Watchdog, t0: Instant, from: u64, to: u64) -> Vec<(u64, Action)> {
        let mut out = Vec::new();
        let mut t = from;
        while t <= to {
            out.extend(w.due(at(t0, t)).into_iter().map(|a| (t, a)));
            t += 500;
        }
        out
    }

    #[test]
    fn a_window_that_reports_ready_in_time_is_left_alone() {
        let t0 = Instant::now();
        let mut w = Watchdog::default();
        w.page_started("main", t0);
        assert!(run(&mut w, t0, 0, 9_500).is_empty());
        assert_eq!(w.ready("main"), None);
        assert!(w.is_ready("main"));
        assert!(run(&mut w, t0, 10_000, 60_000).is_empty());
    }

    #[test]
    fn a_silent_window_is_reloaded_once_then_the_user_is_asked() {
        let t0 = Instant::now();
        let mut w = Watchdog::default();
        w.page_started("main", t0);
        let actions = run(&mut w, t0, 0, 30_000);
        assert_eq!(
            actions,
            vec![(10_000, Action::Reload("main".into())), (20_000, Action::Recover("main".into()))]
        );
        // Asking: nothing more until the user answers.
        assert!(run(&mut w, t0, 30_500, 60_000).is_empty());
    }

    #[test]
    fn the_reloaded_page_starting_restarts_its_clock_and_hears_why_once_ready() {
        let t0 = Instant::now();
        let mut w = Watchdog::default();
        w.page_started("main", t0);
        assert_eq!(run(&mut w, t0, 0, 10_000).len(), 1);
        w.page_started("main", at(t0, 10_200));
        assert!(
            run(&mut w, t0, 10_500, 20_000).is_empty(),
            "the reload's own page gets a full timeout"
        );
        assert_eq!(w.ready("main"), Some(Recovery::Unresponsive));
        assert_eq!(w.ready("main"), None, "told once");
        // A later reload (by the user, Cmd+R) is a fresh start again: reload first, not the dialog.
        w.page_started("main", at(t0, 40_000));
        assert_eq!(run(&mut w, t0, 40_000, 50_000), vec![(50_000, Action::Reload("main".into()))]);
    }

    #[test]
    fn keep_waiting_gives_another_timeout_before_asking_again() {
        let t0 = Instant::now();
        let mut w = Watchdog::default();
        w.page_started("d-1", t0);
        run(&mut w, t0, 0, 20_000);
        w.keep_waiting("d-1", at(t0, 25_000));
        assert_eq!(run(&mut w, t0, 25_000, 40_000), vec![(35_000, Action::Recover("d-1".into()))]);
    }

    #[test]
    fn a_long_pause_of_debrief_itself_restarts_the_clocks_instead_of_firing() {
        // Debrief's own process stopped for 20 s (CPU limiter) right after the window started loading: on resume the
        // window must get its full timeout, not be reloaded at once.
        let t0 = Instant::now();
        let mut w = Watchdog::default();
        w.page_started("main", t0);
        assert!(run(&mut w, t0, 0, 2_000).is_empty());
        assert!(w.due(at(t0, 22_000)).is_empty(), "resumed after a pause: no action");
        assert!(run(&mut w, t0, 22_500, 31_500).is_empty());
        assert_eq!(w.due(at(t0, 32_000)), vec![Action::Reload("main".into())]);
    }

    #[test]
    fn a_paused_webview_that_resumes_before_the_deadline_is_not_reloaded() {
        // Only the content process is paused (Debrief keeps ticking): if it resumes and reports ready within the
        // timeout, nothing happens; the watchdog is in the app process, which the webview's pause cannot stop.
        let t0 = Instant::now();
        let mut w = Watchdog::default();
        w.page_started("main", t0);
        assert!(run(&mut w, t0, 0, 9_000).is_empty());
        assert_eq!(w.ready("main"), None);
        assert!(run(&mut w, t0, 9_500, 30_000).is_empty());
    }

    #[test]
    fn a_terminated_content_process_is_reported_to_the_reloaded_page() {
        let t0 = Instant::now();
        let mut w = Watchdog::default();
        w.page_started("main", t0);
        w.ready("main");
        w.terminated("main", at(t0, 5_000));
        assert!(!w.is_ready("main"));
        assert!(run(&mut w, t0, 5_000, 14_500).is_empty());
        assert_eq!(w.ready("main"), Some(Recovery::Terminated));
    }

    #[test]
    fn closed_windows_and_unknown_labels_are_ignored() {
        let t0 = Instant::now();
        let mut w = Watchdog::default();
        w.page_started("w-2", t0);
        w.closed("w-2");
        assert!(run(&mut w, t0, 0, 30_000).is_empty());
        assert_eq!(w.ready("nobody"), None);
        w.keep_waiting("nobody", t0);
        assert!(!w.is_ready("nobody"));
    }

    #[test]
    fn several_windows_time_out_in_label_order() {
        let t0 = Instant::now();
        let mut w = Watchdog::default();
        for l in ["w-3", "main", "d-1"] {
            w.page_started(l, t0);
        }
        let labels: Vec<String> = w
            .due(at(t0, 0))
            .into_iter()
            .chain(w.due(at(t0, 500)))
            .map(|a| action_label(&a).to_string())
            .collect();
        assert!(labels.is_empty());
        let mut t = 1_000;
        while t < 10_000 {
            assert!(w.due(at(t0, t)).is_empty());
            t += 500;
        }
        let fired: Vec<String> = w.due(at(t0, 10_000)).iter().map(|a| action_label(a).to_string()).collect();
        assert_eq!(fired, vec!["d-1", "main", "w-3"]);
    }

    #[test]
    fn only_debriefs_own_pages_are_watched() {
        assert!(watched("main", "main"));
        assert!(watched("d-3", "d-3"));
        assert!(watched("p-main-1", "p-main-1"));
        assert!(!watched("browser-main-1", "main"), "the internal browser's page");
        assert!(!watched(crate::drag_ghost::GHOST_LABEL, crate::drag_ghost::GHOST_LABEL));
    }

    #[test]
    fn dialog_answers_map_to_choices_on_every_platform() {
        use tauri_plugin_dialog::MessageDialogResult as R;
        assert_eq!(choice_of(&R::Custom(RELOAD_LABEL.into())), Choice::Reload);
        assert_eq!(choice_of(&R::Custom(SAFE_MODE_LABEL.into())), Choice::SafeMode);
        assert_eq!(choice_of(&R::Custom(WAIT_LABEL.into())), Choice::Wait);
        assert_eq!(choice_of(&R::Yes), Choice::Reload);
        assert_eq!(choice_of(&R::No), Choice::SafeMode);
        assert_eq!(choice_of(&R::Cancel), Choice::Wait, "closing the dialog waits");
    }

    #[test]
    fn a_recovered_page_gets_the_last_checkpoint_and_a_normal_start_does_not() {
        let t0 = Instant::now();
        let mut w = Watchdog::default();
        let mut c = Checkpoints::default();
        w.page_started("main", t0);
        c.put("main", serde_json::json!({"query": {"text": "error"}}));
        // A normal start (or a reload by the user) restores nothing: the page's own state is the truth.
        assert_eq!(ready_reply(&mut w, &c, "main"), ReadyReply { recovered: None, ui: None });
        c.put(
            "main",
            serde_json::json!({"query": {"text": "timeout"}, "selection": {"recordId": 42}}),
        );
        w.terminated("main", at(t0, 1_000));
        let reply = ready_reply(&mut w, &c, "main");
        assert_eq!(reply.recovered, Some(Recovery::Terminated));
        assert_eq!(
            reply.ui,
            Some(serde_json::json!({"query": {"text": "timeout"}, "selection": {"recordId": 42}}))
        );
        // Kept for a second reload; forgotten with the window; other windows have their own.
        assert_eq!(c.get("main"), reply.ui);
        assert_eq!(c.get("w-2"), None);
        c.closed("main");
        assert_eq!(c.get("main"), None);
    }

    #[test]
    fn a_recovered_page_without_a_checkpoint_restores_nothing() {
        let mut w = Watchdog::default();
        w.terminated("w-2", Instant::now());
        assert_eq!(
            ready_reply(&mut w, &Checkpoints::default(), "w-2"),
            ReadyReply {
                recovered: Some(Recovery::Terminated),
                ui: None
            }
        );
    }

    #[test]
    fn boot_errors_are_one_bounded_line() {
        assert_eq!(clean_error("a\nb\tc"), "a b c");
        let long = "x".repeat(MAX_ERROR_LEN + 10);
        let cut = clean_error(&long);
        assert_eq!(cut.chars().count(), MAX_ERROR_LEN + 1);
        assert!(cut.ends_with('…'));
    }
}
