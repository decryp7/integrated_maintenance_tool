import { computed, effect, inject, untracked } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withState } from '@ngrx/signals';
import { WorkspaceSummary } from '../core/api.types';
import { ToolCatalogBackend } from '../core/tools-backend';
import { ToolEntry } from '../core/tools.types';
import { WorkspaceStore } from '../core/workspace.store';
import { errorMessage } from './tool-errors';

interface CatalogState {
  entries: ToolEntry[];
  /** `tools_list` answered at least once. Until then every tool counts as there (no flicker). */
  loaded: boolean;
  error: string | null;
}

/** Why a tool cannot be used: its plugin is inactive or quarantined (with the engine's reason), or it is not installed at all. */
export type ToolProblem = { kind: 'plugin'; reason: string } | { kind: 'missing' };

/** Changes when any plugin's status changes, so the catalog is read again. */
function pluginKey(summary: WorkspaceSummary | null): string {
  return (summary?.plugins ?? []).map((p) => `${p.id}:${p.status}:${p.hung}`).join('|');
}

/**
 * The plugin tools of this window (`tools_list`): which exist and whether they can run. Tool views and commands
 * are shown only while their tool is listed (none in safe mode or with the tools plugin disabled); a listed tool
 * whose plugin is inactive or quarantined shows the engine's reason instead of failing on use.
 */
export const ToolCatalog = signalStore(
  { providedIn: 'root' },
  withState<CatalogState>({ entries: [], loaded: false, error: null }),
  withComputed((store) => ({
    byId: computed(() => new Map(store.entries().map((e) => [e.id, e]))),
  })),
  withMethods((store, backend = inject(ToolCatalogBackend)) => ({
    /** Reads the tools again. A failure keeps the previous answer (and its optimism before the first). */
    async load(): Promise<void> {
      try {
        patchState(store, { entries: await backend.toolsList(), loaded: true, error: null });
      } catch (e) {
        patchState(store, { error: errorMessage(e) });
      }
    },
    entry: (id: string): ToolEntry | null => store.byId().get(id) ?? null,
    /** The tool's views and commands are offered. */
    listed: (id: string): boolean => !store.loaded() || store.byId().has(id),
    /** The tool can run now (its platform is supported and its plugin is active). */
    usable: (id: string): boolean => !store.loaded() || store.byId().get(id)?.availability.state === 'available',
    /** Why the tool's plugin cannot serve it; null when it can (an unsupported platform is the view's to explain). */
    problem: (id: string): ToolProblem | null => {
      if (!store.loaded()) return null;
      const entry = store.byId().get(id);
      if (!entry) return { kind: 'missing' };
      return entry.availability.state === 'pluginUnavailable' ? { kind: 'plugin', reason: entry.availability.reason } : null;
    },
  })),
  withHooks({
    onInit(store) {
      const workspace = inject(WorkspaceStore);
      // Loads now, and again whenever a plugin's status changes (quarantine, reset).
      effect(() => {
        pluginKey(workspace.summary());
        untracked(() => void store.load());
      });
    },
  }),
);
