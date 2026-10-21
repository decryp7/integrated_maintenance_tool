//! Untrusted images, made safe for the model and the webview (ADR 0025): the bytes are decoded here, in Rust,
//! within limits (file size, claimed dimensions, decoder allocations), as PNG, JPEG, BMP, GIF (first frame) or
//! WebP only; flattened onto white, downscaled to what the model reads, and re-encoded from the pixels (PNG, or
//! JPEG when that is much smaller), so no metadata, colour profile, extra frame or trailing data survives. The
//! capture time in the EXIF data is the one fact kept. Decoders are third-party code: a panic in one is caught
//! and reported as a damaged image.

use super::exif::{capture_time, ExifTime};
use crate::llm::{ImageInput, ImageMime};
use ::image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, RgbImage};
use std::io::Cursor;
use thiserror::Error;

/// Bounds on what is decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageLimits {
    /// Largest file accepted.
    pub max_file_bytes: usize,
    /// Most pixels a file may claim (the decompression-bomb guard: checked before any pixel is decoded).
    pub max_pixels: u64,
    /// Longest side a file may claim.
    pub max_side: u32,
    /// Most memory the decoder may allocate.
    pub max_alloc: u64,
}

impl Default for ImageLimits {
    fn default() -> Self {
        ImageLimits {
            max_file_bytes: 32 * 1024 * 1024,
            max_pixels: 64 * 1024 * 1024,
            max_side: 16_384,
            max_alloc: 512 * 1024 * 1024,
        }
    }
}

/// Default pixel budget of the copy sent to the model when its projector does not say (`ProjectorInfo::max_image_pixels`).
pub const DEFAULT_TARGET_PIXELS: u64 = 1024 * 1024;
/// Longest side of the copy, whatever the pixel budget (a very long, thin image is cut down too).
pub const MAX_OUTPUT_SIDE: u32 = 4096;
/// A PNG larger than this is replaced by a JPEG when that is at least a third smaller (photos).
const PNG_SOFT_LIMIT: usize = 1536 * 1024;
const JPEG_QUALITY: u8 = 88;

/// The formats accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceFormat {
    Png,
    Jpeg,
    Bmp,
    Gif,
    Webp,
}

impl SourceFormat {
    /// The format of `bytes` by its signature (never by a file name).
    pub fn sniff(bytes: &[u8]) -> Option<SourceFormat> {
        match bytes {
            [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, ..] => Some(SourceFormat::Png),
            [0xff, 0xd8, 0xff, ..] => Some(SourceFormat::Jpeg),
            [b'B', b'M', ..] => Some(SourceFormat::Bmp),
            [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Some(SourceFormat::Gif),
            [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(SourceFormat::Webp),
            _ => None,
        }
    }

    fn image_format(self) -> ImageFormat {
        match self {
            SourceFormat::Png => ImageFormat::Png,
            SourceFormat::Jpeg => ImageFormat::Jpeg,
            SourceFormat::Bmp => ImageFormat::Bmp,
            SourceFormat::Gif => ImageFormat::Gif,
            SourceFormat::Webp => ImageFormat::WebP,
        }
    }

    /// The file extensions IMT opens as images (lower case).
    pub const EXTENSIONS: &'static [&'static str] = &["png", "jpg", "jpeg", "bmp", "gif", "webp"];
}

/// Why an image was refused. Messages say what to do; they never contain the image's content.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ImageError {
    #[error("the image is empty")]
    Empty,
    #[error("the image is {bytes} bytes; images up to {max} bytes are accepted")]
    TooLarge { bytes: usize, max: usize },
    #[error("this is not a PNG, JPEG, BMP, GIF or WebP image")]
    Unsupported,
    #[error("the image claims {width}×{height} pixels, more than IMT decodes (at most {max_pixels} pixels, {max_side} per side)")]
    TooManyPixels {
        width: u32,
        height: u32,
        max_pixels: u64,
        max_side: u32,
    },
    #[error("the image cannot be read: {0}")]
    Damaged(String),
}

/// A decoded, bounded, re-encoded image and what IMT kept from the original.
#[derive(Debug, Clone, PartialEq)]
pub struct SanitizedImage {
    /// The clean copy: what the model gets and the webview shows.
    pub image: ImageInput,
    pub source_format: SourceFormat,
    pub original_width: u32,
    pub original_height: u32,
    /// When it was taken, from its EXIF data.
    pub capture: Option<ExifTime>,
}

fn damaged(e: impl std::fmt::Display) -> ImageError {
    let mut m = e.to_string();
    m.truncate(200);
    ImageError::Damaged(m)
}

/// Decodes untrusted `bytes` within `limits` and returns the clean copy, downscaled to at most `target_pixels`
/// (and [`MAX_OUTPUT_SIDE`] per side). Never panics.
pub fn sanitize(bytes: &[u8], target_pixels: u64, limits: &ImageLimits) -> Result<SanitizedImage, ImageError> {
    if bytes.is_empty() {
        return Err(ImageError::Empty);
    }
    if bytes.len() > limits.max_file_bytes {
        return Err(ImageError::TooLarge {
            bytes: bytes.len(),
            max: limits.max_file_bytes,
        });
    }
    let format = SourceFormat::sniff(bytes).ok_or(ImageError::Unsupported)?;
    let decoded = std::panic::catch_unwind(|| decode(bytes, format, limits)).unwrap_or_else(|_| Err(damaged("the decoder failed")))?;
    let (pixels, capture) = decoded;
    let (original_width, original_height) = (pixels.width(), pixels.height());
    let rgb = downscale(flatten(pixels), target_pixels.max(1));
    let image = std::panic::catch_unwind(|| encode(&rgb)).unwrap_or_else(|_| Err(damaged("the encoder failed")))?;
    Ok(SanitizedImage {
        image,
        source_format: format,
        original_width,
        original_height,
        capture,
    })
}

fn decode(bytes: &[u8], format: SourceFormat, limits: &ImageLimits) -> Result<(DynamicImage, Option<ExifTime>), ImageError> {
    let mut lim = Limits::default();
    lim.max_image_width = Some(limits.max_side);
    lim.max_image_height = Some(limits.max_side);
    lim.max_alloc = Some(limits.max_alloc);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format.image_format());
    // The header is read without limits so a bomb's claimed size is reported as such (no pixel is decoded yet).
    reader.no_limits();
    let mut decoder = reader.into_decoder().map_err(damaged)?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err(damaged("it has no pixels"));
    }
    if u64::from(width) * u64::from(height) > limits.max_pixels || width > limits.max_side || height > limits.max_side {
        return Err(ImageError::TooManyPixels {
            width,
            height,
            max_pixels: limits.max_pixels,
            max_side: limits.max_side,
        });
    }
    decoder.set_limits(lim).map_err(|_| ImageError::TooManyPixels {
        width,
        height,
        max_pixels: limits.max_pixels,
        max_side: limits.max_side,
    })?;
    // Only the capture time is kept; the rest of the EXIF data is dropped with the original encoding.
    let capture = decoder.exif_metadata().ok().flatten().and_then(|raw| capture_time(&raw));
    let pixels = DynamicImage::from_decoder(decoder).map_err(damaged)?;
    Ok((pixels, capture))
}

/// RGB on a white background (transparent screenshots and icons stay readable).
fn flatten(img: DynamicImage) -> RgbImage {
    if !img.color().has_alpha() {
        return img.into_rgb8();
    }
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    let mut out = RgbImage::new(w, h);
    for (dst, src) in out.pixels_mut().zip(rgba.pixels()) {
        let a = u32::from(src[3]);
        for c in 0..3 {
            dst[c] = ((u32::from(src[c]) * a + 255 * (255 - a) + 127) / 255) as u8;
        }
    }
    out
}

/// The size an image of `w`×`h` is scaled to: within `target` pixels and [`MAX_OUTPUT_SIDE`], aspect kept,
/// never enlarged. Pure.
pub fn fitted_size(w: u32, h: u32, target: u64) -> (u32, u32) {
    let (w64, h64) = (u64::from(w.max(1)), u64::from(h.max(1)));
    let by_pixels = if w64 * h64 > target {
        (target as f64 / (w64 * h64) as f64).sqrt()
    } else {
        1.0
    };
    let by_side = f64::from(MAX_OUTPUT_SIDE) / f64::from(w.max(h).max(1));
    let scale = by_pixels.min(by_side).min(1.0);
    let nw = ((f64::from(w.max(1)) * scale).floor() as u32).clamp(1, MAX_OUTPUT_SIDE);
    let nh = ((f64::from(h.max(1)) * scale).floor() as u32).clamp(1, MAX_OUTPUT_SIDE);
    (nw, nh)
}

fn downscale(rgb: RgbImage, target: u64) -> RgbImage {
    let (w, h) = rgb.dimensions();
    let (nw, nh) = fitted_size(w, h, target);
    if (nw, nh) == (w, h) {
        return rgb;
    }
    ::image::imageops::resize(&rgb, nw, nh, ::image::imageops::FilterType::Triangle)
}

fn encode(rgb: &RgbImage) -> Result<ImageInput, ImageError> {
    use ::image::codecs::jpeg::JpegEncoder;
    use ::image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use ::image::{ExtendedColorType, ImageEncoder};
    let (w, h) = rgb.dimensions();
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Default, FilterType::Adaptive)
        .write_image(rgb.as_raw(), w, h, ExtendedColorType::Rgb8)
        .map_err(damaged)?;
    if png.len() > PNG_SOFT_LIMIT {
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
            .write_image(rgb.as_raw(), w, h, ExtendedColorType::Rgb8)
            .map_err(damaged)?;
        if jpeg.len() * 3 < png.len() * 2 {
            return Ok(ImageInput::new(ImageMime::Jpeg, jpeg, w, h));
        }
    }
    Ok(ImageInput::new(ImageMime::Png, png, w, h))
}

/// Test images, made programmatically (never checked-in binaries).
pub mod testing {
    use ::image::{ExtendedColorType, ImageEncoder, Rgb, RgbImage, Rgba, RgbaImage};

    /// A `w`×`h` PNG: a light window with a dark title bar and a red "error" block.
    pub fn screenshot_png(w: u32, h: u32) -> Vec<u8> {
        let img = RgbImage::from_fn(w, h, |x, y| {
            if y < h / 8 {
                Rgb([30, 30, 60])
            } else if x > w / 4 && x < w * 3 / 4 && y > h / 3 && y < h * 2 / 3 {
                Rgb([200, 30, 30])
            } else {
                Rgb([240, 240, 240])
            }
        });
        encode_with(&img, ::image::ImageFormat::Png)
    }

    /// A small image in `format` (PNG, JPEG, BMP, GIF or WebP).
    pub fn encode_with(img: &RgbImage, format: ::image::ImageFormat) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        ::image::DynamicImage::ImageRgb8(img.clone())
            .write_to(&mut out, format)
            .unwrap_or_else(|e| panic!("test image: {e}"));
        out.into_inner()
    }

    /// A half-transparent PNG (alpha must be flattened).
    pub fn transparent_png(w: u32, h: u32) -> Vec<u8> {
        let img = RgbaImage::from_fn(w, h, |_, _| Rgba([0, 0, 0, 128]));
        let mut out = Vec::new();
        ::image::codecs::png::PngEncoder::new(&mut out)
            .write_image(img.as_raw(), w, h, ExtendedColorType::Rgba8)
            .unwrap_or_else(|e| panic!("test image: {e}"));
        out
    }

    /// `jpeg` with an EXIF APP1 segment holding `tiff`, right after SOI.
    pub fn with_exif(jpeg: &[u8], tiff: &[u8]) -> Vec<u8> {
        let mut seg = b"Exif\0\0".to_vec();
        seg.extend(tiff);
        let len = (seg.len() + 2) as u16;
        let mut out = jpeg[..2].to_vec();
        out.extend([0xff, 0xe1]);
        out.extend(len.to_be_bytes());
        out.extend(seg);
        out.extend(&jpeg[2..]);
        out
    }

    /// A PNG whose header claims `w`×`h` pixels but holds almost no data (a decompression bomb's shape).
    pub fn claimed_png(w: u32, h: u32) -> Vec<u8> {
        let mut real = screenshot_png(4, 4);
        // IHDR's width and height are bytes 16..24; the CRC is not checked before the size is.
        real[16..20].copy_from_slice(&w.to_be_bytes());
        real[20..24].copy_from_slice(&h.to_be_bytes());
        // IHDR's CRC covers its type and data (bytes 12..29).
        let crc = crc32(&real[12..29]);
        real[29..33].copy_from_slice(&crc.to_be_bytes());
        real
    }

    /// CRC-32 (IEEE), for PNG chunks.
    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for b in data {
            c ^= u32::from(*b);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use ::image::{GenericImageView, Rgb, RgbImage};

    fn clean(bytes: &[u8]) -> Result<SanitizedImage, ImageError> {
        sanitize(bytes, DEFAULT_TARGET_PIXELS, &ImageLimits::default())
    }

    fn decode_back(i: &ImageInput) -> ::image::DynamicImage {
        ::image::load_from_memory(i.bytes()).unwrap()
    }

    #[test]
    fn every_accepted_format_becomes_a_clean_png() {
        let img = RgbImage::from_fn(40, 30, |x, y| Rgb([(x * 6) as u8, (y * 8) as u8, 90]));
        for (format, kind) in [
            (::image::ImageFormat::Png, SourceFormat::Png),
            (::image::ImageFormat::Jpeg, SourceFormat::Jpeg),
            (::image::ImageFormat::Bmp, SourceFormat::Bmp),
            (::image::ImageFormat::Gif, SourceFormat::Gif),
            (::image::ImageFormat::WebP, SourceFormat::Webp),
        ] {
            let s = clean(&encode_with(&img, format)).unwrap_or_else(|e| panic!("{format:?}: {e}"));
            assert_eq!(s.source_format, kind);
            assert_eq!((s.original_width, s.original_height), (40, 30));
            assert_eq!(s.image.mime(), ImageMime::Png);
            assert_eq!((s.image.width(), s.image.height()), (40, 30));
            assert_eq!(decode_back(&s.image).dimensions(), (40, 30));
        }
    }

    #[test]
    fn big_screenshots_are_downscaled_to_the_model_budget() {
        let s = clean(&screenshot_png(2560, 1440)).unwrap();
        let (w, h) = (s.image.width(), s.image.height());
        assert!(u64::from(w) * u64::from(h) <= DEFAULT_TARGET_PIXELS, "{w}x{h}");
        assert!((f64::from(w) / f64::from(h) - 16.0 / 9.0).abs() < 0.01);
        assert_eq!((s.original_width, s.original_height), (2560, 1440));
        assert_eq!(fitted_size(100, 50, 1_000_000), (100, 50), "never enlarged");
        assert_eq!(fitted_size(20_000, 10, u64::MAX), (MAX_OUTPUT_SIDE, 2));
        assert_eq!(fitted_size(0, 0, 0), (1, 1));
    }

    #[test]
    fn metadata_is_dropped_but_the_capture_time_is_kept() {
        let img = RgbImage::from_fn(16, 16, |_, _| Rgb([10, 200, 10]));
        let jpeg = encode_with(&img, ::image::ImageFormat::Jpeg);
        let tiff = crate::vision::exif::tests::tiff(Some("2026:10:21 09:14:03"), None, Some("+02:00"));
        let with = with_exif(&jpeg, &tiff);
        let s = clean(&with).unwrap();
        let t = s.capture.unwrap();
        assert_eq!((t.local.as_str(), t.offset.as_deref()), ("2026-10-21 09:14:03", Some("+02:00")));
        // The copy holds pixels only: no EXIF, no "Exif" marker anywhere.
        let out = s.image.bytes();
        assert!(!out.windows(4).any(|w| w == b"Exif"));
        assert!(!out.windows(4).any(|w| w == b"eXIf"));
    }

    #[test]
    fn transparency_is_flattened_onto_white() {
        let s = clean(&transparent_png(8, 8)).unwrap();
        let px = decode_back(&s.image).to_rgb8().get_pixel(3, 3).0;
        assert!(px.iter().all(|c| (120..=135).contains(c)), "{px:?}");
    }

    #[test]
    fn limits_and_bad_input_are_typed_errors() {
        assert_eq!(clean(&[]), Err(ImageError::Empty));
        assert_eq!(clean(b"<svg/>"), Err(ImageError::Unsupported));
        assert_eq!(clean(b"%PDF-1.7"), Err(ImageError::Unsupported));
        let small = ImageLimits {
            max_file_bytes: 10,
            ..ImageLimits::default()
        };
        assert!(matches!(
            sanitize(&screenshot_png(8, 8), 1000, &small),
            Err(ImageError::TooLarge { max: 10, .. })
        ));
        // A bomb's header is refused before decoding.
        assert!(matches!(
            clean(&claimed_png(60_000, 60_000)),
            Err(ImageError::TooManyPixels { width: 60_000, .. })
        ));
        assert!(matches!(clean(&claimed_png(16_000, 16_000)), Err(ImageError::TooManyPixels { .. })));
        // Truncated and damaged files.
        let good = screenshot_png(64, 48);
        assert!(matches!(clean(&good[..40]), Err(ImageError::Damaged(_))));
        assert!(matches!(clean(&good[..8]), Err(ImageError::Damaged(_))));
        assert!(clean(b"GIF89a").is_err());
        assert!(clean(b"RIFF\0\0\0\0WEBP").is_err());
        assert!(clean(b"BM").is_err());
        assert!(clean(&[0xff, 0xd8, 0xff]).is_err());
        let e = ImageError::TooManyPixels {
            width: 1,
            height: 2,
            max_pixels: 3,
            max_side: 4,
        };
        assert!(e.to_string().contains("1×2"));
    }

    #[test]
    fn sniffing_needs_the_signature() {
        assert_eq!(SourceFormat::sniff(b"GIF87a..."), Some(SourceFormat::Gif));
        assert_eq!(SourceFormat::sniff(b"GIF88a..."), None);
        assert_eq!(SourceFormat::sniff(b"RIFF1234WAVE"), None);
        assert!(SourceFormat::EXTENSIONS.contains(&"webp"));
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]
        /// Never a panic, whatever the bytes (with and without a valid signature in front).
        #[test]
        fn never_panics(body in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..2048), sig in 0usize..6) {
            let prefix: &[u8] = match sig {
                0 => b"\x89PNG\r\n\x1a\n",
                1 => b"\xff\xd8\xff",
                2 => b"BM",
                3 => b"GIF89a",
                4 => b"RIFF\0\0\0\0WEBP",
                _ => b"",
            };
            let mut bytes = prefix.to_vec();
            bytes.extend(body);
            let _ = sanitize(&bytes, 4096, &ImageLimits::default());
        }

        /// A valid image with bytes flipped still ends in an image or a typed error.
        #[test]
        fn damaged_images_never_panic(flips in proptest::collection::vec((0usize..4096, proptest::prelude::any::<u8>()), 1..16)) {
            let mut bytes = screenshot_png(48, 32);
            for (i, v) in flips {
                let n = bytes.len();
                bytes[i % n] = v;
            }
            let _ = sanitize(&bytes, 4096, &ImageLimits::default());
        }
    }
}
