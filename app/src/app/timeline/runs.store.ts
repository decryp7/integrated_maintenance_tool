import { computed, effect, inject, Injectable, signal, untracked } from '@angular/core';
import { RunsReply, RunsRequest, RunSort, RunStatus } from '../core/api.types';
import { describeError, Logger } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { RunsBackend } from './runs-backend';

/** Runs one page asks for; "Show more" adds another. */
export const RUNS_PAGE = 200;

const EMPTY: RunsReply = { summaries: [], rules: [], runs: [], total: 0, offset: 0 };

/**
 * State of the Runs view: the sort, filters and the rows the engine returned. Reloads whenever the data changes or a
 * parameter does; responses to superseded requests are ignored. Sorting, filtering and the summaries are the engine's.
 */
@Injectable({ providedIn: 'root' })
export class RunsStore {
  private readonly backend = inject(RunsBackend);
  private readonly ws = inject(WorkspaceStore);
  private readonly logger = inject(Logger);

  readonly sort = signal<RunSort>('start');
  readonly descending = signal(false);
  /** Only this span kind (null = all). */
  readonly kind = signal<string | null>(null);
  /** Only this status (null = all). */
  readonly status = signal<RunStatus | null>(null);
  /** Only runs overlapping the timeline's visible range. */
  readonly inView = signal(false);
  readonly limit = signal(RUNS_PAGE);

  readonly reply = signal<RunsReply>(EMPTY);
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);
  readonly loaded = signal(false);

  readonly rows = computed(() => this.reply().runs);
  readonly summaries = computed(() => this.reply().summaries);
  readonly more = computed(() => Math.max(0, this.reply().total - this.reply().runs.length));

  private seq = 0;
  private readonly nonce = signal(0);

  constructor() {
    effect(() => {
      this.ws.dataVersion();
      const t = this.inView() ? this.ws.timeline() : null;
      const status = this.status();
      const req = {
        sort: this.sort(),
        descending: this.descending(),
        kind: this.kind(),
        statuses: status ? [status] : [],
        limit: this.limit(),
        from: t?.from ?? null,
        to: t?.to ?? null,
      };
      this.nonce();
      const hasData = !!this.ws.summary()?.sources.length;
      untracked(() => void this.load(hasData ? req : null));
    });
  }

  private async load(req: RunsRequest | null): Promise<void> {
    const mine = ++this.seq;
    if (!req) {
      this.reply.set(EMPTY);
      this.error.set(null);
      this.loaded.set(true);
      return;
    }
    this.loading.set(true);
    try {
      const reply = await this.backend.queryRuns(req);
      if (mine !== this.seq) return;
      this.reply.set(reply && Array.isArray(reply.runs) ? reply : EMPTY);
      this.error.set(null);
    } catch (e) {
      if (mine !== this.seq) return;
      this.error.set(describeError(e));
      this.logger.warn('ui.runs', 'Runs could not be loaded', { Error: describeError(e) });
    } finally {
      if (mine === this.seq) {
        this.loading.set(false);
        this.loaded.set(true);
      }
    }
  }

  retry(): void {
    this.nonce.update((n) => n + 1);
  }

  /** Sorts by a column; the same column again flips the direction. */
  sortBy(column: RunSort): void {
    if (this.sort() === column) this.descending.update((d) => !d);
    else {
      this.sort.set(column);
      this.descending.set(column === 'duration');
    }
    this.limit.set(RUNS_PAGE);
  }

  showMore(): void {
    this.limit.update((n) => Math.min(1000, n + RUNS_PAGE));
  }
}
