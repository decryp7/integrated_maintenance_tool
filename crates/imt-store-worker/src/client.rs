//! The app side: [`StoreWorker`] starts and supervises the worker process (`imt_ipc` frames, ADR 0017: lazy
//! start, restart with backoff, quarantine after repeated crashes, killed with IMT, a memory cap), and
//! [`RemoteStore`] implements the store ports for one store folder over it. It holds no events: every reply is
//! a page, a summary or an aggregate.
//!
//! A crashed worker is restarted by the next call, which reopens the store (the database is durable); the
//! call that saw the crash fails with a retryable [`StoreError::Engine`], and an import that was running is
//! reported as failed.

use crate::protocol::{self, Handle, IdsAt, Limits, OpenStore, Paths, Session, Source, StoreOp, StoreRequest, WorkerStatus};
use imt_core::api::{
    ChartData, ChartRequest, CompletionRequest, ContextRequest, ExportRequest, ExportResult, ExtractPreview, ExtractPreviewRequest, Facet,
    FacetRequest, FieldStatsRequest, ParsePreview, ParsePreviewRequest, QueryRequest, SourceContext, Timeline, TimelineRequest,
    WorkspaceState,
};
use imt_core::batch::{EventBatch, EventId, SourceId};
use imt_core::ports::{
    DiagnosticRow, Diagnostics, Filter, FixEvaluation, ImportSession, IndexStatus, Ingest, PageQuery, PageReply, Positions, Profiles,
    Query, QuickFix, Search, SearchReply, SearchRequest, StoreError, StoreResult, SummaryReply,
};
use imt_core::workspace::ProgressSnapshot;
use imt_core::Profile;
use imt_ipc::launch::{launch, Child, ChildSpec};
use imt_ipc::peer::{NoCalls, PeerOptions};
use imt_ipc::process::SpawnSpec;
use imt_ipc::sandbox::SandboxSpec;
use imt_ipc::supervisor::{ChildStatus, Supervisor, SupervisorConfig};
use imt_ipc::{IpcError, Json};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Deadline of an ordinary call (a page, a timeline, an aggregate).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Deadline of waiting for an import.
pub const WAIT_TIMEOUT: Duration = Duration::from_secs(24 * 3600);

/// Memory cap of the worker process by default: the import budget (600 MB) with headroom, so only a
/// runaway is stopped (the job object kills the process; the supervisor restarts it).
pub const DEFAULT_MEMORY_CAP: u64 = 1 << 30;

/// How to start the worker.
#[derive(Clone)]
pub struct WorkerConfig {
    pub exe: PathBuf,
    /// Hard memory cap of the process (job object on Windows, address-space limit on Unix), bytes; `None` for none.
    pub memory_cap: Option<u64>,
    /// Restart policy (crash window, backoff, quarantine).
    pub supervisor: SupervisorConfig,
}

impl WorkerConfig {
    pub fn new(exe: impl Into<PathBuf>) -> WorkerConfig {
        WorkerConfig {
            exe: exe.into(),
            memory_cap: Some(DEFAULT_MEMORY_CAP),
            supervisor: SupervisorConfig::new("the store worker"),
        }
    }
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
        // It opens the user's store folders and the DuckDB extension cache by path: a memory cap and a cleared
        // environment, no file-system restriction yet (the store folder handle is passed in 6c).
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
        let sup = Supervisor::new(cfg.supervisor.clone(), move || {
            Ok(Conn {
                child: launch(&spec, Arc::new(NoCalls))?,
                handles: Mutex::new(HashMap::new()),
            })
        });
        Ok(StoreWorker {
            inner: Arc::new(Inner { sup }),
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

    fn report(&self, lease: &imt_ipc::supervisor::Lease<Conn>, e: &StoreError) {
        if let StoreError::Engine(m) = e {
            if m.starts_with(CRASHED) {
                self.inner.sup.report(lease, &IpcError::Died(m.clone()));
            }
        }
    }
}

/// Prefix of the error of a call whose worker failed.
const CRASHED: &str = "the store worker stopped";

fn ipc_error(e: IpcError) -> StoreError {
    match e {
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
    c.child.peer().request(req, Some(timeout)).map_err(|e| match e {
        IpcError::Died(_) if c.child.exited() => IpcError::Died(c.child.exit_text()),
        other => other,
    })
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

    fn session(&self, s: Session) -> StoreResult<ImportSession> {
        let (a, b, c) = (self.clone(), self.clone(), self.clone());
        ImportSession::remote(
            move || {
                a.call::<ProgressSnapshot>(StoreOp::SessionProgress, &())
                    .unwrap_or_else(|_| ProgressSnapshot::stopped())
            },
            move || {
                let _ = b.call::<()>(StoreOp::SessionCancel, &());
            },
            move || c.call_with(StoreOp::SessionWait, &s, WAIT_TIMEOUT),
        )
    }

    /// The store folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

fn not_remote(operation: &'static str) -> StoreError {
    StoreError::unsupported(operation, "6c (over the store worker)")
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
    fn restore(&self, _state: &WorkspaceState) -> StoreResult<ImportSession> {
        Err(not_remote("restore"))
    }
    fn append_tail(&self, changed: &[PathBuf]) -> StoreResult<usize> {
        self.call(StoreOp::AppendTail, &Paths { paths: changed.to_vec() })
    }
}

impl Query for RemoteStore {
    fn summary(&self) -> StoreResult<SummaryReply> {
        self.call(StoreOp::Summary, &())
    }
    fn page(&self, req: &PageQuery) -> StoreResult<PageReply> {
        self.call(StoreOp::Page, req)
    }
    fn timeline(&self, req: &TimelineRequest) -> StoreResult<Timeline> {
        self.call(StoreOp::Timeline, req)
    }
    fn facet(&self, req: &FacetRequest) -> StoreResult<Facet> {
        self.call(StoreOp::Facet, req)
    }
    /// Batches do not cross the process boundary yet (R3 sends Arrow IPC frames).
    fn scan(&self, _filter: &Filter, _visit: &mut dyn FnMut(&EventBatch) -> ControlFlow<()>) -> StoreResult<()> {
        Err(not_remote("scan"))
    }
    fn chart(&self, _req: &ChartRequest) -> StoreResult<ChartData> {
        Err(not_remote("chart"))
    }
    fn field_stats(&self, _req: &FieldStatsRequest) -> StoreResult<imt_core::analysis::FieldStats> {
        Err(not_remote("field statistics"))
    }
    fn source_context(&self, _req: &ContextRequest) -> StoreResult<SourceContext> {
        Err(not_remote("source context"))
    }
    fn run_query_with(&self, _req: &QueryRequest, _control: &imt_core::ql::QueryControl) -> StoreResult<imt_core::ql::QlResult> {
        Err(not_remote("the query language"))
    }
    fn check_query(&self, _text: &str) -> StoreResult<Option<imt_core::ql::QlError>> {
        Err(not_remote("the query language"))
    }
    fn complete_query(&self, _req: &CompletionRequest) -> StoreResult<Vec<imt_core::ql::Completion>> {
        Err(not_remote("the query language"))
    }
    fn export(&self, _req: &ExportRequest) -> StoreResult<ExportResult> {
        Err(not_remote("export"))
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

impl Profiles for RemoteStore {
    fn active_profile(&self) -> StoreResult<Profile> {
        self.call(StoreOp::ActiveProfile, &())
    }
    fn validate_profile(&self, profile: &Profile) -> StoreResult<()> {
        self.call(StoreOp::ValidateProfile, profile)
    }
    fn set_profile(&self, _profile: Profile) -> StoreResult<ImportSession> {
        Err(not_remote("changing the profile"))
    }
    fn preview_parse(&self, _req: &ParsePreviewRequest) -> StoreResult<ParsePreview> {
        Err(not_remote("parse preview"))
    }
    fn preview_extractor(&self, _req: &ExtractPreviewRequest) -> StoreResult<ExtractPreview> {
        Err(not_remote("extractor preview"))
    }
    fn propose_span_rule(&self, _req: &imt_core::spans::infer::SpanRuleRequest) -> StoreResult<imt_core::spans::infer::SpanProposal> {
        Err(not_remote("span rule inference"))
    }
    fn evaluate_span_rule(
        &self,
        _req: &imt_core::spans::infer::SpanRuleCheck,
    ) -> StoreResult<imt_core::ports::Analyzed<imt_core::spans::infer::SpanEvaluation>> {
        Err(not_remote("span rule evaluation"))
    }
}

impl Diagnostics for RemoteStore {
    fn problems(&self) -> StoreResult<Vec<DiagnosticRow>> {
        self.call(StoreOp::Problems, &())
    }
    fn evaluate_fix(&self, _fix: &QuickFix) -> StoreResult<FixEvaluation> {
        Err(not_remote("quick-fix evaluation"))
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
