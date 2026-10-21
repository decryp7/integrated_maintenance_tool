//! The worker protocol over IMT's one IPC (ADR 0017): postcard frames on a private local stream, IMT listening
//! and the worker connecting with the per-launch token (`imt_ipc::launch`, `imt_ipc::child`).
//!
//! - [`WorkerRequest`] is the body of every call; replies are the typed results of [`super::protocol`].
//!   `Chat` streams its token deltas as items (with credits) and ends with the [`ChatResult`]; cancelling the
//!   call stops generation between tokens.
//! - [`FramesTransport`] is the supervisor's side (`WorkerTransport`); [`FramesLauncher`] starts workers.
//! - [`run_worker`] is the worker's `main`: it serves a `WorkerBackend` on one dedicated thread.

use super::backend::{current_rss_bytes, WorkerBackend};
use super::protocol::*;
use super::transport::{CallOpts, TransportError, WorkerLauncher, WorkerTransport};
use imt_ipc::launch::{launch, Child, ChildSpec};
use imt_ipc::peer::{decode_request, encode_reply, CallCtx, Handler, NoCalls, Peer, PeerOptions, StreamEvent};
use imt_ipc::process::SpawnSpec;
use imt_ipc::{IpcError, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// Name of the AI worker's child log (its llama-server's is [`LLAMA_LOG`]).
pub const WORKER_LOG: &str = "ai-worker";
/// Name of llama-server's child log; the worker forwards its output as `[llama-server] text`.
pub const LLAMA_LOG: &str = "llama-server";
/// How long to wait for a freshly started worker to connect.
const CONNECT_WINDOW: Duration = Duration::from_secs(15);
/// How often a waiting chat looks at its cancel flag.
const POLL: Duration = Duration::from_millis(25);
/// Token deltas in flight before the supervisor takes them.
const CHAT_CREDIT: u32 = 64;

/// The worker's log spec: its own standard error, and llama-server's forwarded into a log of its own.
pub fn worker_log() -> imt_ipc::child_log::ChildLogSpec {
    use imt_ipc::child_log::{ChildKind, ChildLogSpec};
    ChildLogSpec::new(WORKER_LOG, ChildKind::AiWorker)
        .with_version(env!("CARGO_PKG_VERSION"))
        .forwarding(ChildLogSpec::new(LLAMA_LOG, ChildKind::LlamaServer))
}

/// A chat request on the wire (the JSON schema as JSON text).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatWire {
    pub prompt: String,
    pub max_tokens: u64,
    pub temperature: f32,
    pub json_schema: Option<Json>,
}

/// What the supervisor asks the worker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorkerRequest {
    /// → [`HelloResult`]. Must come first.
    Hello(HelloParams),
    /// → [`LoadResult`].
    Load(LoadParams),
    /// Items: `String` deltas; → [`ChatResult`].
    Chat(ChatWire),
    /// → `()`.
    Unload,
    /// → [`StatusResult`].
    Status,
    /// → `()`; the worker exits.
    Shutdown,
}

// ---- the worker's side ----

enum Job {
    Load(LoadParams, mpsc::Sender<Result<LoadResult, RpcError>>),
    Chat {
        id: u64,
        params: ChatParams,
        deltas: mpsc::Sender<Result<String, Result<ChatResult, RpcError>>>,
    },
    Unload(mpsc::Sender<()>),
}

fn guarded<T>(f: impl FnOnce() -> Result<T, RpcError>) -> Result<T, RpcError> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| Err(RpcError::new(code::ENGINE, "the engine hit an internal error")))
}

/// Runs the backend on its own thread; it is created there and never leaves it.
fn spawn_backend<F>(factory: F, cancel: Arc<AtomicU64>) -> mpsc::Sender<Job>
where
    F: FnOnce() -> Box<dyn WorkerBackend> + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<Job>();
    std::thread::spawn(move || {
        let mut backend = factory();
        for job in rx {
            match job {
                Job::Load(p, reply) => {
                    let _ = reply.send(guarded(|| backend.load(&p)));
                }
                Job::Unload(reply) => {
                    backend.unload();
                    let _ = reply.send(());
                }
                Job::Chat { id, params, deltas } => {
                    let cancelled = || cancel.load(Ordering::SeqCst) == id;
                    let result = guarded(|| {
                        backend.chat(
                            &params,
                            &mut |t| {
                                let _ = deltas.send(Ok(t.to_string()));
                            },
                            &cancelled,
                        )
                    });
                    let _ = deltas.send(Err(result));
                }
            }
        }
    });
    tx
}

/// Answers the supervisor's calls with a backend.
pub struct WorkerHandler {
    jobs: Mutex<mpsc::Sender<Job>>,
    cancel: Arc<AtomicU64>,
    next_chat: AtomicU64,
    loaded: Mutex<Option<LoadResult>>,
    said_hello: AtomicBool,
    started: Instant,
    /// `Shutdown` ends the process (the worker); in-process servers only stop answering.
    exit_on_shutdown: bool,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl WorkerHandler {
    pub fn new<F>(factory: F) -> WorkerHandler
    where
        F: FnOnce() -> Box<dyn WorkerBackend> + Send + 'static,
    {
        let cancel = Arc::new(AtomicU64::new(0));
        WorkerHandler {
            jobs: Mutex::new(spawn_backend(factory, cancel.clone())),
            cancel,
            next_chat: AtomicU64::new(1),
            loaded: Mutex::new(None),
            said_hello: AtomicBool::new(false),
            started: Instant::now(),
            exit_on_shutdown: false,
        }
    }

    /// The same handler ending its process on `Shutdown` (the worker's).
    pub fn exiting(mut self) -> WorkerHandler {
        self.exit_on_shutdown = true;
        self
    }

    fn send(&self, job: Job) -> Result<(), RpcError> {
        lock(&self.jobs)
            .send(job)
            .map_err(|_| RpcError::new(code::ENGINE, "the engine thread has stopped"))
    }

    fn stopped() -> RpcError {
        RpcError::new(code::ENGINE, "the engine thread has stopped")
    }

    fn chat(&self, ctx: &CallCtx, w: ChatWire) -> Result<Vec<u8>, RpcError> {
        if lock(&self.loaded).is_none() {
            return Err(RpcError::new(code::NOT_LOADED, "no model is loaded"));
        }
        let params = ChatParams {
            prompt: w.prompt,
            max_tokens: w.max_tokens as usize,
            temperature: w.temperature,
            json_schema: w.json_schema.map(Json::into_inner),
        };
        let id = self.next_chat.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.send(Job::Chat { id, params, deltas: tx })?;
        loop {
            if ctx.is_cancelled() {
                self.cancel.store(id, Ordering::SeqCst);
            }
            match rx.recv_timeout(POLL) {
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Self::stopped()),
                Ok(Ok(delta)) => {
                    // A cancelled caller takes no more deltas: generation stops at the next token.
                    if ctx.send(&delta).is_err() {
                        self.cancel.store(id, Ordering::SeqCst);
                    }
                }
                Ok(Err(done)) => return encode_reply(&done?),
            }
        }
    }
}

impl Handler for WorkerHandler {
    fn call(&self, ctx: &CallCtx, body: &[u8]) -> Result<Vec<u8>, RpcError> {
        let req: WorkerRequest = decode_request(body)?;
        if !matches!(req, WorkerRequest::Hello(_)) && !self.said_hello.load(Ordering::SeqCst) {
            return Err(RpcError::new(code::INVALID_REQUEST, "say hello first"));
        }
        match req {
            WorkerRequest::Hello(p) => {
                if p.protocol_version != PROTOCOL_VERSION {
                    return Err(RpcError::new(
                        code::INVALID_PARAMS,
                        format!(
                            "protocol version {} is not supported (this engine speaks {PROTOCOL_VERSION})",
                            p.protocol_version
                        ),
                    ));
                }
                self.said_hello.store(true, Ordering::SeqCst);
                encode_reply(&HelloResult {
                    protocol_version: PROTOCOL_VERSION,
                    worker_version: env!("CARGO_PKG_VERSION").into(),
                    pid: std::process::id(),
                })
            }
            WorkerRequest::Load(p) => {
                let (tx, rx) = mpsc::channel();
                self.send(Job::Load(p, tx))?;
                let result = rx.recv().map_err(|_| Self::stopped())??;
                *lock(&self.loaded) = Some(result.clone());
                encode_reply(&result)
            }
            WorkerRequest::Chat(w) => self.chat(ctx, w),
            WorkerRequest::Unload => {
                let (tx, rx) = mpsc::channel();
                self.send(Job::Unload(tx))?;
                let _ = rx.recv();
                *lock(&self.loaded) = None;
                encode_reply(&())
            }
            WorkerRequest::Status => {
                let loaded = lock(&self.loaded).clone();
                encode_reply(&StatusResult {
                    loaded: loaded.is_some(),
                    backend: loaded.map(|l| l.backend).unwrap_or_default(),
                    rss_bytes: current_rss_bytes(),
                    pid: std::process::id(),
                    uptime_secs: self.started.elapsed().as_secs(),
                })
            }
            WorkerRequest::Shutdown => {
                // Answer first, then leave (the supervisor stops waiting for the reply at once).
                if self.exit_on_shutdown {
                    std::thread::spawn(|| {
                        std::thread::sleep(Duration::from_millis(50));
                        std::process::exit(0);
                    });
                }
                encode_reply(&())
            }
        }
    }
}

/// The worker's `main`: reads its bootstrap, connects, locks down and serves until IMT closes the connection
/// or stdin (the supervisor is gone, so nothing is left behind).
pub fn run_worker<F>(factory: F) -> std::io::Result<()>
where
    F: FnOnce() -> Box<dyn WorkerBackend> + Send + 'static,
{
    imt_ipc::child::child_main("IMT", move |_| Arc::new(WorkerHandler::new(factory).exiting()))
}

// ---- the supervisor's side ----

/// One live connection to one worker.
pub struct FramesTransport {
    peer: Peer,
    child: Option<Child>,
    pid: Option<u32>,
    next_id: u64,
}

fn typed<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, TransportError> {
    serde_json::from_value(v).map_err(|e| TransportError::Protocol(format!("invalid parameters: {e}")))
}

impl FramesTransport {
    /// Starts the worker program and accepts its connection (private endpoint, per-launch token, cleared
    /// environment but what llama-server needs, below-normal priority, a job object on Windows).
    pub fn spawn(exe: &std::path::Path, args: &[String], env: &[(String, String)]) -> Result<FramesTransport, TransportError> {
        let mut spec = SpawnSpec::new(exe.to_path_buf(), "the AI engine");
        spec.args = args.to_vec();
        spec.env = env.to_vec();
        spec.keep_env = super::WORKER_ENV;
        spec.endpoint_prefix = "imt-ai".into();
        spec.connect_window = CONNECT_WINDOW;
        spec.log = Some(worker_log());
        let mut c = ChildSpec::new(spec);
        c.peer = PeerOptions {
            label: "the AI engine".into(),
            max_frame: MAX_MESSAGE_BYTES,
            max_concurrent_calls: 8,
            ..PeerOptions::new("")
        };
        let child = launch(&c, Arc::new(NoCalls))?;
        Ok(FramesTransport {
            peer: child.peer().clone(),
            pid: Some(child.pid()),
            child: Some(child),
            next_id: 1,
        })
    }

    /// A worker served in this process over an in-memory stream (tests, and hosts that need no isolation).
    pub fn in_process<F>(factory: F) -> Result<FramesTransport, TransportError>
    where
        F: FnOnce() -> Box<dyn WorkerBackend> + Send + 'static,
    {
        let rt = imt_ipc::runtime::shared().map_err(TransportError::Spawn)?;
        let (a, b) = tokio::io::duplex(1 << 20);
        let _worker = Peer::start(b, Arc::new(WorkerHandler::new(factory)), PeerOptions::new("IMT"), rt.handle());
        let peer = Peer::start(a, Arc::new(NoCalls), PeerOptions::new("the AI engine"), rt.handle());
        Ok(FramesTransport {
            peer,
            child: None,
            pid: None,
            next_id: 1,
        })
    }

    fn died(&self, e: IpcError) -> TransportError {
        match (e, &self.child) {
            (IpcError::Died(_), Some(c)) => IpcError::Died(c.exit_text()),
            (e, _) => e,
        }
    }

    fn logged(&self, e: TransportError) -> TransportError {
        let e = self.died(e);
        let (level, message) = log_of(&e);
        log::log!(level, Kind = kind_of(&e); "AI engine call failed: {message}");
        e
    }

    fn unary<Rep: serde::de::DeserializeOwned>(&self, req: &WorkerRequest, timeout: Duration) -> Result<Rep, TransportError> {
        self.peer.request(req, Some(timeout)).map_err(|e| self.logged(e))
    }

    fn chat(
        &mut self,
        p: ChatParams,
        opts: CallOpts,
        cancel: Option<&AtomicBool>,
        on_stream: &mut dyn FnMut(Value),
    ) -> Result<Value, TransportError> {
        let id = self.next_id;
        self.next_id += 1;
        let req = WorkerRequest::Chat(ChatWire {
            prompt: p.prompt,
            max_tokens: p.max_tokens as u64,
            temperature: p.temperature,
            json_schema: p.json_schema.map(Json),
        });
        let mut stream = self.peer.stream_request(&req, CHAT_CREDIT).map_err(|e| self.logged(e))?;
        let mut deadline = Instant::now() + opts.timeout;
        let mut cancel_sent: Option<Instant> = None;
        loop {
            if cancel_sent.is_none() && cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                cancel_sent = Some(Instant::now());
                stream.cancel();
            }
            if cancel_sent.is_some_and(|t| t.elapsed() > opts.cancel_grace) {
                return Err(self.logged(TransportError::Timeout));
            }
            match stream.next(Some(POLL)) {
                Ok(StreamEvent::Idle) => {
                    if Instant::now() > deadline {
                        let _ = stream.cancel_and_wait();
                        return Err(self.logged(TransportError::Timeout));
                    }
                }
                Ok(StreamEvent::Item(b)) => {
                    // The silence before this delta was longer than allowed: treat it as a hang.
                    if Instant::now() > deadline {
                        return Err(self.logged(TransportError::Timeout));
                    }
                    deadline = Instant::now() + opts.timeout;
                    let text: String = imt_ipc::wire::decode(&b).map_err(TransportError::Protocol)?;
                    on_stream(json!({"id": id, "text": text}));
                }
                Ok(StreamEvent::Done(b)) => {
                    let r: ChatResult = imt_ipc::wire::decode(&b).map_err(TransportError::Protocol)?;
                    return Ok(json!(r));
                }
                Err(e) => return Err(self.logged(e)),
            }
        }
    }
}

impl WorkerTransport for FramesTransport {
    fn call(
        &mut self,
        method_name: &str,
        params: Value,
        opts: CallOpts,
        cancel: Option<&AtomicBool>,
        on_stream: &mut dyn FnMut(Value),
    ) -> Result<Value, TransportError> {
        let t = opts.timeout;
        match method_name {
            method::HELLO => {
                let r: HelloResult = self.unary(&WorkerRequest::Hello(typed(params)?), t)?;
                Ok(json!(r))
            }
            method::LOAD => {
                let r: LoadResult = self.unary(&WorkerRequest::Load(typed(params)?), t)?;
                Ok(json!(r))
            }
            method::CHAT => self.chat(typed(params)?, opts, cancel, on_stream),
            method::UNLOAD => {
                self.unary::<()>(&WorkerRequest::Unload, t)?;
                Ok(json!({}))
            }
            method::STATUS => {
                let r: StatusResult = self.unary(&WorkerRequest::Status, t)?;
                Ok(json!(r))
            }
            method::SHUTDOWN => {
                self.unary::<()>(&WorkerRequest::Shutdown, t)?;
                Ok(json!({}))
            }
            other => Err(TransportError::Protocol(format!("unknown call {other}"))),
        }
    }

    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn kill(&mut self) {
        self.peer.close();
        if let Some(c) = self.child.take() {
            c.stop();
        }
    }
}

impl Drop for FramesTransport {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The worker's extra environment, evaluated at every launch, so a changed setting (such as the inference build)
/// reaches the next worker without rebuilding the launcher.
pub type WorkerEnv = Arc<dyn Fn() -> Vec<(String, String)> + Send + Sync>;

/// An environment that never changes.
pub fn fixed_env(env: Vec<(String, String)>) -> WorkerEnv {
    Arc::new(move || env.clone())
}

/// Starts workers as child processes.
pub struct FramesLauncher {
    pub exe: PathBuf,
    pub args: Vec<String>,
    /// Extra environment for the worker, evaluated at each launch.
    pub env: WorkerEnv,
}

impl WorkerLauncher for FramesLauncher {
    fn launch(&self) -> Result<Box<dyn WorkerTransport>, TransportError> {
        Ok(Box::new(FramesTransport::spawn(&self.exe, &self.args, &(self.env)())?))
    }
}

/// File name of the worker binary on this platform (as bundled: no target-triple suffix).
pub fn worker_file_name() -> &'static str {
    if cfg!(windows) {
        "imt-ai-worker.exe"
    } else {
        "imt-ai-worker"
    }
}

/// Where the bundled worker lives: next to the application executable (Tauri puts `externalBin` sidecars there,
/// without the target-triple suffix). `IMT_AI_WORKER` overrides it, for development.
pub fn default_worker_path() -> PathBuf {
    if let Some(p) = std::env::var_os("IMT_AI_WORKER") {
        return PathBuf::from(p);
    }
    let dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from))
        .unwrap_or_default();
    dir.join(worker_file_name())
}

/// Longest error text logged.
const MAX_LOG: usize = 300;

fn kind_of(e: &TransportError) -> &'static str {
    match e {
        TransportError::Spawn(_) => "spawn",
        TransportError::Died(_) => "died",
        TransportError::Timeout => "timeout",
        TransportError::Protocol(_) => "protocol",
        TransportError::Remote(_) => "remote",
        TransportError::Denied(_) => "denied",
    }
}

/// How a failed call is logged: an error the engine reports on purpose (a typed error, e.g. no model loaded) at
/// Warning; a timeout, a broken connection or a protocol error at Error. The message is capped (it never carries
/// prompts or answers, only the engine's error text).
fn log_of(e: &TransportError) -> (log::Level, String) {
    let level = if e.is_process_failure() {
        log::Level::Error
    } else {
        log::Level::Warn
    };
    (level, e.to_string().chars().take(MAX_LOG).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_are_logged_by_kind() {
        let (level, text) = log_of(&TransportError::Timeout);
        assert_eq!((level, text.as_str()), (log::Level::Error, "it stopped responding"));
        assert_eq!(
            log_of(&TransportError::Remote(RpcError::new(-32001, "no model"))).0,
            log::Level::Warn
        );
        assert!(log_of(&TransportError::Died("x".repeat(1000))).1.len() <= MAX_LOG * 4);
        for e in [
            TransportError::Spawn(String::new()),
            TransportError::Died(String::new()),
            TransportError::Timeout,
            TransportError::Protocol(String::new()),
            TransportError::Remote(RpcError::new(1, "r")),
            TransportError::Denied(String::new()),
        ] {
            assert!(!kind_of(&e).is_empty());
        }
    }

    #[test]
    fn requests_roundtrip() {
        let r = WorkerRequest::Chat(ChatWire {
            prompt: "p".into(),
            max_tokens: 5,
            temperature: 0.5,
            json_schema: Some(json!({"type": "object"}).into()),
        });
        assert_eq!(
            imt_ipc::wire::decode::<WorkerRequest>(&imt_ipc::wire::encode(&r).unwrap()).unwrap(),
            r
        );
    }

    #[test]
    fn worker_path_is_next_to_the_executable_unless_overridden() {
        // One test body: the environment is process-wide.
        std::env::remove_var("IMT_AI_WORKER");
        let p = default_worker_path();
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), worker_file_name());
        assert_eq!(p.parent(), std::env::current_exe().unwrap().parent());
        std::env::set_var("IMT_AI_WORKER", "/custom/worker");
        assert_eq!(default_worker_path(), PathBuf::from("/custom/worker"));
        std::env::remove_var("IMT_AI_WORKER");
    }
}
