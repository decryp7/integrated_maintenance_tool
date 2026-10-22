use super::bound::{bound, logical_width, rgba_from_bgra, Alpha, RawCapture, MAX_RAW_BYTES, MAX_SOURCE_SIDE, MAX_WIDTH, TARGET_BYTES};
use super::*;
use crate::snapshots::{validate_image, ImageFormat, MAX_IMAGE_BYTES};
use image::{ImageFormat as Codec, Rgba, RgbaImage};
use std::io::Cursor;

fn encoded(image: &RgbaImage, codec: Codec) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image.clone())
        .to_rgb8()
        .write_to(&mut out, codec)
        .unwrap();
    out.into_inner()
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    RgbaImage::from_pixel(width, height, Rgba([30, 60, 90, 255]))
        .write_to(&mut out, Codec::Png)
        .unwrap();
    out.into_inner()
}

/// Pseudo-random pixels: compress badly, so the PNG is large.
fn noise(width: u32, height: u32) -> RgbaImage {
    let mut state = 0x2545_f491_u32;
    RgbaImage::from_fn(width, height, |_, _| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let [a, b, c, _] = state.to_le_bytes();
        Rgba([a, b, c, 255])
    })
}

#[test]
fn a_small_capture_stays_a_png_of_the_same_size() {
    let b = bound(RawCapture::Encoded(png(640, 400))).unwrap();
    assert_eq!((b.format, b.width, b.height), (ImageFormat::Png, 640, 400));
    assert!(b.bytes.len() <= TARGET_BYTES);
    validate_image("image/png", &STANDARD.encode(&b.bytes)).expect("the snapshot store accepts it");
}

#[test]
fn a_wide_capture_is_scaled_to_the_maximum_width_keeping_its_aspect() {
    let b = bound(RawCapture::Encoded(png(3840, 2160))).unwrap();
    assert_eq!((b.width, b.height), (MAX_WIDTH, 900));
}

#[test]
fn bmp_and_jpeg_from_the_platform_are_accepted_too() {
    let image = RgbaImage::from_pixel(64, 32, Rgba([200, 10, 10, 255]));
    for codec in [Codec::Bmp, Codec::Jpeg] {
        let b = bound(RawCapture::Encoded(encoded(&image, codec))).unwrap();
        assert_eq!((b.format, b.width, b.height), (ImageFormat::Png, 64, 32), "{codec:?}");
    }
}

#[test]
fn a_capture_that_compresses_badly_becomes_a_jpeg_within_the_snapshot_limit() {
    let mut raw = Cursor::new(Vec::new());
    noise(1400, 900).write_to(&mut raw, Codec::Png).unwrap();
    let b = bound(RawCapture::Encoded(raw.into_inner())).unwrap();
    assert_eq!(b.format, ImageFormat::Jpeg);
    assert!(b.bytes.len() <= MAX_IMAGE_BYTES);
    validate_image("image/jpeg", &STANDARD.encode(&b.bytes)).expect("the snapshot store accepts it");
}

#[test]
fn a_retina_capture_is_kept_at_the_window_s_logical_width() {
    // A 1440 × 900 pt window drawn at 2 px per point, as WebKit on macOS hands it over (BMP).
    let image = RgbaImage::from_pixel(2880, 1800, Rgba([30, 60, 90, 255]));
    let b = bound(RawCapture::Scaled {
        encoded: encoded(&image, Codec::Bmp),
        logical_width: 1440,
    })
    .unwrap();
    assert_eq!((b.format, b.width, b.height), (ImageFormat::Png, 1440, 900));
    // A window wider in points than the maximum is still bounded by it.
    let wide = RgbaImage::from_pixel(4000, 1000, Rgba([0, 0, 0, 255]));
    let b = bound(RawCapture::Scaled {
        encoded: encoded(&wide, Codec::Bmp),
        logical_width: 2000,
    })
    .unwrap();
    assert_eq!((b.width, b.height), (MAX_WIDTH, 400));
    // A logical width above the pixel width (a scale below 1) never enlarges; zero is treated as one point.
    for logical_width in [5000, 0] {
        let b = bound(RawCapture::Scaled {
            encoded: png(300, 100),
            logical_width,
        })
        .unwrap();
        let expected = if logical_width == 0 { (1, 1) } else { (300, 100) };
        assert_eq!((b.width, b.height), expected, "{logical_width}");
    }
    assert!(matches!(
        bound(RawCapture::Scaled {
            encoded: b"junk".to_vec(),
            logical_width: 100
        }),
        Err(CaptureError::Decode(_))
    ));
}

#[test]
fn only_usable_point_widths_become_logical_widths() {
    assert_eq!(logical_width(1440.0), Some(1440));
    assert_eq!(logical_width(1439.6), Some(1440));
    assert_eq!(logical_width(1.0), Some(1));
    for unusable in [0.0, 0.4, -10.0, f64::NAN, f64::INFINITY, f64::from(MAX_SOURCE_SIDE) + 1.0] {
        assert_eq!(logical_width(unusable), None, "{unusable}");
    }
}

#[test]
fn garbage_unknown_formats_and_oversized_input_are_errors_not_panics() {
    assert!(matches!(bound(RawCapture::Encoded(vec![])), Err(CaptureError::Decode(_))));
    assert!(matches!(
        bound(RawCapture::Encoded(b"not an image at all".to_vec())),
        Err(CaptureError::Decode(_))
    ));
    let gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;".to_vec();
    assert!(
        matches!(bound(RawCapture::Encoded(gif)), Err(CaptureError::Decode(_))),
        "only PNG, BMP and JPEG"
    );
    let mut truncated = png(100, 100);
    truncated.truncate(truncated.len() / 2);
    assert!(matches!(bound(RawCapture::Encoded(truncated)), Err(CaptureError::Decode(_))));
    assert_eq!(bound(RawCapture::Encoded(vec![0; MAX_RAW_BYTES + 1])), Err(CaptureError::TooLarge));
}

#[test]
fn a_png_claiming_huge_dimensions_is_refused_before_decoding() {
    let mut raw = png(1, 1);
    // IHDR width/height live at bytes 16..24; claim 20000 × 20000 (the CRC no longer matches either way).
    raw[16..20].copy_from_slice(&20_000u32.to_be_bytes());
    raw[20..24].copy_from_slice(&20_000u32.to_be_bytes());
    assert!(bound(RawCapture::Encoded(raw)).is_err());
}

#[test]
fn bgra_pixels_are_reordered_and_unpremultiplied() {
    // Two pixels per row, a padded stride of 12 bytes: opaque blue, half-transparent premultiplied red (128 → 255).
    let row = [255, 0, 0, 255, 0, 0, 128, 128, 9, 9, 9, 9];
    let data = [row, row].concat();
    let image = rgba_from_bgra(2, 2, 12, Alpha::Premultiplied, &data).unwrap();
    assert_eq!(image.get_pixel(0, 1).0, [0, 0, 255, 255]);
    assert_eq!(image.get_pixel(1, 0).0, [255, 0, 0, 128]);
    let clear = rgba_from_bgra(1, 1, 4, Alpha::Premultiplied, &[7, 7, 7, 0]).unwrap();
    assert_eq!(clear.get_pixel(0, 0).0, [0, 0, 0, 0]);
    let opaque = rgba_from_bgra(1, 1, 4, Alpha::Ignored, &[1, 2, 3, 0]).unwrap();
    assert_eq!(opaque.get_pixel(0, 0).0, [3, 2, 1, 255], "RGB24 padding is not alpha");
    let b = bound(RawCapture::Bgra {
        width: 2,
        height: 2,
        stride: 12,
        alpha: Alpha::Premultiplied,
        data,
    })
    .unwrap();
    assert_eq!((b.format, b.width, b.height), (ImageFormat::Png, 2, 2));
}

#[test]
fn malformed_bgra_buffers_are_rejected() {
    let decode = |r: Result<RgbaImage, CaptureError>| matches!(r, Err(CaptureError::Decode(_)));
    assert!(decode(rgba_from_bgra(0, 1, 4, Alpha::Ignored, &[0; 4])), "empty");
    assert!(
        decode(rgba_from_bgra(2, 1, 4, Alpha::Ignored, &[0; 8])),
        "stride shorter than a row"
    );
    assert!(decode(rgba_from_bgra(2, 2, 8, Alpha::Ignored, &[0; 12])), "too few bytes");
    assert!(
        decode(rgba_from_bgra(2, 3, usize::MAX, Alpha::Ignored, &[0; 64])),
        "a stride that would overflow"
    );
    assert_eq!(
        rgba_from_bgra(MAX_SOURCE_SIDE + 1, 1, 0, Alpha::Ignored, &[]),
        Err(CaptureError::TooLarge)
    );
}

#[test]
fn the_reply_carries_the_image_or_why_there_is_none() {
    let ok = WindowCapture::from(bound(RawCapture::Encoded(png(8, 8))));
    let WindowCapture::Captured { image, mime, bytes } = &ok else {
        panic!("{ok:?}")
    };
    assert_eq!(mime, "image/png");
    assert_eq!(STANDARD.decode(image).unwrap().len() as u64, *bytes);
    let json = serde_json::to_value(&ok).unwrap();
    assert_eq!(json["status"], "captured");
    let none = serde_json::to_value(WindowCapture::from(Err(CaptureError::Unsupported))).unwrap();
    assert_eq!(
        none,
        serde_json::json!({ "status": "unavailable", "reason": "screenshots are not available on this platform" })
    );
    for e in [
        CaptureError::Webview("E_FAIL".into()),
        CaptureError::NotVisible,
        CaptureError::NoAnswer,
        CaptureError::TimedOut,
        CaptureError::Decode("x".into()),
        CaptureError::Encode("x".into()),
        CaptureError::TooLarge,
    ] {
        assert!(!e.to_string().is_empty());
        assert!(matches!(WindowCapture::from(Err(e)), WindowCapture::Unavailable { .. }));
    }
}

#[test]
fn without_a_platform_webview_the_command_answers_unavailable_at_once() {
    // The mock runtime has no platform webview: `with_webview` drops the request, so nobody answers.
    use crate::test_support::Now;
    use tauri::Manager;
    let app = crate::test_support::test_app(None);
    tauri::WebviewWindowBuilder::new(&app, "w-capture", tauri::WebviewUrl::default())
        .build()
        .unwrap();
    let webview = app.get_webview("w-capture").unwrap();
    let started = std::time::Instant::now();
    let reply = capture_window_image(webview, None).now().unwrap();
    assert_eq!(
        reply,
        WindowCapture::Unavailable {
            reason: CaptureError::NoAnswer.to_string()
        }
    );
    assert!(started.elapsed() < CAPTURE_TIMEOUT, "no waiting for the timeout");
}

#[test]
fn a_refused_capture_is_retried_once_only_for_a_user_request_of_the_apps_own_window() {
    let refused = CaptureError::Webview("An unknown error occurred".into());
    assert!(should_raise_and_retry(&refused, true, true, false));
    assert!(
        !should_raise_and_retry(&refused, false, true, false),
        "automatic snapshots never move windows"
    );
    assert!(!should_raise_and_retry(&refused, true, false, false), "not a docked browser's page");
    assert!(!should_raise_and_retry(&refused, true, true, true), "only once");
    for other in [
        CaptureError::NotVisible,
        CaptureError::NoAnswer,
        CaptureError::TimedOut,
        CaptureError::Unsupported,
        CaptureError::Decode("x".into()),
        CaptureError::Encode("x".into()),
        CaptureError::TooLarge,
    ] {
        assert!(!should_raise_and_retry(&other, true, true, false), "{other:?} is not a refusal");
    }
}
