import { computed, effect, inject, untracked } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withState } from '@ngrx/signals';
import { FieldFilter, Level, MarkerKind, Query, RecordView, Timeline, WorkspaceSummary } from './api.types';
import { ImtBackend } from './backend';
import { DROPPED_FILES_HANDLERS, routeDroppedFiles } from './dropped-files';
import { TimeZoneMode } from './format';
import { Logger } from './logger';
import { Platform } from './platform';
import { RECENT_STORE } from '../welcome/recent-store';

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
  /** Bumped whenever the profile is saved and applied. */
  profileVersion: number;
  /** Bumped after files or folders were opened successfully (the overview opens after an import). */
  importVersion: number;
  query: Query;
  /** Raw search box text; copied into query.text after a debounce. */
  textInput: string;
  selected: RecordView | null;
  timeline: Timeline | null;
  timelineLoading: boolean;
  /** Why the last timeline request failed (shown inline in the timeline panel with Retry). */
  timelineError: string | null;
  /** Bumped to fetch the timeline again after a failure. */
  timelineRetry: number;
  buckets: number;
  matchTotal: number | null;
  /** Request for the grid to scroll to (and select) a record. */
  anchor: { id: number; nonce: number } | null;
  anchorMissed: boolean;
  timezone: TimeZoneMode;
  alwaysShowLifecycle: boolean;
  sidebarOpen: boolean;
  theme: ThemeMode;
  descending: boolean;
  extraColumns: string[];
  busy: string | null;
  error: string | null;
  /** Transient informational message (e.g. "Exported 120 events"). */
  notice: string | null;
  dropHover: boolean;
  focusSearchNonce: number;
}

const PREFS_KEY = 'imt.prefs.v1';
type Prefs = Pick<WorkspaceState, 'timezone' | 'theme' | 'extraColumns' | 'alwaysShowLifecycle'>;

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
  profileVersion: 0,
  importVersion: 0,
  query: EMPTY_QUERY,
  textInput: '',
  selected: null,
  timeline: null,
  timelineLoading: false,
  timelineError: null,
  timelineRetry: 0,
  buckets: 400,
  matchTotal: null,
  anchor: null,
  anchorMissed: false,
  timezone: 'utc',
  alwaysShowLifecycle: true,
  sidebarOpen: true,
  theme: 'system',
  descending: false,
  extraColumns: [],
  busy: null,
  error: null,
  notice: null,
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
  withMethods((store, backend = inject(ImtBackend), platform = inject(Platform), logger = inject(Logger), recent = inject(RECENT_STORE)) => {
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
      /** Applies the summary returned by a profile save and announces it to listeners. */
      profileSaved(summary: WorkspaceSummary) {
        applySummary(summary);
        patchState(store, (s) => ({ profileVersion: s.profileVersion + 1 }));
      },
      patchQuery: (patch: Partial<Query>) => setQuery(patch),
      async init() {
        await run('Loading workspace…', () => backend.workspaceSummary(), 'loaded');
      },
      /**
       * Fetches the summary again after the engine changed the data underneath (live tail). Unlike
       * `init`, it keeps the selection, the query and filters, and shows no busy state; the timeline
       * and the grid reload because the data version changes. Rejects when the backend fails.
       */
      async refreshSummary(): Promise<void> {
        const summary = await backend.workspaceSummary();
        patchState(store, (s) => ({ summary, dataVersion: s.dataVersion + 1 }));
      },
      openPaths: async (paths: string[]): Promise<void> => {
        await run(`Opening ${paths.length} item${paths.length === 1 ? '' : 's'}…`, () => backend.openPaths(paths), 'opened');
        if (!store.error()) {
          recent.add(paths);
          patchState(store, (s) => ({ importVersion: s.importVersion + 1 }));
        }
      },
      async pickAndOpen(directory: boolean) {
        try {
          const paths = await platform.pickPaths(directory);
          if (paths.length) {
            await run(`Opening ${paths.length} item${paths.length === 1 ? '' : 's'}…`, () => backend.openPaths(paths), 'opened');
            if (!store.error()) {
              recent.add(paths);
              patchState(store, (s) => ({ importVersion: s.importVersion + 1 }));
            }
          }
        } catch (e) {
          patchState(store, { error: errorText(e) });
        }
      },
      removeSource: (id: number) => run('Removing source…', () => backend.removeSource(id), 'source removed'),
      clearWorkspace: () => run('Clearing workspace…', () => backend.clearWorkspace(), 'cleared'),
      reload: () => run('Re-importing…', () => backend.reloadWorkspace(), 'reloaded'),
      dismissError: () => patchState(store, { error: null }),
      setError: (error: string | null) => patchState(store, { error }),
      setNotice: (notice: string | null) => patchState(store, { notice }),
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
      /** Puts back a whole query, search box included (undo/redo of filter changes). */
      restoreQuery(query: Query) {
        clearTimeout(textTimer);
        patchState(store, { query, textInput: query.text });
      },

      select: (selected: RecordView | null) => patchState(store, { selected, anchorMissed: false }),
      jumpTo(id: number) {
        patchState(store, { anchor: { id, nonce: ++anchorNonce }, anchorMissed: false });
      },
      anchorResolved(row: RecordView | null) {
        patchState(store, row ? { selected: row, anchorMissed: false } : { anchorMissed: true });
      },
      setMatchTotal: (matchTotal: number | null) => patchState(store, { matchTotal }),
      retryTimeline: () => patchState(store, (s) => ({ timelineRetry: s.timelineRetry + 1 })),
      setBuckets(buckets: number) {
        if (Math.abs(buckets - store.buckets()) > 4) patchState(store, { buckets });
      },

      setTimezone: (timezone: TimeZoneMode) => patchState(store, { timezone }),
      setAlwaysShowLifecycle: (alwaysShowLifecycle: boolean) => patchState(store, { alwaysShowLifecycle }),
      toggleSidebar: () => patchState(store, (s) => ({ sidebarOpen: !s.sidebarOpen })),
      setTheme: (theme: ThemeMode) => patchState(store, { theme }),
      toggleSort: () => patchState(store, (s) => ({ descending: !s.descending })),
      addColumn(field: string) {
        if (!store.extraColumns().includes(field)) patchState(store, { extraColumns: [...store.extraColumns(), field] });
      },
      removeColumn: (field: string) => patchState(store, { extraColumns: store.extraColumns().filter((c) => c !== field) }),
      focusSearch: () => patchState(store, (s) => ({ focusSearchNonce: s.focusSearchNonce + 1 })),
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
        store.timelineRetry();
        if (!summary) return;
        const mine = ++seq;
        untracked(() => patchState(store, { timelineLoading: true }));
        backend
          .queryTimeline(request)
          .then((timeline) => mine === seq && patchState(store, { timeline, timelineLoading: false, timelineError: null }))
          .catch((e) => mine === seq && patchState(store, { timelineLoading: false, timelineError: errorText(e) }));
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
          extraColumns: store.extraColumns(),
          alwaysShowLifecycle: store.alwaysShowLifecycle(),
        }),
      );

      // Features claim their files first (profiling: captures); the rest open as logs, once.
      const dropHandlers = inject(DROPPED_FILES_HANDLERS, { optional: true }) ?? [];
      void inject(Platform).onFilesDropped(
        (paths) => void routeDroppedFiles(dropHandlers, paths).then((rest) => (rest.length ? store.openPaths(rest) : undefined)),
        (over) => store.setDropHover(over),
      );
      void store.init();
    },
  }),
);

export type WorkspaceStoreType = InstanceType<typeof WorkspaceStore>;
