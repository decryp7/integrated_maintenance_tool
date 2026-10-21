import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { FieldFilter, Level, Query } from '../api.types';
import { WorkspaceDataStore } from './workspace-data.store';

export const EMPTY_QUERY: Query = {
  from: null,
  to: null,
  levels: [],
  text: '',
  regex: false,
  caseSensitive: false,
  filters: [],
  groupBy: [],
  groups: [],
  markers: [],
};

/** How long the search box waits after the last keystroke before its text becomes part of the query. */
export const TEXT_DEBOUNCE_MS = 250;

interface QueryState {
  query: Query;
  /** Raw search box text; copied into query.text after a debounce. */
  textInput: string;
}

/** The workspace's query (filters, range, groups, search text) and the derived query views send to the engine. */
export const QueryStore = signalStore(
  { providedIn: 'root' },
  withState<QueryState>({ query: EMPTY_QUERY, textInput: '' }),
  withComputed((store, data = inject(WorkspaceDataStore)) => {
    const effectiveGroupBy = computed(() => {
      const q = store.query();
      return q.groupBy.length ? q.groupBy : (data.summary()?.defaultGroupBy ?? []);
    });
    return {
      effectiveGroupBy,
      /** The query with the profile's default grouping filled in. */
      effectiveQuery: computed<Query>(() => ({ ...store.query(), groupBy: effectiveGroupBy() })),
      hasRange: computed(() => store.query().from != null || store.query().to != null),
      markedOnly: computed(() => store.query().markers.length > 0),
    };
  }),
  withMethods((store, data = inject(WorkspaceDataStore)) => {
    let textTimer: ReturnType<typeof setTimeout> | undefined;
    const setQuery = (patch: Partial<Query>) => patchState(store, { query: { ...store.query(), ...patch } });
    return {
      patchQuery: (patch: Partial<Query>) => setQuery(patch),
      setTextInput(text: string) {
        patchState(store, { textInput: text });
        clearTimeout(textTimer);
        // Smart search: word matches first; the engine's background scan may add partial matches (SmartSearchStore).
        textTimer = setTimeout(() => setQuery({ text, textMode: 'words' }), TEXT_DEBOUNCE_MS);
      },
      setRegex: (regex: boolean) => setQuery({ regex }),
      setCaseSensitive: (caseSensitive: boolean) => setQuery({ caseSensitive }),
      toggleLevel(level: Level) {
        const levels = store.query().levels;
        setQuery({ levels: levels.includes(level) ? levels.filter((l) => l !== level) : [...levels, level] });
      },
      addFilter(filter: FieldFilter) {
        const filters = store.query().filters.filter((f) => !(f.field === filter.field && f.op === filter.op && f.value === filter.value));
        setQuery({ filters: [...filters, filter] });
      },
      removeFilter: (index: number) => setQuery({ filters: store.query().filters.filter((_, i) => i !== index) }),
      setGroupBy: (groupBy: string[]) => setQuery({ groupBy, groups: [] }),
      toggleGroup(key: string) {
        const groups = store.query().groups;
        setQuery({ groups: groups.includes(key) ? groups.filter((g) => g !== key) : [...groups, key] });
      },
      clearGroups: () => setQuery({ groups: [] }),
      setRange: (from: number | null, to: number | null) => setQuery({ from, to }),
      resetRange: () => setQuery({ from: null, to: null }),
      setMarkedOnly(on: boolean) {
        setQuery({ markers: on ? (data.summary()?.markerKinds ?? []).map((k) => k.kind) : [] });
      },
      clearQuery() {
        clearTimeout(textTimer);
        patchState(store, { query: EMPTY_QUERY, textInput: '' });
      },
      /** Puts back a whole query, search box included (undo/redo of filter changes). */
      restoreQuery(query: Query) {
        clearTimeout(textTimer);
        patchState(store, { query, textInput: query.text });
      },
    };
  }),
);
