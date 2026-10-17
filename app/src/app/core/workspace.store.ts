import { computed, effect, inject, untracked } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withState } from '@ngrx/signals';
import { FieldFilter, Level, MarkerKind, Query, RecordView, Timeline, WorkspaceSummary } from './api.types';
import { ImtBackend } from './backend';
import { TimeZoneMode } from './format';
import { Logger } from './logger';
import { Platform } from './platform';

export type EditorTab = 'timeline' | 'metrics' | 'profile';
export type ThemeMode = 'system' | 'dark' | 'light';

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

interface WorkspaceState {
  summary: WorkspaceSummary | null;
  /** Bumped whenever the underlying data changes (open, remove, reload, profile save). */
  dataVersion: number;
  query: Query;
  /** Raw search box text; copied into query.text after a debounce. */
  textInput: string;
  selected: RecordView | null;
  timeline: Timeline | null;
  timelineLoading: boolean;
  buckets: number;
  matchTotal: number | null;
  /** Request for the grid to scroll to (and select) a record. */
  anchor: { id: number; nonce: number } | null;
  anchorMissed: boolean;
  timezone: TimeZoneMode;
  alwaysShowLifecycle: boolean;
  activeTab: EditorTab;
  sidebarOpen: boolean;
  inspectorOpen: boolean;
  theme: ThemeMode;
  descending: boolean;
  extraColumns: string[];
  timelineHeight: number;
  busy: string | null;
  error: string | null;
  dropHover: boolean;
  focusSearchNonce: number;
}

const PREFS_KEY = 'imt.prefs.v1';
type Prefs = Pick<WorkspaceState, 'timezone' | 'theme' | 'timelineHeight' | 'extraColumns' | 'alwaysShowLifecycle'>;

export function loadPrefs(): Partial<Prefs> {
  try {
    return JSON.parse(localStorage.getItem(PREFS_KEY) ?? '{}') as Partial<Prefs>;
  } catch {
    return {};
  }
}

export function savePrefs(p: Prefs): void {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(p));
  } catch {
    /* storage unavailable: preferences are a convenience only */
  }
}

const initialState: WorkspaceState = {
  summary: null,
  dataVersion: 0,
  query: EMPTY_QUERY,
  textInput: '',
  selected: null,
  timeline: null,
  timelineLoading: false,
  buckets: 400,
  matchTotal: null,
  anchor: null,
  anchorMissed: false,
  timezone: 'utc',
  alwaysShowLifecycle: true,
  activeTab: 'timeline',
  sidebarOpen: true,
  inspectorOpen: true,
  theme: 'system',
  descending: false,
  extraColumns: [],
  timelineHeight: 300,
  busy: null,
  error: null,
  dropHover: false,
  focusSearchNonce: 0,
  ...loadPrefs(),
};

function errorText(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}

export const WorkspaceStore = signalStore(
  { providedIn: 'root' },
  withState<WorkspaceState>(initialState),
  withComputed((store) => {
    const effectiveGroupBy = computed(() => {
      const q = store.query();
      return q.groupBy.length ? q.groupBy : (store.summary()?.defaultGroupBy ?? []);
    });
    return {
      effectiveGroupBy,
      effectiveQuery: computed<Query>(() => ({ ...store.query(), groupBy: effectiveGroupBy() })),
      markerKindMap: computed(() => new Map<string, MarkerKind>((store.summary()?.markerKinds ?? []).map((k) => [k.kind, k]))),
      fieldNames: computed(() => (store.summary()?.fields ?? []).map((f) => f.name)),
      numericFields: computed(() => (store.summary()?.fields ?? []).filter((f) => f.numeric).map((f) => f.name)),
      hasRange: computed(() => store.query().from != null || store.query().to != null),
      markedOnly: computed(() => store.query().markers.length > 0),
      instanceCount: computed(() => store.timeline()?.lanes.length ?? 0),
    };
  }),
  withMethods((store, backend = inject(ImtBackend), platform = inject(Platform), logger = inject(Logger)) => {
    let textTimer: ReturnType<typeof setTimeout> | undefined;
    let anchorNonce = 0;

    const setQuery = (patch: Partial<Query>) => patchState(store, { query: { ...store.query(), ...patch } });

    const applySummary = (summary: WorkspaceSummary) =>
      patchState(store, (s) => ({ summary, dataVersion: s.dataVersion + 1, selected: null, busy: null, error: null }));

    /** Runs a workspace command; logs counts only (never names, paths or content). */
    const run = async (label: string, op: () => Promise<WorkspaceSummary>, operation: string) => {
      patchState(store, { busy: label, error: null });
      try {
        const summary = await op();
        applySummary(summary);
        logger.info('ui.workspace', `Workspace ${operation}`, {
          Operation: operation,
          Sources: summary.sources.length,
          Unrecognized: summary.unrecognized.length,
          Events: summary.total,
          LoadMs: summary.loadMs,
        });
      } catch (e) {
        patchState(store, { busy: null, error: errorText(e) });
      }
    };

    return {
      applySummary,
      async init() {
        await run('Loading workspace…', () => backend.workspaceSummary(), 'loaded');
      },
      openPaths: (paths: string[]) => run(`Opening ${paths.length} item${paths.length === 1 ? '' : 's'}…`, () => backend.openPaths(paths), 'opened'),
      async pickAndOpen(directory: boolean) {
        try {
          const paths = await platform.pickPaths(directory);
          if (paths.length) await run(`Opening ${paths.length} item${paths.length === 1 ? '' : 's'}…`, () => backend.openPaths(paths), 'opened');
        } catch (e) {
          patchState(store, { error: errorText(e) });
        }
      },
      removeSource: (id: number) => run('Removing source…', () => backend.removeSource(id), 'source removed'),
      clearWorkspace: () => run('Clearing workspace…', () => backend.clearWorkspace(), 'cleared'),
      reload: () => run('Re-importing…', () => backend.reloadWorkspace(), 'reloaded'),
      dismissError: () => patchState(store, { error: null }),
      setError: (error: string | null) => patchState(store, { error }),
      setDropHover: (dropHover: boolean) => patchState(store, { dropHover }),

      setTextInput(text: string) {
        patchState(store, { textInput: text });
        clearTimeout(textTimer);
        textTimer = setTimeout(() => setQuery({ text }), 250);
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
        setQuery({ markers: on ? (store.summary()?.markerKinds ?? []).map((k) => k.kind) : [] });
      },
      clearQuery() {
        clearTimeout(textTimer);
        patchState(store, { query: EMPTY_QUERY, textInput: '' });
      },

      select: (selected: RecordView | null) => patchState(store, { selected, anchorMissed: false }),
      jumpTo(id: number) {
        patchState(store, { anchor: { id, nonce: ++anchorNonce }, anchorMissed: false });
        if (store.activeTab() !== 'timeline') patchState(store, { activeTab: 'timeline' });
      },
      anchorResolved(row: RecordView | null) {
        patchState(store, row ? { selected: row, anchorMissed: false } : { anchorMissed: true });
      },
      setMatchTotal: (matchTotal: number | null) => patchState(store, { matchTotal }),
      setBuckets(buckets: number) {
        if (Math.abs(buckets - store.buckets()) > 4) patchState(store, { buckets });
      },

      setTimezone: (timezone: TimeZoneMode) => patchState(store, { timezone }),
      setAlwaysShowLifecycle: (alwaysShowLifecycle: boolean) => patchState(store, { alwaysShowLifecycle }),
      setTab: (activeTab: EditorTab) => patchState(store, { activeTab }),
      toggleSidebar: () => patchState(store, (s) => ({ sidebarOpen: !s.sidebarOpen })),
      toggleInspector: () => patchState(store, (s) => ({ inspectorOpen: !s.inspectorOpen })),
      setTheme: (theme: ThemeMode) => patchState(store, { theme }),
      toggleSort: () => patchState(store, (s) => ({ descending: !s.descending })),
      addColumn(field: string) {
        if (!store.extraColumns().includes(field)) patchState(store, { extraColumns: [...store.extraColumns(), field] });
      },
      removeColumn: (field: string) => patchState(store, { extraColumns: store.extraColumns().filter((c) => c !== field) }),
      setTimelineHeight: (h: number) => patchState(store, { timelineHeight: Math.round(h) }),
      focusSearch: () => patchState(store, (s) => ({ focusSearchNonce: s.focusSearchNonce + 1, activeTab: 'timeline' as EditorTab })),
    };
  }),
  withHooks({
    onInit(store) {
      const backend = inject(ImtBackend);
      let seq = 0;

      effect(() => {
        const summary = store.summary();
        const request = { query: store.effectiveQuery(), buckets: store.buckets(), alwaysShowLifecycle: store.alwaysShowLifecycle() };
        store.dataVersion();
        if (!summary) return;
        const mine = ++seq;
        untracked(() => patchState(store, { timelineLoading: true }));
        backend
          .queryTimeline(request)
          .then((timeline) => mine === seq && patchState(store, { timeline, timelineLoading: false }))
          .catch((e) => mine === seq && patchState(store, { timelineLoading: false, error: errorText(e) }));
      });

      effect(() => {
        const theme = store.theme();
        const root = document.documentElement;
        if (theme === 'system') root.removeAttribute('data-theme');
        else root.setAttribute('data-theme', theme);
      });

      effect(() =>
        savePrefs({
          timezone: store.timezone(),
          theme: store.theme(),
          timelineHeight: store.timelineHeight(),
          extraColumns: store.extraColumns(),
          alwaysShowLifecycle: store.alwaysShowLifecycle(),
        }),
      );

      void inject(Platform).onFilesDropped(
        (paths) => void store.openPaths(paths),
        (over) => store.setDropHover(over),
      );
      void store.init();
    },
  }),
);

export type WorkspaceStoreType = InstanceType<typeof WorkspaceStore>;
