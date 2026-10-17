import { afterEach, describe, expect, it, vi } from 'vitest';
import { FlameGraph, HeapSummary, ProfileSummary, ProfileView } from '../api.types';
import { MockProfilingBackend, PROFILING_FAILABLE } from './mock-profiling';
import { HEAP_TAKEN_MS, heapSnapshot, MOCK_CAPTURE_PATHS, ORDERS_STARTED_MS, ordersProfile, REGRESSION_STARTED_MS, regressionProfile } from './mock-profiling-data';
import { transformStack, viewSamples } from './mock-profiling-engine';

async function demo(options = {}) {
  const b = new MockProfilingBackend(options);
  const [orders, regression, h1, h2, h3] = await Promise.all(MOCK_CAPTURE_PATHS.map((p) => b.open(p)));
  return { b, orders: orders as ProfileSummary, regression: regression as ProfileSummary, h1: h1 as HeapSummary, h2: h2 as HeapSummary, h3: h3 as HeapSummary };
}
const rejection = async (p: Promise<unknown>): Promise<string> => {
  try {
    await p;
  } catch (e) {
    return e instanceof Error ? e.message : String(e);
  }
  throw new Error('expected a rejection');
};
const names = (rows: { name: string }[]) => rows.map((r) => r.name);

describe('MockProfilingBackend: captures', () => {
  it('opens profiles and heap snapshots by file name', async () => {
    const { orders, regression, h1, h2, h3 } = await demo();
    expect(orders).toMatchObject({ capture: 'profile', id: 1, name: 'orders.speedscope.json', kind: 'mixed', format: 'speedscope', hasTime: true, defaultMetric: 'timeNs', sampleCount: 1200 });
    expect(orders.eventKinds.sort()).toEqual(['exception', 'gc', 'jit', 'lockWait', 'marker']);
    expect(orders.metrics.find((m) => m.id === 'calls')?.measurement).toBe('exact');
    expect(orders.metrics.find((m) => m.id === 'timeNs')?.measurement).toBe('sampled');
    expect(regression).toMatchObject({ id: 2, name: 'orders-regression.speedscope.json', kind: 'cpu', eventKinds: [], eventCount: 0 });
    expect(regression.metrics.map((m) => m.id)).toEqual(['samples', 'timeNs']);
    expect(h1).toMatchObject({ capture: 'heap', id: 3, name: 'heap-1.heapsnapshot', hasGraph: true, takenMs: HEAP_TAKEN_MS[0], notes: [] });
    expect(h2.takenMs).toBe(HEAP_TAKEN_MS[1]);
    expect(h3.notes).toHaveLength(1);
    expect(h1.typeCount).toBeGreaterThan(3);
    expect(h3.objectCount).toBeGreaterThan(h1.objectCount);
  });

  it('recognises the other profile extensions and heap name variants', async () => {
    const b = new MockProfilingBackend();
    for (const n of ['a.cpuprofile', 'a.nettrace', 'a.folded', 'a.pb.gz', 'a.trace.json', 'a.perf.txt', 'C:\\dir\\A.SPEEDSCOPE.JSON']) {
      expect((await b.open(`/x/${n}`)).capture).toBe('profile');
    }
    expect((await b.open('/x/my-regression.cpuprofile')).name).toBe('my-regression.cpuprofile');
    const sizes = new Map<string, number>();
    for (const n of ['heap.heapsnapshot', 'heap-0.heapsnapshot', 'heap-9.heapsnapshot', 'heap2.heapsnapshot']) {
      const s = (await b.open(n)) as HeapSummary;
      sizes.set(n, s.objectCount);
    }
    const one = (await demo()).h1.objectCount;
    expect(sizes.get('heap.heapsnapshot')).toBe(one);
    expect(sizes.get('heap-0.heapsnapshot')).toBe(one);
    expect(sizes.get('heap-9.heapsnapshot')).toBe((await demo()).h3.objectCount);
    expect(sizes.get('heap2.heapsnapshot')).toBe((await demo()).h2.objectCount);
  });

  it('rejects unrecognised and missing files with typed failures, without consuming an id', async () => {
    const b = new MockProfilingBackend();
    expect(await rejection(b.open('/x/notes.txt'))).toContain('notes.txt is not a profile or heap snapshot that IMT recognises');
    await expect(b.open('/x/notes.txt')).rejects.toMatchObject({ code: 'notACapture' });
    await expect(b.open('/gone/missing.nettrace')).rejects.toMatchObject({ code: 'notFound', message: expect.stringContaining('/gone/missing.nettrace') });
    expect((await b.open('/x/a.folded')).id).toBe(1);
    await expect(b.hotspots({ profileId: 99 })).rejects.toMatchObject({ code: 'notFound' });
    await b.open('/x/b-regression.folded');
    await expect(b.hotspots({ profileId: 2, metric: 'allocBytes' })).rejects.toMatchObject({ code: 'unsupported' });
    await expect(b.hotspots({ profileId: 1, limit: 0 })).rejects.toMatchObject({ code: 'invalid' });
    await expect(b.tree({ profileId: 1, path: [999] })).rejects.toMatchObject({ code: 'invalid' });
  });

  it('sniffs paths by name without opening them', async () => {
    const b = new MockProfilingBackend();
    expect(await b.sniff(['/a/x.log', '/a/orders.nettrace', '/a/heap-2.heapsnapshot', '/a/missing.nettrace'])).toEqual([
      { path: '/a/x.log', kind: 'none' },
      { path: '/a/orders.nettrace', kind: 'capture' },
      { path: '/a/heap-2.heapsnapshot', kind: 'heap' },
      { path: '/a/missing.nettrace', kind: 'none' },
    ]);
    expect(await b.list()).toEqual([]);
  });

  it('enforces the 32 capture limit until one is closed', async () => {
    const b = new MockProfilingBackend();
    for (let i = 0; i < 32; i++) await b.open(`/x/${i}.folded`);
    await expect(b.open('/x/over.folded')).rejects.toMatchObject({ code: 'limit', message: 'open captures limit exceeded (32); close a capture first' });
    expect(await b.close(5)).toBe(true);
    expect(await b.close(5)).toBe(false);
    expect((await b.open('/x/over.folded')).id).toBe(33);
    expect(await b.list()).toHaveLength(32);
  });

  it('lists and closes captures', async () => {
    const { b } = await demo();
    expect((await b.list()).map((c) => c.id)).toEqual([1, 2, 3, 4, 5]);
    await b.close(2);
    expect((await b.list()).map((c) => c.id)).toEqual([1, 3, 4, 5]);
    expect(await rejection(b.hotspots({ profileId: 2 }))).toBe('capture 2 is not open');
  });

  it('reports spans for profiles (start..end) and heaps (a point)', async () => {
    const { b } = await demo();
    const spans = await b.spans();
    expect(spans).toHaveLength(5);
    expect(spans[0]).toEqual({ id: 1, name: 'orders.speedscope.json', capture: 'profile', kind: 'mixed', fromMs: ORDERS_STARTED_MS, toMs: ORDERS_STARTED_MS + 20_000 });
    expect(spans[1]).toMatchObject({ fromMs: REGRESSION_STARTED_MS, toMs: REGRESSION_STARTED_MS + 15_000, kind: 'cpu' });
    expect(spans[2]).toEqual({ id: 3, name: 'heap-1.heapsnapshot', capture: 'heap', kind: null, fromMs: HEAP_TAKEN_MS[0], toMs: HEAP_TAKEN_MS[0] });
  });

  it('keeps profile and heap ids apart with clear errors', async () => {
    const { b } = await demo();
    expect(await rejection(b.hotspots({ profileId: 3 }))).toBe('capture 3 is a heap snapshot, not a profile');
    expect(await rejection(b.heapTypes({ snapshotId: 1 }))).toBe('capture 1 is a profile, not a heap snapshot');
    expect(await rejection(b.heapTypes({ snapshotId: 99 }))).toBe('capture 99 is not open');
    expect(await rejection(b.tree({ profileId: 99 }))).toBe('capture 99 is not open');
    expect(await rejection(b.functionDiff({ baselineId: 3, targetId: 1 }))).toContain('heap snapshot, not a profile');
    expect(await rejection(b.heapDiff({ baselineId: 1, targetId: 3 }))).toContain('is a profile, not a heap');
  });

  it('rejects metrics the profile does not have', async () => {
    const { b, regression } = await demo();
    expect(await rejection(b.hotspots({ profileId: regression.id, metric: 'calls' }))).toBe('the profile has no calls metric');
    expect(await rejection(b.tree({ profileId: 2, metric: 'waitNs' }))).toContain('no waitNs metric');
    expect(await rejection(b.flame({ profileId: 2, widthPx: 100, metric: 'allocBytes' }))).toContain('no allocBytes');
    expect(await rejection(b.diffFlame({ baselineId: 2, targetId: 1, widthPx: 100, metric: 'calls' }))).toBe('the baseline has no calls metric');
  });
});

describe('MockProfilingBackend: options', () => {
  it('parses the browser demo switches', () => {
    const o = MockProfilingBackend.optionsFromSearch;
    expect(o('')).toEqual({});
    expect(o('?mockProfilingLatency=300')).toEqual({ latencyMs: 300 });
    expect(o('?mockProfilingLatency=0')).toEqual({});
    expect(o('?mockProfilingLatency=-5')).toEqual({});
    expect(o('?mockProfilingLatency=1.5')).toEqual({});
    expect(o('?mockProfilingLatency=60000')).toEqual({});
    expect(o('?mockProfilingLatency=abc')).toEqual({});
    expect(o('?mockProfilingFail=hotspots:1,flame:2')).toEqual({ fail: { hotspots: 1, flame: 2 } });
    expect(o('mockProfilingFail=tree,heapTypes:x,nope:3,hotPath:0')).toEqual({ fail: { tree: 1, heapTypes: 1, hotPath: 1 } });
    expect(o('?mockProfilingFail=bogus:2')).toEqual({});
    expect(o('mockProfilingLatency=5&mockProfilingFail=open:1')).toEqual({ latencyMs: 5, fail: { open: 1 } });
    for (const op of PROFILING_FAILABLE) expect(o(`mockProfilingFail=${op}:1`).fail).toEqual({ [op]: 1 });
  });

  it('fails the first N calls of an operation, then succeeds', async () => {
    const b = new MockProfilingBackend({ fail: { open: 1, spans: 2 } });
    expect(await rejection(b.open('/x/a.folded'))).toBe('mock: open failed (simulated)');
    expect((await b.open('/x/a.folded')).id).toBe(1);
    expect(await rejection(b.spans())).toContain('spans failed');
    expect(await rejection(b.spans())).toContain('spans failed');
    expect(await b.spans()).toHaveLength(1);
    expect(await b.list()).toHaveLength(1); // list is never failed
    await expect(new MockProfilingBackend({ fail: { sniff: 1 } }).sniff(['/a.folded'])).rejects.toMatchObject({ code: 'io' });
  });

  it('does not change the caller options when failing', async () => {
    const fail = { hotspots: 1 };
    const b = new MockProfilingBackend({ fail });
    await b.open('/x/a.folded');
    await rejection(b.hotspots({ profileId: 1 }));
    expect(fail.hotspots).toBe(1);
  });

  describe('latency', () => {
    afterEach(() => vi.useRealTimers());
    it('delays every answer', async () => {
      vi.useFakeTimers();
      const b = new MockProfilingBackend({ latencyMs: 250 });
      let done = false;
      const p = b.open('/x/a.folded').then((v) => {
        done = true;
        return v;
      });
      await vi.advanceTimersByTimeAsync(249);
      expect(done).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      expect((await p).id).toBe(1);
    });
  });

  it('opens the preopen paths before the first call (once, ignoring bad ones throws)', async () => {
    const b = new MockProfilingBackend({ preopen: [MOCK_CAPTURE_PATHS[0], MOCK_CAPTURE_PATHS[2]] });
    expect((await b.list()).map((c) => c.name)).toEqual(['orders.speedscope.json', 'heap-1.heapsnapshot']);
    expect(await b.list()).toHaveLength(2);
    const bad = new MockProfilingBackend({ preopen: ['/x/readme.txt'] });
    expect(await rejection(bad.list())).toContain('not a profile or heap snapshot');
    expect(await bad.list()).toEqual([]); // consumed
  });
});

describe('MockProfilingBackend: hotspots', () => {
  it('sorts by average per call (rows without calls last) like the engine', async () => {
    const { b } = await demo();
    const avgs = async (descending: boolean) =>
      (await b.hotspots({ profileId: 1, sort: { metric: 'timeNs', of: 'avgPerCall', descending }, limit: 100 })).rows.map((r) => r.avgPerCallNs);
    const desc = await avgs(true);
    const known = desc.filter((x): x is number => x !== null);
    expect(known).toEqual([...known].sort((x, y) => y - x));
    expect(desc.slice(known.length).every((x) => x === null)).toBe(true);
    const asc = (await avgs(false)).filter((x): x is number => x !== null);
    expect(asc).toEqual([...asc].sort((x, y) => x - y));
    // The regression profile has no call counts.
    await expect(b.hotspots({ profileId: 2, sort: { metric: 'timeNs', of: 'avgPerCall' } })).rejects.toMatchObject({ code: 'unsupported', message: expect.stringContaining('no call counts') });
  });

  it('finds the hottest top-down paths to a function', async () => {
    const { b } = await demo();
    const r = await b.framePaths({ profileId: 1, frame: 4 });
    expect(r.totalPaths).toBeGreaterThan(0);
    expect(r.paths[0].path.at(-1)).toBe(4);
    expect(r.paths[0].frames.map((f) => f.id)).toEqual(r.paths[0].path);
    expect(r.paths[0].path.filter((id) => id === 4)).toHaveLength(1);
    const values = r.paths.map((x) => x.value);
    expect(values).toEqual([...values].sort((x, y) => y - x));
    expect((await b.framePaths({ profileId: 1, frame: 4, view: { threads: [3] } })).totalPaths, 'not in this view').toBe(0);
    await expect(b.framePaths({ profileId: 1, frame: 19 })).rejects.toMatchObject({ code: 'invalid' });
    await expect(b.framePaths({ profileId: 1, frame: 4, limit: 0 })).rejects.toMatchObject({ code: 'invalid' });
    await expect(b.framePaths({ profileId: 9, frame: 4 })).rejects.toMatchObject({ code: 'notFound' });
  });

  it('lists functions sorted by self time descending by default, with sampled/exact metrics and totals', async () => {
    const { b } = await demo();
    const page = await b.hotspots({ profileId: 1 });
    expect(page.metric).toBe('timeNs');
    expect(page.measurement).toBe('sampled');
    expect(page.samplesInView).toBe(1200);
    expect(page.totalRows).toBe(page.rows.length);
    const selfs = page.rows.map((r) => r.self.timeNs ?? 0);
    expect(selfs).toEqual([...selfs].sort((a, b2) => b2 - a));
    expect(page.rows[0].frame).not.toBeNull();
    expect(page.rows[0].avgPerCallNs).toBeGreaterThan(0);
    const sumSelfPct = page.rows.reduce((a, r) => a + r.selfPct, 0);
    expect(sumSelfPct).toBeCloseTo(100, 5);
  });

  it('reports an exact measurement for exact metrics', async () => {
    const { b } = await demo();
    expect((await b.hotspots({ profileId: 1, metric: 'calls' })).measurement).toBe('exact');
    expect((await b.hotspots({ profileId: 1, metric: 'samples' })).measurement).toBe('sampled');
  });

  it('groups by function, type, namespace, module and thread', async () => {
    const { b } = await demo();
    const by = async (groupBy: 'function' | 'type' | 'namespace' | 'module' | 'thread') => (await b.hotspots({ profileId: 1, groupBy })).rows;
    const fn = await by('function');
    expect(fn.every((r) => r.frame !== null && r.thread === null)).toBe(true);
    expect(fn.find((r) => r.name === 'Shop.Orders.OrderService.Place')?.module).toBe('Shop.Orders');
    const types = await by('type');
    expect(names(types)).toContain('Shop.Orders.OrderService');
    expect(types.every((r) => r.frame === null && r.module === null)).toBe(true);
    const ns = await by('namespace');
    expect(names(ns)).toContain('Shop.Orders');
    expect(names(ns)).toContain('System.Text.Json');
    expect(names(ns)).toContain('ntdll.dll'); // two-part name falls back to the module
    expect(names(ns)).toContain('[unknown]'); // no module: falls back to the name
    const modules = await by('module');
    expect(names(modules)).toContain('Shop.Orders');
    expect(names(modules)).toContain('(unknown module)');
    expect(modules.find((r) => r.name === '(unknown module)')?.key).toBe('(unknown)');
    const threads = await by('thread');
    expect(threads.map((r) => r.thread).sort()).toEqual([0, 1, 2, 3]);
    expect(names(threads)).toContain('Main thread');
    expect(names(threads)).toContain('.NET BGC');
    const typeless = await by('type');
    expect(names(typeless)).toContain('[unknown]'); // no dot: the whole name
  });

  it('falls back to a generated thread name for unknown threads', async () => {
    const b = new MockProfilingBackend();
    await b.open('/x/a.speedscope.json');
    const view: ProfileView = { threads: [1] };
    const page = await b.hotspots({ profileId: 1, groupBy: 'thread', view });
    expect(page.rows.map((r) => r.thread)).toEqual([1]);
  });

  it('sorts by name and by metric, ascending and descending', async () => {
    const { b } = await demo();
    // sorting by name is ascending unless `descending` is set
    const byName = (await b.hotspots({ profileId: 1, sort: { metric: null } })).rows;
    expect(names(byName)).toEqual(names(byName).slice().sort((x, y) => x.localeCompare(y)));
    const nameDesc = (await b.hotspots({ profileId: 1, sort: { metric: null, descending: true } })).rows;
    expect(names(nameDesc)).toEqual(names(byName).slice().reverse());
    const totalDesc = (await b.hotspots({ profileId: 1, sort: { metric: 'timeNs', of: 'total' } })).rows.map((r) => r.total.timeNs ?? 0);
    expect(totalDesc).toEqual(totalDesc.slice().sort((x, y) => y - x));
    const callsAsc = (await b.hotspots({ profileId: 1, sort: { metric: 'calls', of: 'self', descending: false } })).rows.map((r) => r.self.calls ?? 0);
    expect(callsAsc).toEqual(callsAsc.slice().sort((x, y) => x - y));
    // sort.metric undefined falls back to the table metric
    const dflt = (await b.hotspots({ profileId: 1, sort: {}, metric: 'samples' })).rows.map((r) => r.self.samples ?? 0);
    expect(dflt).toEqual(dflt.slice().sort((x, y) => y - x));
  });

  it('searches names and modules case-insensitively, ignoring blank searches', async () => {
    const { b } = await demo();
    const hits = await b.hotspots({ profileId: 1, search: ' ORDERREPOSITORY ' });
    expect(names(hits.rows)).toEqual(['Shop.Orders.Data.OrderRepository.Save']);
    expect(hits.totalRows).toBe(1);
    const byModule = await b.hotspots({ profileId: 1, search: 'sqlclient' });
    expect(names(byModule.rows)).toEqual(['Microsoft.Data.SqlClient.SqlCommand.ExecuteReader']);
    expect((await b.hotspots({ profileId: 1, search: '   ' })).totalRows).toBeGreaterThan(10);
    expect((await b.hotspots({ profileId: 1, search: 'nothing-like-this' })).rows).toEqual([]);
  });

  it('pages with offset and limit and validates the limit', async () => {
    const { b } = await demo();
    const all = await b.hotspots({ profileId: 1 });
    const page = await b.hotspots({ profileId: 1, offset: 2, limit: 3 });
    expect(page.rows).toEqual(all.rows.slice(2, 5));
    expect(page.totalRows).toBe(all.totalRows);
    expect((await b.hotspots({ profileId: 1, limit: 10_000 })).rows.length).toBe(all.rows.length);
    expect(await rejection(b.hotspots({ profileId: 1, limit: 0 }))).toBe('limit must be 1..10000');
    expect(await rejection(b.hotspots({ profileId: 1, limit: 10_001 }))).toBe('limit must be 1..10000');
    expect(await rejection(b.tree({ profileId: 1, limit: -1 }))).toContain('limit must be');
  });

  it('applies range and thread filters of the view', async () => {
    const { b } = await demo();
    const full = await b.hotspots({ profileId: 1 });
    const ranged = await b.hotspots({ profileId: 1, view: { range: { fromNs: 0, toNs: 5_000_000_000 } } });
    expect(ranged.samplesInView).toBeLessThan(full.samplesInView);
    expect(ranged.samplesInView).toBeGreaterThan(0);
    const thread3 = await b.hotspots({ profileId: 1, view: { threads: [3] } });
    expect(names(thread3.rows)).toContain('coreclr!gc_heap::garbage_collect');
    expect(names(thread3.rows)).not.toContain('Shop.Orders.OrderService.Place');
    expect((await b.hotspots({ profileId: 1, view: { threads: [] } })).samplesInView).toBe(1200);
  });
});

describe('just my code and recursion folding', () => {
  const p = ordersProfile();
  const stackOf = (n: number[]) => n;

  it('folds consecutive framework frames when enabled (flags by default)', () => {
    // 0 app, 1 framework, 6 framework, 7 framework, 3 app
    expect(transformStack(stackOf([0, 1, 6, 7, 3]), p, { justMyCode: { enabled: true } })).toEqual([0, 1, 3]);
    expect(transformStack([0, 1, 6, 7, 3], p, { justMyCode: {} })).toEqual([0, 1, 3]);
  });

  it('does nothing when disabled or absent', () => {
    expect(transformStack([0, 1, 6, 7, 3], p, { justMyCode: { enabled: false } })).toEqual([0, 1, 6, 7, 3]);
    expect(transformStack([0, 1, 6, 7, 3], p, { justMyCode: null })).toEqual([0, 1, 6, 7, 3]);
    expect(transformStack([0, 1, 6, 7, 3], p, {})).toEqual([0, 1, 6, 7, 3]);
  });

  it('uses framework patterns on names and modules (wildcards, case-insensitive, escaped)', () => {
    const jmc = (frameworkPatterns: string[], useFrameFlags = false) => ({ justMyCode: { frameworkPatterns, useFrameFlags } });
    // useFrameFlags false: nothing is framework without patterns
    expect(transformStack([0, 1, 6, 7, 3], p, jmc([]))).toEqual([0, 1, 6, 7, 3]);
    // module pattern
    expect(transformStack([0, 1, 6, 7, 3], p, jmc(['SYSTEM.*']))).toEqual([0, 1, 3]);
    // name pattern: only 6 and 7 match 'System.Text.Json.*', 1 is app
    expect(transformStack([0, 1, 6, 7, 3], p, jmc(['System.Text.Json.*']))).toEqual([0, 1, 6, 3]);
    // regex metacharacters are literal, '[unknown]' matches frame 16 by name
    expect(transformStack([0, 16, 16], p, jmc(['[unknown]']))).toEqual([0, 16]);
    expect(transformStack([0, 13, 13], p, jmc(['coreclr!gc_heap::garbage_collect']))).toEqual([0, 13]);
    // combined with flags
    expect(transformStack([0, 1, 6], p, jmc([], true))).toEqual([0, 1]);
  });

  it('keeps unknown frame ids and never folds the first frame', () => {
    expect(transformStack([99, 98, 1, 6], p, { justMyCode: { enabled: true } })).toEqual([99, 98, 1]);
    expect(transformStack([1, 6, 7], p, { justMyCode: { enabled: true } })).toEqual([1]);
  });

  it("recursion 'direct' folds adjacent repeats only, 'all' unwinds any cycle, 'off' keeps all", () => {
    expect(transformStack([0, 4, 4, 4, 5, 4], p, { recursion: 'direct' })).toEqual([0, 4, 5, 4]);
    expect(transformStack([0, 4, 5, 4, 5, 6], p, { recursion: 'all' })).toEqual([0, 4, 5, 6]);
    expect(transformStack([0, 4, 4], p, { recursion: 'off' })).toEqual([0, 4, 4]);
  });

  it('is applied through the view of every query', async () => {
    const { b } = await demo();
    const stackDepth = async (view: ProfileView) => {
      const g = await b.flame({ profileId: 1, widthPx: 2000, view, minWidthPx: 0 });
      return g.maxDepth;
    };
    expect(await stackDepth({})).toBe(7);
    // [Main, Dispatch, Post, Place, Calculate, Calculate, Calculate] folds to end at the first Calculate
    const below = (view: ProfileView) => b.tree({ profileId: 1, path: [0, 1, 2, 3, 4], view }).then((t) => t.nodes.length);
    expect(await below({})).toBe(1);
    expect(await below({ recursion: 'off' })).toBe(1);
    expect(await below({ recursion: 'direct' })).toBe(0);
    expect(await below({ recursion: 'all' })).toBe(0);
    const jmc = await b.hotspots({ profileId: 1, view: { justMyCode: { enabled: true } } });
    const plainRows = await b.hotspots({ profileId: 1 });
    expect(jmc.rows.length).toBeLessThan(plainRows.rows.length);
    // only framework frames called from framework frames fold away (Serialize is called from app code)
    expect(names(jmc.rows)).not.toContain('System.Text.Json.Utf8JsonWriter.WriteString');
    expect(names(jmc.rows)).toContain('System.Text.Json.JsonSerializer.Serialize');
    const noFlags = await b.hotspots({ profileId: 1, view: { justMyCode: { useFrameFlags: false } } });
    expect(noFlags.rows.length).toBe(plainRows.rows.length);
    const patterned = await b.hotspots({ profileId: 1, view: { justMyCode: { useFrameFlags: false, frameworkPatterns: ['System.Text.Json'] } } });
    expect(names(patterned.rows)).not.toContain('System.Text.Json.Utf8JsonWriter.WriteString');
  });

  it('viewSamples keeps samples inclusive of both range ends', () => {
    const s = viewSamples(p, { range: { fromNs: p.samples[0].t, toNs: p.samples[0].t } });
    expect(s).toHaveLength(1);
  });
});

describe('MockProfilingBackend: call tree', () => {
  it('pages the top-down children, largest first, with grand totals', async () => {
    const { b } = await demo();
    const root = await b.tree({ profileId: 1 });
    expect(root.measurement).toBe('sampled');
    expect(root.metric).toBe('timeNs');
    expect(root.totalChildren).toBe(root.nodes.length);
    expect(root.nodes[0].frame.name).toBe('Shop.Orders.Program.Main');
    expect(root.nodes[0].totalPct).toBeCloseTo(100, 5);
    expect(root.nodes[0].hasChildren).toBe(true);
    const kids = await b.tree({ profileId: 1, path: [0] });
    const totals = kids.nodes.map((n) => n.total.timeNs ?? 0);
    expect(totals).toEqual(totals.slice().sort((x, y) => y - x));
    const paged = await b.tree({ profileId: 1, path: [0], offset: 1, limit: 1 });
    expect(paged.nodes).toEqual([kids.nodes[1]]);
    expect(paged.totalChildren).toBe(kids.totalChildren);
  });

  it('flags leaves and supports bottom-up trees', async () => {
    const { b } = await demo();
    const mainKids = await b.tree({ profileId: 1, path: [0] });
    expect(mainKids.nodes.find((n) => n.frame.id === 17)?.hasChildren).toBe(true);
    expect(mainKids.nodes.find((n) => n.frame.id === 13)?.hasChildren).toBe(false);
    const leaf = await b.tree({ profileId: 1, path: [0, 17, 18] });
    expect(leaf.nodes).toEqual([]);
    expect(leaf.totalChildren).toBe(0);
    const up = await b.tree({ profileId: 1, direction: 'bottomUp' });
    const leaves = up.nodes.map((n) => n.frame.id);
    expect(leaves).toContain(3); // OrderService.Place is a leaf frame of some stacks
    expect(leaves).not.toContain(0); // Main is never a leaf
    const callers = await b.tree({ profileId: 1, direction: 'bottomUp', path: [13] });
    expect(callers.nodes.map((n) => n.frame.id)).toEqual([0]);
  });

  it('rejects a path that is not in the view', async () => {
    const { b } = await demo();
    expect(await rejection(b.tree({ profileId: 1, path: [0, 19] }))).toBe('no call-tree node at path [0, 19] in this view');
    expect(await rejection(b.hotPath({ profileId: 1, path: [5] }))).toContain('no call-tree node at path [5]');
  });

  it('follows the hottest path until a node falls under the minimum fraction', async () => {
    const { b } = await demo();
    const hot = await b.hotPath({ profileId: 1 });
    expect(hot.path.slice(0, 2)).toEqual([0, 1]);
    expect(hot.nodes).toHaveLength(hot.path.length);
    const strict = await b.hotPath({ profileId: 1, minFraction: 0.5 });
    expect(strict.path.length).toBeLessThan(hot.path.length);
    const nothing = await b.hotPath({ profileId: 1, minFraction: 2 });
    expect(nothing).toEqual({ path: [], nodes: [] });
    const fromPath = await b.hotPath({ profileId: 1, path: [0, 1], direction: 'topDown' });
    expect(fromPath.path.slice(0, 2)).toEqual([0, 1]);
    expect(fromPath.path.length).toBeGreaterThan(2);
    const up = await b.hotPath({ profileId: 1, direction: 'bottomUp', metric: 'samples' });
    expect(up.path.length).toBeGreaterThan(0);
  });
});

describe('MockProfilingBackend: butterfly', () => {
  it('lists callers and callees with shares of the function total', async () => {
    const { b } = await demo();
    const bf = await b.butterfly({ profileId: 1, frame: 3 });
    expect(bf.function.name).toBe('Shop.Orders.OrderService.Place');
    expect(bf.callers.map((r) => r.frame?.id)).toEqual([2]);
    expect(bf.callers[0].pct).toBeCloseTo(100, 5);
    expect(bf.callees.map((r) => r.frame?.id)).toContain(8);
    expect(bf.callees.every((r, i, a) => i === 0 || (a[i - 1].values.timeNs ?? 0) >= (r.values.timeNs ?? 0))).toBe(true);
    expect(bf.metric).toBe('timeNs');
  });

  it('shows a null caller for top-of-stack functions and limits rows', async () => {
    const { b } = await demo();
    const main = await b.butterfly({ profileId: 1, frame: 0 });
    expect(main.callers).toEqual([{ frame: null, values: expect.anything(), pct: expect.any(Number) }]);
    const one = await b.butterfly({ profileId: 1, frame: 0, limit: 1 });
    expect(one.callees).toHaveLength(1);
    expect(main.callees.length).toBeGreaterThan(1);
    const leaf = await b.butterfly({ profileId: 1, frame: 18 });
    expect(leaf.callees).toEqual([]);
  });

  it('rejects a function without samples', async () => {
    const { b } = await demo();
    expect(await rejection(b.butterfly({ profileId: 1, frame: 19 }))).toBe('function 19 has no samples in this view');
    expect(await rejection(b.butterfly({ profileId: 1, frame: 3, limit: 0 }))).toContain('limit must be');
  });
});

describe('MockProfilingBackend: flame graph', () => {
  const req = { profileId: 1, widthPx: 1000 };
  const sumWidth = (g: FlameGraph, depth: number) => g.rects.filter((r) => r.depth === depth).reduce((a, r) => a + r.w, 0);

  it('links each rectangle to its parent and names the focus path', async () => {
    const { b } = await demo();
    const g = await b.flame(req);
    expect(g.focusFrames).toEqual([]);
    g.rects.forEach((r, i) => {
      if (r.depth === 0) expect(r.parent).toBeNull();
      else {
        const p = g.rects[r.parent!];
        expect(r.parent!).toBeLessThan(i);
        expect(p.depth + 1).toBe(r.depth);
        expect(r.x).toBeGreaterThanOrEqual(p.x - 1e-9);
      }
    });
    const zoomed = await b.flame({ ...req, focus: [0, 1] });
    expect(zoomed.focusFrames.map((f) => f.id)).toEqual([0, 1]);
  });

  it('lays out the whole tree as fractions of the width, ordered by name by default', async () => {
    const { b } = await demo();
    const g = await b.flame(req);
    expect(g.metric).toBe('timeNs');
    expect(g.measurement).toBe('sampled');
    expect(g.truncated).toBe(false);
    expect(g.focusValue).toBe(g.grandTotal);
    expect(g.maxDepth).toBe(Math.max(...g.rects.map((r) => r.depth)));
    expect(sumWidth(g, 0)).toBeCloseTo(1, 6);
    const depth0 = g.rects.filter((r) => r.depth === 0);
    expect(depth0[0].frame).toBe(0);
    expect(depth0[0].x).toBe(0);
    const usedFrames = new Set(g.rects.map((r) => r.frame));
    expect(g.frames.map((f) => f.id).sort((x, y) => x - y)).toEqual([...usedFrames].sort((x, y) => x - y));
    const level1 = g.rects.filter((r) => r.depth === 1).map((r) => g.frames.find((f) => f.id === r.frame)!.name);
    expect(level1).toEqual(level1.slice().sort((x, y) => x.localeCompare(y)));
    expect(g.rects.every((r) => r.delta === undefined && r.baselineValue === undefined)).toBe(true);
  });

  it('orders by value when asked', async () => {
    const { b } = await demo();
    const g = await b.flame({ ...req, order: 'value' });
    const level1 = g.rects.filter((r) => r.depth === 1);
    const values = level1.map((r) => r.value);
    expect(values).toEqual(values.slice().sort((x, y) => y - x));
  });

  it('supports bottom-up direction', async () => {
    const { b } = await demo();
    const g = await b.flame({ ...req, direction: 'bottomUp' });
    expect(g.rects.filter((r) => r.depth === 0).every((r) => g.frames.find((f) => f.id === r.frame))).toBe(true);
    expect(g.rects.filter((r) => r.depth === 0).map((r) => r.frame)).toContain(13);
  });

  it('zooms into a focus path (depth 0 is the focus, width 1)', async () => {
    const { b } = await demo();
    const g = await b.flame({ ...req, focus: [0, 1, 2, 3] });
    const top = g.rects.filter((r) => r.depth === 0);
    expect(top).toHaveLength(1);
    expect(top[0]).toMatchObject({ frame: 3, x: 0, w: 1 });
    expect(g.focusValue).toBe(top[0].value);
    expect(g.grandTotal).toBeGreaterThan(g.focusValue);
    expect(await rejection(b.flame({ ...req, focus: [9, 9] }))).toContain('no call-tree node at path');
  });

  it('highlights search matches (plain, case-insensitive) and sums the topmost matches', async () => {
    const { b } = await demo();
    const g = await b.flame({ ...req, search: 'orderrepository' });
    const matched = g.rects.filter((r) => r.matched);
    expect(matched.map((r) => g.frames.find((f) => f.id === r.frame)?.name)).toEqual(['Shop.Orders.Data.OrderRepository.Save']);
    expect(g.matchedValue).toBe(matched[0].value);
    expect(g.matchedPct).toBeCloseTo((matched[0].value / g.focusValue) * 100, 6);
    // a plain search treats regex characters literally
    expect((await b.flame({ ...req, search: 'Order.*' })).matchedValue).toBe(0);
    expect((await b.flame(req)).matchedPct).toBe(0);
  });

  it('does not double count nested matches', async () => {
    const { b } = await demo();
    const g = await b.flame({ ...req, search: 'Shop.Orders' });
    const rootMatch = g.rects.find((r) => r.depth === 0 && r.matched)!;
    expect(g.matchedValue).toBe(rootMatch.value);
  });

  it('searches with a regex and rejects an invalid one', async () => {
    const { b } = await demo();
    const g = await b.flame({ ...req, search: '^System\\.Text\\.Json\\.(Utf8|Json)', searchRegex: true });
    const matched = g.rects.filter((r) => r.matched).map((r) => g.frames.find((f) => f.id === r.frame)!.name);
    expect(matched.sort()).toEqual(['System.Text.Json.JsonSerializer.Serialize', 'System.Text.Json.Utf8JsonWriter.WriteString']);
    expect(await rejection(b.flame({ ...req, search: '(', searchRegex: true }))).toBe('invalid search pattern: (');
  });

  it('crops to the viewport, with edge rectangles extending past 0..1', async () => {
    const { b } = await demo();
    const g = await b.flame({ ...req, viewport: { start: 0.25, end: 0.5 } });
    const rootRect = g.rects.find((r) => r.depth === 0)!;
    expect(rootRect.x).toBeCloseTo(-1, 6);
    expect(rootRect.w).toBeCloseTo(4, 6);
    expect(g.rects.length).toBeLessThan((await b.flame(req)).rects.length);
    for (const r of g.rects) expect(r.x + r.w).toBeGreaterThan(0);
    for (const r of g.rects) expect(r.x).toBeLessThan(1);
  });

  it('validates the width and the viewport', async () => {
    const { b } = await demo();
    expect(await rejection(b.flame({ profileId: 1, widthPx: 0 }))).toBe('widthPx must be 1..100000');
    expect(await rejection(b.flame({ profileId: 1, widthPx: 100_001 }))).toBe('widthPx must be 1..100000');
    expect(await rejection(b.flame({ profileId: 1, widthPx: NaN }))).toContain('widthPx');
    const bad = 'viewport must satisfy 0 ≤ start < end ≤ 1';
    expect(await rejection(b.flame({ ...req, viewport: { start: 0.5, end: 0.5 } }))).toBe(bad);
    expect(await rejection(b.flame({ ...req, viewport: { start: -0.1, end: 0.5 } }))).toBe(bad);
    expect(await rejection(b.flame({ ...req, viewport: { start: 0, end: 1.1 } }))).toBe(bad);
  });

  it('truncates at maxRects and flags it', async () => {
    const { b } = await demo();
    const g = await b.flame({ ...req, maxRects: 5 });
    expect(g.rects).toHaveLength(5);
    expect(g.truncated).toBe(true);
    expect((await b.flame({ ...req, maxRects: 100_000 })).truncated).toBe(false);
  });

  it('culls rectangles narrower than minWidthPx and limits depth', async () => {
    const { b } = await demo();
    const all = await b.flame({ ...req, minWidthPx: 0 });
    const culled = await b.flame({ ...req, minWidthPx: 100 });
    expect(culled.rects.length).toBeLessThan(all.rects.length);
    expect(culled.rects.every((r) => r.w * 1000 >= 100)).toBe(true);
    const shallow = await b.flame({ ...req, maxDepth: 2 });
    expect(shallow.maxDepth).toBe(1);
    expect(shallow.rects.every((r) => r.depth <= 1)).toBe(true);
    const nullable = await b.flame({ ...req, maxDepth: null, minWidthPx: null, maxRects: null });
    expect(nullable.rects.length).toBeGreaterThan(0);
  });

  it('uses the requested metric', async () => {
    const { b } = await demo();
    const g = await b.flame({ ...req, metric: 'calls' });
    expect(g.metric).toBe('calls');
    expect(g.measurement).toBe('exact');
  });

  it('draws a differential graph with delta and baselineValue against the baseline', async () => {
    const { b } = await demo();
    const g = await b.diffFlame({ baselineId: 1, targetId: 2, widthPx: 1000 });
    expect(g.rects.length).toBeGreaterThan(0);
    expect(g.rects.every((r) => typeof r.delta === 'number' && typeof r.baselineValue === 'number')).toBe(true);
    const nameOf = (id: number) => g.frames.find((f) => f.id === id)!.name;
    const json = g.rects.find((r) => nameOf(r.frame) === 'Shop.Orders.Serialization.JsonWriter.Write')!;
    expect(json.delta!).toBeGreaterThan(0.05);
    const audit = g.rects.find((r) => nameOf(r.frame) === 'Shop.Orders.AuditLog.Write')!;
    expect(audit.baselineValue).toBe(0);
    expect(audit.delta!).toBeGreaterThan(0);
    const calc = g.rects.find((r) => nameOf(r.frame) === 'Shop.Orders.Pricing.PriceCalculator.Calculate')!;
    expect(calc.delta!).toBeLessThan(0);
    const root = g.rects.find((r) => r.depth === 0)!;
    expect(Math.abs(root.delta!)).toBeLessThan(1e-9);
  });

  it('diffs a profile against itself with zero deltas, and reports unknown frames as zero baseline', async () => {
    const { b } = await demo();
    const same = await b.diffFlame({ baselineId: 1, targetId: 1, widthPx: 500 });
    expect(same.rects.every((r) => Math.abs(r.delta!) < 1e-9)).toBe(true);
    const reversed = await b.diffFlame({ baselineId: 2, targetId: 1, widthPx: 500 });
    expect(reversed.rects.length).toBeGreaterThan(0);
  });
});

describe('MockProfilingBackend: function diff', () => {
  it('marks new, removed, changed and same functions', async () => {
    const { b } = await demo();
    const page = await b.functionDiff({ baselineId: 1, targetId: 2, metric: 'samples', limit: 100 });
    const status = (n: string) => page.rows.find((r) => r.name === n)?.status;
    expect(status('Shop.Orders.AuditLog.Write')).toBe('new');
    expect(status('System.Text.Json.Utf8JsonWriter.WriteString')).toBe('changed');
    // like the engine, a change of the total share alone counts: JsonWriter.Write has no self time but grew
    expect(status('Shop.Orders.Serialization.JsonWriter.Write')).toBe('changed');
    expect(status('Shop.Orders.Program.Main')).toBe('same');
    expect(page.metric).toBe('samples');
    expect(page.baselineTotal).toBe(1200);
    expect(page.targetTotal).toBe(900);
    const reverse = await b.functionDiff({ baselineId: 2, targetId: 1, metric: 'samples', limit: 100 });
    const gone = reverse.rows.find((r) => r.name === 'Shop.Orders.AuditLog.Write')!;
    expect(gone.status).toBe('removed');
    expect(gone.targetFrame).toBeNull();
    expect(page.rows.find((r) => r.name === 'Shop.Orders.AuditLog.Write')!.targetFrame).toBe(19);
  });

  it('sorts by every DiffSort', async () => {
    const { b } = await demo();
    const get = async (sort: 'absDeltaSelf' | 'absDeltaTotal' | 'deltaSelf' | 'deltaTotal' | 'name' | undefined) =>
      (await b.functionDiff({ baselineId: 1, targetId: 2, sort, limit: 100 })).rows;
    const isSorted = <T>(xs: T[], key: (x: T) => number, dir: 1 | -1) => xs.every((x, i) => i === 0 || (key(xs[i - 1]) - key(x)) * dir >= 0);
    expect(isSorted(await get('absDeltaSelf'), (r) => Math.abs(r.deltaSelfPct), 1)).toBe(true);
    expect(isSorted(await get(undefined), (r) => Math.abs(r.deltaSelfPct), 1)).toBe(true);
    expect(isSorted(await get('absDeltaTotal'), (r) => Math.abs(r.deltaTotalPct), 1)).toBe(true);
    expect(isSorted(await get('deltaSelf'), (r) => r.deltaSelfPct, 1)).toBe(true);
    expect(isSorted(await get('deltaTotal'), (r) => r.deltaTotalPct, 1)).toBe(true);
    const byName = names(await get('name'));
    expect(byName).toEqual(byName.slice().sort((x, y) => x.localeCompare(y)));
  });

  it('searches, pages and groups', async () => {
    const { b } = await demo();
    expect(names((await b.functionDiff({ baselineId: 1, targetId: 2, search: 'AUDIT' })).rows)).toEqual(['Shop.Orders.AuditLog.Write']);
    const page = await b.functionDiff({ baselineId: 1, targetId: 2, offset: 1, limit: 2 });
    expect(page.rows).toHaveLength(2);
    expect(page.totalRows).toBeGreaterThan(2);
    const mods = await b.functionDiff({ baselineId: 1, targetId: 2, groupBy: 'module', metric: null });
    expect(names(mods.rows)).toContain('Shop.Orders');
    expect(mods.metric).toBe('timeNs');
    expect(await rejection(b.functionDiff({ baselineId: 1, targetId: 2, limit: 0 }))).toContain('limit');
    expect(await rejection(b.functionDiff({ baselineId: 1, targetId: 2, metric: 'calls' }))).toBe('the profile has no calls metric');
  });
});

describe('MockProfilingBackend: threads', () => {
  it('buckets samples per thread lane, busiest first, with events', async () => {
    const { b } = await demo();
    const t = await b.threads({ profileId: 1, buckets: 10 });
    expect(t.metric).toBe('samples');
    expect(t.fromNs).toBe(0);
    expect(t.toNs).toBe(20_000_000_000);
    expect(t.bucketNs).toBe(2_000_000_000);
    expect(t.startedMs).toBe(ORDERS_STARTED_MS);
    expect(t.threads.map((l) => l.total)).toEqual(t.threads.map((l) => l.total).sort((x, y) => y - x));
    expect(t.threads.every((l) => l.buckets.length === 10)).toBe(true);
    expect(t.threads.reduce((a, l) => a + l.total, 0)).toBe(1200);
    expect(new Set(t.events.map((e) => e.kind))).toEqual(new Set(['gc', 'exception', 'lockWait', 'jit', 'marker']));
    expect(t.events.map((e) => e.startNs)).toEqual(t.events.map((e) => e.startNs).sort((x, y) => x - y));
    expect(t.eventsTruncated).toBe(false);
  });

  it('defaults to 400 buckets and uses the range for the axis', async () => {
    const { b } = await demo();
    const t = await b.threads({ profileId: 1, view: { range: { fromNs: 1_000_000_000, toNs: 2_000_000_000 } } });
    expect(t.threads[0].buckets).toHaveLength(400);
    expect([t.fromNs, t.toNs]).toEqual([1_000_000_000, 2_000_000_000]);
    expect(t.bucketNs).toBe(2_500_000);
    expect(t.events.every((e) => e.startNs + e.durNs >= 1_000_000_000 && e.startNs <= 2_000_000_000)).toBe(true);
  });

  it('validates the bucket count', async () => {
    const { b } = await demo();
    expect(await rejection(b.threads({ profileId: 1, buckets: 0 }))).toBe('buckets must be 1..4000');
    expect(await rejection(b.threads({ profileId: 1, buckets: 4001 }))).toBe('buckets must be 1..4000');
    expect((await b.threads({ profileId: 1, buckets: 4000 })).threads[0].buckets).toHaveLength(4000);
    expect((await b.threads({ profileId: 1, buckets: 1 })).threads[0].buckets).toHaveLength(1);
  });

  it('filters events by kind and thread, truncates at maxEvents, and honours the metric', async () => {
    const { b } = await demo();
    const gc = await b.threads({ profileId: 1, eventKinds: ['gc'] });
    expect(gc.events.length).toBeGreaterThan(0);
    expect(gc.events.every((e) => e.kind === 'gc')).toBe(true);
    expect((await b.threads({ profileId: 1, eventKinds: [] })).events).toEqual([]);
    const only0 = await b.threads({ profileId: 1, view: { threads: [0] } });
    expect(only0.threads.map((l) => l.thread.id)).toEqual([0]);
    expect(only0.events.every((e) => e.thread === 0 || e.thread === null)).toBe(true);
    expect(only0.events.some((e) => e.kind === 'marker')).toBe(true);
    const cut = await b.threads({ profileId: 1, maxEvents: 3 });
    expect(cut.events).toHaveLength(3);
    expect(cut.eventsTruncated).toBe(true);
    const time = await b.threads({ profileId: 1, metric: 'timeNs', buckets: 4 });
    expect(time.metric).toBe('timeNs');
    expect(time.threads.reduce((a, l) => a + l.total, 0)).toBeGreaterThan(1200 * 1_000_000 - 1);
    expect((await b.threads({ profileId: 2 })).events).toEqual([]);
  });
});

describe('MockProfilingBackend: findings', () => {
  it('reports lock contention, exceptions and top self time, critical first', async () => {
    const { b } = await demo();
    const f = await b.findings({ profileId: 1 });
    const kinds = f.map((x) => x.kind);
    expect(kinds).toContain('lockContention');
    expect(kinds).toContain('exceptionHotPath');
    expect(kinds).toContain('gcPressure');
    expect(kinds).toContain('topSelf');
    expect(kinds).not.toContain('regression');
    const rank = { critical: 0, warning: 1, info: 2 };
    expect(f.map((x) => rank[x.severity])).toEqual(f.map((x) => rank[x.severity]).sort());
    const lock = f.find((x) => x.kind === 'lockContention')!;
    expect(lock.evidence).toMatchObject({ view: 'callTree', profileId: 1, baselineId: null, metric: 'timeNs', eventKind: null });
    expect(lock.evidence.frame?.id).toBe(10);
    expect(lock.evidence.path.at(-1)).toBe(10);
    expect(f.find((x) => x.kind === 'gcPressure')!.evidence).toMatchObject({ view: 'timeline', frame: null, path: [], eventKind: 'gc' });
    const tops = f.filter((x) => x.kind === 'topSelf');
    expect(tops.map((t) => t.evidence.view)).toEqual(['hotspots', 'flame'].slice(0, tops.length));
    expect(tops[0].valuePct).toBeGreaterThanOrEqual(tops[1]?.valuePct ?? 0);
  });

  it('adds regressions against a baseline with the baseline id in the evidence', async () => {
    const { b } = await demo();
    const f = await b.findings({ profileId: 2, baselineId: 1 });
    const regressions = f.filter((x) => x.kind === 'regression');
    expect(regressions.length).toBeGreaterThan(0);
    expect(regressions.every((r) => r.severity === 'critical' && r.evidence.view === 'diff' && r.evidence.baselineId === 1 && (r.valuePct ?? 0) > 5)).toBe(true);
    expect(regressions.map((r) => r.title)).toContain('Regression: System.Text.Json.Utf8JsonWriter.WriteString');
    expect(f[0].severity).toBe('critical');
    // CPU-only profile has no exceptions metric: no exception finding
    expect(f.map((x) => x.kind)).not.toContain('exceptionHotPath');
  });

  it('echoes the view range and threads in the evidence, and finds nothing in an empty view', async () => {
    const { b } = await demo();
    const view: ProfileView = { range: { fromNs: 0, toNs: 20_000_000_000 }, threads: [1, 2] };
    const f = await b.findings({ profileId: 1, view });
    expect(f.length).toBeGreaterThan(0);
    expect(f.every((x) => x.evidence.range?.toNs === 20_000_000_000 && x.evidence.threads?.length === 2)).toBe(true);
    const empty = await b.findings({ profileId: 1, view: { range: { fromNs: 0, toNs: -1 } } });
    expect(empty).toEqual([]);
  });

  it('handles a baseline id of 0/null like no baseline', async () => {
    const { b } = await demo();
    const a = await b.findings({ profileId: 1, baselineId: null });
    const z = await b.findings({ profileId: 1, baselineId: 0 });
    expect(z).toEqual(a);
  });
});

describe('MockProfilingBackend: moment', () => {
  it('returns the top functions of every profile that covers the window', async () => {
    const { b } = await demo();
    const m = await b.moment({ fromMs: ORDERS_STARTED_MS + 1000, toMs: ORDERS_STARTED_MS + 5000 });
    expect(m).toHaveLength(1);
    expect(m[0]).toMatchObject({ profileId: 1, name: 'orders.speedscope.json', metric: 'timeNs', measurement: 'sampled' });
    expect(m[0].range).toEqual({ fromNs: 1_000_000_000, toNs: 5_000_000_000 });
    expect(m[0].top).toHaveLength(5);
    expect(m[0].samples).toBeGreaterThan(0);
    expect(m[0].value).toBeGreaterThan(0);
  });

  it('clamps partially overlapping windows to the capture and honours top (max 50)', async () => {
    const { b } = await demo();
    const m = await b.moment({ fromMs: ORDERS_STARTED_MS - 10_000, toMs: ORDERS_STARTED_MS + 100_000, top: 2 });
    expect(m[0].range).toEqual({ fromNs: 0, toNs: 20_000_000_000 });
    expect(m[0].top).toHaveLength(2);
    const many = await b.moment({ fromMs: ORDERS_STARTED_MS, toMs: ORDERS_STARTED_MS + 20_000, top: 500 });
    expect(many[0].top.length).toBeLessThanOrEqual(50);
    expect(many[0].top.length).toBeGreaterThan(5);
  });

  it('returns several profiles when windows overlap them and none outside any capture', async () => {
    const { b } = await demo();
    const both = await b.moment({ fromMs: ORDERS_STARTED_MS, toMs: REGRESSION_STARTED_MS + 15_000 });
    expect(both.map((x) => x.profileId)).toEqual([1, 2]);
    expect(await b.moment({ fromMs: 0, toMs: 1000 })).toEqual([]);
    expect(await b.moment({ fromMs: ORDERS_STARTED_MS + 30_000, toMs: ORDERS_STARTED_MS + 40_000 })).toEqual([]);
    expect(await b.moment({ fromMs: ORDERS_STARTED_MS + 5000, toMs: ORDERS_STARTED_MS + 1000 })).toEqual([]);
  });

  it('skips a window that contains no samples', async () => {
    const { b } = await demo();
    // a 1 ns slice between two samples (samples are 16.7 ms apart)
    const m = await b.moment({ fromMs: ORDERS_STARTED_MS + 0.000001, toMs: ORDERS_STARTED_MS + 0.000002 });
    expect(m).toEqual([]);
  });
});

describe('MockProfilingBackend: heap', () => {
  it('lists types by bytes with shares and totals; sorts by every key', async () => {
    const { b } = await demo();
    const page = await b.heapTypes({ snapshotId: 3 });
    expect(page.hasGraph).toBe(true);
    expect(page.totalCount).toBeGreaterThan(100);
    expect(page.rows.reduce((a, r) => a + r.bytesPct, 0)).toBeCloseTo(100, 5);
    expect(page.rows.reduce((a, r) => a + r.bytes, 0)).toBe(page.totalBytes);
    const bytes = page.rows.map((r) => r.bytes);
    expect(bytes).toEqual(bytes.slice().sort((x, y) => y - x));
    const sorted = async (sort: 'name' | 'count' | 'bytes' | 'retained') => (await b.heapTypes({ snapshotId: 3, sort })).rows;
    const byName = names(await sorted('name'));
    expect(byName).toEqual(byName.slice().sort((x, y) => x.localeCompare(y)));
    const counts = (await sorted('count')).map((r) => r.count);
    expect(counts).toEqual(counts.slice().sort((x, y) => y - x));
    const retained = (await sorted('retained')).map((r) => r.retained ?? 0);
    expect(retained).toEqual(retained.slice().sort((x, y) => y - x));
    const cache = page.rows.find((r) => r.name === 'Shop.Orders.OrderCache')!;
    expect(cache.retained).toBeGreaterThan(cache.bytes);
  });

  it('searches and pages types', async () => {
    const { b } = await demo();
    expect(names((await b.heapTypes({ snapshotId: 3, search: ' orderline ' })).rows)).toEqual(['Shop.Orders.OrderLine']);
    expect((await b.heapTypes({ snapshotId: 3, search: '  ' })).totalRows).toBeGreaterThan(5);
    const all = await b.heapTypes({ snapshotId: 3 });
    expect((await b.heapTypes({ snapshotId: 3, offset: 1, limit: 2 })).rows).toEqual(all.rows.slice(1, 3));
    expect(await rejection(b.heapTypes({ snapshotId: 3, limit: 0 }))).toContain('limit');
  });

  it('diffs two snapshots: growth, new types, sorts, search and totals', async () => {
    const { b } = await demo();
    const d = await b.heapDiff({ baselineId: 3, targetId: 4 });
    const order = d.rows.find((r) => r.name === 'Shop.Orders.Order')!;
    expect(order).toMatchObject({ countBefore: 50, countAfter: 100, countDelta: 50, bytesDelta: 50 * 120, isNew: false, isGone: false });
    const audit = d.rows.find((r) => r.name === 'Shop.Orders.AuditLog')!;
    expect(audit).toMatchObject({ isNew: true, isGone: false, countBefore: 0 });
    const gone = (await b.heapDiff({ baselineId: 4, targetId: 3 })).rows.find((r) => r.name === 'Shop.Orders.AuditLog')!;
    expect(gone).toMatchObject({ isNew: false, isGone: true });
    expect(d.bytesAfter - d.bytesBefore).toBe(d.rows.reduce((a, r) => a + r.bytesDelta, 0));
    expect(d.countAfter - d.countBefore).toBe(d.rows.reduce((a, r) => a + r.countDelta, 0));
    const by = async (sort?: 'bytesDelta' | 'countDelta' | 'name') => (await b.heapDiff({ baselineId: 3, targetId: 4, sort })).rows;
    const bd = (await by()).map((r) => r.bytesDelta);
    expect(bd).toEqual(bd.slice().sort((x, y) => y - x));
    const cd = (await by('countDelta')).map((r) => r.countDelta);
    expect(cd).toEqual(cd.slice().sort((x, y) => y - x));
    const nm = names(await by('name'));
    expect(nm).toEqual(nm.slice().sort((x, y) => x.localeCompare(y)));
    expect(names((await b.heapDiff({ baselineId: 3, targetId: 4, search: 'auditentry' })).rows)).toEqual(['Shop.Orders.AuditEntry']);
    expect((await b.heapDiff({ baselineId: 3, targetId: 4, offset: 1, limit: 1 })).rows).toHaveLength(1);
  });

  it('analyses growth over three snapshots and rejects fewer', async () => {
    const { b } = await demo();
    const g = await b.heapGrowth({ snapshotIds: [3, 4, 5] });
    const order = g.rows.find((r) => r.name === 'Shop.Orders.Order')!;
    expect(order.steady).toBe(true);
    expect(order.bytes).toEqual([50 * 120, 100 * 120, 150 * 120]);
    expect(order.counts).toEqual([50, 100, 150]);
    expect(order.growthBytes).toBe(100 * 120);
    expect(order.slopeBytes).toBe(50 * 120);
    expect(g.steadyCount).toBeGreaterThan(0);
    expect(g.rows[0].steady).toBe(true);
    const flat = g.rows.find((r) => r.name === 'Shop.Orders.Customer')!;
    expect(flat.steady).toBe(false);
    expect(flat.growthBytes).toBe(0);
    const steady = await b.heapGrowth({ snapshotIds: [3, 4, 5], steadyOnly: true });
    expect(steady.rows.every((r) => r.steady)).toBe(true);
    expect(steady.totalRows).toBe(steady.steadyCount);
    expect(names((await b.heapGrowth({ snapshotIds: [3, 4, 5], search: 'ORDERLINE' })).rows)).toEqual(['Shop.Orders.OrderLine']);
    expect((await b.heapGrowth({ snapshotIds: [3, 4, 5], offset: 1, limit: 1 })).rows).toHaveLength(1);
    // a type that grows then shrinks is not steady
    expect(g.rows.find((r) => r.name === 'System.Byte[]')?.steady).toBe(false);
    expect(await rejection(b.heapGrowth({ snapshotIds: [3, 4] }))).toBe('growth needs at least 3 snapshots');
    expect(await rejection(b.heapGrowth({ snapshotIds: [] }))).toContain('at least 3');
    expect(await rejection(b.heapGrowth({ snapshotIds: [3, 4, 1] }))).toContain('is a profile');
  });

  it('browses the dominator tree by retained size', async () => {
    const { b } = await demo();
    const top = await b.heapDominators({ snapshotId: 3 });
    expect(top.totalBytes).toBeGreaterThan(0);
    const retained = top.rows.map((r) => r.retained);
    expect(retained).toEqual(retained.slice().sort((x, y) => y - x));
    expect(top.rows.map((r) => r.typeName)).toContain('Shop.Orders.OrderCache');
    expect(top.rows.map((r) => r.typeName)).not.toContain('(GC roots)');
    const cache = top.rows.find((r) => r.typeName === 'Shop.Orders.OrderCache')!;
    expect(cache.childCount).toBe(1);
    const kids = await b.heapDominators({ snapshotId: 3, parent: cache.object });
    expect(kids.rows.map((r) => r.typeName)).toEqual(['System.Collections.Generic.Dictionary<Int32, Order>']);
    expect(kids.totalRows).toBe(1);
    expect((await b.heapDominators({ snapshotId: 3, offset: 1, limit: 1 })).rows).toEqual([top.rows[1]]);
    const none = await b.heapDominators({ snapshotId: 3, parent: 99_999 });
    expect(none).toMatchObject({ rows: [], totalRows: 0 });
  });

  it('lists instances of a type by retained size', async () => {
    const { b } = await demo();
    const inst = await b.heapInstances({ snapshotId: 3, typeName: 'Shop.Orders.Order' });
    expect(inst.totalRows).toBe(50);
    expect(inst.totalBytes).toBe(50 * 120);
    expect(inst.rows.every((r) => r.typeName === 'Shop.Orders.Order' && r.childCount === 3)).toBe(true);
    const ret = inst.rows.map((r) => r.retained);
    expect(ret).toEqual(ret.slice().sort((x, y) => y - x));
    expect((await b.heapInstances({ snapshotId: 3, typeName: 'Shop.Orders.Order', offset: 49, limit: 5 })).rows).toHaveLength(1);
    expect(await b.heapInstances({ snapshotId: 3, typeName: 'Nope' })).toEqual({ rows: [], totalRows: 0, totalBytes: 0 });
    expect((await b.heapInstances({ snapshotId: 3, typeName: '(GC roots)' })).rows).toEqual([]);
  });

  it('finds the paths to GC roots', async () => {
    const { b } = await demo();
    const cacheId = (await b.heapDominators({ snapshotId: 3 })).rows.find((r) => r.typeName === 'Shop.Orders.OrderCache')!.object;
    // directly referenced by a root
    const direct = await b.heapPaths({ snapshotId: 3, object: cacheId });
    expect(direct).toEqual({ paths: [[{ object: cacheId, typeName: 'Shop.Orders.OrderCache (static OrderCache.Instance)', edge: null }]], unreachable: false, truncated: false });
    // two referrers: the dictionary chain and the customer chain, shortest first
    const order = (await b.heapInstances({ snapshotId: 3, typeName: 'Shop.Orders.Order' })).rows[0].object;
    const paths = await b.heapPaths({ snapshotId: 3, object: order });
    expect(paths.paths).toHaveLength(2);
    expect(paths.paths[0].length).toBeLessThanOrEqual(paths.paths[1].length);
    for (const p of paths.paths) {
      expect(p[0].edge).toBeNull();
      expect(p[0].typeName).toMatch(/\(static /);
      expect(p.at(-1)).toMatchObject({ object: order, typeName: 'Shop.Orders.Order' });
      expect(p.at(-1)!.edge).not.toBeNull();
    }
    expect(paths.paths.flat().map((s) => s.typeName.replace(/ \(.*$/, ''))).toContain('Shop.Orders.Customer');
    const limited = await b.heapPaths({ snapshotId: 3, object: order, maxPaths: 1 });
    expect(limited.paths).toHaveLength(1);
    expect(limited.truncated).toBe(true);
    expect((await b.heapPaths({ snapshotId: 3, object: order, maxPaths: 1000 })).paths).toHaveLength(2);
  });

  it('rejects objects that are not in the snapshot', async () => {
    const { b } = await demo();
    expect(await rejection(b.heapPaths({ snapshotId: 3, object: 0 }))).toBe('object 0 is not in this snapshot');
    expect(await rejection(b.heapPaths({ snapshotId: 3, object: 999_999 }))).toBe('object 999999 is not in this snapshot');
    expect(await rejection(b.heapPaths({ snapshotId: 3, object: -1 }))).toContain('is not in this snapshot');
  });
});

describe('MockProfilingBackend: empty views', () => {
  const empty: ProfileView = { range: { fromNs: 0, toNs: -1 } };
  it('answers with empty, zero-share results instead of failing', async () => {
    const { b } = await demo();
    const hot = await b.hotspots({ profileId: 1, view: empty });
    expect(hot).toMatchObject({ rows: [], totalRows: 0, samplesInView: 0 });
    const tree = await b.tree({ profileId: 1, view: empty });
    expect(tree).toMatchObject({ nodes: [], totalChildren: 0 });
    const flame = await b.flame({ profileId: 1, widthPx: 100, view: empty });
    expect(flame).toMatchObject({ rects: [], focusValue: 0, matchedPct: 0, truncated: false });
    const diff = await b.diffFlame({ baselineId: 1, targetId: 2, widthPx: 100, view: empty });
    expect(diff.rects).toEqual([]);
    const fd = await b.functionDiff({ baselineId: 1, targetId: 2, view: empty });
    expect(fd).toMatchObject({ rows: [], totalRows: 0, baselineTotal: 0 });
    expect((await b.threads({ profileId: 1, view: empty })).threads).toEqual([]);
  });

  it('computes shares for a profile whose metric totals are zero-free but a baseline lacks the path', async () => {
    const { b } = await demo();
    const g = await b.diffFlame({ baselineId: 2, targetId: 1, widthPx: 1000, view: { range: { fromNs: 0, toNs: 1_000_000_000 } } });
    expect(g.rects.length).toBeGreaterThan(0);
    expect(g.rects.every((r) => Number.isFinite(r.delta!) && Number.isFinite(r.baselineValue!))).toBe(true);
  });
});

describe('mock profiling data', () => {
  it('is deterministic', () => {
    expect(ordersProfile()).toEqual(ordersProfile());
    expect(regressionProfile().samples).toEqual(regressionProfile().samples);
    expect(heapSnapshot(2)).toEqual(heapSnapshot(2));
  });
  it('grows the heap snapshots and introduces AuditLog from the second', () => {
    const types = (n: number) => heapSnapshot(n).objects.map((o) => o.type);
    expect(types(1)).not.toContain('Shop.Orders.AuditLog');
    expect(types(2)).toContain('Shop.Orders.AuditLog');
    expect(types(3).filter((t) => t === 'Shop.Orders.AuditEntry')).toHaveLength(20);
    expect(types(3).filter((t) => t === 'Shop.Orders.Order')).toHaveLength(150);
  });
});
