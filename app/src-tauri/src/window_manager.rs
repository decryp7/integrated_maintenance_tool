//! Window operations: opening, listing, revealing and closing windows, what
//! closing the last window does (quit, keep running in the tray, or ask), and
//! the window commands. The decisions are pure functions; this module only
//! applies them to Tauri windows.
//!
//! Commands: `new_window`, `close_window`, `list_windows`, `focus_window`,
//! `set_window_title`, `resolve_close`, `resolve_open`.
//! Events: `debrief://windows-changed` (all windows), `debrief://close-requested`
//! and `debrief://open-ask` (the window concerned).

use crate::notify::{notify, NotificationKind};
use crate::panel_windows;
use crate::window_prefs::{OnClose, SharedPrefs, WINDOW_SECTION};
use crate::windows::{emit_to_window, label_order, queue_open, CurrentWindow, Windows, MAIN};
use crate::{err, CmdResult};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};

pub const WINDOWS_EVENT: &str = "debrief://windows-changed";
pub const CLOSE_REQUESTED_EVENT: &str = "debrief://close-requested";
pub const OPEN_ASK_EVENT: &str = "debrief://open-ask";
pub const DEFAULT_TITLE: &str = "Debrief";
const MAX_TITLE: usize = 200;
/// More windows than this are refused (each holds a workspace in memory).
const MAX_WINDOWS: usize = 32;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub label: String,
    pub title: String,
    pub focused: bool,
    pub visible: bool,
}

/// What closing a window does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// Close it and free its workspace.
    Close,
    /// Hide it; Debrief keeps running in the tray.
    Hide,
    /// Quit Debrief.
    Quit,
    /// Ask the user (the window shows a prompt and calls `resolve_close`).
    Ask,
}

/// Closing a window while others are visible just closes it. Closing the last
/// visible one follows the user's setting; without a tray icon there is
/// nothing to keep running in, so Debrief quits. Closing again while the question
/// is open quits (the UI may be stuck).
pub fn decide_close(pref: OnClose, other_visible: usize, tray_available: bool, already_asked: bool) -> CloseDecision {
    if other_visible > 0 {
        return CloseDecision::Close;
    }
    match pref {
        OnClose::Quit => CloseDecision::Quit,
        OnClose::Tray | OnClose::Ask if !tray_available => CloseDecision::Quit,
        OnClose::Tray => CloseDecision::Hide,
        OnClose::Ask if already_asked => CloseDecision::Quit,
        OnClose::Ask => CloseDecision::Ask,
    }
}

/// A window title from the UI: one line, at most 200 characters, never empty.
pub fn clean_title(title: &str) -> String {
    let t: String = title.chars().filter(|c| !c.is_control()).take(MAX_TITLE).collect();
    let t = t.trim();
    if t.is_empty() {
        DEFAULT_TITLE.to_string()
    } else {
        t.to_string()
    }
}

/// Pop-out panels and the drag ghost belong to a Debrief window; they are not
/// windows of their own.
pub fn is_auxiliary(label: &str) -> bool {
    panel_windows::is_popout(label) || crate::dialog_windows::is_dialog(label) || label == crate::drag_ghost::GHOST_LABEL
}

/// The open windows, `main` first.
pub fn list<R: Runtime>(app: &AppHandle<R>) -> Vec<WindowInfo> {
    let windows = app.try_state::<Windows>();
    let mut out: Vec<WindowInfo> = app
        .windows()
        .into_iter()
        .filter(|(label, _)| !is_auxiliary(label))
        .map(|(label, w)| {
            let title = windows
                .as_ref()
                .and_then(|ws| ws.get(&label))
                .map(|s| s.title())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| w.title().unwrap_or_default());
            WindowInfo {
                title: if title.is_empty() { DEFAULT_TITLE.into() } else { title },
                focused: w.is_focused().unwrap_or(false),
                visible: w.is_visible().unwrap_or(false),
                label,
            }
        })
        .collect();
    out.sort_by_key(|w| label_order(&w.label));
    out
}

/// Tells every window and the tray that the window list changed (on the main thread soon, coalesced:
/// the window getters in [`list`] wait for it).
pub fn changed<R: Runtime>(app: &AppHandle<R>) {
    crate::ui_dispatch::post(app, crate::ui_dispatch::Slot::WindowList, |app| {
        let _ = app.emit(WINDOWS_EVENT, list(app));
    });
    crate::tray::refresh(app);
}

/// Opens a new, empty window with its own workspace; `pending` is opened by
/// its UI on startup. Returns the window's label.
///
/// Commands that call it must be `async`: a window built from a synchronous command never finishes
/// loading on Windows (WebView2 waits for the main thread the command blocks; the window stays white).
/// `tests/it/window_commands.rs` enforces this.
pub fn open_new<R: Runtime>(app: &AppHandle<R>, pending: Vec<String>) -> anyhow::Result<String> {
    let windows = app
        .try_state::<Windows>()
        .ok_or_else(|| anyhow::anyhow!("Debrief is not ready yet"))?;
    if app.windows().keys().filter(|l| !is_auxiliary(l)).count() >= MAX_WINDOWS {
        anyhow::bail!("Debrief already has {MAX_WINDOWS} windows open; close one first");
    }
    let label = windows.next_label();
    let state = windows.get_or_create(&label)?;
    crate::paths::grant_read_all(app, &label, &pending, crate::paths::GrantSource::CommandLine);
    state.queue(pending);
    let built = crate::native_feel::build(
        WebviewWindowBuilder::new(app, &label, WebviewUrl::default())
            .title(DEFAULT_TITLE)
            .inner_size(1440.0, 900.0)
            .min_inner_size(1100.0, 680.0)
            .resizable(true)
            .focused(true),
    );
    if let Err(e) = built {
        windows.remove(&label);
        anyhow::bail!("could not open a window: {e}");
    }
    crate::window_scoped::created(app, &label);
    windows.note_focus(&label);
    log::info!(Windows = app.windows().len() as u64; "Window opened");
    changed(app);
    Ok(label)
}

/// Shows, restores and focuses a window.
pub fn reveal<R: Runtime>(app: &AppHandle<R>, label: &str) -> CmdResult<()> {
    let w = app.get_window(label).ok_or("this window was closed")?;
    show(&w);
    panel_windows::set_visible(app, label, true);
    crate::dialog_windows::set_visible(app, label, true);
    changed(app);
    Ok(())
}

fn show<R: Runtime>(w: &Window<R>) {
    let _ = w.show();
    let _ = w.unminimize();
    let _ = w.set_focus();
}

/// The window files and tray actions go to: the last focused one, else `main`, else any.
pub fn target<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let windows = app.try_state::<Windows>()?;
    windows
        .last_focused()
        .filter(|l| app.get_window(l).is_some())
        .or_else(|| app.get_window(MAIN).map(|_| MAIN.to_string()))
        .or_else(|| list(app).into_iter().next().map(|w| w.label))
}

/// Asks a window's user whether files from another app open there or in a new window.
pub fn ask_open<R: Runtime>(app: &AppHandle<R>, label: &str, paths: Vec<String>) {
    let names: Vec<String> = paths
        .iter()
        .map(|p| {
            std::path::Path::new(p)
                .file_name()
                .map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned())
        })
        .collect();
    match CurrentWindow::of(app, label) {
        Ok(win) => {
            win.set_pending_ask(paths);
            emit_to_window(
                app,
                label,
                OPEN_ASK_EVENT,
                serde_json::json!({ "count": names.len(), "names": names }),
            );
        }
        Err(e) => log::warn!("could not ask where to open files: {e}"),
    }
}

/// Quits Debrief; windows destroyed from now on keep their sessions for recovery.
pub fn quit<R: Runtime>(app: &AppHandle<R>) {
    begin_quit(app);
    app.exit(0);
}

/// What quitting does before the event loop ends: windows destroyed from now on keep their sessions.
fn begin_quit<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.try_state::<Windows>() {
        w.set_exiting();
    }
    log::info!(Windows = app.windows().len() as u64; "Quit requested");
}

/// Tells the user an import finished in a window that is not in front.
/// Called from commands and workers: the window getters run on the main thread ([`crate::ui_dispatch::run`]).
pub fn import_finished<R: Runtime>(app: &AppHandle<R>, label: &str, events: u64) {
    crate::names::after_import(app, label);
    let title = app
        .try_state::<Windows>()
        .and_then(|ws| ws.get(label))
        .map(|s| s.title())
        .unwrap_or_default();
    let place = if title.is_empty() {
        "a background window".to_string()
    } else {
        format!("\u{201c}{title}\u{201d}")
    };
    let (label, body) = (label.to_string(), format!("{events} events loaded in {place}."));
    crate::ui_dispatch::run(app, move |app| {
        let Some(w) = app.get_window(&label) else { return };
        let seen = w.is_focused().unwrap_or(false) && w.is_visible().unwrap_or(false);
        notify(app, NotificationKind::ImportFinished, seen, "Import finished", &body);
    });
}

/// A webview whose content process died is reloaded at most this often per window within
/// [`RELOAD_WINDOW`]; beyond that it is left as is (a page that crashes on load must not loop).
const MAX_RELOADS: usize = 3;
const RELOAD_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// Whether a window whose web content process terminated may be reloaded now (records the reload).
fn may_reload(history: &mut std::collections::HashMap<String, Vec<std::time::Instant>>, label: &str, now: std::time::Instant) -> bool {
    let times = history.entry(label.to_string()).or_default();
    times.retain(|t| now.duration_since(*t) < RELOAD_WINDOW);
    if times.len() >= MAX_RELOADS {
        return false;
    }
    times.push(now);
    true
}

/// The web content process of a window died (out of memory, a WebKit crash): logs it as a crash and
/// reloads the page, which restores the session from the autosave. Wired with
/// `Builder::on_web_content_process_terminate` (macOS/iOS only: Tauri 2 exposes no such event for
/// WebView2 or WebKitGTK).
#[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
pub fn web_content_terminated<R: Runtime>(webview: &tauri::Webview<R>) {
    let label = webview.label().to_string();
    crate::startup_watchdog::note_terminated(webview);
    content_terminated(&label, std::time::Instant::now(), || webview.reload().map_err(anyhow::Error::from));
}

fn content_terminated(label: &str, now: std::time::Instant, reload: impl FnOnce() -> anyhow::Result<()>) -> bool {
    static HISTORY: std::sync::Mutex<Option<std::collections::HashMap<String, Vec<std::time::Instant>>>> = std::sync::Mutex::new(None);
    let allowed = may_reload(
        HISTORY
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_or_insert_with(Default::default),
        label,
        now,
    );
    log::error!(Severity = "Fatal", Window = label, Reload = allowed; "The web content process of window {label} terminated");
    if !allowed {
        return false;
    }
    match reload() {
        Ok(()) => true,
        Err(e) => {
            log::error!(Window = label; "Could not reload window {label} after its web content process terminated: {e:#}");
            false
        }
    }
}

/// Applies the close decision; called for every window event.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    let app = window.app_handle();
    let label = window.label();
    crate::drag_ghost::on_window_event(window, event);
    match event {
        WindowEvent::CloseRequested { api, .. } if crate::dialog_windows::is_dialog(label) => {
            if !crate::dialog_windows::close_requested(app, label) {
                api.prevent_close();
            }
        }
        WindowEvent::Moved(_) | WindowEvent::Resized(_) if crate::dialog_windows::is_dialog(label) => {
            crate::dialog_windows::moved(app, window)
        }
        WindowEvent::CloseRequested { api, .. } => {
            if close_requested(app, window) != CloseDecision::Close {
                api.prevent_close();
            }
        }
        WindowEvent::Focused(true) if label != crate::drag_ghost::GHOST_LABEL => {
            if let Some(windows) = app.try_state::<Windows>() {
                // A pop-out panel stands for its host (files from other apps go there).
                windows.note_focus(panel_windows::resolve(label));
            }
            crate::notification_centre::on_focus(app, panel_windows::resolve(label));
        }
        WindowEvent::Moved(_) if panel_windows::is_popout(label) => {
            panel_windows::moved(app, label);
            panel_windows::dragged(app, label);
        }
        WindowEvent::Resized(_) if panel_windows::is_popout(label) => panel_windows::moved(app, label),
        // Dropped files are the user's choice: commands of this window may read them.
        WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
            crate::paths::grant_read_all(app, label, paths, crate::paths::GrantSource::Drop);
        }
        WindowEvent::Destroyed => destroyed(app, label),
        _ => {}
    }
}

/// Decides and applies what a close request does; the caller prevents the
/// close unless the decision is [`CloseDecision::Close`].
pub fn close_requested<R: Runtime>(app: &AppHandle<R>, window: &Window<R>) -> CloseDecision {
    let label = window.label();
    let Some(windows) = app.try_state::<Windows>() else {
        return CloseDecision::Close;
    };
    // Closing a pop-out panel window docks the panel back, closing a browser just closes it; the app is not affected.
    if windows.exiting() || is_auxiliary(label) {
        return CloseDecision::Close;
    }
    let other_visible = app
        .windows()
        .iter()
        .filter(|(l, w)| l.as_str() != label && !is_auxiliary(l) && w.is_visible().unwrap_or(false))
        .count();
    let pref = app.try_state::<SharedPrefs>().map(|p| p.get().on_close).unwrap_or_default();
    let state = windows.get(label);
    let asked = state.as_ref().is_some_and(|s| s.close_asked.load(Ordering::SeqCst));
    let decision = decide_close(pref, other_visible, crate::tray::available(app), asked);
    match decision {
        CloseDecision::Close => {}
        CloseDecision::Hide => {
            let _ = window.hide();
            panel_windows::set_visible(app, label, false);
            crate::dialog_windows::set_visible(app, label, false);
            log::info!("Last window hidden; Debrief keeps running in the tray");
            changed(app);
        }
        CloseDecision::Quit => quit(app),
        CloseDecision::Ask => {
            if let Some(s) = state {
                s.close_asked.store(true, Ordering::SeqCst);
            }
            emit_to_window(app, label, CLOSE_REQUESTED_EVENT, ());
        }
    }
    decision
}

/// A window is gone: free its workspace, stop its background work and, unless
/// Debrief is quitting, forget its session (a closed window is not restored).
pub fn destroyed<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if panel_windows::is_popout(label) {
        panel_windows::on_destroyed(app, label);
        return;
    }
    if crate::dialog_windows::is_dialog(label) {
        crate::dialog_windows::on_destroyed(app, label);
        return;
    }
    if label == crate::drag_ghost::GHOST_LABEL {
        return;
    }
    log::info!(Window = label; "Window destroyed");
    panel_windows::close_all_of(app, label);
    crate::dialog_windows::close_all_of(app, label);
    crate::browser::owner_destroyed(app, label);
    let Some(windows) = app.try_state::<Windows>() else { return };
    let exiting = windows.exiting();
    // Off the main thread and bounded (see `lifecycle::teardown_step`): dropping the state stops its live
    // tail, deactivates its plugins and frees the workspace; the registries notify the tray and windows.
    use crate::lifecycle::{teardown_step, TEARDOWN_STEP_LIMIT};
    let state = windows.remove(label);
    teardown_step("window state", TEARDOWN_STEP_LIMIT, move || drop(state));
    // Every feature that keeps state per window (see `window_scoped`).
    crate::window_scoped::destroyed(app, label);
    if !exiting {
        if let Some(life) = app.try_state::<crate::lifecycle::Lifecycle>() {
            life.forget_session(label);
        }
        changed(app);
    }
}

/// Opens a new, empty window and focuses it.
#[tauri::command]
pub async fn new_window<R: Runtime>(app: AppHandle<R>) -> CmdResult<WindowInfo> {
    let label = open_new(&app, Vec::new()).map_err(err)?;
    list(&app)
        .into_iter()
        .find(|w| w.label == label)
        .ok_or_else(|| "the window closed".into())
}

/// Quits Debrief: every window closes at once (File ▸ Exit, Debrief ▸ Quit Debrief). The UI asks about unsaved work in the
/// calling window first; other windows keep their autosaved sessions.
#[tauri::command]
pub async fn quit_app<R: Runtime>(app: AppHandle<R>) -> CmdResult<()> {
    quit(&app);
    Ok(())
}

/// Closes the calling window as its close button does (the close setting applies).
#[tauri::command]
pub async fn close_window<R: Runtime>(window: Window<R>) -> CmdResult<()> {
    crate::error::lift(window.close().map_err(err))
}

#[tauri::command]
pub async fn list_windows<R: Runtime>(app: AppHandle<R>) -> Vec<WindowInfo> {
    list(&app)
}

#[tauri::command]
pub async fn focus_window<R: Runtime>(app: AppHandle<R>, label: String) -> CmdResult<()> {
    reveal(&app, &label)
}

/// Sets the calling window's title (the UI shows the project name there).
#[tauri::command]
pub async fn set_window_title<R: Runtime>(app: AppHandle<R>, window: Window<R>, state: CurrentWindow, title: String) -> CmdResult<()> {
    let title = clean_title(&title);
    if state.title() == title {
        return Ok(());
    }
    window.set_title(&title).map_err(err)?;
    state.set_title(&title);
    changed(&app);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloseChoice {
    Quit,
    Tray,
}

/// The user's answer to `debrief://close-requested`; with `remember` it becomes the setting.
#[tauri::command]
pub async fn resolve_close<R: Runtime>(
    app: AppHandle<R>,
    window: Window<R>,
    state: CurrentWindow,
    choice: CloseChoice,
    remember: bool,
) -> CmdResult<()> {
    state.close_asked.store(false, Ordering::SeqCst);
    if remember {
        let mut prefs = app.try_state::<SharedPrefs>().map(|p| p.get()).unwrap_or_default();
        prefs.on_close = match choice {
            CloseChoice::Quit => OnClose::Quit,
            CloseChoice::Tray => OnClose::Tray,
        };
        // A failure to save must not keep the window from closing.
        match crate::settings::apply_section(&app, WINDOW_SECTION, prefs.to_json()) {
            Ok(r) if r.applied => {}
            Ok(_) => log::warn!("the close choice could not be remembered"),
            Err(e) => log::warn!("the close choice could not be remembered: {e}"),
        }
    }
    match choice {
        CloseChoice::Quit => quit(&app),
        CloseChoice::Tray if crate::tray::available(&app) => {
            window.hide().map_err(err)?;
            panel_windows::set_visible(&app, window.label(), false);
            changed(&app);
        }
        // Nothing to keep running in.
        CloseChoice::Tray => quit(&app),
    }
    Ok(())
}

/// The user's answer to `debrief://open-ask`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpenChoice {
    /// Open the waiting files in the asking window.
    Here,
    NewWindow,
    /// The dialog was dismissed: the waiting files are dropped.
    Cancel,
}

/// The user's answer to `debrief://open-ask`: open the waiting files in this window
/// or a new one, or drop them. The files are taken either way, so a later
/// answer opens nothing.
#[tauri::command]
pub async fn resolve_open<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, choice: OpenChoice) -> CmdResult<()> {
    let paths = state.take_pending_ask();
    if paths.is_empty() {
        return Ok(());
    }
    match choice {
        OpenChoice::Here => queue_open(&app, state.label(), paths),
        OpenChoice::NewWindow => {
            open_new(&app, paths).map_err(err)?;
        }
        OpenChoice::Cancel => log::info!(Files = paths.len() as u64; "Opening files from another app cancelled"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Now as _;
    use crate::test_support::{main_window, test_app, window};
    use std::sync::{Arc, Mutex};
    use tauri::Listener;

    #[test]
    fn terminated_web_content_is_logged_and_reloaded_without_looping() {
        let _g = crate::activity::test_capture();
        let t0 = std::time::Instant::now();
        let reloads = std::cell::Cell::new(0);
        for i in 0..5 {
            content_terminated("w-crashy", t0 + std::time::Duration::from_secs(i), || {
                reloads.set(reloads.get() + 1);
                Ok(())
            });
        }
        assert_eq!(reloads.get(), MAX_RELOADS);
        assert!(content_terminated("w-crashy", t0 + RELOAD_WINDOW * 2, || Ok(())));
        assert!(!content_terminated("w-broken", t0, || Err(anyhow::anyhow!("gone"))));
        let logged = crate::activity::captured();
        let crash = logged.iter().find(|c| c.message.contains("w-crashy terminated")).expect("logged");
        assert_eq!(
            (crash.props["Severity"].as_str(), crash.props["Window"].as_str()),
            ("Fatal", "w-crashy")
        );
        assert!(logged.iter().any(|c| c.message.contains("Could not reload window w-broken")));
    }

    #[test]
    fn close_decisions() {
        use CloseDecision::*;
        assert_eq!(decide_close(OnClose::Ask, 1, true, false), Close, "other windows remain");
        assert_eq!(decide_close(OnClose::Quit, 0, true, false), Quit);
        assert_eq!(decide_close(OnClose::Tray, 0, true, false), Hide);
        assert_eq!(decide_close(OnClose::Tray, 0, false, false), Quit, "no tray to keep running in");
        assert_eq!(decide_close(OnClose::Ask, 0, true, false), Ask);
        assert_eq!(decide_close(OnClose::Ask, 0, false, false), Quit);
        assert_eq!(decide_close(OnClose::Ask, 0, true, true), Quit, "closing again while asked");
    }

    #[test]
    fn titles_are_cleaned() {
        assert_eq!(clean_title("  Incident\n — Debrief "), "Incident — Debrief");
        assert_eq!(clean_title("\u{7}"), DEFAULT_TITLE);
        assert_eq!(clean_title(&"x".repeat(500)).len(), MAX_TITLE);
    }

    #[test]
    fn windows_open_list_reveal_and_free_their_workspace() {
        let app = test_app(None);
        let h = app.handle().clone();
        let main = window(&app, MAIN);
        let changes = Arc::new(Mutex::new(0));
        let c = changes.clone();
        main.listen(WINDOWS_EVENT, move |_| *c.lock().unwrap() += 1);

        let label = open_new(&h, vec!["/x".into()]).unwrap();
        assert_eq!(label, "w-2");
        assert_eq!(CurrentWindow::of(&h, &label).unwrap().take_pending(), ["/x"]);
        let info = tauri::async_runtime::block_on(new_window(h.clone())).unwrap();
        assert_eq!(info.label, "w-3");
        assert_eq!(info.title, DEFAULT_TITLE);
        let labels: Vec<String> = list_windows(h.clone()).now().into_iter().map(|w| w.label).collect();
        assert_eq!(labels, ["main", "w-2", "w-3"]);
        assert!(*changes.lock().unwrap() >= 2, "every window hears about new windows");
        assert_eq!(target(&h).as_deref(), Some("w-3"), "the newest window has the focus");
        focus_window(h.clone(), "w-2".into()).now().unwrap();
        assert!(focus_window(h.clone(), "w-99".into()).now().is_err());

        // Titles reach the window list.
        let w2 = window(&app, "w-2");
        set_window_title(
            h.clone(),
            w2.as_ref().window(),
            CurrentWindow::of(&h, "w-2").unwrap(),
            "Incident — Debrief".into(),
        )
        .now()
        .unwrap();
        set_window_title(
            h.clone(),
            w2.as_ref().window(),
            CurrentWindow::of(&h, "w-2").unwrap(),
            "Incident — Debrief".into(),
        )
        .now()
        .unwrap();
        assert!(list(&h).iter().any(|w| w.title == "Incident — Debrief"));

        // Destroying a window frees its state, its tasks and its session.
        let state = app.state::<Windows>().get("w-2").unwrap();
        let weak = Arc::downgrade(&state);
        drop(state);
        app.state::<crate::background::BackgroundTasks>()
            .register(
                crate::background::TaskSpec {
                    id: "live:w-2".into(),
                    label: "Live tail".into(),
                    kind: crate::background::TaskKind::LiveTail,
                    window: Some("w-2".into()),
                },
                None,
            )
            .unwrap();
        destroyed(&h, "w-2");
        assert!(weak.upgrade().is_none());
        assert_eq!(app.state::<crate::background::BackgroundTasks>().running(), 0);
        assert!(CurrentWindow::of(&h, "w-2").is_err());
        assert_eq!(target(&h).as_deref(), Some("w-3"), "the last focused window");
        // After quitting starts, sessions are kept.
        app.state::<Windows>().set_exiting();
        destroyed(&h, "w-3");
        assert_eq!(target(&h).as_deref(), Some(MAIN), "falls back to main");
    }

    #[test]
    fn open_ask_and_resolve_open() {
        let app = test_app(None);
        let h = app.handle().clone();
        let main = window(&app, MAIN);
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let a = asked.clone();
        main.listen(OPEN_ASK_EVENT, move |e| a.lock().unwrap().push(e.payload().to_string()));
        ask_open(&h, MAIN, vec!["/logs/a.log".into(), "/logs/b".into()]);
        let payload: serde_json::Value = serde_json::from_str(&asked.lock().unwrap()[0]).unwrap();
        assert_eq!(payload, serde_json::json!({ "count": 2, "names": ["a.log", "b"] }));
        // "This window": the paths are queued here.
        tauri::async_runtime::block_on(resolve_open(h.clone(), main_window(&app), OpenChoice::Here)).unwrap();
        assert_eq!(main_window(&app).take_pending(), ["/logs/a.log", "/logs/b"]);
        // Nothing waiting: nothing happens.
        tauri::async_runtime::block_on(resolve_open(h.clone(), main_window(&app), OpenChoice::NewWindow)).unwrap();
        assert_eq!(list(&h).len(), 1);
        // "New window": they go to a new window.
        ask_open(&h, MAIN, vec!["/logs/c".into()]);
        tauri::async_runtime::block_on(resolve_open(h.clone(), main_window(&app), OpenChoice::NewWindow)).unwrap();
        assert_eq!(CurrentWindow::of(&h, "w-2").unwrap().take_pending(), ["/logs/c"]);
        ask_open(&h, "gone", vec![]);
    }

    #[test]
    fn cancelling_the_open_ask_drops_the_waiting_files() {
        let app = test_app(None);
        let h = app.handle().clone();
        window(&app, MAIN);
        ask_open(&h, MAIN, vec!["/logs/a.log".into()]);
        tauri::async_runtime::block_on(resolve_open(h.clone(), main_window(&app), OpenChoice::Cancel)).unwrap();
        // A later answer (a stale dialog, a second click) opens nothing.
        tauri::async_runtime::block_on(resolve_open(h.clone(), main_window(&app), OpenChoice::Here)).unwrap();
        tauri::async_runtime::block_on(resolve_open(h.clone(), main_window(&app), OpenChoice::NewWindow)).unwrap();
        assert!(main_window(&app).take_pending().is_empty());
        assert_eq!(list(&h).len(), 1, "no new window");
        // A new ask after a cancel works as usual.
        ask_open(&h, MAIN, vec!["/logs/b".into()]);
        tauri::async_runtime::block_on(resolve_open(h.clone(), main_window(&app), OpenChoice::Here)).unwrap();
        assert_eq!(main_window(&app).take_pending(), ["/logs/b"]);
    }

    #[test]
    fn open_choices_parse_from_the_wire() {
        let parse = |s: &str| serde_json::from_value::<OpenChoice>(serde_json::json!(s));
        assert_eq!(parse("here").unwrap(), OpenChoice::Here);
        assert_eq!(parse("newWindow").unwrap(), OpenChoice::NewWindow);
        assert_eq!(parse("cancel").unwrap(), OpenChoice::Cancel);
        assert!(parse("true").is_err() && parse("Here").is_err());
        assert!(serde_json::from_value::<OpenChoice>(serde_json::json!(true)).is_err());
    }

    #[test]
    fn closing_follows_the_setting() {
        let app = test_app(None);
        let h = app.handle().clone();
        let main = window(&app, MAIN);
        let requested = Arc::new(Mutex::new(0));
        let r = requested.clone();
        main.listen(CLOSE_REQUESTED_EVENT, move |_| *r.lock().unwrap() += 1);
        crate::tray::set_available(&h, true);
        // The settings service applies process-wide plugin settings: one test at a time.
        let _serial = crate::settings::tests::serial();
        crate::window_prefs::register(&h).unwrap();
        let w = main.as_ref().window();
        // Ask (default): the window stays and its UI is asked.
        assert_eq!(close_requested(&h, &w), CloseDecision::Ask);
        assert_eq!(*requested.lock().unwrap(), 1);
        assert!(main_window(&app).close_asked.load(Ordering::SeqCst));
        // "Keep running in tray", remembered.
        resolve_close(h.clone(), w.clone(), main_window(&app), CloseChoice::Tray, true)
            .now()
            .unwrap();
        assert!(!main_window(&app).close_asked.load(Ordering::SeqCst));
        assert_eq!(app.state::<SharedPrefs>().get().on_close, OnClose::Tray);
        // Now closing hides without asking.
        assert_eq!(close_requested(&h, &w), CloseDecision::Hide);
        assert_eq!(*requested.lock().unwrap(), 1);
        // With another visible window, closing just closes.
        let other = open_new(&h, vec![]).unwrap();
        assert_eq!(close_requested(&h, &window(&app, &other).as_ref().window()), CloseDecision::Close);
        close_window(w.clone()).now().unwrap();
        // While quitting, every close goes through.
        app.state::<Windows>().set_exiting();
        assert_eq!(close_requested(&h, &w), CloseDecision::Close);
        // Focus is tracked.
        on_window_event(&main.as_ref().window(), &WindowEvent::Focused(true));
        assert_eq!(app.state::<Windows>().last_focused().as_deref(), Some(MAIN));
    }

    /// File ▸ Exit and Debrief ▸ Quit Debrief (`quit_app`): one step quits every window.
    #[test]
    fn quitting_keeps_the_sessions_of_the_windows_it_closes() {
        // (The mock runtime cannot exit: `app.exit` is the part not run here.)
        let app = crate::test_support::test_app_with_commands(None);
        crate::test_support::window(&app, "w-2");
        assert!(!app.state::<Windows>().exiting());
        begin_quit(app.handle());
        assert!(app.state::<Windows>().exiting(), "windows destroyed now keep their sessions");
    }
}
