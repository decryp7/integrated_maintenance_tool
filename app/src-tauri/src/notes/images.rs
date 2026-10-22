//! Images opened to mark notes on (ADR 0021 §4). The file is read and decoded in Rust within the limits of ADR 0025
//! (file size, claimed pixels, decoder memory) into a clean copy at most 4096 px a side; the webview only gets its
//! `data:` URL. A note on an image keeps that copy in the project folder (`notes/images/<key>.<png|jpg>`), so the
//! image opens from a shared project without the original file. Keys are the SHA-256 of the original file.

use crate::error::{CmdError, CmdResult, ErrorCode};
use debrief_ai::vision::image::{sanitize, ImageLimits, SanitizedImage};
use debrief_notes::{image_key, is_image_key, ImageRef};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Images a window keeps decoded for notes.
pub const MAX_OPEN: usize = 8;
/// The pixel budget of a clean copy (its longest side is capped at 4096 px by the sanitizer as well).
pub const VIEW_PIXELS: u64 = 4096 * 4096;
/// Folder of the copies, inside the project (store) folder.
pub const COPIES: &str = "notes/images";

/// An image as the notes' image view shows it.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteImage {
    /// The content key notes name it by.
    pub key: String,
    pub name: String,
    /// The file it was opened from (none when it came from the project's copy).
    pub path: Option<String>,
    /// The original's size: note regions are in these pixels.
    pub width: u32,
    pub height: u32,
    /// The size of the copy in `data_url`.
    pub view_width: u32,
    pub view_height: u32,
    /// `data:image/png;base64,...` of the clean copy.
    pub data_url: String,
}

/// An opened image: its clean copy (bytes, for the project copy) and what the view gets.
#[derive(Debug)]
pub struct OpenNoteImage {
    pub dto: NoteImage,
    pub bytes: Vec<u8>,
    pub ext: &'static str,
}

impl OpenNoteImage {
    fn of(image: SanitizedImage, key: String, name: String, path: Option<String>, size: Option<(u32, u32)>) -> OpenNoteImage {
        use base64::Engine as _;
        let img = &image.image;
        let mime = img.mime().as_str();
        let (width, height) = size.unwrap_or((image.original_width, image.original_height));
        OpenNoteImage {
            dto: NoteImage {
                key,
                name,
                path,
                width,
                height,
                view_width: img.width(),
                view_height: img.height(),
                data_url: format!(
                    "data:{mime};base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(img.bytes())
                ),
            },
            bytes: img.bytes().to_vec(),
            ext: if mime == "image/jpeg" { "jpg" } else { "png" },
        }
    }

    /// The reference a note keeps.
    pub fn image_ref(&self) -> ImageRef {
        ImageRef {
            key: self.dto.key.clone(),
            name: self.dto.name.clone(),
            path: self.dto.path.clone(),
            width: self.dto.width,
            height: self.dto.height,
        }
    }
}

/// The window's decoded images, newest last, at most [`MAX_OPEN`].
#[derive(Default)]
pub struct NoteImages {
    inner: Mutex<VecDeque<Arc<OpenNoteImage>>>,
}

impl NoteImages {
    pub fn insert(&self, img: OpenNoteImage) -> Arc<OpenNoteImage> {
        let img = Arc::new(img);
        let mut g = crate::sync::lock(&self.inner);
        g.retain(|i| i.dto.key != img.dto.key);
        g.push_back(img.clone());
        while g.len() > MAX_OPEN {
            g.pop_front();
        }
        img
    }

    pub fn get(&self, key: &str) -> Option<Arc<OpenNoteImage>> {
        crate::sync::lock(&self.inner).iter().find(|i| i.dto.key == key).cloned()
    }
}

fn bad_image(e: debrief_ai::vision::ImageError) -> CmdError {
    CmdError::invalid(e.to_string())
}

fn io(what: &Path, e: std::io::Error) -> CmdError {
    let code = if e.kind() == std::io::ErrorKind::NotFound {
        ErrorCode::NotFound
    } else {
        ErrorCode::Io
    };
    CmdError::new(code, format!("cannot read {}: {e}", what.display()))
}

/// Reads and decodes the image file `file` (already granted). Blocking.
pub fn open_file(file: &Path) -> CmdResult<OpenNoteImage> {
    let limit = ImageLimits::default().max_file_bytes as u64;
    let len = std::fs::metadata(file).map_err(|e| io(file, e))?.len();
    if len > limit {
        return Err(CmdError::new(
            ErrorCode::Limit,
            format!(
                "the image is {:.1} MB; images up to {} MB can be opened",
                len as f64 / 1e6,
                limit / (1024 * 1024)
            ),
        ));
    }
    let bytes = std::fs::read(file).map_err(|e| io(file, e))?;
    let image = sanitize(&bytes, VIEW_PIXELS, &ImageLimits::default()).map_err(bad_image)?;
    let name: String = file
        .file_name()
        .map(|n| n.to_string_lossy().chars().filter(|c| !c.is_control()).take(120).collect())
        .unwrap_or_default();
    Ok(OpenNoteImage::of(
        image,
        image_key(&bytes),
        name,
        Some(file.to_string_lossy().into_owned()),
        None,
    ))
}

/// Where the copy of image `key` is (or goes) in the project folder `dir`, as `.png` or `.jpg`.
fn copy_path(dir: &Path, key: &str, ext: &str) -> PathBuf {
    dir.join(COPIES).join(format!("{key}.{ext}"))
}

/// The existing copy of image `key` in project folder `dir`.
pub fn find_copy(dir: &Path, key: &str) -> Option<PathBuf> {
    if !is_image_key(key) {
        return None;
    }
    ["png", "jpg"].iter().map(|e| copy_path(dir, key, e)).find(|p| p.is_file())
}

/// Keeps the clean copy of `img` in project folder `dir` (once; atomically). Blocking.
pub fn keep_copy(dir: &Path, img: &OpenNoteImage) -> CmdResult<()> {
    if find_copy(dir, &img.dto.key).is_some() {
        return Ok(());
    }
    let target = copy_path(dir, &img.dto.key, img.ext);
    let fail = |e: std::io::Error| CmdError::new(ErrorCode::Io, format!("cannot keep the image in the project: {e}"));
    std::fs::create_dir_all(dir.join(COPIES)).map_err(fail)?;
    crate::config_store::write_atomic(&target, &img.bytes).map_err(fail)
}

/// Opens image `key` from its copy in project folder `dir`, with the name and size a note recorded. Blocking.
pub fn open_copy(dir: &Path, image: &ImageRef) -> CmdResult<OpenNoteImage> {
    let file = find_copy(dir, &image.key)
        .ok_or_else(|| CmdError::not_found(format!("{} is not in this project any more", image.name)).hint("Open the image file again."))?;
    let bytes = std::fs::read(&file).map_err(|e| io(&file, e))?;
    let decoded = sanitize(&bytes, VIEW_PIXELS, &ImageLimits::default()).map_err(bad_image)?;
    Ok(OpenNoteImage::of(
        decoded,
        image.key.clone(),
        image.name.clone(),
        None,
        Some((image.width, image.height)),
    ))
}
