import { inject, Provider } from '@angular/core';
import { invoke as tauriInvoke } from '../core/activity/ipc';
import { jobValue } from '../jobs/job-value';
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
  /** The command starts a job and returns its id; the result is the job's value. */
  async installRuntime() {
    const job = await this.invoke<number>('video_runtime_install');
    return jobValue<VideoRuntimeView>(this.invoke, job);
  }
  rollbackRuntime() {
    return this.invoke<VideoRuntimeView>('video_runtime_rollback');
  }
  deleteRuntime(tag: string) {
    return this.invoke<VideoRuntimeView>('video_runtime_delete', { tag });
  }
  /** The command starts a job and returns its id; the result is the job's value. */
  async setCustomRuntime(path: string) {
    const job = await this.invoke<number>('video_runtime_set_custom', { path });
    return jobValue<VideoRuntimeView>(this.invoke, job);
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
  /** The command starts a job and returns its id; the result is the job's value. */
  async open(request: OpenVideoRequest) {
    const job = await this.invoke<number>('video_open', { request });
    return jobValue<VideoView>(this.invoke, job);
  }
  remove(key: string) {
    return this.invoke<void>('video_remove', { key });
  }
  /** The command starts a job and returns its id; the result is the job's value. */
  async include(request: IncludeRequest) {
    const job = await this.invoke<number>('video_include', { request });
    return jobValue<VideoView>(this.invoke, job);
  }
  getFrameImage(frame: FrameRef) {
    return this.invoke<FrameImage>('video_frame_image', { frame });
  }
  /** The command starts a job and returns its id; the result is the job's value. */
  async addFrame(request: AddFrameRequest) {
    const job = await this.invoke<number>('video_add_frame', { request });
    return jobValue<VideoView>(this.invoke, job);
  }
  removeFrame(frame: FrameRef) {
    return this.invoke<VideoView>('video_remove_frame', { frame });
  }
  setNote(request: NoteRequest) {
    return this.invoke<VideoView>('video_set_note', { request });
  }
  /** The command starts a job and returns its id; the result is the job's value. */
  async generateDescriptions(request: FramesRequest) {
    const job = await this.invoke<number>('video_describe', { request });
    return jobValue<VideoView>(this.invoke, job);
  }
  /** The command starts a job and returns its id; the result is the job's value. */
  async explainFrames(request: ExplainRangeRequest) {
    const job = await this.invoke<number>('video_explain', { request });
    return jobValue<FrameExplanation>(this.invoke, job);
  }
  frameToImage(frame: FrameRef) {
    return this.invoke<AiImage>('video_frame_to_image', { frame });
  }
  setAlignment(request: AlignRequest) {
    return this.invoke<VideoView>('video_set_alignment', { request });
  }
  /** The command starts a job and returns its id; the result is the job's value. */
  async alignFromClock(frame: FrameRef) {
    const job = await this.invoke<number>('video_read_clock', { frame });
    return jobValue<VideoView>(this.invoke, job);
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
