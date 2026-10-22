import { computed, inject, Injectable } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { CmdError } from '../core/cmd-error';
import { SelectOptions, SkippedVideo, UserAlignment, VideoRuntimeView, VideoView } from '../core/generated/video';
import { browserMocks } from '../core/mock-registry';
import { nativePick } from '../core/native-dialog';
import { isTauri } from '../core/platform';
import { FrameAiUse, VideoAi } from './video-ai';
import { VideoBackend } from './video-backend';
import { hasVideoExtension, toFailure, VIDEO_EXTENSIONS, VideoFailure } from './video-format';

/** How a new video is analysed unless the user chooses otherwise (the engine's defaults, ADR 0033 section 3). */
export const DEFAULT_SELECT_OPTIONS: SelectOptions = { mode: { mode: 'changes', threshold: 0.1 }, cap: 30 };

/** The parts of the Video view that fail on their own, each with an inline error and Retry. */
export type VideoScope = 'list' | 'open' | 'frame' | 'align' | 'storage' | 'runtime';

/** What is running for the video shown (the engine runs these as jobs; progress and Cancel are in the Jobs pane). */
export type VideoBusy = 'describe' | 'clock' | 'frame' | 'note' | 'align' | 'include' | 'remove' | 'explain';

/** Which runtime action is running in Settings ▸ Video. */
export type RuntimeBusy = 'install' | 'rollback' | 'delete' | 'custom' | 'clear';

/** Chooses files: the native dialog in Tauri (which grants them to this window), demo paths in a browser. */
@Injectable({ providedIn: 'root' })
export class VideoPicker {
  /** The videos to analyse. */
  pickVideos(): Promise<string[]> {
    /* v8 ignore start: the native dialog exists only inside Tauri */
    if (isTauri()) {
      return nativePick({
        kind: 'openFiles',
        title: $localize`:@@video.pick.video:Open video`,
        filters: [{ name: $localize`:@@video.pick.videoFilter:Videos`, extensions: [...VIDEO_EXTENSIONS] }],
      });
    }
    /* v8 ignore stop */
    return Promise.resolve([...browserMocks().MOCK_VIDEO_PATHS]);
  }

  /** One video file to find a moved video again (the engine accepts only the same content). */
  async locateVideo(): Promise<string | null> {
    /* v8 ignore start: the native dialog exists only inside Tauri */
    if (isTauri()) {
      const picked = await nativePick({
        kind: 'openFile',
        title: $localize`:@@video.pick.locate:Locate the video`,
        filters: [{ name: $localize`:@@video.pick.videoFilter:Videos`, extensions: [...VIDEO_EXTENSIONS] }],
      });
      return picked[0] ?? null;
    }
    /* v8 ignore stop */
    return browserMocks().MOCK_LOCATED_PATH;
  }

  /** The ffmpeg program to use (any file: its name differs by platform). */
  async pickFfmpeg(): Promise<string | null> {
    /* v8 ignore start: the native dialog exists only inside Tauri */
    if (isTauri()) {
      const picked = await nativePick({ kind: 'openFile', title: $localize`:@@video.pick.ffmpeg:Choose ffmpeg` });
      return picked[0] ?? null;
    }
    /* v8 ignore stop */
    return browserMocks().MOCK_FFMPEG_PATH;
  }
}

interface VideoShape {
  videos: VideoView[];
  skipped: SkippedVideo[];
  /** Frames the project keeps over all videos, and the most it keeps. */
  projectFrames: number;
  maxProjectFrames: number;
  activeKey: string | null;
  selectedIndex: number | null;
  runtime: VideoRuntimeView | null;
  runtimeBusy: RuntimeBusy | null;
  loading: boolean;
  /** A video is being analysed. */
  opening: boolean;
  busy: VideoBusy | null;
  errors: Partial<Record<VideoScope, VideoFailure>>;
  notice: string | null;
}

const initial: VideoShape = {
  videos: [],
  skipped: [],
  projectFrames: 0,
  maxProjectFrames: 600,
  activeKey: null,
  selectedIndex: null,
  runtime: null,
  runtimeBusy: null,
  loading: false,
  opening: false,
  busy: null,
  errors: {},
  notice: null,
};

const isCancelled = (e: unknown) => e instanceof CmdError && e.code === 'cancelled';

/**
 * The videos of this project and what the Video view shows: the active video and frame, the FFmpeg runtime. The
 * engine analyses, aligns, describes and stores; the store calls it, keeps the answers and the inline errors with
 * their Retry.
 */
export const VideoStore = signalStore(
  { providedIn: 'root' },
  withState<VideoShape>(initial),
  withComputed((s) => {
    const active = computed(() => s.videos().find((v) => v.record.key === s.activeKey()) ?? s.videos()[0] ?? null);
    return {
      active,
      frame: computed(() => {
        const frames = active()?.record.frames ?? [];
        const i = s.selectedIndex();
        return i !== null && frames[i] ? { index: i, frame: frames[i] } : null;
      }),
      projectFull: computed(() => s.projectFrames() >= s.maxProjectFrames()),
      /** Neither a managed nor a user-provided ffmpeg is usable. */
      runtimeMissing: computed(() => {
        const r = s.runtime();
        return !!r && (r.status.mode === 'none' || !!r.status.problem);
      }),
    };
  }),
  withMethods((s) => {
    const backend = inject(VideoBackend);
    const picker = inject(VideoPicker);
    const ai = inject(VideoAi);
    const retries = new Map<VideoScope, () => Promise<unknown>>();
    const images = new Map<string, Promise<string>>();

    const failWith = (scope: VideoScope, f: VideoFailure) => patchState(s, (st) => ({ errors: { ...st.errors, [scope]: f } }));
    const fail = (scope: VideoScope, e: unknown) => failWith(scope, toFailure(e));
    const clear = (scope: VideoScope) =>
      patchState(s, (st) => {
        const { [scope]: _gone, ...rest } = st.errors;
        return { errors: rest };
      });

    /** Replaces a video by its new state (or adds it) and keeps the project's frame count. */
    const upsert = (v: VideoView) => {
      const videos = s.videos().some((x) => x.record.key === v.record.key) ? s.videos().map((x) => (x.record.key === v.record.key ? v : x)) : [...s.videos(), v];
      const frames = v.record.frames.length;
      const index = s.selectedIndex();
      patchState(s, {
        videos,
        projectFrames: videos.reduce((n, x) => n + x.record.frames.length, 0),
        selectedIndex: s.activeKey() === v.record.key && index !== null ? (frames ? Math.min(index, frames - 1) : null) : s.selectedIndex(),
      });
    };

    /**
     * Runs one engine operation on the video shown: `busy` while it runs, the answer applied, a failure shown in `scope`
     * with Retry. A cancelled job is not an error. Resolves with the answer, or null.
     */
    const run = async <T>(scope: VideoScope, busy: VideoBusy | null, op: () => Promise<T>, apply?: (r: T) => void): Promise<T | null> => {
      const attempt = async (): Promise<T | null> => {
        patchState(s, { busy });
        clear(scope);
        try {
          const r = await op();
          apply?.(r);
          return r;
        } catch (e) {
          if (!isCancelled(e)) fail(scope, e);
          return null;
        } finally {
          patchState(s, { busy: null });
        }
      };
      retries.set(scope, attempt);
      return attempt();
    };

    const withActive = <T>(f: (v: VideoView) => Promise<T | null>): Promise<T | null> => {
      const v = s.active();
      return v ? f(v) : Promise.resolve(null);
    };

    const refresh = async (): Promise<void> => {
      retries.set('list', refresh);
      patchState(s, { loading: true });
      clear('list');
      try {
        const list = await backend.list();
        const key = s.activeKey();
        patchState(s, {
          videos: list.videos,
          skipped: list.skipped,
          projectFrames: list.projectFrames,
          maxProjectFrames: list.maxProjectFrames,
          activeKey: list.videos.some((v) => v.record.key === key) ? key : (list.videos[0]?.record.key ?? null),
        });
      } catch (e) {
        fail('list', e);
      } finally {
        patchState(s, { loading: false });
      }
    };

    const loadRuntime = async (): Promise<void> => {
      retries.set('runtime', loadRuntime);
      clear('runtime');
      try {
        patchState(s, { runtime: await backend.getRuntime() });
      } catch (e) {
        fail('runtime', e);
      }
    };

    /** One runtime action: `runtimeBusy` while it runs; the engine's answer replaces the runtime, a failure shows with Retry. */
    const runtimeAction = async (kind: RuntimeBusy, op: () => Promise<VideoRuntimeView>): Promise<boolean> => {
      const attempt = async (): Promise<boolean> => {
        patchState(s, { runtimeBusy: kind });
        clear('runtime');
        try {
          patchState(s, { runtime: await op() });
          return true;
        } catch (e) {
          if (!isCancelled(e)) fail('runtime', e);
          return false;
        } finally {
          patchState(s, { runtimeBusy: null });
        }
      };
      retries.set('runtime', attempt);
      return attempt();
    };

    /** Analyses each path (failures are collected so the rest still opens). */
    const openPaths = async (paths: string[], include = false, options: SelectOptions = DEFAULT_SELECT_OPTIONS): Promise<VideoView[]> => {
      if (!paths.length) return [];
      const opened: VideoView[] = [];
      const attempt = async (todo: string[]): Promise<VideoView[]> => {
        patchState(s, { opening: true, notice: null });
        clear('open');
        const failed: string[] = [];
        const errors: unknown[] = [];
        for (const path of todo) {
          try {
            const v = await backend.open({ path, options, include });
            upsert(v);
            opened.push(v);
          } catch (e) {
            if (isCancelled(e)) continue;
            failed.push(path);
            errors.push(e);
          }
        }
        const last = opened.at(-1);
        if (last) patchState(s, { activeKey: last.record.key, selectedIndex: last.record.frames.length ? 0 : null });
        if (failed.length) {
          const first = toFailure(errors[0]);
          const more = failed.length - 1;
          failWith('open', more ? { ...first, message: $localize`:@@video.error.openMore:${first.message}:first: (and ${more}:count: more failed)` } : first);
          retries.set('open', () => attempt(failed));
        }
        patchState(s, { opening: false });
        return opened;
      };
      return attempt(paths);
    };

    return {
      refresh,
      loadRuntime,
      openPaths,
      async pickAndOpen(): Promise<VideoView[]> {
        try {
          return await openPaths(await picker.pickVideos());
        } catch (e) {
          fail('open', e);
          return [];
        }
      },
      /** Claims dropped files that are videos (by extension, then the engine's signature check) and analyses them. */
      async openDropped(paths: string[]): Promise<{ claimed: string[]; opened: VideoView[] }> {
        const candidates = paths.filter(hasVideoExtension);
        if (!candidates.length) return { claimed: [], opened: [] };
        const claimed = await backend.sniff(candidates);
        return { claimed, opened: await openPaths(claimed) };
      },
      /** A moved video is found again: the same video analysed from its new place (the engine checks the content). */
      async locate(key: string): Promise<VideoView | null> {
        const v = s.videos().find((x) => x.record.key === key);
        const path = v ? await picker.locateVideo() : null;
        if (!v || !path) return null;
        const opened = await openPaths([path], v.record.included, v.record.options);
        return opened[0] ?? null;
      },
      /** Runs the last failed action of `scope` again. */
      retry: (scope: VideoScope) => retries.get(scope)?.() ?? Promise.resolve(),
      dismiss: (scope: VideoScope) => clear(scope),
      dismissNotice: () => patchState(s, { notice: null }),

      setActive(key: string): void {
        const v = s.videos().find((x) => x.record.key === key);
        if (v) patchState(s, { activeKey: key, selectedIndex: v.record.frames.length ? 0 : null });
      },
      selectFrame(index: number | null): void {
        const n = s.active()?.record.frames.length ?? 0;
        patchState(s, { selectedIndex: index !== null && index >= 0 && index < n ? index : null });
      },

      /** The image of a frame as a data URL; asked once per frame image (thumbnails load lazily). */
      frameImage(key: string, index: number): Promise<string> {
        const frame = s.videos().find((v) => v.record.key === key)?.record.frames[index];
        const id = `${key}/${frame?.image ?? index}/${frame?.videoMs ?? ''}`;
        let cached = images.get(id);
        if (!cached) {
          cached = backend.getFrameImage({ key, index }).then((r) => r.dataUrl);
          cached.catch(() => images.delete(id));
          images.set(id, cached);
        }
        return cached;
      },

      saveNote: (index: number, note: string) =>
        withActive((v) => run('frame', 'note', () => backend.setNote({ key: v.record.key, index, note: note.trim() ? note : null }), upsert)),
      addFrame: (videoMs: number) =>
        withActive((v) =>
          run('frame', 'frame', () => backend.addFrame({ key: v.record.key, videoMs: Math.max(0, Math.round(videoMs)) }), (r) => {
            upsert(r);
            // The new frame is the one nearest the time asked for.
            const at = r.record.frames.findIndex((f) => !v.record.frames.some((o) => o.image === f.image && o.videoMs === f.videoMs));
            patchState(s, { selectedIndex: at >= 0 ? at : s.selectedIndex() });
          }),
        ),
      removeFrame: (index: number) => withActive((v) => run('frame', 'frame', () => backend.removeFrame({ key: v.record.key, index }), upsert)),
      /** The model describes `indices` (none: every frame without a description); `redo` describes again. */
      describe: (indices: number[], redo = false) =>
        withActive((v) => run('frame', 'describe', () => backend.generateDescriptions({ key: v.record.key, indices, redo }), upsert)),
      /** "Explain with AI" / "Find related logs" on a frame: the image view shows the answer. */
      useWithAi: (use: FrameAiUse, index: number) => withActive((v) => run('frame', 'explain', () => ai.run(use, { key: v.record.key, index }))),
      explainFrames: (indices: number[]) => withActive((v) => run('frame', 'explain', () => backend.explainFrames({ key: v.record.key, indices }))),

      setAlignment: (alignment: UserAlignment) => withActive((v) => run('align', 'align', () => backend.setAlignment({ key: v.record.key, alignment }), upsert)),
      readClock: (index: number) => withActive((v) => run('align', 'clock', () => backend.alignFromClock({ key: v.record.key, index }), upsert)),

      include: (include: boolean) => withActive((v) => run('storage', 'include', () => backend.include({ key: v.record.key, include }), upsert)),
      /** Forgets the video here and in the engine (the caller asked the user first). */
      async remove(key: string): Promise<boolean> {
        const done = await run('storage', 'remove', () => backend.remove(key).then(() => true));
        if (!done) return false;
        for (const id of [...images.keys()]) if (id.startsWith(`${key}/`)) images.delete(id);
        const videos = s.videos().filter((v) => v.record.key !== key);
        patchState(s, {
          videos,
          projectFrames: videos.reduce((n, x) => n + x.record.frames.length, 0),
          activeKey: s.activeKey() === key ? (videos[0]?.record.key ?? null) : s.activeKey(),
          selectedIndex: s.activeKey() === key ? (videos[0]?.record.frames.length ? 0 : null) : s.selectedIndex(),
        });
        return true;
      },

      // ---- The FFmpeg runtime (Settings ▸ Video) ----
      installRuntime: () => runtimeAction('install', () => backend.installRuntime()),
      rollbackRuntime: () => runtimeAction('rollback', () => backend.rollbackRuntime()),
      deleteRuntime: (tag: string) => runtimeAction('delete', () => backend.deleteRuntime(tag)),
      useCustomRuntime: (path: string) => runtimeAction('custom', () => backend.setCustomRuntime(path)),
      clearCustomRuntime: () => runtimeAction('clear', () => backend.clearCustomRuntime()),
      /** "Choose ffmpeg…": the user picks the program, the engine tests it. */
      async chooseRuntime(): Promise<boolean> {
        const path = await picker.pickFfmpeg();
        return path ? runtimeAction('custom', () => backend.setCustomRuntime(path)) : false;
      },
    };
  }),
);

export type VideoStoreType = InstanceType<typeof VideoStore>;
