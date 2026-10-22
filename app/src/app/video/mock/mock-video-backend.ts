import { AiImage } from '../../ai/ai.types';
import { CmdError } from '../../core/cmd-error';
import { ErrorCode } from '../../core/generated/errors';
import {
  AddFrameRequest,
  AlignEvidence,
  Alignment,
  AlignRequest,
  ExplainRangeRequest,
  FrameExplanation,
  FrameImage,
  FrameRecord,
  FrameRef,
  FramesRequest,
  IncludeRequest,
  InstalledRelease,
  NoteRequest,
  OpenVideoRequest,
  SelectNote,
  VideoList,
  VideoRecord,
  VideoRuntimeView,
  VideoView,
} from '../../core/generated/video';
import { VideoBackend } from '../video-backend';

/** The demo videos the browser build opens (File ▸ Open video…): one playable, one that moved away. */
export const MOCK_VIDEO_PATHS = ['/Users/demo/Recordings/checkout-session.mp4', '/Volumes/OldDisk/Recordings/moved-recording.mov'];
/** Where "Locate the video…" finds the moved one. */
export const MOCK_LOCATED_PATH = '/Users/demo/Recordings/moved-recording.mov';
/** Paths here are "not there" for the mock (an unplugged disk). */
const GONE = '/Volumes/OldDisk/';
/** The ffmpeg the browser build "chooses". */
export const MOCK_FFMPEG_PATH = '/opt/homebrew/bin/ffmpeg';

export const MAX_VIDEO_FRAMES = 120;
export const MAX_PROJECT_FRAMES = 600;
export const MAX_TEXT = 2000;

const SESSION_START = Date.UTC(2026, 9, 14, 10, 40, 0);
const fail = (code: ErrorCode, message: string, extra: { hint?: string; retryable?: boolean } = {}) =>
  new CmdError({ code, message, retryable: extra.retryable ?? false, ...(extra.hint ? { hint: extra.hint } : {}) });
const name = (path: string) => path.split(/[\\/]/).pop() ?? path;

/** Stable 16 hex digits of a name, like the first digits of a SHA-256. */
export function mockKey(file: string): string {
  let h1 = 0xdeadbeef;
  let h2 = 0x41c6ce57;
  for (let i = 0; i < file.length; i++) {
    h1 = Math.imul(h1 ^ file.charCodeAt(i), 2654435761);
    h2 = Math.imul(h2 ^ file.charCodeAt(i), 1597334677);
  }
  const hex = (n: number) => (n >>> 0).toString(16).padStart(8, '0');
  return hex(h1) + hex(h2);
}

/** A frame's picture: a small SVG with its number and time (the real engine returns PNG). */
export function frameSvg(title: string, index: number, videoMs: number): string {
  const safe = title.replace(/[^A-Za-z0-9 ._-]/g, '').slice(0, 40);
  const hue = (index * 47) % 360;
  const svg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180" viewBox="0 0 320 180">' +
    `<rect width="320" height="180" fill="hsl(${hue} 35% 22%)"/>` +
    '<rect x="24" y="30" width="272" height="120" rx="6" fill="#f4f4f4" opacity="0.92"/>' +
    `<text x="36" y="56" font-family="sans-serif" font-size="14" fill="#222">${safe}</text>` +
    `<text x="36" y="84" font-family="sans-serif" font-size="12" fill="#444">Frame ${index + 1} at ${(videoMs / 1000).toFixed(1)} s</text>` +
    `<rect x="36" y="104" width="${40 + ((index * 37) % 160)}" height="10" rx="3" fill="hsl(${hue} 60% 45%)"/>` +
    '</svg>';
  return `data:image/svg+xml;base64,${btoa(svg)}`;
}

const DESCRIPTIONS = [
  'The shop home page with an empty basket.',
  'The basket page lists two items and a total.',
  'The payment form is filled in; the Pay button is highlighted.',
  'A spinner covers the payment form.',
  'An error dialog says the payment could not be completed.',
  'The basket page again, with a banner asking to try later.',
];

function frames(times: number[]): FrameRecord[] {
  return times.map((videoMs, i) => ({
    videoMs,
    image: `frames/${String(i + 1).padStart(4, '0')}.png`,
    width: 1280,
    height: 720,
    score: i === 0 ? 0 : 0.12 + ((i * 7) % 5) / 20,
    description: null,
    note: null,
    manual: false,
  }));
}

function decide(r: VideoRecord): Alignment {
  const base: Alignment = r.creationMs !== null
    ? { startMs: r.creationMs, by: 'container', confidence: 'medium', evidence: { kind: 'container', createdMs: r.creationMs } }
    : r.fileModifiedMs !== null
      ? { startMs: r.fileModifiedMs - r.durationMs, by: 'fileTime', confidence: 'low', evidence: { kind: 'fileTime', modifiedMs: r.fileModifiedMs } }
      : { startMs: 0, by: 'none', confidence: 'low', evidence: { kind: 'nothing' } };
  const u = r.userAlignment;
  if (u.kind === 'start') return { startMs: u.startMs, by: 'user', confidence: 'high', evidence: { kind: 'userStart' } };
  const from = r.clock?.shown[0];
  const auto: Alignment =
    r.clock && from
      ? {
          startMs: clockMs(r, from.text) - r.clock.frameMs,
          by: 'clock',
          confidence: from.hasDate ? 'high' : 'medium',
          evidence: { kind: 'clock', frameMs: r.clock.frameMs, text: from.text, hasDate: from.hasDate },
        }
      : base;
  if (u.kind === 'offset') return { startMs: auto.startMs + u.offsetMs, by: 'user', confidence: 'high', evidence: { kind: 'userOffset', offsetMs: u.offsetMs, base: auto.evidence as AlignEvidence } };
  return auto;
}

/** The day of the video with the time of day read from the clock. */
function clockMs(r: VideoRecord, text: string): number {
  const day = new Date(r.creationMs ?? r.fileModifiedMs ?? SESSION_START);
  const [h = 0, m = 0, s = 0] = text.split(':').map(Number);
  return Date.UTC(day.getUTCFullYear(), day.getUTCMonth(), day.getUTCDate(), h, m, s);
}

function record(file: string, path: string, options: OpenVideoRequest['options'], included: boolean, times: number[], created: number): VideoRecord {
  const r: VideoRecord = {
    version: 1,
    key: mockKey(file),
    name: file,
    sha256: mockKey(file + '1') + mockKey(file + '2') + mockKey(file + '3') + mockKey(file + '4'),
    size: 48_000_000 + file.length * 1_000_003,
    path,
    included,
    durationMs: 36_000,
    width: 1280,
    height: 720,
    codec: 'h264',
    fps: 30,
    creationMs: created,
    fileModifiedMs: created + 36_000,
    options,
    userAlignment: { kind: 'auto' },
    clock: null,
    alignment: { startMs: created, by: 'container', confidence: 'medium', evidence: { kind: 'container', createdMs: created } },
    selectionNotes: [],
    frames: frames(times),
    analysedMs: SESSION_START + 3_600_000,
  };
  r.alignment = decide(r);
  return r;
}

/** What a mock engine can be asked to do differently (tests and e2e). */
export interface MockVideoOptions {
  /** The runtime: `custom` (Homebrew, the default), `none` on Windows (nothing installed, a pinned build to download), `broken`. */
  runtime?: 'custom' | 'windows' | 'windows-installed' | 'broken';
  /** The model reads images (default); false: describing frames and reading clocks fail with `noVision`. */
  vision?: boolean;
  /** Registers a frame image in the AI image cache of the window (the browser build wires the AI mock here). */
  registerImage?: (name: string) => Promise<AiImage>;
}

/**
 * Browser-demo switches read from the URL, for UI tests and manual checks:
 *   ?mockVideoRuntime=windows|windows-installed|broken   the FFmpeg runtime of another computer (default: Homebrew on macOS)
 *   ?mockVideoVision=off                                  the model reads text only
 */
export function videoOptionsFromSearch(search: string): MockVideoOptions {
  const p = new URLSearchParams(search);
  const options: MockVideoOptions = {};
  const runtime = p.get('mockVideoRuntime');
  if (runtime === 'windows' || runtime === 'windows-installed' || runtime === 'broken') options.runtime = runtime;
  if (p.get('mockVideoVision') === 'off') options.vision = false;
  return options;
}

const RELEASE: InstalledRelease = { tag: 'autobuild-2026-09-30', version: '8.0.1', bytes: 78_000_000 };
const OLDER: InstalledRelease = { tag: 'autobuild-2026-08-31', version: '7.1.2', bytes: 74_000_000 };

/**
 * An in-memory video engine for the browser build and the UI tests. It mimics the contract (what the commands accept,
 * how they fail), not the video analysis: a "video" is a name with six or five plausible frames.
 */
export class MockVideoBackend extends VideoBackend {
  private readonly videos = new Map<string, VideoRecord>();
  private runtime: VideoRuntimeView;
  private readonly vision: boolean;
  private readonly register: (name: string) => Promise<AiImage>;
  private imageIds = 100;

  constructor(options: MockVideoOptions = {}) {
    super();
    this.vision = options.vision ?? true;
    this.runtime = this.initialRuntime(options.runtime ?? 'custom');
    this.register =
      options.registerImage ??
      ((n) => {
        const id = ++this.imageIds;
        return Promise.resolve({ id, name: n, path: null, width: 1280, height: 720, originalWidth: 1280, originalHeight: 720, sourceFormat: 'png', dataUrl: frameSvg(n, 0, 0), capturedAt: null });
      });
  }

  private initialRuntime(kind: NonNullable<MockVideoOptions['runtime']>): VideoRuntimeView {
    const empty = { mode: 'none' as const, active: null, previous: null, pinnedTag: null, pinnedRelease: null, pinnedBytes: null, updateAvailable: false, customPath: null, customVersion: null, problem: null };
    const windows = { ...empty, pinnedTag: RELEASE.tag, pinnedRelease: 'FFmpeg 8.0 (LGPL, shared)', pinnedBytes: RELEASE.bytes };
    switch (kind) {
      case 'windows':
        return { status: windows, platform: 'windows-x64', canDownload: true, detected: [] };
      case 'windows-installed':
        return { status: { ...windows, mode: 'managed', active: OLDER, updateAvailable: true }, platform: 'windows-x64', canDownload: true, detected: [] };
      case 'broken':
        return { status: { ...windows, mode: 'managed', active: RELEASE, problem: 'ffmpeg.exe is missing from the downloaded build. Install it again.' }, platform: 'windows-x64', canDownload: true, detected: [] };
      case 'custom':
        return {
          status: { ...empty, mode: 'custom', customPath: MOCK_FFMPEG_PATH, customVersion: '7.1.1' },
          platform: 'macos-arm64',
          canDownload: false,
          detected: [{ path: MOCK_FFMPEG_PATH, origin: 'Homebrew' }],
        };
    }
  }

  private view(r: VideoRecord): VideoView {
    const media = r.path?.startsWith(GONE) && !r.included ? 'missing' : r.included ? 'included' : 'referenced';
    return { record: structuredClone(r), media, mediaUrl: null, sourceId: null };
  }

  private get(key: string): VideoRecord {
    const r = this.videos.get(key);
    if (!r) throw fail('notFound', 'This video is not in the project any more.');
    return r;
  }

  private frameOf(ref: FrameRef): { r: VideoRecord; f: FrameRecord } {
    const r = this.get(ref.key);
    const f = r.frames[ref.index];
    if (!f) throw fail('notFound', `The video has no frame ${ref.index + 1}.`);
    return { r, f };
  }

  private projectFrames(): number {
    return [...this.videos.values()].reduce((n, r) => n + r.frames.length, 0);
  }

  private decided(r: VideoRecord): VideoView {
    r.alignment = decide(r);
    return this.view(r);
  }

  // ---- The runtime ----

  async getRuntime(): Promise<VideoRuntimeView> {
    return structuredClone(this.runtime);
  }

  async installRuntime(): Promise<VideoRuntimeView> {
    if (!this.runtime.canDownload) throw fail('noPlatform', 'IMT has no FFmpeg build to download for this computer.', { hint: 'Install ffmpeg yourself (on macOS: brew install ffmpeg) and choose it here.' });
    const s = this.runtime.status;
    this.runtime = { ...this.runtime, status: { ...s, mode: s.customPath ? 'custom' : 'managed', active: RELEASE, previous: s.active, updateAvailable: false, problem: null } };
    return structuredClone(this.runtime);
  }

  async rollbackRuntime(): Promise<VideoRuntimeView> {
    const s = this.runtime.status;
    if (!s.previous) throw fail('notFound', 'There is no earlier FFmpeg build to go back to.');
    this.runtime = { ...this.runtime, status: { ...s, active: s.previous, previous: null, updateAvailable: s.pinnedTag !== s.previous.tag, problem: null } };
    return structuredClone(this.runtime);
  }

  async deleteRuntime(tag: string): Promise<VideoRuntimeView> {
    const s = this.runtime.status;
    if (s.previous?.tag === tag) this.runtime = { ...this.runtime, status: { ...s, previous: null } };
    else if (s.active?.tag === tag) this.runtime = { ...this.runtime, status: { ...s, active: null, mode: s.customPath ? 'custom' : 'none' } };
    else throw fail('notFound', `FFmpeg ${tag} is not installed.`);
    return structuredClone(this.runtime);
  }

  async setCustomRuntime(path: string): Promise<VideoRuntimeView> {
    if (!path.trim()) throw fail('invalid', 'Choose the ffmpeg program or the folder it is in.', { retryable: false });
    if (path.includes('broken')) throw fail('selfTestFailed', 'This ffmpeg did not pass the self-test: it could not analyse the test clip.', { hint: 'Choose another ffmpeg.' });
    this.runtime = { ...this.runtime, status: { ...this.runtime.status, mode: 'custom', customPath: path, customVersion: '7.1.1', problem: null } };
    return structuredClone(this.runtime);
  }

  async clearCustomRuntime(): Promise<VideoRuntimeView> {
    const s = this.runtime.status;
    this.runtime = { ...this.runtime, status: { ...s, customPath: null, customVersion: null, mode: s.active ? 'managed' : 'none' } };
    return structuredClone(this.runtime);
  }

  // ---- Videos ----

  async sniff(paths: string[]): Promise<string[]> {
    return paths.filter((p) => /\.(mp4|m4v|mov|mkv|webm|avi)$/i.test(p) && !p.includes('not-a-video'));
  }

  async list(): Promise<VideoList> {
    return { videos: [...this.videos.values()].map((r) => this.view(r)), skipped: [], projectFrames: this.projectFrames(), maxProjectFrames: MAX_PROJECT_FRAMES };
  }

  async open(request: OpenVideoRequest): Promise<VideoView> {
    const file = name(request.path);
    if (this.runtime.status.mode === 'none') throw fail('noRuntime', 'FFmpeg is not set up.', { hint: 'Set it up in Settings ▸ Video.' });
    if (request.path.includes('missing-file')) throw fail('notFound', `${file} was not found.`);
    if (!(await this.sniff([request.path])).length) throw fail('unsupported', `${file} is not a video IMT can read.`, { hint: 'IMT reads MP4, MOV, MKV, WebM and AVI.' });
    const key = mockKey(file);
    const known = this.videos.get(key);
    if (known) {
      known.path = request.path;
      if (request.include) known.included = true;
      return this.view(known);
    }
    if (this.projectFrames() + 5 > MAX_PROJECT_FRAMES) {
      throw fail('limit', `The project already keeps ${this.projectFrames()} frames; the most is ${MAX_PROJECT_FRAMES}.`, { hint: 'Remove a video first.' });
    }
    const moved = file.includes('moved');
    const r = record(file, request.path, request.options, request.include, moved ? [0, 5200, 12400, 21000, 33000] : [0, 4200, 9800, 15500, 23000, 31500], SESSION_START + (moved ? 5_400_000 : 0));
    if (!moved) r.selectionNotes = [{ kind: 'thresholdRaised', from: 0.1, to: 0.15 } satisfies SelectNote];
    this.videos.set(key, r);
    return this.view(r);
  }

  async remove(key: string): Promise<void> {
    if (!this.videos.delete(key)) throw fail('notFound', 'This video is not in the project any more.');
  }

  async include(request: IncludeRequest): Promise<VideoView> {
    const r = this.get(request.key);
    if (request.include && r.path?.startsWith(GONE) && !r.included) throw fail('notFound', `${r.name} is not where it was; locate it first.`);
    r.included = request.include;
    return this.view(r);
  }

  // ---- Frames ----

  async getFrameImage(frame: FrameRef): Promise<FrameImage> {
    const { r, f } = this.frameOf(frame);
    return { dataUrl: frameSvg(r.name, frame.index, f.videoMs) };
  }

  async addFrame(request: AddFrameRequest): Promise<VideoView> {
    const r = this.get(request.key);
    if (request.videoMs < 0 || request.videoMs > r.durationMs) throw fail('invalid', `The video is ${Math.round(r.durationMs / 1000)} s long.`, { retryable: false });
    if (r.frames.length >= MAX_VIDEO_FRAMES) throw fail('limit', `A video keeps at most ${MAX_VIDEO_FRAMES} frames.`, { hint: 'Remove a frame first.' });
    if (this.projectFrames() >= MAX_PROJECT_FRAMES) throw fail('limit', `The project keeps at most ${MAX_PROJECT_FRAMES} frames.`, { hint: 'Remove a frame or a video first.' });
    // The frame at or after the time asked for; the same frame twice is not added.
    const at = Math.min(r.durationMs - 40, request.videoMs + 33);
    if (!r.frames.some((f) => f.videoMs === at)) {
      r.frames.push({ videoMs: at, image: `frames/${String(r.frames.length + 1).padStart(4, '0')}.png`, width: r.width, height: r.height, score: 0, description: null, note: null, manual: true });
      r.frames.sort((a, b) => a.videoMs - b.videoMs);
    }
    return this.decided(r);
  }

  async removeFrame(frame: FrameRef): Promise<VideoView> {
    const { r } = this.frameOf(frame);
    if (r.frames.length <= 1) throw fail('invalid', 'A video keeps at least one frame; remove the video instead.');
    r.frames.splice(frame.index, 1);
    return this.decided(r);
  }

  async setNote(request: NoteRequest): Promise<VideoView> {
    const { r, f } = this.frameOf(request);
    const note = request.note?.trim() ? request.note : null;
    if (note && note.length > MAX_TEXT) throw fail('limit', `A note is at most ${MAX_TEXT} characters.`, { retryable: false });
    f.note = note;
    return this.view(r);
  }

  async generateDescriptions(request: FramesRequest): Promise<VideoView> {
    const r = this.get(request.key);
    if (!this.vision) throw fail('noVision', 'The active model reads text only.', { hint: 'Add its image support or choose a model that reads images in Settings ▸ AI.' });
    const wanted = request.indices.length ? request.indices : r.frames.map((_, i) => i);
    for (const i of wanted) {
      const f = r.frames[i];
      if (!f) throw fail('notFound', `The video has no frame ${i + 1}.`);
      if (request.redo || !f.description) f.description = DESCRIPTIONS[i % DESCRIPTIONS.length] ?? null;
    }
    return this.view(r);
  }

  async explainFrames(request: ExplainRangeRequest): Promise<FrameExplanation> {
    const r = this.get(request.key);
    if (!this.vision) throw fail('noVision', 'The active model reads text only.', { hint: 'Add its image support or choose a model that reads images in Settings ▸ AI.' });
    if (!request.indices.length || request.indices.length > 4) throw fail('invalid', 'Choose one to four frames.');
    return { text: `In ${r.name}, the shopper fills in the payment form and the payment then fails (${request.indices.length} frames read).` };
  }

  async frameToImage(frame: FrameRef): Promise<AiImage> {
    const { r, f } = this.frameOf(frame);
    return this.register(`${r.name} @ ${Math.floor(f.videoMs / 1000)} s`);
  }

  // ---- Alignment ----

  async setAlignment(request: AlignRequest): Promise<VideoView> {
    const r = this.get(request.key);
    const a = request.alignment;
    if (a.kind === 'start' && !Number.isFinite(a.startMs)) throw fail('invalid', 'The start is not a date and time.', { retryable: false });
    if (a.kind === 'offset' && Math.abs(a.offsetMs) > 7 * 86_400_000) throw fail('invalid', 'An offset of more than a week is not accepted.', { retryable: false });
    r.userAlignment = a;
    return this.decided(r);
  }

  async alignFromClock(frame: FrameRef): Promise<VideoView> {
    const { r, f } = this.frameOf(frame);
    if (!this.vision) throw fail('noVision', 'The active model reads text only.', { hint: 'Add its image support or choose a model that reads images in Settings ▸ AI.' });
    // The demo screens show a clock reading 10:42:07 at 12 s.
    const text = new Date(SESSION_START + 2 * 60_000 + 7_000 + (f.videoMs - 12_000)).toISOString().slice(11, 19);
    r.clock = { frameMs: f.videoMs, shown: [{ text, hasDate: false }] };
    return this.decided(r);
  }
}
