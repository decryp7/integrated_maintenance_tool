import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { Playbook, PlaybookLibrary, PlaybookRecord, PlaybookRun, PlaybookStatus, PlaybookStep, PlaybookStepResult } from '../../core/api.types';
import { failureText } from '../../core/cmd-error';
import { WorkspaceStore } from '../../core/workspace.store';
import { PlaybookBackend } from '../investigate-backend';
import { newPlaybook, newRunId, recordOf } from './playbook-model';
import { setError, setLoaded, setLoading, withRequestStatus } from '../../shared/store/with-request-status';

interface PlaybookState {
  library: PlaybookLibrary;
  /** The playbook shown (a saved one or a template). */
  selectedId: string | null;
  /** The runs of the project (what is saved in it). */
  runs: PlaybookRun[];
  activeRunId: string | null;
  /** The step being run or recorded (its buttons wait). */
  busyStep: string | null;
  /** Why the last save, import or export failed (the engine's message). */
  actionError: string | null;
  notice: string | null;
  /** The engine's warnings about saved runs it could not take as they were. */
  restoreWarnings: string[];
  /** Results of steps run in this window (tables and sample events are not saved), by `runId/stepId`. */
  results: Record<string, PlaybookStepResult>;
  /** Notes typed before the step has a result, by `runId/stepId`. */
  drafts: Record<string, string>;
  /** The playbook being edited (a copy), and whether it is not in the library yet. */
  editing: Playbook | null;
  editingIsNew: boolean;
}

const EMPTY_LIBRARY: PlaybookLibrary = { playbooks: [], templates: [], warnings: [] };
const RECORD_VERSION = 1;

const key = (runId: string, stepId: string) => `${runId}/${stepId}`;

/** A result the engine has to record for a step that was done by hand (or whose result is no longer in memory). */
function plainResult(status: PlaybookStatus, summary: string): PlaybookStepResult {
  return { status, summary, count: null, sampleIds: [], columns: [], rows: [] };
}

let runSeq = 0;

/** The playbook library, the project's runs and the active run. The engine validates and computes; this keeps the state. */
export const PlaybookStore = signalStore(
  { providedIn: 'root' },
  withRequestStatus(),
  withState<PlaybookState>({
    library: EMPTY_LIBRARY,
    selectedId: null,
    runs: [],
    activeRunId: null,
    busyStep: null,
    actionError: null,
    notice: null,
    restoreWarnings: [],
    results: {},
    drafts: {},
    editing: null,
    editingIsNew: false,
  }),
  withComputed((store) => ({
    selected: computed<Playbook | null>(() => {
      const id = store.selectedId();
      const lib = store.library();
      return lib.playbooks.find((p) => p.id === id) ?? lib.templates.find((p) => p.id === id) ?? null;
    }),
    selectedIsTemplate: computed(() => {
      const id = store.selectedId();
      return !store.library().playbooks.some((p) => p.id === id) && store.library().templates.some((p) => p.id === id);
    }),
    activeRun: computed<PlaybookRun | null>(() => store.runs().find((r) => r.id === store.activeRunId()) ?? null),
    /** What the project saves: null while there are no runs. */
    record: computed<PlaybookRecord | null>(() => (store.runs().length ? { version: RECORD_VERSION, runs: store.runs() } : null)),
  })),
  withComputed((store) => ({
    /** The runs of the selected playbook, newest first. */
    selectedRuns: computed(() => {
      const id = store.selectedId();
      return store
        .runs()
        .filter((r) => r.playbook.id === id)
        .sort((a, b) => b.startedAt - a.startedAt);
    }),
  })),
  withMethods((store, backend = inject(PlaybookBackend), workspace = inject(WorkspaceStore)) => {
    /** Replaces a run (the engine's returned copy) in the list. */
    const replaceRun = (run: PlaybookRun) => patchState(store, { runs: store.runs().map((r) => (r.id === run.id ? run : r)) });

    const guarded = async <T>(work: () => Promise<T>): Promise<T | null> => {
      patchState(store, { actionError: null });
      try {
        return await work();
      } catch (e) {
        patchState(store, { actionError: failureText(e) });
        return null;
      }
    };

    const load = async () => {
      patchState(store, setLoading());
      try {
        patchState(store, { library: await backend.library() }, setLoaded());
      } catch (e) {
        patchState(store, setError(failureText(e)));
      }
    };

    /** Stores one step's result in the active run. */
    const record = async (step: PlaybookStep, result: PlaybookStepResult, note: string): Promise<boolean> => {
      const run = store.activeRun();
      if (!run) return false;
      patchState(store, { busyStep: step.id, actionError: null });
      try {
        const updated = await backend.record(run, step.id, result, note);
        replaceRun(updated);
        patchState(store, { results: { ...store.results(), [key(run.id, step.id)]: result } });
        return true;
      } catch (e) {
        patchState(store, { actionError: failureText(e) });
        return false;
      } finally {
        patchState(store, { busyStep: null });
      }
    };

    const noteOf = (run: PlaybookRun, stepId: string) => recordOf(run, stepId)?.note ?? store.drafts()[key(run.id, stepId)] ?? '';

    return {
      load,

      select: (id: string | null) => patchState(store, { selectedId: id, activeRunId: null, actionError: null, editing: null }),

      /** Opens the editor on a new playbook. */
      startNew() {
        const taken = new Set([...store.library().playbooks, ...store.library().templates].map((p) => p.id));
        patchState(store, { editing: newPlaybook(taken), editingIsNew: true, activeRunId: null, actionError: null });
      },
      /** Opens the editor on a copy of a saved playbook. */
      startEdit: (playbook: Playbook) => patchState(store, { editing: structuredClone(playbook), editingIsNew: false, activeRunId: null, actionError: null }),
      cancelEdit: () => patchState(store, { editing: null, actionError: null }),

      /** Copies a template into the library (under its own id) and selects the copy. */
      async saveCopyOfTemplate(template: Playbook): Promise<Playbook | null> {
        const saved = await guarded(() => backend.save(structuredClone(template)));
        if (saved) {
          await load();
          patchState(store, { selectedId: saved.id, activeRunId: null });
        }
        return saved;
      },

      /** Saves (creates or replaces) a playbook; the engine validates and its message is kept for the editor. */
      async save(playbook: Playbook): Promise<Playbook | null> {
        const saved = await guarded(() => backend.save(playbook));
        if (saved) {
          await load();
          patchState(store, { selectedId: saved.id, editing: null });
        }
        return saved;
      },

      async remove(id: string): Promise<boolean> {
        const removed = await guarded(() => backend.remove(id));
        if (removed === null) return false;
        if (store.selectedId() === id) patchState(store, { selectedId: null, activeRunId: null });
        await load();
        return removed;
      },

      async import(path: string): Promise<Playbook | null> {
        const imported = await guarded(async () => backend.save(await backend.importFile(path)));
        if (imported) {
          await load();
          patchState(store, { selectedId: imported.id, activeRunId: null });
        }
        return imported;
      },

      async export(playbook: Playbook, path: string): Promise<boolean> {
        const done = await guarded(async () => {
          await backend.exportFile(playbook, path);
          return true;
        });
        patchState(store, { notice: done ? $localize`:@@playbooks.exported:Exported "${playbook.title}:title:".` : null });
        return done === true;
      },

      /** Starts a run of the playbook as it is now (the run keeps its own copy). */
      async startRun(playbook: Playbook): Promise<PlaybookRun | null> {
        const run = await guarded(() => backend.start(playbook, newRunId(Date.now(), ++runSeq)));
        if (run) patchState(store, { runs: [...store.runs(), run], activeRunId: run.id, selectedId: playbook.id });
        return run;
      },

      openRun(id: string) {
        const run = store.runs().find((r) => r.id === id);
        if (run) patchState(store, { activeRunId: id, selectedId: run.playbook.id });
      },

      closeRun: () => patchState(store, { activeRunId: null }),

      deleteRun(id: string) {
        patchState(store, { runs: store.runs().filter((r) => r.id !== id), activeRunId: store.activeRunId() === id ? null : store.activeRunId() });
      },

      /** Runs a step against the open data and records the result. A filter also becomes the workspace's query. */
      async runStep(step: PlaybookStep): Promise<PlaybookStepResult | null> {
        const run = store.activeRun();
        if (!run || store.busyStep()) return null;
        patchState(store, { busyStep: step.id, actionError: null });
        let result: PlaybookStepResult;
        try {
          result = await backend.runStep(step);
        } catch (e) {
          patchState(store, { actionError: failureText(e), busyStep: null });
          return null;
        }
        if (step.action.type === 'filter') workspace.patchQuery({ ql: step.action.ql });
        const ok = await record(step, result, noteOf(run, step.id));
        return ok ? result : null;
      },

      /** Marks a step done or skipped by hand. */
      recordManual(step: PlaybookStep, status: 'pass' | 'skipped', note?: string): Promise<boolean> {
        const run = store.activeRun();
        if (!run) return Promise.resolve(false);
        const summary = status === 'pass' ? $localize`:@@playbooks.doneByHand:Done by hand.` : $localize`:@@playbooks.skippedByHand:Skipped.`;
        return record(step, plainResult(status, summary), note ?? noteOf(run, step.id));
      },

      /** Saves a note with a step; before the step has a result the note waits and is recorded with it. */
      async setNote(step: PlaybookStep, note: string): Promise<boolean> {
        const run = store.activeRun();
        if (!run) return false;
        const rec = recordOf(run, step.id);
        if (!rec) {
          patchState(store, { drafts: { ...store.drafts(), [key(run.id, step.id)]: note } });
          return true;
        }
        const result = store.results()[key(run.id, step.id)] ?? { ...plainResult(rec.status, rec.summary), count: rec.count, sampleIds: rec.sampleIds };
        return record(step, result, note);
      },

      resultOf: (runId: string, stepId: string): PlaybookStepResult | null => store.results()[key(runId, stepId)] ?? null,
      noteFor: (run: PlaybookRun, stepId: string): string => noteOf(run, stepId),

      /** Applies a step's query as the workspace's query and selects the first sample event. */
      showEvents(ql: string | null, sampleIds: readonly number[]) {
        if (ql) workspace.patchQuery({ ql });
        const first = sampleIds[0];
        if (first !== undefined) workspace.jumpTo(first);
      },

      /** The saved runs, checked by the engine (UI state `playbooks`). */
      async restoreRecord(value: unknown): Promise<void> {
        try {
          const r = await backend.restore(value ?? null);
          patchState(store, { runs: r.record.runs, restoreWarnings: r.warnings, activeRunId: null, results: {}, drafts: {} });
        } catch (e) {
          patchState(store, { runs: [], restoreWarnings: [failureText(e)], activeRunId: null, results: {}, drafts: {} });
        }
      },

      clearRuns: () => patchState(store, { runs: [], restoreWarnings: [], activeRunId: null, results: {}, drafts: {} }),
      dismissError: () => patchState(store, { actionError: null }),
      dismissNotice: () => patchState(store, { notice: null }),
    };
  }),
);
