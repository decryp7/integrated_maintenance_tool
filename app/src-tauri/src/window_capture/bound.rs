//! The pure half of a window capture: turns what the platform webview handed over (an encoded image or raw
//! pixels) into a bounded PNG or JPEG for an error snapshot. Runs on a blocking worker, never the UI thread.

use super::CaptureError;
use crate::snapshots::{ImageFormat, MAX_IMAGE_BYTES};
use image::{DynamicImage, ImageFormat as Codec, RgbaImage};
use std::io::Cursor;

/// Widest image kept: wider captures (high-DPI screens) are scaled down to this.
pub const MAX_WIDTH: u32 = 1600;
/// A PNG up to this size is kept; a larger one is replaced by a JPEG.
pub const TARGET_BYTES: usize = 2 * 1024 * 1024;
/// Largest encoded image accepted from the platform: room for macOS's uncompressed BMP of a full-screen window on an
/// 8K display (7680 × 4320 px × 4 B ≈ 127 MiB); a PNG from WebView2 is far smaller.
pub const MAX_RAW_BYTES: usize = 160 * 1024 * 1024;
/// Largest side accepted from the platform, in pixels.
pub const MAX_SOURCE_SIDE: u32 = 16_384;
/// Memory the decoder may allocate.
const MAX_DECODE_ALLOC: u64 = 512 * 1024 * 1024;
/// Below this width a capture is not halved further (it is reported as too large instead).
const MIN_WIDTH: u32 = 200;
const JPEG_QUALITY: u8 = 80;

/// How the alpha byte of raw BGRA pixels is to be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub enum Alpha {
    /// Colour channels are premultiplied by alpha (cairo `ARGB32`).
    Premultiplied,
    /// The fourth byte is padding: every pixel is opaque (cairo `RGB24`).
    Ignored,
}

/// What a platform webview returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawCapture {
    /// An encoded image (PNG from WebView2).
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    Encoded(Vec<u8>),
    /// An encoded image drawn at the screen's backing scale (BMP from WebKit on macOS, 2 px per point on Retina),
    /// with the window's width in points: the snapshot is kept at most that wide, so a Retina capture costs what
    /// the same window does on a standard screen.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Scaled { encoded: Vec<u8>, logical_width: u32 },
    /// Native-endian 32-bit pixels, B, G, R, A in memory on little-endian machines (WebKitGTK's cairo surface).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Bgra {
        width: u32,
        height: u32,
        stride: usize,
        alpha: Alpha,
        data: Vec<u8>,
    },
}

/// A capture ready to be saved as a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bounded {
    pub format: ImageFormat,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Decodes `raw`, scales it to at most [`MAX_WIDTH`] and encodes it as PNG, or as JPEG when the PNG is larger
/// than [`TARGET_BYTES`]; halves the width until the result fits in the snapshot limit ([`MAX_IMAGE_BYTES`]).
pub fn bound(raw: RawCapture) -> Result<Bounded, CaptureError> {
    let mut max_width = MAX_WIDTH;
    let image = match raw {
        RawCapture::Encoded(bytes) => decode(&bytes)?,
        RawCapture::Scaled { encoded, logical_width } => {
            max_width = max_width.min(logical_width.max(1));
            decode(&encoded)?
        }
        RawCapture::Bgra {
            width,
            height,
            stride,
            alpha,
            data,
        } => rgba_from_bgra(width, height, stride, alpha, &data)?,
    };
    let mut width = image.width().min(max_width);
    loop {
        let scaled = scaled_to(&image, width);
        let png = encode_png(&scaled)?;
        if png.len() <= TARGET_BYTES {
            return Ok(done(ImageFormat::Png, png, &scaled));
        }
        let jpeg = encode_jpeg(&scaled)?;
        if jpeg.len() <= MAX_IMAGE_BYTES {
            return Ok(done(ImageFormat::Jpeg, jpeg, &scaled));
        }
        if width <= MIN_WIDTH {
            return Err(CaptureError::TooLarge);
        }
        width = (width / 2).max(MIN_WIDTH);
    }
}

fn done(format: ImageFormat, bytes: Vec<u8>, image: &RgbaImage) -> Bounded {
    Bounded {
        format,
        bytes,
        width: image.width(),
        height: image.height(),
    }
}

/// Decodes a PNG, BMP or JPEG within the size and memory limits.
fn decode(bytes: &[u8]) -> Result<RgbaImage, CaptureError> {
    if bytes.len() > MAX_RAW_BYTES {
        return Err(CaptureError::TooLarge);
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| CaptureError::Decode(e.to_string()))?;
    if !matches!(reader.format(), Some(Codec::Png | Codec::Bmp | Codec::Jpeg)) {
        return Err(CaptureError::Decode("the webview returned an unknown image format".into()));
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| CaptureError::Decode(e.to_string()))?;
    Ok(image.to_rgba8())
}

/// A window width in points as reported by the platform (`NSImage.size.width`), or `None` when it is not a usable
/// width (not finite, not positive, or wider than any capture is accepted).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn logical_width(points: f64) -> Option<u32> {
    let rounded = points.round();
    (rounded.is_finite() && rounded >= 1.0 && rounded <= f64::from(MAX_SOURCE_SIDE)).then_some(rounded as u32)
}

/// Straight RGBA from 32-bit BGRA rows (`stride` bytes apart), un-premultiplying when asked.
pub fn rgba_from_bgra(width: u32, height: u32, stride: usize, alpha: Alpha, data: &[u8]) -> Result<RgbaImage, CaptureError> {
    let bad = |why: &str| CaptureError::Decode(format!("the webview returned unusable pixels ({why})"));
    if width == 0 || height == 0 {
        return Err(bad("empty"));
    }
    if width > MAX_SOURCE_SIDE || height > MAX_SOURCE_SIDE {
        return Err(CaptureError::TooLarge);
    }
    let row = width as usize * 4;
    if stride < row {
        return Err(bad("rows overlap"));
    }
    // Saturating: a bogus stride from the platform must be refused, not overflow.
    let needed = stride.saturating_mul(height as usize - 1).saturating_add(row);
    if data.len() < needed {
        return Err(bad("too few bytes"));
    }
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        let (pixels, _) = data[y * stride..y * stride + row].as_chunks::<4>();
        for &[b, g, r, a] in pixels {
            match alpha {
                Alpha::Ignored => out.extend_from_slice(&[r, g, b, 255]),
                Alpha::Premultiplied if a == 0 => out.extend_from_slice(&[0, 0, 0, 0]),
                Alpha::Premultiplied => out.extend_from_slice(&[unpremultiply(r, a), unpremultiply(g, a), unpremultiply(b, a), a]),
            }
        }
    }
    RgbaImage::from_raw(width, height, out).ok_or_else(|| bad("size mismatch"))
}

fn unpremultiply(c: u8, a: u8) -> u8 {
    let v = (u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a);
    v.min(255) as u8
}

/// `image` scaled to `width` (aspect kept); never enlarged.
fn scaled_to(image: &RgbaImage, width: u32) -> RgbaImage {
    if width >= image.width() {
        return image.clone();
    }
    let height = ((u64::from(image.height()) * u64::from(width)) / u64::from(image.width())).max(1) as u32;
    image::imageops::resize(image, width, height, image::imageops::FilterType::Triangle)
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, CaptureError> {
    let mut out = Cursor::new(Vec::new());
    image
        .write_to(&mut out, Codec::Png)
        .map_err(|e| CaptureError::Encode(e.to_string()))?;
    Ok(out.into_inner())
}

/// JPEG has no alpha: the pixels are flattened to RGB first.
fn encode_jpeg(image: &RgbaImage) -> Result<Vec<u8>, CaptureError> {
    let rgb = DynamicImage::ImageRgba8(image.clone()).to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|e| CaptureError::Encode(e.to_string()))?;
    Ok(out)
}
