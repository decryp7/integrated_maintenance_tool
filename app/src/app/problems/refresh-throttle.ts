/** The timers a `RefreshThrottle` uses (injectable for tests). */
export interface ThrottleClock {
  now(): number;
  setTimeout(run: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

const REAL_CLOCK: ThrottleClock = {
  now: () => Date.now(),
  setTimeout: (run, ms) => setTimeout(run, ms),
  clearTimeout: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

/**
 * Coalesces change notifications into at most one run per interval: the first change after a quiet interval runs at
 * once, later ones collapse into a single run at the end of the interval. The interval is read on every change, so a
 * caller can make it shorter while a view is visible (and call `reschedule` when that changes).
 */
export class RefreshThrottle {
  private last = Number.NEGATIVE_INFINITY;
  private timer: unknown = null;
  private disposed = false;

  constructor(
    private readonly run: () => void,
    private readonly intervalMs: () => number,
    private readonly clock: ThrottleClock = REAL_CLOCK,
  ) {}

  /** Something changed: run now, or once at the end of the current interval. */
  request(): void {
    if (this.disposed || this.timer !== null) return;
    const wait = this.last + this.intervalMs() - this.clock.now();
    if (wait <= 0) this.fire();
    else this.timer = this.clock.setTimeout(() => this.fire(), wait);
  }

  /** The interval changed (e.g. the view became visible): a pending run moves to the new interval's end. */
  reschedule(): void {
    if (this.timer === null) return;
    this.clock.clearTimeout(this.timer);
    this.timer = null;
    this.request();
  }

  /** A pending run is waiting. */
  get pending(): boolean {
    return this.timer !== null;
  }

  dispose(): void {
    this.disposed = true;
    if (this.timer !== null) this.clock.clearTimeout(this.timer);
    this.timer = null;
  }

  private fire(): void {
    this.timer = null;
    this.last = this.clock.now();
    this.run();
  }
}
