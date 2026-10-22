//! macOS: `WKWebView takeSnapshotWithConfiguration:completionHandler:` (ADR 0034). The completion block runs on the
//! main thread and only converts the image to BMP (an uncompressed copy); decoding and encoding happen off it.

use super::bound::RawCapture;
use super::native::ReplyTx;
use super::CaptureError;

/// Starts a snapshot of `webview` (Tauri's `WKWebView` pointer). Must run on the main thread.
pub(super) fn snapshot(webview: *mut std::ffi::c_void, reply: ReplyTx) -> Result<(), CaptureError> {
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSDictionary, NSError};
    use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
    let mtm = MainThreadMarker::new().ok_or_else(|| CaptureError::Webview("not on the main thread".into()))?;
    // SAFETY: Tauri hands over its live WKWebView, on the main thread `with_webview` runs on.
    let view = unsafe { (webview as *const WKWebView).as_ref() }.ok_or_else(|| CaptureError::Webview("no webview".into()))?;
    // SAFETY: a fresh configuration, created and used on the main thread. Its defaults are what IMT wants: the visible
    // bounds at the view's own size, after pending screen updates (the error UI). `afterScreenUpdates` is not set
    // explicitly: the setter needs macOS 10.15, and Tauri supports older systems.
    let config = unsafe { WKSnapshotConfiguration::new(mtm) };
    let handler = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
        // SAFETY: WebKit passes a valid image or a valid error (or null), alive for the duration of the call.
        let result = match unsafe { (image.as_ref(), error.as_ref()) } {
            (Some(image), _) => image
                .TIFFRepresentation()
                .and_then(|tiff| NSBitmapImageRep::imageRepWithData(&tiff))
                // SAFETY: BMP takes no properties; an empty dictionary means none.
                .and_then(|rep| unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::BMP, &NSDictionary::new()) })
                .map(|bmp| RawCapture::Encoded(bmp.to_vec()))
                .ok_or_else(|| CaptureError::Webview("the snapshot could not be read".into())),
            (None, Some(error)) => Err(CaptureError::Webview(error.localizedDescription().to_string())),
            (None, None) => Err(CaptureError::Webview("WebKit returned no snapshot".into())),
        };
        let _ = reply.try_send(result);
    });
    // SAFETY: called on the main thread with a live configuration; WebKit copies the block and calls it once.
    unsafe { view.takeSnapshotWithConfiguration_completionHandler(Some(&config), &handler) };
    Ok(())
}
