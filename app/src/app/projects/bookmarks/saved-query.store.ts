import { inject } from '@angular/core';
import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { SavedQuery } from '../../core/tools.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { sanitizeQuery } from '../../lifecycle/core-ui-state';
import { newId } from './ids';
import { UndoStack } from '../../undo/undo-stack';
import { recordSnapshot } from '../../undo/snapshot-undo';

interface SavedQueryState {
  items: SavedQuery[];
}

export function parseSavedQueries(value: unknown): SavedQuery[] {
  if (!Array.isArray(value)) return [];
  return (value as Partial<SavedQuery>[])
    .filter((q): q is SavedQuery => !!q && typeof q.id === 'string' && typeof q.name === 'string' && !!q.query && typeof q.query === 'object')
    .map((q) => ({ id: q.id, name: q.name, query: sanitizeQuery(q.query) }));
}

/** Named queries (search text, levels, range, filters) that can be re-applied. Saved in the project. */
export const SavedQueryStore = signalStore(
  { providedIn: 'root' },
  withState<SavedQueryState>({ items: [] }),
  withMethods((store, workspace = inject(WorkspaceStore), undo = inject(UndoStack)) => {
    const state = { read: () => store.items(), write: (v: SavedQuery[]) => patchState(store, { items: v }) };
    const edit = (label: string, mutate: () => void, mergeKey?: string) => recordSnapshot(undo, state, label, mutate, { mergeKey });
    return {
      /** Saves the current query under a name (blank names become "Query N"). Uses the search box text, which may not be applied yet (debounce). */
      saveCurrent(name: string): void {
        const label = name.trim() || $localize`:Default name of a saved query|@@projects.savedQueries.defaultName:Query ${store.items().length + 1}:number:`;
        edit($localize`:Undo history entry|@@projects.savedQueries.undoSave:saved query`, () => patchState(store, (s) => ({ items: [...s.items, { id: newId('sq'), name: label, query: { ...structuredClone(workspace.query()), text: workspace.textInput() } }] })));
      },
      rename: (id: string, name: string) => edit($localize`:Undo history entry|@@projects.savedQueries.undoRename:saved query rename`, () => patchState(store, (s) => ({ items: s.items.map((q) => (q.id === id ? { ...q, name: name.trim() || q.name } : q)) })), `saved-query:${id}`),
      remove: (id: string) => edit($localize`:Undo history entry|@@projects.savedQueries.undoRemove:saved query removal`, () => patchState(store, (s) => ({ items: s.items.filter((q) => q.id !== id) }))),
      replace: (items: SavedQuery[]) => patchState(store, { items }),
      apply(id: string): void {
        const saved = store.items().find((q) => q.id === id);
        if (!saved) return;
        const query = sanitizeQuery(saved.query);
        workspace.patchQuery(query);
        workspace.setTextInput(query.text);
      },
    };
  }),
);
