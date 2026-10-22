import { inject } from '@angular/core';
import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { BrowserRuntimeBackend } from './browser-runtime-backend';
import { RuntimeStatus } from './browser.types';

interface BrowserRuntimeState {
  status: RuntimeStatus | null;
  /** The download or the removal is running (the download can take minutes). */
  busy: 'install' | 'remove' | null;
  error: string | null;
}

function message(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return $localize`:@@browser.runtime.failed:The browser did not answer.`;
}

/** Which Chromium research and the browser modes use, as Settings ▸ Web browser shows it; the engine decides. */
export const BrowserRuntimeStore = signalStore(
  { providedIn: 'root' },
  withState<BrowserRuntimeState>({ status: null, busy: null, error: null }),
  withMethods((store, backend = inject(BrowserRuntimeBackend)) => {
    const run = async (busy: 'install' | 'remove' | null, f: () => Promise<RuntimeStatus>): Promise<void> => {
      patchState(store, { busy, error: null });
      try {
        patchState(store, { status: await f() });
      } catch (e) {
        patchState(store, { error: message(e) });
      } finally {
        patchState(store, { busy: null });
      }
    };
    return {
      load: () => run(null, () => backend.getRuntimeStatus()),
      /** Downloads the pinned Chromium (also updates a downloaded older one). */
      install: () => (store.busy() ? Promise.resolve() : run('install', () => backend.installRuntime())),
      remove: () => (store.busy() ? Promise.resolve() : run('remove', () => backend.removeRuntime())),
    };
  }),
);
