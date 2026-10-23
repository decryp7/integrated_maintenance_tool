//! The engine of every window (ADR 0014 6c): the DuckDB store behind the store ports, hosted by the store
//! worker for all windows ([`WorkerHost`]; in process in tests), plus what stays in the app: the window's plugin
//! registry (tools, configuration parsers, marker kinds, plugin status) and its compiled profile.
//!
//! A window's store lives in a folder: a scratch folder under Debrief's data folder (`stores/<window>-…`) until the
//! user saves a project, or the project's own folder. The store is opened on first use, so a window that never
//! opens anything never starts the worker, and a missing libduckdb is an error of that command with a hint,
//! not of the app.
//!
//! - [`StoreHost`]: how stores are created and opened (port; the worker or in process).
//! - [`Engine`]: one window's store slot (open on first use, swap on opening a project).
//! - [`summary`]: the `WorkspaceSummary` the UI shows, from the store's summary and the host's profile.
//! - [`import`]: imports as jobs with progress, Cancel and the import report.

pub(crate) mod annotate;
pub(crate) mod import;
pub(crate) mod store_failure;
pub(crate) mod summary;

use crate::error::{CmdError, CmdResult};
use crate::ErrorCode;
use debrief_core::package::ProjectManifest;
use debrief_core::ports::{Store, StoreError, StoreResult};
use debrief_core::profile::CompiledProfile;
use debrief_core::redact::Redactor;
use debrief_core::{PluginRegistry, Profile};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// A store, shared by the commands of one window.
pub(crate) type SharedStore = Arc<dyn Store>;

/// How stores are created and opened: the store worker in the app, the store in process in tests.
pub(crate) trait StoreHost: Send + Sync {
    /// A new, empty store in `dir` (whatever was there is replaced).
    fn create(&self, dir: &Path, profile: &Profile) -> StoreResult<SharedStore>;
    /// The store already in `dir` (a window restored after a restart).
    fn open(&self, dir: &Path, profile: &Profile) -> StoreResult<SharedStore>;
    /// A project: a `.debrief` folder in place, or an `.debriefz` extracted into a new folder under `dest_root`.
    fn open_project(&self, path: &Path, dest_root: &Path, profile: &Profile) -> StoreResult<(SharedStore, ProjectManifest)>;
    /// The installed third-party plugins that read files now (after a grant change or a removal), for hosts that
    /// start plugins themselves.
    fn set_third_party(&self, _plugins: Vec<debrief_store_client::protocol::ThirdPartyPlugin>) {}
}

/// The store worker of the app: one process for every window's store.
pub(crate) struct WorkerHost {
    worker: debrief_store_client::StoreWorker,
}

impl WorkerHost {
    /// The worker next to Debrief's executable (`DEBRIEF_STORE_WORKER` overrides it), with libduckdb from the installed
    /// resource folder `lib_dir` (installers), next to the worker, or from Debrief's own library path (development
    /// builds, `fetch-libduckdb.sh`). The worker starts the built-in plugins and the installed `third_party` ones
    /// for imports itself (ADR 0027), with `launch` (its cap and tuning from Settings ▸ Performance, ADR 0031) and
    /// `memory_listener` hearing its memory watcher.
    pub(crate) fn new(
        disabled_plugins: Vec<String>,
        safe_mode: bool,
        lib_dir: Option<PathBuf>,
        third_party: Vec<debrief_store_client::protocol::ThirdPartyPlugin>,
        launch: debrief_store_client::WorkerLaunch,
        memory_listener: Option<debrief_ipc::memory_watch::MemoryListener>,
    ) -> StoreResult<WorkerHost> {
        let exe = worker_binary();
        let mut cfg = debrief_store_client::WorkerConfig::new(&exe);
        cfg.memory_mode = launch.memory_mode;
        cfg.memory_cap = launch.memory_cap;
        cfg.tuning = launch.tuning;
        cfg.memory_listener = memory_listener;
        cfg.lib_dir = lib_dir.filter(|d| d.is_dir()).or_else(|| exe.parent().map(Path::to_path_buf));
        cfg.disabled_plugins = disabled_plugins;
        cfg.safe_mode = safe_mode;
        cfg.third_party = third_party;
        cfg.child_log = true;
        Ok(WorkerHost {
            worker: debrief_store_client::StoreWorker::new(cfg)?,
        })
    }

    /// The supervised worker (Settings ▸ Performance hands it new launch values).
    pub(crate) fn worker(&self) -> debrief_store_client::StoreWorker {
        self.worker.clone()
    }
}

/// Where the store worker's executable is: `DEBRIEF_STORE_WORKER`, else next to Debrief's own executable.
pub(crate) fn worker_binary() -> PathBuf {
    if let Some(p) = std::env::var_os("DEBRIEF_STORE_WORKER") {
        return PathBuf::from(p);
    }
    let name = if cfg!(windows) {
        "debrief-store-worker.exe"
    } else {
        "debrief-store-worker"
    };
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

impl WorkerHost {
    /// Opening a store is the user's own request (Open, Try again): a worker that was stopped after repeated failures
    /// gets another start, so "Try again" does what it says instead of repeating the quarantine message.
    fn lift_quarantine(&self) {
        if self.worker.status().state == debrief_ipc::supervisor::ChildState::Quarantined {
            log::info!("The store worker was stopped after repeated failures; starting it again on the user's request");
            self.worker.reset();
        }
    }
}

impl StoreHost for WorkerHost {
    fn create(&self, dir: &Path, profile: &Profile) -> StoreResult<SharedStore> {
        self.lift_quarantine();
        Ok(Arc::new(self.worker.open_store(dir, profile.clone(), true, None)?))
    }

    fn open(&self, dir: &Path, profile: &Profile) -> StoreResult<SharedStore> {
        self.lift_quarantine();
        Ok(Arc::new(self.worker.open_store(dir, profile.clone(), false, None)?))
    }

    fn open_project(&self, path: &Path, dest_root: &Path, profile: &Profile) -> StoreResult<(SharedStore, ProjectManifest)> {
        self.lift_quarantine();
        let (store, manifest) = self.worker.open_project(path, dest_root, profile.clone(), None)?;
        Ok((Arc::new(store), manifest))
    }

    fn set_third_party(&self, plugins: Vec<debrief_store_client::protocol::ThirdPartyPlugin>) {
        if let Err(e) = self.worker.set_third_party(plugins) {
            log::warn!("The store worker did not take the plugin change: {e}");
        }
    }
}

/// What the window's store folder is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StoreKind {
    /// Debrief's own scratch folder for this window (deleted when the window closes).
    Scratch,
    /// A project folder the user chose or opened (kept).
    Project,
}

struct Current {
    store: SharedStore,
    kind: StoreKind,
}

/// One window's store: opened (created) on first use in the window's scratch folder; replaced when a project
/// opens. Holding the slot's lock never spans a store call: commands clone the store and call it outside.
pub(crate) struct Engine {
    host: Arc<dyn StoreHost>,
    /// Where scratch stores of this window are made (`<data>/stores`).
    root: PathBuf,
    label: String,
    current: RwLock<Option<Current>>,
    /// The profile a new store starts with (the window's).
    profile: Mutex<Profile>,
    /// The parser library every store of the window reads with (ADR 0026; the app's, shared by all windows).
    parsers: Mutex<debrief_core::parser::ParserSetup>,
    /// Sources of the store read with a parser that has since changed (the last library change said so).
    stale_parsers: Mutex<usize>,
    /// The import report of the last import (load time, problems).
    last_report: Mutex<Option<debrief_core::ports::ImportReport>>,
    /// The running import's progress (`import_progress`), as its job last read it.
    progress: Mutex<debrief_core::workspace::ProgressSnapshot>,
}

/// Scratch folders made by this process (unique per window and store).
static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(1);

impl Engine {
    pub(crate) fn new(host: Arc<dyn StoreHost>, root: PathBuf, label: &str, profile: Profile) -> Engine {
        Engine {
            host,
            root,
            label: label.to_string(),
            current: RwLock::default(),
            profile: Mutex::new(profile),
            parsers: Mutex::new(debrief_core::parser::ParserSetup::builtins()),
            stale_parsers: Mutex::new(0),
            last_report: Mutex::default(),
            progress: Mutex::new(debrief_core::workspace::ProgressSnapshot::stopped()),
        }
    }

    /// The folder of this window's scratch stores.
    pub(crate) fn scratch_root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn host(&self) -> &Arc<dyn StoreHost> {
        &self.host
    }

    /// The window's store, opened in a new scratch folder on first use.
    pub(crate) fn store(&self) -> CmdResult<SharedStore> {
        if let Some(c) = crate::sync::read(&self.current).as_ref() {
            return Ok(c.store.clone());
        }
        let mut slot = crate::sync::write(&self.current);
        if let Some(c) = slot.as_ref() {
            return Ok(c.store.clone());
        }
        let dir = self.scratch_dir();
        let profile = crate::sync::lock(&self.profile).clone();
        let store = self.host.create(&dir, &profile).map_err(|e| unavailable(&e))?;
        self.hand_parsers(store.as_ref());
        *slot = Some(Current {
            store: store.clone(),
            kind: StoreKind::Scratch,
        });
        Ok(store)
    }

    /// Hands the window's parser library to `store`; remembers how many of its sources need reading again.
    fn hand_parsers(&self, store: &dyn Store) {
        let setup = crate::sync::lock(&self.parsers).clone();
        match store.set_parsers(&setup) {
            Ok(stale) => *crate::sync::lock(&self.stale_parsers) = stale.len(),
            Err(e) => log::warn!("the parser library could not be handed to the store: {e}"),
        }
    }

    /// The parser library changed: new stores read with `setup`, and the open store is told at once.
    pub(crate) fn set_parsers(&self, setup: debrief_core::parser::ParserSetup) {
        *crate::sync::lock(&self.parsers) = setup;
        if let Some(store) = self.opened() {
            self.hand_parsers(store.as_ref());
        }
    }

    /// Sources of the open store read with a parser that has since changed.
    pub(crate) fn stale_parsers(&self) -> usize {
        *crate::sync::lock(&self.stale_parsers)
    }

    /// The stale sources were read again.
    pub(crate) fn clear_stale_parsers(&self) {
        *crate::sync::lock(&self.stale_parsers) = 0;
    }

    /// The store if one is open (no worker start): for cheap questions such as "is anything loaded".
    pub(crate) fn opened(&self) -> Option<SharedStore> {
        crate::sync::read(&self.current).as_ref().map(|c| c.store.clone())
    }

    /// What the store's folder is (`None` before the first use).
    #[cfg(test)]
    pub(crate) fn kind(&self) -> Option<StoreKind> {
        crate::sync::read(&self.current).as_ref().map(|c| c.kind.clone())
    }

    fn scratch_dir(&self) -> PathBuf {
        let n = NEXT_SCRATCH.fetch_add(1, Ordering::SeqCst);
        self.root.join(format!("{}-{}-{n}", self.label, std::process::id()))
    }

    /// Where an opened `.debriefz` project is extracted (a new folder under the stores folder).
    pub(crate) fn unpack_root(&self) -> PathBuf {
        self.scratch_dir()
    }

    /// Makes `store` the window's store; the one before is closed (and its scratch folder deleted).
    pub(crate) fn replace(&self, store: SharedStore, kind: StoreKind) {
        self.hand_parsers(store.as_ref());
        let previous = crate::sync::write(&self.current).replace(Current { store, kind });
        if let Some(p) = previous {
            retire(p);
        }
    }

    /// Opens the store already in `dir` (a restored window) as the window's store.
    pub(crate) fn reopen(&self, dir: &Path, kind: StoreKind) -> CmdResult<SharedStore> {
        let profile = crate::sync::lock(&self.profile).clone();
        let store = self.host.open(dir, &profile).map_err(|e| unavailable(&e))?;
        self.replace(store.clone(), kind);
        Ok(store)
    }

    /// The profile new stores start with (after the window's profile changed).
    pub(crate) fn set_profile(&self, profile: Profile) {
        *crate::sync::lock(&self.profile) = profile;
    }

    pub(crate) fn profile(&self) -> Profile {
        crate::sync::lock(&self.profile).clone()
    }

    pub(crate) fn note_report(&self, report: debrief_core::ports::ImportReport) {
        *crate::sync::lock(&self.last_report) = Some(report);
    }

    pub(crate) fn note_progress(&self, p: debrief_core::workspace::ProgressSnapshot) {
        *crate::sync::lock(&self.progress) = p;
    }

    pub(crate) fn progress(&self) -> debrief_core::workspace::ProgressSnapshot {
        crate::sync::lock(&self.progress).clone()
    }

    pub(crate) fn last_report(&self) -> Option<debrief_core::ports::ImportReport> {
        crate::sync::lock(&self.last_report).clone()
    }

    /// Closes the store (the window closed). A scratch folder is deleted unless Debrief is quitting (then the
    /// session restores it).
    pub(crate) fn shutdown(&self, keep: bool) {
        if let Some(c) = crate::sync::write(&self.current).take() {
            if keep {
                c.store.close();
            } else {
                retire(c);
            }
        }
    }
}

/// Closes a store; a scratch folder goes with it.
fn retire(c: Current) {
    c.store.close();
    if c.kind == StoreKind::Scratch {
        let dir = c.store.store_dir();
        drop(c);
        // The store's files may stay open a moment longer (the worker closes them); a folder left behind is
        // swept at the next start.
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            log::debug!("scratch store not removed yet ({e}); swept at the next start");
        }
    }
}

/// Removes scratch store folders that no window uses (left by a crash or a busy folder), except `keep` (the
/// stores autosaved sessions reopen) and those of Debrief processes still running (another instance).
pub(crate) fn sweep_scratch(root: &Path, keep: &[PathBuf]) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    let keep: Vec<PathBuf> = keep
        .iter()
        .map(|k| std::fs::canonicalize(k).unwrap_or_else(|_| k.clone()))
        .collect();
    let mut sys = sysinfo::System::new();
    for e in entries.flatten() {
        let path = e.path();
        let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !path.is_dir() || keep.iter().any(|k| k == &canonical) {
            continue;
        }
        if let Some(pid) = scratch_pid(&path) {
            let p = sysinfo::Pid::from_u32(pid);
            sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
            if pid == std::process::id() || sys.process(p).is_some() {
                continue;
            }
        }
        if let Err(e) = std::fs::remove_dir_all(&path) {
            log::debug!("a scratch store is still in use ({e})");
        }
    }
}

/// The process id in a scratch folder's name (`<window>-<pid>-<n>`).
pub(crate) fn scratch_pid(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let mut parts = name.rsplitn(3, '-');
    let _n = parts.next()?;
    parts.next()?.parse().ok()
}

/// A store that cannot be started: the worker is missing or failed to start (e.g. libduckdb missing).
fn unavailable(e: &StoreError) -> CmdError {
    match e {
        StoreError::Engine(raw) => worker_failure(raw),
        other => store_error(other.clone()),
    }
}

/// The error of a store worker that cannot start or stopped: a plain message and next step by cause, the raw chain and
/// the worker's recent standard error as the technical detail, and retryable (opening a store lifts a quarantine).
fn worker_failure(raw: &str) -> CmdError {
    let stderr = debrief_ipc::child_log::find("store-worker")
        .map(|l| l.tail_text())
        .unwrap_or_default();
    let cause = store_failure::StoreFailure::classify(raw, &stderr);
    let detail = if stderr.trim().is_empty() {
        raw.to_string()
    } else {
        format!("{raw}\n\n{}", stderr.trim_end())
    };
    let mut err = CmdError::new(ErrorCode::Unavailable, cause.message())
        .hint(cause.hint())
        .detail(Some(detail));
    err.retryable = true;
    err
}

/// Whether an engine error is the store worker's own failure (it did not start, stopped or was quarantined).
fn is_worker_failure(m: &str) -> bool {
    m.starts_with(debrief_store_client::client::CRASHED) || m.contains("store worker")
}

/// A store error as a command error (typed: the UI shows the message and offers Retry when retryable).
pub(crate) fn store_error(e: StoreError) -> CmdError {
    match e {
        StoreError::Unsupported { operation, phase } => CmdError::new(
            ErrorCode::Unsupported,
            format!("{operation} is not available yet with the new log store"),
        )
        .detail(Some(format!("planned for phase {phase}"))),
        StoreError::Invalid(m) => CmdError::invalid(m),
        StoreError::Query(q) => {
            let mut e = CmdError::invalid(q.message.clone()).field("ql");
            e.detail = serde_json::to_string(&q).ok();
            e
        }
        StoreError::NotFound(m) => CmdError::not_found(m),
        StoreError::Busy(m) => CmdError::busy(m),
        StoreError::Cancelled => CmdError::new(ErrorCode::Cancelled, "cancelled"),
        StoreError::Io(m) => CmdError::new(ErrorCode::Io, m),
        // The store worker failing is one condition however a call met it: one message, details folded away.
        StoreError::Engine(m) if is_worker_failure(&m) => worker_failure(&m),
        StoreError::Engine(m) => CmdError::new(ErrorCode::Failed, m),
        StoreError::Damaged(m) => {
            // Retrying reads the same damaged file.
            let mut e = CmdError::new(ErrorCode::Failed, m)
                .hint("Open the original log files again, or open the project's .debriefz export: its copy is intact.");
            e.retryable = false;
            e
        }
        StoreError::Locked(m) => {
            CmdError::new(ErrorCode::Busy, m).hint("Close the other Debrief window or program that has this project open, then try again.")
        }
    }
}

impl From<StoreError> for CmdError {
    fn from(e: StoreError) -> CmdError {
        store_error(e)
    }
}

/// The window's host side: its plugin registry (tools, configuration parsers, marker kinds, plugin status;
/// importers run in the store worker), its profile compiled with it, and the redacted view while it is on.
pub(crate) struct Host {
    pub(crate) registry: Arc<PluginRegistry>,
    compiled: RwLock<Arc<CompiledProfile>>,
    /// The redacted view's redactor (the profile's rules) while the user has it on; the store's replies that show
    /// event text pass through it ([`debrief_core::redact::view`]).
    view: RwLock<Option<Arc<Redactor>>>,
}

impl Host {
    pub(crate) fn new(registry: PluginRegistry, profile: Profile) -> anyhow::Result<Host> {
        let compiled = registry.compile_profile(profile)?;
        Ok(Host {
            registry: Arc::new(registry),
            compiled: RwLock::new(Arc::new(compiled)),
            view: RwLock::default(),
        })
    }

    /// Switches the redacted view on (with the profile's rules) or off. Fails, saying what to add, when the profile
    /// has no rules.
    pub(crate) fn set_redacted_view(&self, on: bool) -> anyhow::Result<()> {
        let view = if on {
            Some(Arc::new(debrief_core::redact::view_redactor(&self.compiled().profile.redact)?))
        } else {
            None
        };
        *crate::sync::write(&self.view) = view;
        Ok(())
    }

    /// The redacted view's redactor, when it is on.
    pub(crate) fn redacted_view(&self) -> Option<Arc<Redactor>> {
        crate::sync::read(&self.view).clone()
    }

    pub(crate) fn compiled(&self) -> Arc<CompiledProfile> {
        crate::sync::read(&self.compiled).clone()
    }

    /// Compiles `profile` with the window's plugins (fails like the store would: unknown fields, bad regexes).
    pub(crate) fn compile(&self, profile: Profile) -> anyhow::Result<CompiledProfile> {
        self.registry.compile_profile(profile)
    }

    /// The window's profile changed: a redacted view that is on takes the new rules (and goes off when the new
    /// profile has none).
    pub(crate) fn set_compiled(&self, compiled: CompiledProfile) {
        let mut view = crate::sync::write(&self.view);
        if view.is_some() {
            *view = match debrief_core::redact::view_redactor(&compiled.profile.redact) {
                Ok(r) => Some(Arc::new(r)),
                Err(e) => {
                    log::info!("The redacted view is off: {e}");
                    None
                }
            };
        }
        *crate::sync::write(&self.compiled) = Arc::new(compiled);
    }
}

/// Runs `f` with the window's store on the blocking pool (a store call may wait for the worker).
pub(crate) async fn with_store<T: Send + 'static>(
    win: &crate::windows::WindowState,
    f: impl FnOnce(&dyn Store) -> StoreResult<T> + Send + 'static,
) -> CmdResult<T> {
    let store = win.engine.store()?;
    crate::sync::blocking("the log store", move || f(store.as_ref()).map_err(CmdError::from)).await
}

#[cfg(test)]
pub(crate) mod local;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_errors_become_typed_command_errors() {
        let e: CmdError = StoreError::Busy("an import runs".into()).into();
        assert_eq!(e.code, ErrorCode::Busy);
        assert!(e.retryable);
        let e: CmdError = StoreError::Cancelled.into();
        assert_eq!(e.code, ErrorCode::Cancelled);
        let e: CmdError = StoreError::unsupported("live tail", "6c").into();
        assert_eq!(e.code, ErrorCode::Unsupported);
        let e: CmdError = StoreError::Damaged("a data file is damaged".into()).into();
        assert_eq!((e.code, e.retryable), (ErrorCode::Failed, false));
        assert!(e.hint.as_deref().is_some_and(|h| h.contains(".debriefz")));
        let e: CmdError = StoreError::Locked("open elsewhere".into()).into();
        assert_eq!((e.code, e.retryable), (ErrorCode::Busy, true));
        assert!(e.hint.as_deref().is_some_and(|h| h.contains("Close")));
        let e: CmdError = StoreError::Query(debrief_core::ql::QlError {
            message: "unknown field".into(),
            offset: 3,
            len: 2,
            suggestions: vec![],
        })
        .into();
        assert_eq!((e.code, e.field.as_deref()), (ErrorCode::Invalid, Some("ql")));
        let e = unavailable(&StoreError::Engine("cannot load duckdb.dll".into()));
        assert_eq!(e.code, ErrorCode::Unavailable);
        assert!(e.message.starts_with("Debrief's log store"), "{}", e.message);
        assert!(e.retryable && e.hint.is_some());
        assert!(e.detail.unwrap().contains("cannot load duckdb.dll"));
        // Every route to the same failure gives the same single, plain error (no nested "try again; try again").
        let chain = "the store worker stopped (the store worker failed 3 times in 5 minutes (last: signal: 6 (SIGABRT)) and was stopped; reset it to try again); try again";
        let via_command = store_error(StoreError::Engine(chain.into()));
        assert_eq!((via_command.code, via_command.retryable), (ErrorCode::Unavailable, true));
        assert!(!via_command.message.contains("SIGABRT") && !via_command.hint.clone().unwrap().contains("try again;"));
        assert!(via_command.detail.unwrap().contains("SIGABRT"));
        assert_eq!(store_error(StoreError::Engine("disk exploded".into())).code, ErrorCode::Failed);
        assert_eq!(unavailable(&StoreError::Io("disk".into())).code, ErrorCode::Io);
    }

    #[test]
    fn a_missing_worker_is_unavailable_with_a_hint_and_nothing_else_breaks() {
        let t = crate::test_support::TempDir::new("missing-worker");
        let mut cfg = debrief_store_client::WorkerConfig::new(t.0.join("no-such-worker.exe"));
        cfg.supervisor.max_crashes = 1;
        let host = WorkerHost {
            worker: debrief_store_client::StoreWorker::new(cfg).unwrap(),
        };
        let engine = Engine::new(Arc::new(host), t.0.join("stores"), "main", Profile::default_profile());
        let e = engine.store().err().unwrap();
        assert!(matches!(e.code, ErrorCode::Unavailable | ErrorCode::Failed), "{e:?}");
        assert!(engine.opened().is_none());
        assert!(engine.kind().is_none());
    }

    #[test]
    fn scratch_stores_are_made_once_swapped_and_removed() {
        let t = crate::test_support::TempDir::new("engine-slot");
        let root = t.0.join("stores");
        let engine = Engine::new(local::host(), root.clone(), "main", Profile::default_profile());
        assert!(engine.opened().is_none());
        let a = engine.store().unwrap();
        assert!(Arc::ptr_eq(&a, &engine.store().unwrap()), "one store per window");
        assert_eq!(engine.kind(), Some(StoreKind::Scratch));
        let first = a.store_dir();
        assert!(first.starts_with(&root));
        drop(a);
        // Another store replaces it; the scratch folder goes.
        let other = local::host().create(&root.join("other"), &Profile::default_profile()).unwrap();
        engine.replace(other, StoreKind::Project);
        assert_eq!(engine.kind(), Some(StoreKind::Project));
        assert!(!first.exists(), "the scratch folder was removed");
        // A project folder stays when the window closes.
        let kept = engine.opened().unwrap().store_dir();
        engine.shutdown(false);
        assert!(kept.exists());
        // Sweeping removes what no window uses, but not a running Debrief's or a session's.
        std::fs::create_dir_all(root.join("stale")).unwrap();
        let dead = root.join("w-2-4294967294-1");
        let mine = root.join(format!("main-{}-9", std::process::id()));
        std::fs::create_dir_all(&dead).unwrap();
        std::fs::create_dir_all(&mine).unwrap();
        sweep_scratch(&root, std::slice::from_ref(&kept));
        assert!(!root.join("stale").exists() && !dead.exists());
        assert!(kept.exists() && mine.exists());
        assert_eq!(scratch_pid(Path::new("main-123-4")), Some(123));
        assert_eq!(scratch_pid(Path::new("stale")), None);
    }
}
