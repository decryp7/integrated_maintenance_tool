//! Per-window state. Every window has its own workspace (store, profile, open
//! requests, live tail, import progress, project); app-wide things (config
//! folder, settings, tools, lifecycle, background tasks) stay in managed
//! state of their own.
//!
//! Commands get the calling window's state with the [`CurrentWindow`]
//! extractor (resolved from the window label of the invoking webview), so a
//! command looks like `async fn query_page(state: CurrentWindow, request: PageRequest)`.
//! A window's state is created on its first use and dropped when the window
//! is destroyed (see `window_manager`), which frees its workspace and stops its
//! live tail.

use crate::live::LiveSlot;
use crate::profile::ProfileDoc;
use crate::{err, CmdResult};
use imt_core::workspace::ImportProgress;
use imt_core::LogStore;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use tauri::ipc::{CommandArg, CommandItem, InvokeError};
use tauri::{Emitter, Manager, Runtime};

/// Label of the window created at startup (tauri.conf.json).
pub const MAIN: &str = "main";
/// Labels of windows opened later: `w-2`, `w-3`, ... (never reused in a run).
pub const LABEL_PREFIX: &str = "w-";
/// Event telling a window that `take_pending_open` has paths for it.
pub const OPEN_EVENT: &str = "imt://open";

/// One window's workspace and everything that belongs to it.
pub struct WindowState {
    label: String,
    pub(crate) workspace: Arc<RwLock<Box<dyn LogStore>>>,
    pub(crate) profile: RwLock<ProfileDoc>,
    /// Paths to open that arrived outside the UI (launch arguments, files
    /// opened from the OS, restored sessions); the UI takes them after `imt://open`.
    pending: Mutex<Vec<String>>,
    /// Paths waiting for the user's answer to "open here or in a new window?".
    pending_ask: Mutex<Vec<String>>,
    /// Import progress, shared with the store (no lock needed to read or cancel).
    pub(crate) progress: Arc<ImportProgress>,
    /// The store's event bus (no lock needed to publish or read metrics).
    pub(crate) events: Option<Arc<imt_core::events::EventBus>>,
    /// The running live tail session, if any.
    pub(crate) live: LiveSlot,
    /// The window title the UI set (project name), for the tray's window list.
    title: Mutex<String>,
    /// The `.imtproj` this window last opened or saved.
    project: Mutex<Option<String>>,
    /// "Close" was asked of the UI and not answered yet.
    pub(crate) close_asked: AtomicBool,
}

impl WindowState {
    pub fn new(label: &str, store: Box<dyn LogStore>, progress: Arc<ImportProgress>, profile: ProfileDoc) -> WindowState {
        WindowState {
            label: label.to_string(),
            events: store.event_bus(),
            workspace: Arc::new(RwLock::new(store)),
            profile: RwLock::new(profile),
            pending: Mutex::default(),
            pending_ask: Mutex::default(),
            progress,
            live: LiveSlot::default(),
            title: Mutex::new(String::new()),
            project: Mutex::default(),
            close_asked: AtomicBool::new(false),
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// The store for reading. A panic during an earlier command must not make
    /// every later command fail, so a poisoned lock is recovered.
    pub(crate) fn store_read(&self) -> RwLockReadGuard<'_, Box<dyn LogStore>> {
        self.workspace.read().unwrap_or_else(|p| {
            log::warn!("recovering the workspace lock after a crash");
            p.into_inner()
        })
    }

    pub(crate) fn store_write(&self) -> RwLockWriteGuard<'_, Box<dyn LogStore>> {
        self.workspace.write().unwrap_or_else(|p| {
            log::warn!("recovering the workspace lock after a crash");
            p.into_inner()
        })
    }

    /// Deactivates the workspace's plugins (window closed, app exiting).
    /// Never waits: a store busy with an import is skipped (its process or
    /// window is going away anyway).
    pub(crate) fn shutdown_plugins(&self) {
        match self.workspace.try_read() {
            Ok(store) => store.shutdown_plugins(),
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner().shutdown_plugins(),
            Err(std::sync::TryLockError::WouldBlock) => log::info!("Plugins not shut down: the workspace is busy"),
        }
    }

    /// Publishes a host event to the workspace's plugins.
    pub(crate) fn publish<E: Send + Sync + 'static>(&self, event: E) {
        if let Some(bus) = &self.events {
            bus.publish(event);
        }
    }

    /// Delivery counters of every event subscriber of the workspace's plugins.
    pub(crate) fn event_metrics(&self) -> Vec<imt_core::events::SubscriberMetrics> {
        self.events.as_ref().map(|b| b.metrics()).unwrap_or_default()
    }

    pub(crate) fn profile_doc(&self) -> ProfileDoc {
        self.profile.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub(crate) fn set_profile_doc(&self, doc: ProfileDoc) {
        *self.profile.write().unwrap_or_else(|p| p.into_inner()) = doc;
    }

    pub(crate) fn queue(&self, paths: impl IntoIterator<Item = String>) {
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).extend(paths);
    }

    pub(crate) fn take_pending(&self) -> Vec<String> {
        std::mem::take(&mut *self.pending.lock().unwrap_or_else(|p| p.into_inner()))
    }

    pub(crate) fn set_pending_ask(&self, paths: Vec<String>) {
        *self.pending_ask.lock().unwrap_or_else(|p| p.into_inner()) = paths;
    }

    pub(crate) fn take_pending_ask(&self) -> Vec<String> {
        std::mem::take(&mut *self.pending_ask.lock().unwrap_or_else(|p| p.into_inner()))
    }

    pub fn title(&self) -> String {
        self.title.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub(crate) fn set_title(&self, title: &str) {
        *self.title.lock().unwrap_or_else(|p| p.into_inner()) = title.to_string();
    }

    pub fn project(&self) -> Option<String> {
        self.project.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub(crate) fn set_project(&self, path: Option<String>) {
        *self.project.lock().unwrap_or_else(|p| p.into_inner()) = path;
    }

    /// Nothing is open or about to open: files from other apps can go here
    /// instead of into a new window. A busy workspace is not pristine.
    pub fn is_pristine(&self) -> bool {
        let empty = match self.workspace.try_read() {
            Ok(store) => store.summary().sources.is_empty(),
            Err(_) => false,
        };
        empty && self.project().is_none() && self.pending.lock().map(|p| p.is_empty()).unwrap_or(false)
    }
}

/// Builds the state of a new window (its own store with its own plugin registry).
pub type WindowFactory = Box<dyn Fn(&str) -> anyhow::Result<WindowState> + Send + Sync>;

/// The registry of window states, keyed by window label. Managed as Tauri state.
pub struct Windows {
    map: RwLock<BTreeMap<String, Arc<WindowState>>>,
    /// Labels of windows that were closed: a late command must not bring them back.
    closed: Mutex<HashSet<String>>,
    factory: WindowFactory,
    next: AtomicU32,
    focused: Mutex<Option<String>>,
    exiting: AtomicBool,
}

impl Windows {
    pub fn new(factory: WindowFactory) -> Windows {
        Windows {
            map: RwLock::default(),
            closed: Mutex::default(),
            factory,
            next: AtomicU32::new(2),
            focused: Mutex::default(),
            exiting: AtomicBool::new(false),
        }
    }

    pub fn get(&self, label: &str) -> Option<Arc<WindowState>> {
        self.map.read().unwrap_or_else(|p| p.into_inner()).get(label).cloned()
    }

    /// The window's state, created on first use. Fails for a window that was closed.
    pub fn get_or_create(&self, label: &str) -> anyhow::Result<Arc<WindowState>> {
        if let Some(w) = self.get(label) {
            return Ok(w);
        }
        if self.closed.lock().unwrap_or_else(|p| p.into_inner()).contains(label) {
            anyhow::bail!("this window was closed");
        }
        let mut map = self.map.write().unwrap_or_else(|p| p.into_inner());
        if let Some(w) = map.get(label) {
            return Ok(w.clone());
        }
        let state = Arc::new((self.factory)(label)?);
        map.insert(label.to_string(), state.clone());
        log::info!(Windows = map.len() as u64; "Window workspace created");
        Ok(state)
    }

    /// Forgets a window (it was destroyed). Its workspace is freed once no
    /// command still runs on it.
    pub fn remove(&self, label: &str) -> Option<Arc<WindowState>> {
        self.closed.lock().unwrap_or_else(|p| p.into_inner()).insert(label.to_string());
        let mut focused = self.focused.lock().unwrap_or_else(|p| p.into_inner());
        if focused.as_deref() == Some(label) {
            *focused = None;
        }
        let removed = self.map.write().unwrap_or_else(|p| p.into_inner()).remove(label);
        if let Some(w) = &removed {
            w.shutdown_plugins();
            log::info!("Window workspace freed");
        }
        removed
    }

    /// Every window state, `main` first, then in creation order.
    pub fn all(&self) -> Vec<Arc<WindowState>> {
        let mut all: Vec<_> = self.map.read().unwrap_or_else(|p| p.into_inner()).values().cloned().collect();
        all.sort_by_key(|w| label_order(w.label()));
        all
    }

    /// Deactivates the plugins of every window's workspace (on exit).
    pub fn shutdown_plugins(&self) {
        for w in self.all() {
            w.shutdown_plugins();
        }
    }

    /// A window to read app-wide facts from (e.g. the installed plugins).
    pub fn any(&self) -> Option<Arc<WindowState>> {
        self.all().into_iter().next()
    }

    /// A fresh label for a new window.
    pub fn next_label(&self) -> String {
        format!("{LABEL_PREFIX}{}", self.next.fetch_add(1, Ordering::SeqCst))
    }

    pub fn note_focus(&self, label: &str) {
        *self.focused.lock().unwrap_or_else(|p| p.into_inner()) = Some(label.to_string());
    }

    /// The window that had the focus last, if it still exists.
    pub fn last_focused(&self) -> Option<String> {
        let label = self.focused.lock().unwrap_or_else(|p| p.into_inner()).clone()?;
        self.get(&label).map(|_| label)
    }

    /// IMT is quitting: windows being destroyed now keep their sessions.
    pub fn set_exiting(&self) {
        self.exiting.store(true, Ordering::SeqCst);
    }

    pub fn exiting(&self) -> bool {
        self.exiting.load(Ordering::SeqCst)
    }
}

/// Sort key: `main`, then `w-2`, `w-3`, ... numerically, then anything else.
pub fn label_order(label: &str) -> (u8, u32, String) {
    if label == MAIN {
        return (0, 0, String::new());
    }
    match label.strip_prefix(LABEL_PREFIX).and_then(|n| n.parse().ok()) {
        Some(n) => (1, n, String::new()),
        None => (2, 0, label.to_string()),
    }
}

/// The calling window's state; a command argument resolved from the window
/// label of the invoking webview (never from anything the webview sends).
#[derive(Clone)]
pub struct CurrentWindow(Arc<WindowState>);

impl CurrentWindow {
    /// The state of window `label` (created on first use).
    pub fn of<R: Runtime, M: Manager<R>>(manager: &M, label: &str) -> CmdResult<CurrentWindow> {
        let windows = manager.try_state::<Windows>().ok_or("IMT is not ready yet")?;
        windows.get_or_create(label).map(CurrentWindow).map_err(err)
    }

    pub fn arc(&self) -> &Arc<WindowState> {
        &self.0
    }
}

impl Deref for CurrentWindow {
    type Target = WindowState;
    fn deref(&self) -> &WindowState {
        &self.0
    }
}

impl<'de, R: Runtime> CommandArg<'de, R> for CurrentWindow {
    fn from_command(command: CommandItem<'de, R>) -> Result<Self, InvokeError> {
        let webview = command.message.webview();
        let label = webview.window().label().to_string();
        CurrentWindow::of(&webview, &label).map_err(InvokeError::from)
    }
}

/// Sends an event to one window only. Never fails (a closed window just misses it).
pub fn emit_to_window<R: Runtime, S: Serialize + Clone>(app: &tauri::AppHandle<R>, label: &str, event: &str, payload: S) {
    let _ = app.emit_to(label, event, payload);
}

/// Queues paths for a window's UI and tells it to take them.
pub fn queue_open<R: Runtime>(app: &tauri::AppHandle<R>, label: &str, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    match CurrentWindow::of(app, label) {
        Ok(win) => {
            win.queue(paths);
            emit_to_window(app, label, OPEN_EVENT, ());
        }
        Err(e) => log::warn!("could not queue files to open: {e}"),
    }
}

/// Runs read-only work on the blocking pool so large scans never stall the UI thread.
pub(crate) async fn read<T: Send + 'static>(
    win: &WindowState,
    f: impl FnOnce(&dyn LogStore) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let ws = win.workspace.clone();
    tauri::async_runtime::spawn_blocking(move || f(ws.read().unwrap_or_else(|p| p.into_inner()).as_ref()).map_err(err))
        .await
        .map_err(err)?
}

pub(crate) async fn write<T: Send + 'static>(
    win: &WindowState,
    f: impl FnOnce(&mut dyn LogStore) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let ws = win.workspace.clone();
    tauri::async_runtime::spawn_blocking(move || f(ws.write().unwrap_or_else(|p| p.into_inner()).as_mut()).map_err(err))
        .await
        .map_err(err)?
}

/// Like [`write`], and tells the window (`imt://error`) about plugin crashes,
/// quarantines and import crashes the engine contained meanwhile.
pub(crate) async fn write_reporting<R: Runtime, T: Send + 'static>(
    app: &tauri::AppHandle<R>,
    win: &WindowState,
    f: impl FnOnce(&mut dyn LogStore) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let (out, errors) = write(win, move |ws| {
        let before = crate::engine_errors::HealthSnapshot::of(&ws.summary());
        let out = f(ws);
        let errors = crate::engine_errors::new_errors(&before, &crate::engine_errors::HealthSnapshot::of(&ws.summary()));
        Ok((out, errors))
    })
    .await?;
    for e in &errors {
        crate::snapshots::emit_engine_error(app, win.label(), e);
    }
    out.map_err(err)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use imt_core::{Profile, Workspace};

    pub(crate) fn factory() -> WindowFactory {
        Box::new(|label| {
            if label == "broken" {
                anyhow::bail!("no workspace for you");
            }
            let ws = Workspace::new(Profile::default_profile())?;
            let progress = ws.progress();
            Ok(WindowState::new(label, Box::new(ws), progress, ProfileDoc::default()))
        })
    }

    #[test]
    fn registry_creates_looks_up_and_frees() {
        let windows = Windows::new(factory());
        assert!(windows.get(MAIN).is_none());
        let main = windows.get_or_create(MAIN).unwrap();
        assert!(Arc::ptr_eq(&main, &windows.get_or_create(MAIN).unwrap()));
        let label = windows.next_label();
        assert_eq!(label, "w-2");
        assert_eq!(windows.next_label(), "w-3");
        let second = windows.get_or_create(&label).unwrap();
        assert_eq!(second.label(), "w-2");
        windows.get_or_create("w-10").unwrap();
        let order: Vec<String> = windows.all().iter().map(|w| w.label().to_string()).collect();
        assert_eq!(order, ["main", "w-2", "w-10"]);
        assert!(windows.get_or_create("broken").is_err());

        windows.note_focus("w-2");
        assert_eq!(windows.last_focused().as_deref(), Some("w-2"));
        // Closing frees the state (no other owner), and the label stays closed.
        let weak = Arc::downgrade(&second);
        drop(second);
        assert!(windows.remove("w-2").is_some());
        assert!(weak.upgrade().is_none(), "the workspace was freed");
        assert!(windows.last_focused().is_none());
        assert!(windows.get_or_create("w-2").err().unwrap().to_string().contains("closed"));
        assert!(windows.remove("w-2").is_none());
        assert_eq!(windows.any().unwrap().label(), MAIN);
        assert!(!windows.exiting());
        windows.set_exiting();
        assert!(windows.exiting());
        assert_eq!(label_order("other"), (2, 0, "other".into()));
    }

    #[test]
    fn window_state_queues_titles_and_pristine() {
        let windows = Windows::new(factory());
        let w = windows.get_or_create(MAIN).unwrap();
        assert!(w.is_pristine());
        w.queue(["/a".to_string()]);
        assert!(!w.is_pristine(), "something is about to open");
        assert_eq!(w.take_pending(), ["/a"]);
        assert!(w.take_pending().is_empty());
        w.set_pending_ask(vec!["/b".into()]);
        assert_eq!(w.take_pending_ask(), ["/b"]);
        assert!(w.take_pending_ask().is_empty());
        w.set_title("Incident — IMT");
        assert_eq!(w.title(), "Incident — IMT");
        w.set_project(Some("/p.imtproj".into()));
        assert!(!w.is_pristine());
        assert_eq!(w.project().as_deref(), Some("/p.imtproj"));
        w.set_project(None);
        {
            let _busy = w.store_write();
            assert!(!w.is_pristine(), "a busy workspace is not reused");
        }
        assert!(w.is_pristine());
        w.set_profile_doc(ProfileDoc {
            name: "x".into(),
            path: None,
            toml: String::new(),
        });
        assert_eq!(w.profile_doc().name, "x");
    }

    /// A plugin counting how often it was deactivated.
    struct Counting(Arc<std::sync::atomic::AtomicUsize>);
    impl imt_core::Plugin for Counting {
        fn id(&self) -> &'static str {
            "test.counting"
        }
        fn name(&self) -> &'static str {
            "Counting"
        }
        fn version(&self) -> &'static str {
            "1.0.0"
        }
        fn register(&self, _: &mut imt_core::PluginRegistry) -> anyhow::Result<()> {
            Ok(())
        }
        fn deactivate(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn counting_window(label: &str, count: &Arc<std::sync::atomic::AtomicUsize>) -> Arc<WindowState> {
        let mut registry = imt_core::PluginRegistry::new();
        registry.install_shared(Arc::new(Counting(count.clone()))).unwrap();
        let ws = Workspace::with_registry(Profile::default_profile(), registry).unwrap();
        let progress = ws.progress();
        Arc::new(WindowState::new(label, Box::new(ws), progress, ProfileDoc::default()))
    }

    #[test]
    fn plugins_shut_down_when_a_window_closes_or_the_app_exits() {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = count.clone();
        let windows = Windows::new(Box::new(move |label| {
            let w = counting_window(label, &c);
            Ok(Arc::try_unwrap(w).ok().unwrap())
        }));
        windows.get_or_create(MAIN).unwrap();
        windows.get_or_create("w-2").unwrap();
        windows.remove("w-2");
        assert_eq!(count.load(Ordering::SeqCst), 1, "closing a window deactivates its plugins");
        windows.shutdown_plugins();
        assert_eq!(count.load(Ordering::SeqCst), 2);
        windows.shutdown_plugins();
        assert_eq!(count.load(Ordering::SeqCst), 2, "already inactive");

        // A busy store is skipped instead of blocking the exit.
        let busy = counting_window("busy", &count);
        let guard = busy.store_write();
        busy.shutdown_plugins();
        drop(guard);
        assert_eq!(count.load(Ordering::SeqCst), 2);
        busy.shutdown_plugins();
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn host_events_reach_plugin_subscribers_and_are_counted() {
        let windows = Windows::new(factory());
        let w = windows.get_or_create(MAIN).unwrap();
        let bus = w.events.clone().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        bus.subscribe::<imt_core::events::SnapshotTaken>("test.listener", move |e| {
            let _ = tx.send(e.id.clone());
        })
        .unwrap();
        w.publish(imt_core::events::SnapshotTaken { id: "snap-1".into() });
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(), "snap-1");
        let m = w.event_metrics();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].plugin, "test.listener");
    }

    #[test]
    fn poisoned_locks_are_recovered() {
        let windows = Windows::new(factory());
        let w = windows.get_or_create(MAIN).unwrap();
        let w2 = w.clone();
        let _ = std::thread::spawn(move || {
            let _g = w2.store_write();
            panic!("crash while holding the workspace");
        })
        .join();
        assert!(w.workspace.is_poisoned());
        assert_eq!(w.store_read().summary().total, 0);
        drop(w.store_write());
    }
}
