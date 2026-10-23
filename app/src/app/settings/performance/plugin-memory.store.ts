import { inject } from '@angular/core';
import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { PerformanceBackend } from './performance-backend';
import { ProcessMemory } from './performance.types';
import { setError, setLoaded, withRequestStatus } from '../../shared/store/with-request-status';

interface PluginMemoryState {
  /** The capped processes as the engine reports them. */
  processes: ProcessMemory[];
  /** A cap to propose for a plugin ("Raise the cap…" of a memory message): shown filled in, not applied. */
  requested: { pluginId: string; mb: number } | null;
}

const message = (e: unknown): string => (e instanceof Error ? e.message : typeof e === 'string' ? e : $localize`:@@performance.memory.failed:The memory caps could not be read.`);

/** The caps and last reported memory use of Debrief's processes, and the plugin caps set in the Plugins panel. */
export const PluginMemoryStore = signalStore(
  { providedIn: 'root' },
  withState<PluginMemoryState>({ processes: [], requested: null }),
  withRequestStatus(),
  withMethods((store, backend = inject(PerformanceBackend)) => ({
    async load(): Promise<void> {
      try {
        patchState(store, { processes: await backend.memory() }, setLoaded());
      } catch (e) {
        patchState(store, setError(message(e)));
      }
    },
    /** The process row of plugin `id`, if the engine has one. */
    plugin(id: string): ProcessMemory | undefined {
      return store.processes().find((p) => p.process.kind === 'plugin' && p.process.pluginId === id);
    },
    /** Sets (`mb`) or removes (`null`) plugin `id`'s cap; resolves with the engine's refusal, if any. */
    async setCap(id: string, mb: number | null): Promise<string | null> {
      try {
        patchState(store, { processes: await backend.setPluginMemory(id, mb) }, setLoaded());
        if (store.requested()?.pluginId === id) patchState(store, { requested: null });
        return null;
      } catch (e) {
        return message(e);
      }
    },
    request(pluginId: string, mb: number): void {
      patchState(store, { requested: { pluginId, mb } });
    },
  })),
);
