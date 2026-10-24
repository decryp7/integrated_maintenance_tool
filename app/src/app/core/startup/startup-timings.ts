import { afterNextRender, effect, EnvironmentProviders, inject, Injector, makeEnvironmentProviders, provideAppInitializer } from '@angular/core';
import { WorkspaceDataStore } from '../workspace/workspace-data.store';
import { WorkspaceStatusStore } from '../workspace/status.store';
import { StartupBackend } from './startup-backend';

/** The start-up marks the page reports to Rust's start-up timings (rule 16); Rust records its own marks. */
export type PageMark = 'firstPaint' | 'workspaceLoaded' | 'firstInteractive';

/** One mark of the `startup_timings` reply: milliseconds since the process started. */
export interface StartupMarkAt {
  mark: 'processStart' | 'setupDone' | 'windowCreated' | 'pageLoad' | 'windowReady' | PageMark;
  ms: number;
}

/** The `startup_timings` reply: the marks reached so far, `setup`'s steps, and whether every mark was reached. */
export interface StartupReport {
  marks: StartupMarkAt[];
  steps: { step: string; ms: number }[];
  complete: boolean;
}

/** What the page reads its clock from (tests pass fakes). */
export interface PageClock {
  /** Wall-clock milliseconds now. */
  now(): number;
  /** The wall-clock time of the page's first contentful paint, or null when the browser has no paint timing. */
  firstPaint(): number | null;
}

export const BROWSER_CLOCK: PageClock = {
  now: () => Date.now(),
  firstPaint: () => {
    const entry = globalThis.performance?.getEntriesByName?.('first-contentful-paint')[0];
    return entry ? performance.timeOrigin + entry.startTime : null;
  },
};

/**
 * Reports the main window's start-up marks: its first paint once Angular rendered the shell (the browser's first
 * contentful paint when it knows it, which may be the splash), the first workspace load (a summary or an error), and
 * the first frame painted after it (the user can work). Each is sent once, as a wall-clock time; failures are ignored
 * (the timings are a measurement, never part of the start).
 */
export function provideStartupTimings(clock: PageClock = BROWSER_CLOCK): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideAppInitializer(() => {
      const injector = inject(Injector);
      const data = inject(WorkspaceDataStore);
      const status = inject(WorkspaceStatusStore);
      const send = (mark: PageMark, at: number) => void injector.get(StartupBackend).reportMark(mark, at).catch(() => undefined);
      afterNextRender(() => send('firstPaint', clock.firstPaint() ?? clock.now()), { injector });
      let loaded = false;
      const watch = effect(
        () => {
          if (loaded || (data.summary() === null && status.error() === null)) return;
          // Side effect only (IPC): the moment the first load finished, then the first frame after it.
          loaded = true;
          send('workspaceLoaded', clock.now());
          afterNextRender(() => send('firstInteractive', clock.now()), { injector });
          watch.destroy();
        },
        { injector },
      );
    }),
  ]);
}
