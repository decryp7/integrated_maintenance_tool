import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withState } from '@ngrx/signals';
import { AiBackend, toAiError } from '../ai-backend';
import { AiUpdateKind, AiUpdatesStatus } from '../ai.types';

interface AiUpdatesState {
  status: AiUpdatesStatus | null;
  /** The last failed request (listing, check, update, skip). */
  error: string | null;
  /** A request from this window is running. */
  busy: boolean;
}

/**
 * The runtime and model update checks as the engine reports them (`ai_updates_*`, `imt://ai-updates`):
 * policies, the last check and its quiet failure, and the updates on offer. The engine decides everything;
 * this only asks and shows.
 */
export const AiUpdatesStore = signalStore(
  { providedIn: 'root' },
  withState<AiUpdatesState>({ status: null, error: null, busy: false }),
  withComputed((s) => ({
    kinds: computed(() => s.status()?.kinds ?? []),
    checking: computed(() => (s.status()?.kinds ?? []).some((k) => k.checking)),
    /** Updates that are not skipped. */
    offered: computed(() => (s.status()?.kinds ?? []).flatMap((k) => k.updates).filter((u) => !u.skipped)),
  })),
  withMethods((store, backend = inject(AiBackend)) => {
    const run = async (f: () => Promise<AiUpdatesStatus>): Promise<boolean> => {
      patchState(store, { busy: true, error: null });
      try {
        patchState(store, { status: await f() });
        return true;
      } catch (e) {
        patchState(store, { error: toAiError(e).message });
        return false;
      } finally {
        patchState(store, { busy: false });
      }
    };
    return {
      load: () => run(() => backend.updatesStatus()),
      check: (kind?: AiUpdateKind | null) => run(() => backend.updatesCheck(kind ?? null)),
      apply: (key: string) => run(() => backend.updatesApply(key)),
      skip: (key: string) => run(() => backend.updatesSkip(key)),
      later: (key: string) => run(() => backend.updatesLater(key)),
      /** An `imt://ai-updates` status from the engine. */
      onStatus(status: AiUpdatesStatus): void {
        patchState(store, { status });
      },
    };
  }),
  withHooks({
    onInit(store, backend = inject(AiBackend)) {
      void backend.onUpdates((s) => store.onStatus(s)).catch(() => undefined);
    },
  }),
);
