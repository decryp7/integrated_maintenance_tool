import { computed } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { Evidence, FrameDto, MetricId, ProfileEventKind, ProfileTimeRange, ProfileView, RecursionMode, TreeDirection } from '../core/api.types';

/** Default "just my code" framework patterns offered in the editor (matched against module and function). */
export const DEFAULT_FRAMEWORK_PATTERNS = ['System.*', 'Microsoft.*'];

/** Engine limits on user patterns (see `JustMyCode`). */
export const MAX_PATTERNS = 64;
export const MAX_PATTERN_LEN = 256;

/** A request to expand a top-down path in the call tree (nonce: repeat requests re-apply). */
export interface Reveal {
  path: number[];
  direction: TreeDirection;
  nonce: number;
}

interface ProfileViewStateShape {
  /** Active profile (every profile view queries it). */
  profileId: number | null;
  /** Active heap snapshot (memory views). */
  heapId: number | null;
  /** Baseline profile for the differential flame graph, function diff and regression findings. */
  baselineId: number | null;
  range: ProfileTimeRange | null;
  threads: number[] | null;
  justMyCode: boolean;
  frameworkPatterns: string[];
  useFrameFlags: boolean;
  recursion: RecursionMode;
  /** Metric of the views; null = the profile's default. */
  metric: MetricId | null;
  /** Function selected for the butterfly and highlighted elsewhere. */
  selectedFrame: FrameDto | null;
  /** Zoomed node of the flame graph (top-down path; empty = everything). */
  flameFocus: number[];
  /** Search highlighted in the flame graph (e.g. "show in flame graph" from a hotspot). */
  flameSearch: string;
  reveal: Reveal | null;
  /** Event kind to emphasise on the thread timeline (from a finding). */
  eventKind: ProfileEventKind | null;
  /** Bumped whenever captures open or close, so dependants (timeline spans) refresh. */
  capturesVersion: number;
}

const INITIAL: ProfileViewStateShape = {
  profileId: null,
  heapId: null,
  baselineId: null,
  range: null,
  threads: null,
  justMyCode: false,
  frameworkPatterns: [...DEFAULT_FRAMEWORK_PATTERNS],
  useFrameFlags: true,
  recursion: 'off',
  metric: null,
  selectedFrame: null,
  flameFocus: [],
  flameSearch: '',
  reveal: null,
  eventKind: null,
  capturesVersion: 0,
};

/** Selection that belongs to one profile and is dropped when another becomes active. */
const PER_PROFILE: Partial<ProfileViewStateShape> = { range: null, threads: null, selectedFrame: null, flameFocus: [], flameSearch: '', reveal: null, eventKind: null, metric: null };

/** Cleans user patterns the way the engine accepts them: trimmed, non-empty, unique, within the limits. */
export function cleanPatterns(patterns: readonly string[]): string[] {
  const out: string[] = [];
  for (const p of patterns.map((x) => x.trim())) {
    if (p && p.length <= MAX_PATTERN_LEN && !out.includes(p)) out.push(p);
    if (out.length === MAX_PATTERNS) break;
  }
  return out;
}

/**
 * The shared profile view: which capture, time range, threads, just-my-code, recursion folding and
 * selected frame. Every profiling view queries the engine with `view()`, so brushing a range on the
 * thread timeline re-aggregates all of them. Pure UI state: no backend.
 */
export const ProfileViewState = signalStore(
  { providedIn: 'root' },
  withState(INITIAL),
  withComputed((s) => ({
    view: computed<ProfileView>(() => ({
      range: s.range(),
      threads: s.threads(),
      justMyCode: s.justMyCode() ? { enabled: true, frameworkPatterns: s.frameworkPatterns(), useFrameFlags: s.useFrameFlags() } : null,
      recursion: s.recursion(),
    })),
    /** True when anything narrows or transforms the full profile. */
    filtered: computed(() => !!s.range() || !!s.threads()?.length || s.justMyCode() || s.recursion() !== 'off'),
  })),
  withMethods((s) => ({
    setProfile(profileId: number | null): void {
      if (profileId === s.profileId()) return;
      patchState(s, { ...PER_PROFILE, profileId, baselineId: s.baselineId() === profileId ? null : s.baselineId() });
    },
    setHeap: (heapId: number | null) => patchState(s, { heapId }),
    setBaseline: (baselineId: number | null) => patchState(s, { baselineId }),
    /** A range with from > to is swapped; null clears it. */
    setRange(range: ProfileTimeRange | null): void {
      if (range && range.fromNs > range.toNs) range = { fromNs: range.toNs, toNs: range.fromNs };
      patchState(s, { range: range ? { fromNs: Math.max(0, Math.floor(range.fromNs)), toNs: Math.ceil(range.toNs) } : null });
    },
    setThreads: (threads: number[] | null) => patchState(s, { threads: threads?.length ? [...threads].sort((a, b) => a - b) : null }),
    toggleThread(thread: number): void {
      const cur = s.threads() ?? [];
      const next = cur.includes(thread) ? cur.filter((t) => t !== thread) : [...cur, thread];
      patchState(s, { threads: next.length ? next.sort((a, b) => a - b) : null });
    },
    setJustMyCode: (justMyCode: boolean) => patchState(s, { justMyCode }),
    setFrameworkPatterns: (patterns: readonly string[]) => patchState(s, { frameworkPatterns: cleanPatterns(patterns) }),
    setUseFrameFlags: (useFrameFlags: boolean) => patchState(s, { useFrameFlags }),
    setRecursion: (recursion: RecursionMode) => patchState(s, { recursion }),
    setMetric: (metric: MetricId | null) => patchState(s, { metric }),
    selectFrame: (selectedFrame: FrameDto | null) => patchState(s, { selectedFrame }),
    setFlameFocus: (flameFocus: number[]) => patchState(s, { flameFocus: [...flameFocus] }),
    setFlameSearch: (flameSearch: string) => patchState(s, { flameSearch }),
    revealPath(path: number[], direction: TreeDirection = 'topDown'): void {
      patchState(s, { reveal: { path: [...path], direction, nonce: (s.reveal()?.nonce ?? 0) + 1 } });
    },
    setEventKind: (eventKind: ProfileEventKind | null) => patchState(s, { eventKind }),
    capturesChanged: () => patchState(s, { capturesVersion: s.capturesVersion() + 1 }),
    /** Makes a finding's evidence the current selection (the navigator then shows its view). */
    applyEvidence(e: Evidence): void {
      patchState(s, {
        ...PER_PROFILE,
        profileId: e.profileId,
        baselineId: e.baselineId ?? s.baselineId(),
        range: e.range,
        threads: e.threads?.length ? e.threads : null,
        metric: e.metric,
        selectedFrame: e.frame,
        flameFocus: e.view === 'flame' ? [...e.path] : [],
        reveal: e.path.length ? { path: [...e.path], direction: 'topDown', nonce: (s.reveal()?.nonce ?? 0) + 1 } : null,
        eventKind: e.eventKind,
      });
    },
    reset: () => patchState(s, INITIAL),
  })),
);

export type ProfileViewStateStore = InstanceType<typeof ProfileViewState>;
