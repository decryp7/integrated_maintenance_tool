import { computed, effect, inject, untracked } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withState } from '@ngrx/signals';
import { Timeline } from '../api.types';
import { DebriefBackend } from '../backend';
import { QueryStore } from './query.store';
import { UiPrefsStore } from './ui-prefs.store';
import { errorText, WorkspaceDataStore } from './workspace-data.store';

interface TimelineState {
  timeline: Timeline | null;
  loading: boolean;
  /** Why the last timeline request failed (shown inline in the timeline panel with Retry). */
  error: string | null;
  /** Bumped to fetch the timeline again after a failure. */
  retryNonce: number;
  buckets: number;
}

/** The workspace timeline for the current query, fetched again whenever the query, the data or the bucket count changes. */
export const TimelineStore = signalStore(
  { providedIn: 'root' },
  withState<TimelineState>({ timeline: null, loading: false, error: null, retryNonce: 0, buckets: 400 }),
  withComputed((store) => ({
    instanceCount: computed(() => store.timeline()?.lanes.length ?? 0),
  })),
  withMethods((store) => ({
    retry: () => patchState(store, (s) => ({ retryNonce: s.retryNonce + 1 })),
    setBuckets(buckets: number) {
      if (Math.abs(buckets - store.buckets()) > 4) patchState(store, { buckets });
    },
  })),
  withHooks({
    onInit(store) {
      const backend = inject(DebriefBackend);
      const data = inject(WorkspaceDataStore);
      const query = inject(QueryStore);
      const prefs = inject(UiPrefsStore);
      let seq = 0;
      // effect-state: IPC timeline request keyed on query and data version; the answer lands in the store
      effect(() => {
        const summary = data.summary();
        const request = { query: query.effectiveQuery(), buckets: store.buckets(), alwaysShowLifecycle: prefs.alwaysShowLifecycle() };
        data.dataVersion();
        store.retryNonce();
        if (!summary) return;
        const mine = ++seq;
        untracked(() => patchState(store, { loading: true }));
        backend
          .queryTimeline(request)
          .then((timeline) => mine === seq && patchState(store, { timeline, loading: false, error: null }))
          .catch((e) => mine === seq && patchState(store, { loading: false, error: errorText(e) }));
      });
    },
  }),
);
