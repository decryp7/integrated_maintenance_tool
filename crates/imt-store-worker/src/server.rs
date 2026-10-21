//! The worker side: the stores this process hosts (one per project folder; handles are per process), the
//! import sessions they run, and the handler that dispatches IMT's calls to them (ADR 0017 frames). Calls run on
//! blocking threads, so a long one (an import's wait, a big query) never holds up the others; a panic in a call
//! is an engine error for that call, not a crash of the worker.

use crate::protocol::{self, method, Handle, IdsAt, OpenStore, Paths, Session, Source, WorkerStatus};
use imt_core::api::{FacetRequest, TimelineRequest};
use imt_core::ports::{
    Diagnostics, ImportSession, Ingest, PageQuery, Positions, Profiles, Query, Search, SearchRequest, StoreError, StoreResult,
};
use imt_core::{PluginRegistry, Profile};
use imt_store_duck::{DuckStore, StoreOptions};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn body<T: DeserializeOwned>(bytes: &[u8]) -> StoreResult<T> {
    serde_json::from_slice(bytes).map_err(|e| StoreError::Invalid(format!("invalid request: {e}")))
}

fn reply<T: Serialize>(v: &T) -> StoreResult<Vec<u8>> {
    serde_json::to_vec(v).map_err(|e| StoreError::Engine(format!("cannot encode the reply: {e}")))
}

/// The stores of this worker.
#[derive(Default)]
pub struct Worker {
    stores: Mutex<HashMap<u64, (PathBuf, DuckStore)>>,
    sessions: Mutex<HashMap<u64, ImportSession>>,
    next: AtomicU64,
}

impl Worker {
    fn store(&self, handle: u64) -> StoreResult<DuckStore> {
        lock(&self.stores)
            .get(&handle)
            .map(|(_, s)| s.clone())
            .ok_or_else(|| StoreError::NotFound(format!("store {handle} is not open in this worker")))
    }

    fn open_store(&self, req: OpenStore) -> StoreResult<Handle> {
        let dir = if req.create {
            std::fs::create_dir_all(&req.dir)?;
            req.dir.clone()
        } else {
            req.dir.clone()
        };
        let dir = std::fs::canonicalize(&dir).map_err(|e| StoreError::Io(format!("{}: {e}", req.dir.display())))?;
        // A folder is open once per worker: a second open gets the same store.
        if let Some((h, _)) = lock(&self.stores).iter().find(|(_, (d, _))| *d == dir) {
            if !req.create {
                return Ok(Handle { store: *h });
            }
        }
        let opts = match req.limits {
            Some(l) => StoreOptions {
                parse_threads: l.parse_threads,
                duck_threads: l.duck_threads,
                memory_limit: l.memory_limit,
            },
            None => StoreOptions::default(),
        };
        let registry = PluginRegistry::new();
        let store = if req.create {
            DuckStore::create_with(&dir, req.profile, registry, opts)?
        } else {
            DuckStore::open_with(&dir, req.profile, registry, opts)?
        };
        let h = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        lock(&self.stores).insert(h, (dir, store));
        Ok(Handle { store: h })
    }

    fn session(&self, s: ImportSession) -> Session {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        lock(&self.sessions).insert(id, s);
        Session { session: id }
    }

    /// Handles one call; a panic is an engine error of the call.
    pub fn call(&self, method: &str, handle: u64, bytes: &[u8]) -> StoreResult<Vec<u8>> {
        catch_unwind(AssertUnwindSafe(|| self.dispatch(method, handle, bytes))).unwrap_or_else(|p| {
            Err(StoreError::Engine(format!(
                "the store worker hit an internal error in {method}: {}",
                imt_core::guard::panic_message(p.as_ref())
            )))
        })
    }

    fn dispatch(&self, m: &str, h: u64, b: &[u8]) -> StoreResult<Vec<u8>> {
        match m {
            method::OPEN_STORE => reply(&self.open_store(body(b)?)?),
            method::CLOSE_STORE => {
                lock(&self.stores).remove(&h);
                reply(&())
            }
            method::STATUS => reply(&WorkerStatus {
                pid: std::process::id(),
                rss_bytes: imt_ipc::process::current_rss_bytes(),
                stores: lock(&self.stores).len(),
            }),
            method::OPEN => {
                let p: Paths = body(b)?;
                let s = self.store(h)?.open(&p.paths)?;
                reply(&self.session(s))
            }
            method::RELOAD => {
                let s = self.store(h)?.reload()?;
                reply(&self.session(s))
            }
            method::REFRESH => {
                let p: Paths = body(b)?;
                let s = self.store(h)?.refresh(&p.paths)?;
                reply(&self.session(s))
            }
            method::REMOVE_SOURCE => reply(&self.store(h)?.remove_source(body::<Source>(b)?.id)?),
            method::CLEAR => reply(&self.store(h)?.clear()?),
            method::APPEND_TAIL => reply(&self.store(h)?.append_tail(&body::<Paths>(b)?.paths)?),
            // The store's progress is that of its running (or last) import.
            method::SESSION_PROGRESS => reply(&self.store(h)?.progress().snapshot()),
            method::SESSION_CANCEL => {
                self.store(h)?.progress().cancel();
                reply(&())
            }
            method::SESSION_WAIT => {
                let s: Session = body(b)?;
                let session = lock(&self.sessions)
                    .remove(&s.session)
                    .ok_or_else(|| StoreError::NotFound(format!("import {} (already waited for?)", s.session)))?;
                reply(&session.wait()?)
            }
            method::SUMMARY => reply(&self.store(h)?.summary()?),
            method::PAGE => reply(&self.store(h)?.page(&body::<PageQuery>(b)?)?),
            method::TIMELINE => reply(&self.store(h)?.timeline(&body::<TimelineRequest>(b)?)?),
            method::FACET => reply(&self.store(h)?.facet(&body::<FacetRequest>(b)?)?),
            method::IDS_AT => {
                let r: IdsAt = body(b)?;
                reply(&self.store(h)?.ids_at(r.from, r.len)?)
            }
            method::POSITION_OF => reply(&self.store(h)?.position_of(body(b)?)?),
            method::POSITIONED => reply(&self.store(h)?.positioned()?),
            method::ACTIVE_PROFILE => reply(&self.store(h)?.active_profile()?),
            method::VALIDATE_PROFILE => reply(&self.store(h)?.validate_profile(&body::<Profile>(b)?)?),
            method::PROBLEMS => reply(&self.store(h)?.problems()?),
            method::SEARCH => reply(&self.store(h)?.search(&body::<SearchRequest>(b)?)?),
            method::INDEX_STATUS => reply(&self.store(h)?.index_status()?),
            other => Err(StoreError::Invalid(format!("unknown method {other}"))),
        }
    }
}

/// Answers IMT's [`protocol::StoreRequest`]s.
pub struct StoreHandler {
    worker: Arc<Worker>,
}

impl StoreHandler {
    pub fn new(worker: Arc<Worker>) -> StoreHandler {
        StoreHandler { worker }
    }

    fn handle(&self, req: protocol::StoreRequest) -> StoreResult<Vec<u8>> {
        let json = |v: imt_ipc::Json| serde_json::to_vec(&v.0).map_err(|e| StoreError::Invalid(e.to_string()));
        match req {
            protocol::StoreRequest::OpenStore(body) => self.worker.call(method::OPEN_STORE, 0, &json(body)?),
            protocol::StoreRequest::CloseStore { store } => self.worker.call(method::CLOSE_STORE, store, b""),
            protocol::StoreRequest::Status => self.worker.call(method::STATUS, 0, b""),
            protocol::StoreRequest::Call { store, op, body } => self.worker.call(op.method(), store, &json(body)?),
        }
    }
}

impl imt_ipc::Handler for StoreHandler {
    fn call(&self, _ctx: &imt_ipc::CallCtx, body: &[u8]) -> Result<Vec<u8>, imt_ipc::RemoteError> {
        let req: protocol::StoreRequest = imt_ipc::peer::decode_request(body)?;
        let reply = self.handle(req).map_err(|e| protocol::to_remote(&e))?;
        let value: serde_json::Value = serde_json::from_slice(&reply)
            .map_err(|e| protocol::to_remote(&StoreError::Engine(format!("cannot encode the reply: {e}"))))?;
        imt_ipc::peer::encode_reply(&imt_ipc::Json(value))
    }
}

/// The worker's `main`: reads its bootstrap, connects to IMT and serves until IMT closes the connection (or
/// stdin closes: IMT is gone).
pub fn run() -> std::io::Result<()> {
    imt_ipc::child::child_main("IMT", |_| Arc::new(StoreHandler::new(Arc::new(Worker::default()))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_are_dispatched_and_failures_are_typed() {
        let w = Worker::default();
        let e = w.call("nope", 0, b"").unwrap_err();
        assert!(matches!(e, StoreError::Invalid(_)), "{e}");
        assert!(matches!(w.call(method::SUMMARY, 9, b"").unwrap_err(), StoreError::NotFound(_)));
        assert!(matches!(w.call(method::OPEN_STORE, 0, b"{").unwrap_err(), StoreError::Invalid(_)));
        let status: WorkerStatus = serde_json::from_slice(&w.call(method::STATUS, 0, b"").unwrap()).unwrap();
        assert_eq!((status.pid, status.stores), (std::process::id(), 0));
    }
}
