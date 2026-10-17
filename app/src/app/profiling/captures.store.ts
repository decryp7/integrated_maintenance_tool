import { computed, inject, Injectable, signal } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { CaptureSummary, FrameDto } from '../core/api.types';
import { isTauri } from '../core/platform';
import { MOCK_CAPTURE_PATHS } from '../core/mock/mock-profiling-data';
import { errorText } from '../analysis/async-resource';
import { ProfilingBackend } from './profiling-backend';
import { profilingErrorCode } from './profiling-errors';
import { isHeap, isProfile } from './profiling-format';
import { ProfileViewState } from './profile-view-state';

/* v8 ignore start: the native dialog exists only inside Tauri */
async function nativePick(): Promise<string[]> {
  const { open } = await import('@tauri-apps/plugin-dialog');
  const result = await open({ multiple: true, directory: false, title: 'Open profile or heap snapshot' });
  if (result === null) return [];
  return Array.isArray(result) ? result : [result];
}
/* v8 ignore stop */

/** Chooses capture files: the native dialog in Tauri, the demo captures in a browser. Replaceable in tests. */
@Injectable({ providedIn: 'root' })
export class CapturePicker {
  pick(): Promise<string[]> {
    /* v8 ignore next */
    if (isTauri()) return nativePick();
    return Promise.resolve([...MOCK_CAPTURE_PATHS]);
  }
}

/** Names of frames seen in any answer, per profile, so breadcrumbs and paths can be labelled. */
@Injectable({ providedIn: 'root' })
export class FrameNames {
  private readonly frames = new Map<string, FrameDto>();
  /** Bumped when new names arrive. */
  readonly version = signal(0);

  remember(profileId: number, frames: readonly (FrameDto | null)[]): void {
    let added = false;
    for (const f of frames) {
      if (!f) continue;
      const key = `${profileId}:${f.id}`;
      if (!this.frames.has(key)) {
        this.frames.set(key, f);
        added = true;
      }
    }
    if (added) this.version.update((v) => v + 1);
  }

  get(profileId: number, id: number): FrameDto | null {
    return this.frames.get(`${profileId}:${id}`) ?? null;
  }

  name(profileId: number, id: number): string {
    return this.get(profileId, id)?.name ?? `frame #${id}`;
  }
}

interface CapturesShape {
  captures: CaptureSummary[];
  loading: boolean;
  /** Last failure of open/close/list, shown inline with Retry. */
  error: string | null;
  /** Paths whose open failed, for Retry. */
  failedPaths: string[];
  notice: string | null;
}

/**
 * The captures open in this window (the engine keeps them per window). Opening, closing and listing
 * go through the profiling backend; the active profile and heap live in `ProfileViewState`.
 */
export const CapturesStore = signalStore(
  { providedIn: 'root' },
  withState<CapturesShape>({ captures: [], loading: false, error: null, failedPaths: [], notice: null }),
  withComputed((s) => ({
    profiles: computed(() => s.captures().filter(isProfile)),
    heaps: computed(() => s.captures().filter(isHeap)),
  })),
  withMethods((s) => {
    const backend = inject(ProfilingBackend);
    const state = inject(ProfileViewState);
    const picker = inject(CapturePicker);

    /** Keeps the active profile/heap valid after the list changed. */
    const settleActive = (preferProfile: number | null, preferHeap: number | null) => {
      const profiles = s.captures().filter(isProfile);
      const heaps = s.captures().filter(isHeap);
      const pid = state.profileId();
      if (preferProfile !== null) state.setProfile(preferProfile);
      else if (pid === null || !profiles.some((p) => p.id === pid)) state.setProfile(profiles[0]?.id ?? null);
      const hid = state.heapId();
      if (preferHeap !== null) state.setHeap(preferHeap);
      else if (hid === null || !heaps.some((h) => h.id === hid)) state.setHeap(heaps[0]?.id ?? null);
      const base = state.baselineId();
      if (base !== null && !profiles.some((p) => p.id === base)) state.setBaseline(null);
    };

    const refresh = async (): Promise<void> => {
      patchState(s, { loading: true });
      try {
        patchState(s, { captures: await backend.list(), error: null, failedPaths: [] });
        settleActive(null, null);
      } catch (e) {
        patchState(s, { error: errorText(e) });
      } finally {
        patchState(s, { loading: false });
      }
    };

    /**
     * Opens each path; failures are collected so the rest still opens. With `missingIsNotice`, files
     * that no longer exist (code `notFound`) are reported in the notice instead of as an error.
     * Resolves to the opened captures.
     */
    const openPaths = async (paths: string[], missingIsNotice = false): Promise<CaptureSummary[]> => {
      if (!paths.length) return [];
      patchState(s, { loading: true, error: null, notice: null });
      const opened: CaptureSummary[] = [];
      const errors: string[] = [];
      const failed: string[] = [];
      const missing: string[] = [];
      for (const path of paths) {
        try {
          opened.push(await backend.open(path));
        } catch (e) {
          if (missingIsNotice && profilingErrorCode(e) === 'notFound') {
            missing.push(path);
            continue;
          }
          errors.push(errorText(e));
          failed.push(path);
        }
      }
      return finishOpen(opened, errors, failed, missing);
    };

    const finishOpen = async (opened: CaptureSummary[], errors: string[], failed: string[], missing: string[] = []): Promise<CaptureSummary[]> => {
      try {
        patchState(s, { captures: await backend.list() });
      } catch (e) {
        errors.push(errorText(e));
      }
      settleActive(opened.find(isProfile)?.id ?? null, opened.find(isHeap)?.id ?? null);
      patchState(s, {
        loading: false,
        error: errors.length ? errors.join('\n') : null,
        failedPaths: failed,
        notice: openNotice(opened.length, missing),
      });
      if (opened.length) state.capturesChanged();
      return opened;
    };

    return {
      refresh,
      openPaths,
      async pickAndOpen(): Promise<CaptureSummary[]> {
        try {
          return await openPaths(await picker.pick());
        } catch (e) {
          patchState(s, { error: errorText(e) });
          return [];
        }
      },
      /**
       * Claims the dropped files the engine sniffs as profiles or heap snapshots and opens them
       * (their open errors are shown here, with Retry); the rest are left to the log import.
       * Resolves to the claimed paths and what opened. Rejects when sniffing fails (nothing claimed).
       */
      async openDropped(paths: string[]): Promise<{ claimed: string[]; opened: CaptureSummary[] }> {
        if (!paths.length) return { claimed: [], opened: [] };
        const claimed = (await backend.sniff(paths)).filter((r) => r.kind !== 'none').map((r) => r.path);
        return { claimed, opened: await openPaths(claimed) };
      },
      /**
       * Opens the captures of a restored project or session; files that no longer exist are listed
       * in the notice (not an error). Closes nothing: `replaceWith` decides what stays open.
       */
      restorePaths: (paths: string[]) => openPaths(paths, true),
      /** Closes every open capture (a new or restored project starts from its own captures). */
      async closeAll(): Promise<void> {
        for (const c of s.captures()) {
          try {
            await backend.close(c.id);
          } catch {
            // Already closed: nothing to do.
          }
        }
        patchState(s, { captures: [], error: null, failedPaths: [], notice: null });
        settleActive(null, null);
        state.capturesChanged();
      },
      retry(): Promise<unknown> {
        const failed = s.failedPaths();
        return failed.length ? openPaths(failed) : refresh();
      },
      async close(id: number): Promise<void> {
        try {
          await backend.close(id);
          patchState(s, { captures: s.captures().filter((c) => c.id !== id), error: null });
          settleActive(null, null);
          state.capturesChanged();
        } catch (e) {
          patchState(s, { error: errorText(e) });
        }
      },
      dismiss: () => patchState(s, { error: null, notice: null, failedPaths: [] }),
      byId: (id: number | null) => (id === null ? null : (s.captures().find((c) => c.id === id) ?? null)),
    };
  }),
);

export type CapturesStoreType = InstanceType<typeof CapturesStore>;

/** "Opened 2 captures." plus the files that could not be found; null when there is nothing to say. */
export function openNotice(opened: number, missing: readonly string[]): string | null {
  const parts: string[] = [];
  if (opened) parts.push(`Opened ${opened} capture${opened === 1 ? '' : 's'}.`);
  if (missing.length) {
    const names = missing.map((p) => p.split(/[\\/]/).pop() ?? p).join(', ');
    parts.push(`${missing.length} capture${missing.length === 1 ? ' was' : 's were'} not found: ${names}.`);
  }
  return parts.length ? parts.join(' ') : null;
}
