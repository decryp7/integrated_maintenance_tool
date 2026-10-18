import { computed, DestroyRef, effect, inject, Injectable, signal, untracked } from '@angular/core';
import { LiveOptions, LiveState, TailReport } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { describeError, Logger } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { RECENT_STORE } from '../welcome/recent-store';

/** Watcher and engine settings sent with `set_live` (engine defaults for everything else). */
export const DEFAULT_LIVE_OPTIONS: LiveOptions = {
  watch: 'auto',
  pollIntervalMs: 2000,
  deletedFiles: 'keep',
  retention: { maxRecords: null, maxAgeMs: null },
  carryFlushMs: 2000,
  detectEveryMs: 1000,
};

/** Receives what each live refresh changed (null: the event carried no payload). */
export type LiveSink = (report: TailReport | null) => void;

/**
 * Live tail: while on, the engine watches the opened files and tells us ("imt://workspace-changed", with a
 * `TailReport`) what changed. This is the only handler of that event in the UI. By default each change
 * refreshes the summary through `WorkspaceStore.refreshSummary`, which reloads the timeline, fields and grid
 * while the query, filters and selection stay as they are; the live feature (`app/live`) connects a sink that
 * coalesces reports and refreshes only what changed. The grid asks `isLiveRefresh` to keep its scroll
 * position. Features that want to follow files (process monitor, PerfMon import) turn live tail on here.
 * On start it takes over the engine's live state for this window (`live_state`): after a webview reload the
 * engine may still be tailing, and the toggle and settings must show that.
 */
@Injectable({ providedIn: 'root' })
export class LiveTail {
  private readonly backend = inject(ImtBackend);
  private readonly store = inject(WorkspaceStore);
  private readonly logger = inject(Logger);
  private readonly recent = inject(RECENT_STORE);

  readonly enabled = signal(false);
  readonly error = signal<string | null>(null);
  /** Settings sent with every `set_live` (saved with the project / session by the live feature). */
  readonly options = signal<LiveOptions>(DEFAULT_LIVE_OPTIONS);
  /** There are opened files to follow (or live tail is on and can be turned off). */
  readonly available = computed(() => this.enabled() || (this.store.summary()?.sources.length ?? 0) > 0);
  /** `dataVersion` produced by the latest live refresh. */
  private liveVersion = -1;
  private unsubscribe: (() => void) | null = null;
  private refreshing = false;
  private again = false;
  private watchedSources = -1;
  private sink: LiveSink | null = null;
  /** Bumped by every user change (on/off, settings), so a late `live_state` answer does not undo it. */
  private changes = 0;

  constructor() {
    inject(DestroyRef).onDestroy(() => this.unsubscribe?.());
    void this.restore();
    // Files opened while live is on must be watched too: the engine asks to call set_live again.
    effect(() => {
      const count = this.store.summary()?.sources.length ?? 0;
      untracked(() => {
        if (this.enabled() && count !== this.watchedSources) {
          this.watchedSources = count;
          void this.backend.setLive(true, this.options()).catch((e) => this.fail(e));
        }
      });
    });
  }

  /** Routes change reports to `sink` instead of the plain summary refresh. Returns the function that disconnects it. */
  connect(sink: LiveSink): () => void {
    this.sink = sink;
    return () => {
      if (this.sink === sink) this.sink = null;
    };
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
    if (!this.enabled()) throw new Error(this.error() ?? $localize`:@@workspace.liveTail.cannotStart:Live tail could not start`);
  }

  /**
   * Takes over the engine's live state: its options, and when it is tailing, the change subscription and the "on"
   * state (without restarting the watchers). Skipped when the user changed something meanwhile. Never rejects.
   */
  async restore(): Promise<void> {
    const before = this.changes;
    let state: LiveState;
    try {
      state = await this.backend.liveState();
    } catch (e) {
      this.logger.warn('ui.live', 'Live tail state unavailable', { Error: describeError(e) });
      return;
    }
    if (this.changes !== before) return;
    this.options.set(state.options);
    if (!state.enabled || this.enabled()) return;
    try {
      const off = await this.backend.onWorkspaceChanged((report) => this.changed(report));
      if (this.changes !== before || this.enabled()) {
        off();
        return;
      }
      this.unsubscribe = off;
      this.watchedSources = this.store.summary()?.sources.length ?? 0;
      this.enabled.set(true);
      this.logger.info('ui.live', 'Live tail on (restored)');
    } catch (e) {
      this.fail(e);
    }
  }

  async set(on: boolean): Promise<void> {
    if (on === this.enabled()) return;
    this.changes++;
    this.error.set(null);
    try {
      if (on) {
        this.unsubscribe = await this.backend.onWorkspaceChanged((report) => this.changed(report));
        this.watchedSources = this.store.summary()?.sources.length ?? 0;
        await this.backend.setLive(true, this.options());
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

  /**
   * Changes the live settings. While live tail is on they are applied at once (`set_live` restarts the
   * watchers); otherwise they are used the next time it starts.
   */
  async setOptions(options: LiveOptions): Promise<void> {
    if (JSON.stringify(options) === JSON.stringify(this.options())) return;
    this.changes++;
    this.options.set(options);
    if (!this.enabled()) return;
    this.error.set(null);
    try {
      await this.backend.setLive(true, options);
      this.logger.info('ui.live', 'Live tail settings applied', { Watch: options.watch ?? 'auto' });
    } catch (e) {
      this.error.set(describeError(e));
      this.logger.warn('ui.live', 'Live tail settings failed', { Error: describeError(e) });
    }
  }

  /**
   * Opens files or folders and follows them live. With `startAtEnd` only lines written from now on are shown
   * (the files' history is skipped by the engine).
   */
  async openForLive(paths: string[], startAtEnd: boolean): Promise<void> {
    if (!paths.length) return;
    this.store.setError(null);
    try {
      const summary = startAtEnd ? await this.backend.openPathsLive(paths, true) : await this.backend.openPaths(paths);
      this.store.applySummary(summary);
      this.recent.add(paths);
      this.logger.info('ui.live', 'Opened for live tail', { Items: paths.length, StartAtEnd: startAtEnd });
    } catch (e) {
      this.store.setError(describeError(e));
      return;
    }
    await this.set(true);
  }

  /**
   * Fetches the summary again (marking the new data version as a live refresh); changes arriving meanwhile are
   * coalesced into one more refresh. Rejects never: failures are shown in `error`.
   */
  async refresh(): Promise<void> {
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

  private changed(report: TailReport | null): void {
    if (this.sink) this.sink(report);
    else void this.refresh();
  }

  private fail(e: unknown): void {
    this.error.set(describeError(e));
    this.enabled.set(false);
    this.logger.warn('ui.live', 'Live tail failed', { Error: describeError(e) });
  }
}
