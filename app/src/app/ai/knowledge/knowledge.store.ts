import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { toAiError } from '../ai-backend';
import { KnowledgeChange, KnowledgeInfo, KnowledgeList, KnowledgeScope } from '../ai.types';
import { KnowledgeBackend } from './knowledge-backend';
import { KnowledgePersistence } from './knowledge-persistence';

interface KnowledgeState {
  list: KnowledgeList | null;
  loading: boolean;
  /** Listing failed (shown with Retry). */
  loadError: string | null;
  /** The last change failed. */
  actionError: string | null;
  /** A change is running. */
  busy: boolean;
}

const INITIAL: KnowledgeState = { list: null, loading: false, loadError: null, actionError: null, busy: false };

/**
 * State of Settings ▸ AI ▸ Knowledge: both scopes' files as the engine lists them, and the outcome of changes. The
 * engine checks every change; after a change to the project's files the saved record is refreshed (the project
 * becomes unsaved). Dialogs and confirmations are the components' business.
 */
export const KnowledgeStore = signalStore(
  { providedIn: 'root' },
  withState<KnowledgeState>(INITIAL),
  withComputed((store) => ({
    project: computed<KnowledgeInfo[]>(() => store.list()?.project ?? []),
    global: computed<KnowledgeInfo[]>(() => store.list()?.global ?? []),
    warnings: computed<string[]>(() => store.list()?.warnings ?? []),
    globalReadOnly: computed(() => store.list()?.globalReadOnly ?? false),
    maxFiles: computed(() => store.list()?.maxFiles ?? 32),
  })),
  withMethods((store, backend = inject(KnowledgeBackend), persistence = inject(KnowledgePersistence)) => {
    const load = async (): Promise<void> => {
      patchState(store, { loading: true, loadError: null });
      try {
        patchState(store, { list: await backend.listKnowledge() });
      } catch (e) {
        patchState(store, { loadError: toAiError(e).message });
      } finally {
        patchState(store, { loading: false });
      }
    };
    /** Runs a change, then lists again; resolves with its result, or null when it failed (the error is shown). */
    const change = async <T>(scope: KnowledgeScope, run: () => Promise<T>): Promise<T | null> => {
      patchState(store, { busy: true, actionError: null });
      try {
        const r = await run();
        if (scope === 'project') await persistence.sync();
        await load();
        return r;
      } catch (e) {
        patchState(store, { actionError: toAiError(e).message });
        return null;
      } finally {
        patchState(store, { busy: false });
      }
    };
    return {
      load,
      create: (scope: KnowledgeScope, name: string, text = '') => change(scope, () => backend.createKnowledge(scope, name, text)),
      importFile: (scope: KnowledgeScope, path: string) => change(scope, () => backend.importKnowledge(scope, path)),
      update: (f: KnowledgeInfo, c: KnowledgeChange) => change(f.scope, () => backend.updateKnowledge(f.scope, f.id, c)),
      setEnabled: (f: KnowledgeInfo, enabled: boolean) => change(f.scope, () => backend.updateKnowledge(f.scope, f.id, { enabled })),
      remove: (f: KnowledgeInfo) => change(f.scope, () => backend.removeKnowledge(f.scope, f.id)),
      /** Writes the file to `path` (rejects with an `AiError`, e.g. `fileExists` without `overwrite`). */
      exportFile: (f: KnowledgeInfo, path: string, overwrite: boolean) => backend.exportKnowledge(f.scope, f.id, path, overwrite),
      /** The file's text for the editor (rejects with an `AiError`). */
      text: (f: KnowledgeInfo) => backend.getKnowledge(f.scope, f.id).then((t) => t.text),
      dismissError: () => patchState(store, { actionError: null }),
    };
  }),
);
