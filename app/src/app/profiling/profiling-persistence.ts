import { inject, Injectable, signal } from '@angular/core';
import { MetricId, RecursionMode } from '../core/api.types';
import { UiStateContribution } from '../lifecycle/ui-state';
import { CapturesStore } from './captures.store';
import { DEFAULT_FRAMEWORK_PATTERNS, ProfileViewState } from './profile-view-state';

/** Version of the saved profiling state; older or newer versions are read field by field. */
export const PROFILING_STATE_VERSION = 1;

/** Key of the profiling state in the project's (and the autosaved session's) `ui` object. */
export const PROFILING_STATE_KEY = 'profiling';

/**
 * What a project or session remembers of profiling: the open captures (by path, so they are
 * imported again on reopen) and the shared view settings. Selections that only make sense for one
 * import (ids, zoom, ranges) are not saved.
 */
export interface ProfilingUiState {
  version: number;
  /** Paths of the open captures, in the order they were opened. */
  captures: string[];
  /** The active profile, heap snapshot and baseline, by path. */
  profile: string | null;
  heap: string | null;
  baseline: string | null;
  justMyCode: boolean;
  frameworkPatterns: string[] | null;
  useFrameFlags: boolean;
  recursion: RecursionMode;
  metric: MetricId | null;
}

const RECURSION: readonly RecursionMode[] = ['off', 'direct', 'all'];
const METRICS: readonly MetricId[] = ['samples', 'timeNs', 'calls', 'allocCount', 'allocBytes', 'exceptions', 'waitNs', 'maxCallNs', 'liveBytes'];
/** Bound on restored captures (the engine keeps at most 32 open). */
const MAX_CAPTURES = 32;

const str = (v: unknown): string | null => (typeof v === 'string' && v.length > 0 ? v : null);

/**
 * Parses saved state (anything: missing, corrupt, from an older or newer IMT) into a valid
 * `ProfilingUiState`; each field that is missing or malformed takes its default. Null when the value
 * is not an object at all.
 */
export function parseProfilingUiState(value: unknown): ProfilingUiState | null {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return null;
  const v = value as Record<string, unknown>;
  const captures = Array.isArray(v['captures']) ? [...new Set(v['captures'].map(str).filter((p): p is string => p !== null))].slice(0, MAX_CAPTURES) : [];
  const patterns = Array.isArray(v['frameworkPatterns']) ? v['frameworkPatterns'].filter((p): p is string => typeof p === 'string') : null;
  return {
    version: typeof v['version'] === 'number' ? v['version'] : PROFILING_STATE_VERSION,
    captures,
    profile: str(v['profile']),
    heap: str(v['heap']),
    baseline: str(v['baseline']),
    justMyCode: v['justMyCode'] === true,
    frameworkPatterns: patterns,
    useFrameFlags: v['useFrameFlags'] !== false,
    recursion: RECURSION.includes(v['recursion'] as RecursionMode) ? (v['recursion'] as RecursionMode) : 'off',
    metric: METRICS.includes(v['metric'] as MetricId) ? (v['metric'] as MetricId) : null,
  };
}

/**
 * Saves and restores the profiling state with the project and the session (a `ui` state
 * contribution). Restoring re-imports the saved captures through the engine; files that no longer
 * exist are reported in the Profiles view, not as a failure.
 */
@Injectable({ providedIn: 'root' })
export class ProfilingPersistence {
  private readonly captures = inject(CapturesStore);
  private readonly state = inject(ProfileViewState);
  /** The last restore, so tests (and callers) can wait for it. */
  pending: Promise<void> = Promise.resolve();
  /**
   * The state being restored, reported by `capture` until the restore finished: the project marks
   * itself clean right after restoring, so the captures reopening in the background must not make
   * it look changed. (It does look changed afterwards when a capture could not be reopened.)
   */
  private readonly restoring = signal<ProfilingUiState | null>(null);

  /** The state to save. Reads signals, so the saved state follows every change. */
  capture(): ProfilingUiState {
    const target = this.restoring();
    if (target) return target;
    const all = this.captures.captures();
    const pathOf = (id: number | null) => (id === null ? null : (all.find((c) => c.id === id)?.path ?? null));
    return {
      version: PROFILING_STATE_VERSION,
      captures: all.map((c) => c.path).filter((p): p is string => !!p),
      profile: pathOf(this.state.profileId()),
      heap: pathOf(this.state.heapId()),
      baseline: pathOf(this.state.baselineId()),
      justMyCode: this.state.justMyCode(),
      frameworkPatterns: this.state.frameworkPatterns(),
      useFrameFlags: this.state.useFrameFlags(),
      recursion: this.state.recursion(),
      metric: this.state.metric(),
    };
  }

  /** Applies saved state: null (new project, no saved profiling state) closes every capture and resets the views. */
  restore(saved: ProfilingUiState | null): Promise<void> {
    this.restoring.set(this.expected(saved));
    this.pending = this.apply(saved).finally(() => this.restoring.set(null));
    return this.pending;
  }

  /** What `capture` returns once `saved` has been applied (all files found). */
  private expected(saved: ProfilingUiState | null): ProfilingUiState {
    if (!saved) {
      return { version: PROFILING_STATE_VERSION, captures: [], profile: null, heap: null, baseline: null, justMyCode: false, frameworkPatterns: [...DEFAULT_FRAMEWORK_PATTERNS], useFrameFlags: true, recursion: 'off', metric: null };
    }
    return { ...saved, version: PROFILING_STATE_VERSION, frameworkPatterns: saved.frameworkPatterns ?? this.state.frameworkPatterns() };
  }

  private async apply(saved: ProfilingUiState | null): Promise<void> {
    if (!saved) {
      if (this.captures.captures().length) await this.captures.closeAll();
      this.state.reset();
      return;
    }
    // Keep captures that are already open from the same file; close the others; open the rest.
    const wanted = new Set(saved.captures);
    for (const c of this.captures.captures()) if (!c.path || !wanted.has(c.path)) await this.captures.close(c.id);
    const open = new Set(this.captures.captures().map((c) => c.path));
    await this.captures.restorePaths(saved.captures.filter((p) => !open.has(p)));
    const idOf = (path: string | null) => (path === null ? null : (this.captures.captures().find((c) => c.path === path)?.id ?? null));
    const profile = idOf(saved.profile);
    if (profile !== null) this.state.setProfile(profile);
    const heap = idOf(saved.heap);
    if (heap !== null) this.state.setHeap(heap);
    this.state.setBaseline(idOf(saved.baseline));
    this.state.setJustMyCode(saved.justMyCode);
    if (saved.frameworkPatterns) this.state.setFrameworkPatterns(saved.frameworkPatterns);
    this.state.setUseFrameFlags(saved.useFrameFlags);
    this.state.setRecursion(saved.recursion);
    this.state.setMetric(saved.metric);
  }

  /** The `ui` state contribution (registered by `provideProfiling`). */
  readonly contribution: UiStateContribution = {
    key: PROFILING_STATE_KEY,
    capture: () => this.capture(),
    restore: (value) => void this.restore(parseProfilingUiState(value)),
    reset: () => void this.restore(null),
  };
}
