import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { FieldFilter } from '../core/api.types';
import { newId } from '../projects/bookmarks/ids';
import { recordSnapshot } from '../undo/snapshot-undo';
import { UndoStack } from '../undo/undo-stack';
import { FilterSet } from './lens-model';

/**
 * Filter sets: named groups of filters switched on and off as a whole (chips). The enabled sets' filters
 * are added to the query the views send to the engine (`QueryStore.effectiveQuery`); the user's own filters
 * stay in the query. Saved in the project. Has no dependency on the workspace so the query store can use it.
 */
export const FilterSetStore = signalStore(
  { providedIn: 'root' },
  withState<{ sets: FilterSet[] }>({ sets: [] }),
  withComputed((store) => ({
    /** The filters of the enabled sets, in set order. */
    enabledFilters: computed<FieldFilter[]>(() => store.sets().flatMap((s) => (s.enabled ? s.filters : []))),
  })),
  withMethods((store, undo = inject(UndoStack)) => {
    const state = { read: () => store.sets(), write: (sets: FilterSet[]) => patchState(store, { sets }) };
    const edit = (label: string, mutate: () => void, mergeKey?: string) => recordSnapshot(undo, state, label, mutate, { mergeKey });
    return {
      /** Adds an enabled set (blank names become "Set N"); returns its id. */
      add(name: string, filters: FieldFilter[]): string {
        const id = newId('fs');
        const label = name.trim() || $localize`:Default name of a filter set|@@lenses.filterSets.defaultName:Set ${store.sets().length + 1}:number:`;
        edit($localize`:Undo history entry|@@lenses.filterSets.undoAdd:new filter set`, () => patchState(store, (s) => ({ sets: [...s.sets, { id, name: label, filters: filters.map((f) => ({ ...f })), enabled: true }] })));
        return id;
      },
      toggle: (id: string) =>
        edit($localize`:Undo history entry|@@lenses.filterSets.undoToggle:filter set switch`, () => patchState(store, (s) => ({ sets: s.sets.map((x) => (x.id === id ? { ...x, enabled: !x.enabled } : x)) }))),
      rename: (id: string, name: string) =>
        edit($localize`:Undo history entry|@@lenses.filterSets.undoRename:filter set rename`, () => patchState(store, (s) => ({ sets: s.sets.map((x) => (x.id === id ? { ...x, name: name.trim() || x.name } : x)) })), `filter-set:${id}`),
      remove: (id: string) => edit($localize`:Undo history entry|@@lenses.filterSets.undoRemove:filter set removal`, () => patchState(store, (s) => ({ sets: s.sets.filter((x) => x.id !== id) }))),
      /** Puts back saved state (project open, session restore); not an undoable edit. */
      replace: (sets: FilterSet[]) => patchState(store, { sets }),
    };
  }),
);
