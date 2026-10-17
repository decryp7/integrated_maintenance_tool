//! Inference runs in a separate worker process so a crash, hang or out-of-memory in native code cannot take
//! IMT down. The wire is gRPC (tonic + prost) on a local socket only: a Unix domain socket in a private
//! 0700 directory, or a named pipe only the current user can open; every call carries a random per-launch
//! token. `WorkerTransport` is the seam the supervisor depends on. The transport plumbing (address, token,
//! process start, connect and serve) is shared with the plugin host in `imt_ipc`.

pub mod backend;
pub mod endpoint;
pub mod grpc_client;
pub mod grpc_server;
pub mod pb;
pub mod protocol;
pub mod supervisor;
pub mod transport;

pub use backend::{current_rss_bytes, WorkerBackend};
pub use grpc_client::{default_worker_path, worker_file_name, GrpcLauncher, GrpcTransport};
pub use grpc_server::run_worker;
pub use supervisor::{EngineState, EngineStatus, Health, Supervisor, WorkerConfig, WorkerEngine, WorkerRuntime};
pub use transport::{CallOpts, TransportError, WorkerLauncher, WorkerTransport};
