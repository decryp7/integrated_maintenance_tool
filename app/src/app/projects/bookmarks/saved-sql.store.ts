import { inject } from '@angular/core';
import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { recordSnapshot } from '../../undo/snapshot-undo';
import { UndoStack } from '../../undo/undo-stack';
import { newId } from './ids';

/** A named SQL text of the SQL console (ADR 0030), saved next to the saved queries. */
export interface SavedSql {
  id: string;
  name: string;
  sql: string;
}

/** Longest saved SQL text (the engine runs at most 64 KB). */
const MAX_SQL = 64 * 1024;

export function parseSavedSql(value: unknown): SavedSql[] {
  if (!Array.isArray(value)) return [];
  return (value as Partial<SavedSql>[])
    .filter((q): q is SavedSql => !!q && typeof q.id === 'string' && typeof q.name === 'string' && typeof q.sql === 'string')
    .map((q) => ({ id: q.id, name: q.name, sql: q.sql.slice(0, MAX_SQL) }));
}

/** Saved SQL texts. Saved in the project (UI state key `savedSql`). */
export const SavedSqlStore = signalStore(
  { providedIn: 'root' },
  withState<{ items: SavedSql[] }>({ items: [] }),
  withMethods((store, undo = inject(UndoStack)) => {
    const state = { read: () => store.items(), write: (v: SavedSql[]) => patchState(store, { items: v }) };
    const edit = (label: string, mutate: () => void) => recordSnapshot(undo, state, label, mutate);
    return {
      /** Saves `sql` under a name (blank names become "SQL N"). */
      save(name: string, sql: string): void {
        if (!sql.trim()) return;
        const label = name.trim() || $localize`:Default name of a saved SQL text|@@projects.savedSql.defaultName:SQL ${store.items().length + 1}:number:`;
        edit($localize`:Undo history entry|@@projects.savedSql.undoSave:saved SQL`, () =>
          patchState(store, (s) => ({ items: [...s.items, { id: newId('sql'), name: label, sql: sql.slice(0, MAX_SQL) }] })),
        );
      },
      remove: (id: string) => edit($localize`:Undo history entry|@@projects.savedSql.undoRemove:saved SQL removal`, () => patchState(store, (s) => ({ items: s.items.filter((q) => q.id !== id) }))),
      replace: (items: SavedSql[]) => patchState(store, { items }),
    };
  }),
);
