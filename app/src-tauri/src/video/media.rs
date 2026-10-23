//! The `media:` URI scheme the Video view's player loads videos from (ADR 0033 §7). A request names a video by its
//! key (`media://localhost/<key>`); it is served only when that video belongs to the calling window's project, from
//! its included copy or the file it was opened from, in bounded `Range` chunks. Nothing else is ever served: no
//! other path, no remote URL, no directory listing.

use super::commands::playable;
use crate::windows::CurrentWindow;
use debrief_video::media::{answer, content_type, RangeAnswer};
use std::io::{Read, Seek, SeekFrom};
use tauri::http::{header, Request, Response, StatusCode};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

/// The scheme's name.
pub(crate) const SCHEME: &str = "media";

fn reply(status: StatusCode, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(body)
        .unwrap_or_default()
}

/// The response to a request of window `label`'s webview. Blocking (reads the file).
fn serve<R: Runtime, M: Manager<R>>(manager: &M, label: &str, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    if request.method() != tauri::http::Method::GET {
        return reply(StatusCode::METHOD_NOT_ALLOWED, Vec::new());
    }
    let key = request.uri().path().trim_start_matches('/');
    let Ok(win) = CurrentWindow::of(manager, label) else {
        return reply(StatusCode::NOT_FOUND, Vec::new());
    };
    let Some((path, name)) = playable(win.arc(), key) else {
        return reply(StatusCode::NOT_FOUND, Vec::new());
    };
    let Some(mime) = content_type(&name) else {
        return reply(StatusCode::UNSUPPORTED_MEDIA_TYPE, Vec::new());
    };
    let Ok(mut file) = std::fs::File::open(&path) else {
        return reply(StatusCode::NOT_FOUND, Vec::new());
    };
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        return reply(StatusCode::NOT_FOUND, Vec::new());
    };
    let range = request.headers().get(header::RANGE).and_then(|v| v.to_str().ok());
    let (start, end) = match answer(range, len) {
        RangeAnswer::Whole { end } => (0, end),
        RangeAnswer::Part { start, end } => (start, end),
        RangeAnswer::Unsatisfiable => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{len}"))
                .body(Vec::new())
                .unwrap_or_default()
        }
    };
    let mut body = vec![0u8; (end - start + 1) as usize];
    if file.seek(SeekFrom::Start(start)).and_then(|_| file.read_exact(&mut body)).is_err() {
        return reply(StatusCode::INTERNAL_SERVER_ERROR, Vec::new());
    }
    Response::builder()
        .status(StatusCode::PARTIAL_CONTENT)
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))
        .header(header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(body)
        .unwrap_or_default()
}

/// The handler to register with `register_asynchronous_uri_scheme_protocol(SCHEME, handler)`: serves on the blocking
/// pool, never on the UI thread.
pub(crate) fn handler<R: Runtime>(ctx: UriSchemeContext<'_, R>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let app = ctx.app_handle().clone();
    let label = ctx.webview_label().to_string();
    tauri::async_runtime::spawn_blocking(move || responder.respond(serve(&app, &label, &request)));
}

#[cfg(test)]
pub(super) fn serve_for_tests<R: Runtime, M: Manager<R>>(manager: &M, label: &str, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    serve(manager, label, request)
}
