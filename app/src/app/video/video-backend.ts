import { inject, Provider } from '@angular/core';
import { invoke as tauriInvoke } from '../core/activity/ipc';
import { AiBackend } from '../ai/ai-backend';
import { AiImage } from '../ai/ai.types';
import { reportBackendFailures } from '../core/failure-reporter';
import {
  AddFrameRequest,
  AlignRequest,
  ExplainRangeRequest,
  FrameExplanation,
  FrameImage,
  FrameRef,
  FramesRequest,
  IncludeRequest,
  NoteRequest,
  OpenVideoRequest,
  VideoList,
  VideoRuntimeView,
  VideoView,
} from '../core/generated/video';
import { Logger, withFailureLogging } from '../core/logger';
import { browserMocks } from '../core/mock-registry';
import { isTauri } from '../core/platform';

/**
 * The video engine as the UI sees it (commands in `app/src-tauri/src/video.rs`, ADR 0033). The engine analyses
 * videos with FFmpeg, chooses and stores the frames, decides the alignment and talks to the model; views only
 * present it. Queries have no side effects. Analysis, description and clock reading run as jobs (the Jobs pane shows
 * progress and Cancel): their promises resolve when the job is done. Every method rejects with a `CmdError`.
 */
export abstract class VideoBackend {
  // ---- The FFmpeg runtime ----
  abstract getRuntime(): Promise<VideoRuntimeView>;
  /** Downloads, verifies and self-tests the pinned build (a download job). */
  abstract installRuntime(): Promise<VideoRuntimeView>;
  abstract rollbackRuntime(): Promise<VideoRuntimeView>;
  abstract deleteRuntime(tag: string): Promise<VideoRuntimeView>;
  /** Uses an ffmpeg executable (or its folder); the engine runs the self-test and rejects with the reason. */
  abstract setCustomRuntime(path: string): Promise<VideoRuntimeView>;
  abstract clearCustomRuntime(): Promise<VideoRuntimeView>;

  // ---- Videos ----
  /** The paths that are videos (by signature), without opening anything: dropped files are routed with it. */
  abstract sniff(paths: string[]): Promise<string[]>;
  abstract list(): Promise<VideoList>;
  /** Analyses a video the user chose (an analysis job; may take minutes). */
  abstract open(request: OpenVideoRequest): Promise<VideoView>;
  /** Forgets the video, its frames, notes and descriptions (and an included copy); never the original file. */
  abstract remove(key: string): Promise<void>;
  abstract include(request: IncludeRequest): Promise<VideoView>;

  // ---- Frames ----
  abstract getFrameImage(frame: FrameRef): Promise<FrameImage>;
  abstract addFrame(request: AddFrameRequest): Promise<VideoView>;
  abstract removeFrame(frame: FrameRef): Promise<VideoView>;
  abstract setNote(request: NoteRequest): Promise<VideoView>;
  /** The model describes frames (a job); no indices means every frame without a description. */
  abstract generateDescriptions(request: FramesRequest): Promise<VideoView>;
  /** Up to four frames explained in one request. */
  abstract explainFrames(request: ExplainRangeRequest): Promise<FrameExplanation>;
  /** The clean frame as an image of this window's image cache, with its aligned time as evidence. */
  abstract frameToImage(frame: FrameRef): Promise<AiImage>;

  // ---- Alignment ----
  abstract setAlignment(request: AlignRequest): Promise<VideoView>;
  /** The vision model reads an on-screen clock in the frame to align the video (a job). */
  abstract alignFromClock(frame: FrameRef): Promise<VideoView>;
}

/** Talks to the Rust shell; `invoke` is Tauri's (tests pass a fake). */
export class TauriVideoBackend extends VideoBackend {
  constructor(private readonly invoke: typeof tauriInvoke = tauriInvoke) {
    super();
  }

  getRuntime() {
    return this.invoke<VideoRuntimeView>('video_runtime');
  }
  installRuntime() {
    return this.invoke<VideoRuntimeView>('video_runtime_install');
  }
  rollbackRuntime() {
    return this.invoke<VideoRuntimeView>('video_runtime_rollback');
  }
  deleteRuntime(tag: string) {
    return this.invoke<VideoRuntimeView>('video_runtime_delete', { tag });
  }
  setCustomRuntime(path: string) {
    return this.invoke<VideoRuntimeView>('video_runtime_set_custom', { path });
  }
  clearCustomRuntime() {
    return this.invoke<VideoRuntimeView>('video_runtime_clear_custom');
  }
  sniff(paths: string[]) {
    return this.invoke<string[]>('video_sniff', { paths });
  }
  list() {
    return this.invoke<VideoList>('video_list');
  }
  open(request: OpenVideoRequest) {
    return this.invoke<VideoView>('video_open', { request });
  }
  remove(key: string) {
    return this.invoke<void>('video_remove', { key });
  }
  include(request: IncludeRequest) {
    return this.invoke<VideoView>('video_include', { request });
  }
  getFrameImage(frame: FrameRef) {
    return this.invoke<FrameImage>('video_frame_image', { frame });
  }
  addFrame(request: AddFrameRequest) {
    return this.invoke<VideoView>('video_add_frame', { request });
  }
  removeFrame(frame: FrameRef) {
    return this.invoke<VideoView>('video_remove_frame', { frame });
  }
  setNote(request: NoteRequest) {
    return this.invoke<VideoView>('video_set_note', { request });
  }
  generateDescriptions(request: FramesRequest) {
    return this.invoke<VideoView>('video_describe', { request });
  }
  explainFrames(request: ExplainRangeRequest) {
    return this.invoke<FrameExplanation>('video_explain', { request });
  }
  frameToImage(frame: FrameRef) {
    return this.invoke<AiImage>('video_frame_to_image', { frame });
  }
  setAlignment(request: AlignRequest) {
    return this.invoke<VideoView>('video_set_alignment', { request });
  }
  alignFromClock(frame: FrameRef) {
    return this.invoke<VideoView>('video_read_clock', { frame });
  }
}

/** The browser build's engine: its frame images go to the AI mock's image cache, like the engine's do. */
function mockVideoBackend(): VideoBackend {
  const mocks = browserMocks();
  const ai = inject(AiBackend);
  return new mocks.MockVideoBackend({
    ...mocks.videoOptionsFromSearch(typeof location === 'undefined' ? '' : location.search),
    registerImage: (name) => ai.imagePaste(PNG_SIGNATURE_BASE64, name),
  });
}

/** The first bytes of a PNG, which the AI mock accepts as an image. */
const PNG_SIGNATURE_BASE64 = 'iVBORw0KGgoAAAAN';

/** The real backend inside Tauri, the in-memory mock in a browser (dev server, e2e). */
export function createVideoBackend(): VideoBackend {
  const real = isTauri() ? new TauriVideoBackend() : mockVideoBackend();
  return withFailureLogging<VideoBackend>(real, inject(Logger), { onFailure: reportBackendFailures() });
}

export function provideVideoBackend(): Provider[] {
  return [{ provide: VideoBackend, useFactory: createVideoBackend }];
}
