//! One plugin's processes as the host sees them: the supervised child that serves the plugin (started on first
//! use, restarted with backoff, quarantined after repeated failures, its settings restored after a restart), the
//! elevated child for operations that need administrator rights (started through the administrator prompt on
//! first use), and the host services the plugin calls back (ADR 0017 §4).
//!
//! Call-scoped data (the text of a file being imported, an annotator's view) stays on the calling thread: a call
//! is a streamed request whose callbacks are *pumped* to the caller, which serves them with its borrowed data
//! between the items of the answer. No copy of a file is made and nothing borrowed crosses threads.

use crate::sandbox::sandbox_for;
use imt_core::plugin::annotate::{AnnotationView, ViewFilter};
use imt_core::plugin::tools::ToolCtx;
use imt_ipc::child_log::{ChildKind, ChildLogSpec};
use imt_ipc::elevation::Elevation;
use imt_ipc::launch::{launch, Child, ChildSpec};
use imt_ipc::peer::{code, decode_request, encode_reply, CallCtx, Handler, StreamEvent};
use imt_ipc::process::SpawnSpec;
use imt_ipc::supervisor::{ChildStatus, Supervisor, SupervisorConfig};
use imt_ipc::{IpcError, Json, RemoteError};
use imt_plugin_api::manifest::{Manifest, Permission};
use imt_plugin_api::protocol::{HostFileRef, HostRequest, PluginRequest, WireEvent, WireViewRecord, READ_CHUNK};
use imt_plugin_api::serve::ELEVATED_ROLE;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// How long a pumped callback may wait for the calling thread.
const PUMP_WAIT: Duration = Duration::from_secs(120);
/// How often the calling thread looks for callbacks while it waits for items.
const PUMP_POLL: Duration = Duration::from_millis(5);
/// Items a streamed call may have in flight.
const CREDIT: u32 = 4;

/// How to start a plugin's process.
#[derive(Debug, Clone)]
pub struct PluginSpec {
    /// The program serving the plugin (`imt-plugins` for the built-ins).
    pub exe: PathBuf,
    /// Its arguments (`--plugin <id>`).
    pub args: Vec<String>,
    /// Permissions the user granted (the built-ins': what they ask for).
    pub granted: Vec<Permission>,
    /// How long it may take to connect.
    pub connect_window: Duration,
    /// Restart policy (backoff, quarantine).
    pub supervisor: SupervisorConfig,
    /// Where its standard error goes.
    pub log: Option<ChildLogSpec>,
}

impl PluginSpec {
    pub fn new(exe: impl Into<PathBuf>, id: &str) -> PluginSpec {
        let log = ChildLogSpec::new(format!("plugin-{id}"), ChildKind::Plugin);
        PluginSpec {
            exe: exe.into(),
            args: vec!["--plugin".into(), id.into()],
            granted: Vec::new(),
            connect_window: Duration::from_secs(15),
            supervisor: SupervisorConfig {
                log: Some(log.clone()),
                ..SupervisorConfig::new(format!("plugin \"{id}\""))
            },
            log: Some(log),
        }
    }
}

/// A callback the calling thread serves.
struct Pumped {
    req: HostRequest,
    reply: mpsc::Sender<Result<Vec<u8>, RemoteError>>,
}

/// What the plugin may call back: call-scoped requests go to the pump of their call; task reports go to the
/// context of the tool's last call.
#[derive(Default)]
pub(crate) struct HostServices {
    /// File ids and view tickets → the pump of the call that registered them.
    pumps: Mutex<HashMap<u64, mpsc::Sender<Pumped>>>,
    next: AtomicU64,
    /// Tool id → the context of its last call (task reports outlive the call that started the task).
    tasks: Mutex<HashMap<String, ToolCtx>>,
}

impl HostServices {
    fn ticket(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub(crate) fn remember_tool(&self, tool: &str, ctx: &ToolCtx) {
        lock(&self.tasks).insert(tool.to_string(), ctx.clone());
    }

    fn pump(&self, id: u64, req: HostRequest) -> Result<Vec<u8>, RemoteError> {
        let Some(tx) = lock(&self.pumps).get(&id).cloned() else {
            return Err(RemoteError::new(code::INVALID, format!("no call is waiting for {id}")));
        };
        let (reply, rx) = mpsc::channel();
        tx.send(Pumped { req, reply })
            .map_err(|_| RemoteError::new(code::CANCELLED, "the call ended"))?;
        rx.recv_timeout(PUMP_WAIT)
            .map_err(|_| RemoteError::new(code::CANCELLED, "the call ended"))?
    }
}

impl Handler for HostServices {
    fn call(&self, _ctx: &CallCtx, body: &[u8]) -> Result<Vec<u8>, RemoteError> {
        let req: HostRequest = decode_request(body)?;
        match req {
            HostRequest::ReadFile { id, .. } => self.pump(id, req),
            HostRequest::ViewSelect { ticket, .. } | HostRequest::ViewInstances { ticket } | HostRequest::ViewTimeRange { ticket } => {
                self.pump(ticket, req)
            }
            HostRequest::TaskStarted { tool, task } => {
                if let Some(ctx) = lock(&self.tasks).get(&tool) {
                    ctx.sink().started(&tool, &task);
                }
                encode_reply(&())
            }
            HostRequest::TaskProgress { tool, task, fraction } => {
                if let Some(ctx) = lock(&self.tasks).get(&tool) {
                    ctx.sink().progress(&tool, &task, fraction);
                }
                encode_reply(&())
            }
            HostRequest::TaskFinished { tool, task } => {
                if let Some(ctx) = lock(&self.tasks).get(&tool) {
                    ctx.sink().finished(&tool, &task);
                }
                encode_reply(&())
            }
        }
    }
}

/// A file the host reads by ranges for the plugin (a large dump), instead of holding it in memory.
pub trait RangeSource: Send + Sync {
    fn len(&self) -> u64;
    /// Up to `len` bytes at `offset` (fewer at the end).
    fn read_at(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>>;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The data one call lends the plugin: files (read with `ReadFile`): bytes the host holds, then files it reads
/// by ranges; and an annotator's view.
#[derive(Default)]
pub(crate) struct Lent<'a> {
    pub files: Vec<&'a [u8]>,
    pub ranges: Vec<&'a dyn RangeSource>,
    pub view: Option<&'a dyn AnnotationView>,
}

/// Ids registered for one call; removed when the call ends.
struct Registration<'s> {
    services: &'s HostServices,
    ids: Vec<u64>,
}

impl Drop for Registration<'_> {
    fn drop(&mut self) {
        let mut p = lock(&self.services.pumps);
        for id in &self.ids {
            p.remove(id);
        }
    }
}

/// Serves one pumped callback with the call's lent data.
fn serve_pumped(p: &Pumped, lent: &Lent, file_ids: &[u64]) -> Result<Vec<u8>, RemoteError> {
    match &p.req {
        HostRequest::ReadFile { id, offset, len } => {
            let i = file_ids
                .iter()
                .position(|f| f == id)
                .ok_or_else(|| RemoteError::new(code::INVALID, "unknown file"))?;
            let want = (*len).min(READ_CHUNK) as usize;
            if let Some(data) = lent.files.get(i) {
                let start = usize::try_from(*offset).unwrap_or(usize::MAX).min(data.len());
                let end = start.saturating_add(want).min(data.len());
                return encode_reply(&data[start..end].to_vec());
            }
            let source = lent
                .ranges
                .get(i - lent.files.len())
                .ok_or_else(|| RemoteError::new(code::INVALID, "unknown file"))?;
            let mut bytes = source
                .read_at(*offset, want)
                .map_err(|e| RemoteError::new(code::INTERNAL, format!("cannot read the file: {e}")))?;
            bytes.truncate(want);
            encode_reply(&bytes)
        }
        HostRequest::ViewSelect { filter, limit, .. } => {
            let view = lent.view.ok_or_else(|| RemoteError::new(code::INVALID, "no view for this call"))?;
            let rows: Result<Vec<WireViewRecord>, String> = serde_json::from_value::<ViewFilter>(filter.0.clone())
                .map_err(|e| format!("invalid filter: {e}"))
                .and_then(|f| view.select(&f, *limit as usize).map_err(|e| format!("{e:#}")))
                .map(|rows| {
                    rows.into_iter()
                        .map(|r| WireViewRecord {
                            event: WireEvent::of(&r.record),
                            source: r.source,
                            instance: r.instance,
                        })
                        .collect()
                });
            encode_reply(&rows)
        }
        HostRequest::ViewInstances { .. } => encode_reply(&lent.view.map(|v| v.instances()).unwrap_or_default()),
        HostRequest::ViewTimeRange { .. } => encode_reply(&lent.view.and_then(|v| v.time_range())),
        _ => Err(RemoteError::new(code::INVALID, "not a call-scoped request")),
    }
}

/// A plugin's processes.
pub struct PluginProcess {
    manifest: Manifest,
    sup: Supervisor<Child>,
    elevated: Mutex<Option<Arc<Supervisor<Child>>>>,
    spec: PluginSpec,
    services: Arc<HostServices>,
    /// The settings last applied, restored when the process restarts.
    settings: Arc<Mutex<Option<serde_json::Value>>>,
}

impl PluginProcess {
    /// The process of the plugin `manifest` describes; started on first use.
    pub fn new(manifest: Manifest, spec: PluginSpec) -> Arc<PluginProcess> {
        let services = Arc::new(HostServices::default());
        let settings: Arc<Mutex<Option<serde_json::Value>>> = Arc::default();
        let child_spec = child_spec(&manifest, &spec, false);
        let handler: Arc<dyn Handler> = services.clone();
        let restore_settings = settings.clone();
        let sup = Supervisor::new(spec.supervisor.clone(), move || launch(&child_spec, handler.clone())).with_restore(
            move |c: &Child, restarted| {
                let values = lock(&restore_settings).clone();
                match (restarted, values) {
                    (true, Some(v)) => c
                        .peer()
                        .request::<_, ()>(&PluginRequest::ApplySettings { values: Json(v) }, Some(Duration::from_secs(10))),
                    _ => Ok(()),
                }
            },
        );
        Arc::new(PluginProcess {
            manifest,
            sup,
            elevated: Mutex::new(None),
            spec,
            services,
            settings,
        })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    /// The process' state (not started, running, quarantined).
    pub fn status(&self) -> ChildStatus {
        self.sup.status()
    }

    /// Whether the plugin was granted `p`.
    pub fn granted(&self, p: Permission) -> bool {
        self.spec.granted.contains(&p) && self.manifest.asks_for(p)
    }

    pub(crate) fn services(&self) -> &HostServices {
        &self.services
    }

    /// Starts the process now (in the background at app start), so the first call does not wait for it.
    pub fn warm_up(&self) {
        let _ = self.sup.lease();
    }

    /// Lifts a quarantine of its processes.
    pub fn reset(&self) {
        self.sup.reset();
        if let Some(e) = lock(&self.elevated).as_ref() {
            e.reset();
        }
    }

    /// Stops its processes (they start again on next use).
    pub fn shutdown(&self) {
        self.sup.shutdown();
        if let Some(e) = lock(&self.elevated).as_ref() {
            e.shutdown();
        }
    }

    pub(crate) fn remember_settings(&self, values: serde_json::Value) {
        *lock(&self.settings) = Some(values);
    }

    fn elevated_sup(&self) -> Arc<Supervisor<Child>> {
        lock(&self.elevated)
            .get_or_insert_with(|| {
                let spec = child_spec(&self.manifest, &self.spec, true);
                let handler: Arc<dyn Handler> = self.services.clone();
                let mut cfg = self.spec.supervisor.clone();
                cfg.label = format!("{} (elevated)", cfg.label);
                Arc::new(Supervisor::new(cfg, move || launch(&spec, handler.clone())))
            })
            .clone()
    }

    /// One call (no callbacks), on the elevated process when `elevated`.
    pub fn call<Rep: serde::de::DeserializeOwned>(
        &self,
        req: &PluginRequest,
        timeout: Option<Duration>,
        elevated: bool,
    ) -> Result<Rep, IpcError> {
        let run = |sup: &Supervisor<Child>| sup.call(|c| c.peer().request(req, timeout));
        if elevated {
            run(&self.elevated_sup())
        } else {
            run(&self.sup)
        }
    }

    /// A streamed call whose callbacks are served on this thread with `lent`; `on_item` receives each item.
    /// Returns the answer.
    pub(crate) fn call_lending<Rep: serde::de::DeserializeOwned>(
        &self,
        make: impl FnOnce(&[HostFileRef], u64) -> PluginRequest,
        lent: &Lent,
        timeout: Option<Duration>,
        mut on_item: impl FnMut(Vec<u8>) -> Result<(), IpcError>,
    ) -> Result<Rep, IpcError> {
        let (tx, rx) = mpsc::channel::<Pumped>();
        let mut reg = Registration {
            services: &self.services,
            ids: Vec::new(),
        };
        let lens = lent.files.iter().map(|f| f.len() as u64).chain(lent.ranges.iter().map(|r| r.len()));
        let files: Vec<HostFileRef> = lens
            .map(|len| {
                let id = self.services.ticket();
                reg.ids.push(id);
                HostFileRef { id, len }
            })
            .collect();
        let ticket = self.services.ticket();
        reg.ids.push(ticket);
        {
            let mut p = lock(&self.services.pumps);
            for id in &reg.ids {
                p.insert(*id, tx.clone());
            }
        }
        drop(tx);
        let file_ids: Vec<u64> = files.iter().map(|f| f.id).collect();
        let req = make(&files, ticket);
        let deadline = timeout.map(|t| std::time::Instant::now() + t);
        self.sup.call(|c| {
            let mut stream = c.peer().stream_request(&req, CREDIT)?;
            loop {
                while let Ok(p) = rx.try_recv() {
                    let _ = p.reply.send(serve_pumped(&p, lent, &file_ids));
                }
                if deadline.is_some_and(|d| std::time::Instant::now() > d) {
                    // Overran its budget: a hang unless it ends the call at once.
                    stream.cancel_and_wait()?;
                    return Err(IpcError::Timeout);
                }
                match stream.next(Some(PUMP_POLL))? {
                    StreamEvent::Idle => {}
                    StreamEvent::Item(b) => on_item(b)?,
                    StreamEvent::Done(b) => {
                        return imt_ipc::wire::decode(&b).map_err(|e| IpcError::Protocol(format!("{}: {e}", c.peer().label())));
                    }
                }
            }
        })
    }
}

/// The child spec of a plugin's normal or elevated process.
fn child_spec(manifest: &Manifest, spec: &PluginSpec, elevated: bool) -> ChildSpec {
    let label = if elevated {
        format!("plugin \"{}\" (elevated)", manifest.id)
    } else {
        format!("plugin \"{}\"", manifest.id)
    };
    let mut s = SpawnSpec::new(spec.exe.clone(), label);
    s.args = spec.args.clone();
    s.current_dir = Some(std::env::temp_dir());
    s.endpoint_prefix = "imt-plugin".into();
    s.connect_window = spec.connect_window;
    s.log = if elevated { None } else { spec.log.clone() };
    s.sandbox = sandbox_for(manifest, &spec.granted, elevated);
    let mut c = ChildSpec::new(s);
    if elevated {
        c.role = ELEVATED_ROLE.into();
        c.elevation = Elevation::Required {
            reason: format!("{} needs administrator rights for this", manifest.name),
        };
    }
    c
}
