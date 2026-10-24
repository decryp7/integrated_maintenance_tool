import { inject } from '@angular/core';
import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { setError, setLoaded, setLoading, withRequestStatus } from '../shared/store/with-request-status';
import { BrowserRuntimeBackend } from './browser-runtime-backend';
import { ResearchProfileStatus } from './browser.types';

interface ResearchProfileState {
  status: ResearchProfileStatus | null;
}

function message(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return $localize`:@@browser.profile.failed:The browser did not answer.`;
}

/** The kept research profile as Settings ▸ Web browser shows it (ADR 0058); the engine decides whether it may be cleared. */
export const ResearchProfileStore = signalStore(
  { providedIn: 'root' },
  withState<ResearchProfileState>({ status: null }),
  withRequestStatus(),
  withMethods((store, backend = inject(BrowserRuntimeBackend)) => {
    const run = async (f: () => Promise<ResearchProfileStatus>): Promise<void> => {
      patchState(store, setLoading());
      try {
        patchState(store, { status: await f() }, setLoaded());
      } catch (e) {
        patchState(store, setError(message(e)));
      }
    };
    return {
      load: () => run(() => backend.getResearchProfile()),
      /** Clears it; a refusal (a research runs, a browser uses it) shows as the error, with the status kept. */
      clear: () => (store.isLoading() ? Promise.resolve() : run(() => backend.clearResearchProfile())),
    };
  }),
);
