//! One plugin's processes as the host sees them: the supervised child that serves the plugin (started on first
//! use, restarted with backoff, quarantined after repeated failures, its settings restored after a restart), the
//! elevated child for operations that need administrator rights (started through the administrator prompt on
//! first use), and the host services the plugin calls back (ADR 0017 §4).
//!
//! Call-scoped data (the text of a file being imported, an annotator's view) stays on the calling thread: a call
//! is a streamed request whose callbacks are *pumped* to the caller, which serves them with its borrowed data
//! between the items of the answer. No copy of a file is made and nothing borrowed crosses threads.

use crate::sandbox::{sandbox_for, sandbox_for_third_party, PluginHome};
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
use imt_plugin_api::protocol::{HostFileRef, HostRequest, PluginRequest, ToolFail, WireEvent, WireViewRecord, READ_CHUNK};
use imt_plugin_api::serve::ELEVATED_ROLE;
use imt_plugin_api::tools::FailureReason;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Longest elevated operation a plugin may ask for.
const MAX_ELEVATED_CALL: Duration = Duration::from_secs(3600);
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
    /// Whether its elevated process is started with administrator rights (the prompt on Windows). Only tests turn
    /// this off, to run the elevated role as an ordinary process.
    #[doc(hidden)]
    pub elevate: bool,
    /// A third-party plugin's folders (ADR 0027): it then runs contained by the launcher
    /// ([`imt_plugin_api::sandbox::sandbox_for_third_party`]) and its tools get its private data folder.
    pub home: Option<PluginHome>,
    /// The SHA-256 its program must have: checked before every start, so a program changed after it was installed
    /// never runs.
    pub program_sha256: Option<[u8; 32]>,
}

/// The SHA-256 of the file at `path`.
pub fn sha256_of(path: &std::path::Path) -> std::io::Result<[u8; 32]> {
    use sha2::Digest;
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
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
            elevate: true,
            home: None,
            program_sha256: None,
        }
    }

    /// The spec of an installed third-party plugin: its own program (no arguments), contained, verified by hash.
    pub fn third_party(exe: impl Into<PathBuf>, id: &str, home: PluginHome, sha256: [u8; 32], granted: Vec<Permission>) -> PluginSpec {
        PluginSpec {
            args: Vec::new(),
            granted,
            home: Some(home),
            program_sha256: Some(sha256),
            ..PluginSpec::new(exe, id)
        }
    }
}

/// Runs one of the plugin's elevated operations for its normal process.
type ElevatedRunner = Box<dyn Fn(&str, &str, Json, Duration) -> Result<Json, ToolFail> + Send + Sync>;

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
    /// The plugin's elevated operations (set once the process exists).
    elevated: OnceLock<ElevatedRunner>,
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
            HostRequest::InvokeElevated {
                tool,
                op,
                args,
                timeout_ms,
            } => {
                let run = self
                    .elevated
                    .get()
                    .ok_or_else(|| RemoteError::new(code::INVALID, "this process cannot run elevated operations"))?;
                let timeout = Duration::from_millis(timeout_ms).min(MAX_ELEVATED_CALL);
                encode_reply(&run(&tool, &op, args, timeout))
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
    /// The permissions granted now (revoking one restarts the process with fewer).
    granted: Arc<Mutex<Vec<Permission>>>,
    services: Arc<HostServices>,
    /// The settings last applied, restored when the process restarts.
    settings: Arc<Mutex<Option<serde_json::Value>>>,
}

impl PluginProcess {
    /// The process of the plugin `manifest` describes; started on first use.
    pub fn new(manifest: Manifest, spec: PluginSpec) -> Arc<PluginProcess> {
        let process = PluginProcess::create(manifest, spec);
        let weak = Arc::downgrade(&process);
        let _ = process
            .services
            .elevated
            .set(Box::new(move |tool, op, args, timeout| match weak.upgrade() {
                Some(p) => p.invoke_elevated(tool, op, args, timeout),
                None => Err(ToolFail {
                    reason: None,
                    message: "the plugin was shut down".into(),
                }),
            }));
        process
    }

    fn create(manifest: Manifest, spec: PluginSpec) -> Arc<PluginProcess> {
        let services = Arc::new(HostServices::default());
        let settings: Arc<Mutex<Option<serde_json::Value>>> = Arc::default();
        let granted = Arc::new(Mutex::new(spec.granted.clone()));
        let handler: Arc<dyn Handler> = services.clone();
        let restore_settings = settings.clone();
        let (m, s, g) = (manifest.clone(), spec.clone(), granted.clone());
        let start = move || {
            verify_program(&m, &s)?;
            let child_spec = child_spec(&m, &s, &lock(&g), false);
            launch(&child_spec, handler.clone())
        };
        let sup = Supervisor::new(spec.supervisor.clone(), start).with_restore(move |c: &Child, restarted| {
            let values = lock(&restore_settings).clone();
            match (restarted, values) {
                (true, Some(v)) => c
                    .peer()
                    .request::<_, ()>(&PluginRequest::ApplySettings { values: Json(v) }, Some(Duration::from_secs(10))),
                _ => Ok(()),
            }
        });
        Arc::new(PluginProcess {
            manifest,
            sup,
            elevated: Mutex::new(None),
            spec,
            granted,
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
        lock(&self.granted).contains(&p) && self.manifest.asks_for(p)
    }

    /// The permissions granted now.
    pub fn granted_now(&self) -> Vec<Permission> {
        lock(&self.granted).clone()
    }

    /// Changes the granted permissions: the process stops now and starts again on next use, in the sandbox of the
    /// new grants (revoking a permission takes effect at once).
    pub fn set_granted(&self, granted: Vec<Permission>) {
        *lock(&self.granted) = granted;
        self.shutdown();
    }

    /// The sandbox its process gets with the current grants.
    pub fn sandbox(&self) -> imt_ipc::SandboxSpec {
        child_spec(&self.manifest, &self.spec, &lock(&self.granted), false).spawn.sandbox
    }

    /// Whether it is a third-party plugin (installed from a package).
    pub fn third_party(&self) -> bool {
        self.spec.home.is_some()
    }

    /// The data folder its tools get: its own private folder for a third-party plugin, else `app`.
    pub fn tool_data_dir(&self, app: Option<&std::path::Path>) -> Option<String> {
        match &self.spec.home {
            Some(h) => Some(h.data_dir.to_string_lossy().into_owned()),
            None => app.map(|d| d.to_string_lossy().into_owned()),
        }
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
                let spec = child_spec(&self.manifest, &self.spec, &lock(&self.granted), true);
                let handler: Arc<dyn Handler> = self.services.clone();
                let mut cfg = self.spec.supervisor.clone();
                cfg.label = format!("{} (elevated)", cfg.label);
                Arc::new(Supervisor::new(cfg, move || launch(&spec, handler.clone())))
            })
            .clone()
    }

    /// Runs `tool op` in the elevated process for the plugin's normal process: only an operation the manifest
    /// declares elevated, with the app's data folder of the tool's last call.
    fn invoke_elevated(&self, tool: &str, op: &str, args: Json, timeout: Duration) -> Result<Json, ToolFail> {
        let fail = |message: String| ToolFail { reason: None, message };
        let declared = self
            .manifest
            .tools
            .iter()
            .find(|t| t.id == tool)
            .and_then(|t| t.ops.iter().find(|o| o.name == op))
            .is_some_and(|o| o.elevated);
        if !declared {
            return Err(fail(format!(
                "\"{tool} {op}\" is not an elevated operation of plugin \"{}\"",
                self.id()
            )));
        }
        let data_dir = lock(&self.services.tasks)
            .get(tool)
            .and_then(|c| c.data_dir().map(|d| d.to_string_lossy().into_owned()));
        let req = PluginRequest::InvokeTool {
            tool: tool.into(),
            op: op.into(),
            args,
            data_dir,
        };
        match self.call::<Result<Json, ToolFail>>(&req, Some(timeout), true) {
            Ok(r) => r,
            Err(IpcError::Denied(m)) => Err(ToolFail {
                reason: Some(FailureReason::Denied),
                message: m,
            }),
            Err(e) => Err(fail(format!("{tool} {op} (elevated): {e}"))),
        }
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

/// Refuses to start a third-party plugin whose program is not the one that was installed.
fn verify_program(manifest: &Manifest, spec: &PluginSpec) -> Result<(), IpcError> {
    let Some(expected) = spec.program_sha256 else {
        return Ok(());
    };
    match sha256_of(&spec.exe) {
        Ok(actual) if actual == expected => Ok(()),
        Ok(_) => Err(IpcError::Spawn(format!(
            "the program of plugin \"{}\" changed after it was installed, so IMT does not run it; reinstall the plugin",
            manifest.id
        ))),
        Err(e) => Err(IpcError::Spawn(format!(
            "cannot check the program of plugin \"{}\" ({}): {e}; reinstall the plugin",
            manifest.id,
            spec.exe.display()
        ))),
    }
}

/// The child spec of a plugin's normal or elevated process with `granted`.
fn child_spec(manifest: &Manifest, spec: &PluginSpec, granted: &[Permission], elevated: bool) -> ChildSpec {
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
    s.sandbox = match &spec.home {
        Some(home) => {
            s.current_dir = Some(home.program_dir.clone());
            sandbox_for_third_party(manifest, granted, home)
        }
        None => sandbox_for(manifest, granted, elevated),
    };
    let mut c = ChildSpec::new(s);
    c.peer.protocol = imt_plugin_api::PROTOCOL;
    if elevated {
        c.role = ELEVATED_ROLE.into();
    }
    if elevated && spec.elevate {
        c.elevation = Elevation::Required {
            reason: format!("{} needs administrator rights for this", manifest.name),
        };
    }
    c
}
