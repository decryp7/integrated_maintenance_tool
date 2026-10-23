//! Images for the AI (ADR 0025): opening one (a file the user chose, or pasted or dropped bytes), decoding it in
//! Rust within limits into a clean copy kept in the window's small image cache, image support for a model (its
//! projector), and the image use cases (explain, find related logs) that run as AI jobs.
//!
//! The webview never decodes the user's image for the AI path: it gets the clean copy as a `data:` URL.

use super::commands::{core, WindowStore};
use super::core::ModelDto;
use super::error::{AiCmdError, AiResult};
use crate::jobs::{self, Controls, Done, JobCtx, JobKind, JobLink, JobProgress, JobResult, JobSpec};
use crate::windows::{CurrentWindow, WindowState};
use debrief_ai::assistant::{Assistant, Narrative};
use debrief_ai::llm::LlmProvider;
use debrief_ai::vision::image::{sanitize, ImageLimits, SanitizedImage, SourceFormat};
use debrief_ai::vision::{RelatedOutcome, RelatedStage};
use debrief_ai::{AiError, StoreAccess, StoreRef};
use debrief_core::related::{find_related, ExifStamp, QuerySource, RelatedRequest, RelatedResult, TimeEvidence};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Runtime};

/// Images a window keeps open (clean copies, a few MB each).
pub const MAX_OPEN_IMAGES: usize = 8;
/// Longest name kept for an image.
const MAX_NAME_CHARS: usize = 120;

type Ai<T> = AiResult<T>;

/// An image the window opened: its clean copy and the evidence of when it was taken.
#[derive(Debug, Clone)]
pub struct OpenImage {
    pub id: u32,
    pub name: String,
    /// The file it came from (none for pasted or dropped images).
    pub path: Option<String>,
    pub image: SanitizedImage,
    pub evidence: TimeEvidence,
}

/// The window's open images, newest last, at most [`MAX_OPEN_IMAGES`] (the oldest is dropped).
#[derive(Default)]
pub struct ImageCache {
    inner: Mutex<(u32, VecDeque<Arc<OpenImage>>)>,
}

impl ImageCache {
    pub fn insert(&self, name: String, path: Option<String>, image: SanitizedImage, evidence: TimeEvidence) -> Arc<OpenImage> {
        let mut g = crate::sync::lock(&self.inner);
        g.0 = g.0.wrapping_add(1).max(1);
        let img = Arc::new(OpenImage {
            id: g.0,
            name,
            path,
            image,
            evidence,
        });
        g.1.push_back(img.clone());
        while g.1.len() > MAX_OPEN_IMAGES {
            g.1.pop_front();
        }
        img
    }

    pub fn get(&self, id: u32) -> Option<Arc<OpenImage>> {
        crate::sync::lock(&self.inner).1.iter().find(|i| i.id == id).cloned()
    }

    /// The images `ids`, or which one is gone.
    pub fn get_all(&self, ids: &[u32]) -> Ai<Vec<Arc<OpenImage>>> {
        if ids.len() > debrief_ai::worker::protocol::MAX_IMAGES {
            return Err(AiCmdError::invalid(format!(
                "attach at most {} images to one question",
                debrief_ai::worker::protocol::MAX_IMAGES
            )));
        }
        ids.iter().map(|id| self.get(*id).ok_or_else(gone)).collect()
    }

    pub fn remove(&self, id: u32) -> bool {
        let mut g = crate::sync::lock(&self.inner);
        let before = g.1.len();
        g.1.retain(|i| i.id != id);
        g.1.len() != before
    }
}

fn gone() -> AiCmdError {
    AiCmdError::not_found("the image is no longer open: attach or open it again")
}

/// An open image as the UI shows it: the clean copy only.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageDto {
    pub id: u32,
    pub name: String,
    pub path: Option<String>,
    pub width: u32,
    pub height: u32,
    pub original_width: u32,
    pub original_height: u32,
    pub source_format: SourceFormat,
    /// `data:image/png;base64,...` of the clean copy.
    pub data_url: String,
    /// The EXIF capture time, as the device wrote it (`YYYY-MM-DD HH:MM:SS`).
    pub captured_at: Option<String>,
}

impl ImageDto {
    pub fn of(i: &OpenImage) -> ImageDto {
        use base64::Engine as _;
        let img = &i.image.image;
        ImageDto {
            id: i.id,
            name: i.name.clone(),
            path: i.path.clone(),
            width: img.width(),
            height: img.height(),
            original_width: i.image.original_width,
            original_height: i.image.original_height,
            source_format: i.image.source_format,
            data_url: format!(
                "data:{};base64,{}",
                img.mime().as_str(),
                base64::engine::general_purpose::STANDARD.encode(img.bytes())
            ),
            captured_at: i.image.capture.as_ref().map(|c| c.local.clone()),
        }
    }
}

fn clean_name(name: &str) -> String {
    let n: String = name.chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect();
    match n.trim() {
        "" => "image".into(),
        t => t.to_string(),
    }
}

fn image_error(e: debrief_ai::vision::ImageError) -> AiCmdError {
    AiCmdError::invalid(e.to_string())
}

/// Decodes and caches an image (blocking: decoding a large file takes a moment).
fn open_bytes(win: &WindowState, bytes: &[u8], name: &str, path: Option<String>, mut evidence: TimeEvidence, target: u64) -> Ai<ImageDto> {
    let image = sanitize(bytes, target, &ImageLimits::default()).map_err(image_error)?;
    evidence.exif = image.capture.as_ref().map(|c| ExifStamp {
        local: c.local.clone(),
        offset: c.offset.clone(),
    });
    let open = win.ai.images.insert(clean_name(name), path, image, evidence);
    Ok(ImageDto::of(&open))
}

/// Whether `raw` is a file the window's workspace listed (an opened source or an unrecognised file of an opened
/// folder), so an image shown in the Explorer opens without choosing it again.
fn listed_in_workspace(win: &WindowState, raw: &str) -> bool {
    let Some(s) = win.engine.opened().and_then(|store| store.summary().ok()) else {
        return false;
    };
    s.unrecognized.iter().any(|u| u.path == raw) || s.sources.iter().any(|x| x.path == raw)
}

/// Opens an image file the user chose (dialog, drag-and-drop) or the Explorer lists, as a clean copy.
#[tauri::command]
pub async fn ai_image_open<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    grants: crate::paths::WindowGrants,
    path: String,
) -> Ai<ImageDto> {
    let core = core(&app)?;
    let w = win.arc().clone();
    let granted = grants.read_or_listed(&path, SourceFormat::EXTENSIONS, |raw| listed_in_workspace(&w, raw))?;
    let file = granted.path().to_path_buf();
    let target = core.image_target_pixels();
    crate::sync::blocking("opening the image", move || {
        let meta = std::fs::metadata(&file).map_err(|e| AiCmdError::not_found(format!("cannot read {}: {e}", file.display())))?;
        let limit = ImageLimits::default().max_file_bytes as u64;
        if meta.len() > limit {
            return Err(AiCmdError::invalid(format!(
                "the image is {:.1} MB; images up to {} MB can be opened",
                meta.len() as f64 / 1e6,
                limit / (1024 * 1024)
            )));
        }
        let bytes = std::fs::read(&file).map_err(|e| AiCmdError::not_found(format!("cannot read {}: {e}", file.display())))?;
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| i64::try_from(d.as_millis()).ok());
        let evidence = TimeEvidence {
            file_name: Some(name.clone()),
            file_modified_ms: modified,
            ..TimeEvidence::default()
        };
        open_bytes(&w, &bytes, &name, Some(file.to_string_lossy().into_owned()), evidence, target)
    })
    .await
}

/// Opens pasted or dropped image bytes (base64 from the clipboard or a drop; the webview does not decode them).
#[tauri::command]
pub async fn ai_image_paste<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, data: String, name: Option<String>) -> Ai<ImageDto> {
    use base64::Engine as _;
    let core = core(&app)?;
    let max = ImageLimits::default().max_file_bytes;
    if data.len() > max / 3 * 4 + 4 {
        return Err(AiCmdError::invalid(format!(
            "images up to {} MB can be pasted",
            max / (1024 * 1024)
        )));
    }
    let w = win.arc().clone();
    let target = core.image_target_pixels();
    crate::sync::blocking("reading the image", move || {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data.trim())
            .map_err(|_| AiCmdError::invalid("the pasted data is not an image"))?;
        let name = name.unwrap_or_else(|| "pasted image".into());
        let evidence = TimeEvidence {
            file_name: Some(clean_name(&name)),
            ..TimeEvidence::default()
        };
        open_bytes(&w, &bytes, &name, None, evidence, target)
    })
    .await
}

/// Forgets an open image.
#[tauri::command]
pub async fn ai_image_close(win: CurrentWindow, id: u32) -> Ai<bool> {
    Ok(win.arc().ai.images.remove(id))
}

/// "Add image support": downloads the catalog projector of an installed model as a download job (never
/// automatically). Returns the job.
#[tauri::command]
pub async fn ai_model_install_projector<R: Runtime>(app: AppHandle<R>, file_name: String) -> Ai<u64> {
    let core = core(&app)?;
    core.availability()?;
    // Fails fast for a model the catalog has no projector for.
    core.projector_download_bytes(&file_name)?;
    let progress = Arc::new(debrief_ai::models::Progress::new());
    let (p, q) = (progress.clone(), progress.clone());
    let controls = Controls::cancel(move || p.cancel()).with_probe(move || {
        let s = q.snapshot();
        (s.total > 0).then(|| JobProgress::counts(s.bytes, s.total))
    });
    let spec = JobSpec::new(JobKind::Download, format!("Image support for {file_name}")).link(JobLink::Model { id: file_name.clone() });
    let h = app.clone();
    let id = jobs::spawn(&app, spec, controls, move |_ctx: &JobCtx| -> Ai<Done<()>> {
        let r = core.install_projector(&file_name, &progress, &mut |_| {});
        let _ = tauri::Emitter::emit(&h, super::STATUS_EVENT, core.status());
        r?;
        Ok(Done::new(()).with_result(JobResult::Model {
            file_name: Some(file_name.clone()),
        }))
    })
    .map_err(AiCmdError::busy)?;
    Ok(id.0)
}

/// Adds a projector file the user chose for an installed model (checked like a download), as a download job.
#[tauri::command]
pub async fn ai_model_add_projector<R: Runtime>(
    app: AppHandle<R>,
    grants: crate::paths::WindowGrants,
    file_name: String,
    path: String,
) -> Ai<ModelDto> {
    let core = core(&app)?;
    core.availability()?;
    let file = grants.read_file(&path, &["gguf"])?.path().to_path_buf();
    let progress = Arc::new(debrief_ai::models::Progress::new());
    let (p, q) = (progress.clone(), progress.clone());
    let controls = Controls::cancel(move || p.cancel()).with_probe(move || {
        let s = q.snapshot();
        (s.total > 0).then(|| JobProgress::counts(s.bytes, s.total))
    });
    let spec = JobSpec::new(JobKind::Download, format!("Checking the image projector for {file_name}")).link(JobLink::Path {
        path: file.to_string_lossy().into_owned(),
    });
    jobs::run(&app, spec, controls, move |_ctx: &JobCtx| -> Ai<Done<ModelDto>> {
        let m = core.add_projector(&file_name, &file, &progress)?;
        Ok(Done::new(m).with_result(JobResult::Model {
            file_name: Some(file_name.clone()),
        }))
    })
    .await
}

// ---- the use cases ----

/// The engine's "find related logs" over the window's store, borrowed for the search only.
pub(crate) fn search_in(store: StoreRef<'_>) -> impl FnMut(&RelatedRequest) -> debrief_ai::Result<RelatedResult> + '_ {
    move |req| {
        store
            .with(|s| find_related(&QuerySource(s), req))
            .map_err(|e| AiError::Engine(format!("the search failed: {e}")))
    }
}

/// The time zone for times without one: the generic parser's (formats are parsers, ADR 0026).
pub(crate) fn timezone(_store: StoreRef<'_>) -> String {
    debrief_core::parser::builtin::generic().format.assume_timezone
}

/// "Explain with AI" on an image.
pub(crate) fn explain<P: LlmProvider>(a: &Assistant<P>, img: &OpenImage, question: Option<&str>) -> Ai<Narrative> {
    Ok(a.explain_image(&img.image.image, question)?)
}

/// "Find related logs" for an image, reporting its stages as the job's progress.
pub(crate) fn related<P: LlmProvider>(a: &Assistant<P>, win: &Arc<WindowState>, img: &OpenImage, ctx: &JobCtx) -> Ai<RelatedOutcome> {
    let access = WindowStore(win.clone());
    let store: StoreRef<'_> = (&access as &dyn StoreAccess).into();
    let tz = timezone(store);
    let mut search = search_in(store);
    Ok(
        a.find_related_logs(&img.image.image, img.evidence.clone(), &tz, &mut search, &mut |s| {
            ctx.progress(JobProgress::fraction(match s {
                RelatedStage::Reading => 0.05,
                RelatedStage::Searching => 0.7,
                RelatedStage::Explaining => 0.8,
            }))
        })?,
    )
}

/// The path of an open image, for a job's link.
pub(crate) fn link_of(img: &OpenImage) -> Option<JobLink> {
    img.path
        .as_ref()
        .filter(|p| Path::new(p).is_absolute())
        .map(|p| JobLink::Path { path: p.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_ai::vision::image::testing::screenshot_png;

    fn clean() -> SanitizedImage {
        sanitize(&screenshot_png(64, 40), 1 << 20, &ImageLimits::default()).unwrap()
    }

    #[test]
    fn the_cache_is_bounded_and_ids_are_unique() {
        let c = ImageCache::default();
        let ids: Vec<u32> = (0..MAX_OPEN_IMAGES + 2)
            .map(|i| c.insert(format!("i{i}"), None, clean(), TimeEvidence::default()).id)
            .collect();
        assert_eq!(ids.len(), MAX_OPEN_IMAGES + 2);
        assert!(c.get(ids[0]).is_none() && c.get(ids[1]).is_none(), "the oldest are dropped");
        assert!(c.get(ids[2]).is_some());
        assert!(c.get_all(&[ids[2], ids[3]]).is_ok());
        assert_eq!(c.get_all(&[ids[0]]).unwrap_err().code, super::super::error::AiErrorCode::NotFound);
        assert!(c.get_all(&[ids[2]; 5]).is_err());
        assert!(c.remove(ids[2]) && !c.remove(ids[2]));
    }

    #[test]
    fn the_ui_gets_the_clean_copy_only() {
        let c = ImageCache::default();
        let mut img = clean();
        img.capture = Some(debrief_ai::vision::exif::ExifTime {
            local: "2026-10-21 09:14:03".into(),
            offset: None,
        });
        let open = c.insert(
            clean_name("shot\u{7}.png"),
            Some("/x/shot.png".into()),
            img,
            TimeEvidence::default(),
        );
        let dto = ImageDto::of(&open);
        assert_eq!(dto.name, "shot.png");
        assert!(dto.data_url.starts_with("data:image/png;base64,"));
        assert_eq!((dto.width, dto.height, dto.original_width), (64, 40, 64));
        assert_eq!(dto.captured_at.as_deref(), Some("2026-10-21 09:14:03"));
        assert_eq!(clean_name("  "), "image");
        assert_eq!(clean_name(&"n".repeat(500)).chars().count(), MAX_NAME_CHARS);
        if Path::new("/x/shot.png").is_absolute() {
            assert_eq!(
                link_of(&open),
                Some(JobLink::Path {
                    path: "/x/shot.png".into()
                })
            );
        } else {
            assert_eq!(link_of(&open), None);
        }
    }
}
