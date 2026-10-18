//! The supervisor side of the protocol: `GrpcTransport` implements `WorkerTransport` over gRPC on a local
//! socket (Unix domain socket or Windows named pipe, never TCP), using the shared transport of `imt_ipc`.
//! It starts the worker as a child process, hands it a random per-launch token and a private address on
//! stdin (stdin staying open is the worker's lifeline: when IMT exits, the worker exits), and attaches the
//! token to every call. Every call has a deadline; a chat stream tolerates only `timeout` of silence.

use super::pb::{self, worker_client::WorkerClient};
use super::protocol::*;
use super::transport::{CallOpts, TransportError, WorkerLauncher, WorkerTransport};
use imt_ipc::client::{authenticated, connect_channel, AuthChannel};
use imt_ipc::process::{launch, ChildProcess, SpawnSpec};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio_stream::StreamExt;
use tonic::{Request, Status};

type Client = WorkerClient<AuthChannel>;

/// How long to wait for a freshly started worker to start listening.
const CONNECT_WINDOW: Duration = Duration::from_secs(15);

pub struct GrpcTransport {
    rt: Runtime,
    client: Client,
    process: Option<ChildProcess>,
    pid: Option<u32>,
    next_id: u64,
    _endpoint: Option<imt_ipc::endpoint::PrivateEndpoint>,
}

fn runtime() -> Result<Runtime, TransportError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| TransportError::Spawn(format!("cannot start the network runtime: {e}")))
}

fn make_client(channel: AuthChannel) -> Client {
    WorkerClient::new(channel)
        .max_decoding_message_size(MAX_MESSAGE_BYTES)
        .max_encoding_message_size(MAX_MESSAGE_BYTES)
}

fn typed<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, TransportError> {
    serde_json::from_value(v).map_err(|e| TransportError::Protocol(format!("invalid parameters: {e}")))
}

impl GrpcTransport {
    /// Attaches to a worker that is already listening (in-process servers and tests).
    pub fn connect(address: &str, token: &str) -> Result<GrpcTransport, TransportError> {
        let rt = runtime()?;
        let channel = rt
            .block_on(connect_channel(address.to_string()))
            .map_err(|e| TransportError::Died(format!("cannot reach the engine: {e}")))?;
        let client = make_client(authenticated(channel, token)?);
        Ok(GrpcTransport {
            rt,
            client,
            process: None,
            pid: None,
            next_id: 1,
            _endpoint: None,
        })
    }

    /// Starts the worker program and connects to it (see `imt_ipc::process`: private address, per-launch
    /// token on stdin, below-normal priority, job object on Windows).
    #[allow(clippy::ptr_arg)] // the public signature is kept
    pub fn spawn(exe: &PathBuf, args: &[String], env: &[(String, String)]) -> Result<GrpcTransport, TransportError> {
        let rt = runtime()?;
        let mut spec = SpawnSpec::new(exe.clone(), "the AI engine");
        spec.args = args.to_vec();
        spec.env = env.to_vec();
        spec.endpoint_prefix = "imt-ai".into();
        spec.connect_window = CONNECT_WINDOW;
        let launched = rt.block_on(launch(&spec))?;
        Ok(GrpcTransport {
            client: make_client(launched.channel),
            pid: Some(launched.process.pid()),
            process: Some(launched.process),
            _endpoint: Some(launched.endpoint),
            rt,
            next_id: 1,
        })
    }

    fn exit_text(&mut self) -> String {
        match &self.process {
            Some(p) => p.exit_text(),
            None => "the engine closed the connection".into(),
        }
    }

    /// A gRPC failure as a transport error: typed worker errors pass through; a broken connection is a crash.
    fn map_status(&mut self, s: Status) -> TransportError {
        let (level, message) = status_log(&s);
        let e = imt_ipc::error::from_status(&s, "the engine", || self.exit_text());
        // A broken connection means the worker died: its exit code or signal says why.
        let exit = match &e {
            TransportError::Died(why) => why.chars().take(MAX_STATUS_LOG).collect(),
            _ => String::new(),
        };
        log::log!(level, StatusCode = format!("{:?}", s.code()).as_str(), Code = s.code() as i32, Exit = exit.as_str(); "AI engine call failed: {message}");
        e
    }

    fn request<T>(msg: T, timeout: Duration) -> Request<T> {
        let mut r = Request::new(msg);
        r.set_timeout(timeout);
        r
    }

    fn unary<R>(
        &mut self,
        timeout: Duration,
        fut: impl std::future::Future<Output = Result<tonic::Response<R>, Status>>,
    ) -> Result<R, TransportError> {
        match self
            .rt
            .block_on(async { tokio::time::timeout(timeout + Duration::from_millis(250), fut).await })
        {
            Err(_) => Err(TransportError::Timeout),
            Ok(Err(s)) => Err(self.map_status(s)),
            Ok(Ok(r)) => Ok(r.into_inner()),
        }
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
        let req = pb::ChatRequest {
            request_id: id,
            prompt: p.prompt,
            max_tokens: p.max_tokens as u64,
            temperature: p.temperature,
            json_schema: p.json_schema.map(|s| s.to_string()).unwrap_or_default(),
        };
        let mut client = self.client.clone();
        let mut stream = match self
            .rt
            .block_on(async { tokio::time::timeout(opts.timeout, client.chat(Request::new(req))).await })
        {
            Err(_) => return Err(TransportError::Timeout),
            Ok(Err(s)) => return Err(self.map_status(s)),
            Ok(Ok(r)) => r.into_inner(),
        };
        let mut deadline = Instant::now() + opts.timeout;
        let mut cancel_sent: Option<Instant> = None;
        loop {
            if cancel_sent.is_none() && cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                cancel_sent = Some(Instant::now());
                let mut c2 = self.client.clone();
                self.rt.spawn(async move {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(2),
                        c2.cancel(Request::new(pb::CancelRequest { request_id: id })),
                    )
                    .await;
                });
            }
            if cancel_sent.is_some_and(|t| t.elapsed() > opts.cancel_grace) {
                return Err(TransportError::Timeout);
            }
            match self
                .rt
                .block_on(async { tokio::time::timeout(Duration::from_millis(25), stream.next()).await })
            {
                Err(_) => {
                    if Instant::now() > deadline {
                        return Err(TransportError::Timeout);
                    }
                }
                Ok(None) => {
                    return Err(if self.child_gone() {
                        TransportError::Died(self.exit_text())
                    } else {
                        TransportError::Protocol("the answer ended without a result".into())
                    })
                }
                Ok(Some(Err(s))) => return Err(self.map_status(s)),
                Ok(Some(Ok(ev))) => {
                    // The silence before this message was longer than allowed: treat it as a hang.
                    if Instant::now() > deadline {
                        return Err(TransportError::Timeout);
                    }
                    deadline = Instant::now() + opts.timeout;
                    match ev.event {
                        Some(pb::chat_event::Event::Delta(d)) => on_stream(json!({"id": id, "text": d.text})),
                        Some(pb::chat_event::Event::Done(d)) => {
                            let finish = match pb::Finish::try_from(d.finish) {
                                Ok(pb::Finish::Stop) => WireFinish::Stop,
                                Ok(pb::Finish::Length) => WireFinish::Length,
                                Ok(pb::Finish::Cancelled) => WireFinish::Cancelled,
                                Err(_) => return Err(TransportError::Protocol("unknown finish reason".into())),
                            };
                            return Ok(json!(ChatResult {
                                text: d.text,
                                prompt_tokens: d.prompt_tokens as usize,
                                completion_tokens: d.completion_tokens as usize,
                                finish
                            }));
                        }
                        None => return Err(TransportError::Protocol("an empty event in the answer".into())),
                    }
                }
            }
        }
    }

    fn child_gone(&mut self) -> bool {
        self.process.as_ref().is_some_and(ChildProcess::exited)
    }
}

impl WorkerTransport for GrpcTransport {
    fn call(
        &mut self,
        method_name: &str,
        params: Value,
        opts: CallOpts,
        cancel: Option<&AtomicBool>,
        on_stream: &mut dyn FnMut(Value),
    ) -> Result<Value, TransportError> {
        let mut client = self.client.clone();
        let t = opts.timeout;
        match method_name {
            method::HELLO => {
                let p: HelloParams = typed(params)?;
                let r = self.unary(
                    t,
                    client.hello(Self::request(
                        pb::HelloRequest {
                            protocol_version: p.protocol_version,
                            client: p.client,
                        },
                        t,
                    )),
                )?;
                Ok(json!(HelloResult {
                    protocol_version: r.protocol_version,
                    worker_version: r.worker_version,
                    pid: r.pid
                }))
            }
            method::LOAD => {
                let p: LoadParams = typed(params)?;
                let r = self.unary(
                    t,
                    client.load(Self::request(
                        pb::LoadRequest {
                            model_path: p.model_path,
                            gpu_layers: p.gpu_layers,
                            context: p.context as u64,
                            threads: p.threads,
                        },
                        t,
                    )),
                )?;
                Ok(json!(LoadResult {
                    backend: r.backend,
                    context: r.context as usize,
                    name: r.name,
                    rss_bytes: r.rss_bytes
                }))
            }
            method::CHAT => self.chat(typed(params)?, opts, cancel, on_stream),
            method::UNLOAD => {
                self.unary(t, client.unload(Self::request(pb::Empty {}, t)))?;
                Ok(json!({}))
            }
            method::STATUS => {
                let r = self.unary(t, client.status(Self::request(pb::Empty {}, t)))?;
                Ok(json!(StatusResult {
                    loaded: r.loaded,
                    backend: r.backend,
                    rss_bytes: r.rss_bytes,
                    pid: r.pid,
                    uptime_secs: r.uptime_secs
                }))
            }
            method::SHUTDOWN => {
                self.unary(t, client.shutdown(Self::request(pb::Empty {}, t)))?;
                Ok(json!({}))
            }
            other => Err(TransportError::Protocol(format!("unknown call {other}"))),
        }
    }

    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn kill(&mut self) {
        if let Some(p) = self.process.take() {
            p.kill();
        }
    }
}

impl Drop for GrpcTransport {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The worker's extra environment, evaluated at every launch, so a changed setting (such as the inference
/// build) reaches the next worker without rebuilding the launcher.
pub type WorkerEnv = Arc<dyn Fn() -> Vec<(String, String)> + Send + Sync>;

/// An environment that never changes.
pub fn fixed_env(env: Vec<(String, String)>) -> WorkerEnv {
    Arc::new(move || env.clone())
}

/// Starts workers as child processes speaking gRPC.
pub struct GrpcLauncher {
    pub exe: PathBuf,
    pub args: Vec<String>,
    /// Extra environment for the worker, evaluated at each launch.
    pub env: WorkerEnv,
}

impl WorkerLauncher for GrpcLauncher {
    fn launch(&self) -> Result<Box<dyn WorkerTransport>, TransportError> {
        Ok(Box::new(GrpcTransport::spawn(&self.exe, &self.args, &(self.env)())?))
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

/// Where the bundled worker lives: next to the application executable (Tauri puts `externalBin` sidecars
/// there, without the target-triple suffix). `IMT_AI_WORKER` overrides it, for development.
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

/// Longest gRPC status message logged.
const MAX_STATUS_LOG: usize = 300;

/// How a failed gRPC call is logged: an error the engine reports on purpose (a typed error in the trailer,
/// e.g. no model loaded) at Warning; a timeout, a broken connection or a protocol error at Error. The
/// message is capped (it never carries prompts or answers, only the engine's error text).
fn status_log(s: &Status) -> (log::Level, String) {
    let typed = imt_ipc::error::remote_error(s).is_some();
    let level = if typed { log::Level::Warn } else { log::Level::Error };
    let message: String = s.message().chars().take(MAX_STATUS_LOG).collect();
    (level, format!("{:?}: {message}", s.code()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grpc_failures_are_logged_by_kind() {
        let (level, text) = status_log(&Status::deadline_exceeded("too slow"));
        assert_eq!((level, text.as_str()), (log::Level::Error, "DeadlineExceeded: too slow"));
        let typed = imt_ipc::error::to_status(
            tonic::Code::FailedPrecondition,
            &imt_ipc::error::RemoteError::new(-32001, "no model"),
        );
        assert_eq!(status_log(&typed).0, log::Level::Warn);
        let long = Status::unavailable("x".repeat(1000));
        assert!(status_log(&long).1.len() < MAX_STATUS_LOG + 20);
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
