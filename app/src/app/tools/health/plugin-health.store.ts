import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { LifecycleBackend } from '../../core/tools-backend';
import { PluginInfo } from '../../core/api.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { LifecycleService } from '../../lifecycle/lifecycle.service';
import { errorMessage } from '../tool-errors';

/** A plugin row: the engine's report with what the user changed since this run started. */
export interface PluginRow extends PluginInfo {
  /** The user enabled or disabled it; takes effect after a restart. */
  pendingEnabled: boolean | null;
}

interface HealthState {
  /** Quarantine resets applied in this run. */
  reset: string[];
  /** Enable/disable choices waiting for a restart (id -> enabled). */
  pending: Record<string, boolean>;
  busy: string | null;
  error: string | null;
}

/** Plugin status, failures and the actions the engine offers: reset a quarantine, enable or disable at next start. */
export const PluginHealthStore = signalStore(
  { providedIn: 'root' },
  withState<HealthState>({ reset: [], pending: {}, busy: null, error: null }),
  withComputed((store, workspace = inject(WorkspaceStore)) => ({
    plugins: computed<PluginRow[]>(() =>
      (workspace.summary()?.plugins ?? []).map((p): PluginRow => {
        const wasReset = store.reset().includes(p.id);
        return {
          ...p,
          status: wasReset ? 'active' : p.status,
          failures: wasReset ? 0 : p.failures,
          pendingEnabled: p.id in store.pending() ? store.pending()[p.id] : null,
        };
      }),
    ),
    needsRestart: computed(() => Object.keys(store.pending()).length > 0),
  })),
  withMethods((store, backend = inject(LifecycleBackend), life = inject(LifecycleService)) => {
    const run = async (label: string, op: () => Promise<void>) => {
      patchState(store, { busy: label, error: null });
      try {
        await op();
      } catch (e) {
        patchState(store, { error: errorMessage(e) });
      } finally {
        patchState(store, { busy: null });
      }
    };
    return {
      /** Lifts a quarantine now (the plugin is called again). */
      reset: (id: string) =>
        run('Resetting…', async () => {
          await backend.pluginReset(id);
          patchState(store, (s) => ({ reset: [...s.reset, id] }));
        }),
      /** Saves the choice for the next start. */
      setEnabled: (id: string, enabled: boolean) =>
        run(enabled ? 'Enabling…' : 'Disabling…', async () => {
          await backend.pluginSetEnabled(id, enabled);
          patchState(store, (s) => ({ pending: { ...s.pending, [id]: enabled } }));
        }),
      /** Relaunches so enable/disable choices apply; the session is reopened. */
      restart: () => life.restartKeepingSession(),
      dismissError: () => patchState(store, { error: null }),
    };
  }),
);
