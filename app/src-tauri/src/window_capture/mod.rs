//! Native capture of a window's webview for error snapshots (ADR 0034). The UI asks with
//! `capture_window_image`; the shell asks the platform webview for a picture (`native`), waits for its
//! completion callback without blocking any thread, then bounds and encodes it on a blocking worker (`bound`).
//! A capture that can't be taken is not an error: the reply says why, and the snapshot is saved without a
//! picture. The wire contract is `WindowCapture` in `app/src/app/core/api.types.ts`.

pub mod bound;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod native;

use crate::CmdResult;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use bound::Bounded;
use serde::Serialize;
use std::fmt;
use std::time::Duration;

/// How long the webview has to deliver its picture.
pub const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);

/// Why no picture was taken; shown to the user in the snapshot ("No screenshot: …").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    /// This platform has no native webview capture.
    #[cfg_attr(any(windows, target_os = "macos", target_os = "linux"), allow(dead_code))]
    Unsupported,
    /// The window is not on screen (minimised, hidden, on another desktop or covered), so there is nothing to capture.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    NotVisible,
    /// The webview refused or failed.
    Webview(String),
    /// The webview went away without answering.
    NoAnswer,
    /// The webview did not answer within [`CAPTURE_TIMEOUT`].
    TimedOut,
    /// The picture could not be decoded.
    Decode(String),
    /// The picture could not be encoded.
    Encode(String),
    /// Even scaled down, the picture exceeds the limits.
    TooLarge,
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => write!(f, "screenshots are not available on this platform"),
            Self::NotVisible => write!(f, "the window was not on screen (minimised, hidden, covered or on another desktop)"),
            Self::Webview(e) => write!(f, "the window could not be captured: {e}"),
            Self::NoAnswer => write!(f, "the window's webview did not return a picture"),
            Self::TimedOut => write!(f, "the window did not return a picture within {} s", CAPTURE_TIMEOUT.as_secs()),
            Self::Decode(e) => write!(f, "the picture could not be read: {e}"),
            Self::Encode(e) => write!(f, "the picture could not be encoded: {e}"),
            Self::TooLarge => write!(f, "the picture is too large"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// Result of `capture_window_image`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum WindowCapture {
    /// A bounded PNG or JPEG, base64 without a `data:` prefix: what `save_error_snapshot` takes.
    Captured { image: String, mime: String, bytes: u64 },
    /// No picture; `reason` says why.
    Unavailable { reason: String },
}

impl From<Result<Bounded, CaptureError>> for WindowCapture {
    fn from(result: Result<Bounded, CaptureError>) -> Self {
        match result {
            Ok(b) => Self::Captured {
                image: STANDARD.encode(&b.bytes),
                mime: b.format.mime().to_string(),
                bytes: b.bytes.len() as u64,
            },
            Err(e) => Self::Unavailable { reason: e.to_string() },
        }
    }
}

/// A picture of the calling window's webview. Never blocks the UI thread: the platform call returns at once
/// and its completion callback is awaited; encoding runs on a blocking worker.
#[tauri::command]
pub async fn capture_window_image<R: tauri::Runtime>(webview: tauri::Webview<R>) -> CmdResult<WindowCapture> {
    let result = capture(&webview).await;
    match &result {
        // Size only: no image data.
        Ok(b) => log::debug!(Width = b.width, Height = b.height, Bytes = b.bytes.len() as u64; "Window captured"),
        Err(e) => log::info!(Reason = e.to_string().as_str(); "Window capture unavailable"),
    }
    Ok(result.into())
}

async fn capture<R: tauri::Runtime>(webview: &tauri::Webview<R>) -> Result<Bounded, CaptureError> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    native::start(webview, tx)?;
    let raw = match tokio::time::timeout(CAPTURE_TIMEOUT, rx.recv()).await {
        Err(_) => return Err(CaptureError::TimedOut),
        Ok(None) => return Err(CaptureError::NoAnswer),
        Ok(Some(reply)) => reply?,
    };
    tauri::async_runtime::spawn_blocking(move || bound::bound(raw))
        .await
        .map_err(|e| CaptureError::Encode(e.to_string()))?
}

#[cfg(test)]
mod tests;
