// Generated from the Rust wire types by `cargo test -p debrief-app --lib generated_types` (ts-rs; ADR 0016).
// Do not edit: change the Rust type and run the test with DEBRIEF_WRITE_TYPES=1.

export type UserAlignment = { "kind": "auto" } | { "kind": "start", startMs: number, } | { "kind": "offset", offsetMs: number, };

export type ClockReading = { 
/**
 * The frame's time in the video.
 */
frameMs: number, 
/**
 * The times the model saw, as shown.
 */
shown: Array<{ text: string; hasDate: boolean }>, };

export type AlignEvidence = { "kind": "userStart" } | { "kind": "userOffset", offsetMs: number, base: AlignEvidence, } | { "kind": "clock", frameMs: number, text: string, hasDate: boolean, } | { "kind": "fileName", text: string, } | { "kind": "container", createdMs: number, } | { "kind": "fileTime", modifiedMs: number, } | { "kind": "nothing" };

export type Alignment = { 
/**
 * Epoch ms (UTC) of video time 0.
 */
startMs: number, by: 'user' | 'clock' | 'fileName' | 'container' | 'fileTime' | 'none', confidence: 'low' | 'medium' | 'high', evidence: AlignEvidence, };

export type SelectMode = { "mode": "changes", threshold: number, } | { "mode": "interval", everyMs: number, };

export type SelectOptions = { mode: SelectMode, cap: number, };

export type SelectNote = { "kind": "thresholdRaised", from: number, to: number, } | { "kind": "spreadEvenly", leftOut: number, } | { "kind": "noChanges" };

export type FrameRecord = { 
/**
 * Its exact time in the video (ms), as FFmpeg's `showinfo` reported it.
 */
videoMs: number, 
/**
 * Its image, relative to the video's folder (`frames/0003.png`).
 */
image: string, width: number, height: number, 
/**
 * How much it differed from the frame before (FFmpeg's scene score, 0–1).
 */
score: number, description?: string | null, note?: string | null, 
/**
 * Added by the user (not chosen by the selection).
 */
manual: boolean, };

export type VideoRecord = { version: number, key: string, 
/**
 * The file name.
 */
name: string, sha256: string, size: number, 
/**
 * Where the file was on the machine that analysed it (it is referenced, not copied, by default).
 */
path: string | null, 
/**
 * A copy is in `media/` (and goes with the project).
 */
included: boolean, durationMs: number, width: number, height: number, codec: string, fps: number | null, creationMs: number | null, fileModifiedMs: number | null, options: SelectOptions, userAlignment: UserAlignment, clock: ClockReading | null, 
/**
 * The alignment as last decided (kept for display; decided again whenever its inputs change).
 */
alignment: Alignment, selectionNotes: Array<SelectNote>, frames: Array<FrameRecord>, 
/**
 * When it was analysed (epoch ms).
 */
analysedMs: number, };

export type InstallStage = "downloading" | "unpacking" | "testing" | "finishing";

export type RuntimeMode = "none" | "managed" | "custom";

export type InstalledRelease = { tag: string, 
/**
 * FFmpeg's version as its self-test reported it.
 */
version: string | null, bytes: number, };

export type RuntimeStatus = { mode: RuntimeMode, active: InstalledRelease | null, 
/**
 * The release an update replaced ("Roll back").
 */
previous: InstalledRelease | null, 
/**
 * The pinned release for this platform (none on macOS for now).
 */
pinnedTag: string | null, pinnedRelease: string | null, pinnedBytes: number | null, 
/**
 * The pinned release is not the active one.
 */
updateAvailable: boolean, 
/**
 * The user-provided ffmpeg, when one is set.
 */
customPath: string | null, customVersion: string | null, 
/**
 * Why the runtime in use cannot run (a missing binary), with what to do.
 */
problem: string | null, };

export type SelfTestReport = { 
/**
 * The version FFmpeg reports.
 */
version: string | null, 
/**
 * The build includes GPL components (a user-provided FFmpeg may; the pinned ones never do).
 */
gpl: boolean, };

export type MediaState = "included" | "referenced" | "missing";

export type VideoView = { record: VideoRecord, media: MediaState, 
/**
 * What the player loads (`media://localhost/<key>`), for a video that is playable.
 */
mediaUrl: string | null, 
/**
 * The source of its frames in the workspace (events on the timeline), when open.
 */
sourceId: number | null, };

export type VideoList = { videos: Array<VideoView>, 
/**
 * Folders whose record could not be read (key, why).
 */
skipped: Array<SkippedVideo>, 
/**
 * Frames the project keeps over all videos.
 */
projectFrames: number, maxProjectFrames: number, };

export type SkippedVideo = { key: string, why: string, };

export type DetectedFfmpeg = { path: string, 
/**
 * Where it comes from ("Homebrew").
 */
origin: string, };

export type VideoRuntimeView = { status: RuntimeStatus, 
/**
 * `windows-x64`, `macos-arm64`, ...
 */
platform: string, 
/**
 * Debrief can download a pinned build for this platform (not macOS yet).
 */
canDownload: boolean, 
/**
 * ffmpeg programs found in the usual places, for "Use this one".
 */
detected: Array<DetectedFfmpeg>, };

export type OpenVideoRequest = { path: string, options: SelectOptions, 
/**
 * Include a copy of the video in the project.
 */
include: boolean, };

export type AlignRequest = { key: string, alignment: UserAlignment, };

export type FrameRef = { key: string, index: number, };

export type NoteRequest = { key: string, index: number, 
/**
 * `None` or empty removes the note.
 */
note: string | null, };

export type FramesRequest = { key: string, indices: Array<number>, 
/**
 * Describe frames that already have a description again.
 */
redo: boolean, };

export type ExplainRangeRequest = { key: string, 
/**
 * Up to four frames, in time order.
 */
indices: Array<number>, };

export type AddFrameRequest = { key: string, videoMs: number, };

export type IncludeRequest = { key: string, include: boolean, };

export type FrameImage = { 
/**
 * `data:image/png;base64,...`
 */
dataUrl: string, };

export type FrameExplanation = { text: string, };
