/**
 * The one clock every mock backend's timer runs on (browser development and tests only; the real backends never
 * use it). Mock features driven by time (live tail ticks and alerts, slow imports, simulated latency, update
 * checks) all go through it: each mock takes a `clock` option (default {@link systemMockClock}) and hands it to the
 * helpers it creates, so a unit test can pass its own, and Playwright's `page.clock` (which patches the page's
 * timers before the app starts) drives every mock feature at once in simulated time.
 */
export type MockTimer = ReturnType<typeof globalThis.setTimeout>;

export interface MockClock {
  /** Wall-clock milliseconds (like `Date.now()`). */
  now(): number;
  /** A monotonic reading for measuring durations (like `performance.now()`). */
  elapsed(): number;
  setTimeout(fn: () => void, ms: number): MockTimer;
  clearTimeout(timer: MockTimer | undefined): void;
  setInterval(fn: () => void, ms: number): MockTimer;
  clearInterval(timer: MockTimer | undefined): void;
  /** Resolves after `ms`. */
  sleep(ms: number): Promise<void>;
}

/**
 * The page's own timers, looked up at every call (never captured when the module loads), so whatever the page's
 * globals are at that moment (real, or Playwright's or Vitest's fake timers) is what runs.
 */
export const systemMockClock: MockClock = {
  now: () => Date.now(),
  elapsed: () => globalThis.performance?.now() ?? Date.now(),
  setTimeout: (fn, ms) => globalThis.setTimeout(fn, ms),
  clearTimeout: (t) => globalThis.clearTimeout(t),
  setInterval: (fn, ms) => globalThis.setInterval(fn, ms),
  clearInterval: (t) => globalThis.clearInterval(t),
  sleep: (ms) => new Promise<void>((resolve) => globalThis.setTimeout(resolve, ms)),
};
