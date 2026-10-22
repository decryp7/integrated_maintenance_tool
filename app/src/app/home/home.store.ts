import { computed, DestroyRef, effect, inject, Injectable, signal, untracked } from '@angular/core';
import { ActivityLog } from '../core/activity/activity-log';
import type { Breadcrumb } from '../core/activity/types';
import { AsyncResource } from '../analysis/async-resource';
import { HomeSummary, Query, StoreMemory, StoreOverview } from '../core/api.types';
import { ConsumerGate } from '../core/panel-visibility';
import { EMPTY_QUERY, QueryStore } from '../core/workspace/query.store';
import { WorkspaceDataStore } from '../core/workspace/workspace-data.store';
import { HealthFindingsStore } from './health/health-findings.store';
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
  private readonly findings = inject(HealthFindingsStore);
  private readonly storePane = inject(StorePaneBackend);
  private readonly data = inject(WorkspaceDataStore);
  private readonly query = inject(QueryStore);
  private readonly activity = inject(ActivityLog);
  readonly gate = new ConsumerGate();

  /** The summary without the analyses: the counts, the history and the markers, quickly even on a huge store. */
  private readonly quick = new AsyncResource<HomeSummary>();
  /** The summary with the anomalies and exception groups, which are slow when cold. */
  private readonly full = new AsyncResource<HomeSummary>();
  /** The best summary so far: the full one once it arrives (the previous full one stays while a refresh runs). */
  readonly summary = computed(() => this.full.data() ?? this.quick.data());
  /** Nothing to show yet: loading or failed. Once there is a summary, a failure of the analyses only blanks their tiles. */
  readonly summaryLoading = computed(() => !this.summary() && (this.quick.loading() || this.full.loading()));
  readonly summaryError = computed(() => (this.summary() ? null : (this.quick.error() ?? this.full.error())));
  readonly summaryRetryable = computed(() => this.quick.retryable() && this.full.retryable());
  /** The analyses are running or failed: their tiles show a placeholder. */
  readonly analysing = computed(() => this.full.loading());
  /** The health check, shared with the timeline's headline. */
  readonly health = this.findings.findings;
  readonly storeOverview = new AsyncResource<StoreOverview>();
  readonly memory = new AsyncResource<StoreMemory>();

  /** The latest activity (the recent activity feed), re-read with every load: the log itself is not reactive. */
  readonly crumbs = signal<Breadcrumb[]>([]);

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
    this.load(true);
  }

  /** Retries a failed summary. */
  retrySummary(): void {
    this.loadSummary({ ...EMPTY_QUERY, groupBy: [...this.query.effectiveGroupBy()] });
  }

  private loadSummary(query: Query): void {
    void this.quick.load(() => this.home.querySummary({ query, buckets: HOME_BUCKETS, analyses: false }));
    void this.full.load(() => this.home.querySummary({ query, buckets: HOME_BUCKETS, analyses: true }));
  }

  private load(force = false): void {
    this.crumbs.set(this.activity.breadcrumbs(60));
    if (!this.data.summary()?.sources.length) {
      for (const r of [this.quick, this.full, this.storeOverview, this.memory]) r.reset();
      this.findings.clear();
      return;
    }
    // Everything opened, not the current filters: only the grouping (the lanes' instances) applies.
    const query = { ...EMPTY_QUERY, groupBy: [...this.query.effectiveGroupBy()] };
    this.loadSummary(query);
    if (force) this.findings.refresh();
    else this.findings.ensure();
    void this.storeOverview.load(() => this.storePane.overview());
    void this.memory.load(() => this.storePane.describeMemory());
  }
}
