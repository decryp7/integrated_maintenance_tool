import { afterEach, describe, expect, it, vi } from 'vitest';
import { MockClock, MockTimer, systemMockClock } from './mock-clock';
import { MockBackend } from './mock-backend';
import { MockImportSim } from './mock-import';
import { MockAlertMutes } from './mock-live';
import { tinyDataset } from '../../../testing/helpers';

/** A clock that only moves when told: every mock timer becomes a step the test takes. */
class ManualClock implements MockClock {
  private t = 1_000_000;
  private seq = 0;
  private readonly timers = new Map<number, { at: number; fn: () => void; every: number | null }>();

  now = () => this.t;
  elapsed = () => this.t;
  setTimeout = (fn: () => void, ms: number) => this.add(fn, ms, null);
  setInterval = (fn: () => void, ms: number) => this.add(fn, ms, Math.max(1, ms));
  clearTimeout = (t: MockTimer | undefined) => void this.timers.delete(t as unknown as number);
  clearInterval = (t: MockTimer | undefined) => void this.timers.delete(t as unknown as number);
  sleep = (ms: number) => new Promise<void>((resolve) => this.setTimeout(resolve, ms));

  private add(fn: () => void, ms: number, every: number | null): MockTimer {
    const id = ++this.seq;
    this.timers.set(id, { at: this.t + Math.max(0, ms), fn, every });
    return id as unknown as MockTimer;
  }

  /** Moves time on by `ms`, firing every timer due on the way (and letting the promises they settle run). */
  async advance(ms: number): Promise<void> {
    const end = this.t + ms;
    for (;;) {
      const next = [...this.timers.entries()].filter(([, t]) => t.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
      if (!next) break;
      const [id, timer] = next;
      this.t = timer.at;
      if (timer.every === null) this.timers.delete(id);
      else timer.at += timer.every;
      timer.fn();
      for (let i = 0; i < 20; i++) await Promise.resolve();
    }
    this.t = end;
  }
}

describe('mock clock', () => {
  afterEach(() => vi.useRealTimers());

  it('the system clock follows the page timers at each call (fake timers included)', async () => {
    vi.useFakeTimers({ now: 5_000 });
    expect(systemMockClock.now()).toBe(5_000);
    const fired: string[] = [];
    systemMockClock.setTimeout(() => fired.push('timeout'), 100);
    const every = systemMockClock.setInterval(() => fired.push('interval'), 40);
    const dropped = systemMockClock.setTimeout(() => fired.push('never'), 50);
    systemMockClock.clearTimeout(dropped);
    const slept = systemMockClock.sleep(200).then(() => fired.push('slept'));
    await vi.advanceTimersByTimeAsync(100);
    systemMockClock.clearInterval(every);
    await vi.advanceTimersByTimeAsync(200);
    await slept;
    expect(fired).toEqual(['interval', 'interval', 'timeout', 'slept']);
    expect(systemMockClock.now()).toBe(5_300);
    expect(systemMockClock.elapsed()).toBeGreaterThan(0);
  });

  it('a slow import runs in the injected clock: nothing happens until time moves, then exactly the files due', async () => {
    const clock = new ManualClock();
    const sim = new MockImportSim(1_000, clock);
    const read: string[] = [];
    const run = sim.run(['a', 'b', 'c'], (f) => read.push(f));
    await clock.advance(999);
    expect(read).toEqual([]);
    await clock.advance(1);
    expect(read).toEqual(['a']);
    expect(sim.progress()).toMatchObject({ running: true, done: 1, elapsedMs: 1_000 });
    sim.cancel();
    await clock.advance(1_000);
    expect(await run).toEqual({ read: ['a', 'b'], skipped: ['c'] });
  });

  it('live tail ticks and alert mutes run on the backend clock', async () => {
    const clock = new ManualClock();
    const b = new MockBackend({ latency: false, dataset: tinyDataset, pluginMarkerKinds: [], liveRate: 20, liveAlertMs: 1_000, liveRotateMs: 0, clock });
    const alerts: string[] = [];
    await b.onLiveAlert((a) => alerts.push(a.kind));
    await b.setLive(true);
    await clock.advance(999);
    expect(alerts).toEqual([]);
    await clock.advance(1);
    expect(alerts).toEqual(['errorSpike']);
    // A mute is timed on the same clock: it holds the kind back (the next error spike, due in 3 s) until it ends.
    await b.muteAlerts({ kind: 'errorSpike' }, clock.now() + 3_500);
    await clock.advance(3_000);
    expect(alerts).toEqual(['errorSpike', 'rule', 'silence']);
    await clock.advance(3_000);
    expect(alerts).toEqual(['errorSpike', 'rule', 'silence', 'rule', 'silence', 'errorSpike']);
    await b.setLive(false);
    await clock.advance(5_000);
    expect(alerts).toHaveLength(6);
    const mutes = new MockAlertMutes(clock.now);
    mutes.mute({ kind: 'silence' }, clock.now() + 10);
    expect(mutes.covers({ kind: 'silence', rule: null })).toBe(true);
    await clock.advance(10);
    expect(mutes.covers({ kind: 'silence', rule: null })).toBe(false);
  });
});
