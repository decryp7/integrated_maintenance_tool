import { DestroyRef, effect, inject, Injectable, signal, untracked } from '@angular/core';
import { TailReport } from '../core/api.types';
import { Logger } from '../core/logger';
import { LiveTail } from '../workspace/live-tail';
import { MIN_REFRESH_INTERVAL_MS, RefreshCoalescer, Throttle } from './coalescer';
import { LiveFollow } from './live-follow.store';
import { changesSources, planRefresh, unknownChange } from './live-report';
import { TailStatusStore } from './tail-status.store';

/** Facet counts refresh at most this often while live tail runs (they scan every matching event). */
export const FACET_INTERVAL_MS = 2000;
/** The tail status (per-file table, sidebar badges) refreshes at most this often from live changes. */
export const STATUS_INTERVAL_MS = 1000;

/**
 * Turns live change reports into the smallest set of UI refreshes. Reports are coalesced (at most one refresh
 * per `MIN_REFRESH_INTERVAL_MS`, never overlapping), then, according to `planRefresh`:
 * the summary is fetched again (which refreshes the timeline buckets and the charts in view through the data
 * version), facet counts are refreshed at most every `FACET_INTERVAL_MS`, the grid reloads the rows in view when
 * following or only counts the new events otherwise ("N new events"), and the tail status is refreshed.
 */
@Injectable({ providedIn: 'root' })
export class LiveFeed {
  private readonly live = inject(LiveTail);
  private readonly follow = inject(LiveFollow);
  private readonly status = inject(TailStatusStore);
  private readonly logger = inject(Logger);

  /** Bumped (throttled) when facet counts should be fetched again. */
  readonly facetVersion = signal(0);
  /** New events not shown in the grid because it is not following. */
  readonly newEvents = signal(0);
  /** Bumped when the grid should reload and show the newest events (pill click, Resume). */
  readonly revealNonce = signal(0);
  /** Timestamp of the newest record reported by live tail. */
  readonly newestTs = signal<number | null>(null);
  /** Number of UI refreshes done (diagnostics, tests). */
  readonly refreshes = signal(0);

  /** How the latest live refresh treats the grid: reload the rows in view, or keep them and count. */
  private gridMode: 'follow' | 'count' = 'follow';
  private readonly coalescer = new RefreshCoalescer((r) => this.apply(r), MIN_REFRESH_INTERVAL_MS);
  private readonly facets = new Throttle(() => this.facetVersion.update((v) => v + 1), FACET_INTERVAL_MS);
  private readonly statusThrottle = new Throttle(() => void this.status.refresh(), STATUS_INTERVAL_MS);

  constructor() {
    const disconnect = this.live.connect((report) => this.coalescer.push(report ?? unknownChange()));
    inject(DestroyRef).onDestroy(() => {
      disconnect();
      this.coalescer.stop();
      this.facets.cancel();
      this.statusThrottle.cancel();
    });

    // Live tail off: nothing is pending any more; on: the status table starts from fresh numbers.
    let wasOn = false;
    effect(() => {
      const on = this.live.enabled();
      untracked(() => {
        if (on === wasOn) return;
        wasOn = on;
        if (on) {
          this.coalescer.resume();
        } else {
          this.coalescer.stop();
          this.newEvents.set(0);
          this.gridMode = 'follow';
        }
        void this.status.refresh();
      });
    });

    // Following again (Resume, Follow on): show what was held back.
    effect(() => {
      const nonce = this.follow.resumeNonce();
      untracked(() => {
        if (nonce > 0) this.showNewEvents();
      });
    });
  }

  /** True when the grid should keep its rows for the current live refresh (not following). */
  holdGrid(): boolean {
    return this.gridMode === 'count';
  }

  /** Clears the "N new events" count and asks the grid to reload with the newest events in view. */
  showNewEvents(): void {
    this.newEvents.set(0);
    this.gridMode = 'follow';
    this.revealNonce.update((n) => n + 1);
  }

  /** Applies one coalesced report. */
  private async apply(report: TailReport): Promise<void> {
    if (!this.live.enabled()) return;
    const plan = planRefresh(report, !this.follow.holding());
    if (report.newestTs !== null) this.newestTs.set(report.newestTs);
    if (plan.grid === 'count') this.newEvents.update((n) => n + report.appended);
    if (plan.grid !== 'none') this.gridMode = plan.grid;
    this.refreshes.update((n) => n + 1);
    // Badges first: the report says which files changed state (the status refresh below stays the fallback).
    this.status.applyFileStates(report.fileStates);
    if (plan.summary) await this.live.refresh();
    if (plan.facets) this.facets.request();
    if (plan.status) {
      if (changesSources(report)) void this.status.refresh();
      else this.statusThrottle.request();
    }
    this.logger.debug('ui.live', 'Live refresh', { Appended: report.appended, Grid: plan.grid });
  }
}
