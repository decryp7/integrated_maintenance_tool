//! The app side: [`StoreWorker`] starts and supervises the worker process (the ADR 0002 pattern through
//! `imt_ipc`: lazy start, restart with backoff, quarantine after repeated crashes, killed with IMT, a memory
//! cap on Windows), and [`RemoteStore`] implements the store ports for one store folder over it. It holds no
//! events: every reply is a page, a summary or an aggregate.
//!
//! A crashed worker is restarted by the next call, which reopens the store (the database is durable); the
//! call that saw the crash fails with a retryable [`StoreError::Engine`], and an import that was running is
//! reported as failed.

use crate::protocol::{self, method, Handle, IdsAt, Limits, OpenStore, Paths, Session, Source, WorkerStatus};
use crate::server::pb::{self, store_client::StoreClient};
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
use imt_ipc::client::AuthChannel;
use imt_ipc::process::{launch, ChildProcess, SpawnSpec};
use imt_ipc::supervisor::{ChildStatus, Supervisor, SupervisorConfig};
use imt_ipc::IpcError;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::runtime::Runtime;

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
    /// Hard memory cap of the process (Windows job object), bytes; `None` for none.
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
    client: StoreClient<AuthChannel>,
    process: ChildProcess,
    _endpoint: imt_ipc::endpoint::PrivateEndpoint,
    /// Store folder -> handle in this process.
    handles: Mutex<HashMap<PathBuf, u64>>,
}

impl imt_ipc::supervisor::Connection for Conn {
    fn pid(&self) -> Option<u32> {
        Some(self.process.pid())
    }
    fn kill(&self) {
        self.process.kill();
    }
}

struct Inner {
    rt: Runtime,
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
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("imt-store-client")
            .enable_all()
            .build()
            .map_err(|e| StoreError::Engine(format!("cannot start the client runtime: {e}")))?;
        let handle = rt.handle().clone();
        let spec = {
            let mut s = SpawnSpec::new(cfg.exe.clone(), "the store worker");
            s.endpoint_prefix = "imt-store".into();
            s.memory_limit = cfg.memory_cap;
            s
        };
        let sup = Supervisor::new(cfg.supervisor.clone(), move || {
            let launched = handle.block_on(launch(&spec))?;
            let client = StoreClient::new(launched.channel)
                .max_decoding_message_size(protocol::MAX_MESSAGE_BYTES)
                .max_encoding_message_size(protocol::MAX_MESSAGE_BYTES);
            Ok(Conn {
                client,
                process: launched.process,
                _endpoint: launched.endpoint,
                handles: Mutex::new(HashMap::new()),
            })
        });
        Ok(StoreWorker {
            inner: Arc::new(Inner { rt, sup }),
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
            .call(|c| raw(&self.inner.rt, c, method::STATUS, 0, Vec::new(), CALL_TIMEOUT))
            .map_err(ipc_error)
            .and_then(|b| decode(&b))
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

fn decode<T: DeserializeOwned>(b: &[u8]) -> StoreResult<T> {
    serde_json::from_slice(b).map_err(|e| StoreError::Engine(format!("the store worker sent an invalid reply: {e}")))
}

fn raw(rt: &Runtime, c: &Conn, method: &str, store: u64, body: Vec<u8>, timeout: Duration) -> Result<Vec<u8>, IpcError> {
    let mut client = c.client.clone();
    let mut req = tonic::Request::new(pb::CallRequest {
        method: method.to_string(),
        store,
        body,
    });
    req.set_timeout(timeout);
    rt.block_on(client.call(req))
        .map(|r| r.into_inner().body)
        .map_err(|s| imt_ipc::error::from_status(&s, "the store worker", || c.process.exit_text()))
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
        let body = serde_json::to_vec(&req).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let h: Handle = decode(&raw(&self.worker.inner.rt, conn, method::OPEN_STORE, 0, body, CALL_TIMEOUT).map_err(ipc_error)?)?;
        lock(&conn.handles).insert(self.dir.clone(), h.store);
        Ok(h.store)
    }

    fn call_with<T: DeserializeOwned>(&self, method: &str, body: &impl Serialize, timeout: Duration) -> StoreResult<T> {
        let body = serde_json::to_vec(body).map_err(|e| StoreError::Invalid(format!("cannot encode the request: {e}")))?;
        let w = &self.worker.inner;
        let lease = w.sup.lease().map_err(ipc_error)?;
        let result = self
            .ensure_open(&lease.conn, false)
            .and_then(|h| raw(&w.rt, &lease.conn, method, h, body, timeout).map_err(ipc_error))
            .and_then(|b| decode(&b));
        if let Err(e) = &result {
            self.worker.report(&lease, e);
        }
        result
    }

    fn call<T: DeserializeOwned>(&self, method: &str, body: &impl Serialize) -> StoreResult<T> {
        self.call_with(method, body, CALL_TIMEOUT)
    }

    fn session(&self, s: Session) -> StoreResult<ImportSession> {
        let (a, b, c) = (self.clone(), self.clone(), self.clone());
        ImportSession::remote(
            move || {
                a.call::<ProgressSnapshot>(method::SESSION_PROGRESS, &())
                    .unwrap_or_else(|_| ProgressSnapshot::stopped())
            },
            move || {
                let _ = b.call::<()>(method::SESSION_CANCEL, &());
            },
            move || c.call_with(method::SESSION_WAIT, &s, WAIT_TIMEOUT),
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
        let s = self.call(method::OPEN, &Paths { paths: paths.to_vec() })?;
        self.session(s)
    }
    fn reload(&self) -> StoreResult<ImportSession> {
        let s = self.call(method::RELOAD, &())?;
        self.session(s)
    }
    fn refresh(&self, changed: &[PathBuf]) -> StoreResult<ImportSession> {
        let s = self.call(method::REFRESH, &Paths { paths: changed.to_vec() })?;
        self.session(s)
    }
    fn remove_source(&self, id: SourceId) -> StoreResult<()> {
        self.call(method::REMOVE_SOURCE, &Source { id })
    }
    fn clear(&self) -> StoreResult<()> {
        self.call(method::CLEAR, &())
    }
    fn restore(&self, _state: &WorkspaceState) -> StoreResult<ImportSession> {
        Err(not_remote("restore"))
    }
    fn append_tail(&self, changed: &[PathBuf]) -> StoreResult<usize> {
        self.call(method::APPEND_TAIL, &Paths { paths: changed.to_vec() })
    }
}

impl Query for RemoteStore {
    fn summary(&self) -> StoreResult<SummaryReply> {
        self.call(method::SUMMARY, &())
    }
    fn page(&self, req: &PageQuery) -> StoreResult<PageReply> {
        self.call(method::PAGE, req)
    }
    fn timeline(&self, req: &TimelineRequest) -> StoreResult<Timeline> {
        self.call(method::TIMELINE, req)
    }
    fn facet(&self, req: &FacetRequest) -> StoreResult<Facet> {
        self.call(method::FACET, req)
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
    fn run_query(&self, _req: &QueryRequest) -> StoreResult<imt_core::ql::QlResult> {
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
        self.call(method::IDS_AT, &IdsAt { from, len })
    }
    fn position_of(&self, id: EventId) -> StoreResult<Option<u64>> {
        self.call(method::POSITION_OF, &id)
    }
    fn positioned(&self) -> StoreResult<u64> {
        self.call(method::POSITIONED, &())
    }
}

impl Profiles for RemoteStore {
    fn active_profile(&self) -> StoreResult<Profile> {
        self.call(method::ACTIVE_PROFILE, &())
    }
    fn validate_profile(&self, profile: &Profile) -> StoreResult<()> {
        self.call(method::VALIDATE_PROFILE, profile)
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
}

impl Diagnostics for RemoteStore {
    fn problems(&self) -> StoreResult<Vec<DiagnosticRow>> {
        self.call(method::PROBLEMS, &())
    }
    fn evaluate_fix(&self, _fix: &QuickFix) -> StoreResult<FixEvaluation> {
        Err(not_remote("quick-fix evaluation"))
    }
}

impl Search for RemoteStore {
    fn search(&self, req: &SearchRequest) -> StoreResult<SearchReply> {
        self.call(method::SEARCH, req)
    }
    fn index_status(&self) -> StoreResult<IndexStatus> {
        self.call(method::INDEX_STATUS, &())
    }
}
