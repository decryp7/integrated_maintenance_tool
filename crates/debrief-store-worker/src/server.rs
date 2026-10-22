//! The worker side: the stores this process hosts (one per project folder; handles are per process), the
//! import sessions they run, and the handler that dispatches Debrief's calls to them (ADR 0017 frames). Calls run on
//! blocking threads, so a long one (an import's wait, a big query) never holds up the others; a panic in a call
//! is an engine error for that call, not a crash of the worker. A cancelled call interrupts the statement it
//! runs (the caller's `Cancel` frame reaches [`DuckStore::with_control`]).

use crate::plugins::{NoPlugins, PluginChoice, PluginSource};
use debrief_core::api::WorkspaceState;
use debrief_core::package::ProjectManifest;
use debrief_core::ports::{
    Analysis, Console, Diagnostics, Filter, ImportSession, Ingest, Inspect, Maintain, Positions, Profiles, Projects, Query, Search,
    StoreError, StoreResult,
};
use debrief_core::ql::QueryControl;
use debrief_core::{PluginRegistry, Profile};
use debrief_ipc::CancelProbe;
use debrief_store_client::protocol::{
    self, ExportProject, Handle, IdsAt, Limits, MemoryMode, OpenProject, OpenStore, OpenedProject, Paths, Session, Source, StoreOp,
    StoreRequest, StoreTuning, ThirdPartyPlugin, WorkerStatus,
};
use debrief_store_duck::{DuckStore, StoreOptions};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn body<T: DeserializeOwned>(v: serde_json::Value) -> StoreResult<T> {
    serde_json::from_value(v).map_err(|e| StoreError::Invalid(format!("invalid request: {e}")))
}

fn reply<T: Serialize>(v: &T) -> StoreResult<serde_json::Value> {
    serde_json::to_value(v).map_err(|e| StoreError::Engine(format!("cannot encode the reply: {e}")))
}

pub use crate::plugins::install_plugins;

/// The stores of this worker.
pub struct Worker {
    stores: Mutex<HashMap<u64, (PathBuf, DuckStore)>>,
    sessions: Mutex<HashMap<u64, ImportSession>>,
    next: AtomicU64,
    plugins: Arc<dyn PluginSource>,
    mode: MemoryMode,
    /// The knobs of Settings ▸ Performance (its command line, see [`StoreTuning::from_args`]).
    tuning: StoreTuning,
}

impl Default for Worker {
    fn default() -> Self {
        Worker::new(Arc::new(NoPlugins))
    }
}

impl Worker {
    pub fn new(plugins: Arc<dyn PluginSource>) -> Worker {
        Worker::with_mode(plugins, MemoryMode::Normal)
    }

    /// A worker whose stores use the options of `mode` unless a request names limits.
    pub fn with_mode(plugins: Arc<dyn PluginSource>, mode: MemoryMode) -> Worker {
        Worker {
            stores: Mutex::default(),
            sessions: Mutex::default(),
            next: AtomicU64::new(0),
            plugins,
            mode,
            tuning: StoreTuning::default(),
        }
    }

    /// The worker with the knobs of Settings ▸ Performance over its mode's values.
    pub fn with_tuning(mut self, tuning: StoreTuning) -> Worker {
        self.tuning = tuning;
        self
    }

    /// The options of a store opened in this worker: the mode's, with the worker's tuning, with the request's limits.
    pub fn options(&self, limits: Option<Limits>) -> StoreOptions {
        let mode = match self.mode {
            MemoryMode::Normal => StoreOptions::default(),
            MemoryMode::Low => StoreOptions::low_memory(),
        };
        let defaults = tuned(mode, &self.tuning);
        match limits {
            Some(l) => StoreOptions {
                parse_threads: l.parse_threads,
                duck_threads: l.duck_threads,
                memory_limit: l.memory_limit,
                ..defaults
            },
            None => defaults,
        }
    }

    fn registry(&self, disabled: &[String], safe_mode: bool, third_party: &[ThirdPartyPlugin]) -> PluginRegistry {
        self.plugins.registry(&PluginChoice {
            disabled,
            safe_mode,
            third_party,
        })
    }

    fn store(&self, handle: u64) -> StoreResult<DuckStore> {
        lock(&self.stores)
            .get(&handle)
            .map(|(_, s)| s.clone())
            .ok_or_else(|| StoreError::NotFound(format!("store {handle} is not open in this worker")))
    }

    fn open_store(&self, req: OpenStore) -> StoreResult<Handle> {
        if req.create {
            std::fs::create_dir_all(&req.dir)?;
        }
        let dir = std::fs::canonicalize(&req.dir).map_err(|e| StoreError::Io(format!("{}: {e}", req.dir.display())))?;
        // A folder is open once per worker: a second open gets the same store; a create replaces it.
        if req.create {
            lock(&self.stores).retain(|_, (d, _)| *d != dir);
        } else if let Some((h, _)) = lock(&self.stores).iter().find(|(_, (d, _))| *d == dir) {
            return Ok(Handle {
                store: *h,
                wire: protocol::WIRE_VERSION,
            });
        }
        let opts = self.options(req.limits);
        let registry = self.registry(&req.disabled_plugins, req.safe_mode, &req.third_party);
        let store = if req.create {
            DuckStore::create_with(&dir, req.profile, registry, opts)?
        } else {
            DuckStore::open_with(&dir, req.profile, registry, opts)?
        };
        if let Some(setup) = &req.parsers {
            debrief_core::ports::Profiles::set_parsers(&store, setup)?;
        }
        let h = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        lock(&self.stores).insert(h, (dir, store));
        Ok(Handle {
            store: h,
            wire: protocol::WIRE_VERSION,
        })
    }

    fn open_project(&self, req: OpenProject) -> StoreResult<OpenedProject> {
        std::fs::create_dir_all(&req.dest_root)?;
        let registry = self.registry(&req.disabled_plugins, req.safe_mode, &req.third_party);
        let (store, manifest) = DuckStore::open_project(&req.path, &req.dest_root, req.profile, registry)?;
        if let Some(setup) = &req.parsers {
            debrief_core::ports::Profiles::set_parsers(&store, setup)?;
        }
        if req.limits.is_some() || self.mode != MemoryMode::Normal || self.tuning != StoreTuning::default() {
            store.configure(self.options(req.limits))?;
        }
        let dir = std::fs::canonicalize(store.dir()).map_err(|e| StoreError::Io(format!("{}: {e}", store.dir().display())))?;
        let mut stores = lock(&self.stores);
        stores.retain(|_, (d, _)| *d != dir);
        let h = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        stores.insert(h, (dir.clone(), store));
        Ok(OpenedProject { store: h, dir, manifest })
    }

    fn session(&self, s: ImportSession) -> Session {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        lock(&self.sessions).insert(id, s);
        Session { session: id }
    }

    /// Handles one request; a panic is an engine error of the call. `probe` tells whether the caller cancelled;
    /// `items` sends one streamed item (false: the caller is gone or cancelled).
    pub fn handle(
        &self,
        req: StoreRequest,
        probe: Option<CancelProbe>,
        items: &mut dyn FnMut(Vec<u8>) -> bool,
    ) -> StoreResult<serde_json::Value> {
        catch_unwind(AssertUnwindSafe(|| self.dispatch(req, probe, items))).unwrap_or_else(|p| {
            Err(StoreError::Engine(format!(
                "the store worker hit an internal error: {}",
                debrief_core::guard::panic_message(p.as_ref())
            )))
        })
    }

    fn dispatch(
        &self,
        req: StoreRequest,
        probe: Option<CancelProbe>,
        items: &mut dyn FnMut(Vec<u8>) -> bool,
    ) -> StoreResult<serde_json::Value> {
        match req {
            StoreRequest::OpenStore(b) => reply(&self.open_store(body(b.0)?)?),
            StoreRequest::OpenProject(b) => reply(&self.open_project(body(b.0)?)?),
            StoreRequest::SetPlugins(b) => {
                let plugins: Vec<ThirdPartyPlugin> = body(b.0)?;
                self.plugins.set_third_party(&plugins);
                reply(&())
            }
            StoreRequest::CloseStore { store } => {
                lock(&self.stores).remove(&store);
                reply(&())
            }
            StoreRequest::Status => reply(&WorkerStatus {
                pid: std::process::id(),
                rss_bytes: debrief_ipc::process::current_rss_bytes(),
                stores: lock(&self.stores).len(),
                memory_mode: self.mode,
                tuning: self.tuning.clone(),
            }),
            StoreRequest::Scan { store, filter } => {
                let filter: Filter = body(filter.0)?;
                let s = self.store(store)?;
                let mut failed: Option<StoreError> = None;
                cancellable(probe, || {
                    s.scan(
                        &filter,
                        &mut |batch| match debrief_store_client::arrow_wire::encode(batch, protocol::MAX_MESSAGE_BYTES - 4096) {
                            Ok(parts) => {
                                for p in parts {
                                    if !items(p) {
                                        return ControlFlow::Break(());
                                    }
                                }
                                ControlFlow::Continue(())
                            }
                            Err(e) => {
                                failed = Some(StoreError::Engine(e.0));
                                ControlFlow::Break(())
                            }
                        },
                    )
                })?;
                match failed {
                    Some(e) => Err(e),
                    None => reply(&()),
                }
            }
            StoreRequest::Call { store, op, body: b } => self.call(store, op, b.0, probe, items),
        }
    }

    fn call(
        &self,
        h: u64,
        op: StoreOp,
        b: serde_json::Value,
        probe: Option<CancelProbe>,
        items: &mut dyn FnMut(Vec<u8>) -> bool,
    ) -> StoreResult<serde_json::Value> {
        let s = self.store(h)?;
        self.run_op(&s, op, b, dispatch::Call { probe, items })
    }
}

#[path = "dispatch.rs"]
mod dispatch;

/// Runs `f` under a control that the caller's cancel triggers (when there is a probe). A scan streams its own
/// items, so it reports no progress.
fn cancellable<R>(probe: Option<CancelProbe>, f: impl FnOnce() -> StoreResult<R>) -> StoreResult<R> {
    match probe {
        None => f(),
        Some(p) => {
            let control = QueryControl::new();
            let _watch = Watch::start(p, control.clone());
            DuckStore::with_control(&control, f)
        }
    }
}

/// Runs `f` under a control that the caller's cancel triggers (when there is a probe), sending its progress to
/// `items` meanwhile.
fn controlled<R: Send>(
    probe: Option<CancelProbe>,
    items: &mut dyn FnMut(Vec<u8>) -> bool,
    f: impl FnOnce() -> StoreResult<R> + Send,
) -> StoreResult<R> {
    match probe {
        None => f(),
        Some(p) => {
            let control = QueryControl::new();
            let _watch = Watch::start(p, control.clone());
            with_progress(&control, items, || DuckStore::with_control(&control, f))
        }
    }
}

/// Runs `f` on its own thread while this one sends `control`'s progress to `items` every
/// [`protocol::PROGRESS_EVERY`] when it changed (the caller's stream; a refused item ends the sending, not the
/// run). A panic in `f` is an engine error of the call.
fn with_progress<R: Send>(
    control: &QueryControl,
    items: &mut dyn FnMut(Vec<u8>) -> bool,
    f: impl FnOnce() -> StoreResult<R> + Send,
) -> StoreResult<R> {
    std::thread::scope(|scope| {
        let (done, finished) = std::sync::mpsc::channel::<()>();
        let run = std::thread::Builder::new()
            .name("debrief-store-call".into())
            .spawn_scoped(scope, move || {
                let out = f();
                let _ = done.send(());
                out
            })
            .map_err(|e| StoreError::Engine(format!("cannot start the call: {e}")))?;
        let mut last = None;
        let mut sending = true;
        while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = finished.recv_timeout(protocol::PROGRESS_EVERY) {
            let now = control.progress();
            if sending && last != Some(now) {
                last = Some(now);
                sending = match debrief_ipc::wire::encode(&now) {
                    Ok(bytes) => items(bytes),
                    Err(e) => {
                        log::warn!("cannot encode the progress: {e}");
                        false
                    }
                };
            }
        }
        run.join().unwrap_or_else(|p| {
            Err(StoreError::Engine(format!(
                "the store worker hit an internal error: {}",
                debrief_core::guard::panic_message(p.as_ref())
            )))
        })
    })
}

/// Cancels a control when the caller cancels the call; stops when dropped.
struct Watch {
    done: Arc<AtomicBool>,
}

impl Watch {
    fn start(probe: CancelProbe, control: QueryControl) -> Watch {
        let done = Arc::new(AtomicBool::new(false));
        let d = done.clone();
        let spawned = std::thread::Builder::new().name("debrief-store-cancel".into()).spawn(move || {
            while !d.load(Ordering::Relaxed) {
                if probe.is_cancelled() {
                    control.cancel();
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        if let Err(e) = spawned {
            log::warn!("cannot watch for cancels: {e}");
        }
        Watch { done }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
    }
}

/// Answers Debrief's [`StoreRequest`]s.
pub struct StoreHandler {
    worker: Arc<Worker>,
}

impl StoreHandler {
    pub fn new(worker: Arc<Worker>) -> StoreHandler {
        StoreHandler { worker }
    }
}

impl debrief_ipc::Handler for StoreHandler {
    fn call(&self, ctx: &debrief_ipc::CallCtx, body: &[u8]) -> Result<Vec<u8>, debrief_ipc::RemoteError> {
        let req: StoreRequest = debrief_ipc::peer::decode_request(body)?;
        let probe = ctx.cancel_probe();
        let mut send = |item: Vec<u8>| ctx.send_item(item).is_ok();
        let value = match self.worker.handle(req, Some(probe.clone()), &mut send) {
            Ok(v) => v,
            Err(_) if probe.is_cancelled() => return Err(debrief_ipc::CallCtx::cancelled_error()),
            Err(e) => return Err(protocol::to_remote(&e)),
        };
        debrief_ipc::peer::encode_reply(&debrief_ipc::Json(value))
    }
}

/// `base` with each knob `tuning` names.
fn tuned(base: StoreOptions, tuning: &StoreTuning) -> StoreOptions {
    let mb = |n: u64| format!("{n}MB");
    StoreOptions {
        parse_threads: tuning.parse_threads.unwrap_or(base.parse_threads),
        duck_threads: tuning.duck_threads.unwrap_or(base.duck_threads),
        memory_limit: tuning.memory_limit_mb.map(mb).unwrap_or(base.memory_limit),
        index: debrief_store_duck::WriterBudget {
            // Tantivy needs at least 15 MB per writer thread.
            heap: tuning.index_heap_mb.map_or(base.index.heap, |m| (m as usize) << 20),
            threads: tuning.index_threads.unwrap_or(base.index.threads),
        }
        .floored(),
        spill_dir: tuning.spill_dir.clone().or(base.spill_dir),
        max_spill: tuning.max_spill_mb.map(mb).or(base.max_spill),
        allocator_flush: tuning.allocator_flush_mb.map(|m| format!("{m}MiB")).unwrap_or(base.allocator_flush),
        allocator_background_threads: tuning.allocator_background_threads.unwrap_or(base.allocator_background_threads),
    }
}

/// The worker's `main`: reads its bootstrap, connects to Debrief and serves until Debrief closes the connection (or
/// stdin closes: Debrief is gone).
///
/// `mode` is how the worker uses memory (its command line, see [`MemoryMode::from_args`]) and `tuning` the knobs of
/// Settings ▸ Performance ([`StoreTuning::from_args`]).
pub fn run(plugins: Arc<dyn PluginSource>, mode: MemoryMode, tuning: StoreTuning) -> std::io::Result<()> {
    debrief_ipc::child::child_main("Debrief", move |_| {
        Arc::new(StoreHandler::new(Arc::new(
            Worker::with_mode(plugins.clone(), mode).with_tuning(tuning.clone()),
        )))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: Vec<u8>) -> bool {
        true
    }

    #[test]
    fn stores_take_the_options_of_the_workers_memory_mode() {
        let low = Worker::with_mode(Arc::new(NoPlugins), MemoryMode::Low);
        assert_eq!(low.options(None), StoreOptions::low_memory());
        assert_eq!(low.options(None).memory_limit, "128MB");
        assert_eq!(Worker::default().options(None), StoreOptions::default());
        // Limits a request names win; the rest stays the mode's.
        let merged = low.options(Some(Limits {
            parse_threads: 3,
            duck_threads: 1,
            memory_limit: "64MB".into(),
        }));
        assert_eq!(
            (merged.parse_threads, merged.duck_threads, merged.memory_limit.as_str()),
            (3, 1, "64MB")
        );
        assert_eq!(merged.index, StoreOptions::low_memory().index);
        let status: WorkerStatus = serde_json::from_value(low.handle(StoreRequest::Status, None, &mut none).unwrap()).unwrap();
        assert_eq!(status.memory_mode, MemoryMode::Low);
    }

    #[test]
    fn stores_take_the_workers_tuning_over_its_mode_and_requests_win_over_both() {
        let tuning = StoreTuning {
            parse_threads: Some(6),
            duck_threads: Some(3),
            memory_limit_mb: Some(512),
            index_heap_mb: Some(10),
            index_threads: Some(2),
            spill_dir: Some(std::env::temp_dir().join("spill")),
            max_spill_mb: Some(4096),
            allocator_flush_mb: Some(16),
            allocator_background_threads: Some(false),
            ..StoreTuning::default()
        };
        let w = Worker::with_mode(Arc::new(NoPlugins), MemoryMode::Low).with_tuning(tuning.clone());
        let o = w.options(None);
        assert_eq!((o.parse_threads, o.duck_threads, o.memory_limit.as_str()), (6, 3, "512MB"));
        // 10 MB is below what two writer threads need: raised to 15 MB each.
        assert_eq!((o.index.heap, o.index.threads), (30 << 20, 2));
        assert_eq!(o.spill_dir, tuning.spill_dir);
        assert_eq!(
            (o.max_spill.as_deref(), o.allocator_flush.as_str(), o.allocator_background_threads),
            (Some("4096MB"), "16MiB", false)
        );
        let req = w.options(Some(Limits {
            parse_threads: 1,
            duck_threads: 1,
            memory_limit: "64MB".into(),
        }));
        assert_eq!(
            (req.parse_threads, req.memory_limit.as_str(), req.max_spill.as_deref()),
            (1, "64MB", Some("4096MB"))
        );
        // Nothing tuned: the mode's options, unchanged.
        let partial = Worker::default().with_tuning(StoreTuning {
            duck_threads: Some(7),
            ..StoreTuning::default()
        });
        assert_eq!(
            partial.options(None),
            StoreOptions {
                duck_threads: 7,
                ..StoreOptions::default()
            }
        );
        let status: WorkerStatus = serde_json::from_value(w.handle(StoreRequest::Status, None, &mut none).unwrap()).unwrap();
        assert_eq!(status.tuning, tuning);
    }

    #[test]
    fn a_long_call_streams_its_changed_progress_and_a_panic_is_an_engine_error() {
        use debrief_core::ql::{QueryProgress, QueryStage};
        let control = QueryControl::new();
        let mut sent: Vec<QueryProgress> = Vec::new();
        let c = control.clone();
        let reply = with_progress(
            &control,
            &mut |bytes| {
                sent.push(debrief_ipc::wire::decode(&bytes).unwrap());
                true
            },
            move || {
                c.begin(QueryStage::Evaluating, 100);
                c.advance(40);
                std::thread::sleep(protocol::PROGRESS_EVERY * 3);
                Ok(7)
            },
        )
        .unwrap();
        assert_eq!(reply, 7);
        assert!(!sent.is_empty(), "progress reached the caller while the call ran");
        assert_eq!(
            sent[0],
            QueryProgress {
                stage: QueryStage::Evaluating,
                done: 40,
                total: 100
            }
        );
        assert!(sent.windows(2).all(|w| w[0] != w[1]), "only changes are sent");

        // The caller refusing an item (it is gone) stops the sending, not the call.
        let control = QueryControl::new();
        let c = control.clone();
        let mut refused = 0;
        let reply = with_progress(
            &control,
            &mut |_| {
                refused += 1;
                false
            },
            move || {
                c.advance(1);
                std::thread::sleep(protocol::PROGRESS_EVERY * 3);
                Ok(1)
            },
        );
        assert_eq!((reply.unwrap(), refused), (1, 1));

        // A panic inside the call is an engine error of that call; the worker lives on.
        let control = QueryControl::new();
        let err = with_progress::<()>(&control, &mut none, || panic!("query exploded")).unwrap_err();
        assert!(matches!(&err, StoreError::Engine(m) if m.contains("query exploded")), "{err}");
    }

    #[test]
    fn a_cancel_reaches_a_running_call() {
        let control = QueryControl::new();
        control.cancel();
        let c = control.clone();
        let reply = with_progress(&control, &mut none, move || {
            if c.is_cancelled() {
                Err(StoreError::Cancelled)
            } else {
                Ok(())
            }
        });
        assert_eq!(reply.unwrap_err(), StoreError::Cancelled);
    }

    #[test]
    fn calls_are_dispatched_and_failures_are_typed() {
        let w = Worker::default();
        let call = |store, op| StoreRequest::Call {
            store,
            op,
            body: debrief_ipc::Json(serde_json::Value::Null),
        };
        for op in StoreOp::ALL {
            let e = w.handle(call(9, *op), None, &mut none).unwrap_err();
            assert!(matches!(e, StoreError::NotFound(_)), "{op:?}: {e}");
        }
        let bad = StoreRequest::OpenStore(debrief_ipc::Json(serde_json::json!({"dir": 1})));
        assert!(matches!(w.handle(bad, None, &mut none).unwrap_err(), StoreError::Invalid(_)));
        let status: WorkerStatus = serde_json::from_value(w.handle(StoreRequest::Status, None, &mut none).unwrap()).unwrap();
        assert_eq!((status.pid, status.stores), (std::process::id(), 0));
    }
}
