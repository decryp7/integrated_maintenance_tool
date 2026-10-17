import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { ActionKind, AgentConfigFile, AgentIssue, AgentSchema, ScheduleFile, TriggerKind } from '../core/agent.types';
import { errorMessage } from '../tools/tool-errors';
import { OverwriteGuard } from '../ui/overwrite-guard';
import { issueIndex, newAction, newSchedule, newTrigger, setKey, TimingKind, withTiming } from './schedule-model';
import { SchedulesBackend } from './schedules-backend';

interface SchedulesState {
  /** The config file (empty: IMT's default). */
  path: string;
  config: AgentConfigFile | null;
  /** The file exists (false: a starter document). */
  exists: boolean;
  /** The file `exists` and `config` came from: saving back to it replaces it without asking. */
  loadedPath: string;
  schema: AgentSchema | null;
  issues: AgentIssue[];
  dirty: boolean;
  busy: string | null;
  error: string | null;
  notice: string | null;
}

const INITIAL: SchedulesState = { path: '', config: null, exists: false, loadedPath: '', schema: null, issues: [], dirty: false, busy: null, error: null, notice: null };

/** Editing `imt-agent.toml`: the engine validates and saves; the store keeps the draft and its issues. */
export const SchedulesStore = signalStore(
  { providedIn: 'root' },
  withState<SchedulesState>(INITIAL),
  withComputed((store) => ({
    issuesByPath: computed(() => issueIndex(store.issues())),
    schedules: computed(() => store.config()?.schedules ?? []),
  })),
  withMethods((store, backend = inject(SchedulesBackend), guard = inject(OverwriteGuard)) => {
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
    const pathArg = () => store.path().trim() || undefined;
    const edit = (f: (c: AgentConfigFile) => AgentConfigFile) => {
      const c = store.config();
      if (c) patchState(store, { config: f(c), dirty: true, notice: null });
    };
    const editSchedule = (i: number, f: (s: ScheduleFile) => ScheduleFile) => edit((c) => ({ ...c, schedules: c.schedules.map((s, k) => (k === i ? f(s) : s)) }));
    const schema = () => store.schema();

    return {
      setPath: (path: string) => patchState(store, { path }),
      dismissError: () => patchState(store, { error: null }),

      load: () =>
        run('Loading…', async () => {
          const [s, doc] = await Promise.all([store.schema() ? Promise.resolve(store.schema()!) : backend.schema(), backend.load(pathArg())]);
          patchState(store, { schema: s, path: doc.path, loadedPath: doc.path, config: doc.config, exists: doc.exists, issues: doc.issues, dirty: false });
          if (!doc.exists) patchState(store, { notice: 'No schedules saved yet: this is a starter configuration. Save it to create the file.' });
        }),

      validate: () =>
        run('Validating…', async () => {
          const c = store.config();
          if (!c) return;
          const issues = await backend.validate(c, pathArg());
          patchState(store, { issues, notice: issues.length ? null : 'The configuration is valid.' });
        }),

      save: () =>
        run('Saving…', async () => {
          const c = store.config();
          if (!c) return;
          // Saving back to the file that was loaded is the editor's normal save; any other existing file
          // (another path typed in, or one created meanwhile) is replaced only after asking.
          const own = store.exists() && store.path().trim() === store.loadedPath();
          const issues = await guard.write(store.path(), (overwrite) => backend.save(c, pathArg(), overwrite || own), 'app');
          if (issues === null) {
            patchState(store, { notice: 'Not saved: the existing file was kept.' });
          } else if (issues.length) {
            patchState(store, { issues, error: `Not saved: ${issues.length} problem${issues.length === 1 ? '' : 's'} to fix.` });
          } else {
            patchState(store, { issues, dirty: false, exists: true, loadedPath: store.path().trim(), notice: `Saved ${store.path()}` });
          }
        }),

      setAgent: (key: string, value: unknown, required = false) => edit((c) => ({ ...c, agent: setKey(c.agent, key, value, required) })),
      setLog: (key: string, value: unknown) => edit((c) => ({ ...c, agent: { ...c.agent, log: setKey(c.agent.log, key, value, true) } })),
      setRetention: (key: string, value: unknown) => edit((c) => ({ ...c, retention: setKey(c.retention, key, value === undefined ? null : value, true) })),

      addSchedule: () => {
        const s = schema();
        if (s) edit((c) => ({ ...c, schedules: [...c.schedules, newSchedule(c.schedules, s)] }));
      },
      removeSchedule: (i: number) => edit((c) => ({ ...c, schedules: c.schedules.filter((_, k) => k !== i) })),
      setScheduleKey: (i: number, key: string, value: unknown, required = false) => editSchedule(i, (s) => setKey(s, key, value, required)),
      setTiming: (i: number, kind: TimingKind) => {
        const s = schema();
        if (s) editSchedule(i, (x) => withTiming(x, kind, s));
      },
      setTriggerKind: (i: number, kind: TriggerKind) => {
        const s = schema();
        if (s) editSchedule(i, (x) => ({ ...x, trigger: newTrigger(kind, s) }));
      },
      setTriggerKey: (i: number, key: string, value: unknown, required = false) =>
        editSchedule(i, (x) => (x.trigger ? { ...x, trigger: setKey(x.trigger, key, value, required) } : x)),
      addAction: (i: number, kind: ActionKind = 'collect') => {
        const s = schema();
        if (s) editSchedule(i, (x) => ({ ...x, actions: [...x.actions, newAction(kind, s)] }));
      },
      removeAction: (i: number, j: number) => editSchedule(i, (x) => ({ ...x, actions: x.actions.filter((_, k) => k !== j) })),
      setActionKind: (i: number, j: number, kind: ActionKind) => {
        const s = schema();
        if (s) editSchedule(i, (x) => ({ ...x, actions: x.actions.map((a, k) => (k === j ? newAction(kind, s) : a)) }));
      },
      setActionKey: (i: number, j: number, key: string, value: unknown, required = false) =>
        editSchedule(i, (x) => ({ ...x, actions: x.actions.map((a, k) => (k === j ? setKey(a, key, value, required) : a)) })),
    };
  }),
);
