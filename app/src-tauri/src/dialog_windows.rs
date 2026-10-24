//! Dialogs as real OS windows (ADR 0050): every dialog of Debrief (setup wizards, Settings, managers, About,
//! export and share wizards, pickers, confirmations that need more than a message box) is a window of its own,
//! owned by the Debrief window that opened it.
//!
//! A dialog belongs to a host window: its label is `d-<host>-<n>`, it has no workspace of its own
//! ([`crate::panel_windows::resolve`] gives it the host's, so every command works unchanged from inside it), it
//! hears the host's events and never outlives the host. One dialog window exists per dialog id and host: opening
//! it again focuses the existing one. A dialog can open dialogs of its own (a confirmation from Settings); the
//! answer goes to the window that opened it.
//!
//! * **Modal** dialogs disable their parent while they are open (owner window on Windows, child window on macOS,
//!   transient window on Linux; the UI also puts a scrim over the parent) and stay above it. On macOS the parent is
//!   NOT disabled natively: tao implements that as a sheet on the parent, and a child window of a parent that carries a
//!   sheet is never key, so the dialog showed greyed out and the app froze. There the UI scrim blocks the parent.
//! * **Modeless** dialogs are owned too (they stay above the parent and minimise with it) but the parent stays usable.
//!   Except on macOS, where a child window is confined to its parent's screen and vanishes when dragged to another
//!   display: modeless dialogs are independent windows there (see [`Parenting`]).
//!
//! Data never travels through window globals: the input is validated and parked here at open
//! (`dialog_take_input` hands it to the dialog's own webview) and the result goes back through `dialog_close`
//! as the `debrief://dialog-result` event of the host.
//!
//! Commands: `dialog_open`, `dialog_take_input`, `dialog_close`.
//! Events: `debrief://dialog-result`, `debrief://dialog-closed` (to the host), `debrief://dialog-close-requested` (to the dialog).

use crate::panel_windows::{valid_view_id, Bounds};
use crate::window_manager::clean_title;
use crate::{err, CmdResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder, Window};

/// Labels of dialog windows: `d-<host>-<n>`.
pub const LABEL_PREFIX: &str = "d-";
/// More dialog windows per host than this are refused.
pub const MAX_DIALOGS: usize = 8;
/// To the window that opened the dialog: it answered. Payload: [`DialogResult`].
pub const RESULT_EVENT: &str = "debrief://dialog-result";
/// To the window that opened the dialog: it is gone (with or without a result). Payload: [`DialogClosed`].
pub const CLOSED_EVENT: &str = "debrief://dialog-closed";
/// To the dialog: the user asked to close it (title bar, Cmd+W); the dialog decides and calls `dialog_close`.
pub const CLOSE_REQUESTED_EVENT: &str = "debrief://dialog-close-requested";

/// The largest input a dialog is opened with, and the largest result it answers with.
pub const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RESULT_BYTES: usize = 1024 * 1024;
const MIN_SIZE: (f64, f64) = (320.0, 200.0);
const MAX_SIZE: f64 = 4096.0;
const DEFAULT_SIZE: (f64, f64) = (640.0, 520.0);
/// A dialog that did not answer a close request this long is closed anyway (a hung webview must not trap the user).
const CLOSE_GRACE: Duration = Duration::from_millis(1500);

/// How a dialog window is attached to its parent natively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parenting {
    /// Owned / child / transient window: above the parent, minimised with it.
    Owned,
    /// An independent window (it can be dragged to any display).
    Independent,
}

impl Parenting {
    /// The parenting of a dialog of `modality` on the OS `os` (`std::env::consts::OS`).
    pub fn of(modality: Modality, os: &str) -> Parenting {
        match (os, modality) {
            ("macos", Modality::Modeless) => Parenting::Independent,
            _ => Parenting::Owned,
        }
    }
}

/// Whether opening a modal dialog disables its parent window natively (`set_enabled(false)`). Not on macOS, where that
/// begins a sheet on the parent and blocks the dialog's own window from becoming key.
pub fn disables_parent_natively(os: &str) -> bool {
    os != "macos"
}

/// Whether the dialog blocks its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modality {
    Modal,
    Modeless,
}

/// The host of a dialog label (`d-main-3` -> `main`, `d-w-2-1` -> `w-2`), or `None`.
pub fn host_of(label: &str) -> Option<&str> {
    let rest = label.strip_prefix(LABEL_PREFIX)?;
    let (host, n) = rest.rsplit_once('-')?;
    let numeric = !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
    (numeric && !host.is_empty() && !host.starts_with(LABEL_PREFIX) && !host.starts_with(crate::panel_windows::LABEL_PREFIX))
        .then_some(host)
}

pub fn is_dialog(label: &str) -> bool {
    host_of(label).is_some()
}

/// What the UI asks for when opening a dialog.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DialogRequest {
    pub dialog_id: String,
    /// Tells apart the windows of a dialog that is open once per input (a hash of the input; letters and digits).
    #[serde(default)]
    pub instance: Option<String>,
    pub title: String,
    pub modality: Modality,
    /// A modal whose answer cannot be skipped (it has no Cancel): Escape and a close request leave it open.
    #[serde(default)]
    pub required_choice: bool,
    /// The typed input of the dialog, handed to its webview through `dialog_take_input`.
    #[serde(default)]
    pub input: Option<Value>,
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
    #[serde(default)]
    pub min_width: Option<f64>,
    #[serde(default)]
    pub min_height: Option<f64>,
    /// Remembered bounds (already on a visible monitor, the UI's job); else the dialog is centred on its parent.
    #[serde(default)]
    pub bounds: Option<Bounds>,
}

/// A dialog window as the UI sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogInfo {
    pub label: String,
    pub dialog_id: String,
    /// The window existed already and was focused.
    pub existing: bool,
}

/// What a dialog's own webview needs to render.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogStart {
    pub dialog_id: String,
    pub title: String,
    pub modality: Modality,
    pub input: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogResult {
    pub label: String,
    pub dialog_id: String,
    pub result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogClosed {
    pub label: String,
    pub dialog_id: String,
    /// Where the window was when it closed (logical px), for remembering.
    pub bounds: Option<Bounds>,
}

#[derive(Debug, Clone)]
struct Entry {
    host: String,
    parent: String,
    dialog_id: String,
    instance: Option<String>,
    title: String,
    modality: Modality,
    required_choice: bool,
    input: Option<Value>,
    bounds: Option<Bounds>,
    asked: Option<Instant>,
    /// The window watchdog found its page not answering: nobody can make the required choice, so the OS close may.
    unresponsive: bool,
}

/// Which dialog windows exist, keyed by label. Managed as Tauri state.
#[derive(Default)]
pub struct Dialogs {
    map: Mutex<BTreeMap<String, Entry>>,
    next: AtomicU32,
}

impl Dialogs {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Entry>> {
        crate::sync::lock(&self.map)
    }

    /// The window of `dialog_id` opened by `parent`, if there is one (the parent is the one waiting for the answer).
    fn find(&self, parent: &str, dialog_id: &str, instance: Option<&str>) -> Option<String> {
        self.lock()
            .iter()
            .find(|(_, e)| e.parent == parent && e.dialog_id == dialog_id && e.instance.as_deref() == instance)
            .map(|(l, _)| l.clone())
    }

    fn reserve(&self, entry: Entry) -> anyhow::Result<String> {
        let mut map = self.lock();
        if map.values().filter(|e| e.host == entry.host).count() >= MAX_DIALOGS {
            anyhow::bail!("At most {MAX_DIALOGS} dialogs can be open at once; close one first");
        }
        let label = format!("{LABEL_PREFIX}{}-{}", entry.host, self.next.fetch_add(1, Ordering::SeqCst) + 1);
        map.insert(label.clone(), entry);
        Ok(label)
    }

    fn remove(&self, label: &str) -> Option<Entry> {
        self.lock().remove(label)
    }

    fn get(&self, label: &str) -> Option<Entry> {
        self.lock().get(label).cloned()
    }

    /// Labels of the host's dialogs.
    pub fn of_host(&self, host: &str) -> Vec<String> {
        self.lock().iter().filter(|(_, e)| e.host == host).map(|(l, _)| l.clone()).collect()
    }

    fn modal_children_of(&self, parent: &str) -> usize {
        self.lock()
            .values()
            .filter(|e| e.parent == parent && e.modality == Modality::Modal)
            .count()
    }
}

/// The URL a dialog loads: the app, routed to the dialog shell.
pub fn dialog_url(dialog_id: &str, host: &str, label: &str) -> String {
    format!("index.html#/dialog?id={dialog_id}&host={host}&label={label}")
}

/// A size within what a window can sensibly be; `None` for a value that is not a number.
fn clamp_size(v: Option<f64>, min: f64, default: f64) -> f64 {
    match v {
        Some(v) if v.is_finite() => v.clamp(min, MAX_SIZE),
        _ => default.max(min),
    }
}

/// The validated window geometry of a request: (width, height, min width, min height).
pub fn geometry(r: &DialogRequest) -> (f64, f64, f64, f64) {
    let min_w = clamp_size(r.min_width, MIN_SIZE.0, MIN_SIZE.0);
    let min_h = clamp_size(r.min_height, MIN_SIZE.1, MIN_SIZE.1);
    (
        clamp_size(r.width, min_w, DEFAULT_SIZE.0),
        clamp_size(r.height, min_h, DEFAULT_SIZE.1),
        min_w,
        min_h,
    )
}

/// Validates a request: a plausible dialog id, a title, an input within the cap.
pub fn validate(r: &DialogRequest) -> anyhow::Result<()> {
    if !valid_view_id(&r.dialog_id) {
        anyhow::bail!("not a dialog id: {:?}", r.dialog_id.chars().take(64).collect::<String>());
    }
    if r.instance
        .as_ref()
        .is_some_and(|i| i.is_empty() || i.len() > 16 || !i.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        anyhow::bail!("not an instance key");
    }
    if r.input.as_ref().is_some_and(|i| i.to_string().len() > MAX_INPUT_BYTES) {
        anyhow::bail!("the dialog input is larger than {MAX_INPUT_BYTES} bytes");
    }
    Ok(())
}

/// Centred on the parent (logical px), or at `fallback` when the parent's place is unknown.
pub fn centred(parent: Option<Bounds>, width: f64, height: f64) -> (f64, f64) {
    match parent {
        Some(p) => (p.x + (p.width - width) / 2.0, (p.y + (p.height - height) / 3.0).max(p.y)),
        None => (100.0, 100.0),
    }
}

fn bounds_of<R: Runtime>(w: &Window<R>) -> Option<Bounds> {
    let scale = w.scale_factor().ok()?;
    let pos = w.outer_position().ok()?.to_logical::<f64>(scale);
    let size = w.inner_size().ok()?.to_logical::<f64>(scale);
    Some(Bounds {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
    })
}

/// Opens the dialog `request.dialog_id` for the window `caller` (a Debrief window or one of its dialogs), or
/// focuses the one already open for its host.
pub fn open<R: Runtime>(app: &AppHandle<R>, caller: &str, request: DialogRequest) -> anyhow::Result<DialogInfo> {
    validate(&request)?;
    let host = crate::panel_windows::resolve(caller).to_string();
    let dialogs = app
        .try_state::<Dialogs>()
        .ok_or_else(|| anyhow::anyhow!("Debrief is not ready yet"))?;
    if let Some(existing) = dialogs.find(caller, &request.dialog_id, request.instance.as_deref()) {
        if let Some(w) = app.get_window(&existing) {
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
        return Ok(DialogInfo {
            label: existing,
            dialog_id: request.dialog_id,
            existing: true,
        });
    }
    let parent_window = app
        .get_webview_window(caller)
        .or_else(|| app.get_webview_window(&host))
        .ok_or_else(|| anyhow::anyhow!("the window was closed"))?;
    let title = clean_title(&request.title);
    let label = dialogs.reserve(Entry {
        host: host.clone(),
        parent: parent_window.label().to_string(),
        dialog_id: request.dialog_id.clone(),
        instance: request.instance.clone(),
        title: title.clone(),
        modality: request.modality,
        required_choice: request.required_choice && request.modality == Modality::Modal,
        input: request.input.clone(),
        bounds: None,
        asked: None,
        unresponsive: false,
    })?;
    let (w, h, min_w, min_h) = geometry(&request);
    let (x, y) = match request.bounds.and_then(Bounds::sanitized) {
        Some(b) => (b.x, b.y),
        None => centred(bounds_of(&parent_window.as_ref().window()), w, h),
    };
    let (w, h) = match request.bounds.and_then(Bounds::sanitized) {
        Some(b) => (b.width.max(min_w), b.height.max(min_h)),
        // A default size is bounded by the parent's screen, so a tall dialog never opens larger than the display.
        None => within_screen((w, h), (min_w, min_h), parent_monitor_size(&parent_window)),
    };
    let built = build_window(
        app,
        &label,
        &host,
        &request.dialog_id,
        &title,
        (x, y, w, h),
        (min_w, min_h),
        &parent_window,
        request.modality,
    );
    if let Err(e) = built {
        dialogs.remove(&label);
        anyhow::bail!("could not open the dialog window: {e}");
    }
    if request.modality == Modality::Modal && disables_parent_natively(std::env::consts::OS) {
        let _ = parent_window.set_enabled(false);
    }
    log::info!(Dialog = request.dialog_id.as_str(), Host = host.as_str(); "Dialog window opened");
    Ok(DialogInfo {
        label,
        dialog_id: request.dialog_id,
        existing: false,
    })
}

/// The share of the screen a dialog opens at most (unless the user resized it before).
const SCREEN_SHARE: f64 = 0.75;

/// `size` limited to [`SCREEN_SHARE`] of `screen` (logical px), never below `min`; unchanged when the screen is unknown.
pub fn within_screen(size: (f64, f64), min: (f64, f64), screen: Option<(f64, f64)>) -> (f64, f64) {
    match screen {
        Some((sw, sh)) if sw.is_finite() && sh.is_finite() => {
            (size.0.min(sw * SCREEN_SHARE).max(min.0), size.1.min(sh * SCREEN_SHARE).max(min.1))
        }
        _ => size,
    }
}

fn parent_monitor_size<R: Runtime>(w: &tauri::WebviewWindow<R>) -> Option<(f64, f64)> {
    let m = w.current_monitor().ok().flatten()?;
    let s = m.size().to_logical::<f64>(m.scale_factor());
    Some((s.width, s.height))
}

/// The builder configuration of a dialog window, separate from the build so a test can check it without a window.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowConfig {
    pub url: String,
    pub title: String,
    pub inner: (f64, f64),
    pub position: (f64, f64),
    pub min_inner: (f64, f64),
    pub resizable: bool,
    pub owned: bool,
    /// The native close button: none (see [`title_buttons`]).
    pub closable: bool,
    /// The native minimise button: none.
    pub minimizable: bool,
    /// The native zoom / maximise button: none.
    pub maximizable: bool,
}

/// The title-bar buttons of a dialog window: (closable, minimizable, maximizable). None: every dialog, modal or
/// modeless, closes from its content (a Close or Cancel button, Escape, Cmd/Ctrl+W), and a window-manager close
/// request reaches the dialog as its own Close/Cancel ([`close_requested`]). Modeless windows stay resizable and movable.
pub fn title_buttons(_modality: Modality) -> (bool, bool, bool) {
    (false, false, false)
}

pub fn window_config(
    label: &str,
    host: &str,
    dialog_id: &str,
    title: &str,
    place: (f64, f64, f64, f64),
    min: (f64, f64),
    modality: Modality,
) -> WindowConfig {
    let (closable, minimizable, maximizable) = title_buttons(modality);
    WindowConfig {
        closable,
        minimizable,
        maximizable,
        url: dialog_url(dialog_id, host, label),
        title: title.to_string(),
        inner: (place.2, place.3),
        position: (place.0, place.1),
        min_inner: min,
        resizable: true,
        owned: true,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_window<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
    host: &str,
    dialog_id: &str,
    title: &str,
    place: (f64, f64, f64, f64),
    min: (f64, f64),
    parent: &tauri::WebviewWindow<R>,
    modality: Modality,
) -> anyhow::Result<()> {
    let c = window_config(label, host, dialog_id, title, place, min, modality);
    // A native title bar with the dialog's title: dialogs are ordinary OS windows.
    let builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App(c.url.clone().into()))
        .title(c.title.clone())
        .inner_size(c.inner.0, c.inner.1)
        .position(c.position.0, c.position.1)
        .min_inner_size(c.min_inner.0, c.min_inner.1)
        .resizable(c.resizable)
        .closable(c.closable)
        .minimizable(c.minimizable)
        .maximizable(c.maximizable)
        .focused(true);
    let builder = if Parenting::of(modality, std::env::consts::OS) == Parenting::Owned {
        owned_by(builder, parent)?
    } else {
        builder
    };
    crate::native_feel::build(builder).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

/// An owned window of `owner`: above it, minimised and hidden with it (Windows: owner; macOS: child; Linux: transient).
#[cfg(not(test))]
fn owned_by<'a, R: Runtime, M: Manager<R>>(
    b: WebviewWindowBuilder<'a, R, M>,
    owner: &tauri::WebviewWindow<R>,
) -> anyhow::Result<WebviewWindowBuilder<'a, R, M>> {
    b.parent(owner).map_err(|e| anyhow::anyhow!("{e}"))
}

/// The mock runtime has no native windows to own: tests check the bookkeeping only.
#[cfg(test)]
fn owned_by<'a, R: Runtime, M: Manager<R>>(
    b: WebviewWindowBuilder<'a, R, M>,
    _owner: &tauri::WebviewWindow<R>,
) -> anyhow::Result<WebviewWindowBuilder<'a, R, M>> {
    Ok(b)
}

/// The user (or the OS: Cmd/Ctrl+W, a window-manager close) asked to close a dialog window. The window itself has no
/// close button when modal, so a request on a modal means Cancel: the dialog decides (unsaved edits), and one that does
/// not answer in time is closed anyway. A required-choice modal ignores it. Returns whether the close may go ahead now.
pub fn close_requested<R: Runtime>(app: &AppHandle<R>, label: &str) -> bool {
    let Some(dialogs) = app.try_state::<Dialogs>() else { return true };
    let mut map = dialogs.lock();
    let Some(entry) = map.get_mut(label) else { return true };
    if entry.required_choice {
        // No Cancel to map the request to: the dialog stays until it is answered, unless its page cannot answer.
        return entry.unresponsive;
    }
    let now = Instant::now();
    if entry.asked.is_some_and(|t| now.duration_since(t) >= CLOSE_GRACE) {
        return true;
    }
    entry.asked = Some(now);
    drop(map);
    let _ = app.emit_to(label, CLOSE_REQUESTED_EVENT, ());
    false
}

/// The window watchdog (ADR 0058) found a dialog's page not answering, or answering again. A required-choice dialog
/// whose page hangs can then be closed by the OS close (rule 19: the way out never depends on the hung page).
// TODO(integration): the startup watchdog of fix/blank-start calls this when it decides a dialog page is unresponsive.
#[cfg_attr(not(test), allow(dead_code))]
pub fn page_unresponsive<R: Runtime>(app: &AppHandle<R>, label: &str, unresponsive: bool) {
    if let Some(d) = app.try_state::<Dialogs>() {
        if let Some(e) = d.lock().get_mut(label) {
            e.unresponsive = unresponsive;
        }
    }
}

/// A dialog moved or was resized: remember where it is.
pub fn moved<R: Runtime>(app: &AppHandle<R>, window: &Window<R>) {
    if let Some(d) = app.try_state::<Dialogs>() {
        if let Some(e) = d.lock().get_mut(window.label()) {
            e.bounds = bounds_of(window);
        }
    }
}

/// A dialog window is gone: forget it, re-enable and refocus its parent, tell the host.
pub fn on_destroyed<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let Some(entry) = app.try_state::<Dialogs>().and_then(|d| d.remove(label)) else {
        return;
    };
    log::info!(Dialog = entry.dialog_id.as_str(), Host = entry.host.as_str(); "Dialog window closed");
    if app.try_state::<crate::windows::Windows>().is_some_and(|w| w.exiting()) {
        return;
    }
    let dialogs = app.state::<Dialogs>();
    if let Some(parent) = app.get_webview_window(&entry.parent) {
        if entry.modality == Modality::Modal
            && disables_parent_natively(std::env::consts::OS)
            && dialogs.modal_children_of(&entry.parent) == 0
        {
            let _ = parent.set_enabled(true);
        }
        let _ = parent.set_focus();
    }
    if app.get_window(&entry.parent).is_some() {
        let _ = app.emit_to(
            entry.parent.as_str(),
            CLOSED_EVENT,
            DialogClosed {
                label: label.to_string(),
                dialog_id: entry.dialog_id,
                bounds: entry.bounds,
            },
        );
    }
}

/// The host is gone: close its dialogs.
pub fn close_all_of<R: Runtime>(app: &AppHandle<R>, host: &str) -> usize {
    let Some(dialogs) = app.try_state::<Dialogs>() else { return 0 };
    let labels = dialogs.of_host(host);
    for label in &labels {
        dialogs.remove(label);
        if let Some(w) = app.get_window(label) {
            let _ = w.destroy();
        }
    }
    labels.len()
}

/// Hides or shows the host's dialogs with it (the host went to the tray, or came back).
pub fn set_visible<R: Runtime>(app: &AppHandle<R>, host: &str, visible: bool) {
    let Some(dialogs) = app.try_state::<Dialogs>() else { return };
    for label in dialogs.of_host(host) {
        if let Some(w) = app.get_window(&label) {
            let _ = if visible { w.show() } else { w.hide() };
        }
    }
}

/// Opens a dialog window for the calling window. Async: a window must not be created from a synchronous command.
#[tauri::command]
pub async fn dialog_open<R: Runtime>(app: AppHandle<R>, window: Window<R>, request: DialogRequest) -> CmdResult<DialogInfo> {
    crate::error::lift(open(&app, window.label(), request).map_err(err))
}

/// The calling dialog's start data (its id, title and input). Only a dialog window has any.
#[tauri::command]
pub async fn dialog_take_input<R: Runtime>(app: AppHandle<R>, window: Window<R>) -> CmdResult<DialogStart> {
    let entry = app
        .try_state::<Dialogs>()
        .and_then(|d| d.get(window.label()))
        .ok_or("this is not a dialog window")?;
    Ok(DialogStart {
        dialog_id: entry.dialog_id,
        title: entry.title,
        modality: entry.modality,
        input: entry.input,
    })
}

/// Closes the calling dialog, answering its host with `result` (none when it was dismissed).
#[tauri::command]
pub async fn dialog_close<R: Runtime>(app: AppHandle<R>, window: Window<R>, result: Option<Value>) -> CmdResult<()> {
    let label = window.label().to_string();
    let entry = app
        .try_state::<Dialogs>()
        .and_then(|d| d.get(&label))
        .ok_or("this is not a dialog window")?;
    if result.as_ref().is_some_and(|r| r.to_string().len() > MAX_RESULT_BYTES) {
        return Err(format!("the dialog result is larger than {MAX_RESULT_BYTES} bytes").into());
    }
    if result.is_some() {
        let _ = app.emit_to(
            entry.parent.as_str(),
            RESULT_EVENT,
            DialogResult {
                label: label.clone(),
                dialog_id: entry.dialog_id.clone(),
                result,
            },
        );
    }
    crate::error::lift(window.destroy().map_err(err))
}

/// The window height (logical px) that fits `content` px of dialog content: within the minimum and `max` (the work
/// area's height less a margin), `None` for a value that is not a number.
pub fn fitted_height(content: f64, max: f64) -> Option<f64> {
    if !content.is_finite() || !max.is_finite() {
        return None;
    }
    Some(content.ceil().clamp(MIN_SIZE.1, max.clamp(MIN_SIZE.1, MAX_SIZE)))
}

/// Sizes the calling dialog window to its content: the dialog measured `content` px and asks for that height (the
/// width stays). Dialogs that need more room name a height in the registry and do not call this.
#[tauri::command]
pub async fn dialog_fit_height<R: Runtime>(app: AppHandle<R>, window: Window<R>, content: f64) -> CmdResult<()> {
    let known = app.try_state::<Dialogs>().is_some_and(|d| d.get(window.label()).is_some());
    if !known {
        return Err("this is not a dialog window".to_string().into());
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    let max = window
        .current_monitor()
        .ok()
        .flatten()
        .map_or(MAX_SIZE, |m| m.size().to_logical::<f64>(m.scale_factor()).height * 0.9);
    // The title bar is not part of the measured content: the inner size is what the content fills.
    let Some(height) = fitted_height(content, max) else {
        return Err("the content height is not a number".to_string().into());
    };
    let width = window.inner_size().map_err(err)?.to_logical::<f64>(scale).width;
    crate::error::lift(window.set_size(tauri::LogicalSize::new(width, height)).map_err(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{test_app, window};
    use crate::windows::MAIN;
    use std::sync::Arc;
    use tauri::test::MockRuntime;
    use tauri::Listener;

    fn request(id: &str, modality: Modality) -> DialogRequest {
        DialogRequest {
            dialog_id: id.into(),
            instance: None,
            title: "Set up the AI assistant".into(),
            modality,
            required_choice: false,
            input: Some(serde_json::json!({ "modelId": "m1" })),
            width: Some(760.0),
            height: None,
            min_width: None,
            min_height: None,
            bounds: None,
        }
    }

    fn app() -> tauri::App<MockRuntime> {
        let app = test_app(None);
        app.manage(Dialogs::default());
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
    fn a_default_size_is_bounded_by_three_quarters_of_the_screen() {
        assert_eq!(within_screen((680.0, 900.0), (320.0, 200.0), Some((1440.0, 800.0))), (680.0, 600.0));
        assert_eq!(
            within_screen((1600.0, 560.0), (320.0, 200.0), Some((1440.0, 800.0))),
            (1080.0, 560.0)
        );
        assert_eq!(within_screen((400.0, 300.0), (320.0, 200.0), Some((300.0, 200.0))), (320.0, 200.0));
        assert_eq!(within_screen((680.0, 900.0), (320.0, 200.0), None), (680.0, 900.0));
    }

    #[test]
    fn a_dialog_window_is_fitted_to_its_content_within_bounds() {
        assert_eq!(fitted_height(180.2, 900.0), Some(MIN_SIZE.1.max(181.0)));
        assert_eq!(fitted_height(50.0, 900.0), Some(MIN_SIZE.1));
        assert_eq!(fitted_height(5000.0, 700.0), Some(700.0));
        assert_eq!(fitted_height(f64::NAN, 700.0), None);
        assert_eq!(fitted_height(300.0, f64::INFINITY), None);
    }

    #[test]
    fn macos_modeless_dialogs_are_independent_and_macos_never_disables_the_parent_natively() {
        use Modality::*;
        assert_eq!(Parenting::of(Modeless, "macos"), Parenting::Independent);
        assert_eq!(Parenting::of(Modal, "macos"), Parenting::Owned);
        for os in ["windows", "linux"] {
            assert_eq!(Parenting::of(Modeless, os), Parenting::Owned);
            assert!(disables_parent_natively(os));
        }
        assert!(!disables_parent_natively("macos"));
    }

    #[test]
    fn labels_parse_to_their_host_and_are_not_popouts() {
        assert_eq!(host_of("d-main-1"), Some("main"));
        assert_eq!(host_of("d-w-2-13"), Some("w-2"));
        for not in [
            "main",
            "d-",
            "d-main",
            "d-main-x",
            "d-d-main-1-2",
            "d-p-main-1-2",
            "p-main-1",
            "x-main-1",
        ] {
            assert_eq!(host_of(not), None, "{not}");
        }
        assert!(is_dialog("d-main-1") && !is_dialog("p-main-1"));
        assert_eq!(crate::panel_windows::resolve("d-w-2-4"), "w-2");
        assert!(!crate::panel_windows::is_popout("d-main-1"));
    }

    #[test]
    fn requests_are_validated_and_sizes_bounded() {
        assert!(validate(&request("ai-setup", Modality::Modeless)).is_ok());
        for bad in ["", "../x", "a b", "x?y", &"x".repeat(65)] {
            assert!(validate(&request(bad, Modality::Modal)).is_err(), "{bad}");
        }
        let mut big = request("ai-setup", Modality::Modal);
        big.input = Some(Value::String("x".repeat(MAX_INPUT_BYTES + 1)));
        assert!(validate(&big).unwrap_err().to_string().contains("larger"));
        for bad in ["", "a b", "../x", &"x".repeat(17)] {
            let mut r = request("a", Modality::Modal);
            r.instance = Some(bad.to_string());
            assert!(validate(&r).is_err(), "{bad}");
        }
        let mut r = request("a", Modality::Modal);
        r.width = Some(f64::NAN);
        r.height = Some(1e9);
        r.min_width = Some(10.0);
        assert_eq!(geometry(&r), (DEFAULT_SIZE.0, MAX_SIZE, MIN_SIZE.0, MIN_SIZE.1));
        assert!(serde_json::from_value::<DialogRequest>(
            serde_json::json!({ "dialogId": "a", "title": "t", "modality": "modal", "evil": 1 })
        )
        .is_err());
        assert!(
            serde_json::from_value::<DialogRequest>(serde_json::json!({ "dialogId": "a", "title": "t", "modality": "sheet" })).is_err()
        );
    }

    #[test]
    fn no_dialog_window_has_title_bar_buttons_but_modeless_ones_stay_resizable() {
        let place = (0.0, 0.0, 500.0, 400.0);
        for (id, modality) in [("unsaved-changes", Modality::Modal), ("settings", Modality::Modeless)] {
            let c = window_config("d-main-1", "main", id, "t", place, (320.0, 200.0), modality);
            assert_eq!((c.closable, c.minimizable, c.maximizable), (false, false, false), "{id}");
            assert!(c.resizable, "{id}");
        }
    }

    #[test]
    fn centred_on_the_parent_and_the_window_config_is_an_owned_resizable_window() {
        let parent = Bounds {
            x: 100.0,
            y: 50.0,
            width: 1000.0,
            height: 800.0,
        };
        assert_eq!(centred(Some(parent), 600.0, 400.0), (300.0, 183.33333333333334));
        assert_eq!(centred(None, 600.0, 400.0), (100.0, 100.0));
        let c = window_config(
            "d-main-1",
            "main",
            "ai-setup",
            "Set up",
            (1.0, 2.0, 760.0, 520.0),
            (400.0, 300.0),
            Modality::Modeless,
        );
        assert_eq!(c.url, "index.html#/dialog?id=ai-setup&host=main&label=d-main-1");
        assert!(c.owned && c.resizable);
        assert_eq!((c.inner, c.position, c.min_inner), ((760.0, 520.0), (1.0, 2.0), (400.0, 300.0)));
    }

    #[test]
    fn opening_twice_focuses_the_one_window_and_hands_the_input_to_it_only() {
        let app = app();
        let h = app.handle().clone();
        let first = open(&h, MAIN, request("ai-setup", Modality::Modeless)).unwrap();
        assert_eq!((first.label.as_str(), first.existing), ("d-main-1", false));
        assert!(h.get_webview_window("d-main-1").is_some());
        let mut other = request("ai-setup", Modality::Modeless);
        other.instance = Some("abc1".into());
        assert!(!open(&h, MAIN, other.clone()).unwrap().existing, "another input is another window");
        assert!(open(&h, MAIN, other).unwrap().existing);
        let again = open(&h, MAIN, request("ai-setup", Modality::Modeless)).unwrap();
        assert_eq!((again.label.as_str(), again.existing), ("d-main-1", true));
        // A second dialog is a second window; a dialog can open another one for the same host.
        let nested = open(&h, "d-main-1", request("confirm-x", Modality::Modal)).unwrap();
        assert_eq!(nested.label, "d-main-3");
        let start = crate::test_support::Now::now(dialog_take_input(h.clone(), window(&app, "d-main-1").as_ref().window()));
        assert_eq!(start.unwrap().input, Some(serde_json::json!({ "modelId": "m1" })));
        assert!(crate::test_support::Now::now(dialog_take_input(h.clone(), window(&app, MAIN).as_ref().window())).is_err());
        assert!(open(&h, "w-9", request("x", Modality::Modal))
            .unwrap_err()
            .to_string()
            .contains("closed"));
    }

    #[test]
    fn at_most_eight_dialogs_per_host() {
        let app = app();
        let h = app.handle().clone();
        for i in 0..MAX_DIALOGS {
            open(&h, MAIN, request(&format!("d{i}"), Modality::Modeless)).unwrap();
        }
        assert!(open(&h, MAIN, request("too-many", Modality::Modeless))
            .unwrap_err()
            .to_string()
            .contains("At most 8"));
    }

    #[test]
    fn a_result_goes_to_the_host_and_closing_reports_where_the_window_was() {
        let app = app();
        let h = app.handle().clone();
        let main = window(&app, MAIN);
        let results = recorder(&main, RESULT_EVENT);
        let closed = recorder(&main, CLOSED_EVENT);
        open(&h, MAIN, request("ai-setup", Modality::Modeless)).unwrap();
        let dialog = window(&app, "d-main-1");
        crate::test_support::Now::now(dialog_close(h.clone(), dialog.as_ref().window(), Some(serde_json::json!(true)))).unwrap();
        crate::window_manager::on_window_event(&dialog.as_ref().window(), &tauri::WindowEvent::Destroyed);
        let r: Value = serde_json::from_str(&results.lock().unwrap()[0]).unwrap();
        assert_eq!(
            r,
            serde_json::json!({ "label": "d-main-1", "dialogId": "ai-setup", "result": true })
        );
        let c: Value = serde_json::from_str(&closed.lock().unwrap()[0]).unwrap();
        assert_eq!(c["dialogId"], "ai-setup");
        assert!(h.state::<Dialogs>().of_host(MAIN).is_empty());
        // A dialog dismissed (no result) only reports closing.
        open(&h, MAIN, request("ai-setup", Modality::Modeless)).unwrap();
        let d2 = window(&app, "d-main-2");
        crate::test_support::Now::now(dialog_close(h.clone(), d2.as_ref().window(), None)).unwrap();
        crate::window_manager::on_window_event(&d2.as_ref().window(), &tauri::WindowEvent::Destroyed);
        assert_eq!(results.lock().unwrap().len(), 1);
        assert_eq!(closed.lock().unwrap().len(), 2);
    }

    #[test]
    fn oversized_results_are_refused_and_only_dialogs_may_close() {
        let app = app();
        let h = app.handle().clone();
        open(&h, MAIN, request("ai-setup", Modality::Modal)).unwrap();
        let big = Some(Value::String("x".repeat(MAX_RESULT_BYTES + 1)));
        let dialog = window(&app, "d-main-1");
        assert!(crate::test_support::Now::now(dialog_close(h.clone(), dialog.as_ref().window(), big)).is_err());
        assert!(crate::test_support::Now::now(dialog_close(h.clone(), window(&app, MAIN).as_ref().window(), None)).is_err());
    }

    #[test]
    fn a_close_request_goes_to_the_dialog_first_and_a_hung_dialog_is_closed_anyway() {
        let app = app();
        let h = app.handle().clone();
        open(&h, MAIN, request("settings", Modality::Modeless)).unwrap();
        let asked = recorder(&window(&app, "d-main-1"), CLOSE_REQUESTED_EVENT);
        assert!(!close_requested(&h, "d-main-1"), "the dialog decides");
        assert_eq!(asked.lock().unwrap().len(), 1);
        std::thread::sleep(CLOSE_GRACE + Duration::from_millis(50));
        assert!(close_requested(&h, "d-main-1"), "no answer: closed anyway");
        assert!(close_requested(&h, "d-main-9"), "unknown windows just close");
    }

    #[test]
    fn a_required_choice_modal_ignores_close_requests_and_a_plain_modal_gets_them_as_cancel() {
        let app = app();
        let h = app.handle().clone();
        let mut required = request("close-prompt", Modality::Modal);
        required.required_choice = true;
        open(&h, MAIN, required).unwrap();
        let asked = recorder(&window(&app, "d-main-1"), CLOSE_REQUESTED_EVENT);
        assert!(!close_requested(&h, "d-main-1"));
        std::thread::sleep(CLOSE_GRACE + Duration::from_millis(50));
        assert!(!close_requested(&h, "d-main-1"), "never closed behind the user's back");
        assert!(
            asked.lock().unwrap().is_empty(),
            "nothing to cancel, so the dialog is not even asked"
        );
        // A required choice only means something for a modal.
        let mut modeless = request("settings", Modality::Modeless);
        modeless.required_choice = true;
        open(&h, MAIN, modeless).unwrap();
        let asked2 = recorder(&window(&app, "d-main-2"), CLOSE_REQUESTED_EVENT);
        assert!(!close_requested(&h, "d-main-2"));
        assert_eq!(asked2.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_required_choice_modal_whose_page_hangs_can_still_be_closed() {
        let app = app();
        let h = app.handle().clone();
        let mut required = request("close-prompt", Modality::Modal);
        required.required_choice = true;
        open(&h, MAIN, required).unwrap();
        assert!(!close_requested(&h, "d-main-1"), "a live page keeps its required choice");
        // The window watchdog decided the page does not answer: nothing could ever answer the choice.
        page_unresponsive(&h, "d-main-1", true);
        assert!(close_requested(&h, "d-main-1"), "the OS close ends it");
        // A page that answers again keeps its choice.
        page_unresponsive(&h, "d-main-1", false);
        assert!(!close_requested(&h, "d-main-1"));
        page_unresponsive(&h, "d-main-9", true);
    }

    #[test]
    fn closing_the_host_closes_its_dialogs() {
        let app = app();
        let h = app.handle().clone();
        open(&h, MAIN, request("a", Modality::Modeless)).unwrap();
        open(&h, MAIN, request("b", Modality::Modal)).unwrap();
        assert_eq!(close_all_of(&h, MAIN), 2);
        assert!(h.state::<Dialogs>().of_host(MAIN).is_empty());
    }
}
