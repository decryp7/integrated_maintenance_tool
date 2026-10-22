import { Page, PageRequest, Query, RecordView } from '../core/api.types';
import { EMPTY_QUERY } from '../core/workspace/query.store';
import { liveMode, PagedRowsStore } from './paged-rows.store';

const row = (id: number) => ({ id }) as RecordView;

/** A backend of `total` rows whose answers the test releases. */
function fakeEngine(total: number) {
  const calls: { request: PageRequest; resolve(): void; reject(e: unknown): void }[] = [];
  let rows = total;
  const fetch = (request: PageRequest) =>
    new Promise<Page>((resolve, reject) => {
      const answer = (): Page => {
        const ids = [];
        for (let i = request.offset; i < Math.min(rows, request.offset + request.limit); i++) ids.push(row(request.descending ? rows - 1 - i : i));
        return { total: rows, offset: request.offset, rows: ids, anchorIndex: null };
      };
      calls.push({ request, resolve: () => resolve(answer()), reject });
    });
  return {
    calls,
    fetch,
    setTotal: (n: number) => (rows = n),
    /** Releases every pending answer. */
    async flush() {
      const pending = calls.splice(0);
      for (const c of pending) c.resolve();
      await Promise.resolve();
      await Promise.resolve();
    },
    offsets: () => calls.map((c) => c.request.offset),
  };
}

const q = (patch: Partial<Query> = {}): Query => ({ ...EMPTY_QUERY, ...patch });

describe('liveMode', () => {
  it('follows the end oldest-first, keeps the view newest-first, and obeys reveal requests', () => {
    expect(liveMode(false, false, true)).toBe('end');
    expect(liveMode(false, true, false)).toBe('keep');
    expect(liveMode(true, true, false)).toBe('keep');
    expect(liveMode(true, false, false)).toBe('shift');
    expect(liveMode(true, false, false, 'end')).toBe('end');
    expect(liveMode(false, false, true, 'start')).toBe('keep');
  });
});

describe('PagedRowsStore', () => {
  it('fetches the first page on reset and serves rows from the cache', async () => {
    const engine = fakeEngine(1200);
    const totals: (number | null)[] = [];
    const store = new PagedRowsStore({ fetch: engine.fetch, pageSize: 100, onTotal: (t) => totals.push(t) });
    store.reset(q(), false);
    expect(engine.offsets()).toEqual([0]);
    expect(store.loaded()).toBe(false);
    expect(store.rowAt(5)).toBeNull();
    await engine.flush();
    expect(store.loaded()).toBe(true);
    expect(store.total()).toBe(1200);
    expect(store.rowAt(5)?.id).toBe(5);
    expect(store.rowAt(150)).toBeNull();
    // A new query first clears the old total, so nothing shows a count for the wrong query.
    expect(totals).toEqual([null, 1200]);
    store.setTotal(1190);
    expect(totals).toEqual([null, 1200, 1190]);
    store.setTotal(1200);
    store.ensureRows([150, 160, 5]);
    expect(engine.offsets()).toEqual([100]);
    // In flight: not asked for twice.
    store.ensurePage(1);
    expect(engine.offsets()).toEqual([100]);
  });

  it('drops answers for an older query', async () => {
    const engine = fakeEngine(500);
    const store = new PagedRowsStore({ fetch: engine.fetch, pageSize: 100 });
    store.reset(q({ text: 'a' }), false);
    const old = engine.calls.splice(0);
    store.reset(q({ text: 'b' }), true);
    for (const c of old) c.resolve();
    await Promise.resolve();
    expect(store.loaded()).toBe(false);
    await engine.flush();
    expect(store.descending).toBe(true);
    expect(store.query?.text).toBe('b');
    expect(store.rowAt(0)?.id).toBe(499);
  });

  it('does not retry a failed page on scroll, only on retry', async () => {
    const engine = fakeEngine(1000);
    const store = new PagedRowsStore({ fetch: engine.fetch, pageSize: 100 });
    store.reset(q(), false);
    await engine.flush();
    store.ensurePage(3);
    engine.calls.splice(0)[0]!.reject(new Error('engine busy'));
    await Promise.resolve();
    await Promise.resolve();
    expect(store.error()).toBe('engine busy');
    store.ensureRows([350]);
    expect(engine.calls).toHaveLength(0);
    store.retry([350, 420]);
    expect(store.error()).toBeNull();
    expect(engine.offsets().sort()).toEqual([300, 400]);
  });

  it('retries from scratch when nothing loaded', async () => {
    const engine = fakeEngine(10);
    const store = new PagedRowsStore({ fetch: engine.fetch });
    store.reset(q(), false);
    const gen = store.generation;
    engine.calls.splice(0)[0]!.reject('down');
    await Promise.resolve();
    await Promise.resolve();
    expect(store.error()).toBe('down');
    store.retry([]);
    expect(store.generation).toBe(gen + 1);
    await engine.flush();
    expect(store.total()).toBe(10);
  });

  it('evicts the pages farthest from the view', async () => {
    const engine = fakeEngine(10_000);
    let centre = 0;
    const store = new PagedRowsStore({ fetch: engine.fetch, pageSize: 100, maxPages: 3, centre: () => centre });
    store.reset(q(), false);
    await engine.flush();
    store.ensureRows([100, 200]);
    await engine.flush();
    expect(store.pageCount).toBe(3);
    centre = 9000;
    store.ensureRows([9000]);
    await engine.flush();
    expect(store.pageCount).toBe(3);
    expect(store.rowAt(0)).toBeNull();
    expect(store.rowAt(9000)?.id).toBe(9000);
  });

  it('waits for scrolling to pause before fetching', async () => {
    vi.useFakeTimers();
    try {
      const engine = fakeEngine(1000);
      let now = 0;
      const store = new PagedRowsStore({ fetch: engine.fetch, pageSize: 100, idleMs: 50, now: () => now });
      store.reset(q(), false);
      await engine.flush();
      store.noteScroll();
      store.fill(() => [500]);
      expect(engine.calls).toHaveLength(0);
      store.fill(() => [600]);
      now = 60;
      vi.advanceTimersByTime(50);
      expect(engine.offsets()).toEqual([600]);
      // Not scrolling: at once.
      store.fill(() => [800]);
      expect(engine.offsets()).toEqual([600, 800]);
      store.dispose();
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps old rows during a live refresh and reports the arrival once', async () => {
    const engine = fakeEngine(250);
    const arrivals: unknown[] = [];
    const store = new PagedRowsStore({ fetch: engine.fetch, pageSize: 100, onLive: (a) => arrivals.push(a) });
    store.reset(q(), false);
    await engine.flush();
    store.ensureRows([150, 249]);
    await engine.flush();
    engine.setTotal(320);
    store.liveRefresh('end', 200, 250);
    // Old rows stay visible until the new ones arrive.
    expect(store.rowAt(150)?.id).toBe(150);
    expect(engine.offsets().sort()).toEqual([0, 200]);
    await engine.flush();
    expect(arrivals).toEqual([{ mode: 'end', newTotal: 320, delta: 70 }]);
    expect(store.total()).toBe(320);
    // Stale pages refetch once in view again.
    store.ensureRows([150]);
    expect(engine.offsets()).toEqual([100]);
  });

  it('drops the cache on a shifting live refresh (indexes moved)', async () => {
    const engine = fakeEngine(300);
    const store = new PagedRowsStore({ fetch: engine.fetch, pageSize: 100 });
    store.reset(q(), true);
    await engine.flush();
    engine.setTotal(310);
    store.liveRefresh('shift', 0, 10);
    expect(store.rowAt(0)).toBeNull();
    await engine.flush();
    expect(store.rowAt(0)?.id).toBe(309);
  });

  it('ignores a fetch before any query and accepts totals from jumps', () => {
    const engine = fakeEngine(5);
    const store = new PagedRowsStore({ fetch: engine.fetch });
    store.ensurePage(0);
    expect(engine.calls).toHaveLength(0);
    store.setTotal(42);
    expect(store.total()).toBe(42);
  });
});
