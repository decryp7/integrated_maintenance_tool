import { computed, DestroyRef, effect, inject, Injectable, signal, untracked } from '@angular/core';
import { ImtBackend } from '../core/backend';
import { describeError, Logger } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';

/**
 * Live tail: while on, the engine watches the opened files and tells us ("imt://workspace-changed") when
 * it has re-imported something. We then refresh the summary through `WorkspaceStore.refreshSummary`, which
 * reloads the timeline, fields and grid while the query, filters and selection stay as they are. The grid
 * asks `isLiveRefresh` to keep its scroll position. This is the only handler of that event in the UI;
 * features that want to follow files (process monitor, PerfMon import) turn live tail on here.
 */
@Injectable({ providedIn: 'root' })
export class LiveTail {
  private readonly backend = inject(ImtBackend);
  private readonly store = inject(WorkspaceStore);
  private readonly logger = inject(Logger);

  readonly enabled = signal(false);
  readonly error = signal<string | null>(null);
  /** There are opened files to follow (or live tail is on and can be turned off). */
  readonly available = computed(() => this.enabled() || (this.store.summary()?.sources.length ?? 0) > 0);
  /** `dataVersion` produced by the latest live refresh. */
  private liveVersion = -1;
  private unsubscribe: (() => void) | null = null;
  private refreshing = false;
  private again = false;
  private watchedSources = -1;

  constructor() {
    inject(DestroyRef).onDestroy(() => this.unsubscribe?.());
    // Files opened while live is on must be watched too: the engine asks to call set_live again.
    effect(() => {
      const count = this.store.summary()?.sources.length ?? 0;
      untracked(() => {
        if (this.enabled() && count !== this.watchedSources) {
          this.watchedSources = count;
          void this.backend.setLive(true).catch((e) => this.fail(e));
        }
      });
    });
  }

  /** True when the current data version came from a live refresh (and not from open, reload or a profile save). */
  isLiveRefresh(): boolean {
    return this.store.dataVersion() === this.liveVersion;
  }

  toggle(): Promise<void> {
    return this.set(!this.enabled());
  }

  /** Turns live tail on for a feature that needs it; rejects with the reason when it could not start. */
  async start(): Promise<void> {
    await this.set(true);
    if (!this.enabled()) throw new Error(this.error() ?? 'Live tail could not start');
  }

  async set(on: boolean): Promise<void> {
    if (on === this.enabled()) return;
    this.error.set(null);
    try {
      if (on) {
        // The event may carry what changed ({appended, sourcesChanged, rotated, truncated}) or nothing;
        // either way the whole summary is fetched again, so the payload is not needed here.
        this.unsubscribe = await this.backend.onWorkspaceChanged(() => void this.refresh());
        this.watchedSources = this.store.summary()?.sources.length ?? 0;
        await this.backend.setLive(true);
      } else {
        this.unsubscribe?.();
        this.unsubscribe = null;
        await this.backend.setLive(false);
      }
      this.enabled.set(on);
      this.logger.info('ui.live', on ? 'Live tail on' : 'Live tail off');
    } catch (e) {
      this.unsubscribe?.();
      this.unsubscribe = null;
      this.fail(e);
    }
  }

  /** Fetches the summary again; changes arriving meanwhile are coalesced into one more refresh. */
  private async refresh(): Promise<void> {
    if (this.refreshing) {
      this.again = true;
      return;
    }
    this.refreshing = true;
    try {
      do {
        this.again = false;
        await this.store.refreshSummary();
        this.liveVersion = this.store.dataVersion();
      } while (this.again);
    } catch (e) {
      this.error.set(describeError(e));
      this.logger.warn('ui.live', 'Live refresh failed', { Error: describeError(e) });
    } finally {
      this.refreshing = false;
    }
  }

  private fail(e: unknown): void {
    this.error.set(describeError(e));
    this.enabled.set(false);
    this.logger.warn('ui.live', 'Live tail failed', { Error: describeError(e) });
  }
}
