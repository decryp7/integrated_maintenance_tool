//! Video sources (ADR 0033): the shell's side. The engine is `imt_video` (FFmpeg runtime, sandboxed passes, frame
//! selection, alignment, the records kept in the project); this module holds the app-wide runtime state
//! ([`VideoState`]), turns engine errors into [`CmdError`]s, builds the views the UI shows and keeps each video's
//! frames source in the window's workspace up to date ([`source`]). Commands: [`runtime_commands`] (Settings ▸ Video)
//! and [`commands`] (the Video view); [`media`] serves the player.
//!
//! Long work (installing the runtime, analysing a video, asking the model) runs as jobs in the Jobs queue.

pub(crate) mod commands;
pub(crate) mod media;
pub(crate) mod runtime_commands;
pub(crate) mod source;

use crate::error::{CmdError, ErrorCode};
use crate::windows::WindowState;
use imt_video::api::{MediaState, VideoView};
use imt_video::AiUnavailable;
use imt_video::ingest::media_file;
use imt_video::record::{VideoFolder, VideoRecord};
use imt_video::runtime::{FfmpegCatalog, FfmpegStore};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Runtime};

/// Memory an FFmpeg process may use for a video pass.
pub(crate) const FFMPEG_MEMORY: u64 = 1 << 30;

/// Where the player loads a video from: the `media` scheme (a plain host on Windows, where WebView2 maps custom
/// schemes to `http://<scheme>.localhost`).
pub(crate) const MEDIA_BASE: &str = if cfg!(windows) {
    "http://media.localhost/"
} else {
    "media://localhost/"
};

/// App-wide video state: the FFmpeg runtimes on disk and the lock that serialises changes to a video record.
pub(crate) struct VideoState {
    pub(crate) ffmpeg: FfmpegStore,
    pub(crate) catalog: FfmpegCatalog,
    /// Scratch folders of passes and self-tests (emptied at start).
    pub(crate) scratch: PathBuf,
    edit: Mutex<()>,
}

impl VideoState {
    pub(crate) fn new(root: PathBuf) -> VideoState {
        let scratch = root.join("scratch");
        let _ = std::fs::remove_dir_all(&scratch);
        VideoState {
            ffmpeg: FfmpegStore::new(root.join("ffmpeg")),
            catalog: FfmpegCatalog::embedded(),
            scratch,
            edit: Mutex::new(()),
        }
    }

    /// Runs `f` while no other change to a video record is in progress (a description arriving while the user types a
    /// note must not overwrite it).
    pub(crate) fn edit<T>(&self, f: impl FnOnce() -> T) -> T {
        let _g = crate::sync::lock(&self.edit);
        f()
    }

    /// A fresh scratch folder for one pass; removed when dropped.
    pub(crate) fn scratch_dir(&self, what: &str) -> std::io::Result<Scratch> {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = self.scratch.join(format!("{what}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        Ok(Scratch(dir))
    }
}

/// A scratch folder, removed with its guard.
pub(crate) struct Scratch(PathBuf);

impl Scratch {
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Registers the video state (call from `setup`). Never fails: without a data folder the runtime lives in the
/// temporary one.
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let data = crate::app_dirs::data(app).unwrap_or_else(|_| std::env::temp_dir().join("imt-data"));
    app.manage(VideoState::new(data.join("video")));
}

/// The app's video state.
pub(crate) fn state<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::State<'_, VideoState>, CmdError> {
    app.try_state::<VideoState>()
        .ok_or_else(|| CmdError::unavailable("video support is not ready yet"))
}

/// An engine error as the one command error.
pub(crate) fn cmd(e: VideoError) -> CmdError {
    use imt_video::VideoError as V;
    let msg = e.to_string();
    match e {
        V::Invalid(_) => CmdError::invalid(msg),
        V::NotFound(_) => CmdError::not_found(msg),
        V::NoRuntime(_) => CmdError::new(ErrorCode::Missing, msg).hint("Set up FFmpeg in Settings ▸ Video."),
        V::Integrity(_) => CmdError::new(ErrorCode::Integrity, msg),
        V::SelfTest { .. } => CmdError::new(ErrorCode::SelfTestFailed, msg),
        V::Tool(_) => CmdError::failed(msg),
        V::Timeout(_) => CmdError::new(ErrorCode::Timeout, msg),
        V::OverMemory(_) => CmdError::new(ErrorCode::Memory, msg),
        V::Limit(_) => CmdError::new(ErrorCode::Limit, msg),
        V::Cancelled => CmdError::new(ErrorCode::Cancelled, msg),
        V::Io(_) => CmdError::new(ErrorCode::Io, msg),
        V::AiUnavailable { kind, .. } => CmdError::new(
            match kind {
                V::AiUnavailable::NoModel => ErrorCode::NoModel,
                V::AiUnavailable::NoRuntime => ErrorCode::NoRuntime,
                V::AiUnavailable::NoVision => ErrorCode::NoVision,
            },
            msg,
        ),
        V::Ai(_) => CmdError::new(ErrorCode::Engine, msg),
    }
}

/// The window's project folder videos live in (opens the window's store when it is not open yet).
pub(crate) fn folder_of(win: &WindowState) -> Result<VideoFolder, CmdError> {
    Ok(VideoFolder::of_project(&win.engine.store()?.store_dir()))
}

/// The zone for times shown without one: the generic parser's (formats are parsers, ADR 0026).
pub(crate) fn timezone() -> String {
    imt_core::parser::builtin::generic().format.assume_timezone
}

/// A record as the Video view shows it.
pub(crate) fn view_of(folder: &VideoFolder, record: VideoRecord, source_id: Option<u32>) -> VideoView {
    let media = match media_file(folder, &record) {
        Some((_, true)) => MediaState::Included,
        Some((_, false)) => MediaState::Referenced,
        None => MediaState::Missing,
    };
    VideoView {
        media_url: (media != MediaState::Missing).then(|| format!("{MEDIA_BASE}{}", record.key)),
        record,
        media,
        source_id,
    }
}

#[cfg(test)]
mod tests;
