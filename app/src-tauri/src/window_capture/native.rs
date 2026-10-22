//! The platform half of a window capture (ADR 0034): asks the window's own webview for a picture of what it
//! shows. Each platform call is started on the UI thread through `with_webview` and returns at once; the
//! webview answers later through its own completion callback, which only copies the bytes into `reply`.
//! Decoding, scaling and encoding happen off the UI thread (`super::bound`).
//!
//! - Windows: WebView2 `ICoreWebView2::CapturePreview` (PNG into a memory stream).
//! - macOS: `WKWebView takeSnapshotWithConfiguration:completionHandler:` (BMP: no compression on the main thread).
//! - Linux: WebKitGTK `webkit_web_view_get_snapshot` (the visible area as a cairo image surface).

use super::bound::RawCapture;
use super::CaptureError;
use tauri::{Runtime, Webview};

/// What the webview's completion callback sends back.
pub type Reply = Result<RawCapture, CaptureError>;
/// Where the reply goes; one reply per capture (the channel holds one).
pub type ReplyTx = tokio::sync::mpsc::Sender<Reply>;

/// Starts capturing `webview`. Returns once the request is queued for the UI thread; the reply arrives on
/// `reply`. When the platform webview can't be reached, `reply` is dropped unanswered.
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub fn start<R: Runtime>(webview: &Webview<R>, reply: ReplyTx) -> Result<(), CaptureError> {
    webview
        .with_webview(move |platform| {
            let failed = reply.clone();
            if let Err(e) = platform_capture(&platform, reply) {
                let _ = failed.try_send(Err(e));
            }
        })
        .map_err(|e| CaptureError::Webview(e.to_string()))
}

/// Other platforms have no native capture: the snapshot is saved without a picture.
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn start<R: Runtime>(_webview: &Webview<R>, _reply: ReplyTx) -> Result<(), CaptureError> {
    Err(CaptureError::Unsupported)
}

#[cfg(windows)]
fn platform_capture(platform: &tauri::webview::PlatformWebview, reply: ReplyTx) -> Result<(), CaptureError> {
    use webview2_com::CapturePreviewCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG;
    use windows::Win32::UI::Shell::SHCreateMemStream;
    let com = |e: windows::core::Error| CaptureError::Webview(e.message());
    // SAFETY: creates an empty, growable in-memory stream; no pointers are passed.
    let stream = unsafe { SHCreateMemStream(None) }.ok_or_else(|| CaptureError::Webview("no memory for the screenshot".into()))?;
    let written = stream.clone();
    let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
        let _ = reply.try_send(result.map_err(com).and_then(|()| read_stream(&written)).map(RawCapture::Encoded));
        Ok(())
    }));
    // SAFETY: COM calls on the webview's own controller, on the thread `with_webview` runs them on (the UI thread).
    // CapturePreview returns at once; WebView2 encodes in its own process and calls `handler` when done.
    unsafe {
        let core = platform.controller().CoreWebView2().map_err(com)?;
        core.CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &handler)
            .map_err(com)
    }
}

/// The bytes WebView2 wrote into `stream`, at most [`super::bound::MAX_RAW_BYTES`].
#[cfg(windows)]
fn read_stream(stream: &windows::Win32::System::Com::IStream) -> Result<Vec<u8>, CaptureError> {
    use windows::Win32::System::Com::STREAM_SEEK_SET;
    let com = |e: windows::core::Error| CaptureError::Webview(e.message());
    // SAFETY: seeks the stream this capture owns.
    unsafe { stream.Seek(0, STREAM_SEEK_SET, None) }.map_err(com)?;
    let mut out = Vec::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let mut read = 0u32;
        // SAFETY: `buf` is valid for `buf.len()` bytes and `read` outlives the call.
        unsafe { stream.Read(buf.as_mut_ptr().cast(), buf.len() as u32, Some(&mut read)) }
            .ok()
            .map_err(com)?;
        let read = read as usize;
        if read == 0 {
            return Ok(out);
        }
        if out.len() + read > super::bound::MAX_RAW_BYTES {
            return Err(CaptureError::TooLarge);
        }
        out.extend_from_slice(&buf[..read.min(buf.len())]);
    }
}

#[cfg(target_os = "macos")]
fn platform_capture(platform: &tauri::webview::PlatformWebview, reply: ReplyTx) -> Result<(), CaptureError> {
    super::macos::snapshot(platform.inner(), reply)
}

#[cfg(target_os = "linux")]
fn platform_capture(platform: &tauri::webview::PlatformWebview, reply: ReplyTx) -> Result<(), CaptureError> {
    super::linux::snapshot(&platform.inner(), reply)
}
