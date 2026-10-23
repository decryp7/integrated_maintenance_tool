import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withState } from '@ngrx/signals';
import { MarkerKind, WorkspaceSummary } from '../api.types';
import { DebriefBackend } from '../backend';
import { failureText } from '../cmd-error';
import { Logger } from '../logger';
import { Platform } from '../platform';
import { JobsBackend } from '../../jobs/jobs-backend';
import { RECENT_STORE } from '../../welcome/recent-store';
import { plural } from '../../i18n/locale';
import { SelectionStore } from './selection.store';
import { WorkspaceBusy, WorkspaceStatusStore } from './status.store';

interface DataState {
  summary: WorkspaceSummary | null;
  /** Bumped when the set of sources or the profile changed (open, remove, clear, reload, profile save). */
  sourcesVersion: number;
  /** Bumped when the engine added events to the sources already open (live tail). */
  appendVersion: number;
  /** Bumped whenever the profile is saved and applied. */
  profileVersion: number;
  /** Bumped after files or folders were opened successfully (Home opens after an import). */
  importVersion: number;
}

/** The text the error banner shows: the message, then the engine's next step (`hint`) when it gave one. */
export function errorText(e: unknown): string {
  return failureText(e);
}

/** The busy state while `n` paths are being opened. */
function opening(n: number): WorkspaceBusy {
  const label = plural(n, {
    one: $localize`:@@core.workspace.busy.openingOne:Opening ${n}:count: item…`,
    other: $localize`:@@core.workspace.busy.openingMany:Opening ${n}:count: items…`,
  });
  return { kind: 'import', label, jobId: null };
}

/** Same sources (by id, in order) and the same profile: a refresh only appended events. */
export function sameSources(a: WorkspaceSummary | null, b: WorkspaceSummary): boolean {
  if (!a || a.profileName !== b.profileName || a.sources.length !== b.sources.length) return false;
  return a.sources.every((s, i) => s.id === b.sources[i]?.id);
}

/**
 * The engine's view of the workspace: its summary, the commands that replace it (open, remove, clear, reload,
 * profile) and the generations views key their requests on. Views that only care about which sources are open
 * read `sourcesVersion`; views that show events read `dataVersion` (sources or appended events). The first summary
 * loads when the store is first used (a read of the engine, nothing else: the window's side effects start in
 * `provideWorkspace`).
 */
export const WorkspaceDataStore = signalStore(
  { providedIn: 'root' },
  withState<DataState>({ summary: null, sourcesVersion: 0, appendVersion: 0, profileVersion: 0, importVersion: 0 }),
  withComputed((store) => ({
    /** Changes whenever the data changed in any way (sources, profile or appended events). */
    dataVersion: computed(() => store.sourcesVersion() + store.appendVersion()),
    markerKindMap: computed(() => new Map<string, MarkerKind>((store.summary()?.markerKinds ?? []).map((k) => [k.kind, k]))),
    fieldNames: computed(() => (store.summary()?.fields ?? []).map((f) => f.name)),
    numericFields: computed(() => (store.summary()?.fields ?? []).filter((f) => f.numeric).map((f) => f.name)),
  })),
  withMethods(
    (
      store,
      backend = inject(DebriefBackend),
      platform = inject(Platform),
      logger = inject(Logger),
      recent = inject(RECENT_STORE),
      jobs = inject(JobsBackend),
      status = inject(WorkspaceStatusStore),
      selection = inject(SelectionStore),
    ) => {
      const applySummary = (summary: WorkspaceSummary) => {
        patchState(store, (s) => ({ summary, sourcesVersion: s.sourcesVersion + 1 }));
        selection.select(null);
        status.setBusy(null);
        status.setError(null);
      };

      /** Runs a workspace command; logs counts only (never names, paths or content). */
      const run = async (busy: WorkspaceBusy, op: () => Promise<WorkspaceSummary>, operation: string) => {
        status.setBusy(busy);
        status.setError(null);
        try {
          const summary = await op();
          applySummary(summary);
          logger.info('ui.workspace', `Workspace ${operation}`, {
            Operation: operation,
            Sources: summary.sources.length,
            Unrecognized: summary.unrecognized.length,
            Events: summary.total,
            LoadMs: summary.loadMs,
          });
        } catch (e) {
          status.setBusy(null);
          status.setError(errorText(e));
        }
      };

      const open = async (paths: string[]) => {
        await run(opening(paths.length), () => backend.openPaths(paths), 'opened');
        if (!status.error()) {
          recent.add(paths);
          patchState(store, (s) => ({ importVersion: s.importVersion + 1 }));
        }
      };

      return {
        applySummary,
        /** Applies the summary of a project or bundle that brought its own profile and announces it to listeners. */
        profileSaved(summary: WorkspaceSummary) {
          applySummary(summary);
          patchState(store, (s) => ({ profileVersion: s.profileVersion + 1 }));
        },
        /**
         * Follows the re-import a profile save started (`saveProfile` returned its import job): the status bar shows
         * its progress, the new summary applies and listeners hear of the profile when it is done. Callers need not
         * wait for it. A failed re-import shows its reason like any import.
         */
        async applyProfile(jobId: number): Promise<void> {
          const label = $localize`:@@core.workspace.busy.applyingProfile:Applying the profile…`;
          await run(
            { kind: 'import', label, jobId },
            async () => {
              const end = await jobs.wait(jobId);
              if (end.state === 'failed') throw new Error(end.error ?? $localize`:@@core.workspace.profileFailed:The profile could not be applied.`);
              return backend.workspaceSummary();
            },
            'profile applied',
          );
          if (!status.error()) patchState(store, (s) => ({ profileVersion: s.profileVersion + 1 }));
        },
        /**
         * Follows a re-read the parser library started (`parsers_reread`, `parser_assign`: ADR 0026) like an import:
         * the status bar shows its progress and the new summary applies. A failed re-read shows its reason.
         */
        async followReread(jobId: number): Promise<void> {
          const label = $localize`:@@core.workspace.busy.rereading:Reading files again with their parsers…`;
          await run(
            { kind: 'import', label, jobId },
            async () => {
              const end = await jobs.wait(jobId);
              if (end.state === 'failed') throw new Error(end.error ?? $localize`:@@core.workspace.rereadFailed:The files could not be read again.`);
              return backend.workspaceSummary();
            },
            'reread',
          );
        },
        async init() {
          await run({ kind: 'op', label: $localize`:@@core.workspace.busy.loading:Loading workspace…` }, () => backend.workspaceSummary(), 'loaded');
        },
        /**
         * Fetches the summary again after the engine changed the data underneath (live tail, or the host window of a
         * pop-out). Unlike `init`, it keeps the selection, the query and filters, and shows no busy state. The same
         * sources bump `appendVersion`, others `sourcesVersion`. Rejects when the backend fails.
         */
        async refreshSummary(): Promise<void> {
          const summary = await backend.workspaceSummary();
          const appended = sameSources(store.summary(), summary);
          patchState(store, (s) => (appended ? { summary, appendVersion: s.appendVersion + 1 } : { summary, sourcesVersion: s.sourcesVersion + 1 }));
        },
        /** Applies the summary of text the engine saved and opened as a source (Paste as log), like an open does. */
        applyImported(summary: WorkspaceSummary): void {
          applySummary(summary);
          patchState(store, (s) => ({ importVersion: s.importVersion + 1 }));
        },
        openPaths: (paths: string[]): Promise<void> => open(paths),
        /**
         * Opens `paths`, reading only the events inside a time range (epoch ms; either end may be null): events
         * outside it never reach the store (ADR 0047). The range applies to this open only.
         */
        async openInRange(paths: string[], range: { from: number | null; to: number | null }): Promise<void> {
          try {
            await backend.setImportWindow(range.from, range.to);
          } catch (e) {
            status.setError(errorText(e));
            return;
          }
          try {
            await open(paths);
          } finally {
            await backend.setImportWindow(null, null).catch(() => undefined);
          }
        },
        async pickAndOpen(directory: boolean) {
          try {
            const paths = await platform.pickPaths(directory);
            if (paths.length) await open(paths);
          } catch (e) {
            status.setError(errorText(e));
          }
        },
        removeSource: (id: number) =>
          run({ kind: 'op', label: $localize`:@@core.workspace.busy.removing:Removing source…` }, () => backend.removeSource(id), 'source removed'),
        clearWorkspace: () => run({ kind: 'op', label: $localize`:@@core.workspace.busy.clearing:Clearing workspace…` }, () => backend.clearWorkspace(), 'cleared'),
        reload: () =>
          run({ kind: 'import', label: $localize`:@@core.workspace.busy.reimporting:Re-importing…`, jobId: null }, () => backend.reloadWorkspace(), 'reloaded'),
      };
    },
  ),
  withHooks({
    onInit(store) {
      void store.init();
    },
  }),
);
