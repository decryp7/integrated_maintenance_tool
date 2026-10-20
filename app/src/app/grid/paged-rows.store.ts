import { signal } from '@angular/core';
import { Page, PageRequest, Query, RecordView } from '../core/api.types';
import { describeError } from '../core/logger';
import { Debouncer } from '../shared/throttle';

/** Rows per page fetched from the engine. */
export const PAGE = 500;
/** Pages kept in memory; the ones farthest from the viewport are dropped first. */
export const MAX_PAGES = 40;
/** Pages for the rows in view are fetched once scrolling has paused this long. */
export const SCROLL_IDLE_MS = 50;

/**
 * What a live refresh does with the rows in view once its first page arrives:
 * - `end`: follow the newest events (scroll to the end);
 * - `shift`: new events were inserted above the rows in view (newest first): keep the same events in view;
 * - `keep`: stay where the view is.
 */
export type LiveMode = 'end' | 'shift' | 'keep';

/**
 * The live-follow policy: what a refresh does, given the sort order, where the view is and whether the user asked
 * to reveal the newest events ("N new events" or resumed following).
 */
export function liveMode(descending: boolean, atStart: boolean, atEnd: boolean, reveal?: 'start' | 'end'): LiveMode {
  if (reveal === 'end') return 'end';
  if (reveal === 'start') return 'keep';
  if (descending) return atStart ? 'keep' : 'shift';
  return atEnd ? 'end' : 'keep';
}

/** The first response of a live refresh: how many rows were added and what to do with the view. */
export interface LiveArrival {
  mode: LiveMode;
  newTotal: number;
  delta: number;
}

export interface PagedRowsOptions {
  fetch(request: PageRequest): Promise<Page>;
  /** Every successful page: the match total (the workspace shows it). */
  onTotal?(total: number): void;
  /** The first page of a live refresh arrived. */
  onLive?(arrival: LiveArrival): void;
  /** A page arrived (rows that were waiting for it can be selected now). */
  onPage?(): void;
  /** The row at the top of the view: eviction keeps the pages around it. */
  centre?(): number;
  pageSize?: number;
  maxPages?: number;
  idleMs?: number;
  now?: () => number;
}

/**
 * The events grid's rows, without any DOM: a sparse cache of pages for one query and sort order, fetched on demand,
 * bounded (pages far from the view are evicted), keyed by a generation so late answers for an old query are dropped.
 * A page that failed is not asked for again until `retry` (scrolling over it must not hammer a failing engine).
 * While the user scrolls, missing pages are fetched only once scrolling pauses (`fill`).
 */
export class PagedRowsStore {
  readonly pageSize: number;
  private readonly maxPages: number;
  private readonly now: () => number;
  private readonly idle: Debouncer;
  private readonly idleMs: number;

  /** Rows matching the query (from the latest page). */
  readonly total = signal(0);
  /** At least one page arrived for the current query. */
  readonly loaded = signal(false);
  /** Why the last page failed (null when none did since). */
  readonly error = signal<string | null>(null);
  /** Bumped whenever the cache changed (rows arrived, were evicted or dropped). */
  readonly version = signal(0);

  private pages = new Map<number, RecordView[]>();
  private readonly inflight = new Set<number>();
  private stale = new Set<number>();
  private readonly failed = new Set<number>();
  private gen = 0;
  private currentQuery: Query | null = null;
  private desc = false;
  private scrolledAt = Number.NEGATIVE_INFINITY;
  private pendingLive: { oldTotal: number; mode: LiveMode } | null = null;

  constructor(private readonly options: PagedRowsOptions) {
    this.pageSize = options.pageSize ?? PAGE;
    this.maxPages = options.maxPages ?? MAX_PAGES;
    this.idleMs = options.idleMs ?? SCROLL_IDLE_MS;
    this.now = options.now ?? (() => performance.now());
    this.idle = new Debouncer(this.idleMs);
  }

  /** The query the rows are for (null before the first `reset`). */
  get query(): Query | null {
    return this.currentQuery;
  }

  get descending(): boolean {
    return this.desc;
  }

  /** Changes on every reset, live refresh and full retry: answers to older requests are dropped. */
  get generation(): number {
    return this.gen;
  }

  /** Pages in memory (for tests and diagnostics). */
  get pageCount(): number {
    return this.pages.size;
  }

  pageOf(index: number): number {
    return Math.floor(index / this.pageSize);
  }

  rowAt(index: number): RecordView | null {
    return this.pages.get(this.pageOf(index))?.[index % this.pageSize] ?? null;
  }

  /** A new query or sort order: drops everything and fetches the first page. */
  reset(query: Query, descending: boolean): void {
    this.gen++;
    this.currentQuery = query;
    this.desc = descending;
    this.pages = new Map();
    this.inflight.clear();
    this.stale.clear();
    this.failed.clear();
    this.pendingLive = null;
    this.loaded.set(false);
    this.error.set(null);
    this.bump();
    void this.fetchPage(0);
  }

  /**
   * The engine's data changed under the same query (live tail): fetches the first page, the pages of `first..last`
   * (the rows in view) and, when following the end, the old last page. The old rows stay until the new ones arrive,
   * except with `shift`, where indexes moved and old rows would show the wrong events.
   */
  liveRefresh(mode: LiveMode, first: number, last: number): void {
    const oldTotal = this.total();
    this.gen++;
    this.inflight.clear();
    this.failed.clear();
    this.error.set(null);
    if (mode === 'shift') {
      this.pages = new Map();
      this.stale.clear();
    } else {
      this.stale = new Set(this.pages.keys());
    }
    this.pendingLive = { oldTotal, mode };
    this.bump();
    const wanted = new Set<number>([0, this.pageOf(first), this.pageOf(Math.max(first, last - 1))]);
    if (mode === 'end') wanted.add(this.pageOf(Math.max(0, oldTotal - 1)));
    for (const page of wanted) this.ensurePage(page);
  }

  /** The user scrolled: page fetches wait until scrolling pauses. */
  noteScroll(): void {
    this.scrolledAt = this.now();
  }

  /**
   * Fetches the pages of `missing` rows: at once, or once scrolling has paused when the user is scrolling (dragging
   * the scrollbar through a million rows must not ask the engine for every page it passes).
   */
  fill(missing: () => Iterable<number>): void {
    if (this.now() - this.scrolledAt < this.idleMs) this.idle.run(() => this.ensureRows(missing()));
    else {
      this.idle.cancel();
      this.ensureRows(missing());
    }
  }

  /** Fetches the page of each row index unless it is loaded, in flight or failed. */
  ensureRows(indexes: Iterable<number>): void {
    for (const i of indexes) this.ensurePage(this.pageOf(i));
  }

  ensurePage(page: number): void {
    if (this.failed.has(page) || this.inflight.has(page) || (this.pages.has(page) && !this.stale.has(page))) return;
    void this.fetchPage(page);
  }

  /** Retries after a failure: from scratch when nothing loaded, else the failed pages and the `missing` rows. */
  retry(missing: Iterable<number>): void {
    this.error.set(null);
    if (!this.loaded() || !this.pages.size) {
      this.failed.clear();
      this.gen++;
      this.inflight.clear();
      void this.fetchPage(0);
      return;
    }
    const pages = new Set(this.failed);
    this.failed.clear();
    for (const i of missing) pages.add(this.pageOf(i));
    for (const page of pages) this.ensurePage(page);
  }

  /** Keeps memory bounded: drops the pages farthest from `centreRow` beyond `maxPages`. */
  evictFar(centreRow: number): void {
    if (this.pages.size <= this.maxPages) return;
    const centre = this.pageOf(centreRow);
    const far = [...this.pages.keys()].filter((p) => !this.inflight.has(p)).sort((a, b) => Math.abs(b - centre) - Math.abs(a - centre));
    for (const page of far.slice(0, this.pages.size - this.maxPages)) {
      this.pages.delete(page);
      this.stale.delete(page);
    }
  }

  /** Sets the total from an answer outside the cache (a jump to a record). */
  setTotal(total: number): void {
    this.total.set(total);
  }

  /** Stops waiting for scrolling to pause. */
  dispose(): void {
    this.idle.cancel();
  }

  private async fetchPage(page: number): Promise<void> {
    const gen = this.gen;
    const query = this.currentQuery;
    if (!query) return;
    this.inflight.add(page);
    try {
      const res = await this.options.fetch({ query, offset: page * this.pageSize, limit: this.pageSize, descending: this.desc, anchor: null });
      if (gen !== this.gen) return;
      this.pages.set(page, res.rows);
      this.stale.delete(page);
      this.failed.delete(page);
      this.evictFar(this.options.centre?.() ?? 0);
      this.total.set(res.total);
      this.loaded.set(true);
      this.error.set(null);
      this.options.onTotal?.(res.total);
      this.arriveLive(res.total);
      this.bump();
      this.options.onPage?.();
    } catch (e) {
      if (gen === this.gen) {
        this.failed.add(page);
        this.error.set(describeError(e));
      }
    } finally {
      this.inflight.delete(page);
    }
  }

  private arriveLive(newTotal: number): void {
    const live = this.pendingLive;
    if (!live) return;
    this.pendingLive = null;
    this.options.onLive?.({ mode: live.mode, newTotal, delta: newTotal - live.oldTotal });
  }

  private bump(): void {
    this.version.update((v) => v + 1);
  }
}
