import { Debouncer, FrameScheduler, FrameThrottle } from './throttle';
import { bucketsForWidth, MAX_BUCKETS, MIN_BUCKETS } from './bucket-count';

/** Frames the test advances by hand. */
function frames() {
  const queue = new Map<number, () => void>();
  let next = 0;
  const scheduler: FrameScheduler = {
    request: (cb) => (queue.set(++next, cb), next),
    cancel: (h) => queue.delete(h as number),
  };
  const tick = () => {
    const due = [...queue.values()];
    queue.clear();
    due.forEach((cb) => cb());
  };
  return { scheduler, tick, size: () => queue.size };
}

describe('FrameThrottle', () => {
  it('runs only the latest task, once per frame', () => {
    const f = frames();
    const t = new FrameThrottle(f.scheduler);
    const seen: number[] = [];
    for (let i = 0; i < 30; i++) t.run(() => seen.push(i));
    expect(f.size()).toBe(1);
    expect(t.pending).toBe(true);
    f.tick();
    expect(seen).toEqual([29]);
    expect(t.pending).toBe(false);
    t.run(() => seen.push(30));
    f.tick();
    expect(seen).toEqual([29, 30]);
  });

  it('cancel drops the scheduled task', () => {
    const f = frames();
    const t = new FrameThrottle(f.scheduler);
    const task = vi.fn();
    t.run(task);
    t.cancel();
    f.tick();
    expect(task).not.toHaveBeenCalled();
    t.cancel();
  });

  it('uses animation frames by default', async () => {
    const t = new FrameThrottle();
    const task = vi.fn();
    t.run(task);
    await vi.waitFor(() => expect(task).toHaveBeenCalledOnce());
  });
});

describe('Debouncer', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('runs the latest task once calls have stopped', () => {
    const d = new Debouncer(50);
    const seen: number[] = [];
    for (let i = 0; i < 5; i++) {
      d.run(() => seen.push(i));
      vi.advanceTimersByTime(40);
    }
    expect(seen).toEqual([]);
    expect(d.pending).toBe(true);
    vi.advanceTimersByTime(10);
    expect(seen).toEqual([4]);
    expect(d.pending).toBe(false);
    d.run(() => seen.push(9));
    d.cancel();
    vi.advanceTimersByTime(100);
    expect(seen).toEqual([4]);
  });
});

describe('bucketsForWidth', () => {
  it('gives about one bucket per 2 px in steps of 50, bounded', () => {
    expect(bucketsForWidth(800)).toBe(400);
    // A few pixels of resizing keep the same count (no re-query).
    expect(bucketsForWidth(790)).toBe(400);
    expect(bucketsForWidth(830)).toBe(400);
    expect(bucketsForWidth(860)).toBe(450);
    expect(bucketsForWidth(10)).toBe(MIN_BUCKETS);
    expect(bucketsForWidth(0)).toBe(MIN_BUCKETS);
    expect(bucketsForWidth(Number.NaN)).toBe(MIN_BUCKETS);
    expect(bucketsForWidth(1e6)).toBe(MAX_BUCKETS);
  });
});
