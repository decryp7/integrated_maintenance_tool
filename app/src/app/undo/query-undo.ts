import { effect, inject, Injectable, Injector, untracked } from '@angular/core';
import { Query } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { UndoStack } from './undo-stack';

/**
 * The parts of the query that undo covers. The time range (`from`/`to`, `window`) is left out on purpose: it
 * changes with every zoom and pan and has its own Back/Forward history (Analyze menu), so undoing a filter
 * never jumps the timeline. Instance selection (`groups`) is timeline interaction, also left out.
 */
const TRACKED = ['text', 'regex', 'caseSensitive', 'levels', 'filters', 'groupBy', 'markers', 'ql'] as const;
type Tracked = Pick<Query, (typeof TRACKED)[number]>;

/** The undoable part of a query. */
export function trackedPart(q: Query): Tracked {
  return { text: q.text, regex: q.regex, caseSensitive: q.caseSensitive, levels: q.levels, filters: q.filters, groupBy: q.groupBy, markers: q.markers, ql: q.ql ?? '' };
}

/** "Undo <label>" for a change between two queries (the first tracked difference names it). */
export function queryChangeLabel(before: Tracked, after: Tracked): string {
  const differs = (k: keyof Tracked) => JSON.stringify(before[k]) !== JSON.stringify(after[k]);
  if (differs('filters')) return 'field filter change';
  if (differs('levels')) return 'level filter change';
  if (differs('text')) return 'search text change';
  if (differs('regex') || differs('caseSensitive')) return 'search option change';
  if (differs('markers')) return 'marked-only filter change';
  if (differs('groupBy')) return 'group-by change';
  return 'query change';
}

/**
 * Records filter and query changes on the undo stack, whichever control made them (filter bar, facets,
 * context menus, saved queries, palette commands). Undo puts the tracked part back and keeps the current time
 * range. Typing in the search box is merged into one entry.
 */
@Injectable({ providedIn: 'root' })
export class QueryUndo {
  private readonly store = inject(WorkspaceStore);
  private readonly undo = inject(UndoStack);
  private readonly injector = inject(Injector);
  private last: Tracked | null = null;
  private started = false;

  /** Starts observing (idempotent). */
  start(): void {
    if (this.started) return;
    this.started = true;
    effect(
      () => {
        const q = this.store.query();
        untracked(() => this.observe(q));
      },
      { injector: this.injector },
    );
  }

  /** Takes the current query as the new starting point without recording (a project or session was opened). */
  rebase(): void {
    this.last = trackedPart(this.store.query());
  }

  /** @internal Exposed for tests. */
  observe(q: Query): void {
    const next = trackedPart(q);
    const prev = this.last;
    this.last = next;
    if (!prev || JSON.stringify(prev) === JSON.stringify(next)) return;
    const label = queryChangeLabel(prev, next);
    this.undo.record({
      label,
      mergeKey: label === 'search text change' ? 'query.text' : undefined,
      undo: () => this.restore(prev),
      do: () => this.restore(next),
    });
  }

  private restore(part: Tracked): void {
    // Seen as the starting point, so the effect does not record the restore itself.
    this.last = part;
    this.store.restoreQuery({ ...this.store.query(), ...part });
  }
}
