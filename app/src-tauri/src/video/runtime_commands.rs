//! Settings ▸ Video (ADR 0033 §1): the FFmpeg runtime. Install (a download job: pinned build, SHA-256, unpack,
//! self-test, then switch), roll back, delete, and use the user's own ffmpeg (Homebrew's, a package manager's). The
//! self-test runs the whole sandboxed pipeline on a clip Debrief generates; a runtime that fails it is never used.

use super::{cmd, state, VideoState};
use crate::error::{CmdError, CmdResult};
use crate::jobs::{self, Controls, Done, JobCtx, JobKind, JobProgress, JobSpec};
use debrief_ai::models::{Progress, SystemDisk};
use debrief_ai::runtimes::{GithubReleases, ReleaseConfig};
use debrief_video::api::VideoRuntimeView;
use debrief_video::run::Tools;
use debrief_video::runtime::{current_platform, detect, self_test, SelfTestReport};
use debrief_video::VideoError;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Runtime};

/// How often the download reports progress to the job.
const TICK: Duration = Duration::from_millis(250);
/// Longest path accepted for a user-provided ffmpeg.
const MAX_PATH: usize = 4096;

fn view(st: &VideoState) -> VideoRuntimeView {
    let platform = current_platform();
    VideoRuntimeView {
        status: st.ffmpeg.status(&st.catalog, &platform),
        can_download: st.catalog.asset(&platform).is_some(),
        detected: detect(std::env::var_os("PATH").as_deref()),
        platform,
    }
}

/// The self-test as the store wants it: in a scratch folder of its own, with the job's cancel flag.
fn check<'a>(st: &'a VideoState, cancel: &'a AtomicBool) -> impl Fn(&Tools) -> debrief_video::Result<SelfTestReport> + 'a {
    move |tools| {
        let scratch = st.scratch_dir("selftest")?;
        self_test(tools, scratch.path(), cancel)
    }
}

/// The runtime as Settings ▸ Video shows it.
#[tauri::command]
pub async fn video_runtime<R: Runtime>(app: AppHandle<R>) -> CmdResult<VideoRuntimeView> {
    let h = app.clone();
    crate::sync::blocking("looking for FFmpeg", move || Ok::<_, CmdError>(view(&*state(&h)?))).await
}

/// Downloads, verifies and self-tests the pinned build for this computer and makes it the one in use, as a download
/// job (progress and Cancel in the Jobs pane). Not offered where no build is pinned (macOS): use your own ffmpeg.
#[tauri::command]
pub async fn video_runtime_install<R: Runtime>(app: AppHandle<R>) -> CmdResult<VideoRuntimeView> {
    let platform = current_platform();
    let h = app.clone();
    {
        let st = state(&app)?;
        if st.catalog.asset(&platform).is_none() {
            return Err(CmdError::new(
                crate::ErrorCode::Unsupported,
                "Debrief has no FFmpeg build to download for this computer yet",
            )
            .hint("Install FFmpeg yourself (on macOS: brew install ffmpeg) and choose it in Settings ▸ Video."));
        }
    }
    let proxy = crate::ai::commands::core(&app).ok().and_then(|c| c.settings().download_proxy);
    let progress = Arc::new(Progress::new());
    let (p, q) = (progress.clone(), progress.clone());
    let controls = Controls::cancel(move || p.cancel()).with_probe(move || {
        let s = q.snapshot();
        (s.total > 0).then(|| JobProgress::bytes(s.bytes, s.total))
    });
    let spec = JobSpec::new(JobKind::Download, "Installing the FFmpeg video runtime");
    jobs::run(&app, spec, controls, move |ctx: &JobCtx| -> CmdResult<Done<VideoRuntimeView>> {
        let st = state(&h)?;
        let source = GithubReleases::new(
            ReleaseConfig {
                proxy,
                ..ReleaseConfig::default()
            },
            st.catalog.as_runtime_catalog(),
        )
        .map_err(|e| cmd(e.into()))?;
        let cancel = AtomicBool::new(false);
        let result = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let test = check(&st, &cancel);
                st.ffmpeg
                    .install(&st.catalog, &platform, &source, &SystemDisk, &progress, &mut |_| {}, &test)
            });
            while !worker.is_finished() {
                std::thread::sleep(TICK);
                if ctx.is_cancelled() {
                    progress.cancel();
                    cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
            worker
                .join()
                .unwrap_or_else(|_| Err(VideoError::Io("the FFmpeg install stopped unexpectedly".into())))
        });
        match &result {
            Ok(r) => log::info!(Tag = r.tag.as_str(); "FFmpeg runtime installed"),
            Err(e) => log::warn!("FFmpeg runtime not installed: {e}"),
        }
        result.map_err(cmd)?;
        Ok(Done::new(view(&st)))
    })
    .await
}

/// Goes back to the release before the last update.
#[tauri::command]
pub async fn video_runtime_rollback<R: Runtime>(app: AppHandle<R>) -> CmdResult<VideoRuntimeView> {
    let h = app.clone();
    crate::sync::blocking("rolling FFmpeg back", move || {
        let st = state(&h)?;
        let cancel = AtomicBool::new(false);
        st.ffmpeg.rollback(&check(&st, &cancel)).map_err(cmd)?;
        Ok::<_, CmdError>(view(&st))
    })
    .await
}

/// Deletes an installed release.
#[tauri::command]
pub async fn video_runtime_delete<R: Runtime>(app: AppHandle<R>, tag: String) -> CmdResult<VideoRuntimeView> {
    if tag.is_empty() || tag.len() > 64 {
        return Err(CmdError::invalid("that is not an FFmpeg release"));
    }
    let h = app.clone();
    crate::sync::blocking("deleting the FFmpeg release", move || {
        let st = state(&h)?;
        st.ffmpeg.delete(&tag).map_err(cmd)?;
        Ok::<_, CmdError>(view(&st))
    })
    .await
}

/// Uses the user's own ffmpeg (a file or its folder): it must pass the self-test, otherwise nothing changes and the
/// error says why. `path` is one the user chose in a dialog or one of the detected ones.
#[tauri::command]
pub async fn video_runtime_set_custom<R: Runtime>(app: AppHandle<R>, path: String) -> CmdResult<VideoRuntimeView> {
    if path.trim().is_empty() || path.len() > MAX_PATH {
        return Err(CmdError::invalid("choose the ffmpeg program or the folder it is in"));
    }
    let h = app.clone();
    let path = PathBuf::from(path.trim());
    jobs::run(
        &app,
        JobSpec::new(JobKind::Analysis, "Checking the chosen FFmpeg"),
        Controls::default(),
        move |ctx: &JobCtx| -> CmdResult<Done<VideoRuntimeView>> {
            let st = state(&h)?;
            let flag = ctx.cancel_flag();
            st.ffmpeg.set_custom(&path, &check(&st, &flag)).map_err(cmd)?;
            Ok(Done::new(view(&st)))
        },
    )
    .await
}

/// Stops using the user's own ffmpeg.
#[tauri::command]
pub async fn video_runtime_clear_custom<R: Runtime>(app: AppHandle<R>) -> CmdResult<VideoRuntimeView> {
    let h = app.clone();
    crate::sync::blocking("clearing the FFmpeg choice", move || {
        let st = state(&h)?;
        st.ffmpeg.clear_custom().map_err(cmd)?;
        Ok::<_, CmdError>(view(&st))
    })
    .await
}
