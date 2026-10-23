import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { PerfmonBackend } from '../../core/tools-backend';
import { CollectorAction, CollectorStatus, CounterPreset, GeneratedCollector } from '../../core/tools.types';
import { Platform } from '../../core/platform';
import { WorkspaceStore } from '../../core/workspace.store';
import { LifecycleService } from '../../lifecycle/lifecycle.service';
import { plural } from '../../i18n/locale';
import { LiveTail } from '../../workspace/live-tail';
import { TOOL_IDS } from '../../core/tools.types';
import { OverwriteGuard } from '../../ui/overwrite-guard';
import { ToolCatalog } from '../tool-catalog.store';
import { errorMessage } from '../tool-errors';
import { addProcess, DEFAULT_DRAFT, PerfmonDraft, toggleCounter, togglePreset, toTemplate, validateDraft } from './perfmon-model';

interface PerfmonState {
  presets: CounterPreset[];
  draft: PerfmonDraft;
  generated: GeneratedCollector | null;
  status: CollectorStatus | null;
  busy: string | null;
  error: string | null;
  notice: string | null;
  liveTail: boolean;
}

/** The busy label of a collector-set action. */
function controlBusy(action: CollectorAction): string {
  switch (action) {
    case 'query':
      return $localize`:@@tools.perfmon.busy.query:Query…`;
    case 'start':
      return $localize`:@@tools.perfmon.busy.start:Start…`;
    case 'stop':
      return $localize`:@@tools.perfmon.busy.stop:Stop…`;
    case 'delete':
      return $localize`:@@tools.perfmon.busy.delete:Delete…`;
  }
}

const XML_FORMATS = [{ name: $localize`:@@tools.perfmon.xmlFormat:Data Collector Set XML`, extension: 'xml' }];

/** State and actions of the Performance Monitor wizard. The engine builds the XML and runs logman. */
export const PerfmonStore = signalStore(
  { providedIn: 'root' },
  withState<PerfmonState>({ presets: [], draft: DEFAULT_DRAFT, generated: null, status: null, busy: null, error: null, notice: null, liveTail: true }),
  withComputed((store, life = inject(LifecycleService), catalog = inject(ToolCatalog)) => ({
    /** Collector sets can only be managed on Windows; elsewhere the wizard generates XML and instructions. */
    onWindows: life.isWindows,
    /** Collector sets can be managed here: Windows, and the tools plugin serves them. */
    canManage: computed(() => life.isWindows() && catalog.usable(TOOL_IDS.perfmonSets)),
    /** Templates can be generated (the tools plugin serves them). */
    canGenerate: computed(() => catalog.usable(TOOL_IDS.perfmonTemplates)),
    problems: computed(() => validateDraft(store.draft())),
  })),
  withMethods((store, backend = inject(PerfmonBackend), live = inject(LiveTail), workspace = inject(WorkspaceStore), platform = inject(Platform), guard = inject(OverwriteGuard)) => {
    const template = () => toTemplate(store.draft(), store.presets());

    /** Runs an operation with the busy label; failures become `error`. */
    const run = async (label: string, op: () => Promise<void>): Promise<boolean> => {
      patchState(store, { busy: label, error: null, notice: null });
      try {
        await op();
        return true;
      } catch (e) {
        patchState(store, { error: errorMessage(e) });
        return false;
      } finally {
        patchState(store, { busy: null });
      }
    };

    return {
      async loadPresets(): Promise<void> {
        if (store.presets().length) return;
        await run($localize`:@@tools.perfmon.busy.loading:Loading presets…`, async () => {
          const presets = await backend.perfmonPresets();
          patchState(store, (s) => {
            // The first load selects the CPU and memory presets, like the engine's default template.
            const pick = presets.filter((p) => p.id === 'cpu' || p.id === 'memory').flatMap((p) => p.counters.map((c) => c.path));
            return { presets, draft: s.draft.counters.length ? s.draft : { ...s.draft, counters: pick } };
          });
        });
      },
      patchDraft: (patch: Partial<PerfmonDraft>) => patchState(store, (s) => ({ draft: { ...s.draft, ...patch }, generated: null })),
      addProcess: (input: string) => patchState(store, (s) => ({ draft: { ...s.draft, processes: addProcess(s.draft.processes, input) }, generated: null })),
      removeProcess: (name: string) => patchState(store, (s) => ({ draft: { ...s.draft, processes: s.draft.processes.filter((p) => p !== name) }, generated: null })),
      toggleCounter: (path: string) => patchState(store, (s) => ({ draft: { ...s.draft, counters: toggleCounter(s.draft.counters, path) }, generated: null })),
      togglePreset: (preset: CounterPreset) => patchState(store, (s) => ({ draft: { ...s.draft, counters: togglePreset(preset, s.draft.counters) }, generated: null })),
      setLiveTail: (liveTail: boolean) => patchState(store, { liveTail }),
      dismissError: () => patchState(store, { error: null }),

      /** Builds the XML and the logman commands. */
      generate: () =>
        run($localize`:@@tools.perfmon.busy.generating:Generating…`, async () => {
          patchState(store, { generated: await backend.perfmonGenerate(template()) });
        }),

      /** Asks for a file and has the engine write the XML there (an existing file only after confirming). */
      async saveXml(): Promise<void> {
        const path = await platform.pickExportPath(`${store.draft().name.trim() || 'Debrief'}.xml`, XML_FORMATS);
        if (!path) return;
        await run($localize`:@@tools.perfmon.busy.saving:Saving…`, async () => {
          const generated = await guard.write(path, (overwrite) => backend.perfmonGenerate(template(), path, overwrite));
          if (generated) patchState(store, { generated, notice: $localize`:@@tools.perfmon.notice.saved:Saved ${path}:path:` });
        });
      },

      /** Creates the collector set in Performance Monitor and starts it. */
      installAndStart: () =>
        run($localize`:@@tools.perfmon.busy.installing:Installing…`, async () => {
          const installed = await backend.perfmonInstall(template());
          patchState(store, { status: installed });
          patchState(store, { status: await backend.perfmonControl(installed.name, 'start') });
        }),

      control: (action: CollectorAction) =>
        run(controlBusy(action), async () => {
          const name = store.status()?.name ?? store.draft().name.trim();
          const next = await backend.perfmonControl(name, action);
          patchState(store, { status: next, notice: action === 'delete' ? $localize`:@@tools.perfmon.notice.deleted:Deleted ${name}:name:` : null });
        }),

      /** Opens the collector's output files in the workspace, optionally following them live. */
      importResults: () =>
        run($localize`:@@tools.perfmon.busy.importing:Importing results…`, async () => {
          const name = store.status()?.name ?? store.draft().name.trim();
          const files = await backend.perfmonOutputs(name);
          if (!files.length) {
            patchState(store, { notice: $localize`:@@tools.perfmon.notice.noFiles:The collector has not written any files yet.` });
            return;
          }
          await workspace.openPaths(files);
          if (workspace.error()) return;
          if (store.liveTail()) await live.start();
          const n = files.length;
          const withTail = store.liveTail();
          const notice = withTail
            ? plural(n, {
                one: $localize`:@@tools.perfmon.notice.openedLive.one:Opened ${n}:count: file with live tail`,
                other: $localize`:@@tools.perfmon.notice.openedLive.other:Opened ${n}:count: files with live tail`,
              })
            : plural(n, {
                one: $localize`:@@tools.perfmon.notice.opened.one:Opened ${n}:count: file`,
                other: $localize`:@@tools.perfmon.notice.opened.other:Opened ${n}:count: files`,
              });
          patchState(store, { notice });
        }),
    };
  }),
);
