import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { FrameDto, HotspotRow, HotspotsPage, HotspotsRequest } from '../../core/api.types';
import { FrameNames } from '../captures.store';
import { ProfilingBackend } from '../profiling-backend';
import { openDemoCaptures, setupProfiling } from '../testing/profiling-test-support';
import { HOTSPOT_PAGE, HotspotsQuery, HotspotsStore } from './hotspots.store';
import { at } from '../../../testing/must';

const frame = (id: number): FrameDto => ({ id, name: `F${id}`, module: null, file: null, line: null, kind: 'managed', isFramework: false });
const row = (i: number): HotspotRow => ({
  key: `k${i}`, name: `F${i}`, module: null, frame: frame(i), thread: null, self: { timeNs: i }, total: { timeNs: i }, selfPct: 0, totalPct: 0, avgPerCallNs: null, maxPerCallNs: null,
});
const query = (patch: Partial<HotspotsQuery> = {}): HotspotsQuery => ({ profileId: 1, metric: 'timeNs', ...patch });

/** A backend whose hotspots() is served from `total` synthetic rows and can be held back per call. */
function fake(total = 650) {
  const calls: HotspotsRequest[] = [];
  const hold: Array<{ resolve: () => void; reject: (e: unknown) => void }> = [];
  let holding = false;
  const page = (r: HotspotsRequest): HotspotsPage => {
    const offset = r.offset ?? 0;
    const n = Math.max(0, Math.min(r.limit ?? HOTSPOT_PAGE, total - offset));
    return { rows: Array.from({ length: n }, (_, i) => row(offset + i)), totalRows: total, grandTotal: { timeNs: 100 }, metric: 'timeNs', measurement: 'sampled', samplesInView: 7 };
  };
  const backend = {
    hotspots: vi.fn((r: HotspotsRequest) => {
      calls.push(r);
      if (!holding) return Promise.resolve(page(r));
      return new Promise<HotspotsPage>((resolve, reject) => hold.push({ resolve: () => resolve(page(r)), reject }));
    }),
  } as unknown as ProfilingBackend;
  return { backend, calls, hold, holdBack: () => (holding = true) };
}

function setup(f = fake()) {
  setupProfiling({ backend: f.backend });
  TestBed.configureTestingModule({ providers: [HotspotsStore] });
  return { f, store: TestBed.inject(HotspotsStore) };
}

describe('HotspotsStore', () => {
  it('loads the first page for a new query and exposes meta and rows', async () => {
    const { f, store } = setup();
    await store.setQuery(query());
    expect(f.calls).toEqual([{ profileId: 1, metric: 'timeNs', offset: 0, limit: HOTSPOT_PAGE }]);
    expect(store.meta()).toEqual({ totalRows: 650, grandTotal: { timeNs: 100 }, metric: 'timeNs', measurement: 'sampled', samplesInView: 7 });
    expect(store.row(0)?.name).toBe('F0');
    expect(store.row(199)?.name).toBe('F199');
    expect(store.row(200)).toBeNull();
    expect(store.loading()).toBe(false);
    expect(TestBed.inject(FrameNames).get(1, 5)?.name).toBe('F5');
  });

  it('an identical query is a no-op; a different one drops every page', async () => {
    const { f, store } = setup();
    await store.setQuery(query());
    await store.setQuery(query());
    expect(f.calls).toHaveLength(1);
    await store.ensure(0, 450);
    expect(f.calls).toHaveLength(3);
    await store.setQuery(query({ search: 'x' }));
    expect(f.calls).toHaveLength(4);
    expect(store.row(450)).toBeNull();
    expect(store.row(0)).not.toBeNull();
  });

  it('null clears pages and meta, and is idempotent', async () => {
    const { f, store } = setup();
    await store.setQuery(query());
    store.select(3);
    await store.setQuery(null);
    expect(store.meta()).toBeNull();
    expect(store.row(0)).toBeNull();
    expect(store.selected()).toBeNull();
    await store.setQuery(null);
    expect(f.calls).toHaveLength(1);
    await store.ensure(0, 10);
    expect(f.calls).toHaveLength(1);
  });

  it('ensure loads only the pages covering the requested rows', async () => {
    const { f, store } = setup();
    await store.setQuery(query());
    await store.ensure(450, 500);
    expect(f.calls.map((c) => c.offset)).toEqual([0, 400]);
    await store.ensure(150, 420);
    expect(f.calls.map((c) => c.offset)).toEqual([0, 400, 200]);
    expect(store.row(449)?.name).toBe('F449');
    expect(store.row(649)).toBeNull();
    // Beyond the total is clamped; negative start is clamped to the first page.
    await store.ensure(-5, 100_000);
    expect(f.calls.map((c) => c.offset)).toEqual([0, 400, 200, 600]);
    expect(store.row(649)?.name).toBe('F649');
  });

  it('ensure does nothing before the first answer or for an empty result', async () => {
    const { f, store } = setup(fake(0));
    await store.ensure(0, 10);
    expect(f.calls).toHaveLength(0);
    await store.setQuery(query());
    await store.ensure(0, 10);
    expect(f.calls).toHaveLength(1);
    expect(store.row(0)).toBeNull();
  });

  it('de-duplicates in-flight page requests', async () => {
    const f = fake();
    const { store } = setup(f);
    await store.setQuery(query());
    f.holdBack();
    const a = store.ensure(200, 250);
    const b = store.ensure(200, 250);
    expect(store.loading()).toBe(true);
    expect(f.calls.filter((c) => c.offset === 200)).toHaveLength(1);
    at(f.hold, 0).resolve();
    await Promise.all([a, b]);
    expect(store.loading()).toBe(false);
    expect(store.row(200)?.name).toBe('F200');
  });

  it('drops answers (and failures) from an older query', async () => {
    const f = fake();
    const { store } = setup(f);
    f.holdBack();
    const first = store.setQuery(query());
    const second = store.setQuery(query({ search: 'new' }));
    expect(f.hold).toHaveLength(2);
    at(f.hold, 1).resolve();
    await second;
    at(f.hold, 0).reject('old failure');
    await first;
    expect(store.error()).toBeNull();
    expect(store.row(0)?.name).toBe('F0');
    expect(store.loading()).toBe(false);

    const third = store.setQuery(query({ search: 'third' }));
    at(f.hold, 2).resolve();
    await third;
    // A stale success arriving late must not overwrite the page.
    const stale = store.ensure(200, 200);
    const fresh = store.setQuery(query({ search: 'fourth' }));
    at(f.hold, 4).resolve();
    await fresh;
    at(f.hold, 3).resolve();
    await stale;
    expect(store.row(200)).toBeNull();
  });

  it('shows a failure and retry reloads from the first page', async () => {
    const { store } = (() => {
      setupProfiling({ options: { fail: { hotspots: 1 }, preopen: ['/demo/profiles/orders.speedscope.json'] } });
      TestBed.configureTestingModule({ providers: [HotspotsStore] });
      return { store: TestBed.inject(HotspotsStore) };
    })();
    await store.setQuery({ profileId: 1, metric: 'timeNs' });
    expect(store.error()).toContain('hotspots failed');
    expect(store.row(0)).toBeNull();
    expect(store.loading()).toBe(false);
    await store.retry();
    expect(store.error()).toBeNull();
    expect(store.row(0)).not.toBeNull();
    expect(store.meta()?.totalRows).toBeGreaterThan(0);
  });

  it('works against the mock engine', async () => {
    setupProfiling();
    TestBed.configureTestingModule({ providers: [HotspotsStore] });
    await openDemoCaptures();
    const store = TestBed.inject(HotspotsStore);
    await store.setQuery({ profileId: 1 });
    expect(store.meta()?.totalRows).toBeGreaterThan(5);
    expect(store.row(0)?.frame).not.toBeNull();
  });

  it('setGroupBy, setSearch and select update the view state', () => {
    const { store } = setup();
    store.setGroupBy('module');
    store.setSearch('json');
    store.select(4);
    expect(store.groupBy()).toBe('module');
    expect(store.search()).toBe('json');
    expect(store.selected()).toBe(4);
    store.select(null);
    expect(store.selected()).toBeNull();
  });

  it('sortBy defaults to descending for metrics and ascending for name, and toggles on the same column', () => {
    const { store } = setup();
    expect(store.sort()).toBeNull();
    store.sortBy({ metric: 'timeNs', of: 'self' });
    expect(store.sort()).toEqual({ metric: 'timeNs', of: 'self', descending: true });
    store.sortBy({ metric: 'timeNs', of: 'self' });
    expect(store.sort()?.descending).toBe(false);
    store.sortBy({ metric: 'timeNs', of: 'self' });
    expect(store.sort()?.descending).toBe(true);
    store.sortBy({ metric: 'timeNs', of: 'total' });
    expect(store.sort()).toEqual({ metric: 'timeNs', of: 'total', descending: true });
    store.sortBy({ metric: null });
    expect(store.sort()).toEqual({ metric: null, descending: false });
    store.sortBy({ metric: null });
    expect(store.sort()?.descending).toBe(true);
  });
});
