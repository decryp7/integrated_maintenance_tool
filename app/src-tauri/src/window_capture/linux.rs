//! Linux: WebKitGTK `webkit_web_view_get_snapshot` (ADR 0034). The callback runs on the GTK main thread and only
//! copies the cairo surface's pixels; converting and encoding happen off it.

use super::bound::{Alpha, RawCapture, MAX_RAW_BYTES};
use super::native::ReplyTx;
use super::CaptureError;

/// Starts a snapshot of the visible area of `webview`. Must run on the GTK main thread.
pub(super) fn snapshot(webview: &webkit2gtk::WebView, reply: ReplyTx) -> Result<(), CaptureError> {
    use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};
    webview.snapshot(
        SnapshotRegion::Visible,
        SnapshotOptions::NONE,
        None::<&webkit2gtk::gio::Cancellable>,
        move |result| {
            let _ = reply.try_send(result.map_err(|e| CaptureError::Webview(e.to_string())).and_then(surface_pixels));
        },
    );
    Ok(())
}

/// A copy of a cairo image surface's pixels (B, G, R, A in memory on the little-endian machines IMT runs on).
fn surface_pixels(surface: cairo::Surface) -> Result<RawCapture, CaptureError> {
    let unusable = || CaptureError::Webview("WebKit returned an unusable snapshot".into());
    let image = cairo::ImageSurface::try_from(surface).map_err(|_| unusable())?;
    let alpha = match image.format() {
        cairo::Format::ARgb32 => Alpha::Premultiplied,
        cairo::Format::Rgb24 => Alpha::Ignored,
        _ => return Err(unusable()),
    };
    image.flush();
    let width = u32::try_from(image.width()).map_err(|_| unusable())?;
    let height = u32::try_from(image.height()).map_err(|_| unusable())?;
    let stride = usize::try_from(image.stride()).map_err(|_| unusable())?;
    if stride.saturating_mul(height as usize) > MAX_RAW_BYTES {
        return Err(CaptureError::TooLarge);
    }
    let mut data = Vec::new();
    image.with_data(|d| data = d.to_vec()).map_err(|_| unusable())?;
    Ok(RawCapture::Bgra {
        width,
        height,
        stride,
        alpha,
        data,
    })
}
