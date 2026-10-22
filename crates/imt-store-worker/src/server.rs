//! The worker side: the stores this process hosts (one per project folder; handles are per process), the
//! import sessions they run, and the handler that dispatches IMT's calls to them (ADR 0017 frames). Calls run on
//! blocking threads, so a long one (an import's wait, a big query) never holds up the others; a panic in a call
//! is an engine error for that call, not a crash of the worker. A cancelled call interrupts the statement it
//! runs (the caller's `Cancel` frame reaches [`DuckStore::with_control`]).

use imt_core::api::WorkspaceState;
use imt_core::package::ProjectManifest;
use imt_core::ports::{
    Analysis, Diagnostics, Filter, ImportSession, Ingest, Positions, Profiles, Projects, Query, Search, StoreError, StoreResult,
};
use imt_core::ql::QueryControl;
use imt_core::{PluginRegistry, Profile};
use imt_ipc::CancelProbe;
use imt_store_client::protocol::{
    self, ExportProject, Handle, IdsAt, OpenProject, OpenStore, OpenedProject, Paths, Session, Source, StoreOp, StoreRequest, WorkerStatus,
};
use imt_store_duck::{DuckStore, StoreOptions};
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

/// Builds the plugin registry of a store: the importers of every plugin except those the user turned off
/// (the first argument); none in safe mode (the second).
pub type RegistryFactory = Arc<dyn Fn(&[String], bool) -> PluginRegistry + Send + Sync>;

/// A registry with `plugins` installed except the `disabled` ones (the same rules as IMT's own registry: a
/// plugin that cannot be installed is logged and left out).
pub fn install_plugins(plugins: Vec<Arc<dyn imt_core::Plugin>>, disabled: &[String]) -> PluginRegistry {
    let mut registry = PluginRegistry::new();
    registry.disable(disabled.iter().cloned());
    for plugin in plugins {
        let id = plugin.id().to_string();
        if let Err(e) = registry.install_shared(plugin) {
            log::error!("plugin {id} not installed in the store worker: {e:#}");
        }
    }
    registry.resolve();
    registry
}

/// The stores of this worker.
pub struct Worker {
    stores: Mutex<HashMap<u64, (PathBuf, DuckStore)>>,
    sessions: Mutex<HashMap<u64, ImportSession>>,
    next: AtomicU64,
    registry: RegistryFactory,
}

impl Default for Worker {
    fn default() -> Self {
        Worker::new(Arc::new(|_, _| PluginRegistry::new()))
    }
}

impl Worker {
    pub fn new(registry: RegistryFactory) -> Worker {
        Worker {
            stores: Mutex::default(),
            sessions: Mutex::default(),
            next: AtomicU64::new(0),
            registry,
        }
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
            return Ok(Handle { store: *h });
        }
        let opts = match req.limits {
            Some(l) => StoreOptions {
                parse_threads: l.parse_threads,
                duck_threads: l.duck_threads,
                memory_limit: l.memory_limit,
            },
            None => StoreOptions::default(),
        };
        let registry = (self.registry)(&req.disabled_plugins, req.safe_mode);
        let store = if req.create {
            DuckStore::create_with(&dir, req.profile, registry, opts)?
        } else {
            DuckStore::open_with(&dir, req.profile, registry, opts)?
        };
        if let Some(setup) = &req.parsers {
            imt_core::ports::Profiles::set_parsers(&store, setup)?;
        }
        let h = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        lock(&self.stores).insert(h, (dir, store));
        Ok(Handle { store: h })
    }

    fn open_project(&self, req: OpenProject) -> StoreResult<OpenedProject> {
        std::fs::create_dir_all(&req.dest_root)?;
        let registry = (self.registry)(&req.disabled_plugins, req.safe_mode);
        let (store, manifest) = DuckStore::open_project(&req.path, &req.dest_root, req.profile, registry)?;
        if let Some(setup) = &req.parsers {
            imt_core::ports::Profiles::set_parsers(&store, setup)?;
        }
        if let Some(l) = req.limits {
            store.configure(StoreOptions {
                parse_threads: l.parse_threads,
                duck_threads: l.duck_threads,
                memory_limit: l.memory_limit,
            })?;
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
                imt_core::guard::panic_message(p.as_ref())
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
            StoreRequest::CloseStore { store } => {
                lock(&self.stores).remove(&store);
                reply(&())
            }
            StoreRequest::Status => reply(&WorkerStatus {
                pid: std::process::id(),
                rss_bytes: imt_ipc::process::current_rss_bytes(),
                stores: lock(&self.stores).len(),
            }),
            StoreRequest::Scan { store, filter } => {
                let filter: Filter = body(filter.0)?;
                let s = self.store(store)?;
                let mut failed: Option<StoreError> = None;
                controlled(probe, || {
                    s.scan(
                        &filter,
                        &mut |batch| match imt_store_client::arrow_wire::encode(batch, protocol::MAX_MESSAGE_BYTES - 4096) {
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
            StoreRequest::Call { store, op, body: b } => self.call(store, op, b.0, probe),
        }
    }

    fn call(&self, h: u64, op: StoreOp, b: serde_json::Value, probe: Option<CancelProbe>) -> StoreResult<serde_json::Value> {
        let s = self.store(h)?;
        // A cancellable operation: the caller's cancel interrupts its statements.
        macro_rules! run {
            ($e:expr) => {
                reply(&controlled(probe, || $e)?)
            };
        }
        match op {
            StoreOp::Open => {
                let p: Paths = body(b)?;
                let session = s.open(&p.paths)?;
                reply(&self.session(session))
            }
            StoreOp::Reload => {
                let session = s.reload()?;
                reply(&self.session(session))
            }
            StoreOp::Refresh => {
                let p: Paths = body(b)?;
                let session = s.refresh(&p.paths)?;
                reply(&self.session(session))
            }
            StoreOp::RemoveSource => reply(&s.remove_source(body::<Source>(b)?.id)?),
            StoreOp::Clear => reply(&s.clear()?),
            StoreOp::Restore => {
                let state: WorkspaceState = body(b)?;
                let session = s.restore(&state)?;
                reply(&self.session(session))
            }
            StoreOp::AppendTail => reply(&s.append_tail(&body::<Paths>(b)?.paths)?),
            // The store's progress is that of its running (or last) import.
            StoreOp::SessionProgress => reply(&s.progress().snapshot()),
            StoreOp::SessionCancel => {
                s.progress().cancel();
                reply(&())
            }
            StoreOp::SessionWait => {
                let id: Session = body(b)?;
                let session = lock(&self.sessions)
                    .remove(&id.session)
                    .ok_or_else(|| StoreError::NotFound(format!("import {} (already waited for?)", id.session)))?;
                reply(&session.wait()?)
            }
            StoreOp::Summary => reply(&s.summary()?),
            StoreOp::Page => reply(&s.page(&body(b)?)?),
            StoreOp::Events => reply(&s.events(&body(b)?)?),
            StoreOp::Timeline => reply(&s.timeline(&body(b)?)?),
            StoreOp::Facet => reply(&s.facet(&body(b)?)?),
            StoreOp::Chart => run!(s.chart(&body(b)?)),
            StoreOp::FieldStats => run!(s.field_stats(&body(b)?)),
            StoreOp::SourceContext => reply(&s.source_context(&body(b)?)?),
            StoreOp::RunQuery => {
                let req = body(b)?;
                let control = QueryControl::new();
                let _watch = probe.map(|p| Watch::start(p, control.clone()));
                reply(&s.run_query_with(&req, &control)?)
            }
            StoreOp::CheckQuery => reply(&s.check_query(&body::<String>(b)?)?),
            StoreOp::CompleteQuery => reply(&s.complete_query(&body(b)?)?),
            StoreOp::Export => run!(s.export(&body(b)?)),
            StoreOp::IdsAt => {
                let r: IdsAt = body(b)?;
                reply(&s.ids_at(r.from, r.len)?)
            }
            StoreOp::PositionOf => reply(&s.position_of(body(b)?)?),
            StoreOp::Positioned => reply(&s.positioned()?),
            StoreOp::Patterns => run!(s.patterns(&body(b)?)),
            StoreOp::ExceptionGroups => run!(s.exception_groups(&body(b)?)),
            StoreOp::Anomalies => run!(s.anomalies(&body(b)?)),
            StoreOp::Compare => run!(s.compare(&body(b)?)),
            StoreOp::Correlate => run!(s.correlate(&body(b)?)),
            StoreOp::Traces => run!(s.traces(&body(b)?)),
            StoreOp::Requests => run!(s.requests(&body(b)?)),
            StoreOp::Moment => run!(s.moment(&body(b)?)),
            StoreOp::Overview => run!(s.overview(&body(b)?)),
            StoreOp::Runs => run!(s.runs(&body(b)?)),
            StoreOp::ActiveProfile => reply(&s.active_profile()?),
            StoreOp::ValidateProfile => reply(&s.validate_profile(&body::<Profile>(b)?)?),
            StoreOp::SetProfile => {
                let session = s.set_profile(body::<Profile>(b)?)?;
                reply(&self.session(session))
            }
            StoreOp::PreviewParse => run!(s.preview_parse(&body(b)?)),
            StoreOp::PreviewExtractor => run!(s.preview_extractor(&body(b)?)),
            StoreOp::ProposeSpanRule => run!(s.propose_span_rule(&body(b)?)),
            StoreOp::PreviewSpans => run!(s.preview_spans(&body(b)?)),
            StoreOp::Parsers => reply(&s.parsers()?),
            StoreOp::ClassifyParser => reply(&s.classify_parser(&body(b)?)?),
            StoreOp::AssignParser => {
                let r: protocol::AssignParser = body(b)?;
                let session = s.assign_parser(r.source, r.parser.as_deref())?;
                reply(&self.session(session))
            }
            StoreOp::EvaluateSpanRule => run!(s.evaluate_span_rule(&body(b)?)),
            StoreOp::Problems => reply(&s.problems()?),
            StoreOp::EvaluateFix => run!(s.evaluate_fix(&body(b)?)),
            StoreOp::Search => reply(&s.search(&body(b)?)?),
            StoreOp::IndexStatus => reply(&s.index_status()?),
            StoreOp::PartialMatches => run!(s.partial_matches(&body(b)?)),
            StoreOp::SetParsers => reply(&s.set_parsers(&body(b)?)?),
            StoreOp::RereadParsers => {
                let session = s.reread_parsers()?;
                reply(&self.session(session))
            }
            StoreOp::SaveProject => reply(&Projects::save_project(&s, &body::<ProjectManifest>(b)?)?),
            StoreOp::ExportProject => {
                let r: ExportProject = body(b)?;
                reply(&Projects::export_project(&s, &r.manifest, &r.out)?)
            }
        }
    }
}

/// Runs `f` under a control that the caller's cancel triggers (when there is a probe).
fn controlled<R>(probe: Option<CancelProbe>, f: impl FnOnce() -> StoreResult<R>) -> StoreResult<R> {
    match probe {
        None => f(),
        Some(p) => {
            let control = QueryControl::new();
            let _watch = Watch::start(p, control.clone());
            DuckStore::with_control(&control, f)
        }
    }
}

/// Cancels a control when the caller cancels the call; stops when dropped.
struct Watch {
    done: Arc<AtomicBool>,
}

impl Watch {
    fn start(probe: CancelProbe, control: QueryControl) -> Watch {
        let done = Arc::new(AtomicBool::new(false));
        let d = done.clone();
        let spawned = std::thread::Builder::new().name("imt-store-cancel".into()).spawn(move || {
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

/// Answers IMT's [`StoreRequest`]s.
pub struct StoreHandler {
    worker: Arc<Worker>,
}

impl StoreHandler {
    pub fn new(worker: Arc<Worker>) -> StoreHandler {
        StoreHandler { worker }
    }
}

impl imt_ipc::Handler for StoreHandler {
    fn call(&self, ctx: &imt_ipc::CallCtx, body: &[u8]) -> Result<Vec<u8>, imt_ipc::RemoteError> {
        let req: StoreRequest = imt_ipc::peer::decode_request(body)?;
        let probe = ctx.cancel_probe();
        let mut send = |item: Vec<u8>| ctx.send_item(item).is_ok();
        let value = match self.worker.handle(req, Some(probe.clone()), &mut send) {
            Ok(v) => v,
            Err(_) if probe.is_cancelled() => return Err(imt_ipc::CallCtx::cancelled_error()),
            Err(e) => return Err(protocol::to_remote(&e)),
        };
        imt_ipc::peer::encode_reply(&imt_ipc::Json(value))
    }
}

/// The worker's `main`: reads its bootstrap, connects to IMT and serves until IMT closes the connection (or
/// stdin closes: IMT is gone).
pub fn run(registry: RegistryFactory) -> std::io::Result<()> {
    imt_ipc::child::child_main("IMT", move |_| Arc::new(StoreHandler::new(Arc::new(Worker::new(registry.clone())))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: Vec<u8>) -> bool {
        true
    }

    #[test]
    fn calls_are_dispatched_and_failures_are_typed() {
        let w = Worker::default();
        let call = |store, op| StoreRequest::Call {
            store,
            op,
            body: imt_ipc::Json(serde_json::Value::Null),
        };
        for op in StoreOp::ALL {
            let e = w.handle(call(9, *op), None, &mut none).unwrap_err();
            assert!(matches!(e, StoreError::NotFound(_)), "{op:?}: {e}");
        }
        let bad = StoreRequest::OpenStore(imt_ipc::Json(serde_json::json!({"dir": 1})));
        assert!(matches!(w.handle(bad, None, &mut none).unwrap_err(), StoreError::Invalid(_)));
        let status: WorkerStatus = serde_json::from_value(w.handle(StoreRequest::Status, None, &mut none).unwrap()).unwrap();
        assert_eq!((status.pid, status.stores), (std::process::id(), 0));
    }
}
