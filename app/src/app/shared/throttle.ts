/** Schedules a callback for the next frame; returns a handle for `cancel`. */
export type FrameScheduler = { request(cb: () => void): unknown; cancel(handle: unknown): void };

/** The browser's animation frames, or a 16 ms timer where there are none. */
export const ANIMATION_FRAMES: FrameScheduler = {
  request: (cb) => (typeof requestAnimationFrame === 'function' ? requestAnimationFrame(() => cb()) : setTimeout(cb, 16)),
  cancel: (h) => (typeof cancelAnimationFrame === 'function' ? cancelAnimationFrame(h as number) : clearTimeout(h as ReturnType<typeof setTimeout>)),
};

/**
 * Coalesces work to at most once per animation frame: `run` keeps only the latest task, which runs on the next frame.
 * Pointer moves fire far more often than the screen refreshes; publishing each would re-render for nothing.
 */
export class FrameThrottle {
  private task: (() => void) | null = null;
  private handle: unknown = null;

  constructor(private readonly frames: FrameScheduler = ANIMATION_FRAMES) {}

  /** Runs `task` on the next frame, instead of any task scheduled earlier in this frame. */
  run(task: () => void): void {
    this.task = task;
    if (this.handle !== null) return;
    this.handle = this.frames.request(() => {
      this.handle = null;
      const t = this.task;
      this.task = null;
      t?.();
    });
  }

  /** Drops the scheduled task (e.g. the pointer left: a late move must not publish over the leave). */
  cancel(): void {
    this.task = null;
    if (this.handle !== null) this.frames.cancel(this.handle);
    this.handle = null;
  }

  get pending(): boolean {
    return this.task !== null;
  }
}

/**
 * Runs the latest task once calls have stopped for `ms` (trailing debounce). For work that only matters once a burst
 * settles: a splitter drag, fast scrolling, holding an arrow key.
 */
export class Debouncer {
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(private readonly ms: number) {}

  run(task: () => void): void {
    this.cancel();
    this.timer = setTimeout(() => {
      this.timer = null;
      task();
    }, this.ms);
  }

  cancel(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }

  get pending(): boolean {
    return this.timer !== null;
  }
}
