import { computed, DestroyRef, effect, inject, Injectable, untracked } from '@angular/core';
import { HomeSummary } from '../core/api.types';
import { AsyncResource } from '../analysis/async-resource';
import { ConsumerGate } from '../core/panel-visibility';
import { EMPTY_QUERY, QueryStore } from '../core/workspace/query.store';
import { WorkspaceDataStore } from '../core/workspace/workspace-data.store';
import { HomeBackend } from '../home/home-backend';
import { RefreshLimiter } from '../home/home.store';

/** Buckets of the minimap's whole-extent history (a strip is narrow: a coarse picture is enough). */
export const MINIMAP_BUCKETS = 120;
/** Least time between two refreshes (live tail appends arrive many times a second). */
export const MINIMAP_REFRESH_MS = 2000;

/**
 * The whole data extent's events by level for the timeline minimap: the engine's Home summary without the slow
 * analyses, over everything opened (not the current filters or zoom, which is the point of a minimap). Asked only
 * while a timeline is mounted and visible, again when the data changes.
 */
@Injectable({ providedIn: 'root' })
export class TimelineMinimapStore {
  private readonly backend = inject(HomeBackend);
  private readonly data = inject(WorkspaceDataStore);
  private readonly query = inject(QueryStore);
  readonly gate = new ConsumerGate();

  private readonly resource = new AsyncResource<HomeSummary>();
  /** Events per bucket and level over the whole extent; empty before it arrives or without timestamps. */
  readonly histogram = computed(() => this.resource.data()?.histogram ?? []);
  private readonly limiter = new RefreshLimiter(() => this.load(), MINIMAP_REFRESH_MS);
  private lastKey = '';

  constructor() {
    inject(DestroyRef).onDestroy(() => this.limiter.cancel());
    effect(() => {
      if (!this.gate.active()) return;
      const sources = this.data.summary()?.sources.length ?? 0;
      const key = JSON.stringify([this.data.dataVersion(), this.data.profileVersion(), this.query.effectiveGroupBy(), sources]);
      untracked(() => {
        if (key === this.lastKey) return;
        this.lastKey = key;
        this.limiter.request();
      });
    });
  }

  private load(): void {
    if (!this.data.summary()?.sources.length) {
      this.resource.reset();
      return;
    }
    const query = { ...EMPTY_QUERY, groupBy: [...this.query.effectiveGroupBy()] };
    // A failure only leaves the strip empty: the minimap is a convenience, the timeline itself reports problems.
    void this.resource.load(() => this.backend.querySummary({ query, buckets: MINIMAP_BUCKETS, analyses: false }));
  }
}
