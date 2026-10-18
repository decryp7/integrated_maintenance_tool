import { inject } from '@angular/core';
import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { RunManifest, RunSummary } from '../core/agent.types';
import { errorMessage } from '../tools/tool-errors';
import { SchedulesBackend } from './schedules-backend';
import { SchedulesStore } from './schedules.store';

interface RunsState {
  outputDir: string | null;
  runs: RunSummary[];
  truncated: number;
  /** The run whose details are shown, and its manifest. */
  selected: string | null;
  manifest: RunManifest | null;
  loading: boolean;
  error: string | null;
  notice: string | null;
}

/** The runs the agent wrote into its output folder. Uses the config path of the schedules editor. */
export const RunsStore = signalStore(
  { providedIn: 'root' },
  withState<RunsState>({ outputDir: null, runs: [], truncated: 0, selected: null, manifest: null, loading: false, error: null, notice: null }),
  withMethods((store, backend = inject(SchedulesBackend), schedules = inject(SchedulesStore)) => {
    const path = () => schedules.path().trim() || undefined;
    return {
      refresh: async () => {
        patchState(store, { loading: true, error: null });
        try {
          const listing = await backend.runs(path());
          patchState(store, { outputDir: listing.outputDir, runs: listing.runs, truncated: listing.truncated });
        } catch (e) {
          patchState(store, { error: errorMessage(e), runs: [], outputDir: null });
        } finally {
          patchState(store, { loading: false });
        }
      },
      /** Shows (or hides, when shown) a run's manifest. */
      toggle: async (id: string) => {
        if (store.selected() === id) {
          patchState(store, { selected: null, manifest: null });
          return;
        }
        patchState(store, { selected: id, manifest: null, error: null });
        try {
          const m = await backend.manifest(id, path());
          if (store.selected() === id) patchState(store, { manifest: m });
        } catch (e) {
          patchState(store, { error: errorMessage(e) });
        }
      },
      open: async (id: string) => {
        patchState(store, { error: null, notice: null });
        try {
          await backend.openRun(id, path());
          patchState(store, { notice: $localize`:@@schedules.runs.opened:Opened ${id}:id: in a new window.` });
        } catch (e) {
          patchState(store, { error: errorMessage(e) });
        }
      },
      dismiss: () => patchState(store, { error: null, notice: null }),
    };
  }),
);
