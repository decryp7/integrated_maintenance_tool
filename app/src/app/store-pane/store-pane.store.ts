import { inject } from '@angular/core';
import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { MaintenanceAction, SegmentsPage, StoreFields, StoreIndex, StoreMemory, StoreOverview } from '../core/api.types';
import { failureText } from '../core/cmd-error';
import { StorePaneBackend } from './store-pane-backend';

/** The Store pane's tabs. */
export type StoreTab = 'overview' | 'segments' | 'fields' | 'index' | 'memory' | 'sql';
export const STORE_TABS: readonly StoreTab[] = ['overview', 'segments', 'fields', 'index', 'memory', 'sql'];

/** Rows of a segments page. */
export const SEGMENT_PAGE = 100;

/** A report the pane loads, and how it went. */
export interface Section<T> {
  value: T | null;
  loading: boolean;
  error: string | null;
}

const empty = <T>(): Section<T> => ({ value: null, loading: false, error: null });

interface StorePaneState {
  tab: StoreTab;
  overview: Section<StoreOverview>;
  segments: Section<SegmentsPage>;
  fields: Section<StoreFields>;
  index: Section<StoreIndex>;
  memory: Section<StoreMemory>;
  /** The maintenance action being started. */
  starting: MaintenanceAction | null;
  /** What the last maintenance request said (its job started, or why it did not). */
  notice: { text: string; error: boolean } | null;
}

type Report = 'overview' | 'segments' | 'fields' | 'index' | 'memory';

const INITIAL: StorePaneState = {
  tab: 'overview',
  overview: empty<StoreOverview>(),
  segments: empty<SegmentsPage>(),
  fields: empty<StoreFields>(),
  index: empty<StoreIndex>(),
  memory: empty<StoreMemory>(),
  starting: null,
  notice: null,
};

/**
 * The Store pane's reports (`store_*`), each loaded on demand with its own loading and error state, and the
 * maintenance requests (`store_maintenance`; the pane confirms first). The engine computes every number.
 */
export const StorePaneStore = signalStore(
  { providedIn: 'root' },
  withState<StorePaneState>(INITIAL),
  withMethods((store, backend = inject(StorePaneBackend)) => {
    async function load<K extends Report>(key: K, fetch: () => Promise<NonNullable<StorePaneState[K]['value']>>): Promise<void> {
      patchState(store, { [key]: { ...store[key](), loading: true, error: null } } as Partial<StorePaneState>);
      try {
        const value = await fetch();
        patchState(store, { [key]: { value, loading: false, error: null } } as Partial<StorePaneState>);
      } catch (e) {
        patchState(store, { [key]: { ...store[key](), loading: false, error: failureText(e) } } as Partial<StorePaneState>);
      }
    }
    const loaders: Record<Report, () => Promise<void>> = {
      overview: () => load('overview', () => backend.overview()),
      segments: () => load('segments', () => backend.listSegments({ offset: store.segments().value?.offset ?? 0, limit: SEGMENT_PAGE })),
      fields: () => load('fields', () => backend.listFields()),
      index: () => load('index', () => backend.describeIndex()),
      memory: () => load('memory', () => backend.describeMemory()),
    };
    return {
      /** Shows a tab and loads its report (the SQL tab has its own store). */
      async show(tab: StoreTab): Promise<void> {
        patchState(store, { tab });
        if (tab !== 'sql') await loaders[tab]();
      },
      /** Loads the current tab's report again. */
      async refresh(): Promise<void> {
        const tab = store.tab();
        if (tab !== 'sql') await loaders[tab]();
      },
      /** Another page of segments. */
      segmentsAt(offset: number): Promise<void> {
        return load('segments', () => backend.listSegments({ offset: Math.max(0, offset), limit: SEGMENT_PAGE }));
      },
      /** Starts a maintenance job (after the user confirmed); the Jobs pane follows it. */
      async maintain(action: MaintenanceAction): Promise<void> {
        patchState(store, { starting: action, notice: null });
        try {
          await backend.maintain(action);
          patchState(store, { starting: null, notice: { text: $localize`:@@store.maintain.started:Started. The Jobs pane shows its progress and what it did.`, error: false } });
        } catch (e) {
          patchState(store, { starting: null, notice: { text: failureText(e), error: true } });
        }
      },
      dismissNotice: () => patchState(store, { notice: null }),
    };
  }),
);
