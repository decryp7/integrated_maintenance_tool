import { effect, inject, Injectable, InjectionToken, Injector, Signal, untracked } from '@angular/core';
import { PANEL_WINDOWS } from '../../dock/panel-windows';
import { Query } from '../api.types';
import { WorkspaceStore } from '../workspace.store';
import { ActivityLog } from './activity-log';
import { safeText } from './privacy';
import { ActivityParams, Trigger } from './types';

/**
 * What the activity log watches of the dock (the port: the dock implements it, `provideDocking` provides it). A window
 * without a dock (a dialog window) provides none and loads none of the dock's code.
 */
export interface ActivityDock {
  readonly openIds: Signal<ReadonlySet<string>>;
  readonly autoHide: Signal<{ readonly items: readonly { readonly id: string }[] }>;
}
export const ACTIVITY_DOCK = new InjectionToken<ActivityDock>('ACTIVITY_DOCK');

/** Typing and dragging settle within this before one entry is logged. */
export const QUERY_DEBOUNCE_MS = 800;
/** Selection moves (arrow keys through the grid) are merged within this. */
export const SELECTION_THROTTLE_MS = 400;
/** Ids of coalesced operations remembered (to tell them from explicit operations). */
const MAX_OWN = 64;

const same = (a: readonly unknown[] | undefined, b: readonly unknown[] | undefined) => JSON.stringify(a ?? []) === JSON.stringify(b ?? []);

/** What changed between two queries, as activity: one key per kind of change. */
export function queryChanges(prev: Query, next: Query): ('search' | 'query' | 'filter' | 'timeRange')[] {
  const out: ('search' | 'query' | 'filter' | 'timeRange')[] = [];
  if (prev.text !== next.text || prev.regex !== next.regex || prev.caseSensitive !== next.caseSensitive) out.push('search');
  if ((prev.ql ?? '') !== (next.ql ?? '')) out.push('query');
  if (!same(prev.filters, next.filters) || !same(prev.levels, next.levels) || !same(prev.markers, next.markers) || !same(prev.groups, next.groups) || !same(prev.groupBy, next.groupBy))
    out.push('filter');
  if (prev.from !== next.from || prev.to !== next.to || JSON.stringify(prev.window ?? null) !== JSON.stringify(next.window ?? null)) out.push('timeRange');
  return out;
}

/**
 * Logs user operations that are state changes rather than calls: query, search, filter and time-range
 * edits (debounced), selection moves (throttled, ids only), and dock operations (open, close, auto-hide,
 * pop out, dock back). Watching the stores instead of instrumenting every component keeps one place for it.
 *
 * Only user-caused changes are logged: a change made while an explicit operation runs (a command) belongs
 * to that operation, and a change with no recent user input (session restore, live tail) is not a user
 * operation.
 */
@Injectable({ providedIn: 'root' })
export class ActivityObservers {
  private readonly log = inject(ActivityLog);
  private readonly injector = inject(Injector);
  private readonly own = new Set<string>();
  private started = false;

  /** Starts watching (idempotent). */
  start(): void {
    if (this.started) return;
    this.started = true;
    const store = this.injector.get(WorkspaceStore, null);
    if (store) {
      this.watch(store.query, (prev, next) => this.queryChanged(prev, next));
      this.watch(store.selected, (_, next) => {
        if (next) this.coalesce('selection', 'selection.change', { RecordId: next.id }, SELECTION_THROTTLE_MS);
      });
    }
    const dock = this.injector.get(ACTIVITY_DOCK, null);
    if (dock) {
      this.watch(dock.openIds, (prev, next) => this.setDiff(prev, next, 'dock.open', 'dock.close'));
      this.watch(dock.autoHide, (prev, next) =>
        this.setDiff(new Set(prev.items.map((i) => i.id)), new Set(next.items.map((i) => i.id)), 'dock.autoHide', 'dock.pin'),
      );
    }
    const windows = this.injector.get(PANEL_WINDOWS, null);
    if (windows) this.watch(windows.poppedIds, (prev, next) => this.setDiff(prev, next, 'dock.popout', 'dock.dockBack'));
  }

  /** Calls `changed(previous, next)` on every change after the first value. */
  private watch<T>(source: Signal<T>, changed: (prev: T, next: T) => void): void {
    let prev: T | undefined;
    let first = true;
    effect(
      () => {
        const next = source();
        untracked(() => {
          if (!first && prev !== next) {
            try {
              changed(prev as T, next);
            } catch {
              /* activity logging must never break the app */
            }
          }
          first = false;
          prev = next;
        });
      },
      { injector: this.injector },
    );
  }

  /**
   * The trigger of a change that is the user's own operation, or null when it is not. No time window: an input explains
   * its change however late it arrives, and a change without new input is the program's (a restore, the live tail).
   * - the rest of a burst of ours (typing, dragging): its trigger;
   * - led to by an input nothing has explained yet: inferred from that input;
   * - what the same input changed in this task (one click that docks a panel back and closes another): its trigger;
   * - led to by a weak operation (a call the user started with no domain name, e.g. a pop-out): that call's trigger;
   * - else (made by another operation, or by the program): not the user's.
   */
  private userTrigger(): Trigger | 'auto' | null {
    const runtime = this.log.runtime;
    const current = runtime.current();
    if (current && this.own.has(current.id) && runtime.coalescing(current.id)) return current.trigger;
    // A new input explains the change (an input after an operation started is never that operation's).
    if (runtime.triggers.pendingInput()) return 'auto';
    if (current && this.own.has(current.id) && runtime.triggers.explainedThisTask()) return current.trigger;
    if (current?.weak) return current.trigger;
    return null;
  }

  private coalesce(key: string, operation: string, params: ActivityParams, delayMs: number): void {
    const trigger = this.userTrigger();
    if (trigger === null) return;
    this.remember(this.log.coalesce(key, operation, trigger, params, delayMs));
    // Later keystrokes of the same burst explain their own changes; nothing else may claim them.
    this.log.runtime.triggers.consume();
  }

  private remember(id: string): void {
    this.own.add(id);
    if (this.own.size > MAX_OWN) this.own.delete(this.own.values().next().value as string);
  }

  private queryChanged(prev: Query, next: Query): void {
    const detail = this.log.detail;
    for (const change of queryChanges(prev, next)) {
      if (change === 'search') this.coalesce('search', 'search.change', { ...safeText('Search', next.text, detail), Regex: next.regex }, QUERY_DEBOUNCE_MS);
      else if (change === 'query') this.coalesce('query', 'query.change', safeText('Query', next.ql ?? '', detail), QUERY_DEBOUNCE_MS);
      else if (change === 'filter')
        this.coalesce('filter', 'filter.change', { Filters: next.filters.length, Levels: next.levels.join(','), Markers: next.markers.length, Groups: next.groups.length }, QUERY_DEBOUNCE_MS);
      else
        this.coalesce(
          'timeRange',
          'timeRange.change',
          {
            RangeMs: next.from != null && next.to != null ? next.to - next.from : undefined,
            Cleared: next.from == null && next.to == null && !next.window,
            LiveWindowMs: next.window?.lastMs,
          },
          QUERY_DEBOUNCE_MS,
        );
    }
  }

  private setDiff(prev: ReadonlySet<string>, next: ReadonlySet<string>, added: string, removed: string): void {
    // Typing in a field does not open or close panels: a dock change right after typing is the app's own
    // (a view opening for the data that just loaded).
    const trigger = this.userTrigger();
    if (trigger === null || (trigger === 'auto' ? this.log.runtime.triggers.infer() : trigger) === 'keyboard') return;
    for (const id of next) if (!prev.has(id)) this.remember(this.log.record(added, trigger, { Panel: id }));
    for (const id of prev) if (!next.has(id)) this.remember(this.log.record(removed, trigger, { Panel: id }));
  }
}
