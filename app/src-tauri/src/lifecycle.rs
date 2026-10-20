//! Robustness around the app's lifetime: safe mode, detection of unclean
//! shutdowns, persisted plugin enable/disable, session autosave/restore and a
//! crash handler that records panics in IMT's own log.
//!
//! Sessions (format version 2): each window autosaves to
//! `sessions/<window>/last-session.imtproj`, and `sessions/manifest.json`
//! (`{ "version": 2, "windows": [...] }`) lists the windows of the current run.
//! At start the previous run's sessions are copied to `recovered/` (what a
//! crash restore offers) and the manifest is reset. Missing or corrupt files
//! fall back to "no session".
//!
//! Unclean shutdowns: `running.marker` (`{ "version": 1, "pid": .., "startedAt": .. }`)
//! names the process that is running. At start a marker left behind is a
//! crash only when its process is gone: no process with that PID, or one whose
//! start time differs (the PID was reused). A marker whose process still runs
//! (e.g. the previous instance is still exiting after "Restart") is taken over
//! without reporting a crash, and [`Lifecycle::end`] removes the marker only
//! while it is still this process's, so the old process cannot erase the new
//! one's. A marker without a start time (the OS did not tell) counts as running
//! while its PID is alive. An unreadable marker counts as a crash.
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
    /// `None` when the OS did not tell.
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
    /// still carry a `pid`).
    pub fn parse(text: &str) -> MarkerFile {
        let text = text.trim();
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
        let mut previous = self.listed(SESSIONS_DIR);
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
    // One list for the app and the `imt-plugin-host` binary that runs them isolated.
    imt_plugin_host::builtins::all()
}

/// The engine's plugins: none in safe mode, else every compiled-in plugin
/// except those the user disabled, resolved (dependencies checked, activation
/// hooks run in dependency order).
// The app installs `plugins_with` (it shares the tools plugin); tests use this.
#[cfg_attr(not(test), allow(dead_code))]
pub fn plugins(safe_mode: bool, disabled: &[String]) -> anyhow::Result<PluginRegistry> {
    plugins_with(safe_mode, disabled, &[])
}

/// [`plugins`] plus `shared`: plugins whose state is app-wide, so one value
/// is installed in every window's registry (the tools plugin, see `AppState`).
// The app installs `plugins_from` (isolated plugins replaced by their process); tests use this.
#[cfg_attr(not(test), allow(dead_code))]
pub fn plugins_with(safe_mode: bool, disabled: &[String], shared: &[Arc<dyn Plugin>]) -> anyhow::Result<PluginRegistry> {
    plugins_from(safe_mode, disabled, builtin_plugins(), shared)
}

/// [`plugins_with`] with the built-in plugins given (some may be plugin processes, see
/// `plugin_isolation`).
pub fn plugins_from(
    safe_mode: bool,
    disabled: &[String],
    builtins: Vec<Arc<dyn Plugin>>,
    shared: &[Arc<dyn Plugin>],
) -> anyhow::Result<PluginRegistry> {
    let mut all = builtins;
    all.extend(shared.iter().cloned());
    install(all, safe_mode, disabled)
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

/// Longest backtrace logged with a panic (bytes, cut at a line boundary).
pub const MAX_BACKTRACE: usize = 8 * 1024;
/// Panics logged with a backtrace per run (capturing one is slow; a panic loop must not stall IMT).
const MAX_BACKTRACES: u32 = 20;
static PANICS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// `text` cut to at most `max` bytes at a line boundary, with a note of what was left out.
pub fn bounded_lines(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut cut = max;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let cut = text[..cut].rfind('\n').unwrap_or(cut);
    format!("{}\n… ({} more bytes)", &text[..cut], text.len() - cut)
}

/// The panicking thread's name, or its id when it has none.
fn thread_name() -> String {
    let t = std::thread::current();
    t.name().map(str::to_string).unwrap_or_else(|| format!("{:?}", t.id()))
}

/// Longest one teardown step (closing a window, quitting) may hold the closing up; it goes on in the
/// background after that, and IMT closes anyway.
pub const TEARDOWN_STEP_LIMIT: std::time::Duration = std::time::Duration::from_secs(3);

/// How a teardown step ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepEnd {
    Done,
    /// Still running after the limit (left behind).
    TimedOut,
    /// Crashed (logged by the panic hook) or could not start.
    Failed,
}

/// Runs one step of closing a window or IMT on a thread of its own and waits at most `limit` for it,
/// logging (at Info) that it started and how it ended, so a hang in a log names its step. Dropping
/// window state, stopping plugins and jobs never runs on the main thread: they may wait for it.
pub fn teardown_step(step: &str, limit: std::time::Duration, f: impl FnOnce() + Send + 'static) -> StepEnd {
    log::info!(Step = step; "Teardown: {step}");
    let started = std::time::Instant::now();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let spawned = std::thread::Builder::new().name(format!("imt-teardown-{step}")).spawn(move || {
        f();
        let _ = tx.send(());
    });
    let end = match spawned.map(|_| rx.recv_timeout(limit)) {
        Ok(Ok(())) => StepEnd::Done,
        Ok(Err(std::sync::mpsc::RecvTimeoutError::Timeout)) => StepEnd::TimedOut,
        Ok(Err(std::sync::mpsc::RecvTimeoutError::Disconnected)) | Err(_) => StepEnd::Failed,
    };
    let ms = started.elapsed().as_millis() as u64;
    match end {
        StepEnd::Done => log::info!(Step = step, ElapsedMs = ms; "Teardown: {step} done"),
        StepEnd::TimedOut => log::warn!(Step = step, ElapsedMs = ms; "Teardown: {step} did not finish in time; IMT goes on closing"),
        StepEnd::Failed => log::warn!(Step = step, ElapsedMs = ms; "Teardown: {step} failed; IMT goes on closing"),
    }
    end
}

/// IMT exits (`RunEvent::Exit`): plugins stop, jobs are interrupted, the instance marker goes. Every step
/// is bounded ([`teardown_step`]), so a stuck plugin or job cannot keep the process alive.
pub fn exit_teardown<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    let a = app.clone();
    teardown_step("plugins", TEARDOWN_STEP_LIMIT, move || {
        if let Some(w) = a.try_state::<Windows>() {
            w.shutdown_plugins();
        }
    });
    let a = app.clone();
    teardown_step("jobs", TEARDOWN_STEP_LIMIT, move || crate::jobs::shutdown(&a));
    if let Some(life) = app.try_state::<Lifecycle>() {
        life.end();
        log::info!("Teardown: instance marker removed");
    }
}

/// Logs a panic on any thread (UI, Tauri's async runtime, rayon pools, watcher and event-bus threads,
/// helper threads): the panic hook is process-wide. Logged at Error with `Severity = "Fatal"`, the
/// thread, the location, a bounded backtrace and, inside a command, its window and operation id (the
/// activity logger attaches the breadcrumbs). Panics that a guard contains (plugin code) are logged too:
/// they are bugs, and the guard reports the containment separately. Then the previous hook runs.
pub fn install_panic_logging() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log_panic(info);
        previous(info);
    }));
}

fn log_panic(info: &std::panic::PanicHookInfo<'_>) {
    let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
    let thread = thread_name();
    let n = PANICS.fetch_add(1, Ordering::Relaxed);
    let backtrace = if n < MAX_BACKTRACES {
        bounded_lines(&std::backtrace::Backtrace::force_capture().to_string(), MAX_BACKTRACE)
    } else {
        "(not captured: too many panics in this run)".into()
    };
    let op = crate::activity::current_op().unwrap_or_default();
    let window = crate::activity::current_window().unwrap_or_default();
    log::error!(
        Severity = "Fatal", Thread = thread.as_str(), Location = location.as_str(), Backtrace = backtrace.as_str(),
        OperationId = op.as_str(), Window = window.as_str();
        "IMT crashed on thread {thread}: {}", imt_core::guard::panic_message(info.payload())
    );
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
pub async fn app_status(state: CurrentWindow, lifecycle: State<'_, Lifecycle>) -> CmdResult<AppStatus> {
    Ok(AppStatus {
        version: env!("CARGO_PKG_VERSION"),
        safe_mode: lifecycle.safe_mode,
        previous_crash: lifecycle.previous_crash && state.label() == MAIN,
        session_available: lifecycle.session_path(state.label()).is_some(),
    })
}

/// The relaunched process's command line, built here only (the webview sends no arguments): the flags,
/// then `--` and the session file to reopen. The session must be an existing file (canonicalised, so
/// absolute); anything else is left out, so no path can turn into a flag of the new process.
pub(crate) fn relaunch_args(safe_mode: bool, session: Option<&Path>) -> Vec<std::ffi::OsString> {
    let mut args = Vec::new();
    if safe_mode {
        args.push(SAFE_MODE_FLAG.into());
    }
    let session = session
        .and_then(|p| p.canonicalize().ok())
        .filter(|p| p.is_file() && p.is_absolute());
    if let Some(path) = session {
        args.push(crate::instance::END_OF_FLAGS.into());
        args.push(path.into_os_string());
    }
    args
}

/// The calling window's autosaved session, when `reopen` asks for it.
pub(crate) fn session_to_reopen(state: &CurrentWindow, lifecycle: &Lifecycle, reopen: bool) -> Option<PathBuf> {
    reopen.then(|| lifecycle.session_path(state.label())).flatten()
}

/// Relaunches IMT, optionally in safe mode, reopening the calling window's autosaved session when
/// `reopen_session` (save it first with `save_session`).
#[tauri::command]
pub async fn restart_app<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: CurrentWindow,
    lifecycle: State<'_, Lifecycle>,
    safe_mode: bool,
    reopen_session: bool,
) -> CmdResult<()> {
    let exe = std::env::current_exe().map_err(err)?;
    let args = relaunch_args(safe_mode, session_to_reopen(&state, &lifecycle, reopen_session).as_deref());
    let mut cmd = imt_core::process::quiet_command(exe);
    // The new process must become the running instance, not hand over to this one.
    crate::instance::release(&app);
    cmd.args(&args).spawn().map_err(err)?;
    log::info!(SafeMode = safe_mode, Reopen = args.len() > usize::from(safe_mode); "Restarting");
    crate::window_manager::quit(&app);
    Ok(())
}

/// Lifts a plugin's quarantine (after repeated crashes) immediately, in every window.
#[tauri::command]
pub async fn plugin_reset(windows: State<'_, Windows>, state: State<'_, AppState>, op: crate::activity::OpId, id: String) -> CmdResult<()> {
    // An isolated plugin's process may be quarantined too.
    state.isolation.isolation.reset(&id);
    for w in windows.all() {
        w.store_read().reset_plugin(&id);
    }
    log::info!(Plugin = id.as_str(); "Plugin re-enabled after quarantine");
    crate::activity::domain_event("plugin.reset", op.as_deref(), &[("Plugin", &id)]);
    Ok(())
}

/// Enables or disables a plugin from the next start (returns `true`: restart needed).
#[tauri::command]
pub async fn plugin_set_enabled(state: State<'_, AppState>, op: crate::activity::OpId, id: String, enabled: bool) -> CmdResult<bool> {
    set_plugin_enabled(&state, id, enabled, op.as_deref())
}

/// Enables or disables a plugin from the next start (what `plugin_set_enabled` does); `true`: restart needed.
pub fn set_plugin_enabled(state: &AppState, id: String, enabled: bool, op: Option<&str>) -> CmdResult<bool> {
    let dir = state.config_dir.as_ref().ok_or("no configuration folder")?;
    let mut s = load_plugin_settings(Some(dir));
    s.disabled.retain(|d| *d != id);
    if !enabled {
        s.disabled.push(id.clone());
    }
    save_plugin_settings(dir, &s).map_err(err)?;
    log::info!(Plugin = id.as_str(), Enabled = enabled; "Plugin setting changed");
    let event = if enabled { "plugin.enabled" } else { "plugin.disabled" };
    crate::activity::domain_event(event, op, &[("Plugin", &id)]);
    Ok(true)
}

/// Autosaves the calling window's session (open files, profile, UI state) for crash recovery.
#[tauri::command]
pub async fn save_session(state: CurrentWindow, lifecycle: State<'_, Lifecycle>, ui: Value, reports: Option<Vec<Value>>) -> CmdResult<()> {
    let Some(store) = &lifecycle.sessions else { return Ok(()) };
    let mut project = crate::workspace_commands::project_of(&state, "Last session".into(), ui);
    // Autosave never fails over a report: one that does not parse is left out.
    project.reports = reports
        .unwrap_or_default()
        .into_iter()
        .filter_map(|r| imt_core::report::Report::parse(r).ok())
        .collect();
    crate::error::lift(store.save(state.label(), &project).map(|_| ()).map_err(err))
}

/// Path of the calling window's autosaved session (open it with `open_project`, which it is granted to),
/// if any.
#[tauri::command]
pub async fn last_session(
    state: CurrentWindow,
    grants: crate::paths::WindowGrants,
    lifecycle: State<'_, Lifecycle>,
) -> CmdResult<Option<String>> {
    let Some(path) = lifecycle.session_path(state.label()) else {
        return Ok(None);
    };
    Ok(grants
        .grant(&path, crate::paths::Access::Read, crate::paths::GrantSource::Output)
        .ok()
        .map(|g| g.path().to_string_lossy().into_owned()))
}

/// Whether `path` is an autosaved session (IMT's own file: not a project of the recent list).
pub fn is_session_file(path: &std::path::Path) -> bool {
    path.file_name().is_some_and(|n| n == SESSION_FILE)
}

/// After a crash: reopens the previous run's other windows, each with its
/// session queued (it opens it on startup). Once per run; returns how many opened.
#[tauri::command]
pub async fn restore_windows<R: tauri::Runtime>(
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
    use crate::test_support::Now as _;

    #[test]
    fn teardown_steps_are_bounded_and_say_how_they_ended() {
        let quick = std::time::Duration::from_millis(200);
        assert_eq!(teardown_step("quick", quick, || {}), StepEnd::Done);
        let started = std::time::Instant::now();
        let stuck = teardown_step("stuck", quick, || std::thread::sleep(std::time::Duration::from_secs(5)));
        assert_eq!(stuck, StepEnd::TimedOut);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        // A generous limit: the panic hook (a backtrace, symbolised in a debug build) runs before the step
        // unwinds and can take seconds; the step still ends as soon as it has unwound.
        let started = std::time::Instant::now();
        assert_eq!(
            teardown_step("crashing", std::time::Duration::from_secs(60), || panic!("a crash in teardown")),
            StepEnd::Failed
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(60));
    }

    /// Closing the last window and exiting finish in bounded time with a live tail whose batch waits for
    /// the workspace (an import holds it), a job whose stop never returns, and the tray present.
    #[test]
    fn quitting_finishes_with_live_tail_jobs_and_the_tray() {
        use crate::test_support::{block, test_app_with_commands, window, TempDir};
        use tauri::Manager;
        let dir = TempDir::new("quit-bounded");
        let log = dir.0.join("a.log");
        std::fs::write(&log, "2026-10-14 10:00:00 INFO first\n").unwrap();
        let app = test_app_with_commands(None);
        let _main = window(&app, "main");
        crate::paths::allow_under(app.handle(), "main", &dir.0);
        let state = CurrentWindow::of(app.handle(), "main").unwrap();
        block(crate::workspace_commands::open_paths(
            app.handle().clone(),
            state.clone(),
            vec![log.to_string_lossy().into_owned()],
        ))
        .unwrap();
        block(crate::workspace_commands::set_live(app.handle().clone(), state.clone(), true, None)).unwrap();
        crate::tray::create(app.handle());
        let (entered, running) = std::sync::mpsc::channel();
        let spec = crate::jobs::JobSpec::new(crate::jobs::JobKind::Export, "Never stops").window("main");
        let controls = crate::jobs::Controls::cancel(|| std::thread::sleep(std::time::Duration::from_secs(30)));
        crate::jobs::spawn(
            app.handle(),
            spec,
            controls,
            move |_ctx: &crate::jobs::JobCtx| -> Result<crate::jobs::Done<()>, String> {
                let _ = entered.send(());
                std::thread::sleep(std::time::Duration::from_secs(30));
                Ok(crate::jobs::Done::new(()))
            },
        )
        .unwrap();
        running.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        let import = state.store_write();
        std::fs::write(&log, "2026-10-14 10:00:00 INFO first\n2026-10-14 10:00:01 INFO second\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let started = std::time::Instant::now();
        app.state::<Windows>().set_exiting();
        crate::window_manager::destroyed(app.handle(), "main");
        exit_teardown(app.handle());
        let took = started.elapsed();
        assert!(
            took < TEARDOWN_STEP_LIMIT * 2 + std::time::Duration::from_secs(1),
            "quitting took {took:?}"
        );
        drop(import);
    }

    #[test]
    fn relaunch_arguments_come_from_rust_only() {
        let tmp = crate::test_support::TempDir::new("relaunch");
        let session = tmp.0.join("last-session.imtproj");
        std::fs::write(&session, "{}").unwrap();
        let canonical = session.canonicalize().unwrap();
        let strings = |a: Vec<std::ffi::OsString>| a.into_iter().map(|s| s.to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert!(relaunch_args(false, None).is_empty());
        assert_eq!(strings(relaunch_args(true, None)), [SAFE_MODE_FLAG]);
        assert_eq!(
            strings(relaunch_args(true, Some(&session))),
            [SAFE_MODE_FLAG.to_string(), "--".into(), canonical.to_string_lossy().into_owned()]
        );
        // A missing, relative, flag-like or folder path is left out.
        for bad in [
            "--export-diagnostics",
            "--isolated",
            "--safe-mode",
            "relative.imtproj",
            "/does/not/exist.imtproj",
        ] {
            assert!(relaunch_args(false, Some(Path::new(bad))).is_empty(), "{bad}");
        }
        assert!(relaunch_args(false, Some(&tmp.0)).is_empty(), "a folder");
        // The new process reads no flag from the relaunch but safe mode.
        let args = strings(relaunch_args(true, Some(&session)));
        assert_eq!(crate::activity::report::cli::parse(&args), None);
        let launch = crate::instance::parse_args(args, None);
        assert!(launch.safe_mode && launch.paths == [canonical]);
    }

    #[test]
    fn restart_takes_no_arguments_from_the_webview() {
        let app = crate::test_support::test_app_with_commands(None);
        let main = crate::test_support::window(&app, "main");
        // The old `reopen` list is not part of the commands any more: they need `reopenSession`.
        for cmd in ["restart_app", "restart_elevated"] {
            let sent = serde_json::json!({ "safeMode": false, "reopen": ["--export-diagnostics", "/tmp/x.zip", "--force"] });
            assert!(crate::tools::tests::ipc(&main, cmd, sent).is_err(), "{cmd}");
        }
        // Without a saved session nothing is reopened: the command line holds no path at all.
        let state = CurrentWindow::of(&main, "main").unwrap();
        let lifecycle = Lifecycle::begin(None, false);
        assert_eq!(session_to_reopen(&state, &lifecycle, true), None);
        assert_eq!(session_to_reopen(&state, &lifecycle, false), None);
    }

    /// Installs the panic logging hook once for this test process (it chains the previous hook).
    fn panic_logging() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(install_panic_logging);
    }

    fn crashes(marker: &str) -> Vec<crate::activity::Captured> {
        crate::activity::captured()
            .into_iter()
            .filter(|c| c.message.contains(marker))
            .collect()
    }

    #[test]
    fn panics_on_any_thread_are_logged_as_fatal_with_thread_and_backtrace() {
        let _g = crate::activity::test_capture();
        panic_logging();
        // A named std thread (watcher, event-bus subscriber and helper threads are std threads).
        let t = std::thread::Builder::new()
            .name("imt-test-worker".into())
            .spawn(|| panic!("boom on a worker (hook test)"))
            .unwrap();
        assert!(t.join().is_err());
        let c = crashes("boom on a worker (hook test)");
        assert_eq!(c.len(), 1);
        let e = &c[0];
        assert_eq!(
            (e.level, e.props["Severity"].as_str(), e.props["Thread"].as_str()),
            (log::Level::Error, "Fatal", "imt-test-worker")
        );
        assert!(e.props["Location"].contains("lifecycle.rs"), "{:?}", e.props);
        assert!(!e.props["Backtrace"].is_empty() && e.props["Backtrace"].len() <= MAX_BACKTRACE + 40);
        // A task on Tauri's async runtime (tokio worker threads).
        let task = tauri::async_runtime::spawn(async { panic!("boom in an async task (hook test)") });
        assert!(crate::test_support::block(task).is_err());
        assert_eq!(crashes("boom in an async task (hook test)").len(), 1);
        // A blocking-pool task (where commands do their work).
        let blocking = tauri::async_runtime::spawn_blocking(|| panic!("boom in a blocking task (hook test)"));
        assert!(crate::test_support::block(blocking).is_err());
        assert_eq!(crashes("boom in a blocking task (hook test)").len(), 1);
        // A thread without a name is identified by its id.
        assert!(std::thread::spawn(|| panic!("boom unnamed (hook test)")).join().is_err());
        assert!(crashes("boom unnamed (hook test)")[0].props["Thread"].starts_with("ThreadId("));
    }

    #[test]
    fn contained_plugin_crashes_quarantines_and_overruns_are_logged() {
        let _g = crate::activity::test_capture();
        let health = imt_core::guard::Health::default();
        for i in 0..imt_core::guard::QUARANTINE_AFTER {
            health.fail("t.flaky-log", &format!("bad record {i}"));
        }
        let logged = crate::activity::captured();
        let crash = logged
            .iter()
            .find(|c| c.message.contains("t.flaky-log crashed (contained): bad record 0"))
            .expect("crash logged");
        assert_eq!(
            (crash.level, crash.target.as_str(), crash.props["Plugin"].as_str()),
            (log::Level::Error, imt_core::guard::LOG_TARGET, "t.flaky-log")
        );
        assert!(logged.iter().any(|c| c.message.contains("t.flaky-log quarantined")));
    }

    #[test]
    fn plugin_setting_changes_are_domain_events() {
        let _g = crate::activity::test_capture();
        let dir = crate::test_support::TempDir::new("plugin-events");
        let app = crate::test_support::test_app_with_commands(Some(dir.0.join("profiles")));
        let main = crate::test_support::window(&app, "main");
        let ipc = |cmd: &str, body: Value, op: &str| crate::tools::tests::ipc_op(&main, cmd, body, op);
        ipc(
            "plugin_set_enabled",
            serde_json::json!({"id": "net.decryptology.web", "enabled": false}),
            "op-plugin-disable",
        )
        .unwrap();
        ipc(
            "plugin_set_enabled",
            serde_json::json!({"id": "net.decryptology.web", "enabled": true}),
            "op-plugin-enable",
        )
        .unwrap();
        ipc("plugin_reset", serde_json::json!({"id": "net.decryptology.web"}), "op-plugin-reset").unwrap();
        let events: Vec<(String, String, String)> = crate::activity::captured()
            .into_iter()
            // The logger is process-wide: keep only this test's operations, not those of tests running alongside.
            .filter(|c| c.props.get("Kind").is_some_and(|k| k == "domain"))
            .filter(|c| c.props.get("OperationId").is_some_and(|op| op.starts_with("op-plugin-")))
            .map(|c| {
                (
                    c.props["Operation"].clone(),
                    c.props["OperationId"].clone(),
                    c.props["Plugin"].clone(),
                )
            })
            .collect();
        let want = |e: &str, op: &str| (e.to_string(), op.to_string(), "net.decryptology.web".to_string());
        assert_eq!(
            events,
            vec![
                want("plugin.disabled", "op-plugin-disable"),
                want("plugin.enabled", "op-plugin-enable"),
                want("plugin.reset", "op-plugin-reset")
            ]
        );
        let _ = std::fs::remove_dir_all(&dir.0);
    }

    #[test]
    fn backtraces_are_bounded_at_line_boundaries() {
        assert_eq!(bounded_lines("short", 100), "short");
        let text = "frame one\nframe two\nframe three\n";
        let cut = bounded_lines(text, 15);
        assert!(cut.starts_with("frame one\n…") || cut.starts_with("frame one\n"), "{cut}");
        assert!(cut.contains("more bytes"));
        assert!(bounded_lines("ééééé", 3).contains("more bytes"));
    }

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
        // No start time recorded: a live PID cannot be told from reuse; no false crash.
        assert_eq!(previous_run(marker(10, None), own, &probe), PreviousRun::StillRunning);
        assert_eq!(previous_run(marker(30, None), own, &probe), PreviousRun::Crashed);
        assert_eq!(previous_run(MarkerFile::Unreadable, own, &probe), PreviousRun::Crashed);
    }

    #[test]
    fn markers_parse_newer_and_corrupt_text() {
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
            "1234",
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
            ..Default::default()
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
    fn sessions_tolerate_corrupt_files() {
        let d = temp("corrupt");
        std::fs::create_dir_all(&d).unwrap();
        let store = SessionStore::new(d.clone());
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
        let grants = |label: &str| crate::paths::WindowGrants::of(&h, label);
        let life = || app.state::<Lifecycle>();
        let status = app_status(main_window(&app), life()).now().unwrap();
        assert!(status.previous_crash && status.session_available);
        // Another window does not offer crash recovery.
        let w2 = crate::windows::CurrentWindow::of(&h, "w-7").unwrap();
        let other = app_status(w2.clone(), life()).now().unwrap();
        assert!(!other.previous_crash && !other.session_available);
        assert!(last_session(w2.clone(), grants("w-7"), life()).now().unwrap().is_none());
        assert!(last_session(main_window(&app), grants(MAIN), life())
            .now()
            .unwrap()
            .unwrap()
            .ends_with(SESSION_FILE));

        // Restoring reopens the other window with its session queued, once.
        assert_eq!(
            tauri::async_runtime::block_on(restore_windows(h.clone(), main_window(&app), life())).unwrap(),
            1
        );
        assert_eq!(
            tauri::async_runtime::block_on(restore_windows(h.clone(), main_window(&app), life())).unwrap(),
            0
        );
        let restored = crate::windows::CurrentWindow::of(&h, "w-2").unwrap().take_pending();
        assert_eq!(restored.len(), 1);
        assert_eq!(name_of(Path::new(&restored[0])), "second");

        // Autosave of this run, per window.
        crate::paths::allow_under(&h, MAIN, &samples());
        let csv = samples().join("csv").to_string_lossy().into_owned();
        block(crate::workspace_commands::open_paths(h.clone(), main_window(&app), vec![csv])).unwrap();
        save_session(
            main_window(&app),
            life(),
            serde_json::json!({ "k": 1 }),
            Some(vec![serde_json::json!({"id": "r"}), Value::Null]),
        )
        .now()
        .unwrap();
        save_session(w2.clone(), life(), Value::Null, None).now().unwrap();
        let saved = ProjectFile::load(
            &last_session(main_window(&app), grants(MAIN), life())
                .now()
                .unwrap()
                .map(PathBuf::from)
                .unwrap(),
        )
        .unwrap();
        assert_eq!((saved.sources.len(), saved.ui["k"].as_i64()), (1, Some(1)));
        assert_eq!(saved.reports.len(), 1, "an unreadable report is left out of the autosave");
        assert!(last_session(w2, grants("w-7"), life()).now().unwrap().is_some());
        life().forget_session("w-7");
        // Without a data folder sessions are off.
        let app2 = test_app(None);
        app2.manage(Lifecycle::begin(None, false));
        save_session(main_window(&app2), app2.state::<Lifecycle>(), Value::Null, None)
            .now()
            .unwrap();
        assert_eq!(
            tauri::async_runtime::block_on(restore_windows(
                app2.handle().clone(),
                main_window(&app2),
                app2.state::<Lifecycle>()
            ))
            .unwrap(),
            0
        );
        app2.state::<Lifecycle>().forget_session(MAIN);
        std::fs::remove_dir_all(d).unwrap();
    }
}
