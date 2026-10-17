//! Robustness around the app's lifetime: safe mode, detection of unclean
//! shutdowns, persisted plugin enable/disable, session autosave/restore and a
//! crash handler that records panics in IMT's own log.
//!
//! Sessions (format version 2): each window autosaves to
//! `sessions/<window>/last-session.imtproj`, and `sessions/manifest.json`
//! (`{ "version": 2, "windows": [...] }`) lists the windows of the current run.
//! At start the previous run's sessions are copied to `recovered/` (what a
//! crash restore offers) and the manifest is reset. A version 1 install had a
//! single `last-session.imtproj` next to them; it is recovered as the main
//! window's session. Missing or corrupt files fall back to "no session".
//!
//! Unclean shutdowns: `running.marker` (`{ "version": 1, "pid": .., "startedAt": .. }`)
//! names the process that is running. At start a marker left behind is a
//! crash only when its process is gone: no process with that PID, or one whose
//! start time differs (the PID was reused). A marker whose process still runs
//! (e.g. the previous instance is still exiting after "Restart") is taken over
//! without reporting a crash, and [`Lifecycle::end`] removes the marker only
//! while it is still this process's, so the old process cannot erase the new
//! one's. A version 0 marker (a bare PID) has no start time: a live PID counts
//! as running. An unreadable marker counts as a crash.
//!
//! Instances: the single (main) instance keeps its marker and sessions in the
//! data folder. A separate process (`--new-instance`, safe mode) uses
//! `instances/<pid>/` instead, so it never rotates the main instance's
//! sessions or reports a crash of it; the folder is removed on a clean exit,
//! and every start prunes the folders of processes that are no longer alive.
//! A crashed separate instance is therefore not offered for recovery.

use crate::windows::{CurrentWindow, Windows, MAIN};
use crate::{err, AppState, CmdResult, State};
use imt_core::project::ProjectFile;
use imt_core::{Plugin, PluginRegistry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Command-line flag that starts IMT with built-in importers only and the default profile.
pub const SAFE_MODE_FLAG: &str = "--safe-mode";
const MARKER: &str = "running.marker";
const PLUGINS_FILE: &str = "plugins.json";
/// File name of an autosaved session (the UI recognises sessions by it).
const SESSION_FILE: &str = "last-session.imtproj";
const SESSIONS_DIR: &str = "sessions";
const RECOVERED_DIR: &str = "recovered";
const MANIFEST: &str = "manifest.json";
const SESSIONS_VERSION: u32 = 2;
const MARKER_VERSION: u32 = 1;
/// Folder of the separate instances' state, one `<pid>` folder each.
const INSTANCES_DIR: &str = "instances";
/// At most this many instance folders are inspected per start (bounded work).
const MAX_INSTANCE_DIRS: usize = 256;
/// Start times are whole seconds; allow for rounding between reads.
const START_TOLERANCE_SECS: u64 = 1;

/// Which state folder a process uses (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceScope {
    /// The single instance: the data folder itself.
    Main,
    /// `--new-instance` or safe mode: `instances/<pid>/`.
    Separate,
}

impl InstanceScope {
    /// The main scope for the process that holds (or wants) the single-instance lock.
    pub fn of(single_instance: bool) -> InstanceScope {
        if single_instance {
            InstanceScope::Main
        } else {
            InstanceScope::Separate
        }
    }
}

/// Returns the start time (seconds since the Unix epoch) of the process with
/// this PID, or `None` when no such process exists.
pub type Probe<'a> = &'a dyn Fn(u32) -> Option<u64>;

/// The running process recorded in `running.marker`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub pid: u32,
    /// `None` for a version 0 marker (a bare PID) or when the OS did not tell.
    pub started_at: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarkerJson {
    version: u32,
    pid: u32,
    #[serde(default)]
    started_at: Option<u64>,
}

/// What is on disk where the marker lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerFile {
    Missing,
    /// Present but not a marker (truncated write, garbage).
    Unreadable,
    Found(Marker),
}

impl MarkerFile {
    /// Parses a marker's text: version 1 JSON (later versions too, when they
    /// still carry a `pid`) or a version 0 bare PID.
    pub fn parse(text: &str) -> MarkerFile {
        let text = text.trim();
        if let Ok(pid) = text.parse::<u32>() {
            return MarkerFile::Found(Marker { pid, started_at: None });
        }
        match serde_json::from_str::<MarkerJson>(text) {
            Ok(m) if m.version >= 1 => MarkerFile::Found(Marker {
                pid: m.pid,
                started_at: m.started_at,
            }),
            _ => MarkerFile::Unreadable,
        }
    }

    fn read(path: &Path) -> MarkerFile {
        match std::fs::read(path) {
            Ok(bytes) => MarkerFile::parse(&String::from_utf8_lossy(&bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => MarkerFile::Missing,
            Err(_) => MarkerFile::Unreadable,
        }
    }
}

fn write_marker(dir: &Path, m: &Marker) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = MarkerJson {
        version: MARKER_VERSION,
        pid: m.pid,
        started_at: m.started_at,
    };
    let tmp = dir.join(format!("{MARKER}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(&json)?)?;
    std::fs::rename(tmp, dir.join(MARKER))?;
    Ok(())
}

/// How the run that left a marker ended, as far as this start can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviousRun {
    /// No marker: it shut down cleanly (or never ran).
    Clean,
    /// A marker whose process is gone (or an unreadable marker).
    Crashed,
    /// The marker's process is still alive (another instance, or one still exiting).
    StillRunning,
}

/// Whether the marker's process is alive: the PID exists and, when the marker
/// recorded it, with the same start time (else the PID was reused).
fn alive(m: &Marker, probe: Probe<'_>) -> bool {
    match (probe(m.pid), m.started_at) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(now), Some(then)) => now.abs_diff(then) <= START_TOLERANCE_SECS,
    }
}

/// Decides how the previous run ended. A marker with this process's own PID
/// is necessarily from a dead process. Pure: the OS is reached through `probe`.
pub fn previous_run(marker: MarkerFile, own_pid: u32, probe: Probe<'_>) -> PreviousRun {
    match marker {
        MarkerFile::Missing => PreviousRun::Clean,
        MarkerFile::Unreadable => PreviousRun::Crashed,
        MarkerFile::Found(m) if m.pid != own_pid && alive(&m, probe) => PreviousRun::StillRunning,
        MarkerFile::Found(_) => PreviousRun::Crashed,
    }
}

/// The OS's process table (the production [`Probe`]).
pub fn os_probe(pid: u32) -> Option<u64> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let pid = Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), false, ProcessRefreshKind::nothing());
    sys.process(pid).map(|p| p.start_time())
}

/// Removes the folders of separate instances that are no longer alive (and any
/// left with this process's PID, which can only be stale). Folders whose name
/// is not a PID are left alone. Returns how many were removed. Never fails.
pub fn prune_instances(data: &Path, own_pid: u32, probe: Probe<'_>) -> usize {
    let Ok(entries) = std::fs::read_dir(data.join(INSTANCES_DIR)) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten().take(MAX_INSTANCE_DIRS) {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = entry.path();
        let live = pid != own_pid
            && match MarkerFile::read(&path.join(MARKER)) {
                MarkerFile::Found(m) if m.pid == pid => alive(&m, probe),
                // Being created right now, or damaged: decide by the PID alone.
                _ => probe(pid).is_some(),
            };
        if !live && std::fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// How this run started; managed as Tauri state.
pub struct Lifecycle {
    pub safe_mode: bool,
    /// IMT did not shut down cleanly last time (crash, kill, power loss).
    pub previous_crash: bool,
    /// This instance's state folder (see [`InstanceScope`]).
    dir: Option<PathBuf>,
    scope: InstanceScope,
    /// What this process wrote to its marker.
    own: Marker,
    sessions: Option<SessionStore>,
    /// `restore_windows` ran in this run.
    restored: AtomicBool,
}

impl Lifecycle {
    /// [`Lifecycle::begin_in`] for the main instance.
    #[cfg(test)]
    pub fn begin(data: Option<PathBuf>, safe_mode: bool) -> Lifecycle {
        Lifecycle::begin_in(data, safe_mode, InstanceScope::Main)
    }

    /// Records that IMT is running; detects whether the previous run ended
    /// uncleanly and keeps the previous run's sessions for recovery.
    pub fn begin_in(data: Option<PathBuf>, safe_mode: bool, scope: InstanceScope) -> Lifecycle {
        Lifecycle::begin_with(data, safe_mode, scope, std::process::id(), &os_probe)
    }

    /// [`Lifecycle::begin_in`] for a given PID and process table (tests).
    pub fn begin_with(data: Option<PathBuf>, safe_mode: bool, scope: InstanceScope, own_pid: u32, probe: Probe<'_>) -> Lifecycle {
        let own = Marker {
            pid: own_pid,
            started_at: probe(own_pid),
        };
        if let Some(d) = &data {
            let pruned = prune_instances(d, own_pid, probe);
            if pruned > 0 {
                log::info!(Folders = pruned as u64; "Removed the state of instances that are no longer running");
            }
        }
        let dir = data.map(|d| match scope {
            InstanceScope::Main => d,
            InstanceScope::Separate => d.join(INSTANCES_DIR).join(own_pid.to_string()),
        });
        let mut previous_crash = false;
        if let Some(d) = &dir {
            match previous_run(MarkerFile::read(&d.join(MARKER)), own_pid, probe) {
                PreviousRun::Clean => {}
                PreviousRun::Crashed => previous_crash = true,
                PreviousRun::StillRunning => {
                    log::info!("Another IMT process still holds the running marker; taking it over")
                }
            }
            if let Err(e) = write_marker(d, &own) {
                log::warn!("could not record that IMT is running: {e:#}");
            }
        }
        let sessions = dir.as_ref().map(|d| SessionStore::new(d.clone()));
        if let Some(s) = &sessions {
            s.rotate();
        }
        Lifecycle {
            safe_mode,
            previous_crash,
            dir,
            scope,
            own,
            sessions,
            restored: AtomicBool::new(false),
        }
    }

    /// Clean shutdown: the next start will not report a crash. A separate
    /// instance removes its whole folder; the main instance removes the marker
    /// unless a newer process has taken it over.
    pub fn end(&self) {
        let Some(d) = &self.dir else { return };
        match self.scope {
            InstanceScope::Separate => {
                let _ = std::fs::remove_dir_all(d);
            }
            InstanceScope::Main => {
                if MarkerFile::read(&d.join(MARKER)) == MarkerFile::Found(self.own) {
                    let _ = std::fs::remove_file(d.join(MARKER));
                }
            }
        }
    }

    /// The session a window can restore or reopen: its own autosave of this
    /// run, else (for the main window) the previous run's main window.
    fn session_path(&self, label: &str) -> Option<PathBuf> {
        self.sessions.as_ref().and_then(|s| s.last_session(label))
    }

    /// A window was closed by the user: it is not restored after a crash.
    pub fn forget_session(&self, label: &str) {
        if let Some(s) = &self.sessions {
            s.forget(label);
        }
    }
}

/// The session folders (see the module docs). Plain files; testable without Tauri.
pub struct SessionStore {
    dir: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct SessionManifest {
    version: u32,
    #[serde(default)]
    windows: Vec<String>,
}

/// Window labels become folder names: only `[A-Za-z0-9_-]`, 1..=64 characters.
fn valid_label(label: &str) -> bool {
    !label.is_empty() && label.len() <= 64 && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl SessionStore {
    pub fn new(dir: PathBuf) -> SessionStore {
        SessionStore { dir }
    }

    fn file(&self, folder: &str, label: &str) -> Option<PathBuf> {
        valid_label(label).then(|| self.dir.join(folder).join(label).join(SESSION_FILE))
    }

    /// The windows listed in a manifest; a missing or corrupt one lists the session folders instead.
    fn listed(&self, folder: &str) -> Vec<String> {
        let root = self.dir.join(folder);
        let manifest = std::fs::read_to_string(root.join(MANIFEST))
            .ok()
            .and_then(|t| serde_json::from_str::<SessionManifest>(&t).ok());
        let labels = match manifest {
            Some(m) => m.windows,
            None => {
                let mut found: Vec<String> = std::fs::read_dir(&root)
                    .map(|r| r.flatten().filter_map(|e| e.file_name().to_str().map(String::from)).collect())
                    .unwrap_or_default();
                found.sort_by_key(|l| crate::windows::label_order(l));
                found
            }
        };
        labels
            .into_iter()
            .filter(|l| self.file(folder, l).is_some_and(|f| f.is_file()))
            .collect()
    }

    fn write_manifest(&self, folder: &str, windows: Vec<String>) -> anyhow::Result<()> {
        let root = self.dir.join(folder);
        std::fs::create_dir_all(&root)?;
        let tmp = root.join(format!("{MANIFEST}.tmp"));
        let manifest = SessionManifest {
            version: SESSIONS_VERSION,
            windows,
        };
        std::fs::write(&tmp, serde_json::to_vec_pretty(&manifest)?)?;
        std::fs::rename(tmp, root.join(MANIFEST))?;
        Ok(())
    }

    /// At start: the previous run's sessions become the recovered ones and the
    /// current run starts with an empty manifest. Session files themselves stay
    /// (a restart reopens them by path). Never fails.
    pub fn rotate(&self) {
        let sessions = self.dir.join(SESSIONS_DIR);
        let has_manifest = sessions.join(MANIFEST).exists();
        let mut previous = self.listed(SESSIONS_DIR);
        let legacy = self.dir.join(SESSION_FILE);
        let recovered = self.dir.join(RECOVERED_DIR);
        let _ = std::fs::remove_dir_all(&recovered);
        let mut copied = Vec::new();
        for label in previous.drain(..) {
            if let (Some(from), Some(to)) = (self.file(SESSIONS_DIR, &label), self.file(RECOVERED_DIR, &label)) {
                if to.parent().is_some_and(|p| std::fs::create_dir_all(p).is_ok()) && std::fs::copy(from, to).is_ok() {
                    copied.push(label);
                }
            }
        }
        // Version 1: one session, the main window's.
        if !has_manifest && copied.is_empty() && legacy.is_file() {
            if let Some(to) = self.file(RECOVERED_DIR, MAIN) {
                if to.parent().is_some_and(|p| std::fs::create_dir_all(p).is_ok()) && std::fs::copy(&legacy, to).is_ok() {
                    copied.push(MAIN.to_string());
                }
            }
        }
        if let Err(e) = self
            .write_manifest(RECOVERED_DIR, copied)
            .and_then(|_| self.write_manifest(SESSIONS_DIR, Vec::new()))
        {
            log::warn!("could not prepare session recovery: {e:#}");
        }
    }

    /// Autosaves a window's session and lists the window in this run's manifest.
    pub fn save(&self, label: &str, project: &ProjectFile) -> anyhow::Result<PathBuf> {
        let path = self.file(SESSIONS_DIR, label).ok_or_else(|| anyhow::anyhow!("invalid window"))?;
        std::fs::create_dir_all(path.parent().ok_or_else(|| anyhow::anyhow!("invalid session folder"))?)?;
        project.save(&path)?;
        let mut windows = self.current_manifest();
        if !windows.iter().any(|w| w == label) {
            windows.push(label.to_string());
            self.write_manifest(SESSIONS_DIR, windows)?;
        }
        Ok(path)
    }

    fn current_manifest(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join(SESSIONS_DIR).join(MANIFEST))
            .ok()
            .and_then(|t| serde_json::from_str::<SessionManifest>(&t).ok())
            .map(|m| m.windows)
            .unwrap_or_default()
    }

    /// Removes a closed window's session.
    pub fn forget(&self, label: &str) {
        let Some(file) = self.file(SESSIONS_DIR, label) else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
        let mut windows = self.current_manifest();
        let before = windows.len();
        windows.retain(|w| w != label);
        if windows.len() != before {
            let _ = self.write_manifest(SESSIONS_DIR, windows);
        }
    }

    /// The previous run's sessions, `main` first.
    pub fn recovered(&self) -> Vec<(String, PathBuf)> {
        self.listed(RECOVERED_DIR)
            .into_iter()
            .filter_map(|l| self.file(RECOVERED_DIR, &l).map(|f| (l, f)))
            .collect()
    }

    pub fn last_session(&self, label: &str) -> Option<PathBuf> {
        self.file(SESSIONS_DIR, label).filter(|f| f.is_file()).or_else(|| {
            (label == MAIN)
                .then(|| self.file(RECOVERED_DIR, MAIN))
                .flatten()
                .filter(|f| f.is_file())
        })
    }
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginSettings {
    #[serde(default)]
    pub disabled: Vec<String>,
}

pub fn load_plugin_settings(dir: Option<&Path>) -> PluginSettings {
    dir.and_then(|d| std::fs::read_to_string(d.join(PLUGINS_FILE)).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_plugin_settings(dir: &Path, s: &PluginSettings) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(PLUGINS_FILE), serde_json::to_string_pretty(s)?)?;
    Ok(())
}

/// Every compiled-in plugin. Add new plugins here; install order does not
/// matter (activation follows the dependency graph).
pub fn builtin_plugins() -> Vec<Arc<dyn Plugin>> {
    vec![
        Arc::new(imt_plugin_registry::RegistryPlugin),
        Arc::new(imt_plugin_otel::OtelPlugin),
        Arc::new(imt_plugin_syslog::SyslogPlugin),
        Arc::new(imt_plugin_windows::WindowsPlugin),
        Arc::new(imt_plugin_web::WebPlugin),
    ]
}

/// The engine's plugins: none in safe mode, else every compiled-in plugin
/// except those the user disabled, resolved (dependencies checked, activation
/// hooks run in dependency order).
pub fn plugins(safe_mode: bool, disabled: &[String]) -> anyhow::Result<PluginRegistry> {
    install(builtin_plugins(), safe_mode, disabled)
}

/// Installs `all` and resolves them. A plugin that fails to install does not
/// stop IMT: the error is logged, and a plugin whose registration failed is
/// listed as inactive with the reason (so are its dependents).
fn install(all: Vec<Arc<dyn Plugin>>, safe_mode: bool, disabled: &[String]) -> anyhow::Result<PluginRegistry> {
    let mut registry = PluginRegistry::new();
    if safe_mode {
        return Ok(registry);
    }
    registry.disable(disabled.iter().cloned());
    for plugin in all {
        let id = plugin.id();
        if let Err(e) = registry.install_shared(plugin) {
            log::error!(Plugin = id; "Plugin not installed: {e:#}");
        }
    }
    registry.resolve();
    for p in registry.plugins() {
        if let Some(reason) = &p.inactive_reason {
            log::warn!(Plugin = p.id.as_str(); "Plugin inactive: {reason}");
        }
        for w in &p.warnings {
            log::info!(Plugin = p.id.as_str(); "Plugin note: {w}");
        }
    }
    Ok(registry)
}

/// Logs panics (message and location) to IMT's own CLEF log before the default handler runs.
pub fn install_panic_logging() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        log::error!(Location = location.as_str(); "IMT crashed: {}", imt_core::guard::panic_message(info.payload()));
        previous(info);
    }));
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub version: &'static str,
    pub safe_mode: bool,
    pub previous_crash: bool,
    pub session_available: bool,
}

/// The calling window's start-up facts. Only the main window offers crash recovery.
#[tauri::command]
pub fn app_status(state: CurrentWindow, lifecycle: State<'_, Lifecycle>) -> AppStatus {
    AppStatus {
        version: env!("CARGO_PKG_VERSION"),
        safe_mode: lifecycle.safe_mode,
        previous_crash: lifecycle.previous_crash && state.label() == MAIN,
        session_available: lifecycle.session_path(state.label()).is_some(),
    }
}

/// Relaunches IMT, optionally in safe mode, reopening `reopen` paths.
#[tauri::command]
pub fn restart_app<R: tauri::Runtime>(app: tauri::AppHandle<R>, safe_mode: bool, reopen: Vec<String>) -> CmdResult<()> {
    let exe = std::env::current_exe().map_err(err)?;
    let mut cmd = std::process::Command::new(exe);
    if safe_mode {
        cmd.arg(SAFE_MODE_FLAG);
    }
    // The new process must become the running instance, not hand over to this one.
    crate::instance::release(&app);
    cmd.args(&reopen).spawn().map_err(err)?;
    log::info!(SafeMode = safe_mode; "Restarting");
    crate::window_manager::quit(&app);
    Ok(())
}

/// Lifts a plugin's quarantine (after repeated crashes) immediately, in every window.
#[tauri::command]
pub fn plugin_reset(windows: State<'_, Windows>, id: String) -> CmdResult<()> {
    for w in windows.all() {
        w.store_read().reset_plugin(&id);
    }
    log::info!(Plugin = id.as_str(); "Plugin re-enabled after quarantine");
    Ok(())
}

/// Enables or disables a plugin from the next start (returns `true`: restart needed).
#[tauri::command]
pub fn plugin_set_enabled(state: State<'_, AppState>, id: String, enabled: bool) -> CmdResult<bool> {
    let dir = state.config_dir.as_ref().ok_or("no configuration folder")?;
    let mut s = load_plugin_settings(Some(dir));
    s.disabled.retain(|d| *d != id);
    if !enabled {
        s.disabled.push(id.clone());
    }
    save_plugin_settings(dir, &s).map_err(err)?;
    log::info!(Plugin = id.as_str(), Enabled = enabled; "Plugin setting changed");
    Ok(true)
}

/// Autosaves the calling window's session (open files, profile, UI state) for crash recovery.
#[tauri::command]
pub fn save_session(state: CurrentWindow, lifecycle: State<'_, Lifecycle>, ui: Value) -> CmdResult<()> {
    let Some(store) = &lifecycle.sessions else { return Ok(()) };
    let project = crate::workspace_commands::project_of(&state, "Last session".into(), ui);
    store.save(state.label(), &project).map(|_| ()).map_err(err)
}

/// Path of the calling window's autosaved session (open it with `open_project`), if any.
#[tauri::command]
pub fn last_session(state: CurrentWindow, lifecycle: State<'_, Lifecycle>) -> Option<String> {
    lifecycle.session_path(state.label()).map(|p| p.to_string_lossy().into_owned())
}

/// After a crash: reopens the previous run's other windows, each with its
/// session queued (it opens it on startup). Once per run; returns how many opened.
#[tauri::command]
pub fn restore_windows<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: CurrentWindow,
    lifecycle: State<'_, Lifecycle>,
) -> CmdResult<usize> {
    let Some(store) = &lifecycle.sessions else { return Ok(0) };
    if lifecycle.restored.swap(true, Ordering::SeqCst) {
        return Ok(0);
    }
    let mut opened = 0;
    for (label, path) in store.recovered() {
        if label == MAIN || label == state.label() {
            continue;
        }
        match crate::window_manager::open_new(&app, vec![path.to_string_lossy().into_owned()]) {
            Ok(_) => opened += 1,
            Err(e) => log::warn!("could not restore a window: {e:#}"),
        }
    }
    log::info!(Windows = opened as u64; "Windows restored after a crash");
    Ok(opened)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Needy(&'static str, Option<&'static str>);
    impl Plugin for Needy {
        fn id(&self) -> &'static str {
            self.0
        }
        fn name(&self) -> &'static str {
            self.0
        }
        fn version(&self) -> &'static str {
            "1.0.0"
        }
        fn manifest(&self) -> imt_core::plugin::Manifest {
            imt_core::plugin::Manifest::new(self.0, "1.0.0").requires("t.missing", "^1")
        }
        fn register(&self, _: &mut PluginRegistry) -> anyhow::Result<()> {
            match self.1 {
                Some(e) => Err(anyhow::anyhow!(e)),
                None => Ok(()),
            }
        }
    }

    #[test]
    fn broken_or_unresolved_plugins_do_not_stop_startup() {
        let registry = install(
            vec![
                Arc::new(Needy("t.needy", None)),
                Arc::new(Needy("t.broken", Some("bad file"))),
                Arc::new(Needy("t.needy", None)),
            ],
            false,
            &[],
        )
        .unwrap();
        let all = registry.plugins();
        assert_eq!(all.len(), 2, "the duplicate is skipped");
        assert_eq!(all[0].inactive_reason.as_deref(), Some("needs t.missing ^1 (not installed)"));
        assert!(all[1].inactive_reason.as_deref().unwrap().starts_with("registration failed"));
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("imt-life-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn detects_unclean_shutdown() {
        let d = temp("marker");
        let first = Lifecycle::begin(Some(d.clone()), false);
        assert!(!first.previous_crash);
        // No end(): simulates a crash.
        let second = Lifecycle::begin(Some(d.clone()), true);
        assert!(second.previous_crash && second.safe_mode);
        second.end();
        assert!(!Lifecycle::begin(Some(d.clone()), false).previous_crash);
        assert!(!Lifecycle::begin(None, false).previous_crash);
        Lifecycle::begin(None, false).end();
        std::fs::remove_dir_all(d).unwrap();
    }

    /// A fake process table: PID → start time.
    fn table(procs: &[(u32, u64)]) -> impl Fn(u32) -> Option<u64> + '_ {
        move |pid| procs.iter().find(|(p, _)| *p == pid).map(|(_, t)| *t)
    }

    fn marker(pid: u32, started_at: Option<u64>) -> MarkerFile {
        MarkerFile::Found(Marker { pid, started_at })
    }

    #[test]
    fn previous_run_tells_crash_from_a_live_process() {
        let procs = [(10, 1_000), (20, 2_000)];
        let probe = table(&procs);
        let own = 99;
        assert_eq!(previous_run(MarkerFile::Missing, own, &probe), PreviousRun::Clean);
        assert_eq!(
            previous_run(marker(10, Some(1_000)), own, &probe),
            PreviousRun::StillRunning,
            "live PID, same start"
        );
        assert_eq!(
            previous_run(marker(10, Some(1_001)), own, &probe),
            PreviousRun::StillRunning,
            "within rounding"
        );
        assert_eq!(previous_run(marker(30, Some(1_000)), own, &probe), PreviousRun::Crashed, "dead PID");
        assert_eq!(
            previous_run(marker(20, Some(1_000)), own, &probe),
            PreviousRun::Crashed,
            "PID reused later"
        );
        assert_eq!(
            previous_run(marker(own, Some(5)), own, &probe),
            PreviousRun::Crashed,
            "this process's PID"
        );
        assert_eq!(previous_run(marker(own, None), own, &probe), PreviousRun::Crashed);
        // Version 0 (bare PID): a live PID cannot be told from reuse; no false crash.
        assert_eq!(previous_run(marker(10, None), own, &probe), PreviousRun::StillRunning);
        assert_eq!(previous_run(marker(30, None), own, &probe), PreviousRun::Crashed);
        assert_eq!(previous_run(MarkerFile::Unreadable, own, &probe), PreviousRun::Crashed);
    }

    #[test]
    fn markers_parse_old_new_and_corrupt_text() {
        assert_eq!(MarkerFile::parse("1234\n"), marker(1234, None), "version 0");
        assert_eq!(MarkerFile::parse(r#"{"version":1,"pid":7,"startedAt":9}"#), marker(7, Some(9)));
        assert_eq!(MarkerFile::parse(r#"{"version":1,"pid":7}"#), marker(7, None));
        assert_eq!(
            MarkerFile::parse(r#"{"version":3,"pid":7,"startedAt":9,"x":1}"#),
            marker(7, Some(9)),
            "newer"
        );
        for bad in [
            "",
            "{",
            "-1",
            "99999999999",
            r#"{"version":0,"pid":7}"#,
            r#"{"version":1}"#,
            "\u{0}\u{1}",
        ] {
            assert_eq!(MarkerFile::parse(bad), MarkerFile::Unreadable, "{bad:?}");
        }
        let d = temp("marker-file");
        assert_eq!(MarkerFile::read(&d.join(MARKER)), MarkerFile::Missing);
        std::fs::create_dir_all(d.join(MARKER)).unwrap(); // a folder where the file should be
        assert_eq!(MarkerFile::read(&d.join(MARKER)), MarkerFile::Unreadable);
        std::fs::remove_dir_all(&d).unwrap();
        // Round trip.
        write_marker(
            &d,
            &Marker {
                pid: 5,
                started_at: Some(6),
            },
        )
        .unwrap();
        assert_eq!(MarkerFile::read(&d.join(MARKER)), marker(5, Some(6)));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn the_os_probe_sees_this_process_only_while_it_runs() {
        let started = os_probe(std::process::id()).expect("this process exists");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(started <= now + START_TOLERANCE_SECS && now - started < 24 * 3600);
        assert_eq!(os_probe(std::process::id()), Some(started), "stable between reads");
        assert_eq!(os_probe(4_000_000_000), None);
    }

    #[test]
    fn a_live_previous_process_is_not_a_crash_and_keeps_its_marker_safe() {
        let d = temp("restart");
        let procs = [(10, 1_000), (11, 1_100)];
        let probe = table(&procs);
        // Process 10 runs; process 11 starts while 10 is still exiting ("Restart").
        let old = Lifecycle::begin_with(Some(d.clone()), false, InstanceScope::Main, 10, &probe);
        let new = Lifecycle::begin_with(Some(d.clone()), false, InstanceScope::Main, 11, &probe);
        assert!(!new.previous_crash);
        // The old process exits: the marker is the new one's and stays.
        old.end();
        assert_eq!(MarkerFile::read(&d.join(MARKER)), marker(11, Some(1_100)));
        new.end();
        assert_eq!(MarkerFile::read(&d.join(MARKER)), MarkerFile::Missing);
        // A crash of 11, then its PID reused by an unrelated process: reported.
        let crashed = Lifecycle::begin_with(Some(d.clone()), false, InstanceScope::Main, 11, &probe);
        drop(crashed);
        let reused = [(11, 5_000), (12, 6_000)];
        let later = Lifecycle::begin_with(Some(d.clone()), false, InstanceScope::Main, 12, &table(&reused));
        assert!(later.previous_crash);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn separate_instances_keep_their_own_state() {
        let d = temp("instances");
        let procs = [(10, 1_000), (20, 2_000)];
        let probe = table(&procs);
        // The main instance (10) has a session and is running.
        let main = Lifecycle::begin_with(Some(d.clone()), false, InstanceScope::Main, 10, &probe);
        main.sessions.as_ref().unwrap().save(MAIN, &project("main work")).unwrap();
        let manifest = std::fs::read(d.join(SESSIONS_DIR).join(MANIFEST)).unwrap();

        // A --new-instance / safe-mode process (20).
        let second = Lifecycle::begin_with(Some(d.clone()), true, InstanceScope::Separate, 20, &probe);
        let own = d.join(INSTANCES_DIR).join("20");
        assert_eq!(second.dir.as_deref(), Some(own.as_path()));
        assert!(!second.previous_crash, "the main instance's marker is not this one's");
        assert_eq!(MarkerFile::read(&own.join(MARKER)), marker(20, Some(2_000)));
        second.sessions.as_ref().unwrap().save(MAIN, &project("second work")).unwrap();
        assert_eq!(name_of(&own.join(SESSIONS_DIR).join(MAIN).join(SESSION_FILE)), "second work");
        // The main instance's sessions and marker are untouched.
        assert_eq!(std::fs::read(d.join(SESSIONS_DIR).join(MANIFEST)).unwrap(), manifest);
        assert_eq!(name_of(&main.session_path(MAIN).unwrap()), "main work");
        assert_eq!(MarkerFile::read(&d.join(MARKER)), marker(10, Some(1_000)));

        // A clean exit removes the folder.
        second.end();
        assert!(!own.exists());
        main.end();
        assert!(!Lifecycle::begin_with(Some(d.clone()), false, InstanceScope::Main, 10, &probe).previous_crash);
        // Without a data folder nothing is written.
        let none = Lifecycle::begin_with(None, false, InstanceScope::Separate, 20, &probe);
        assert!(none.dir.is_none());
        none.end();
        assert_eq!(InstanceScope::of(true), InstanceScope::Main);
        assert_eq!(InstanceScope::of(false), InstanceScope::Separate);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn stale_instance_folders_are_pruned_live_ones_kept() {
        let d = temp("prune");
        let instances = d.join(INSTANCES_DIR);
        let procs = [(10, 1_000), (20, 2_000), (30, 3_000), (99, 9_000)];
        let probe = table(&procs);
        let with_marker = |pid: u32, m: Option<Marker>| {
            let dir = instances.join(pid.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            if let Some(m) = m {
                write_marker(&dir, &m).unwrap();
            }
        };
        with_marker(
            10,
            Some(Marker {
                pid: 10,
                started_at: Some(1_000),
            }),
        ); // alive
        with_marker(
            20,
            Some(Marker {
                pid: 20,
                started_at: Some(1_500),
            }),
        ); // PID reused
        with_marker(
            40,
            Some(Marker {
                pid: 40,
                started_at: Some(4_000),
            }),
        ); // dead
        with_marker(30, None); // no marker yet, process alive
        with_marker(50, None); // no marker, process gone
        with_marker(
            99,
            Some(Marker {
                pid: 99,
                started_at: Some(9_000),
            }),
        ); // this process's PID: stale
        std::fs::create_dir_all(instances.join("notes")).unwrap(); // not ours to judge
        std::fs::write(instances.join("60"), "a file").unwrap(); // not a folder

        assert_eq!(prune_instances(&d, 99, &probe), 4);
        let mut left: Vec<String> = std::fs::read_dir(&instances)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["10", "30", "60", "notes"]);
        // Every start prunes, and a new separate instance never inherits a stale folder.
        with_marker(
            40,
            Some(Marker {
                pid: 40,
                started_at: Some(4_000),
            }),
        );
        std::fs::create_dir_all(instances.join("99").join(SESSIONS_DIR).join(MAIN)).unwrap();
        project("stale")
            .save(&instances.join("99").join(SESSIONS_DIR).join(MAIN).join(SESSION_FILE))
            .unwrap();
        let life = Lifecycle::begin_with(Some(d.clone()), false, InstanceScope::Separate, 99, &probe);
        assert!(!instances.join("40").exists());
        assert!(!life.previous_crash && life.session_path(MAIN).is_none());
        assert_eq!(prune_instances(&temp("prune-none"), 1, &probe), 0, "no folder");
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn plugin_settings_and_safe_mode() {
        let d = temp("plugins");
        assert_eq!(load_plugin_settings(Some(&d)), PluginSettings::default());
        save_plugin_settings(
            &d,
            &PluginSettings {
                disabled: vec!["net.decryptology.web".into()],
            },
        )
        .unwrap();
        let s = load_plugin_settings(Some(&d));
        let registry = plugins(false, &s.disabled).unwrap();
        let web = registry.plugins().into_iter().find(|p| p.id == "net.decryptology.web").unwrap();
        assert_eq!(web.status, imt_core::plugin::PluginStatus::Disabled);
        assert!(plugins(true, &[]).unwrap().plugins().is_empty());
        let all = plugins(false, &[]).unwrap().plugins();
        assert_eq!(all.len(), builtin_plugins().len());
        assert!(all
            .iter()
            .all(|p| p.status == imt_core::plugin::PluginStatus::Active && p.inactive_reason.is_none()));
        std::fs::write(d.join(PLUGINS_FILE), "not json").unwrap();
        assert_eq!(load_plugin_settings(Some(&d)), PluginSettings::default());
        std::fs::remove_dir_all(d).unwrap();
    }

    fn project(name: &str) -> ProjectFile {
        ProjectFile {
            version: 1,
            name: name.into(),
            sources: vec![],
            removed: vec![],
            profile: "name = 'x'".into(),
            ui: Value::Null,
        }
    }

    fn name_of(path: &Path) -> String {
        ProjectFile::load(path).unwrap().name
    }

    #[test]
    fn sessions_per_window_survive_a_crash() {
        let d = temp("sessions");
        let store = SessionStore::new(d.clone());
        store.rotate();
        assert!(store.recovered().is_empty() && store.last_session(MAIN).is_none());
        store.save(MAIN, &project("main 1")).unwrap();
        store.save("w-2", &project("second")).unwrap();
        store.save("w-3", &project("third")).unwrap();
        store.save("w-2", &project("second, later")).unwrap();
        // The user closes w-3: it is not restored.
        store.forget("w-3");
        store.forget("w-3");
        assert!(store.save("../evil", &project("x")).is_err());
        store.forget("../evil");

        // Next start (after a crash): the previous run's windows are recovered.
        let next = SessionStore::new(d.clone());
        next.rotate();
        let recovered = next.recovered();
        let labels: Vec<&str> = recovered.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["main", "w-2"]);
        assert_eq!(name_of(&recovered[1].1), "second, later");
        // Main offers the recovered session until it saves its own.
        assert_eq!(name_of(&next.last_session(MAIN).unwrap()), "main 1");
        assert!(next.last_session("w-9").is_none());
        next.save(MAIN, &project("main 2")).unwrap();
        assert_eq!(name_of(&next.last_session(MAIN).unwrap()), "main 2");
        // The recovered copy is not overwritten by the new run.
        assert_eq!(name_of(&next.recovered()[0].1), "main 1");
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn sessions_tolerate_old_and_corrupt_files() {
        // Version 1: a single last-session.imtproj is the main window's.
        let d = temp("legacy");
        std::fs::create_dir_all(&d).unwrap();
        project("old single session").save(&d.join(SESSION_FILE)).unwrap();
        let store = SessionStore::new(d.clone());
        store.rotate();
        let recovered = store.recovered();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].0, MAIN);
        assert_eq!(name_of(&store.last_session(MAIN).unwrap()), "old single session");
        // Once the new layout exists, the legacy file is not recovered again.
        store.rotate();
        assert!(store.recovered().is_empty());

        // A corrupt manifest: the session folders are listed instead.
        store.save("w-4", &project("four")).unwrap();
        store.save(MAIN, &project("main")).unwrap();
        std::fs::write(d.join(SESSIONS_DIR).join(MANIFEST), "{ not json").unwrap();
        std::fs::create_dir_all(d.join(SESSIONS_DIR).join("w-5")).unwrap(); // no session file in it
        store.rotate();
        let labels: Vec<String> = store.recovered().into_iter().map(|(l, _)| l).collect();
        assert_eq!(labels, ["main", "w-4"]);
        // A manifest naming a missing or invalid window lists only real sessions.
        std::fs::write(
            d.join(RECOVERED_DIR).join(MANIFEST),
            r#"{"version":2,"windows":["main","gone","../x"]}"#,
        )
        .unwrap();
        assert_eq!(store.recovered().len(), 1);
        std::fs::remove_dir_all(d).unwrap();
        // No folder at all: nothing, and nothing fails.
        let none = SessionStore::new(temp("none").join("missing"));
        assert!(none.recovered().is_empty());
    }

    #[test]
    fn session_commands_per_window() {
        use crate::test_support::{block, main_window, samples, test_app, window};
        use tauri::Manager;
        let d = temp("session-cmds");
        // A previous run with two windows that crashed.
        {
            let store = SessionStore::new(d.clone());
            store.rotate();
            store.save(MAIN, &project("main")).unwrap();
            store.save("w-2", &project("second")).unwrap();
            // A process that no longer exists.
            std::fs::write(d.join(MARKER), r#"{"version":1,"pid":4000000000,"startedAt":1}"#).unwrap();
        }
        let app = test_app(None);
        app.manage(Lifecycle::begin(Some(d.clone()), false));
        let h = app.handle().clone();
        window(&app, MAIN);
        let life = || app.state::<Lifecycle>();
        let status = app_status(main_window(&app), life());
        assert!(status.previous_crash && status.session_available);
        // Another window does not offer crash recovery.
        let w2 = crate::windows::CurrentWindow::of(&h, "w-7").unwrap();
        let other = app_status(w2.clone(), life());
        assert!(!other.previous_crash && !other.session_available);
        assert!(last_session(w2.clone(), life()).is_none());
        assert!(last_session(main_window(&app), life()).unwrap().ends_with(SESSION_FILE));

        // Restoring reopens the other window with its session queued, once.
        assert_eq!(restore_windows(h.clone(), main_window(&app), life()).unwrap(), 1);
        assert_eq!(restore_windows(h.clone(), main_window(&app), life()).unwrap(), 0);
        let restored = crate::windows::CurrentWindow::of(&h, "w-2").unwrap().take_pending();
        assert_eq!(restored.len(), 1);
        assert_eq!(name_of(Path::new(&restored[0])), "second");

        // Autosave of this run, per window.
        let csv = samples().join("csv").to_string_lossy().into_owned();
        block(crate::workspace_commands::open_paths(h.clone(), main_window(&app), vec![csv])).unwrap();
        save_session(main_window(&app), life(), serde_json::json!({ "k": 1 })).unwrap();
        save_session(w2.clone(), life(), Value::Null).unwrap();
        let saved = ProjectFile::load(&last_session(main_window(&app), life()).map(PathBuf::from).unwrap()).unwrap();
        assert_eq!((saved.sources.len(), saved.ui["k"].as_i64()), (1, Some(1)));
        assert!(last_session(w2, life()).is_some());
        life().forget_session("w-7");
        // Without a data folder sessions are off.
        let app2 = test_app(None);
        app2.manage(Lifecycle::begin(None, false));
        save_session(main_window(&app2), app2.state::<Lifecycle>(), Value::Null).unwrap();
        assert_eq!(
            restore_windows(app2.handle().clone(), main_window(&app2), app2.state::<Lifecycle>()).unwrap(),
            0
        );
        app2.state::<Lifecycle>().forget_session(MAIN);
        std::fs::remove_dir_all(d).unwrap();
    }
}
