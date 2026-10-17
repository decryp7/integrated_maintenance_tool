import { TailReport } from '../core/api.types';
import { report } from '../../testing/live-helpers';
import { RefreshCoalescer, Throttle } from './coalescer';
import { changesSources, isStructural, mergeReports, planRefresh, unknownChange } from './live-report';

describe('live reports', () => {
  it('merges counts, sets, the earliest id change and the newest time', () => {
    const a = report({
      appended: 2,
      sourcesChanged: [1],
      idsChangedFrom: 50,
      newestTs: 10,
      instancesChanged: ['a'],
      rotated: [{ sourceId: 1, from: 'x', to: 'x.1' }],
      elapsedMs: 1,
      fileStates: [
        { sourceId: 1, state: { kind: 'rotated' } },
        { sourceId: 2, state: { kind: 'tailing' } },
      ],
    });
    const b = report({ appended: 3, updated: 1, sourcesChanged: [1, 2], idsChangedFrom: 20, newestTs: 5, instancesChanged: ['a', 'b'], evicted: 4, alertsRaised: 1, elapsedMs: 2, added: [3], truncated: [2], deleted: [4], reimported: [5], fileStates: [{ sourceId: 2, state: { kind: 'truncated' } }] });
    expect(mergeReports(a, b)).toEqual({
      appended: 5,
      updated: 1,
      sourcesChanged: [1, 2],
      added: [3],
      rotated: [{ sourceId: 1, from: 'x', to: 'x.1' }],
      truncated: [2],
      deleted: [4],
      reimported: [5],
      evicted: 4,
      idsChangedFrom: 20,
      newestTs: 10,
      instancesChanged: ['a', 'b'],
      fileStates: [
        { sourceId: 1, state: { kind: 'rotated' } },
        { sourceId: 2, state: { kind: 'truncated' } },
      ],
      alertsRaised: 1,
      elapsedMs: 3,
    });
    expect(mergeReports(report(), report({ newestTs: 3, idsChangedFrom: 1 }))).toMatchObject({ newestTs: 3, idsChangedFrom: 1 });
    expect(mergeReports(report({ newestTs: 3, idsChangedFrom: 1 }), report())).toMatchObject({ newestTs: 3, idsChangedFrom: 1 });
  });

  it('tells appends from structural changes', () => {
    expect(isStructural(report({ appended: 10 }))).toBe(false);
    expect(isStructural(report({ rotated: [{ sourceId: 1, from: 'a', to: 'b' }], added: [2] }))).toBe(false);
    for (const p of [{ idsChangedFrom: 0 }, { evicted: 1 }, { updated: 1 }, { truncated: [1] }, { deleted: [1] }, { reimported: [1] }]) {
      expect(isStructural(report(p))).toBe(true);
    }
    expect(isStructural(unknownChange())).toBe(true);
    expect(changesSources(report({ appended: 3 }))).toBe(false);
    for (const p of [{ added: [1] }, { rotated: [{ sourceId: 1, from: 'a', to: 'b' }] }, { truncated: [1] }, { deleted: [1] }, { reimported: [1] }]) {
      expect(changesSources(report(p))).toBe(true);
    }
  });

  it('plans the smallest refresh: follow or count new events, nothing for an empty report', () => {
    expect(planRefresh(report({ appended: 3 }), true)).toEqual({ summary: true, charts: true, facets: true, grid: 'follow', status: true });
    expect(planRefresh(report({ appended: 3 }), false).grid).toBe('count');
    expect(planRefresh(report({ appended: 3, idsChangedFrom: 2 }), false).grid).toBe('follow');
    expect(planRefresh(report(), true)).toEqual({ summary: false, charts: false, facets: false, grid: 'none', status: true });
    expect(planRefresh(report({ added: [4] }), false)).toMatchObject({ summary: true, grid: 'count' });
  });
});

describe('RefreshCoalescer', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('turns a burst of 200 events into at most one refresh per interval, never overlapping', async () => {
    const runs: TailReport[] = [];
    let active = 0;
    let maxActive = 0;
    const c = new RefreshCoalescer(async (r) => {
      active++;
      maxActive = Math.max(maxActive, active);
      runs.push(r);
      await new Promise((res) => setTimeout(res, 40));
      active--;
    }, 250);
    // 200 events over one second (one every 5 ms), as a busy log would produce.
    for (let i = 0; i < 200; i++) {
      c.push(report({ appended: 1, newestTs: i }));
      await vi.advanceTimersByTimeAsync(5);
    }
    await vi.advanceTimersByTimeAsync(1000);
    expect(runs.length).toBeLessThanOrEqual(6);
    expect(runs.length).toBeGreaterThanOrEqual(4);
    expect(maxActive).toBe(1);
    expect(runs.reduce((a, r) => a + r.appended, 0)).toBe(200);
    expect(runs.at(-1)!.newestTs).toBe(199);
    expect(c.runs).toBe(runs.length);
    expect(c.hasPending).toBe(false);
  });

  it('merges reports that arrive while a refresh runs into one follow-up', async () => {
    let release!: () => void;
    const seen: number[] = [];
    const c = new RefreshCoalescer(async (r) => {
      seen.push(r.appended);
      if (seen.length === 1) await new Promise<void>((res) => (release = res));
    }, 0);
    c.push(report({ appended: 1 }));
    await vi.advanceTimersByTimeAsync(0);
    c.push(report({ appended: 2 }));
    c.push(report({ appended: 3 }));
    await vi.advanceTimersByTimeAsync(100);
    expect(seen).toEqual([1]);
    release();
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toEqual([1, 5]);
  });

  it('keeps going after a failed refresh, and drops everything once stopped', async () => {
    const run = vi.fn().mockRejectedValueOnce(new Error('boom')).mockResolvedValue(undefined);
    const c = new RefreshCoalescer(run, 10);
    c.push(report({ appended: 1 }));
    await vi.advanceTimersByTimeAsync(0);
    c.push(report({ appended: 2 }));
    await vi.advanceTimersByTimeAsync(20);
    expect(run).toHaveBeenCalledTimes(2);
    c.push(report({ appended: 3 }));
    c.stop();
    c.push(report({ appended: 4 }));
    await vi.advanceTimersByTimeAsync(50);
    expect(run).toHaveBeenCalledTimes(2);
    c.resume();
    c.push(report({ appended: 5 }));
    await vi.advanceTimersByTimeAsync(50);
    expect(run).toHaveBeenLastCalledWith(expect.objectContaining({ appended: 5 }));
  });
});

describe('Throttle', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('fires at once, then at most once per interval with a trailing call', async () => {
    const fire = vi.fn();
    const t = new Throttle(fire, 1000);
    t.request();
    expect(fire).toHaveBeenCalledTimes(1);
    t.request();
    t.request();
    expect(fire).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(fire).toHaveBeenCalledTimes(2);
    t.request();
    t.cancel();
    await vi.advanceTimersByTimeAsync(2000);
    expect(fire).toHaveBeenCalledTimes(2);
    t.request();
    expect(fire).toHaveBeenCalledTimes(3);
  });
});
