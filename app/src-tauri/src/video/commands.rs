//! The Video view's commands (ADR 0033 §3-7): open (analyse) a video, list the project's videos, frame images,
//! alignment, notes, AI descriptions, an on-screen clock reading, explanations, the user's own frames, the optional
//! copy of the video in the project and removal. Every command validates its arguments; work that takes time is a job
//! (analysis, AI) and every change to a record rewrites its frames source and replaces it in the workspace.

use super::source::{replace, replace_now, source_id};
use super::{cmd, folder_of, state, timezone, view_of, FFMPEG_MEMORY};
use crate::ai::vision::ImageDto;
use crate::error::{CmdError, CmdResult, ErrorCode};
use crate::jobs::{self, Controls, Done, JobCtx, JobKind, JobLink, JobProgress, JobResult, JobSpec};
use crate::paths::WindowGrants;
use crate::windows::{CurrentWindow, WindowState};
use debrief_ai::vision::{sanitize, ImageLimits};
use debrief_core::related::{ExifStamp, TimeEvidence};
use debrief_video::api::*;
use debrief_video::describe::{describe_frame, explain_range, load_frame, read_clock, utc_stamp};
use debrief_video::ingest::{add_frame, analyse, media_file, Ingest, Stage};
use debrief_video::media::{sniff, EXTENSIONS};
use debrief_video::record::{clean_text, is_key, VideoFolder, VideoRecord, MAX_PROJECT_FRAMES};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Runtime};

/// Paths one `video_sniff` call accepts (one drop).
const MAX_SNIFF_PATHS: usize = 1_000;
/// Frames one explanation may name.
const MAX_EXPLAIN: usize = debrief_ai::vision::MAX_IMAGES_PER_REQUEST;
/// Largest frame image read for display.
const MAX_FRAME_BYTES: u64 = 24 << 20;

fn scratch_error(e: std::io::Error) -> CmdError {
    CmdError::new(ErrorCode::Io, format!("cannot prepare a working folder for FFmpeg: {e}"))
}

fn key_of_request(key: &str) -> CmdResult<&str> {
    if is_key(key) {
        Ok(key)
    } else {
        Err(CmdError::not_found("that video is not in this project").field("key"))
    }
}

fn index(i: u32, record: &VideoRecord) -> CmdResult<usize> {
    let i = i as usize;
    if i < record.frames.len() {
        Ok(i)
    } else {
        Err(CmdError::not_found(format!("frame {} of {}", i + 1, record.name)).field("index"))
    }
}

/// Loads, changes and saves a record under the edit lock; returns the new record.
fn edit_record(
    app_state: &super::VideoState,
    folder: &VideoFolder,
    key: &str,
    f: impl FnOnce(&mut VideoRecord) -> CmdResult<()>,
) -> CmdResult<VideoRecord> {
    app_state.edit(|| {
        let mut r = folder.load(key).map_err(cmd)?;
        f(&mut r)?;
        folder.save(&r).map_err(cmd)?;
        Ok(r)
    })
}

/// The record after a change, as the view, with its source replaced in the workspace.
fn publish(win: &WindowState, folder: &VideoFolder, record: VideoRecord) -> CmdResult<VideoView> {
    let id = replace_now(win, &record)?;
    Ok(view_of(folder, record, id))
}

fn is_video_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, e)| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn read_head(grants: &WindowGrants, path: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let g = grants.read_file(path, &[]).ok()?;
    let mut buf = Vec::new();
    std::fs::File::open(g.path()).ok()?.take(64).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Which of the dropped `paths` are videos (by extension and first bytes), for drag-and-drop routing.
#[tauri::command]
pub async fn video_sniff(grants: WindowGrants, paths: Vec<String>) -> CmdResult<Vec<String>> {
    if paths.len() > MAX_SNIFF_PATHS {
        return Err(CmdError::new(
            ErrorCode::Limit,
            format!("at most {MAX_SNIFF_PATHS} files can be checked at once"),
        ));
    }
    crate::sync::blocking("checking the files", move || {
        Ok::<_, CmdError>(
            paths
                .into_iter()
                .filter(|p| is_video_name(p) && read_head(&grants, p).is_some_and(|h| sniff(&h)))
                .collect(),
        )
    })
    .await
}

/// The project's videos.
#[tauri::command]
pub async fn video_list(win: CurrentWindow) -> CmdResult<VideoList> {
    let w = win.arc().clone();
    crate::sync::blocking("listing the videos", move || {
        let folder = folder_of(&w)?;
        let listed = folder.list();
        let store = w.engine.store()?;
        let videos: Vec<VideoView> = listed
            .videos
            .into_iter()
            .map(|r| {
                let id = super::source::frames_file(&w, &r).ok().and_then(|f| source_id(store.as_ref(), &f));
                view_of(&folder, r, id)
            })
            .collect();
        let project_frames = videos.iter().map(|v| v.record.frames.len()).sum::<usize>();
        Ok::<_, CmdError>(VideoList {
            videos,
            skipped: listed.skipped.into_iter().map(|(key, why)| SkippedVideo { key, why }).collect(),
            project_frames: project_frames as u32,
            max_project_frames: MAX_PROJECT_FRAMES as u32,
        })
    })
    .await
}

fn stage_text(s: Stage) -> &'static str {
    match s {
        Stage::Hashing => "Reading the video",
        Stage::Probing => "Looking at the video",
        Stage::Analysing => "Finding the frames that change",
        Stage::Extracting => "Extracting the frames",
        Stage::Saving => "Saving the frames",
    }
}

/// Analyses a video the user chose and saves it in the project (a job: progress and Cancel in the Jobs pane). The
/// video is read, never changed; only its frames, times and notes are kept (the file itself only when asked).
#[tauri::command]
pub async fn video_open<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    grants: WindowGrants,
    request: OpenVideoRequest,
) -> CmdResult<VideoView> {
    let granted = grants.read_file(&request.path, &[])?;
    let input = granted.path().to_path_buf();
    let name = input.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !is_video_name(&name) {
        return Err(
            CmdError::new(ErrorCode::Unsupported, format!("{name} is not a video Debrief can open"))
                .hint("Videos: MP4, MOV, MKV, WebM, AVI."),
        );
    }
    let tools = state(&app)?.ffmpeg.tools().map_err(cmd)?;
    let w = win.arc().clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    let spec = JobSpec::new(JobKind::Analysis, format!("Analysing the video {name}"))
        .window(win.label())
        .link(JobLink::Path {
            path: input.to_string_lossy().into_owned(),
        });
    let h = app.clone();
    jobs::run(
        &app,
        spec,
        Controls::cancel(move || c.store(true, Ordering::SeqCst)),
        move |ctx: &JobCtx| -> CmdResult<Done<VideoView>> {
            let st = state(&h)?;
            let folder = folder_of(&w)?;
            let scratch = st.scratch_dir("analysis").map_err(scratch_error)?;
            let tz = timezone();
            let record = {
                let flag = ctx.cancel_flag();
                let i = Ingest {
                    tools: &tools,
                    folder: &folder,
                    input: &input,
                    options: request.options,
                    include: request.include,
                    tz: &tz,
                    work: scratch.path(),
                    memory_bytes: FFMPEG_MEMORY,
                    cancel: &flag,
                };
                st.edit(|| {
                    analyse(&i, &mut |stage, p| {
                        ctx.message(stage_text(stage));
                        ctx.progress(JobProgress::fraction(p as f32));
                    })
                })
                .map_err(cmd)?
            };
            let id = replace(&w, ctx, &cancel, &record)?;
            log::info!(Frames = record.frames.len(), Aligned = record.alignment.by.as_str(); "Video analysed");
            Ok(Done::new(view_of(&folder, record, id)).with_result(JobResult::Workspace))
        },
    )
    .await
}

/// The clean image of a frame, for display.
#[tauri::command]
pub async fn video_frame_image(win: CurrentWindow, frame: FrameRef) -> CmdResult<FrameImage> {
    use base64::Engine as _;
    let w = win.arc().clone();
    crate::sync::blocking("reading the frame", move || {
        let folder = folder_of(&w)?;
        let r = folder.load(key_of_request(&frame.key)?).map_err(cmd)?;
        let path = folder.frame_path(&r, index(frame.index, &r)?).map_err(cmd)?;
        let len = std::fs::metadata(&path)
            .map_err(|e| CmdError::not_found(format!("the frame image is missing ({e})")))?
            .len();
        if len > MAX_FRAME_BYTES {
            return Err(CmdError::new(ErrorCode::Limit, "the frame image is too large"));
        }
        let bytes = std::fs::read(&path).map_err(|e| CmdError::new(ErrorCode::Io, format!("cannot read the frame image: {e}")))?;
        let mime = if path.extension().is_some_and(|e| e == "jpg") {
            "image/jpeg"
        } else {
            "image/png"
        };
        Ok(FrameImage {
            data_url: format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)),
        })
    })
    .await
}

/// Puts a frame into the window's image cache as a clean image whose time evidence is the frame's aligned moment, so
/// the existing image use cases (Explain with AI, Find related logs) work on it unchanged.
#[tauri::command]
pub async fn video_frame_to_image<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, frame: FrameRef) -> CmdResult<ImageDto> {
    let target = crate::ai::commands::core(&app)?.image_target_pixels();
    let w = win.arc().clone();
    crate::sync::blocking("opening the frame", move || {
        let folder = folder_of(&w)?;
        let r = folder.load(key_of_request(&frame.key)?).map_err(cmd)?;
        let i = index(frame.index, &r)?;
        let path = folder.frame_path(&r, i).map_err(cmd)?;
        let bytes = std::fs::read(&path).map_err(|e| CmdError::new(ErrorCode::Io, format!("cannot read the frame image: {e}")))?;
        let clean = sanitize(&bytes, target, &ImageLimits::default()).map_err(|e| CmdError::invalid(e.to_string()))?;
        let at = r.alignment.at(r.frames[i].video_ms);
        let evidence = TimeEvidence {
            exif: utc_stamp(at).map(|(local, offset)| ExifStamp {
                local,
                offset: Some(offset),
            }),
            file_name: Some(r.name.clone()),
            ..TimeEvidence::default()
        };
        let name = format!("{} · frame {}", r.name, i + 1);
        let open = w.ai.images.insert(name, None, clean, evidence);
        Ok(ImageDto::of(&open))
    })
    .await
}

/// Sets, shifts or resets the alignment of a video to the logs.
#[tauri::command]
pub async fn video_set_alignment<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, request: AlignRequest) -> CmdResult<VideoView> {
    let w = win.arc().clone();
    let h = app.clone();
    crate::sync::blocking("aligning the video", move || {
        let st = state(&h)?;
        let folder = folder_of(&w)?;
        let tz = timezone();
        let r = edit_record(&st, &folder, key_of_request(&request.key)?, |r| {
            r.user_alignment = request.alignment;
            r.realign(&tz);
            Ok(())
        })?;
        publish(&w, &folder, r)
    })
    .await
}

/// Sets or removes the note on a frame.
#[tauri::command]
pub async fn video_set_note<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, request: NoteRequest) -> CmdResult<VideoView> {
    let w = win.arc().clone();
    let h = app.clone();
    crate::sync::blocking("saving the note", move || {
        let st = state(&h)?;
        let folder = folder_of(&w)?;
        let r = edit_record(&st, &folder, key_of_request(&request.key)?, |r| {
            let i = index(request.index, r)?;
            r.frames[i].note = request.note.as_deref().and_then(clean_text);
            Ok(())
        })?;
        publish(&w, &folder, r)
    })
    .await
}

/// Removes one frame (the video and its other frames stay).
#[tauri::command]
pub async fn video_remove_frame<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, frame: FrameRef) -> CmdResult<VideoView> {
    let w = win.arc().clone();
    let h = app.clone();
    crate::sync::blocking("removing the frame", move || {
        let st = state(&h)?;
        let folder = folder_of(&w)?;
        let r = edit_record(&st, &folder, key_of_request(&frame.key)?, |r| {
            let i = index(frame.index, r)?;
            if r.frames.len() == 1 {
                return Err(CmdError::invalid("a video keeps at least one frame: remove the video instead"));
            }
            r.frames.remove(i);
            debrief_video::ingest::renumber(&folder.dir(&r.key).map_err(cmd)?, &mut r.frames).map_err(cmd)?;
            Ok(())
        })?;
        publish(&w, &folder, r)
    })
    .await
}

/// The video file a record's frames can be taken from, or why not.
fn media_or_explain(folder: &VideoFolder, r: &VideoRecord) -> CmdResult<std::path::PathBuf> {
    media_file(folder, r).map(|(p, _)| p).ok_or_else(|| {
        CmdError::not_found(format!("the video file {} is not where it was", r.name))
            .hint("Open the video again from where it is now: its frames and notes are kept.")
    })
}

/// Adds the frame at or after a time (the player's position) as the user's own frame (a job: it runs FFmpeg).
#[tauri::command]
pub async fn video_add_frame<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, request: AddFrameRequest) -> CmdResult<VideoView> {
    let w = win.arc().clone();
    let tools = state(&app)?.ffmpeg.tools().map_err(cmd)?;
    let key = key_of_request(&request.key)?.to_string();
    let h = app.clone();
    let spec = JobSpec::new(JobKind::Analysis, "Adding a video frame").window(win.label());
    jobs::run(&app, spec, Controls::default(), move |ctx: &JobCtx| -> CmdResult<Done<VideoView>> {
        let st = state(&h)?;
        let folder = folder_of(&w)?;
        let scratch = st.scratch_dir("frame").map_err(scratch_error)?;
        let flag = ctx.cancel_flag();
        let tz = timezone();
        let record = st.edit(|| -> CmdResult<VideoRecord> {
            let mut r = folder.load(&key).map_err(cmd)?;
            let input = media_or_explain(&folder, &r)?;
            let i = Ingest {
                tools: &tools,
                folder: &folder,
                input: &input,
                options: r.options,
                include: false,
                tz: &tz,
                work: scratch.path(),
                memory_bytes: FFMPEG_MEMORY,
                cancel: &flag,
            };
            add_frame(&i, &mut r, request.video_ms).map_err(cmd)?;
            Ok(r)
        })?;
        let id = replace(&w, ctx, &flag, &record)?;
        Ok(Done::new(view_of(&folder, record, id)))
    })
    .await
}

/// Includes a copy of the video in the project, or drops that copy.
#[tauri::command]
pub async fn video_include<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, request: IncludeRequest) -> CmdResult<VideoView> {
    let w = win.arc().clone();
    let h = app.clone();
    let key = key_of_request(&request.key)?.to_string();
    let spec = JobSpec::new(
        JobKind::Maintenance,
        if request.include {
            "Copying the video into the project"
        } else {
            "Removing the video copy"
        },
    )
    .window(win.label());
    jobs::run(
        &app,
        spec,
        Controls::default(),
        move |_ctx: &JobCtx| -> CmdResult<Done<VideoView>> {
            let st = state(&h)?;
            let folder = folder_of(&w)?;
            let r = st.edit(|| -> CmdResult<VideoRecord> {
                let mut r = folder.load(&key).map_err(cmd)?;
                if request.include {
                    let from = media_or_explain(&folder, &r)?;
                    folder.include(&r, &from).map_err(cmd)?;
                } else {
                    folder.exclude(&r).map_err(cmd)?;
                }
                r.included = request.include;
                folder.save(&r).map_err(cmd)?;
                Ok(r)
            })?;
            let id = super::source::frames_file(&w, &r)
                .ok()
                .and_then(|f| w.engine.store().ok().and_then(|s| source_id(s.as_ref(), &f)));
            Ok(Done::new(view_of(&folder, r, id)))
        },
    )
    .await
}

/// Removes a video from the project: its source, frames, notes, descriptions and any included copy. The original
/// file is never touched. The UI asks first.
#[tauri::command]
pub async fn video_remove<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, key: String) -> CmdResult<()> {
    let w = win.arc().clone();
    let h = app.clone();
    crate::sync::blocking("removing the video", move || {
        let st = state(&h)?;
        let folder = folder_of(&w)?;
        let key = key_of_request(&key)?;
        st.edit(|| -> CmdResult<()> {
            if let Ok(r) = folder.load(key) {
                super::source::forget(&w, &r)?;
            }
            folder.remove(key).map_err(cmd)
        })
    })
    .await
}

fn frame_indices(requested: &[u32], r: &VideoRecord, redo: bool) -> CmdResult<Vec<usize>> {
    let all: Vec<usize> = if requested.is_empty() {
        (0..r.frames.len()).collect()
    } else {
        requested.iter().map(|i| index(*i, r)).collect::<CmdResult<_>>()?
    };
    Ok(all
        .into_iter()
        .filter(|i| redo || !requested.is_empty() || r.frames[*i].description.is_none())
        .collect())
}

/// The assistant for a job of this window (loaded model, the job's cancel flag).
fn assistant<R: Runtime>(
    app: &AppHandle<R>,
    w: &Arc<WindowState>,
    ctx: &JobCtx,
) -> CmdResult<debrief_ai::assistant::Assistant<Arc<dyn debrief_ai::llm::LlmProvider>>> {
    let core = crate::ai::commands::core(app)?;
    core.assistant(&crate::ai::knowledge::for_window(w, &Default::default()), ctx.cancel_flag())
}

/// Describes frames with the loaded vision model (a job; each description is saved as it arrives, so Cancel keeps
/// what is done). Without `indices`, the frames that have no description yet.
#[tauri::command]
pub async fn video_describe<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, request: FramesRequest) -> CmdResult<VideoView> {
    let w = win.arc().clone();
    let key = key_of_request(&request.key)?.to_string();
    let h = app.clone();
    let spec = JobSpec::new(JobKind::Ai, "Describing the frames of a video").window(win.label());
    jobs::run(&app, spec, Controls::default(), move |ctx: &JobCtx| -> CmdResult<Done<VideoView>> {
        let st = state(&h)?;
        let folder = folder_of(&w)?;
        let first = folder.load(&key).map_err(cmd)?;
        let todo = frame_indices(&request.indices, &first, request.redo)?;
        let a = assistant(&h, &w, ctx)?;
        let (mut failed, mut last_error) = (0usize, None);
        for (n, i) in todo.iter().enumerate() {
            if ctx.is_cancelled() {
                break;
            }
            ctx.message(&format!("Frame {} of {}", n + 1, todo.len()));
            ctx.progress(JobProgress::fraction(n as f32 / todo.len().max(1) as f32));
            let r = st.edit(|| folder.load(&key)).map_err(cmd)?;
            let path = folder.frame_path(&r, *i).map_err(cmd)?;
            let ms = r.frames[*i].video_ms;
            let described = load_frame(&path).and_then(|img| describe_frame(&a, &img, &r.name, *i + 1, r.frames.len(), ms));
            match described {
                Ok(text) => {
                    edit_record(&st, &folder, &key, |r| {
                        if let Some(f) = r.frames.get_mut(*i).filter(|f| f.video_ms == ms) {
                            f.description = Some(text);
                        }
                        Ok(())
                    })?;
                }
                // The model cannot see, is gone or was cancelled: stop and say so.
                Err(e @ (debrief_video::VideoError::AiUnavailable { .. } | debrief_video::VideoError::Cancelled)) => {
                    if matches!(e, debrief_video::VideoError::Cancelled) {
                        break;
                    }
                    return Err(cmd(e));
                }
                Err(e) => {
                    failed += 1;
                    last_error = Some(e);
                }
            }
        }
        if failed > 0 && failed == todo.len() {
            return Err(last_error
                .map(cmd)
                .unwrap_or_else(|| CmdError::failed("no frame could be described")));
        }
        let record = folder.load(&key).map_err(cmd)?;
        let id = replace(&w, ctx, &ctx.cancel_flag(), &record)?;
        Ok(Done::new(view_of(&folder, record, id)).cancelled_if(ctx.is_cancelled()))
    })
    .await
}

/// Has the model read the clock on one frame and aligns the video by it (a job).
#[tauri::command]
pub async fn video_read_clock<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, frame: FrameRef) -> CmdResult<VideoView> {
    let w = win.arc().clone();
    let key = key_of_request(&frame.key)?.to_string();
    let h = app.clone();
    let spec = JobSpec::new(JobKind::Ai, "Reading the clock on a video frame").window(win.label());
    jobs::run(&app, spec, Controls::default(), move |ctx: &JobCtx| -> CmdResult<Done<VideoView>> {
        let st = state(&h)?;
        let folder = folder_of(&w)?;
        let r = folder.load(&key).map_err(cmd)?;
        let i = index(frame.index, &r)?;
        let ms = r.frames[i].video_ms;
        let a = assistant(&h, &w, ctx)?;
        let image = load_frame(&folder.frame_path(&r, i).map_err(cmd)?).map_err(cmd)?;
        let reading = read_clock(&a, &image, ms).map_err(cmd)?;
        if reading.is_none() {
            return Err(CmdError::invalid("the model found no clock on this frame").hint("Try a frame that shows the time."));
        }
        let tz = timezone();
        let record = edit_record(&st, &folder, &key, |r| {
            r.clock = reading;
            r.realign(&tz);
            Ok(())
        })?;
        let id = replace(&w, ctx, &ctx.cancel_flag(), &record)?;
        Ok(Done::new(view_of(&folder, record, id)))
    })
    .await
}

/// Explains up to four frames of a video together (a job; the text is shown, not kept).
#[tauri::command]
pub async fn video_explain<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, request: ExplainRangeRequest) -> CmdResult<FrameExplanation> {
    if request.indices.is_empty() || request.indices.len() > MAX_EXPLAIN {
        return Err(CmdError::invalid(format!("choose between 1 and {MAX_EXPLAIN} frames")).field("indices"));
    }
    let w = win.arc().clone();
    let key = key_of_request(&request.key)?.to_string();
    let h = app.clone();
    let spec = JobSpec::new(JobKind::Ai, "Explaining video frames").window(win.label());
    jobs::run(
        &app,
        spec,
        Controls::default(),
        move |ctx: &JobCtx| -> CmdResult<Done<FrameExplanation>> {
            let folder = folder_of(&w)?;
            let r = folder.load(&key).map_err(cmd)?;
            let mut images = Vec::new();
            let mut frames = Vec::new();
            for i in &request.indices {
                let i = index(*i, &r)?;
                images.push(load_frame(&folder.frame_path(&r, i).map_err(cmd)?).map_err(cmd)?);
                frames.push((i + 1, r.frames[i].video_ms));
            }
            let a = assistant(&h, &w, ctx)?;
            let text = explain_range(&a, &images, &r.name, &frames).map_err(cmd)?;
            Ok(Done::new(FrameExplanation { text }))
        },
    )
    .await
}

/// The path of the video a window may play, for the `media:` scheme; `None` when it is not in the window's project.
pub(crate) fn playable(win: &WindowState, key: &str) -> Option<(std::path::PathBuf, String)> {
    if !is_key(key) {
        return None;
    }
    let folder = folder_of(win).ok()?;
    let r = folder.load(key).ok()?;
    let (p, _) = media_file(&folder, &r)?;
    Some((p, r.name))
}
