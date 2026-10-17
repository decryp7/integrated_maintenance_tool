import { inject } from '@angular/core';
import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { HotspotRow, HotspotsRequest, Measurement, MetricId, MetricValues, ProfileGroupBy, ProfileSortSpec } from '../../core/api.types';
import { errorText } from '../../analysis/async-resource';
import { FrameNames } from '../captures.store';
import { ProfilingBackend } from '../profiling-backend';
import { sameSort } from './hotspot-columns';

/** Rows per engine request. */
export const HOTSPOT_PAGE = 200;

/** What the hotspots table asks for, apart from paging. */
export type HotspotsQuery = Omit<HotspotsRequest, 'offset' | 'limit'>;

interface Meta {
  totalRows: number;
  grandTotal: MetricValues;
  metric: MetricId;
  measurement: Measurement;
  samplesInView: number;
}

interface HotspotsShape {
  groupBy: ProfileGroupBy;
  /** null = the engine default (self of the metric, descending). */
  sort: ProfileSortSpec | null;
  search: string;
  pages: Record<number, HotspotRow[]>;
  meta: Meta | null;
  loading: boolean;
  error: string | null;
  /** Row index with keyboard focus/selection. */
  selected: number | null;
}

/** Paged hotspots from the engine: pages load as the table scrolls; any query change starts over. */
export const HotspotsStore = signalStore(
  withState<HotspotsShape>({ groupBy: 'function', sort: null, search: '', pages: {}, meta: null, loading: false, error: null, selected: null }),
  withMethods((s) => {
    const backend = inject(ProfilingBackend);
    const names = inject(FrameNames);
    let query: HotspotsQuery | null = null;
    let key = '';
    let generation = 0;
    const inflight = new Set<number>();

    const fetchPage = async (page: number): Promise<void> => {
      if (!query || s.pages()[page] || inflight.has(page)) return;
      const mine = generation;
      const q = query;
      inflight.add(page);
      patchState(s, { loading: true });
      try {
        const r = await backend.hotspots({ ...q, offset: page * HOTSPOT_PAGE, limit: HOTSPOT_PAGE });
        if (mine !== generation) return;
        names.remember(q.profileId, r.rows.map((x) => x.frame));
        patchState(s, {
          pages: { ...s.pages(), [page]: r.rows },
          meta: { totalRows: r.totalRows, grandTotal: r.grandTotal, metric: r.metric, measurement: r.measurement, samplesInView: r.samplesInView },
          error: null,
        });
      } catch (e) {
        if (mine === generation) patchState(s, { error: errorText(e) });
      } finally {
        if (mine === generation) {
          inflight.delete(page);
          patchState(s, { loading: inflight.size > 0 });
        }
      }
    };

    const restart = () => {
      generation++;
      inflight.clear();
      patchState(s, { pages: {}, error: null, selected: null, loading: false });
      return fetchPage(0);
    };

    return {
      /** Sets the query (profile, view, metric, …); a different one drops every page. null clears. */
      setQuery(q: HotspotsQuery | null): Promise<void> {
        const k = JSON.stringify(q);
        if (k === key) return Promise.resolve();
        key = k;
        query = q;
        if (!q) {
          generation++;
          inflight.clear();
          patchState(s, { pages: {}, meta: null, error: null, selected: null, loading: false });
          return Promise.resolve();
        }
        return restart();
      },
      /** Loads the pages covering rows `first..last` (inclusive). */
      async ensure(first: number, last: number): Promise<void> {
        const total = s.meta()?.totalRows ?? 0;
        if (!total) return;
        const pages: number[] = [];
        for (let p = Math.floor(Math.max(0, first) / HOTSPOT_PAGE); p <= Math.floor(Math.min(total - 1, last) / HOTSPOT_PAGE); p++) pages.push(p);
        await Promise.all(pages.map(fetchPage));
      },
      row(index: number): HotspotRow | null {
        return s.pages()[Math.floor(index / HOTSPOT_PAGE)]?.[index % HOTSPOT_PAGE] ?? null;
      },
      retry: () => restart(),
      setGroupBy: (groupBy: ProfileGroupBy) => patchState(s, { groupBy }),
      setSearch: (search: string) => patchState(s, { search }),
      /** Sorting by the same column again flips the direction. */
      sortBy(spec: ProfileSortSpec): void {
        const cur = s.sort();
        // A stored sort always has an explicit direction (set below).
        if (cur && sameSort(cur, spec)) patchState(s, { sort: { ...cur, descending: !cur.descending } });
        else patchState(s, { sort: { ...spec, descending: spec.metric !== null } });
      },
      select: (selected: number | null) => patchState(s, { selected }),
    };
  }),
);

export type HotspotsStoreType = InstanceType<typeof HotspotsStore>;
