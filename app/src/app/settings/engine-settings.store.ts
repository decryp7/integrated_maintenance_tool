import { OverwriteGuard } from '../ui/overwrite-guard';
import { computed, inject, Injector } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { errorMessage } from '../tools/tool-errors';
import { SettingsBackend } from './settings-backend';
import { GENERAL_ID, sameValue } from './settings-model';
import { FieldError, ImportResult, SettingsOverview, SettingsSection, SettingValue } from './settings.types';

interface EngineSettingsState {
  overview: SettingsOverview | null;
  loading: boolean;
  /** Loading the sections failed (shown with Retry). */
  loadError: string | null;
  /** Navigation selection: a section id or `GENERAL_ID`. */
  selected: string;
  /** Unapplied edits: section id → key → value. Kept when the screen is closed without Apply. */
  drafts: Record<string, Record<string, SettingValue>>;
  /** The engine's validation messages from the last Apply, by section. */
  errors: Record<string, FieldError[]>;
  busy: boolean;
  /** Result of the last action (applied, exported, imported). */
  notice: string | null;
  /** A failed action (apply rejected, export failed, ...). */
  actionError: string | null;
  query: string;
  showAdvanced: boolean;
  /** A change applied in this run needs a restart. */
  restartRequired: boolean;
  /** "Discard unapplied changes?" is showing. */
  confirmDiscard: boolean;
}

const INITIAL: EngineSettingsState = {
  overview: null,
  loading: false,
  loadError: null,
  selected: GENERAL_ID,
  drafts: {},
  errors: {},
  busy: false,
  notice: null,
  actionError: null,
  query: '',
  showAdvanced: false,
  restartRequired: false,
  confirmDiscard: false,
};

function describeImport(r: ImportResult): string {
  const parts = [`Imported ${r.applied.length} section${r.applied.length === 1 ? '' : 's'}`];
  if (r.kept.length) parts.push(`kept ${r.kept.length} for later (${r.kept.join(', ')})`);
  if (r.rejected.length) parts.push(`rejected ${r.rejected.map((s) => s.section || 'invalid').join(', ')}`);
  return parts.join('; ') + '.';
}

/**
 * State of the Settings screen: the engine's sections, the user's unapplied edits and the engine's
 * answers. Values are validated and applied by the engine; this store only keeps drafts.
 */
export const EngineSettingsStore = signalStore(
  { providedIn: 'root' },
  withState<EngineSettingsState>(INITIAL),
  withComputed((store) => ({
    sections: computed<SettingsSection[]>(() => store.overview()?.sections ?? []),
    /** Sections with an edit that differs from the applied value. */
    dirtySections: computed(() => Object.keys(store.drafts()).filter((id) => Object.keys(store.drafts()[id]).length > 0)),
    restartPending: computed(() => store.restartRequired() || !!store.overview()?.restartPending),
  })),
  withComputed((store) => ({
    dirty: computed(() => store.dirtySections().length > 0),
    current: computed(() => store.sections().find((s) => s.id === store.selected()) ?? null),
  })),
  withMethods((store, backend = inject(SettingsBackend), injector = inject(Injector), overwrite = inject(OverwriteGuard)) => {
    const section = (id: string) => store.sections().find((s) => s.id === id);
    const replaceValues = (id: string, values: Record<string, SettingValue>) => {
      const overview = store.overview();
      if (!overview) return;
      patchState(store, { overview: { ...overview, sections: overview.sections.map((s) => (s.id === id ? { ...s, values } : s)) } });
    };
    const without = <T>(map: Record<string, T>, id: string): Record<string, T> => {
      const { [id]: _drop, ...rest } = map;
      return rest;
    };
    const run = async (op: () => Promise<void>) => {
      patchState(store, { busy: true, notice: null, actionError: null });
      try {
        await op();
      } catch (e) {
        patchState(store, { actionError: errorMessage(e) });
      } finally {
        patchState(store, { busy: false });
      }
    };
    /** Sends one section (applied values with the edits on top); true when the engine applied it. */
    const applySection = async (id: string): Promise<boolean> => {
      const s = section(id);
      if (!s) {
        patchState(store, (st) => ({ drafts: without(st.drafts, id) }));
        return true;
      }
      try {
        const r = await backend.set(id, { ...s.values, ...store.drafts()[id] });
        if (!r.applied) {
          patchState(store, (st) => ({ errors: { ...st.errors, [id]: r.errors } }));
          return false;
        }
        replaceValues(id, r.values);
        patchState(store, (st) => ({ drafts: without(st.drafts, id), errors: without(st.errors, id), restartRequired: st.restartRequired || r.restartRequired }));
        return true;
      } catch (e) {
        patchState(store, (st) => ({ errors: { ...st.errors, [id]: [{ key: '', message: errorMessage(e) }] } }));
        return false;
      }
    };
    return {
      /** (Re)reads the sections. Unapplied edits are kept. */
      async load(): Promise<void> {
        patchState(store, { loading: true, loadError: null });
        try {
          patchState(store, { overview: await backend.sections() });
        } catch (e) {
          patchState(store, { loadError: errorMessage(e) });
        } finally {
          patchState(store, { loading: false });
        }
      },
      select: (selected: string) => patchState(store, { selected, query: '' }),
      setQuery: (query: string) => patchState(store, { query }),
      setShowAdvanced: (showAdvanced: boolean) => patchState(store, { showAdvanced }),
      dismissNotice: () => patchState(store, { notice: null, actionError: null }),
      /** The value shown for a field: the edit if any, else the applied value. */
      valueOf(id: string, key: string): SettingValue | undefined {
        return store.drafts()[id]?.[key] ?? section(id)?.values[key];
      },
      errorFor(id: string, key: string): string | null {
        return store.errors()[id]?.find((e) => e.key === key)?.message ?? null;
      },
      /** Records an edit; editing back to the applied value removes it. Clears that field's engine error. */
      edit(id: string, key: string, value: SettingValue): void {
        const applied = section(id)?.values[key];
        patchState(store, (st) => {
          const draft = { ...(st.drafts[id] ?? {}) };
          if (sameValue(applied, value)) delete draft[key];
          else draft[key] = value;
          const errors = (st.errors[id] ?? []).filter((e) => e.key !== key);
          return {
            drafts: Object.keys(draft).length ? { ...st.drafts, [id]: draft } : without(st.drafts, id),
            errors: errors.length ? { ...st.errors, [id]: errors } : without(st.errors, id),
          };
        });
      },
      /** Edits a field back to its default (applied with Apply). */
      resetField(id: string, key: string): void {
        const field = section(id)?.fields.find((f) => f.key === key);
        if (field) this.edit(id, key, field.default);
      },
      /** Resets a whole section in the engine now. */
      resetSection: (id: string) =>
        run(async () => {
          const r = await backend.reset(id);
          replaceValues(id, r.values);
          patchState(store, (st) => ({
            drafts: without(st.drafts, id),
            errors: without(st.errors, id),
            restartRequired: st.restartRequired || r.restartRequired,
            notice: `${section(id)?.title ?? id} reset to defaults.`,
          }));
        }),
      /** Applies every edited section; on engine errors, shows the first section with errors. */
      async apply(): Promise<boolean> {
        let ok = true;
        await run(async () => {
          for (const id of store.dirtySections()) ok = (await applySection(id)) && ok;
          if (ok) patchState(store, { notice: 'Settings applied.' });
          else {
            const first = Object.keys(store.errors())[0];
            if (first) patchState(store, { selected: first, query: '' });
          }
        });
        return ok;
      },
      /** Drops every unapplied edit and engine error. */
      discard: () => patchState(store, { drafts: {}, errors: {}, confirmDiscard: false }),
      /** Whether the screen may close now; with unapplied edits it asks first and says no. */
      canClose(): boolean {
        if (!store.dirty()) return true;
        patchState(store, { confirmDiscard: true });
        return false;
      },
      keepEditing: () => patchState(store, { confirmDiscard: false }),
      exportTo: (path: string) =>
        run(async () => {
          if ((await overwrite.write(path, (replace) => backend.exportTo(path, replace))) === null) return;
          patchState(store, { notice: `Settings exported to ${path}.` });
        }),
      importFrom: (path: string) =>
        run(async () => {
          const r = await backend.importFrom(path);
          patchState(store, (st) => ({
            errors: Object.fromEntries(r.rejected.filter((s) => s.section).map((s) => [s.section, s.errors])),
            restartRequired: st.restartRequired || r.restartRequired,
            notice: describeImport(r),
          }));
          patchState(store, { overview: await backend.sections() });
        }),
      /** Relaunches IMT (reopening the session) so restart-only settings take effect. */
      restart: () => injector.get(LifecycleService).restartKeepingSession(),
    };
  }),
);
