import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withState } from '@ngrx/signals';
import { UpdatesBackend } from './updates-backend';
import { toUpdateError, UpdateError, UpdateStatus } from './updates.types';

interface UpdatesState {
  status: UpdateStatus | null;
  /** A request that failed (the engine's own failures are in the phase). */
  error: UpdateError | null;
  /** The Updates dialog is open (no toast then). */
  dialogOpen: boolean;
  /** Version announced by the "update available" toast; null when hidden. */
  notice: string | null;
  /** Versions already announced in this run. */
  announced: string[];
}

/** The updater as the UI sees it: the engine's status, kept current from its events. */
export const UpdatesStore = signalStore(
  { providedIn: 'root' },
  withState<UpdatesState>({ status: null, error: null, dialogOpen: false, notice: null, announced: [] }),
  withComputed((s) => ({
    phase: computed(() => s.status()?.phase ?? null),
    /** The build can update itself (it has an update key). */
    available: computed(() => !!s.status() && s.status()!.phase.kind !== 'unavailable'),
    /** Download progress 0..1, or null when unknown / not downloading. */
    progress: computed(() => {
      const p = s.status()?.phase;
      if (p?.kind !== 'downloading' || !p.total) return null;
      return Math.min(1, p.downloaded / p.total);
    }),
  })),
  withMethods((store, backend = inject(UpdatesBackend)) => {
    const apply = (status: UpdateStatus) => {
      const p = status.phase;
      const fresh = p.kind === 'available' && !store.dialogOpen() && !store.announced().includes(p.info.version);
      patchState(store, (st) => ({
        status,
        notice: fresh ? p.info.version : p.kind === 'available' || p.kind === 'downloading' || p.kind === 'ready' ? st.notice : null,
        announced: fresh ? [...st.announced, p.info.version] : st.announced,
      }));
    };
    const run = async (request: () => Promise<UpdateStatus | void>) => {
      patchState(store, { error: null });
      try {
        const s = await request();
        if (s) apply(s);
      } catch (e) {
        patchState(store, { error: toUpdateError(e) });
      }
    };
    return {
      apply,
      refresh: () => run(() => backend.status()),
      check: () => run(() => backend.check()),
      download: () => run(() => backend.download()),
      cancel: () => run(() => backend.cancel()),
      setInstallOnQuit: (enabled: boolean) => run(() => backend.installOnQuit(enabled)),
      restart: () => run(() => backend.restart()),
      openReleases: () => run(() => backend.openReleases()),
      /** The dialog shows the status; nothing is announced while it is open. */
      setDialogOpen(open: boolean) {
        patchState(store, { dialogOpen: open, notice: open ? null : store.notice() });
      },
      dismissNotice() {
        patchState(store, { notice: null });
      },
    };
  }),
  withHooks({
    onInit(store, backend = inject(UpdatesBackend)) {
      void store.refresh();
      void backend.onStatus((s) => store.apply(s)).catch(() => undefined);
    },
  }),
);
