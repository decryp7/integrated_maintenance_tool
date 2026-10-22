import { effect, inject, untracked } from '@angular/core';
import { patchState, signalStore, withHooks, withState } from '@ngrx/signals';
import { LEVELS, Level, QlQuery, Query } from '../api.types';
import { ImtBackend } from '../backend';
import { QueryStore } from './query.store';
import { WorkspaceDataStore } from './workspace-data.store';

/** Events per level that match every filter except the level chips themselves. */
export type LevelCounts = Record<Level, number>;

interface LevelCountsState {
  /** Null while nothing but the level chips narrows the result (the workspace totals apply) or before the engine answered. */
  matching: LevelCounts | null;
}

/** Whether anything other than the level chips narrows the result. */
export function narrowsBeyondLevels(q: Query): boolean {
  return !!(q.text.trim() || q.filters.length || q.groups.length || q.markers.length || q.from != null || q.to != null || (q as QlQuery).ql?.trim());
}

/**
 * The level chips' counts under the other filters: the engine's `@level` facet of the query without its levels, so a
 * chip says how many events it would add ("ERR 0 / 403" when the search matches no error). One request per query
 * change; a newer query supersedes an answer in flight; a failure leaves the totals in place.
 */
export const LevelCountsStore = signalStore(
  { providedIn: 'root' },
  withState<LevelCountsState>({ matching: null }),
  withHooks({
    onInit(store) {
      const backend = inject(ImtBackend);
      const query = inject(QueryStore);
      const data = inject(WorkspaceDataStore);
      let seq = 0;
      effect(() => {
        const q = { ...query.effectiveQuery(), levels: [] };
        data.dataVersion();
        const loaded = data.summary() !== null;
        const mine = ++seq;
        if (!loaded || !narrowsBeyondLevels(q)) {
          untracked(() => patchState(store, { matching: null }));
          return;
        }
        backend
          .queryFacet({ query: q, field: '@level', limit: LEVELS.length + 1 })
          .then((facet) => {
            if (mine !== seq) return;
            const matching = Object.fromEntries(LEVELS.map((l) => [l, 0])) as LevelCounts;
            for (const v of facet.values) if ((LEVELS as readonly string[]).includes(v.value)) matching[v.value as Level] = v.count;
            patchState(store, { matching });
          })
          .catch(() => {
            if (mine === seq) patchState(store, { matching: null });
          });
      });
    },
  }),
);
