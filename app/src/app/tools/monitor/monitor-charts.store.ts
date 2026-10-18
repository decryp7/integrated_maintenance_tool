import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { ChartData } from '../../core/api.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { MONITOR_CHARTS } from './monitor-charts-backend';
import { assignSlots, MONITOR_GROUPS, hasMonitorSamples, MONITOR_BUCKETS, MONITOR_METRICS, monitorChartDef, monitorQuery, monitorSourceIds, MonitorWindow } from './monitor-charts-model';
import { MonitorStore } from './monitor.store';
import { errorMessage } from '../tool-errors';

/** Palette slots that get their own colour; further processes are drawn as neutral "Other" lines. */
export const MONITOR_SLOTS = 4;

interface MonitorChartsState {
  windowMin: MonitorWindow;
  /** Follow new samples as they are written; false = paused (the charts keep what they show). */
  following: boolean;
  /** Series keys switched off in the shared legend. */
  hidden: string[];
  /** Engine results, aligned with MONITOR_METRICS; empty until the first load. */
  charts: ChartData[];
  /** Palette slot per series key, kept while the panel shows the same sources so colours follow the process. */
  slots: Record<string, number>;
  /** The source ids the slots were assigned for: another run starts with fresh slots. */
  slotScope: string;
  loading: boolean;
  /** Bumped by Retry, so the panel loads again even when nothing else changed. */
  attempt: number;
  error: string | null;
}

/** State of the process monitor's live charts. The engine aggregates; this store only asks and keeps the answers. */
export const MonitorChartsStore = signalStore(
  withState<MonitorChartsState>({ windowMin: 5, following: true, hidden: [], charts: [], slots: {}, slotScope: '', loading: false, attempt: 0, error: null }),
  withComputed((store, workspace = inject(WorkspaceStore), monitor = inject(MonitorStore)) => {
    const sources = computed(() => monitorSourceIds(workspace.summary(), monitor.dir()), { equal: (a, b) => a.join(',') === b.join(',') });
    return {
      /** Shown while recording and whenever the workspace holds monitor samples. */
      visible: computed(() => monitor.running() || sources().length > 0 || hasMonitorSamples(workspace.summary())),
      sources,
      /** No chart has a series: nothing sampled in the window yet. */
      empty: computed(() => store.charts().length > 0 && store.charts().every((c) => c.series.length === 0)),
      /** The shared legend: every process seen, in slot order, then "Other" when more processes than slots exist. */
      legend: computed(() => {
        const slots = store.slots();
        const keys = new Set(store.charts().flatMap((c) => c.series.map((s) => s.key)));
        const named = Object.entries(slots)
          .filter(([k]) => keys.has(k))
          .sort((a, b) => a[1] - b[1])
          .map(([key, slot]) => ({ key, slot: slot as number | null }));
        const others = [...keys].some((k) => !(k in slots));
        return others ? [...named, { key: 'Other', slot: null }] : named;
      }),
      hiddenSet: computed(() => new Set(store.hidden())),
      /** The chart groups to show, each with its metrics and their results; a group without any samples is left out. */
      groups: computed(() => {
        const data = store.charts();
        return MONITOR_GROUPS.map((g) => ({
          ...g,
          metrics: MONITOR_METRICS.map((metric, index) => ({ metric, index, data: data[index] ?? null })).filter((m) => m.metric.group === g.id),
        })).filter((g) => g.id === 'process' || g.metrics.some((m) => !!m.data?.series.length));
      }),
      /** The newest bucket's end the engine resolved for the window (null before data). */
      until: computed(() => store.charts()[0]?.to ?? null),
    };
  }),
  withMethods((store, charts = inject(MONITOR_CHARTS)) => {
    let seq = 0;
    return {
      setWindow: (windowMin: MonitorWindow) => patchState(store, { windowMin }),
      retry: () => patchState(store, (s) => ({ attempt: s.attempt + 1 })),
      pause: () => patchState(store, { following: false }),
      follow: () => patchState(store, { following: true }),
      toggle: (key: string) => patchState(store, (s) => ({ hidden: s.hidden.includes(key) ? s.hidden.filter((k) => k !== key) : [...s.hidden, key] })),

      /** Asks the engine for every small multiple; a newer load wins over a slower older one. */
      async load(): Promise<void> {
        const mine = ++seq;
        const query = monitorQuery(store.sources(), store.windowMin());
        const scope = (query.sources ?? []).join(',');
        patchState(store, { loading: true });
        try {
          const data = await Promise.all(MONITOR_METRICS.map((m) => charts.queryChart({ query, def: monitorChartDef(m), chartId: null, buckets: MONITOR_BUCKETS })));
          if (mine !== seq) return;
          patchState(store, (s) => ({ charts: data, slots: assignSlots(s.slotScope === scope ? s.slots : {}, data, MONITOR_SLOTS), slotScope: scope, error: null }));
        } catch (e) {
          if (mine === seq) patchState(store, { error: errorMessage(e) });
        } finally {
          if (mine === seq) patchState(store, { loading: false });
        }
      },
    };
  }),
);
