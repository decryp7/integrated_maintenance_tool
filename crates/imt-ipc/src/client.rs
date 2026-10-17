//! Connecting to a child's private address (Unix socket or named pipe; never TCP).

use crate::auth::AttachToken;
use crate::IpcError;
use std::time::{Duration, Instant};
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;

/// HTTP/2 flow-control windows. The defaults (64 KiB) throttle bulk transfers (file content, record
/// batches) to a fraction of what a local socket carries; these let a whole batch be in flight.
pub const STREAM_WINDOW: u32 = 16 * 1024 * 1024;
pub const CONNECTION_WINDOW: u32 = 64 * 1024 * 1024;
/// HTTP/2 frame size: 1 MiB frames instead of the default 16 KiB cut per-frame costs on bulk data.
pub const MAX_FRAME: u32 = 1024 * 1024;

/// A channel whose every call carries the per-launch token.
pub type AuthChannel = InterceptedService<Channel, AttachToken>;

/// Opens a channel to `address` (one attempt).
pub async fn connect_channel(address: String) -> Result<Channel, tonic::transport::Error> {
    let endpoint = tonic::transport::Endpoint::try_from("http://localhost")?
        .connect_timeout(Duration::from_secs(5))
        .initial_stream_window_size(Some(STREAM_WINDOW))
        .initial_connection_window_size(Some(CONNECTION_WINDOW))
        .max_frame_size(Some(MAX_FRAME));
    #[cfg(unix)]
    {
        endpoint
            .connect_with_connector(tower::service_fn(move |_: tonic::codegen::http::Uri| {
                let a = address.clone();
                async move { Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(tokio::net::UnixStream::connect(a).await?)) }
            }))
            .await
    }
    #[cfg(windows)]
    {
        endpoint
            .connect_with_connector(tower::service_fn(move |_: tonic::codegen::http::Uri| {
                let a = address.clone();
                async move { Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(crate::winpipe::connect(&a).await?)) }
            }))
            .await
    }
}

/// `channel` with `token` attached to every call.
pub fn authenticated(channel: Channel, token: &str) -> Result<AuthChannel, IpcError> {
    Ok(InterceptedService::new(channel, AttachToken::new(token)?))
}

/// Connects to a child that binds its address a moment after starting: retries every 20 ms for up to
/// `window`. `exited` is asked between attempts and stops the wait with its description when the child
/// has died.
pub async fn connect_with_retry(address: &str, window: Duration, mut exited: impl FnMut() -> Option<String>) -> Result<Channel, IpcError> {
    let started = Instant::now();
    loop {
        match connect_channel(address.to_string()).await {
            Ok(c) => return Ok(c),
            Err(e) => {
                if let Some(why) = exited() {
                    return Err(IpcError::Died(why));
                }
                if started.elapsed() > window {
                    return Err(IpcError::Died(format!("it did not start listening ({e})")));
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
}
