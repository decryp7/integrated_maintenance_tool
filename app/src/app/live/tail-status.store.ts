import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { TailFileChange, TailFileState, TailStatus } from '../core/api.types';
import { DebriefBackend } from '../core/backend';
import { describeError } from '../core/logger';

interface TailStatusState {
  status: TailStatus | null;
  loading: boolean;
  error: string | null;
}

/** File states worth a badge in the sidebar (tailing and whole files need none). */
export type NotableState = Extract<TailFileState['kind'], 'rotated' | 'truncated' | 'deleted' | 'error'>;
const NOTABLE: ReadonlySet<string> = new Set(['rotated', 'truncated', 'deleted', 'error']);

/**
 * The engine's per-file live tail state (`tail_status`), shared by the tail status panel and the sidebar. Live
 * reports carry the files whose state changed (`fileStates`), applied at once by `applyFileStates`; polling
 * `refresh` stays the fallback for everything else (offsets, rates, totals).
 */
export const TailStatusStore = signalStore(
  { providedIn: 'root' },
  withState<TailStatusState>({ status: null, loading: false, error: null }),
  withComputed((store) => ({
    /** Notable state per source id (rotated, truncated, deleted, error), for badges. */
    notable: computed(() => {
      const map = new Map<number, TailFileState>();
      for (const f of store.status()?.files ?? []) if (NOTABLE.has(f.state.kind)) map.set(f.sourceId, f.state);
      return map;
    }),
  })),
  withMethods((store, backend = inject(DebriefBackend)) => {
    let seq = 0;
    /** Fetches the status again; overlapping calls keep only the newest answer. Never rejects. */
    const refresh = async (): Promise<void> => {
      const mine = ++seq;
      patchState(store, { loading: true });
      try {
        const status = await backend.tailStatus();
        if (mine === seq) patchState(store, { status, loading: false, error: null });
      } catch (e) {
        if (mine === seq) patchState(store, { loading: false, error: describeError(e) });
      }
    };
    return {
      refresh,
      /**
       * Applies the per-file state changes of a live report to the rows already known, so badges update without
       * waiting for a poll. A file not listed yet (or no status loaded) fetches the whole status instead.
       */
      applyFileStates(changes: readonly TailFileChange[]): void {
        if (!changes.length) return;
        const status = store.status();
        const changed = new Map(changes.map((c) => [c.sourceId, c.state]));
        const known = new Set(status?.files.map((f) => f.sourceId) ?? []);
        if (status) {
          const files = status.files.map((f) => {
            const state = changed.get(f.sourceId);
            return state ? { ...f, state } : f;
          });
          const tailing = files.filter((f) => f.state.kind === 'tailing' || f.state.kind === 'rotated').length;
          const errors = files.filter((f) => f.state.kind === 'error').length;
          patchState(store, { status: { files, totals: { ...status.totals, tailing, errors } } });
        }
        if (changes.some((c) => !known.has(c.sourceId))) void refresh();
      },
    };
  }),
);

export type TailStatusStoreType = InstanceType<typeof TailStatusStore>;

/** Human label of a file state (never conveyed by colour alone). */
export function stateLabel(state: TailFileState): string {
  switch (state.kind) {
    case 'tailing':
      return $localize`:@@live.tail.state.tailing:Tailing`;
    case 'rotated':
      return $localize`:@@live.tail.state.rotated:Rotated`;
    case 'truncated':
      return $localize`:@@live.tail.state.truncated:Truncated`;
    case 'deleted':
      return $localize`:@@live.tail.state.deleted:Deleted`;
    case 'whole':
      return $localize`:@@live.tail.state.whole:Read whole`;
    case 'error':
      return $localize`:@@live.tail.state.error:Error`;
  }
}

/** Tooltip explaining a file state. */
export function stateHint(state: TailFileState): string {
  switch (state.kind) {
    case 'tailing':
      return $localize`:@@live.tail.hint.tailing:New lines are read as the file grows.`;
    case 'rotated':
      return $localize`:@@live.tail.hint.rotated:Renamed by log rotation; its events are kept and it is read until the writer lets go.`;
    case 'truncated':
      return $localize`:@@live.tail.hint.truncated:Truncated or rewritten in place; its events are kept and the file is read again from its start.`;
    case 'deleted':
      return $localize`:@@live.tail.hint.deleted:Gone from disk; its events are kept unless deleted files are dropped.`;
    case 'whole':
      return $localize`:@@live.tail.hint.whole:Binary format or archive: read again as a whole when it changes.`;
    case 'error':
      return $localize`:@@live.tail.hint.error:Cannot be read: ${state.message}:message:`;
  }
}
