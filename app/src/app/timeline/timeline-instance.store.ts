import { computed, effect, inject, untracked } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withProps, withState } from '@ngrx/signals';
import { Timeline } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { WorkspaceStore } from '../core/workspace.store';
import { TimeCursor } from '../analysis/time-cursor';
import { DOCK_PANEL_ID } from '../dock/angular-panel';
import { instanceQuery, LaneMode, laneGroupBy, MAIN_TIMELINE, TimelineConfig, TimelineScope } from './timeline-config';
import { TimelineSource } from './timeline-source';
import { TimelineViews } from './timeline-views';
import { Range, RangeZoom, summaryExtent, TimelineZoom } from './zoom';

interface InstanceState {
  timeline: Timeline | null;
  loading: boolean;
  error: string | null;
  /** Bumped to fetch again after a failure. */
  retryNonce: number;
  buckets: number;
  /** The panel's own range while not linked to the main timeline (null = everything). */
  range: Range | null;
}

const initialState: InstanceState = { timeline: null, loading: false, error: null, retryNonce: 0, buckets: 400, range: null };

/** Structural equality for request DTOs, so unrelated workspace changes do not refetch. */
const sameJson = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

function errorText(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}

/**
 * The state of one additional timeline panel, provided by the panel component (one instance per panel). Its
 * settings (`TimelineConfig`) live in `TimelineViews` so they persist; the timeline itself is computed by the
 * engine (`queryTimeline`) for the query `instanceQuery` composes. While linked ("Sync time") it zooms and pans
 * through the workspace's `TimelineZoom` and shares the time cursor; unlinked it keeps its own range and cursor.
 */
export const TimelineInstanceStore = signalStore(
  withState<InstanceState>(initialState),
  withProps(() => ({
    id: inject(DOCK_PANEL_ID, { optional: true }) ?? `${MAIN_TIMELINE}#2`,
    main: false as const,
    _views: inject(TimelineViews),
    _workspace: inject(WorkspaceStore),
    _backend: inject(ImtBackend),
    _sharedZoom: inject(TimelineZoom),
    _sharedCursor: inject(TimeCursor),
    _localCursor: new TimeCursor(),
  })),
  withComputed((store) => {
    const config = computed<TimelineConfig>(() => store._views.config(store.id));
    const synced = computed(() => config().sync);
    return {
      config,
      synced,
      title: computed(() => config().title),
      groupBy: computed(() => laneGroupBy(config(), store._workspace.effectiveGroupBy())),
      /** The query sent to the engine. */
      request: computed(() => instanceQuery(store._workspace.query(), store._workspace.effectiveGroupBy(), config(), store.range()), { equal: sameJson }),
      hasRange: computed(() => (synced() ? store._workspace.hasRange() : store.range() !== null)),
      cursor: computed(() => (synced() ? store._sharedCursor : store._localCursor)),
    };
  }),
  withComputed((store) => ({
    markers: computed(() => store.request().markers),
    groups: computed(() => store.request().groups),
  })),
  withProps((store) => {
    const extent = computed(() => summaryExtent(store._workspace.summary()));
    const own = computed(() => store.range() !== null);
    return {
      _localZoom: new RangeZoom({
        extent,
        view: computed(() => {
          const ext = extent();
          return ext ? (store.range() ?? ext) : null;
        }),
        hasRange: own,
        canPan: own,
        setRange: (range) => patchState(store, { range }),
      }),
    };
  }),
  withComputed((store) => ({
    zoom: computed<RangeZoom>(() => (store.synced() ? store._sharedZoom : store._localZoom)),
  })),
  withMethods((store) => {
    const update = (patch: Partial<Omit<TimelineConfig, 'id'>>) => store._views.update(store.id, patch);
    return {
      retry: () => patchState(store, (s) => ({ retryNonce: s.retryNonce + 1 })),
      setBuckets(buckets: number) {
        if (Math.abs(buckets - store.buckets()) > 4) patchState(store, { buckets });
      },
      showOnlyMarker: (kind: string | null) => update({ markers: kind ? [kind] : null }),
      toggleGroup(key: string) {
        const groups = store.groups();
        update({ groups: groups.includes(key) ? groups.filter((g) => g !== key) : [...groups, key] });
      },
      rename: (title: string) => store._views.rename(store.id, title),
      setScope: (scope: TimelineScope) => update({ scope, groups: [] }),
      /** The query-language expression of an `own` scope (validated by the engine; errors show inline). */
      setQl: (ql: string) => update({ ql }),
      setLanes: (lanes: LaneMode, laneField = store.config().laneField) => update({ lanes, laneField, groups: [] }),
      /**
       * Links or unlinks zoom, pan and the time cursor with the main timeline. Unlinking starts from what the main
       * timeline shows, so both stay lined up until one of them moves.
       */
      setSync(sync: boolean) {
        if (sync === store.synced()) return;
        if (!sync) {
          const shared = store._sharedZoom;
          patchState(store, { range: shared.isZoomed() ? shared.view() : null });
        } else {
          store._localZoom.cancelPending();
          patchState(store, { range: null });
        }
        update({ sync });
      },
    };
  }),
  withHooks({
    onInit(store) {
      let seq = 0;
      effect(() => {
        const summary = store._workspace.summary();
        const request = { query: store.request(), buckets: store.buckets(), alwaysShowLifecycle: store._workspace.alwaysShowLifecycle() };
        store._workspace.dataVersion();
        store.retryNonce();
        if (!summary) return;
        const mine = ++seq;
        untracked(() => patchState(store, { loading: true }));
        store._backend
          .queryTimeline(request)
          .then((timeline) => mine === seq && patchState(store, { timeline, loading: false, error: null }))
          .catch((e: unknown) => mine === seq && patchState(store, { loading: false, error: errorText(e) }));
      });
    },
    onDestroy(store) {
      store._localZoom.cancelPending();
    },
  }),
);

export type TimelineInstanceStoreType = InstanceType<typeof TimelineInstanceStore>;

/** Compile-time check that the store is a `TimelineSource`. */
export const asTimelineSource = (store: TimelineInstanceStoreType): TimelineSource => store;
