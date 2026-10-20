//! Panels popped out of a window into OS windows of their own (like the tool
//! windows of VS Code or Visual Studio).
//!
//! A pop-out belongs to a host window: its label is `p-<host>-<n>`, it has no
//! workspace of its own ([`crate::windows::CurrentWindow`] resolves it to the
//! host's state, so every command works unchanged from inside it), it hears
//! the host's events ([`crate::windows::emit_to_window`] fans out), and it
//! never outlives the host. Closing a pop-out docks its panel back: the host
//! gets `imt://popout-closed` and puts the panel where it was.
//!
//! Commands: `popout_panel`, `dock_panel`, `list_popouts`, `set_popout_on_top`.
//! Events (to the host only): `imt://popout-closed`, `imt://popouts-changed`.

use crate::window_manager::clean_title;
use crate::windows::Windows;
use crate::{err, CmdResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder, Window};

/// Labels of pop-out windows: `p-<host>-<n>`.
pub const LABEL_PREFIX: &str = "p-";
/// More pop-outs per host window than this are refused.
pub const MAX_POPOUTS: usize = 12;
/// To the host: a pop-out is gone; dock its panel back. Payload: [`PopoutClosed`].
pub const POPOUT_CLOSED_EVENT: &str = "imt://popout-closed";
/// To the host: a pop-out opened, moved, resized or closed (persist the layout).
pub const POPOUTS_CHANGED_EVENT: &str = "imt://popouts-changed";
/// To the host: a pop-out window is being moved; where the cursor is. Payload: [`PopoutMoved`]. The host shows
/// its dock guides under the cursor and docks the panel when the window is dropped on one.
pub const POPOUT_MOVED_EVENT: &str = "imt://popout-moved";

const MAX_VIEW_ID: usize = 64;
const MAX_PARAMS_BYTES: usize = 4096;
const MIN_WIDTH: f64 = 240.0;
const MIN_HEIGHT: f64 = 160.0;
const MAX_SIZE: f64 = 16_384.0;
const MAX_COORD: f64 = 100_000.0;
const DEFAULT_SIZE: (f64, f64) = (560.0, 420.0);

/// The host of a pop-out label (`p-main-3` → `main`, `p-w-2-1` → `w-2`), or
/// `None` when the label is not a pop-out's.
pub fn host_of(label: &str) -> Option<&str> {
    let rest = label.strip_prefix(LABEL_PREFIX)?;
    let (host, n) = rest.rsplit_once('-')?;
    let numeric = !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
    // A pop-out never hosts pop-outs.
    (numeric && !host.is_empty() && !host.starts_with(LABEL_PREFIX)).then_some(host)
}

pub fn is_popout(label: &str) -> bool {
    host_of(label).is_some()
}

/// The window whose workspace a label works on: a pop-out's host, else the label itself.
pub fn resolve(label: &str) -> &str {
    host_of(label).unwrap_or(label)
}

pub fn popout_label(host: &str, n: u32) -> String {
    format!("{LABEL_PREFIX}{host}-{n}")
}

/// Is `id` a plausible view id (`timeline`, `query-results`, `profiling.flame`)?
pub fn valid_view_id(id: &str) -> bool {
    let mut bytes = id.bytes();
    id.len() <= MAX_VIEW_ID
        && bytes.next().is_some_and(|b| b.is_ascii_alphanumeric())
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// A window's place on the desktop in logical pixels (outer position, inner size).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    /// Bounds a window can be given: finite, at least a usable size, nothing absurd.
    /// `None` when a value is not a number. (Placement on a visible monitor is the UI's job.)
    pub fn sanitized(self) -> Option<Bounds> {
        if ![self.x, self.y, self.width, self.height].iter().all(|v| v.is_finite()) {
            return None;
        }
        Some(Bounds {
            x: self.x.clamp(-MAX_COORD, MAX_COORD),
            y: self.y.clamp(-MAX_COORD, MAX_COORD),
            width: self.width.clamp(MIN_WIDTH, MAX_SIZE),
            height: self.height.clamp(MIN_HEIGHT, MAX_SIZE),
        })
    }
}

/// What the UI asks for when popping a panel out.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PopoutRequest {
    pub view_id: String,
    /// Opaque view parameters, handed back to the pop-out through `list_popouts`.
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub bounds: Option<Bounds>,
    /// The panel title (the window title is "<title> — <host title>").
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub always_on_top: bool,
}

/// A pop-out as the UI sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PopoutInfo {
    pub label: String,
    pub host: String,
    pub view_id: String,
    pub title: String,
    pub params: Option<Value>,
    pub bounds: Option<Bounds>,
    /// Name of the monitor the window is on, when the platform says.
    pub monitor: Option<String>,
    pub always_on_top: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PopoutClosed {
    pub label: String,
    pub view_id: String,
}

/// A pop-out window moved: the cursor in logical screen coordinates.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PopoutMoved {
    pub label: String,
    pub view_id: String,
    pub x: f64,
    pub y: f64,
}

/// A physical screen position in logical pixels; `None` for a nonsensical scale or position.
pub fn to_logical(x: f64, y: f64, scale: f64) -> Option<(f64, f64)> {
    (x.is_finite() && y.is_finite() && scale.is_finite() && scale > 0.0).then(|| (x / scale, y / scale))
}

#[derive(Debug, Clone)]
struct Entry {
    host: String,
    view_id: String,
    title: String,
    params: Option<Value>,
    always_on_top: bool,
}

/// Which pop-outs exist, keyed by label. Managed as Tauri state.
#[derive(Default)]
pub struct Popouts {
    map: Mutex<BTreeMap<String, Entry>>,
    next: AtomicU32,
}

impl Popouts {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Entry>> {
        crate::sync::lock(&self.map)
    }

    /// The label of the pop-out showing `view_id` for `host`, if there is one.
    pub fn find(&self, host: &str, view_id: &str) -> Option<String> {
        self.lock()
            .iter()
            .find(|(_, e)| e.host == host && e.view_id == view_id)
            .map(|(l, _)| l.clone())
    }

    /// Reserves a label for a new pop-out of `host`; fails past [`MAX_POPOUTS`].
    fn reserve(&self, host: &str, entry: Entry) -> anyhow::Result<String> {
        let mut map = self.lock();
        if map.values().filter(|e| e.host == host).count() >= MAX_POPOUTS {
            anyhow::bail!("At most {MAX_POPOUTS} panels can be popped out of one window; dock one back first");
        }
        let label = popout_label(host, self.next.fetch_add(1, Ordering::SeqCst) + 1);
        map.insert(label.clone(), entry);
        Ok(label)
    }

    fn remove(&self, label: &str) -> Option<Entry> {
        self.lock().remove(label)
    }

    /// Labels of the host's pop-outs, oldest first.
    pub fn of_host(&self, host: &str) -> Vec<String> {
        let mut labels: Vec<String> = self.lock().iter().filter(|(_, e)| e.host == host).map(|(l, _)| l.clone()).collect();
        labels.sort_by_key(|l| l.rsplit_once('-').and_then(|(_, n)| n.parse::<u32>().ok()).unwrap_or(u32::MAX));
        labels
    }

    fn get(&self, label: &str) -> Option<Entry> {
        self.lock().get(label).cloned()
    }

    fn set_on_top(&self, label: &str, on: bool) {
        if let Some(e) = self.lock().get_mut(label) {
            e.always_on_top = on;
        }
    }
}

/// The URL a pop-out loads: the app, routed to the pop-out shell.
pub fn popout_url(view_id: &str, host: &str, label: &str) -> String {
    format!("index.html#/popout?view={view_id}&host={host}&label={label}")
}

fn info<R: Runtime>(app: &AppHandle<R>, label: &str, e: &Entry) -> PopoutInfo {
    let window = app.get_webview_window(label);
    let bounds = window.as_ref().and_then(|w| {
        let scale = w.scale_factor().ok()?;
        let pos = w.outer_position().ok()?.to_logical::<f64>(scale);
        let size = w.inner_size().ok()?.to_logical::<f64>(scale);
        Some(Bounds {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
        })
    });
    PopoutInfo {
        label: label.to_string(),
        host: e.host.clone(),
        view_id: e.view_id.clone(),
        title: e.title.clone(),
        params: e.params.clone(),
        bounds,
        monitor: window
            .and_then(|w| w.current_monitor().ok().flatten())
            .and_then(|m| m.name().cloned()),
        always_on_top: e.always_on_top,
    }
}

fn notify_changed<R: Runtime>(app: &AppHandle<R>, host: &str) {
    let _ = app.emit_to(host, POPOUTS_CHANGED_EVENT, ());
}

/// Opens a pop-out window for `request.view_id` belonging to `host`, or focuses
/// the one already showing it. The panel stays where it is when this fails.
pub fn open<R: Runtime>(app: &AppHandle<R>, host: &str, request: PopoutRequest) -> anyhow::Result<PopoutInfo> {
    if is_popout(host) {
        anyhow::bail!("a pop-out cannot host pop-outs");
    }
    if !valid_view_id(&request.view_id) {
        anyhow::bail!("not a view id: {:?}", request.view_id.chars().take(MAX_VIEW_ID).collect::<String>());
    }
    if request.params.as_ref().is_some_and(|p| p.to_string().len() > MAX_PARAMS_BYTES) {
        anyhow::bail!("the view parameters are larger than {MAX_PARAMS_BYTES} bytes");
    }
    let host_window = app
        .get_webview_window(host)
        .ok_or_else(|| anyhow::anyhow!("the window was closed"))?;
    let popouts = app.try_state::<Popouts>().ok_or_else(|| anyhow::anyhow!("IMT is not ready yet"))?;
    if let Some(existing) = popouts.find(host, &request.view_id) {
        if let Some(w) = app.get_webview_window(&existing) {
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
        let entry = popouts.get(&existing).ok_or_else(|| anyhow::anyhow!("the pop-out was closed"))?;
        return Ok(info(app, &existing, &entry));
    }
    let title = clean_title(request.title.as_deref().unwrap_or(&request.view_id));
    let host_title = app
        .try_state::<Windows>()
        .and_then(|w| w.get(host))
        .map(|s| s.title())
        .unwrap_or_default();
    let label = popouts.reserve(
        host,
        Entry {
            host: host.to_string(),
            view_id: request.view_id.clone(),
            title: title.clone(),
            params: request.params.clone(),
            always_on_top: request.always_on_top,
        },
    )?;
    let window_title = if host_title.is_empty() {
        title
    } else {
        clean_title(&format!("{title} \u{2014} {host_title}"))
    };
    let (x, y, width, height) = match request.bounds.and_then(Bounds::sanitized) {
        Some(b) => (b.x, b.y, b.width, b.height),
        None => fallback_place(&host_window),
    };
    let builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(popout_url(&request.view_id, host, &label).into()))
        .title(window_title)
        .inner_size(width, height)
        .position(x, y)
        .min_inner_size(MIN_WIDTH, MIN_HEIGHT)
        .resizable(true)
        .always_on_top(request.always_on_top)
        .focused(true);
    let built = crate::native_feel::build(frameless(builder));
    if let Err(e) = built {
        popouts.remove(&label);
        anyhow::bail!("could not open a window for the panel: {e}");
    }
    log::info!(Popout = label.as_str(), Host = host, View = request.view_id.as_str(); "Panel popped out");
    notify_changed(app, host);
    let entry = popouts.get(&label).ok_or_else(|| anyhow::anyhow!("the pop-out was closed"))?;
    Ok(info(app, &label, &entry))
}

/// Pop-outs draw IMT's own compact title bar (like VS / VS Code tool windows). macOS keeps the native traffic
/// lights (full screen, tiling, Mission Control) drawn over the web content; elsewhere the window has no native
/// frame (the shell draws minimise / maximise / close) but keeps the OS shadow and its resize border.
#[cfg(target_os = "macos")]
fn frameless<'a, R: Runtime, M: Manager<R>>(b: WebviewWindowBuilder<'a, R, M>) -> WebviewWindowBuilder<'a, R, M> {
    b.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true)
}

#[cfg(not(target_os = "macos"))]
fn frameless<'a, R: Runtime, M: Manager<R>>(b: WebviewWindowBuilder<'a, R, M>) -> WebviewWindowBuilder<'a, R, M> {
    b.decorations(false).shadow(true)
}

/// Right of the host's top-right corner, when the UI gave no bounds.
fn fallback_place<R: Runtime>(host: &tauri::WebviewWindow<R>) -> (f64, f64, f64, f64) {
    let (w, h) = DEFAULT_SIZE;
    let place = || {
        let scale = host.scale_factor().ok()?;
        let pos = host.outer_position().ok()?.to_logical::<f64>(scale);
        let size = host.outer_size().ok()?.to_logical::<f64>(scale);
        Some((pos.x + size.width - w - 40.0, pos.y + 80.0))
    };
    let (x, y) = place().unwrap_or((100.0, 100.0));
    (x, y, w, h)
}

/// A pop-out window is gone: forget it and, unless its host is gone too or IMT
/// is quitting, tell the host to dock the panel back.
pub fn on_destroyed<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let Some(entry) = app.try_state::<Popouts>().and_then(|p| p.remove(label)) else {
        return;
    };
    let exiting = app.try_state::<Windows>().is_some_and(|w| w.exiting());
    log::info!(Popout = label, Host = entry.host.as_str(); "Pop-out closed");
    if exiting || app.get_webview_window(&entry.host).is_none() {
        return;
    }
    let _ = app.emit_to(
        entry.host.as_str(),
        POPOUT_CLOSED_EVENT,
        PopoutClosed {
            label: label.to_string(),
            view_id: entry.view_id,
        },
    );
    notify_changed(app, &entry.host);
}

/// The host is gone: close its pop-outs (without docking anything back).
pub fn close_all_of<R: Runtime>(app: &AppHandle<R>, host: &str) -> usize {
    let Some(popouts) = app.try_state::<Popouts>() else { return 0 };
    let labels = popouts.of_host(host);
    for label in &labels {
        popouts.remove(label);
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.destroy();
        }
    }
    if !labels.is_empty() {
        log::info!(Host = host, Popouts = labels.len() as u64; "Host window closed; its pop-outs closed too");
    }
    labels.len()
}

/// Hides or shows the host's pop-outs with it (the host went to the tray, or came back).
pub fn set_visible<R: Runtime>(app: &AppHandle<R>, host: &str, visible: bool) {
    let Some(popouts) = app.try_state::<Popouts>() else { return };
    for label in popouts.of_host(host) {
        if let Some(w) = app.get_webview_window(&label) {
            let _ = if visible { w.show() } else { w.hide() };
        }
    }
}

/// A pop-out moved or was resized.
pub fn moved<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(host) = host_of(label) {
        notify_changed(app, host);
    }
}

/// A pop-out window is being moved (by its title bar or the shell's drag handle): tells its host where the
/// cursor is, so the host can show its dock guides and dock the panel when the window is dropped on one.
pub fn dragged<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let (Some(host), Some(popouts)) = (host_of(label), app.try_state::<Popouts>()) else {
        return;
    };
    let Some(entry) = popouts.get(label) else { return };
    let scale = app.get_webview_window(host).and_then(|w| w.scale_factor().ok()).unwrap_or(1.0);
    let Some((x, y)) = app.cursor_position().ok().and_then(|p| to_logical(p.x, p.y, scale)) else {
        return;
    };
    let _ = app.emit_to(
        host,
        POPOUT_MOVED_EVENT,
        PopoutMoved {
            label: label.to_string(),
            view_id: entry.view_id,
            x,
            y,
        },
    );
}

/// Pops a panel of the calling window out into a window of its own.
#[tauri::command]
pub async fn popout_panel<R: Runtime>(app: AppHandle<R>, window: Window<R>, request: PopoutRequest) -> CmdResult<PopoutInfo> {
    // Async: a window must not be created from a synchronous command (it deadlocks on Windows).
    crate::error::lift(crate::panel_windows::open(&app, resolve(window.label()), request).map_err(err))
}

/// Closes a pop-out of the calling window's group; its panel docks back into the host.
#[tauri::command]
pub async fn dock_panel<R: Runtime>(app: AppHandle<R>, window: Window<R>, popout_label: String) -> CmdResult<()> {
    if host_of(&popout_label) != Some(resolve(window.label())) {
        return Err("that is not a pop-out of this window".into());
    }
    let w = app.get_webview_window(&popout_label).ok_or("the pop-out was already closed")?;
    crate::error::lift(w.close().map_err(err))
}

/// The pop-outs of the calling window's group (the host and its pop-outs see the same list).
#[tauri::command]
pub async fn list_popouts<R: Runtime>(app: AppHandle<R>, window: Window<R>) -> Vec<PopoutInfo> {
    let host = resolve(window.label());
    let Some(popouts) = app.try_state::<Popouts>() else {
        return Vec::new();
    };
    popouts
        .of_host(host)
        .into_iter()
        .filter_map(|l| popouts.get(&l).map(|e| info(&app, &l, &e)))
        .collect()
}

/// Keeps the calling pop-out above other windows (or not).
#[tauri::command]
pub async fn set_popout_on_top<R: Runtime>(app: AppHandle<R>, window: Window<R>, on: bool) -> CmdResult<()> {
    let label = window.label();
    if !is_popout(label) {
        return Err("only a pop-out window can stay on top".into());
    }
    window.set_always_on_top(on).map_err(err)?;
    if let Some(p) = app.try_state::<Popouts>() {
        p.set_on_top(label, on);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Now as _;
    use crate::test_support::{test_app, test_app_with_commands, window};
    use crate::windows::{emit_to_window, CurrentWindow, MAIN};
    use std::sync::{Arc, Mutex};
    use tauri::test::MockRuntime;
    use tauri::Listener;

    fn request(view: &str) -> PopoutRequest {
        PopoutRequest {
            view_id: view.into(),
            params: None,
            bounds: Some(Bounds {
                x: 10.0,
                y: 20.0,
                width: 500.0,
                height: 300.0,
            }),
            title: Some("Inspector".into()),
            always_on_top: false,
        }
    }

    fn app() -> tauri::App<MockRuntime> {
        let app = test_app(None);
        app.manage(Popouts::default());
        window(&app, MAIN);
        app
    }

    fn recorder(w: &tauri::WebviewWindow<MockRuntime>, event: &str) -> Arc<Mutex<Vec<String>>> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        w.listen(event, move |e| s.lock().unwrap().push(e.payload().to_string()));
        seen
    }

    #[test]
    fn labels_parse_to_their_host() {
        assert_eq!(host_of("p-main-1"), Some("main"));
        assert_eq!(host_of("p-w-2-13"), Some("w-2"));
        assert_eq!(popout_label("w-3", 4), "p-w-3-4");
        assert_eq!(host_of(&popout_label("w-3", 4)), Some("w-3"));
        for not in [
            "main",
            "w-2",
            "p-",
            "p-main",
            "p-main-",
            "p--1",
            "p-main-x",
            "p-p-main-1-2",
            "x-main-1",
        ] {
            assert_eq!(host_of(not), None, "{not}");
        }
        assert_eq!(resolve("p-main-7"), "main");
        assert_eq!(resolve("w-2"), "w-2");
        assert!(is_popout("p-main-1") && !is_popout("main"));
    }

    #[test]
    fn view_ids_and_bounds_are_validated() {
        for ok in ["timeline", "query-results", "profiling.flame", "a_b", "x1"] {
            assert!(valid_view_id(ok), "{ok}");
        }
        for bad in ["", "-x", "a b", "a/b", "../x", "x?y=1", "a#b", &"x".repeat(65)] {
            assert!(!valid_view_id(bad), "{bad}");
        }
        let b = |x: f64, y: f64, width: f64, height: f64| Bounds { x, y, width, height };
        assert_eq!(b(1.0, 2.0, 3.0, 4.0).sanitized(), Some(b(1.0, 2.0, MIN_WIDTH, MIN_HEIGHT)));
        assert_eq!(
            b(1e9, -1e9, 1e9, 500.0).sanitized(),
            Some(b(MAX_COORD, -MAX_COORD, MAX_SIZE, 500.0))
        );
        assert_eq!(b(f64::NAN, 0.0, 500.0, 500.0).sanitized(), None);
        assert_eq!(b(0.0, 0.0, f64::INFINITY, 500.0).sanitized(), None);
        let parsed: PopoutRequest = serde_json::from_value(serde_json::json!({ "viewId": "events" })).unwrap();
        assert!(parsed.bounds.is_none() && !parsed.always_on_top);
        assert!(serde_json::from_value::<PopoutRequest>(serde_json::json!({ "viewId": "events", "evil": 1 })).is_err());
        assert_eq!(
            popout_url("events", "main", "p-main-1"),
            "index.html#/popout?view=events&host=main&label=p-main-1"
        );
    }

    #[test]
    fn popping_out_opens_a_window_of_the_host_group_and_refuses_bad_requests() {
        let app = app();
        let h = app.handle().clone();
        let changed = recorder(&window(&app, MAIN), POPOUTS_CHANGED_EVENT);
        let info = open(&h, MAIN, request("inspector")).unwrap();
        assert_eq!(info.label, "p-main-1");
        assert_eq!(
            (info.host.as_str(), info.view_id.as_str(), info.title.as_str()),
            ("main", "inspector", "Inspector")
        );
        assert!(h.get_webview_window("p-main-1").is_some(), "a real (mock) window");
        assert_eq!(changed.lock().unwrap().len(), 1);
        // Popping out the same view again focuses the existing window.
        assert_eq!(open(&h, MAIN, request("inspector")).unwrap().label, "p-main-1");
        let listed = list_popouts(h.clone(), window(&app, "p-main-1").as_ref().window()).now();
        assert_eq!(listed.len(), 1, "a pop-out sees its group's list");
        assert_eq!(
            list_popouts(h.clone(), window(&app, MAIN).as_ref().window()).now()[0].label,
            "p-main-1"
        );

        let mut bad = request("../etc");
        assert!(open(&h, MAIN, bad.clone()).is_err());
        bad.view_id = "events".into();
        bad.params = Some(serde_json::json!("x".repeat(MAX_PARAMS_BYTES + 1)));
        assert!(open(&h, MAIN, bad).unwrap_err().to_string().contains("bytes"));
        assert!(open(&h, "p-main-1", request("events"))
            .unwrap_err()
            .to_string()
            .contains("cannot host"));
        assert!(open(&h, "w-9", request("events")).unwrap_err().to_string().contains("closed"));
        let mut nobounds = request("events");
        nobounds.bounds = None;
        nobounds.title = None;
        let second = open(&h, MAIN, nobounds).unwrap();
        assert_eq!(second.title, "events");
    }

    #[test]
    fn at_most_twelve_popouts_per_host() {
        let app = app();
        let h = app.handle().clone();
        for i in 0..MAX_POPOUTS {
            open(&h, MAIN, request(&format!("view{i}"))).unwrap();
        }
        let e = open(&h, MAIN, request("one-too-many")).unwrap_err().to_string();
        assert!(e.contains("At most 12"), "{e}");
        assert_eq!(app.state::<Popouts>().of_host(MAIN).len(), MAX_POPOUTS);
        // Another window has a budget of its own.
        crate::window_manager::open_new(&h, vec![]).unwrap();
        assert!(open(&h, "w-2", request("view0")).is_ok());
        // Closing one frees a slot.
        on_destroyed(&h, "p-main-3");
        assert!(open(&h, MAIN, request("one-too-many")).is_ok());
    }

    #[test]
    fn closing_a_popout_docks_the_panel_back_into_the_host() {
        let app = app();
        let h = app.handle().clone();
        let closed = recorder(&window(&app, MAIN), POPOUT_CLOSED_EVENT);
        open(&h, MAIN, request("inspector")).unwrap();
        // The OS close button: the window event handler sees Destroyed.
        crate::window_manager::on_window_event(&window(&app, "p-main-1").as_ref().window(), &tauri::WindowEvent::Destroyed);
        let payload: Value = serde_json::from_str(&closed.lock().unwrap()[0]).unwrap();
        assert_eq!(payload, serde_json::json!({ "label": "p-main-1", "viewId": "inspector" }));
        assert!(app.state::<Popouts>().of_host(MAIN).is_empty());
        // The host's workspace is untouched by its pop-out closing.
        assert!(CurrentWindow::of(&h, MAIN).is_ok());
        // A second Destroyed (or an unknown label) does nothing.
        on_destroyed(&h, "p-main-1");
        on_destroyed(&h, "p-main-99");
        assert_eq!(closed.lock().unwrap().len(), 1);
        // While quitting nothing is docked back (the layout keeps the pop-out for next time).
        open(&h, MAIN, request("events")).unwrap();
        app.state::<Windows>().set_exiting();
        on_destroyed(&h, "p-main-2");
        assert_eq!(closed.lock().unwrap().len(), 1);
    }

    #[test]
    fn closing_the_host_closes_its_popouts_without_docking() {
        let app = app();
        let h = app.handle().clone();
        let other = crate::window_manager::open_new(&h, vec![]).unwrap();
        open(&h, MAIN, request("inspector")).unwrap();
        open(&h, MAIN, request("events")).unwrap();
        open(&h, &other, request("events")).unwrap();
        let closed = recorder(&window(&app, &other), POPOUT_CLOSED_EVENT);
        crate::window_manager::destroyed(&h, &other);
        assert!(app.state::<Popouts>().of_host(&other).is_empty(), "the host's pop-outs are gone");
        assert_eq!(app.state::<Popouts>().of_host(MAIN).len(), 2, "other hosts keep theirs");
        assert!(closed.lock().unwrap().is_empty());
        assert_eq!(close_all_of(&h, "nobody"), 0);
    }

    #[test]
    fn commands_from_a_popout_reach_the_host_workspace() {
        let app = app();
        let h = app.handle().clone();
        open(&h, MAIN, request("inspector")).unwrap();
        let host = CurrentWindow::of(&h, MAIN).unwrap();
        let from_popout = CurrentWindow::of(&h, "p-main-1").unwrap();
        assert!(Arc::ptr_eq(host.arc(), from_popout.arc()), "same workspace");
        assert_eq!(from_popout.label(), MAIN);
        assert!(
            app.state::<Windows>().get("p-main-1").is_none(),
            "a pop-out has no workspace of its own"
        );
        // A pop-out of a closed host resolves to nothing.
        crate::window_manager::destroyed(&h, MAIN);
        assert!(CurrentWindow::of(&h, "p-main-1").is_err());
    }

    #[test]
    fn invoking_a_command_from_a_popout_webview_uses_the_host_state() {
        use tauri::test::{get_ipc_response, INVOKE_KEY};
        use tauri::webview::InvokeRequest;
        let app = test_app_with_commands(None);
        app.manage(Popouts::default());
        window(&app, MAIN);
        let h = app.handle().clone();
        open(&h, MAIN, request("inspector")).unwrap();
        CurrentWindow::of(&h, MAIN).unwrap().set_title("Incident");
        let popout = window(&app, "p-main-1");
        let invoke = |cmd: &str, body: Value| {
            get_ipc_response(
                &popout,
                InvokeRequest {
                    cmd: cmd.into(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: crate::test_support::ipc_url(),
                    body: tauri::ipc::InvokeBody::Json(body),
                    headers: Default::default(),
                    invoke_key: INVOKE_KEY.to_string(),
                },
            )
            .map(|r| match r {
                tauri::ipc::InvokeResponseBody::Json(s) => serde_json::from_str::<Value>(&s).unwrap(),
                tauri::ipc::InvokeResponseBody::Raw(_) => Value::Null,
            })
        };
        let summary = invoke("workspace_summary", serde_json::json!({})).unwrap();
        assert!(summary.get("total").is_some(), "{summary}");
        assert!(
            app.state::<Windows>().get("p-main-1").is_none(),
            "no state was created for the pop-out"
        );
        let listed = invoke("list_popouts", serde_json::json!({})).unwrap();
        assert_eq!(listed[0]["viewId"], "inspector");
        invoke("set_popout_on_top", serde_json::json!({ "on": true })).unwrap();
        assert!(list_popouts(h.clone(), window(&app, MAIN).as_ref().window()).now()[0].always_on_top);
        assert!(set_popout_on_top(h.clone(), window(&app, MAIN).as_ref().window(), true)
            .now()
            .is_err());
        // Docking: only pop-outs of the caller's own group.
        assert!(dock_panel(h.clone(), window(&app, MAIN).as_ref().window(), "p-w-9-1".into())
            .now()
            .is_err());
        assert!(dock_panel(h.clone(), window(&app, MAIN).as_ref().window(), "main".into())
            .now()
            .is_err());
        assert!(dock_panel(h.clone(), window(&app, MAIN).as_ref().window(), "p-main-1".into())
            .now()
            .is_ok());
        assert!(dock_panel(h.clone(), window(&app, MAIN).as_ref().window(), "p-main-5".into())
            .now()
            .is_err());
    }

    #[test]
    fn host_events_fan_out_to_its_popouts_except_host_only_ones() {
        let app = app();
        let h = app.handle().clone();
        let other = crate::window_manager::open_new(&h, vec![]).unwrap();
        open(&h, MAIN, request("inspector")).unwrap();
        open(&h, &other, request("inspector")).unwrap();
        let host = recorder(&window(&app, MAIN), "imt://test");
        let mine = recorder(&window(&app, "p-main-1"), "imt://test");
        let theirs = recorder(&window(&app, "p-w-2-2"), "imt://test");
        emit_to_window(&h, MAIN, "imt://test", 1);
        assert_eq!(host.lock().unwrap().len(), 1);
        assert_eq!(mine.lock().unwrap().len(), 1, "the pop-out hears its host's events");
        assert!(theirs.lock().unwrap().is_empty(), "other groups do not");
        // Host-only events (files to open, close prompts) stay with the host.
        let open_main = recorder(&window(&app, MAIN), crate::windows::OPEN_EVENT);
        let open_popout = recorder(&window(&app, "p-main-1"), crate::windows::OPEN_EVENT);
        emit_to_window(&h, MAIN, crate::windows::OPEN_EVENT, ());
        assert_eq!((open_main.lock().unwrap().len(), open_popout.lock().unwrap().len()), (1, 0));
        // Moves of a pop-out tell its host.
        let changed = recorder(&window(&app, MAIN), POPOUTS_CHANGED_EVENT);
        crate::window_manager::on_window_event(
            &window(&app, "p-main-1").as_ref().window(),
            &tauri::WindowEvent::Moved(Default::default()),
        );
        assert_eq!(changed.lock().unwrap().len(), 1);
        moved(&h, "main");
        assert_eq!(changed.lock().unwrap().len(), 1);
    }

    #[test]
    fn moving_a_popout_tells_its_host_where_the_cursor_is() {
        let app = app();
        let h = app.handle().clone();
        open(&h, MAIN, request("inspector")).unwrap();
        let seen = recorder(&window(&app, MAIN), POPOUT_MOVED_EVENT);
        crate::window_manager::on_window_event(
            &window(&app, "p-main-1").as_ref().window(),
            &tauri::WindowEvent::Moved(Default::default()),
        );
        let payload: Value = serde_json::from_str(&seen.lock().unwrap()[0]).unwrap();
        assert_eq!(
            (payload["label"].as_str(), payload["viewId"].as_str()),
            (Some("p-main-1"), Some("inspector"))
        );
        assert!(payload["x"].is_number() && payload["y"].is_number());
        // Resizing is not a move; unknown and non-pop-out labels are ignored.
        crate::window_manager::on_window_event(
            &window(&app, "p-main-1").as_ref().window(),
            &tauri::WindowEvent::Resized(Default::default()),
        );
        dragged(&h, "p-main-9");
        dragged(&h, MAIN);
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(to_logical(200.0, 100.0, 2.0), Some((100.0, 50.0)));
        assert_eq!(to_logical(f64::NAN, 1.0, 1.0), None);
        assert_eq!(to_logical(1.0, 1.0, 0.0), None);
    }

    #[test]
    fn popouts_are_not_app_windows() {
        let app = app();
        let h = app.handle().clone();
        open(&h, MAIN, request("inspector")).unwrap();
        let labels: Vec<String> = crate::window_manager::list(&h).into_iter().map(|w| w.label).collect();
        assert_eq!(labels, ["main"], "the tray and Window menu list app windows only");
        // Closing a pop-out always just closes it (no tray/quit question).
        let w = window(&app, "p-main-1");
        assert_eq!(
            crate::window_manager::close_requested(&h, &w.as_ref().window()),
            crate::window_manager::CloseDecision::Close
        );
        // Focusing a pop-out makes its host the target for files from other apps.
        crate::window_manager::on_window_event(&w.as_ref().window(), &tauri::WindowEvent::Focused(true));
        assert_eq!(crate::window_manager::target(&h).as_deref(), Some(MAIN));
    }
}
