//! The app side: [`StoreWorker`] starts and supervises the worker process (`imt_ipc` frames, ADR 0017: lazy
//! start, restart with backoff, quarantine after repeated crashes, killed with IMT, a memory cap, its own child
//! log), and [`RemoteStore`] implements the store ports for one store folder over it. It holds no events: every
//! reply is a page, a summary or an aggregate, and a scan streams batches with credits.
//!
//! A crashed worker is restarted by the next call, which reopens the store (the database is durable); the
//! call that saw the crash fails with a retryable [`StoreError::Engine`], and an import that was running is
//! reported as failed. Analyses and the query language are cancellable: a cancelled [`QueryControl`] cancels
//! the call, and the worker interrupts the running statement.

use crate::protocol::{
    self, ExportProject, Handle, IdsAt, Limits, OpenProject, OpenStore, OpenedProject, Paths, Session, Source, StoreOp, StoreRequest,
    WorkerStatus,
};
use imt_core::analysis::{Anomaly, Comparison, FieldStats, RequestSpan, WaterfallRow};
use imt_core::api::{
    ChartData, ChartRequest, CompareRequest, CompletionRequest, ContextRequest, CorrelateRequest, ExportRequest, ExportResult,
    ExtractPreview, ExtractPreviewRequest, Facet, FacetRequest, FieldStatsRequest, Finding, MomentRequest, MomentSummary, ParsePreview,
    ParsePreviewRequest, QueryRequest, RequestsRequest, SourceContext, Timeline, TimelineRequest, TraceRequest, TraceSummary,
    WorkspaceState,
};
use imt_core::batch::{EventBatch, EventId, SourceId};
use imt_core::exceptions::ExceptionGroup;
use imt_core::package::ProjectManifest;
use imt_core::patterns::Pattern;
use imt_core::ports::{
    Analysis, Analyzed, DiagnosticRow, Diagnostics, EventView, EventsRequest, Filter, FixEvaluation, ImportSession, IndexStatus, Ingest,
    PageQuery, PageReply, Positions, Profiles, Projects, Query, QuickFix, Search, SearchReply, SearchRequest, StoreError, StoreResult,
    SummaryReply,
};
use imt_core::ql::{Completion, QlError, QlResult, QueryControl};
use imt_core::spans::infer::{SpanEvaluation, SpanProposal, SpanRuleCheck, SpanRuleRequest};
use imt_core::spans::{RunsReply, RunsRequest};
use imt_core::workspace::ProgressSnapshot;
use imt_core::Profile;
use imt_ipc::child_log::{ChildKind, ChildLogSpec};
use imt_ipc::launch::{launch, Child, ChildSpec};
use imt_ipc::peer::{NoCalls, PeerOptions};
use imt_ipc::process::SpawnSpec;
use imt_ipc::sandbox::SandboxSpec;
use imt_ipc::supervisor::{ChildStatus, Supervisor, SupervisorConfig};
use imt_ipc::{IpcError, Json, StreamEvent};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Deadline of an ordinary call (a page, a timeline, an aggregate).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Deadline of a long call that can be cancelled (analyses, the query language, exports, previews).
pub const LONG_TIMEOUT: Duration = Duration::from_secs(3600);
/// Deadline of waiting for an import.
pub const WAIT_TIMEOUT: Duration = Duration::from_secs(24 * 3600);
/// How often a cancellable call checks its control.
const POLL: Duration = Duration::from_millis(50);

/// Memory cap of the worker process by default: the import budget (600 MB) with headroom, so only a
/// runaway is stopped (the job object or address-space limit stops the process; the supervisor restarts it).
pub const DEFAULT_MEMORY_CAP: u64 = 1 << 30;

/// How to start the worker.
#[derive(Clone)]
pub struct WorkerConfig {
    pub exe: PathBuf,
    /// Hard memory cap of the process (job object on Windows, address-space limit on Unix), bytes; `None` for none.
    pub memory_cap: Option<u64>,
    /// Restart policy (crash window, backoff, quarantine).
    pub supervisor: SupervisorConfig,
    /// The folder holding libduckdb when it is not next to the worker (put first on the worker's library path).
    pub lib_dir: Option<PathBuf>,
    /// Plugins the user turned off: their importers are not installed in the worker.
    pub disabled_plugins: Vec<String>,
    /// Write the worker's standard error to its child log (the app; tests leave it off).
    pub child_log: bool,
    /// Safe mode: the worker starts no plugin.
    pub safe_mode: bool,
    /// Where libduckdb is looked for before the worker starts (`None`: `lib_dir`, the worker's folder and the
    /// library path), so a missing library is a clear message instead of a failing process.
    pub library_dirs: Option<Vec<PathBuf>>,
}

impl WorkerConfig {
    pub fn new(exe: impl Into<PathBuf>) -> WorkerConfig {
        WorkerConfig {
            exe: exe.into(),
            memory_cap: Some(DEFAULT_MEMORY_CAP),
            supervisor: SupervisorConfig::new("the store worker"),
            lib_dir: None,
            disabled_plugins: Vec::new(),
            child_log: false,
            safe_mode: false,
            library_dirs: None,
        }
    }
}

/// The environment variable a platform searches for shared libraries, and its separator.
fn library_path_var() -> (&'static str, char) {
    if cfg!(windows) {
        ("PATH", ';')
    } else if cfg!(target_os = "macos") {
        ("DYLD_LIBRARY_PATH", ':')
    } else {
        ("LD_LIBRARY_PATH", ':')
    }
}

/// The worker's library path: `lib_dir` first, then what IMT itself searches (so a development build finds the
/// libduckdb of `fetch-libduckdb.sh` that is on IMT's own path).
pub fn library_path_env(lib_dir: Option<&Path>) -> Option<(String, String)> {
    let (var, sep) = library_path_var();
    let inherited = std::env::var(var).ok().filter(|v| !v.is_empty());
    let value = match (lib_dir, inherited) {
        (Some(d), Some(rest)) => format!("{}{sep}{rest}", d.display()),
        (Some(d), None) => d.display().to_string(),
        (None, Some(rest)) => rest,
        (None, None) => return None,
    };
    Some((var.to_string(), value))
}

/// The file name of libduckdb on this platform.
pub fn libduckdb_file() -> &'static str {
    if cfg!(windows) {
        "duckdb.dll"
    } else if cfg!(target_os = "macos") {
        "libduckdb.dylib"
    } else {
        "libduckdb.so"
    }
}

/// The first of `dirs` that holds `file`.
pub fn find_library(file: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join(file)).find(|p| p.is_file())
}

/// Where the worker's loader looks for libduckdb: the bundled folder, the worker's folder, the library path.
fn library_search(cfg: &WorkerConfig) -> Vec<PathBuf> {
    if let Some(dirs) = &cfg.library_dirs {
        return dirs.clone();
    }
    let mut dirs: Vec<PathBuf> = cfg.lib_dir.iter().cloned().collect();
    dirs.extend(cfg.exe.parent().map(Path::to_path_buf));
    let (var, _) = library_path_var();
    if let Some(path) = std::env::var_os(var) {
        dirs.extend(std::env::split_paths(&path));
    }
    if cfg!(target_os = "linux") {
        dirs.extend(
            [
                "/usr/lib",
                "/usr/local/lib",
                "/usr/lib/x86_64-linux-gnu",
                "/usr/lib/aarch64-linux-gnu",
            ]
            .map(PathBuf::from),
        );
    }
    dirs
}

/// One live worker process and the stores opened in it.
pub struct Conn {
    child: Child,
    /// Store folder -> handle in this process.
    handles: Mutex<HashMap<PathBuf, u64>>,
}

impl imt_ipc::supervisor::Connection for Conn {
    fn pid(&self) -> Option<u32> {
        Some(self.child.pid())
    }
    fn kill(&self) {
        self.child.stop();
    }
}

struct Inner {
    sup: Supervisor<Conn>,
    disabled: Vec<String>,
    safe_mode: bool,
    /// Why the worker cannot start (its database library is missing).
    unavailable: Option<String>,
}

/// The supervised worker process. Cheap to clone.
#[derive(Clone)]
pub struct StoreWorker {
    inner: Arc<Inner>,
}

impl StoreWorker {
    /// Prepares the worker; it starts on the first call.
    pub fn new(cfg: WorkerConfig) -> StoreResult<StoreWorker> {
        let mut spawn = SpawnSpec::new(cfg.exe.clone(), "the store worker");
        spawn.endpoint_prefix = "imt-store".into();
        // Imports and queries run at normal priority: they are what the user waits for.
        spawn.below_normal_priority = false;
        if let Some(kv) = library_path_env(cfg.lib_dir.as_deref()) {
            spawn.env.push(kv);
        }
        let unavailable = find_library(libduckdb_file(), &library_search(&cfg)).is_none().then(|| {
            format!(
                "the store worker cannot start: its database library ({}) was not found next to it",
                libduckdb_file()
            )
        });
        let mut sup_cfg = cfg.supervisor.clone();
        if cfg.child_log {
            let log = ChildLogSpec::new("store-worker", ChildKind::StoreWorker).with_version(env!("CARGO_PKG_VERSION"));
            spawn.log = Some(log.clone());
            sup_cfg.log = Some(log);
        }
        // It opens the user's store folders and the files they import by path: a memory cap and a cleared
        // environment, no file-system restriction (ADR 0017 §6: workers have their own sandbox).
        let spec = {
            let mut c = ChildSpec::new(spawn).sandbox(SandboxSpec {
                memory_limit: cfg.memory_cap,
                ..SandboxSpec::unrestricted()
            });
            c.peer = PeerOptions {
                max_frame: protocol::MAX_MESSAGE_BYTES,
                ..c.peer
            };
            c
        };
        let sup = Supervisor::new(sup_cfg, move || {
            Ok(Conn {
                child: launch(&spec, Arc::new(NoCalls))?,
                handles: Mutex::new(HashMap::new()),
            })
        });
        Ok(StoreWorker {
            inner: Arc::new(Inner {
                sup,
                disabled: cfg.disabled_plugins,
                safe_mode: cfg.safe_mode,
                unavailable,
            }),
        })
    }

    /// Supervisor state (running, restarts, quarantine).
    pub fn status(&self) -> ChildStatus {
        self.inner.sup.status()
    }

    /// The worker's own report (starts it if needed).
    pub fn worker_status(&self) -> StoreResult<WorkerStatus> {
        self.inner
            .sup
            .call(|c| raw(c, &StoreRequest::Status, CALL_TIMEOUT))
            .map_err(ipc_error)
            .and_then(decode)
    }

    /// Stops the worker (it starts again on the next call).
    pub fn shutdown(&self) {
        self.inner.sup.shutdown();
    }

    /// Lifts a quarantine after repeated crashes.
    pub fn reset(&self) {
        self.inner.sup.reset();
    }

    /// Opens the store in `dir` (or creates it, replacing what is there).
    pub fn open_store(&self, dir: &Path, profile: Profile, create: bool, limits: Option<Limits>) -> StoreResult<RemoteStore> {
        self.check_available()?;
        let store = RemoteStore {
            worker: self.clone(),
            dir: dir.to_path_buf(),
            profile: Arc::new(Mutex::new(profile)),
            limits,
        };
        let lease = self.inner.sup.lease().map_err(ipc_error)?;
        store.ensure_open(&lease.conn, create).inspect_err(|e| self.report(&lease, e))?;
        Ok(store)
    }

    /// Opens a project: a `<name>.imt` folder in place, or an `.imtz` extracted into a new folder under
    /// `dest_root`. Returns the store (in the project's folder) and the project's manifest.
    pub fn open_project(
        &self,
        path: &Path,
        dest_root: &Path,
        profile: Profile,
        limits: Option<Limits>,
    ) -> StoreResult<(RemoteStore, ProjectManifest)> {
        self.check_available()?;
        let req = OpenProject {
            path: path.to_path_buf(),
            dest_root: dest_root.to_path_buf(),
            profile,
            disabled_plugins: self.inner.disabled.clone(),
            safe_mode: self.inner.safe_mode,
            limits: limits.clone(),
        };
        let lease = self.inner.sup.lease().map_err(ipc_error)?;
        let opened: StoreResult<OpenedProject> = encode(&req)
            .and_then(|b| raw(&lease.conn, &StoreRequest::OpenProject(b), LONG_TIMEOUT).map_err(ipc_error))
            .and_then(decode);
        let opened = opened.inspect_err(|e| self.report(&lease, e))?;
        lock(&lease.conn.handles).insert(opened.dir.clone(), opened.store);
        let store = RemoteStore {
            worker: self.clone(),
            dir: opened.dir,
            profile: Arc::new(Mutex::new(req.profile)),
            limits,
        };
        // Reopening after a restart uses the profile with the project's overrides merged in.
        if let Ok(active) = store.active_profile() {
            store.remember_profile(&active);
        }
        Ok((store, opened.manifest))
    }

    /// A missing database library, as the error of every call that would start the worker.
    fn check_available(&self) -> StoreResult<()> {
        match &self.inner.unavailable {
            Some(m) => Err(StoreError::Engine(m.clone())),
            None => Ok(()),
        }
    }

    fn report(&self, lease: &imt_ipc::supervisor::Lease<Conn>, e: &StoreError) {
        if let StoreError::Engine(m) = e {
            if m.starts_with(CRASHED) {
                self.inner.sup.report(lease, &IpcError::Died(m.clone()));
            }
        }
    }
}

/// Prefix of the error of a call whose worker failed.
pub const CRASHED: &str = "the store worker stopped";

fn ipc_error(e: IpcError) -> StoreError {
    match e {
        IpcError::Remote(r) if r.code == imt_ipc::peer::code::CANCELLED => StoreError::Cancelled,
        IpcError::Remote(r) => protocol::from_remote(r),
        other => StoreError::Engine(format!("{CRASHED} ({other}); try again")),
    }
}

fn decode<T: DeserializeOwned>(j: Json) -> StoreResult<T> {
    serde_json::from_value(j.0).map_err(|e| StoreError::Engine(format!("the store worker sent an invalid reply: {e}")))
}

fn encode(body: &impl Serialize) -> StoreResult<Json> {
    serde_json::to_value(body)
        .map(Json)
        .map_err(|e| StoreError::Invalid(format!("cannot encode the request: {e}")))
}

fn raw(c: &Conn, req: &StoreRequest, timeout: Duration) -> Result<Json, IpcError> {
    c.child.peer().request(req, Some(timeout)).map_err(|e| died(c, e))
}

fn died(c: &Conn, e: IpcError) -> IpcError {
    match e {
        IpcError::Died(_) if c.child.exited() => IpcError::Died(c.child.exit_text()),
        other => other,
    }
}

/// One store folder in the worker; implements the store ports. Cheap to clone.
#[derive(Clone)]
pub struct RemoteStore {
    worker: StoreWorker,
    dir: PathBuf,
    profile: Arc<Mutex<Profile>>,
    limits: Option<Limits>,
}

impl RemoteStore {
    /// The store's handle in `conn`, opening it there first (after a restart: reopening the database).
    fn ensure_open(&self, conn: &Conn, create: bool) -> StoreResult<u64> {
        if !create {
            if let Some(h) = lock(&conn.handles).get(&self.dir) {
                return Ok(*h);
            }
        }
        let req = OpenStore {
            dir: self.dir.clone(),
            profile: lock(&self.profile).clone(),
            create,
            disabled_plugins: self.worker.inner.disabled.clone(),
            safe_mode: self.worker.inner.safe_mode,
            limits: self.limits.clone(),
        };
        let h: Handle = decode(raw(conn, &StoreRequest::OpenStore(encode(&req)?), CALL_TIMEOUT).map_err(ipc_error)?)?;
        lock(&conn.handles).insert(self.dir.clone(), h.store);
        Ok(h.store)
    }

    fn call_with<T: DeserializeOwned>(&self, op: StoreOp, body: &impl Serialize, timeout: Duration) -> StoreResult<T> {
        let body = encode(body)?;
        let lease = self.worker.inner.sup.lease().map_err(ipc_error)?;
        let result = self
            .ensure_open(&lease.conn, false)
            .and_then(|store| raw(&lease.conn, &StoreRequest::Call { store, op, body }, timeout).map_err(ipc_error))
            .and_then(decode);
        if let Err(e) = &result {
            self.worker.report(&lease, e);
        }
        result
    }

    fn call<T: DeserializeOwned>(&self, op: StoreOp, body: &impl Serialize) -> StoreResult<T> {
        self.call_with(op, body, CALL_TIMEOUT)
    }

    /// A long call that `control` can cancel: it runs as a stream (no items), so a cancel reaches the worker,
    /// which interrupts the statement; the call then fails with [`StoreError::Cancelled`].
    fn call_controlled<T: DeserializeOwned>(&self, op: StoreOp, body: &impl Serialize, control: &QueryControl) -> StoreResult<T> {
        control.check().map_err(|_| StoreError::Cancelled)?;
        let body = encode(body)?;
        let lease = self.worker.inner.sup.lease().map_err(ipc_error)?;
        let result = (|| {
            let store = self.ensure_open(&lease.conn, false)?;
            let req = StoreRequest::Call { store, op, body };
            let mut stream = lease
                .conn
                .child
                .peer()
                .stream_request(&req, 1)
                .map_err(|e| ipc_error(died(&lease.conn, e)))?;
            let started = Instant::now();
            let mut cancelled: Option<Instant> = None;
            loop {
                match cancelled {
                    None if control.is_cancelled() => {
                        cancelled = Some(Instant::now());
                        stream.cancel();
                    }
                    // The worker interrupts the statement; one that does not stop in time is left to finish.
                    Some(at) if at.elapsed() > imt_ipc::peer::CANCEL_GRACE => return Err(StoreError::Cancelled),
                    _ => {}
                }
                if started.elapsed() > LONG_TIMEOUT {
                    stream.cancel();
                    return Err(StoreError::Engine(format!(
                        "{} took longer than {} s",
                        op.name(),
                        LONG_TIMEOUT.as_secs()
                    )));
                }
                match stream.next(Some(POLL)).map_err(|e| ipc_error(died(&lease.conn, e)))? {
                    StreamEvent::Idle => continue,
                    StreamEvent::Item(_) => {
                        return Err(StoreError::Engine(format!("the store worker streamed items for {}", op.name())));
                    }
                    StreamEvent::Done(bytes) => {
                        let json: Json = imt_ipc::wire::decode(&bytes)
                            .map_err(|e| StoreError::Engine(format!("the store worker sent an invalid reply: {e}")))?;
                        return decode(json);
                    }
                }
            }
        })();
        let result = match result {
            Err(_) if control.is_cancelled() => Err(StoreError::Cancelled),
            other => other,
        };
        if let Err(e) = &result {
            self.worker.report(&lease, e);
        }
        result
    }

    /// An analysis: cancellable through the thread's current control (`QueryControl::scope`; none: not).
    fn analysis<T: DeserializeOwned>(&self, op: StoreOp, body: &impl Serialize) -> StoreResult<T> {
        match QueryControl::current() {
            Some(control) => self.call_controlled(op, body, &control),
            None => self.call_with(op, body, LONG_TIMEOUT),
        }
    }

    fn session(&self, s: Session) -> StoreResult<ImportSession> {
        let (a, b, c) = (self.clone(), self.clone(), self.clone());
        ImportSession::remote(
            move || {
                a.call::<ProgressSnapshot>(StoreOp::SessionProgress, &s)
                    .unwrap_or_else(|_| ProgressSnapshot::stopped())
            },
            move || {
                let _ = b.call::<()>(StoreOp::SessionCancel, &s);
            },
            move || c.call_with(StoreOp::SessionWait, &s, WAIT_TIMEOUT),
        )
    }

    /// The store folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Remembers the profile for reopening after a restart (the worker's store has it already).
    fn remember_profile(&self, profile: &Profile) {
        *lock(&self.profile) = profile.clone();
    }
}

impl Projects for RemoteStore {
    fn store_dir(&self) -> PathBuf {
        self.dir.clone()
    }
    fn save_project(&self, manifest: &ProjectManifest) -> StoreResult<ProjectManifest> {
        self.call(StoreOp::SaveProject, manifest)
    }
    fn export_project(&self, manifest: &ProjectManifest, out: &Path) -> StoreResult<u64> {
        self.call_with(
            StoreOp::ExportProject,
            &ExportProject {
                manifest: manifest.clone(),
                out: out.to_path_buf(),
            },
            LONG_TIMEOUT,
        )
    }
    /// Closes the store in the worker (the folder stays). Later calls reopen it.
    fn close(&self) {
        let Ok(lease) = self.worker.inner.sup.lease() else { return };
        let handle = lock(&lease.conn.handles).remove(&self.dir);
        if let Some(store) = handle {
            let _ = raw(&lease.conn, &StoreRequest::CloseStore { store }, CALL_TIMEOUT);
        }
    }
}

impl Ingest for RemoteStore {
    fn open(&self, paths: &[PathBuf]) -> StoreResult<ImportSession> {
        let s = self.call(StoreOp::Open, &Paths { paths: paths.to_vec() })?;
        self.session(s)
    }
    fn reload(&self) -> StoreResult<ImportSession> {
        let s = self.call(StoreOp::Reload, &())?;
        self.session(s)
    }
    fn refresh(&self, changed: &[PathBuf]) -> StoreResult<ImportSession> {
        let s = self.call(StoreOp::Refresh, &Paths { paths: changed.to_vec() })?;
        self.session(s)
    }
    fn remove_source(&self, id: SourceId) -> StoreResult<()> {
        self.call(StoreOp::RemoveSource, &Source { id })
    }
    fn clear(&self) -> StoreResult<()> {
        self.call(StoreOp::Clear, &())
    }
    fn restore(&self, state: &WorkspaceState) -> StoreResult<ImportSession> {
        let s = self.call(StoreOp::Restore, state)?;
        self.session(s)
    }
    fn append_tail(&self, changed: &[PathBuf]) -> StoreResult<usize> {
        self.call_with(StoreOp::AppendTail, &Paths { paths: changed.to_vec() }, LONG_TIMEOUT)
    }
}

impl Query for RemoteStore {
    fn summary(&self) -> StoreResult<SummaryReply> {
        self.call(StoreOp::Summary, &())
    }
    fn page(&self, req: &PageQuery) -> StoreResult<PageReply> {
        self.call(StoreOp::Page, req)
    }
    fn events(&self, req: &EventsRequest) -> StoreResult<Vec<EventView>> {
        self.call(StoreOp::Events, req)
    }
    fn timeline(&self, req: &TimelineRequest) -> StoreResult<Timeline> {
        self.call(StoreOp::Timeline, req)
    }
    fn facet(&self, req: &FacetRequest) -> StoreResult<Facet> {
        self.call(StoreOp::Facet, req)
    }
    /// Batches stream from the worker as Arrow IPC with credits ([`crate::arrow_wire`]); `visit` breaking
    /// cancels the stream.
    fn scan(&self, filter: &Filter, visit: &mut dyn FnMut(&EventBatch) -> ControlFlow<()>) -> StoreResult<()> {
        let filter = encode(filter)?;
        let lease = self.worker.inner.sup.lease().map_err(ipc_error)?;
        let result = (|| {
            let store = self.ensure_open(&lease.conn, false)?;
            let mut stream = lease
                .conn
                .child
                .peer()
                .stream_request(&StoreRequest::Scan { store, filter }, protocol::SCAN_CREDIT)
                .map_err(|e| ipc_error(died(&lease.conn, e)))?;
            loop {
                match stream.next(Some(LONG_TIMEOUT)).map_err(|e| ipc_error(died(&lease.conn, e)))? {
                    StreamEvent::Idle => return Err(StoreError::Engine("the store worker stopped sending the scan".into())),
                    StreamEvent::Item(bytes) => {
                        let batch = crate::arrow_wire::decode(&bytes).map_err(|e| StoreError::Engine(e.0))?;
                        if visit(&batch).is_break() {
                            stream.cancel();
                            return Ok(());
                        }
                    }
                    StreamEvent::Done(_) => return Ok(()),
                }
            }
        })();
        if let Err(e) = &result {
            self.worker.report(&lease, e);
        }
        result
    }
    fn chart(&self, req: &ChartRequest) -> StoreResult<ChartData> {
        self.analysis(StoreOp::Chart, req)
    }
    fn field_stats(&self, req: &FieldStatsRequest) -> StoreResult<FieldStats> {
        self.analysis(StoreOp::FieldStats, req)
    }
    fn source_context(&self, req: &ContextRequest) -> StoreResult<SourceContext> {
        self.call(StoreOp::SourceContext, req)
    }
    fn run_query_with(&self, req: &QueryRequest, control: &QueryControl) -> StoreResult<QlResult> {
        self.call_controlled(StoreOp::RunQuery, req, control)
    }
    fn check_query(&self, text: &str) -> StoreResult<Option<QlError>> {
        self.call(StoreOp::CheckQuery, &text)
    }
    fn complete_query(&self, req: &CompletionRequest) -> StoreResult<Vec<Completion>> {
        self.call(StoreOp::CompleteQuery, req)
    }
    fn export(&self, req: &ExportRequest) -> StoreResult<ExportResult> {
        self.analysis(StoreOp::Export, req)
    }
}

impl Positions for RemoteStore {
    fn ids_at(&self, from: u64, len: usize) -> StoreResult<Vec<EventId>> {
        self.call(StoreOp::IdsAt, &IdsAt { from, len })
    }
    fn position_of(&self, id: EventId) -> StoreResult<Option<u64>> {
        self.call(StoreOp::PositionOf, &id)
    }
    fn positioned(&self) -> StoreResult<u64> {
        self.call(StoreOp::Positioned, &())
    }
}

impl Analysis for RemoteStore {
    fn patterns(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Pattern>>> {
        self.analysis(StoreOp::Patterns, req)
    }
    fn exception_groups(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<ExceptionGroup>>> {
        self.analysis(StoreOp::ExceptionGroups, req)
    }
    fn anomalies(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Anomaly>>> {
        self.analysis(StoreOp::Anomalies, req)
    }
    fn compare(&self, req: &CompareRequest) -> StoreResult<Analyzed<Comparison>> {
        self.analysis(StoreOp::Compare, req)
    }
    fn correlate(&self, req: &CorrelateRequest) -> StoreResult<Analyzed<Vec<EventId>>> {
        self.analysis(StoreOp::Correlate, req)
    }
    fn traces(&self, req: &TraceRequest) -> StoreResult<Analyzed<(Vec<TraceSummary>, Vec<WaterfallRow>)>> {
        self.analysis(StoreOp::Traces, req)
    }
    fn requests(&self, req: &RequestsRequest) -> StoreResult<Analyzed<Vec<RequestSpan>>> {
        self.analysis(StoreOp::Requests, req)
    }
    fn moment(&self, req: &MomentRequest) -> StoreResult<Analyzed<MomentSummary>> {
        self.analysis(StoreOp::Moment, req)
    }
    fn overview(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Finding>>> {
        self.analysis(StoreOp::Overview, req)
    }
    fn runs(&self, req: &RunsRequest) -> StoreResult<RunsReply> {
        self.analysis(StoreOp::Runs, req)
    }
}

impl Profiles for RemoteStore {
    fn active_profile(&self) -> StoreResult<Profile> {
        self.call(StoreOp::ActiveProfile, &())
    }
    fn validate_profile(&self, profile: &Profile) -> StoreResult<()> {
        self.call(StoreOp::ValidateProfile, profile)
    }
    fn set_profile(&self, profile: Profile) -> StoreResult<ImportSession> {
        let s = self.call(StoreOp::SetProfile, &profile)?;
        self.remember_profile(&profile);
        self.session(s)
    }
    fn preview_parse(&self, req: &ParsePreviewRequest) -> StoreResult<ParsePreview> {
        self.analysis(StoreOp::PreviewParse, req)
    }
    fn preview_extractor(&self, req: &ExtractPreviewRequest) -> StoreResult<ExtractPreview> {
        self.analysis(StoreOp::PreviewExtractor, req)
    }
    fn preview_spans(&self, req: &imt_core::spans::SpanPreviewRequest) -> StoreResult<imt_core::spans::SpanPreview> {
        self.analysis(StoreOp::PreviewSpans, req)
    }
    fn propose_span_rule(&self, req: &SpanRuleRequest) -> StoreResult<SpanProposal> {
        self.analysis(StoreOp::ProposeSpanRule, req)
    }
    fn evaluate_span_rule(&self, req: &SpanRuleCheck) -> StoreResult<Analyzed<SpanEvaluation>> {
        self.analysis(StoreOp::EvaluateSpanRule, req)
    }
}

impl Diagnostics for RemoteStore {
    fn problems(&self) -> StoreResult<Vec<DiagnosticRow>> {
        self.call(StoreOp::Problems, &())
    }
    fn evaluate_fix(&self, fix: &QuickFix) -> StoreResult<FixEvaluation> {
        self.analysis(StoreOp::EvaluateFix, fix)
    }
}

impl Search for RemoteStore {
    fn search(&self, req: &SearchRequest) -> StoreResult<SearchReply> {
        self.call(StoreOp::Search, req)
    }
    fn index_status(&self) -> StoreResult<IndexStatus> {
        self.call(StoreOp::IndexStatus, &())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_path_puts_the_bundled_folder_first() {
        let (var, sep) = library_path_var();
        let (name, value) = library_path_env(Some(Path::new("/opt/imt/lib"))).unwrap();
        assert_eq!(name, var);
        assert!(value.starts_with("/opt/imt/lib"), "{value}");
        if let Ok(rest) = std::env::var(var) {
            if !rest.is_empty() {
                assert_eq!(value, format!("/opt/imt/lib{sep}{rest}"));
            }
        }
    }

    #[test]
    fn a_missing_database_library_is_a_clear_message_before_anything_starts() {
        let dir = std::env::temp_dir();
        assert!(find_library("no-such-library.dll", std::slice::from_ref(&dir)).is_none());
        let mut cfg = WorkerConfig::new(dir.join("imt-store-worker-test.exe"));
        cfg.library_dirs = Some(vec![dir.join("no-such-folder")]);
        let w = StoreWorker::new(cfg).unwrap();
        let e = w
            .open_store(&dir.join("never-made"), Profile::default_profile(), true, None)
            .err()
            .unwrap();
        assert!(e.to_string().contains(libduckdb_file()), "{e}");
        assert!(w.status().pid.is_none(), "the worker was not started");
    }

    #[test]
    fn crashes_are_retryable_and_cancels_are_typed() {
        let e = ipc_error(IpcError::Died("gone".into()));
        assert!(e.is_retryable() && e.to_string().contains(CRASHED));
        assert_eq!(
            ipc_error(IpcError::Remote(imt_ipc::CallCtx::cancelled_error())),
            StoreError::Cancelled
        );
    }
}
