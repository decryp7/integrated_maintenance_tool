import { TailReport } from '../core/api.types';
import { mergeReports } from './live-report';

/** At most this many UI refreshes per second from live tail (the engine itself emits about four). */
export const MIN_REFRESH_INTERVAL_MS = 250;

/**
 * Coalesces live reports into refreshes: reports arriving while a refresh runs, or sooner than
 * `minIntervalMs` after the previous one started, are merged into a single follow-up refresh. A burst of
 * any size therefore costs at most one refresh per interval, and refreshes never overlap.
 */
export class RefreshCoalescer {
  private pending: TailReport | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private running = false;
  private lastStart = Number.NEGATIVE_INFINITY;
  private stopped = false;
  /** Number of refreshes started (for diagnostics and tests). */
  runs = 0;

  constructor(
    private readonly run: (report: TailReport) => Promise<void>,
    private readonly minIntervalMs = MIN_REFRESH_INTERVAL_MS,
    private readonly now: () => number = () => Date.now(),
  ) {}

  /** Queues a report; a refresh follows as soon as the rate limit allows. */
  push(report: TailReport): void {
    if (this.stopped) return;
    this.pending = this.pending ? mergeReports(this.pending, report) : report;
    this.schedule();
  }

  /** Drops what is pending and ignores further reports until `push` is allowed again by `resume`. */
  stop(): void {
    this.stopped = true;
    this.pending = null;
    clearTimeout(this.timer);
    this.timer = undefined;
  }

  resume(): void {
    this.stopped = false;
  }

  /** True while a report waits for its refresh. */
  get hasPending(): boolean {
    return this.pending !== null;
  }

  private schedule(): void {
    if (this.running || this.timer !== undefined || !this.pending) return;
    const wait = Math.max(0, this.lastStart + this.minIntervalMs - this.now());
    this.timer = setTimeout(() => {
      this.timer = undefined;
      void this.flush();
    }, wait);
  }

  private async flush(): Promise<void> {
    const report = this.pending;
    if (!report || this.stopped) return;
    this.pending = null;
    this.running = true;
    this.lastStart = this.now();
    this.runs++;
    try {
      await this.run(report);
    } catch {
      // The refresh reports its own failures; the next report simply tries again.
    } finally {
      this.running = false;
      this.schedule();
    }
  }
}

/**
 * Leading-and-trailing throttle: `fire` runs at once when it has not run for `intervalMs`, otherwise once at
 * the end of the interval, however many times `request` was called meanwhile.
 */
export class Throttle {
  private last = Number.NEGATIVE_INFINITY;
  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor(
    private readonly fire: () => void,
    private readonly intervalMs: number,
    private readonly now: () => number = () => Date.now(),
  ) {}

  request(): void {
    if (this.timer !== undefined) return;
    const wait = this.last + this.intervalMs - this.now();
    if (wait <= 0) {
      this.last = this.now();
      this.fire();
      return;
    }
    this.timer = setTimeout(() => {
      this.timer = undefined;
      this.last = this.now();
      this.fire();
    }, wait);
  }

  cancel(): void {
    clearTimeout(this.timer);
    this.timer = undefined;
  }
}
