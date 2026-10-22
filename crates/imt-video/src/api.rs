//! The wire types between the shell and the UI for video (camelCase JSON). With the `ts` feature they derive ts-rs and
//! the app's `generated_types` test writes them to `app/src/app/core/generated/video.ts` (ADR 0016). The records
//! themselves (`VideoRecord`, `FrameRecord`) are the project's own file format and are sent as they are.

use crate::align::UserAlignment;
use crate::record::VideoRecord;
use crate::runtime::RuntimeStatus;
use crate::select::SelectOptions;
use serde::{Deserialize, Serialize};

/// Where the video file a record refers to can be played from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum MediaState {
    /// A copy in the project.
    Included,
    /// The file the video was opened from, still there with the same size.
    Referenced,
    /// Not found (moved, deleted, another machine): the frames and notes still work.
    Missing,
}

/// A video of the project as the Video view shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct VideoView {
    pub record: VideoRecord,
    pub media: MediaState,
    /// What the player loads (`media://localhost/<key>`), for a video that is playable.
    pub media_url: Option<String>,
    /// The source of its frames in the workspace (events on the timeline), when open.
    pub source_id: Option<u32>,
}

/// The project's videos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct VideoList {
    pub videos: Vec<VideoView>,
    /// Folders whose record could not be read (key, why).
    pub skipped: Vec<SkippedVideo>,
    /// Frames the project keeps over all videos.
    pub project_frames: u32,
    pub max_project_frames: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SkippedVideo {
    pub key: String,
    pub why: String,
}

/// An ffmpeg found on this machine that could be chosen (Homebrew's, a package manager's).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct DetectedFfmpeg {
    pub path: String,
    /// Where it comes from ("Homebrew").
    pub origin: String,
}

/// Settings ▸ Video.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct VideoRuntimeView {
    pub status: RuntimeStatus,
    /// `windows-x64`, `macos-arm64`, ...
    pub platform: String,
    /// IMT can download a pinned build for this platform (not macOS yet).
    pub can_download: bool,
    /// ffmpeg programs found in the usual places, for "Use this one".
    pub detected: Vec<DetectedFfmpeg>,
}

/// Opens (analyses) a video file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct OpenVideoRequest {
    pub path: String,
    #[serde(default)]
    pub options: SelectOptions,
    /// Include a copy of the video in the project.
    #[serde(default)]
    pub include: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AlignRequest {
    pub key: String,
    pub alignment: UserAlignment,
}

/// A frame of a video.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FrameRef {
    pub key: String,
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct NoteRequest {
    pub key: String,
    pub index: u32,
    /// `None` or empty removes the note.
    pub note: Option<String>,
}

/// Frames of a video (all when `indices` is empty).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FramesRequest {
    pub key: String,
    #[serde(default)]
    pub indices: Vec<u32>,
    /// Describe frames that already have a description again.
    #[serde(default)]
    pub redo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ExplainRangeRequest {
    pub key: String,
    /// Up to four frames, in time order.
    pub indices: Vec<u32>,
}

/// The first frame at or after `video_ms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AddFrameRequest {
    pub key: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub video_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct IncludeRequest {
    pub key: String,
    pub include: bool,
}

/// The clean image of a frame for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FrameImage {
    /// `data:image/png;base64,...`
    pub data_url: String,
}

/// An explanation of frames (text; the engine's answer, shown as is).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FrameExplanation {
    pub text: String,
}
