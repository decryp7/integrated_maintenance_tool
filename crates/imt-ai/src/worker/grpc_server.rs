//! The worker side of the protocol: a gRPC server on a local socket, protected by the per-launch token. It
//! runs a `WorkerBackend` on one dedicated thread (so the backend need not be `Send`), streams token deltas
//! as a server-streaming RPC, and ends when asked to shut down or when the supervisor goes away (stdin
//! closes). Used by the real worker (llama.cpp) and by the fake worker of the tests.

use super::backend::{current_rss_bytes, WorkerBackend};
use super::pb::{
    self,
    worker_server::{Worker, WorkerServer},
};
use super::protocol::*;
use std::future::Future;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot, Notify};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status};

enum ChatItem {
    Delta(String),
    Done(Result<ChatResult, RpcError>),
}

enum Job {
    Load(LoadParams, oneshot::Sender<Result<LoadResult, RpcError>>),
    Chat {
        id: u64,
        params: ChatParams,
        out: mpsc::UnboundedSender<ChatItem>,
    },
    Unload(oneshot::Sender<()>),
}

fn guarded<T>(f: impl FnOnce() -> Result<T, RpcError>) -> Result<T, RpcError> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| Err(RpcError::new(code::ENGINE, "the engine hit an internal error")))
}

/// Runs the backend on its own thread; it is created there and never leaves it.
fn spawn_backend<F>(factory: F, cancel: Arc<AtomicU64>) -> std::sync::mpsc::Sender<Job>
where
    F: FnOnce() -> Box<dyn WorkerBackend> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel::<Job>();
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
                Job::Chat { id, params, out } => {
                    let cancelled = || cancel.load(Ordering::SeqCst) == id;
                    let result = guarded(|| {
                        backend.chat(
                            &params,
                            &mut |t| {
                                let _ = out.send(ChatItem::Delta(t.to_string()));
                            },
                            &cancelled,
                        )
                    });
                    let _ = out.send(ChatItem::Done(result));
                }
            }
        }
    });
    tx
}

/// An error as a gRPC status, with the typed error in the binary trailer.
pub fn to_status(e: &RpcError) -> Status {
    let c = match e.code {
        code::INVALID_PARAMS | code::INVALID_REQUEST => tonic::Code::InvalidArgument,
        code::LOAD_FAILED | code::NOT_LOADED => tonic::Code::FailedPrecondition,
        code::CONTEXT_OVERFLOW => tonic::Code::ResourceExhausted,
        code::METHOD_NOT_FOUND => tonic::Code::Unimplemented,
        code::BUSY => tonic::Code::Unavailable,
        _ => tonic::Code::Internal,
    };
    imt_ipc::error::to_status(c, e)
}

struct Service {
    jobs: std::sync::mpsc::Sender<Job>,
    cancel: Arc<AtomicU64>,
    loaded: Mutex<Option<LoadResult>>,
    said_hello: AtomicBool,
    started: Instant,
    shutdown: Arc<Notify>,
}

type ChatStream = Pin<Box<dyn Stream<Item = Result<pb::ChatEvent, Status>> + Send>>;

impl Service {
    fn gate(&self) -> Result<(), Status> {
        if self.said_hello.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(to_status(&RpcError::new(code::INVALID_REQUEST, "say hello first")))
        }
    }

    fn send(&self, job: Job) -> Result<(), Status> {
        self.jobs
            .send(job)
            .map_err(|_| to_status(&RpcError::new(code::ENGINE, "the engine thread has stopped")))
    }
}

#[tonic::async_trait]
impl Worker for Service {
    async fn hello(&self, req: Request<pb::HelloRequest>) -> Result<Response<pb::HelloReply>, Status> {
        let r = req.into_inner();
        if r.protocol_version != PROTOCOL_VERSION {
            return Err(to_status(&RpcError::new(
                code::INVALID_PARAMS,
                format!(
                    "protocol version {} is not supported (this engine speaks {PROTOCOL_VERSION})",
                    r.protocol_version
                ),
            )));
        }
        self.said_hello.store(true, Ordering::SeqCst);
        Ok(Response::new(pb::HelloReply {
            protocol_version: PROTOCOL_VERSION,
            worker_version: env!("CARGO_PKG_VERSION").into(),
            pid: std::process::id(),
        }))
    }

    async fn load(&self, req: Request<pb::LoadRequest>) -> Result<Response<pb::LoadReply>, Status> {
        self.gate()?;
        let r = req.into_inner();
        let params = LoadParams {
            model_path: r.model_path,
            gpu_layers: r.gpu_layers,
            context: r.context as usize,
            threads: r.threads,
        };
        let (tx, rx) = oneshot::channel();
        self.send(Job::Load(params, tx))?;
        let result = rx
            .await
            .map_err(|_| to_status(&RpcError::new(code::ENGINE, "the engine thread has stopped")))?
            .map_err(|e| to_status(&e))?;
        *self.loaded.lock().unwrap_or_else(|p| p.into_inner()) = Some(result.clone());
        Ok(Response::new(pb::LoadReply {
            backend: result.backend,
            context: result.context as u64,
            name: result.name,
            rss_bytes: result.rss_bytes,
        }))
    }

    type ChatStream = ChatStream;

    async fn chat(&self, req: Request<pb::ChatRequest>) -> Result<Response<Self::ChatStream>, Status> {
        self.gate()?;
        if self.loaded.lock().unwrap_or_else(|p| p.into_inner()).is_none() {
            return Err(to_status(&RpcError::new(code::NOT_LOADED, "no model is loaded")));
        }
        let r = req.into_inner();
        let json_schema = if r.json_schema.trim().is_empty() {
            None
        } else {
            Some(
                serde_json::from_str(&r.json_schema)
                    .map_err(|e| to_status(&RpcError::new(code::INVALID_PARAMS, format!("invalid json_schema: {e}"))))?,
            )
        };
        let params = ChatParams {
            prompt: r.prompt,
            max_tokens: r.max_tokens as usize,
            temperature: r.temperature,
            json_schema,
        };
        let (out, rx) = mpsc::unbounded_channel();
        self.send(Job::Chat {
            id: r.request_id,
            params,
            out,
        })?;
        let stream = UnboundedReceiverStream::new(rx).map(|item| match item {
            ChatItem::Delta(text) => Ok(pb::ChatEvent {
                event: Some(pb::chat_event::Event::Delta(pb::Delta { text })),
            }),
            ChatItem::Done(Ok(r)) => Ok(pb::ChatEvent {
                event: Some(pb::chat_event::Event::Done(pb::Done {
                    text: r.text,
                    prompt_tokens: r.prompt_tokens as u64,
                    completion_tokens: r.completion_tokens as u64,
                    finish: match r.finish {
                        WireFinish::Stop => pb::Finish::Stop,
                        WireFinish::Length => pb::Finish::Length,
                        WireFinish::Cancelled => pb::Finish::Cancelled,
                    } as i32,
                })),
            }),
            ChatItem::Done(Err(e)) => Err(to_status(&e)),
        });
        Ok(Response::new(Box::pin(stream)))
    }

    async fn cancel(&self, req: Request<pb::CancelRequest>) -> Result<Response<pb::Empty>, Status> {
        self.cancel.store(req.into_inner().request_id, Ordering::SeqCst);
        Ok(Response::new(pb::Empty {}))
    }

    async fn unload(&self, _: Request<pb::Empty>) -> Result<Response<pb::Empty>, Status> {
        self.gate()?;
        let (tx, rx) = oneshot::channel();
        self.send(Job::Unload(tx))?;
        let _ = rx.await;
        *self.loaded.lock().unwrap_or_else(|p| p.into_inner()) = None;
        Ok(Response::new(pb::Empty {}))
    }

    async fn status(&self, _: Request<pb::Empty>) -> Result<Response<pb::StatusReply>, Status> {
        self.gate()?;
        let loaded = self.loaded.lock().unwrap_or_else(|p| p.into_inner()).clone();
        Ok(Response::new(pb::StatusReply {
            loaded: loaded.is_some(),
            backend: loaded.map(|l| l.backend).unwrap_or_default(),
            rss_bytes: current_rss_bytes(),
            pid: std::process::id(),
            uptime_secs: self.started.elapsed().as_secs(),
        }))
    }

    async fn shutdown(&self, _: Request<pb::Empty>) -> Result<Response<pb::Empty>, Status> {
        self.shutdown.notify_one();
        Ok(Response::new(pb::Empty {}))
    }
}

fn routes<F>(factory: F, shutdown: Arc<Notify>) -> tonic::service::Routes
where
    F: FnOnce() -> Box<dyn WorkerBackend> + Send + 'static,
{
    let cancel = Arc::new(AtomicU64::new(0));
    let svc = Service {
        jobs: spawn_backend(factory, cancel.clone()),
        cancel,
        loaded: Mutex::new(None),
        said_hello: AtomicBool::new(false),
        started: Instant::now(),
        shutdown,
    };
    let inner = WorkerServer::new(svc)
        .max_decoding_message_size(MAX_MESSAGE_BYTES)
        .max_encoding_message_size(MAX_MESSAGE_BYTES);
    tonic::service::Routes::new(inner)
}

fn options() -> imt_ipc::server::ServeOptions {
    imt_ipc::server::ServeOptions {
        name: format!("imt-ai-worker {}", env!("CARGO_PKG_VERSION")),
        max_message_bytes: MAX_MESSAGE_BYTES,
    }
}

/// Serves `address` (a Unix socket path or a Windows pipe name) until `Shutdown` is called or `stop` completes.
/// Every call must carry `token` (see `imt_ipc::server::serve`).
pub async fn serve<F>(factory: F, address: &str, token: String, stop: impl Future<Output = ()>) -> io::Result<()>
where
    F: FnOnce() -> Box<dyn WorkerBackend> + Send + 'static,
{
    let shutdown = Arc::new(Notify::new());
    let routes = routes(factory, shutdown.clone());
    imt_ipc::server::serve(routes, address, &token, options(), async move {
        tokio::select! {
            _ = shutdown.notified() => {}
            _ = stop => {}
        }
    })
    .await
}

/// The worker's `main`: reads the handshake (address and token) from stdin, serves, and exits when asked to
/// shut down or when stdin closes (the supervisor is gone, so nothing is left behind).
pub fn run_worker<F>(factory: F) -> io::Result<()>
where
    F: FnOnce() -> Box<dyn WorkerBackend> + Send + 'static,
{
    imt_ipc::server::child_main(imt_ipc::server::RuntimeKind::CurrentThread, options(), move |stop| {
        routes(factory, stop.clone())
    })
}
