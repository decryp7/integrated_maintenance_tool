//! Local gRPC transport shared by IMT's out-of-process workers and plugins (ADR 0002).
//!
//! - [`endpoint`]: a private address (Unix socket in a 0700 directory, owner-only named pipe) and the
//!   per-launch token; never TCP.
//! - [`auth`]: the token on every call (client) and its check (server).
//! - [`error`]: [`IpcError`] for every failed call and the typed [`RemoteError`] trailer.
//! - [`client`] / [`server`]: connecting with retry; serving with the token check and the
//!   [`pb::control_server::Control`] health service; [`server::child_main`] bootstraps a child process.
//! - [`process`]: starting a child (below-normal priority, job object / stdin lifeline, no orphans).
//! - [`supervisor`]: lazy start, restart with backoff, quarantine after repeated failures.
//! - [`child_log`]: each child's captured standard error and start/exit lines, for problem reports.

pub mod auth;
pub mod child_log;
pub mod client;
pub mod endpoint;
pub mod error;
#[cfg(windows)]
mod job;
pub mod process;
pub mod server;
pub mod supervisor;
#[cfg(windows)]
mod winpipe;

/// Generated code of `imt.ipc.v1.Control` (`proto/control.proto`).
#[allow(clippy::all, missing_docs)]
pub mod pb {
    tonic::include_proto!("imt.ipc.v1");
}

/// Generated code of the test-only echo service the fake child of the tests serves.
#[doc(hidden)]
#[allow(clippy::all, missing_docs)]
pub mod testing {
    tonic::include_proto!("imt.ipc.testing");
}

pub use error::{IpcError, RemoteError};

/// Error code of a [`RemoteError`] for a call the child does not implement.
pub const CODE_UNIMPLEMENTED: i32 = -32601;
