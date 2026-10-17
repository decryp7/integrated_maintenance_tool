import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withState } from '@ngrx/signals';
import { Logger } from '../core/logger';
import { AiBackend, toAiError } from './ai-backend';
import { AiStatus, SetupEvent } from './ai.types';

interface AiStatusState {
  status: AiStatus | null;
  /** The last set-up or activation event (any window may have started it). */
  setup: SetupEvent | null;
  error: string | null;
}

const TERMINAL = new Set(['ready', 'failed', 'cancelled']);

/**
 * Whether the local AI assistant can be used in this run, and what its engine is doing. Small and eager:
 * every AI entry point reads `available` to hide itself (safe mode, no worker binary, AI off) before the
 * lazily loaded AI views are needed.
 */
export const AiStatusStore = signalStore(
  { providedIn: 'root' },
  withState<AiStatusState>({ status: null, setup: null, error: null }),
  withComputed((s) => ({
    /** AI can be used in this run (it may still need a set-up). */
    available: computed(() => !!s.status()?.available),
    /** A model is set up (loaded or loading on first use). */
    hasModel: computed(() => !!s.status()?.available && !['noModel', 'unavailable'].includes(s.status()?.state ?? 'unavailable')),
    needsSetup: computed(() => s.status()?.state === 'noModel'),
    /** A set-up or activation is running (in this or another window). */
    setupRunning: computed(() => !!s.status()?.setupRunning || (!!s.setup() && !TERMINAL.has(s.setup()!.stage))),
  })),
  withMethods((store, backend = inject(AiBackend), logger = inject(Logger)) => ({
    async refresh(): Promise<AiStatus | null> {
      try {
        const status = await backend.status();
        patchState(store, { status, error: null });
        return status;
      } catch (e) {
        const err = toAiError(e);
        patchState(store, { status: null, error: err.message });
        logger.warn('ai', 'AI status unavailable', { Code: err.code });
        return null;
      }
    },
    setStatus(status: AiStatus): void {
      patchState(store, { status, error: null });
    },
    noteSetup(setup: SetupEvent): void {
      patchState(store, { setup });
    },
  })),
  withHooks({
    onInit(store, backend = inject(AiBackend)) {
      void store.refresh();
      void backend.onStatus((s) => store.setStatus(s)).catch(() => undefined);
      void backend
        .onSetup((e) => {
          store.noteSetup(e);
          if (TERMINAL.has(e.stage)) void store.refresh();
        })
        .catch(() => undefined);
    },
  }),
);
