//! The seam between the supervisor and the wire. The supervisor and the use cases depend only on these
//! traits, so the transport can change (gRPC over a local socket or named pipe today) without touching them.

use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// What went wrong with a call: `Spawn` (the worker could not be started), `Died` (it exited or closed the
/// connection), `Timeout` (silence for the whole timeout, or it ignored a cancel), `Protocol` (not valid
/// protocol), `Remote` (the worker answered with an error; not a failure of the worker). Shared with every
/// IMT child process (`imt_ipc::IpcError`).
pub type TransportError = imt_ipc::IpcError;

#[derive(Debug, Clone, Copy)]
pub struct CallOpts {
    /// Longest silence tolerated: any message from the worker (a streamed delta counts) restarts the clock.
    pub timeout: Duration,
    /// After a cancel is sent, how long the worker may take to stop before it is considered hung.
    pub cancel_grace: Duration,
}

/// One live connection to one worker. Calls are sequential.
pub trait WorkerTransport: Send {
    /// Sends a request and waits for its response. Streamed notifications for the call (`delta` params) go to
    /// `on_stream`. When `cancel` becomes true a cancel is sent once and the call keeps waiting for the
    /// response within `cancel_grace`.
    fn call(
        &mut self,
        method: &str,
        params: Value,
        opts: CallOpts,
        cancel: Option<&AtomicBool>,
        on_stream: &mut dyn FnMut(Value),
    ) -> Result<Value, TransportError>;
    fn pid(&self) -> Option<u32>;
    /// Stops the worker immediately and reaps it.
    fn kill(&mut self);
}

/// Starts workers. Contract: every transport it returns is a fresh process.
pub trait WorkerLauncher: Send + Sync {
    fn launch(&self) -> Result<Box<dyn WorkerTransport>, TransportError>;
}
