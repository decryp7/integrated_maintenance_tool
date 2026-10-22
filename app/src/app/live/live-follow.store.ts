import { computed, effect, inject, untracked } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withState } from '@ngrx/signals';
import { Query } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { LiveTail } from '../workspace/live-tail';

/** Sliding windows offered for "follow now"; `all` follows without a window (everything up to the newest event). */
export type WindowPreset = '1m' | '5m' | '15m' | '60m' | 'custom' | 'all';

export const WINDOW_PRESETS: readonly { id: WindowPreset; label: string; ms: number | null }[] = [
  { id: '1m', label: $localize`:@@live.follow.preset.1m:Last 1 min`, ms: 60_000 },
  { id: '5m', label: $localize`:@@live.follow.preset.5m:Last 5 min`, ms: 5 * 60_000 },
  { id: '15m', label: $localize`:@@live.follow.preset.15m:Last 15 min`, ms: 15 * 60_000 },
  { id: '60m', label: $localize`:@@live.follow.preset.60m:Last 60 min`, ms: 60 * 60_000 },
  { id: 'custom', label: $localize`:@@live.follow.preset.custom:Custom…`, ms: null },
  { id: 'all', label: $localize`:@@live.follow.preset.all:All`, ms: null },
];

/** Bounds of the custom window, minutes. */
export const CUSTOM_MINUTES = { min: 1, max: 7 * 24 * 60 } as const;

export interface FollowState {
  /** The user's "Follow" toggle (on by default whenever live tail is on). */
  follow: boolean;
  /** Following was paused by a manual zoom or pan (or anything else that set a fixed range). */
  paused: boolean;
  preset: WindowPreset;
  /** Length of the custom window, minutes. */
  customMinutes: number;
  /** Bumped by Resume, so the grid scrolls back to the newest events. */
  resumeNonce: number;
}

const initialState: FollowState = { follow: true, paused: false, preset: 'all', customMinutes: 30, resumeNonce: 0 };

/** The window length of a preset in ms (null = no window). */
export function windowMsOf(preset: WindowPreset, customMinutes: number): number | null {
  if (preset === 'custom') return Math.round(clampMinutes(customMinutes) * 60_000);
  return WINDOW_PRESETS.find((p) => p.id === preset)?.ms ?? null;
}

export function clampMinutes(m: number): number {
  return Number.isFinite(m) ? Math.min(CUSTOM_MINUTES.max, Math.max(CUSTOM_MINUTES.min, m)) : initialState.customMinutes;
}

/** Validates saved follow settings (untrusted project/session state) on top of the defaults. */
export function sanitizeFollow(value: unknown): Pick<FollowState, 'follow' | 'preset' | 'customMinutes'> {
  const v = value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
  const preset = WINDOW_PRESETS.some((p) => p.id === v['preset']) ? (v['preset'] as WindowPreset) : initialState.preset;
  return {
    follow: typeof v['follow'] === 'boolean' ? v['follow'] : initialState.follow,
    preset,
    customMinutes: typeof v['customMinutes'] === 'number' ? clampMinutes(v['customMinutes']) : initialState.customMinutes,
  };
}

/** True when the query's window equals `ms` (null = no window). */
function sameWindow(q: Query, ms: number | null): boolean {
  const current = q.window?.kind === 'live' ? q.window.lastMs : null;
  return current === ms;
}

/**
 * "Follow now": while live tail is on and following, the query carries a sliding window
 * (`{ kind: 'live', lastMs }`) that the ENGINE resolves against the newest record, so the timeline, charts,
 * facets and grid all slide together. A fixed range set by anything else (zoom, pan, brush, an alert's "Show",
 * a note) pauses following until Resume.
 */
export const LiveFollow = signalStore(
  { providedIn: 'root' },
  withState<FollowState>(initialState),
  withComputed((store, live = inject(LiveTail)) => ({
    windowMs: computed(() => windowMsOf(store.preset(), store.customMinutes())),
    /** Following right now: live tail on, Follow on and not paused. */
    active: computed(() => live.enabled() && store.follow() && !store.paused()),
    /** Live tail is on but not following (Follow off or paused): new events are counted, not shown. */
    holding: computed(() => live.enabled() && (!store.follow() || store.paused())),
  })),
  withMethods((store) => ({
    setFollow(follow: boolean) {
      patchState(store, (s) => ({ follow, paused: false, resumeNonce: follow ? s.resumeNonce + 1 : s.resumeNonce }));
    },
    setPreset(preset: WindowPreset) {
      patchState(store, { preset, paused: false });
    },
    setCustomMinutes(minutes: number) {
      patchState(store, { customMinutes: clampMinutes(minutes), preset: 'custom', paused: false });
    },
    pause() {
      if (!store.paused()) patchState(store, { paused: true });
    },
    resume() {
      patchState(store, (s) => ({ paused: false, follow: true, resumeNonce: s.resumeNonce + 1 }));
    },
    restore(saved: unknown) {
      patchState(store, { ...sanitizeFollow(saved), paused: false });
    },
    reset() {
      patchState(store, { follow: initialState.follow, preset: initialState.preset, customMinutes: initialState.customMinutes, paused: false });
    },
  })),
  withHooks({
    onInit(store) {
      const ws = inject(WorkspaceStore);
      let wasActive = false;
      let applied: number | null | undefined;
      effect(() => {
        const active = store.active();
        const windowMs = store.windowMs();
        const q = ws.query();
        untracked(() => {
          if (!active) {
            wasActive = false;
            applied = undefined;
            if (q.window) ws.patchQuery({ window: undefined });
            return;
          }
          const fresh = !wasActive || applied !== windowMs;
          wasActive = true;
          if (fresh) {
            applied = windowMs;
            // (Re)start following: drop any fixed range and slide with the newest record.
            if (q.from != null || q.to != null || !sameWindow(q, windowMs)) {
              ws.patchQuery({ from: null, to: null, window: windowMs === null ? undefined : { kind: 'live', lastMs: windowMs } });
            }
            return;
          }
          // Something set a fixed range while following: that is a manual zoom or pan.
          if (q.from != null || q.to != null) store.pause();
          else if (!sameWindow(q, windowMs)) ws.patchQuery({ window: windowMs === null ? undefined : { kind: 'live', lastMs: windowMs } });
        });
      });
    },
  }),
);

export type LiveFollowStore = InstanceType<typeof LiveFollow>;
