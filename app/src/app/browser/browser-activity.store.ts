import { inject } from '@angular/core';
import { patchState, signalStore, withHooks, withMethods, withState } from '@ngrx/signals';
import { BrowserBackend } from './browser-backend';
import { AiDriver, BrowserActivityKind, BrowserEvent } from './browser.types';

interface BrowserActivityState {
  /** Who drives the AI's browser window now (a chat's agent mode, or research); null: nobody. */
  driver: AiDriver | null;
  /** A `driving` event came: it wins over a slower first answer of `browser_driving`. */
  driverHeard: boolean;
  /** The engine's progress line ("Reading 3 of 5 results"), when it sent one. */
  text: string | null;
  activity: BrowserActivityKind | null;
  /** The site the window is on. */
  host: string | null;
}

const idle: Omit<BrowserActivityState, 'driver' | 'driverHeard'> = { text: null, activity: null, host: null };

/**
 * What the AI's browser window is doing, for the status chip: who drives it, what it reads or searches, and on which
 * site. The shell owns the window and reports (`browser_driving`, then the `driving`, `status` and `activity` events);
 * this keeps the last report and forgets it when the AI lets go.
 */
export const BrowserActivityStore = signalStore(
  { providedIn: 'root' },
  withState<BrowserActivityState>({ driver: null, driverHeard: false, ...idle }),
  withMethods((store, backend = inject(BrowserBackend)) => ({
    onEvent(e: BrowserEvent): void {
      if (e.type === 'driving') patchState(store, { driver: e.by, driverHeard: true, ...(e.by ? {} : idle) });
      else if (e.type === 'status') patchState(store, { text: e.text });
      else if (e.type === 'activity') patchState(store, { activity: e.activity, host: e.host });
    },

    /** Who drives now (a failure leaves the chip hidden; the events still arrive). */
    async load(): Promise<void> {
      const d = await backend.driving().catch(() => undefined);
      if (d !== undefined && !store.driverHeard()) patchState(store, { driver: d });
    },
  })),
  withHooks((store, backend = inject(BrowserBackend)) => {
    let off: (() => void) | undefined;
    let destroyed = false;
    return {
      onInit() {
        void backend
          .onEvent((e) => store.onEvent(e))
          .then((u) => (destroyed ? u() : (off = u)))
          .catch(() => undefined);
        void store.load();
      },
      onDestroy() {
        destroyed = true;
        off?.();
      },
    };
  }),
);

export type BrowserActivityStoreType = InstanceType<typeof BrowserActivityStore>;
