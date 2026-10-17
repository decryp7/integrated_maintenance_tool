import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { CollectBackend } from '../../core/tools-backend';
import { WorkspaceStore } from '../../core/workspace.store';
import { LifecycleService } from '../../lifecycle/lifecycle.service';
import { LiveTail } from '../../workspace/live-tail';
import { TOOL_IDS } from '../../core/tools.types';
import { ToolCatalog } from '../tool-catalog.store';
import { errorMessage } from '../tool-errors';
import { DEFAULT_MONITOR_DRAFT, MonitorDraft, TargetKind, toMonitorConfig, validateMonitor } from './monitor-model';

interface MonitorState {
  draft: MonitorDraft;
  running: boolean;
  /** Folder of the current or last run. */
  dir: string | null;
  busy: string | null;
  error: string | null;
  notice: string | null;
}

/** The pseudo-field the process monitor sets on events it could not fully read. */
export const ACCESS_DENIED_FIELD = 'AccessDenied';

/** State and actions of the process monitor panel. The engine samples; the UI configures and follows. */
export const MonitorStore = signalStore(
  { providedIn: 'root' },
  withState<MonitorState>({ draft: DEFAULT_MONITOR_DRAFT, running: false, dir: null, busy: null, error: null, notice: null }),
  withComputed((store, workspace = inject(WorkspaceStore), life = inject(LifecycleService), catalog = inject(ToolCatalog)) => ({
    /** The tools plugin serves the monitor now (not inactive, quarantined, disabled or in safe mode). */
    canRun: computed(() => catalog.usable(TOOL_IDS.processMonitor)),
    problems: computed(() => validateMonitor(store.draft())),
    /** Performance counters (and elevation) are a Windows feature. */
    countersAvailable: life.isWindows,
    /** Events in the workspace that carry the AccessDenied field: some processes need administrator rights. */
    accessDeniedSeen: computed(() => workspace.summary()?.fields.find((f) => f.name === ACCESS_DENIED_FIELD)?.count ?? 0),
  })),
  withMethods((store, backend = inject(CollectBackend), live = inject(LiveTail), workspace = inject(WorkspaceStore)) => {
    const run = async (label: string, op: () => Promise<void>): Promise<void> => {
      patchState(store, { busy: label, error: null, notice: null });
      try {
        await op();
      } catch (e) {
        patchState(store, { error: errorMessage(e) });
      } finally {
        patchState(store, { busy: null });
      }
    };

    return {
      patchDraft: (patch: Partial<MonitorDraft>) => patchState(store, (s) => ({ draft: { ...s.draft, ...patch } })),
      addTarget: () => patchState(store, (s) => ({ draft: { ...s.draft, targets: [...s.draft.targets, { kind: 'name' as TargetKind, value: '' }] } })),
      removeTarget: (index: number) =>
        patchState(store, (s) => {
          const targets = s.draft.targets.filter((_, i) => i !== index);
          return { draft: { ...s.draft, targets: targets.length ? targets : DEFAULT_MONITOR_DRAFT.targets } };
        }),
      setTarget: (index: number, patch: { kind?: TargetKind; value?: string }) =>
        patchState(store, (s) => ({ draft: { ...s.draft, targets: s.draft.targets.map((t, i) => (i === index ? { ...t, ...patch } : t)) } })),
      dismissError: () => patchState(store, { error: null }),

      /** Reads whether a run is in progress (the monitor outlives the panel). */
      refresh: () =>
        run('Checking…', async () => {
          const status = await backend.monitorStatus();
          patchState(store, { running: status.running, dir: status.dir ?? store.dir() });
        }),

      /** Starts sampling, opens the output folder and follows it live. */
      start: () =>
        run('Starting…', async () => {
          const d = store.draft();
          const dir = await backend.monitorStart(toMonitorConfig(d), d.dir.trim() || undefined);
          patchState(store, { running: true, dir });
          await workspace.openPaths([dir]);
          await live.start();
          patchState(store, { notice: 'Monitoring started. The results appear in the workspace as they are written.' });
        }),

      stop: () =>
        run('Stopping…', async () => {
          const dir = await backend.monitorStop();
          patchState(store, { running: false, dir: dir ?? store.dir(), notice: 'Monitoring stopped.' });
        }),
    };
  }),
);
