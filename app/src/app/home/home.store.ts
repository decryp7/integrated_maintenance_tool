import { DestroyRef, effect, inject, Injectable, untracked } from '@angular/core';
import { AnalysisResource, AsyncResource } from '../analysis/async-resource';
import { Finding, HomeSummary, StoreMemory, StoreOverview } from '../core/api.types';
import { ConsumerGate } from '../core/panel-visibility';
import { EMPTY_QUERY, QueryStore } from '../core/workspace/query.store';
import { WorkspaceDataStore } from '../core/workspace/workspace-data.store';
import { ShareBackend } from '../share/share-backend';
import { StorePaneBackend } from '../store-pane/store-pane-backend';
import { HomeBackend } from './home-backend';
import { HOME_BUCKETS, HOME_REFRESH_MS } from './home-model';

/**
 * Runs `task` at most once per `ms`: the first request runs at once, requests during the pause become one trailing
 * run at its end. Pushed updates (an import's progress, live tail appends) can arrive many times a second.
 */
export class RefreshLimiter {
  private last = Number.NEGATIVE_INFINITY;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private readonly task: () => void,
    private readonly ms: number,
    private readonly now: () => number = () => Date.now(),
  ) {}

  request(): void {
    if (this.timer !== null) return;
    const wait = this.last + this.ms - this.now();
    if (wait <= 0) return this.fire();
    this.timer = setTimeout(() => {
      this.timer = null;
      this.fire();
    }, wait);
  }

  cancel(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }

  private fire(): void {
    this.last = this.now();
    this.task();
  }
}

/**
 * The Home dashboard's data: the engine's `home_summary` (counts, history, anomalies, exceptions), its health check
 * (`overview`) and the store's size and memory (the Store pane's reports). Each part loads and fails on its own (one
 * tile's error never blanks the others). Asked only while Home is mounted and visible, again when the data or the
 * grouping changed (imports, live tail appends, profile changes), at most once per `HOME_REFRESH_MS`.
 */
@Injectable({ providedIn: 'root' })
export class HomeStore {
  private readonly home = inject(HomeBackend);
  private readonly share = inject(ShareBackend);
  private readonly storePane = inject(StorePaneBackend);
  private readonly data = inject(WorkspaceDataStore);
  private readonly query = inject(QueryStore);
  readonly gate = new ConsumerGate();

  readonly summary = new AsyncResource<HomeSummary>();
  readonly health = new AnalysisResource<Finding[]>();
  readonly storeOverview = new AsyncResource<StoreOverview>();
  readonly memory = new AsyncResource<StoreMemory>();

  private readonly limiter = new RefreshLimiter(() => this.load(), HOME_REFRESH_MS);
  /** What the last load was for: the same data seen again (Home shown again) is not asked again. */
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

  /** Asks every part again now (the Refresh button). */
  refresh(): void {
    this.limiter.cancel();
    this.load();
  }

  private load(): void {
    if (!this.data.summary()?.sources.length) {
      for (const r of [this.summary, this.health, this.storeOverview, this.memory]) r.reset();
      return;
    }
    // Everything opened, not the current filters: only the grouping (the lanes' instances) applies.
    const query = { ...EMPTY_QUERY, groupBy: [...this.query.effectiveGroupBy()] };
    void this.summary.load(() => this.home.querySummary({ query, buckets: HOME_BUCKETS }));
    void this.health.load(() => this.share.overview(query));
    void this.storeOverview.load(() => this.storePane.overview());
    void this.memory.load(() => this.storePane.describeMemory());
  }
}
