import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { computed } from '@angular/core';

/**
 * What the workspace is busy with. An import (open, re-import, profile apply) is told apart by its kind, never by
 * matching its label: the label is display text in the running language. `jobId` is the import job when the
 * caller already knows it (a profile apply returns its job; an open only reports its summary when it is done).
 */
export type WorkspaceBusy = { kind: 'import'; label: string; jobId: number | null } | { kind: 'op'; label: string };

/** A button on the notice toast ("Undo", "Show"); running it dismisses the notice. */
export interface NoticeAction {
  label: string;
  run: () => void;
}

interface StatusState {
  busy: WorkspaceBusy | null;
  error: string | null;
  /** Transient informational message (e.g. "Exported 120 events"). */
  notice: string | null;
  /** The notice's action, if it has one. */
  noticeAction: NoticeAction | null;
}

/** The workspace's busy state, error banner and transient notice. */
export const WorkspaceStatusStore = signalStore(
  { providedIn: 'root' },
  withState<StatusState>({ busy: null, error: null, notice: null, noticeAction: null }),
  withComputed((store) => ({
    /** The busy text the title bar shows; null when idle. */
    busyLabel: computed(() => store.busy()?.label ?? null),
    /** True while an import runs (the import progress follows it). */
    importing: computed(() => store.busy()?.kind === 'import'),
  })),
  withMethods((store) => {
    // A notice goes after NOTICE_MS like a notification toast (paused while hovered or focused); an error stays
    // until it is dismissed, so it is never missed.
    let timer: ReturnType<typeof setTimeout> | undefined;
    const clearLater = () => {
      clearTimeout(timer);
      timer = setTimeout(() => patchState(store, { notice: null }), NOTICE_MS);
    };
    return {
      setBusy: (busy: WorkspaceBusy | null) => patchState(store, { busy }),
      dismissError: () => patchState(store, { error: null }),
      setError: (error: string | null) => patchState(store, { error }),
      setNotice: (notice: string | null, action?: NoticeAction) => {
        patchState(store, { notice, noticeAction: notice === null ? null : (action ?? null) });
        if (notice === null) clearTimeout(timer);
        else clearLater();
      },
      /** Runs the notice's action and dismisses the notice. */
      runNoticeAction: () => {
        const action = store.noticeAction();
        patchState(store, { notice: null, noticeAction: null });
        clearTimeout(timer);
        action?.run();
      },
      /** Keeps the notice while the pointer or focus is on it. */
      holdNotice: () => clearTimeout(timer),
      /** The pointer and focus left the notice: it goes after the usual time again. */
      releaseNotice: () => {
        if (store.notice() !== null) clearLater();
      },
    };
  }),
);

/** How long a notice stays (the same as a notification toast). */
export const NOTICE_MS = 8000;
