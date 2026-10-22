//! IMT's one IPC (ADR 0017), shared by its workers and plugin processes.
//!
//! - [`frame`]: the frame codec (length-prefixed postcard; bounded, never panicking); [`wire`]: message bodies.
//! - [`peer`]: a connection after the handshake: calls both ways, streams with credits, cancel, typed errors.
//! - [`link`]: the host listens (private endpoint, per-launch token), the child connects; peer pids are checked.
//! - [`endpoint`]: a private address (Unix socket in a 0700 directory, owner-only named pipe); never TCP.
//! - [`launch`] / [`child`]: the one launcher and the one child loop; [`process`]: starting a child (no console,
//!   below-normal priority, job object / stdin lifeline, cleared environment).
//! - [`sandbox`]: what a child may do, applied by the launcher and by the child itself; [`elevation`]: children
//!   that need administrator rights.
//! - [`supervisor`]: lazy start, restart with backoff and a restore hook, quarantine after repeated failures.
//! - [`child_log`]: each child's captured standard error and start/exit lines, for problem reports.

pub mod child;
pub mod child_log;
#[cfg(windows)]
pub mod contain;
pub mod elevation;
pub mod endpoint;
pub mod error;
pub mod frame;
#[cfg(windows)]
mod job;
pub mod launch;
pub mod link;
pub mod memory_watch;
pub mod peer;
pub mod process;
pub mod runtime;
pub mod sandbox;
pub mod supervisor;
pub mod wire;

pub use error::{IpcError, RemoteError};
pub use launch::{launch, Bootstrap, Child, ChildSpec};
pub use peer::{CallCtx, CallStream, CancelProbe, Handler, Peer, PeerOptions, StreamEvent};
pub use sandbox::{Containment, FsAccess, SandboxSpec};
pub use wire::Json;

/// The message of a panic payload (`&str` or `String`), for errors that report a contained panic.
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".into()
    }
}
