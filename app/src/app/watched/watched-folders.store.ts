import { inject } from '@angular/core';
import { patchState, signalStore, withHooks, withMethods, withState } from '@ngrx/signals';
import { WatchedFolderInfo } from '../core/api.types';
import { DebriefBackend } from '../core/backend';
import { failureText } from '../core/cmd-error';
import { WorkspaceStore } from '../core/workspace.store';

interface WatchedState {
  folders: WatchedFolderInfo[];
  error: string | null;
}

/** What a project saves of a watched folder. */
export interface SavedWatchedFolder {
  path: string;
  enabled: boolean;
}

/** Parses saved watched folders (anything else is ignored). */
export function parseWatchedFolders(value: unknown): SavedWatchedFolder[] {
  if (!Array.isArray(value)) return [];
  return (value as Partial<SavedWatchedFolder>[])
    .filter((f): f is SavedWatchedFolder => !!f && typeof f.path === 'string' && f.path.length > 0)
    .slice(0, 8)
    .map((f) => ({ path: f.path, enabled: f.enabled !== false }));
}

/**
 * The window's watched folders (ADR 0048): the engine polls them and imports files that settle; this store lists
 * them, adds and removes them, and refreshes the workspace when the engine says it imported something. The list is
 * saved with the project (`ProjectUiState`) and applied again on open.
 */
export const WatchedFoldersStore = signalStore(
  { providedIn: 'root' },
  withState<WatchedState>({ folders: [], error: null }),
  withMethods((store, backend = inject(DebriefBackend), workspace = inject(WorkspaceStore)) => {
    const apply = async (call: () => Promise<WatchedFolderInfo[]>): Promise<void> => {
      try {
        patchState(store, { folders: await call(), error: null });
      } catch (e) {
        patchState(store, { error: failureText(e) });
      }
    };
    return {
      load: () => apply(() => backend.watchedFolders()),
      add: (path: string) => apply(() => backend.addWatchedFolder(path)),
      remove: (path: string) => apply(() => backend.removeWatchedFolder(path)),
      setEnabled: (path: string, enabled: boolean) => apply(() => backend.setWatchedFolderEnabled(path, enabled)),
      /** What to save with the project. */
      saved: (): SavedWatchedFolder[] => store.folders().map((f) => ({ path: f.path, enabled: f.enabled })),
      /** Watches exactly `saved` (a project was opened): folders not listed are dropped. */
      async replace(saved: SavedWatchedFolder[]): Promise<void> {
        for (const f of store.folders()) if (!saved.some((s) => s.path === f.path)) await apply(() => backend.removeWatchedFolder(f.path));
        for (const s of saved) {
          await apply(() => backend.addWatchedFolder(s.path));
          if (!s.enabled) await apply(() => backend.setWatchedFolderEnabled(s.path, false));
        }
      },
      /** Called when the engine imported files from a watched folder. */
      async imported(): Promise<void> {
        await workspace.refreshSummary().catch(() => undefined);
        await apply(() => backend.watchedFolders());
      },
    };
  }),
  withHooks({
    onInit(store) {
      const backend = inject(DebriefBackend);
      void store.load();
      void backend.onWatchedImported(() => void store.imported()).catch(() => undefined);
    },
  }),
);
