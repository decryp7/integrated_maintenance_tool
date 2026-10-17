//! The child side: a gRPC server on a private local address, protected by the per-launch token, that also
//! serves the [`Control`](crate::pb::control_server::Control) health service. [`child_main`] is the whole
//! bootstrap of a child process: read the handshake from stdin, serve, and exit when asked to or when the
//! supervisor goes away (stdin closes), so a child is never left behind.

use crate::auth::RequireToken;
use crate::endpoint::Handshake;
use crate::pb;
use std::future::Future;
use std::io::{self, BufRead, Read};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Notify;
use tonic::service::{InterceptorLayer, Routes};
use tonic::{Request, Response, Status};

/// Resident memory of this process, in bytes (0 if unknown).
pub fn current_rss_bytes() -> u64 {
    let Ok(pid) = sysinfo::get_current_pid() else { return 0 };
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.memory()).unwrap_or(0)
}

/// The control service: status for the supervisor's health check, and a polite shutdown.
pub struct ControlService {
    name: String,
    started: Instant,
    shutdown: Arc<Notify>,
}

impl ControlService {
    pub fn new(name: impl Into<String>, shutdown: Arc<Notify>) -> Self {
        ControlService {
            name: name.into(),
            started: Instant::now(),
            shutdown,
        }
    }
}

#[tonic::async_trait]
impl pb::control_server::Control for ControlService {
    async fn status(&self, _: Request<pb::StatusRequest>) -> Result<Response<pb::StatusReply>, Status> {
        Ok(Response::new(pb::StatusReply {
            pid: std::process::id(),
            rss_bytes: current_rss_bytes(),
            uptime_secs: self.started.elapsed().as_secs(),
            name: self.name.clone(),
        }))
    }

    async fn shutdown(&self, _: Request<pb::ShutdownRequest>) -> Result<Response<pb::ShutdownReply>, Status> {
        self.shutdown.notify_one();
        Ok(Response::new(pb::ShutdownReply {}))
    }
}

/// How a child serves.
pub struct ServeOptions {
    /// Reported by `Control.Status`.
    pub name: String,
    /// Largest message accepted by the control service (the caller sizes its own services).
    pub max_message_bytes: usize,
}

impl Default for ServeOptions {
    fn default() -> Self {
        ServeOptions {
            name: "imt child".into(),
            max_message_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Serves `routes` plus the control service on `address` (a Unix socket path or a Windows pipe name) until
/// `Control.Shutdown` is called or `stop` completes. Every call must carry `token`.
pub async fn serve(routes: Routes, address: &str, token: &str, opts: ServeOptions, stop: impl Future<Output = ()>) -> io::Result<()> {
    let shutdown = Arc::new(Notify::new());
    let control = pb::control_server::ControlServer::new(ControlService::new(opts.name, shutdown.clone()))
        .max_decoding_message_size(opts.max_message_bytes)
        .max_encoding_message_size(opts.max_message_bytes);
    let routes = routes.add_service(control);
    // Stop on `Shutdown` or when `stop` completes: connections get a second to drain, then the server ends
    // even if a client keeps its connection open.
    let (stopping_tx, stopping_rx) = tokio::sync::watch::channel(false);
    let graceful = {
        let mut rx = stopping_rx.clone();
        async move {
            let _ = rx.wait_for(|v| *v).await;
        }
    };
    let hard_stop = {
        let mut rx = stopping_rx;
        async move {
            let _ = rx.wait_for(|v| *v).await;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    };
    let trigger = async move {
        tokio::select! {
            _ = shutdown.notified() => {}
            _ = stop => {}
        }
        let _ = stopping_tx.send(true);
        std::future::pending::<()>().await
    };
    let mut builder = tonic::transport::Server::builder()
        .initial_stream_window_size(Some(crate::client::STREAM_WINDOW))
        .initial_connection_window_size(Some(crate::client::CONNECTION_WINDOW))
        .max_frame_size(Some(crate::client::MAX_FRAME))
        .layer(InterceptorLayer::new(RequireToken::new(token)));
    #[cfg(unix)]
    let running = {
        use std::os::unix::fs::PermissionsExt;
        let listener = tokio::net::UnixListener::bind(address)?;
        std::fs::set_permissions(address, std::fs::Permissions::from_mode(0o600))?;
        let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);
        builder.add_routes(routes).serve_with_incoming_shutdown(incoming, graceful)
    };
    #[cfg(windows)]
    let running = {
        let incoming = crate::winpipe::incoming(address)?;
        builder.add_routes(routes).serve_with_incoming_shutdown(incoming, graceful)
    };
    tokio::select! {
        r = running => r.map_err(io::Error::other),
        _ = hard_stop => Ok(()),
        _ = trigger => Ok(()),
    }
}

/// Reads the handshake line the supervisor writes to stdin (at most 64 KiB).
pub fn read_handshake(input: impl Read) -> io::Result<Handshake> {
    let mut line = String::new();
    io::BufReader::new(input.take(64 * 1024)).read_line(&mut line)?;
    serde_json::from_str(line.trim()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("bad handshake: {e}")))
}

/// Which tokio runtime a child serves on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeKind {
    /// One thread: enough when the real work runs on threads of its own (the inference worker).
    CurrentThread,
    /// A worker pool, for children that handle concurrent calls (the plugin host).
    MultiThread,
}

/// A child process' `main`: reads the handshake (address and token) from stdin, serves the routes `make`
/// builds (on the runtime, so it may spawn tasks), and returns when asked to shut down; when stdin closes
/// (the supervisor is gone) the process exits, so nothing is left behind.
pub fn child_main(kind: RuntimeKind, opts: ServeOptions, make: impl FnOnce(&Arc<Notify>) -> Routes) -> io::Result<()> {
    let hs = read_handshake(io::stdin().lock())?;
    // The lifeline: when the supervisor closes stdin (or dies), exit at once. There is nobody left to
    // answer, and exiting normally (not being killed) lets the process flush what it must.
    std::thread::spawn(move || {
        let mut sink = [0u8; 256];
        while matches!(io::stdin().read(&mut sink), Ok(n) if n > 0) {}
        std::process::exit(0);
    });
    let rt = match kind {
        RuntimeKind::CurrentThread => tokio::runtime::Builder::new_current_thread().enable_all().build()?,
        RuntimeKind::MultiThread => tokio::runtime::Builder::new_multi_thread().enable_all().build()?,
    };
    let stop = Arc::new(Notify::new());
    let routes = {
        let _guard = rt.enter();
        make(&stop)
    };
    rt.block_on(serve(routes, &hs.address, &hs.token, opts, async move { stop.notified().await }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshakes_are_read_from_one_line() {
        let hs = read_handshake(&b"{\"address\":\"a\",\"token\":\"t\"}\nrest"[..]).unwrap();
        assert_eq!((hs.address.as_str(), hs.token.as_str()), ("a", "t"));
        assert_eq!(read_handshake(&b"nope\n"[..]).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert!(read_handshake(&b""[..]).is_err());
        // A line longer than the limit is cut and does not parse.
        let long = format!("{{\"address\":\"{}\",\"token\":\"t\"}}\n", "a".repeat(70_000));
        assert!(read_handshake(long.as_bytes()).is_err());
    }

    #[test]
    fn rss_is_reported() {
        assert!(current_rss_bytes() > 0);
        assert_eq!(ServeOptions::default().max_message_bytes, 16 << 20);
    }
}
