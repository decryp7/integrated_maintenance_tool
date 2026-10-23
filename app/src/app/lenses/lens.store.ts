import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { WorkspaceStore } from '../core/workspace.store';
import { HighlightRules } from '../projects/bookmarks/highlight-rules.store';
import { newId } from '../projects/bookmarks/ids';
import { recordSnapshot } from '../undo/snapshot-undo';
import { UndoStack } from '../undo/undo-stack';
import { FilterSetStore } from './filter-set.store';
import { Lens, lensFromProfile, lensQueryOf } from './lens-model';

interface LensState {
  /** Lenses the user saved (in the project). */
  saved: Lens[];
  /** The lens applied last, for the menu; null after a reset. */
  activeId: string | null;
  /** The chart the active lens brings forward in Metrics. */
  focusChart: string | null;
}

/**
 * Lenses: named view presets. The profile's lenses are built in (the engine validated them and reports them in the
 * summary); the user saves the current view as a lens. Applying one sets the query, the extra columns, adds its row
 * highlights and picks the chart to show first. Filter sets and the time range are not touched.
 */
export const LensStore = signalStore(
  { providedIn: 'root' },
  withState<LensState>({ saved: [], activeId: null, focusChart: null }),
  withComputed((store, workspace = inject(WorkspaceStore)) => {
    const builtIn = computed(() => (workspace.summary()?.lenses ?? []).map(lensFromProfile));
    const all = computed(() => {
      const ids = new Set(builtIn().map((l) => l.id));
      return [...builtIn(), ...store.saved().filter((l) => !ids.has(l.id))];
    });
    return { builtIn, all, active: computed(() => all().find((l) => l.id === store.activeId()) ?? null) };
  }),
  withMethods((store, workspace = inject(WorkspaceStore), highlights = inject(HighlightRules), sets = inject(FilterSetStore), undo = inject(UndoStack)) => {
    const state = { read: () => store.saved(), write: (saved: Lens[]) => patchState(store, { saved }) };
    const edit = (label: string, mutate: () => void, mergeKey?: string) => recordSnapshot(undo, state, label, mutate, { mergeKey });
    return {
      /** Saves the current view (query incl. the search box text, extra columns, enabled highlight rules, focused chart) as a lens; returns its id. */
      saveCurrent(name: string): string {
        const id = newId('lens');
        const label = name.trim() || $localize`:Default name of a saved lens|@@lenses.defaultName:Lens ${store.saved().length + 1}:number:`;
        const query = lensQueryOf({ ...workspace.query(), text: workspace.textInput() });
        const lens: Lens = {
          id,
          name: label,
          description: '',
          query,
          columns: [...workspace.extraColumns()],
          highlights: highlights.items().filter((r) => r.enabled).map((r) => ({ field: r.field, op: r.op, value: r.value, color: r.color })),
          chart: store.focusChart(),
        };
        edit($localize`:Undo history entry|@@lenses.undoSave:saved lens`, () => patchState(store, (s) => ({ saved: [...s.saved, lens], activeId: id })));
        return id;
      },
      rename: (id: string, name: string) =>
        edit($localize`:Undo history entry|@@lenses.undoRename:lens rename`, () => patchState(store, (s) => ({ saved: s.saved.map((l) => (l.id === id ? { ...l, name: name.trim() || l.name } : l)) })), `lens:${id}`),
      remove: (id: string) =>
        edit($localize`:Undo history entry|@@lenses.undoRemove:lens removal`, () => patchState(store, (s) => ({ saved: s.saved.filter((l) => l.id !== id), activeId: s.activeId === id ? null : s.activeId }))),
      /** Puts back saved state (project open, session restore); not an undoable edit. */
      replace: (saved: Lens[]) => patchState(store, { saved, activeId: null, focusChart: null }),
      /** Switches to a lens; an unknown id does nothing. */
      apply(id: string): void {
        const lens = store.all().find((l) => l.id === id);
        if (!lens) return;
        const q = lens.query;
        workspace.patchQuery({ levels: [...q.levels], regex: q.regex, ql: q.ql, markers: [...q.markers], filters: q.filters.map((f) => ({ ...f })), groups: [] });
        workspace.setTextInput(q.text);
        if (lens.columns.length) workspace.setColumns(lens.columns);
        for (const h of lens.highlights) {
          if (!highlights.items().some((r) => r.field === h.field && r.op === h.op && r.value === h.value && r.color === h.color)) highlights.add(h);
        }
        patchState(store, { activeId: id, focusChart: lens.chart });
      },
      /** Moves the current filters into a new, enabled filter set (the query keeps none of them); returns the set id, or null when there are no filters. */
      saveFiltersAsSet(name: string): string | null {
        const filters = workspace.query().filters;
        if (!filters.length) return null;
        const id = sets.add(name, filters);
        workspace.patchQuery({ filters: [] });
        return id;
      },
    };
  }),
);
