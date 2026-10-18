import { computed, inject } from '@angular/core';
import {
  patchState,
  signalStore,
  withComputed,
  withHooks,
  withMethods,
  withState,
} from '@ngrx/signals';
import { AiBackend, toAiError } from '../ai-backend';
import { AiRuntimeEvent, AiRuntimeOverview } from '../ai.types';
import {
  isTerminal,
  orderedBuilds,
  RuntimeRunning,
  RuntimeTerminal,
  suggestedDownload,
} from './runtime-view';

interface AiRuntimeState {
  overview: AiRuntimeOverview | null;
  loading: boolean;
  /** The last failed request (listing, delete, choose). */
  error: string | null;
  /** The install in progress (started in any window): its build and latest stage. */
  job: { id: string; stage: RuntimeRunning | null; cancelling: boolean } | null;
  /** How the last install ended. */
  outcome: RuntimeTerminal | null;
  /** A delete or a choice is being applied. */
  busy: boolean;
}

/**
 * The inference runtimes (llama-server builds, ADR 0009): the engine's overview of what is installed and
 * works here, and the download in progress. Shared by the set-up wizard and the model manager, so a
 * download started in one shows in the other; the engine streams progress to every window.
 */
export const AiRuntimeStore = signalStore(
  { providedIn: 'root' },
  withState<AiRuntimeState>({
    overview: null,
    loading: false,
    error: null,
    job: null,
    outcome: null,
    busy: false,
  }),
  withComputed((s) => ({
    builds: computed(() => orderedBuilds(s.overview())),
    /** The build to offer first (nothing usable is installed yet). */
    suggested: computed(() => suggestedDownload(s.overview())),
    needsDownload: computed(() => !!s.overview()?.needsDownload),
    installing: computed(() => !!s.job()),
  })),
  withMethods((store, backend = inject(AiBackend)) => {
    const fail = (e: unknown) => patchState(store, { error: toAiError(e).message });
    const methods = {
      async load(): Promise<AiRuntimeOverview | null> {
        patchState(store, { loading: true, error: null });
        try {
          const overview = await backend.runtimes();
          patchState(store, { overview });
          return overview;
        } catch (e) {
          fail(e);
          return null;
        } finally {
          patchState(store, { loading: false });
        }
      },
      async install(id: string): Promise<void> {
        if (store.job()) return;
        patchState(store, {
          job: { id, stage: null, cancelling: false },
          outcome: null,
          error: null,
        });
        try {
          await backend.runtimeInstall(id);
        } catch (e) {
          const err = toAiError(e);
          patchState(store, {
            job: null,
            outcome: { stage: 'failed', id, code: err.code, message: err.message },
          });
        }
      },
      async cancel(): Promise<void> {
        const i = store.job();
        if (!i) return;
        patchState(store, { job: { ...i, cancelling: true } });
        try {
          await backend.runtimeCancel();
        } catch {
          /* the terminal event still ends the install */
        }
      },
      async remove(tag: string, id: string): Promise<boolean> {
        patchState(store, { busy: true, error: null });
        try {
          await backend.runtimeDelete(tag, id);
          await methods.load();
          return true;
        } catch (e) {
          fail(e);
          return false;
        } finally {
          patchState(store, { busy: false });
        }
      },
      async choose(id: string): Promise<boolean> {
        patchState(store, { busy: true, error: null });
        try {
          await backend.runtimeChoose(id);
          await methods.load();
          return true;
        } catch (e) {
          fail(e);
          return false;
        } finally {
          patchState(store, { busy: false });
        }
      },
      /** An `imt://ai-runtime` event (from any window). */
      onEvent(e: AiRuntimeEvent): void {
        if (isTerminal(e)) {
          patchState(store, { job: null, outcome: e });
          void methods.load();
          return;
        }
        const i = store.job();
        patchState(store, {
          job: { id: e.id, stage: e, cancelling: i?.id === e.id ? i.cancelling : false },
        });
      },
      dismissOutcome(): void {
        patchState(store, { outcome: null });
      },
    };
    return methods;
  }),
  withHooks({
    onInit(store, backend = inject(AiBackend)) {
      void backend.onRuntime((e) => store.onEvent(e)).catch(() => undefined);
    },
  }),
);
