//! Per-window state. Every window has its own workspace (store, profile, open
//! requests, live tail, import progress, project); app-wide things (config
//! folder, settings, tools, lifecycle, background tasks) stay in managed
//! state of their own.
//!
//! Commands get the calling window's state with the [`CurrentWindow`]
//! extractor (resolved from the window label of the invoking webview), so a
//! command looks like `async fn query_page(state: CurrentWindow, request: PageQuery)`.
//! A window's state is created on its first use and dropped when the window
//! is destroyed (see `window_manager`), which closes its store (in the store
//! worker) and stops its live tail.

use crate::engine::{Engine, Host};
use crate::live::LiveSlot;
use crate::profile::ProfileDoc;
use crate::{err, CmdResult};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tauri::ipc::{CommandArg, CommandItem, InvokeError};
use tauri::{Emitter, EventTarget, Manager, Runtime};

/// Label of the window created at startup (tauri.conf.json).
pub const MAIN: &str = "main";
/// Labels of windows opened later: `w-2`, `w-3`, ... (never reused in a run).
pub const LABEL_PREFIX: &str = "w-";
/// Event telling a window that `take_pending_open` has paths for it.
pub const OPEN_EVENT: &str = "imt://open";

/// One window's workspace and everything that belongs to it.
pub struct WindowState {
    label: String,
    /// The window's store (in the store worker), opened on first use.
    pub(crate) engine: Engine,
    /// The window's plugin registry and compiled profile (tools, marker kinds, plugin status).
    pub(crate) host: Host,
    pub(crate) profile: RwLock<ProfileDoc>,
    /// Paths to open that arrived outside the UI (launch arguments, files
    /// opened from the OS, restored sessions); the UI takes them after `imt://open`.
    pending: Mutex<Vec<String>>,
    /// Paths waiting for the user's answer to "open here or in a new window?".
    pending_ask: Mutex<Vec<String>>,
    /// The running live tail session, if any.
    pub(crate) live: LiveSlot,
    /// The window title the UI set (project name), for the tray's window list.
    title: Mutex<String>,
    /// The project this window last opened or saved.
    project: Mutex<Option<String>>,
    /// Something was imported or a project opened (and not cleared since).
    loaded: AtomicBool,
    /// "Close" was asked of the UI and not answered yet.
    pub(crate) close_asked: AtomicBool,
    /// Open profiles and heap snapshots (see `profiling`).
    pub(crate) profiling: crate::profiling::ProfilingSlot,
    /// The window's AI chats and running AI use cases.
    pub(crate) ai: crate::ai::WindowAi,
}

impl WindowState {
    pub fn new(label: &str, engine: Engine, host: Host, profile: ProfileDoc) -> WindowState {
        WindowState {
            label: label.to_string(),
            engine,
            host,
            profile: RwLock::new(profile),
            pending: Mutex::default(),
            pending_ask: Mutex::default(),
            live: LiveSlot::default(),
            title: Mutex::new(String::new()),
            project: Mutex::default(),
            loaded: AtomicBool::new(false),
            close_asked: AtomicBool::new(false),
            profiling: crate::profiling::ProfilingSlot::default(),
            ai: crate::ai::WindowAi::default(),
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Deactivates the window's plugins and closes its store (window closed, app exiting). `keep` keeps a
    /// scratch store's folder (IMT is quitting: the session reopens it).
    pub(crate) fn shutdown(&self, keep: bool) {
        self.host.registry.shutdown();
        self.engine.shutdown(keep);
    }

    /// Publishes a host event to the workspace's plugins.
    pub(crate) fn publish<E: Send + Sync + 'static>(&self, event: E) {
        self.host.registry.events().publish(event);
    }

    /// Delivery counters of every event subscriber of the workspace's plugins.
    pub(crate) fn event_metrics(&self) -> Vec<imt_core::events::SubscriberMetrics> {
        self.host.registry.events().metrics()
    }

    /// The tools of the window's plugins.
    pub(crate) fn tools(&self) -> imt_core::plugin::tools::ToolHost {
        self.host.registry.tools()
    }

    pub(crate) fn profile_doc(&self) -> ProfileDoc {
        crate::sync::read(&self.profile).clone()
    }

    pub(crate) fn set_profile_doc(&self, doc: ProfileDoc) {
        *crate::sync::write(&self.profile) = doc;
    }

    pub(crate) fn queue(&self, paths: impl IntoIterator<Item = String>) {
        crate::sync::lock(&self.pending).extend(paths);
    }

    pub(crate) fn take_pending(&self) -> Vec<String> {
        std::mem::take(&mut *crate::sync::lock(&self.pending))
    }

    pub(crate) fn set_pending_ask(&self, paths: Vec<String>) {
        *crate::sync::lock(&self.pending_ask) = paths;
    }

    pub(crate) fn take_pending_ask(&self) -> Vec<String> {
        std::mem::take(&mut *crate::sync::lock(&self.pending_ask))
    }

    pub fn title(&self) -> String {
        crate::sync::lock(&self.title).clone()
    }

    pub(crate) fn set_title(&self, title: &str) {
        *crate::sync::lock(&self.title) = title.to_string();
    }

    pub fn project(&self) -> Option<String> {
        crate::sync::lock(&self.project).clone()
    }

    pub(crate) fn set_project(&self, path: Option<String>) {
        *crate::sync::lock(&self.project) = path;
    }

    /// Something was imported (or nothing is any more, after `clear`).
    pub(crate) fn set_loaded(&self, loaded: bool) {
        self.loaded.store(loaded, Ordering::SeqCst);
    }

    /// Nothing is open or about to open: files from other apps can go here
    /// instead of into a new window. An importing window is not pristine.
    pub fn is_pristine(&self) -> bool {
        !self.loaded.load(Ordering::SeqCst)
            && !self.engine.progress().running
            && self.project().is_none()
            && self.pending.lock().map(|p| p.is_empty()).unwrap_or(false)
    }
}

/// Builds the state of a new window (its own store with its own plugin registry).
pub type WindowFactory = Box<dyn Fn(&str) -> anyhow::Result<WindowState> + Send + Sync>;

/// The registry of window states, keyed by window label. Managed as Tauri state.
pub struct Windows {
    map: RwLock<BTreeMap<String, Arc<WindowState>>>,
    /// Held while a window's state is built (outside `map`'s lock, so lookups never wait for a build).
    creating: Mutex<()>,
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
            creating: Mutex::default(),
            closed: Mutex::default(),
            factory,
            next: AtomicU32::new(2),
            focused: Mutex::default(),
            exiting: AtomicBool::new(false),
        }
    }

    pub fn get(&self, label: &str) -> Option<Arc<WindowState>> {
        crate::sync::read(&self.map).get(label).cloned()
    }

    /// The window's state, created on first use. Fails for a window that was closed.
    pub fn get_or_create(&self, label: &str) -> anyhow::Result<Arc<WindowState>> {
        if let Some(w) = self.get(label) {
            return Ok(w);
        }
        // One build at a time (no window is built twice); lookups and other windows' commands go on.
        let _building = crate::sync::lock(&self.creating);
        if let Some(w) = self.get(label) {
            return Ok(w);
        }
        if crate::sync::lock(&self.closed).contains(label) {
            anyhow::bail!("this window was closed");
        }
        let state = Arc::new((self.factory)(label)?);
        let mut map = crate::sync::write(&self.map);
        map.insert(label.to_string(), state.clone());
        log::info!(Windows = map.len() as u64; "Window workspace created");
        Ok(state)
    }

    /// Forgets a window (it was destroyed). Its workspace is freed once no
    /// command still runs on it.
    pub fn remove(&self, label: &str) -> Option<Arc<WindowState>> {
        crate::sync::lock(&self.closed).insert(label.to_string());
        let mut focused = crate::sync::lock(&self.focused);
        if focused.as_deref() == Some(label) {
            *focused = None;
        }
        let removed = crate::sync::write(&self.map).remove(label);
        if let Some(w) = &removed {
            // Quitting keeps scratch stores for the session; closing a window deletes them.
            w.shutdown(self.exiting());
            log::info!("Window workspace freed");
        }
        removed
    }

    /// Every window state, `main` first, then in creation order.
    pub fn all(&self) -> Vec<Arc<WindowState>> {
        let mut all: Vec<_> = crate::sync::read(&self.map).values().cloned().collect();
        all.sort_by_key(|w| label_order(w.label()));
        all
    }

    /// Deactivates the plugins of every window and lets go of their stores (on exit; scratch stores stay for
    /// the session).
    pub fn shutdown_plugins(&self) {
        for w in self.all() {
            w.shutdown(true);
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
        *crate::sync::lock(&self.focused) = Some(label.to_string());
    }

    /// The window that had the focus last, if it still exists.
    pub fn last_focused(&self) -> Option<String> {
        let label = crate::sync::lock(&self.focused).clone()?;
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
    /// The state of window `label` (created on first use). A pop-out panel
    /// window (`p-<host>-<n>`) and a browser's "AI is browsing" banner (`aib-<owner>`)
    /// have no workspace of their own: they get their window's.
    pub fn of<R: Runtime, M: Manager<R>>(manager: &M, label: &str) -> CmdResult<CurrentWindow> {
        let windows = manager.try_state::<Windows>().ok_or("IMT is not ready yet")?;
        crate::error::lift(
            windows
                .get_or_create(crate::browser::banner_owner(label).unwrap_or_else(|| crate::panel_windows::resolve(label)))
                .map(CurrentWindow)
                .map_err(err),
        )
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

/// Events only the window itself acts on (files to open, its close and open
/// prompts); its pop-out panels do not get them.
pub const HOST_ONLY_EVENTS: [&str; 3] = [
    OPEN_EVENT,
    crate::window_manager::CLOSE_REQUESTED_EVENT,
    crate::window_manager::OPEN_ASK_EVENT,
];

/// Sends an event to one window and its pop-out panels (they show the same
/// workspace), never to other windows. Never fails (a closed window just misses it).
pub fn emit_to_window<R: Runtime, S: Serialize + Clone>(app: &tauri::AppHandle<R>, label: &str, event: &str, payload: S) {
    if HOST_ONLY_EVENTS.contains(&event) {
        let _ = app.emit_to(label, event, payload);
        return;
    }
    // Like `emit_to(label)` (any target kind with that label), for the whole group.
    let _ = app.emit_filter(event, payload, |target| match target {
        EventTarget::Window { label: l }
        | EventTarget::Webview { label: l }
        | EventTarget::WebviewWindow { label: l }
        | EventTarget::AnyLabel { label: l } => l == label || crate::panel_windows::host_of(l) == Some(label),
        _ => false,
    });
}

/// Queues paths for a window's UI and tells it to take them.
pub fn queue_open<R: Runtime>(app: &tauri::AppHandle<R>, label: &str, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    // Files from the command line, a second launch or the OS were chosen by the user.
    crate::paths::grant_read_all(app, label, &paths, crate::paths::GrantSource::CommandLine);
    match CurrentWindow::of(app, label) {
        Ok(win) => {
            win.queue(paths);
            emit_to_window(app, label, OPEN_EVENT, ());
        }
        Err(e) => log::warn!("could not queue files to open: {e}"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use imt_core::{PluginRegistry, Profile};

    /// A window's state with the store in process (scratch folders under the system temp folder).
    pub(crate) fn window_state(label: &str, registry: PluginRegistry) -> WindowState {
        let root = std::env::temp_dir().join(format!("imt-test-stores-{}", std::process::id()));
        let engine = Engine::new(crate::engine::local::host(), root, label, Profile::default_profile());
        registry.resolve();
        let host = Host::new(registry, Profile::default_profile()).unwrap();
        WindowState::new(label, engine, host, ProfileDoc::default())
    }

    pub(crate) fn factory() -> WindowFactory {
        Box::new(|label| {
            if label == "broken" {
                anyhow::bail!("no workspace for you");
            }
            Ok(window_state(label, PluginRegistry::new()))
        })
    }

    /// Building a window's state (plugins, profile: slow) happens outside the registry lock.
    #[test]
    fn lookups_do_not_wait_while_a_window_is_built() {
        let (entered, building) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let released = Mutex::new(released);
        let inner = factory();
        let windows = Arc::new(Windows::new(Box::new(move |label| {
            if label == "slow" {
                let _ = entered.send(());
                let _ = released.lock().unwrap().recv_timeout(std::time::Duration::from_secs(10));
            }
            inner(label)
        })));
        windows.get_or_create(MAIN).unwrap();
        let w = windows.clone();
        let slow = std::thread::spawn(move || w.get_or_create("slow").map(|s| s.label().to_string()));
        building.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        let started = std::time::Instant::now();
        assert!(windows.get(MAIN).is_some());
        assert_eq!(windows.all().len(), 1);
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "the registry lock was free");
        release.send(()).unwrap();
        assert_eq!(slow.join().unwrap().unwrap(), "slow");
        assert!(Arc::ptr_eq(&windows.get("slow").unwrap(), &windows.get_or_create("slow").unwrap()));
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
        w.set_project(Some("/p.imtz".into()));
        assert!(!w.is_pristine());
        assert_eq!(w.project().as_deref(), Some("/p.imtz"));
        w.set_project(None);
        w.set_loaded(true);
        assert!(!w.is_pristine(), "a window with data is not reused");
        w.set_loaded(false);
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

    #[test]
    fn plugins_shut_down_when_a_window_closes_or_the_app_exits() {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = count.clone();
        let windows = Windows::new(Box::new(move |label| {
            let mut registry = PluginRegistry::new();
            registry.install_shared(Arc::new(Counting(c.clone()))).unwrap();
            Ok(window_state(label, registry))
        }));
        windows.get_or_create(MAIN).unwrap();
        windows.get_or_create("w-2").unwrap();
        windows.remove("w-2");
        assert_eq!(count.load(Ordering::SeqCst), 1, "closing a window deactivates its plugins");
        windows.shutdown_plugins();
        assert_eq!(count.load(Ordering::SeqCst), 2);
        windows.shutdown_plugins();
        assert_eq!(count.load(Ordering::SeqCst), 2, "already inactive");
    }

    #[test]
    fn host_events_reach_plugin_subscribers_and_are_counted() {
        let windows = Windows::new(factory());
        let w = windows.get_or_create(MAIN).unwrap();
        let bus = w.host.registry.events();
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
}
