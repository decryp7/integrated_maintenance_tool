import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withState } from '@ngrx/signals';
import { JobsBackend } from './jobs-backend';
import { EMPTY_JOBS, JOB_KINDS, JobInfo, JobKind, JobsSnapshot } from './jobs.types';

/** The jobs the pane lists: everything, what is running or waiting, or what failed. */
export type JobsView = 'all' | 'running' | 'failed';


interface JobsState {
  snapshot: JobsSnapshot;
  /** The kinds the pane shows (empty: all). */
  kinds: JobKind[];
  /** All, running (and queued), or failed jobs. */
  view: JobsView;
  /** The last request that failed (shown inline with Retry). */
  error: string | null;
  loaded: boolean;
}

const initial: JobsState = { snapshot: EMPTY_JOBS, kinds: [], view: 'all', error: null, loaded: false };

function message(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  if (e && typeof e === 'object' && typeof (e as { message?: unknown }).message === 'string') return (e as { message: string }).message;
  return $localize`:@@jobs.store.failed:The jobs service did not answer.`;
}

const isActive = (j: JobInfo) => j.state === 'queued' || j.state === 'running';

/**
 * The window's jobs as the engine reports them (`debrief://jobs`): the engine decides what is visible, the
 * queue order, positions and limits; this keeps the list, the kind filter and the last error.
 */
export const JobsStore = signalStore(
  { providedIn: 'root' },
  withState<JobsState>(initial),
  withComputed((store) => {
    const inView = (j: JobInfo, view: JobsView) => view === 'all' || (view === 'running' ? isActive(j) : j.state === 'failed');
    const filtered = computed(() => {
      const kinds = store.kinds();
      const view = store.view();
      return store.snapshot().jobs.filter((j) => (!kinds.length || kinds.includes(j.kind)) && inView(j, view));
    });
    return {
      /** How many jobs each view would show (under the kind filter). */
      viewCounts: computed(() => {
        const kinds = store.kinds();
        const jobs = store.snapshot().jobs.filter((j) => !kinds.length || kinds.includes(j.kind));
        return { all: jobs.length, running: jobs.filter(isActive).length, failed: jobs.filter((j) => j.state === 'failed').length } satisfies Record<JobsView, number>;
      }),
      /** How many jobs each kind has (under the view), for the Kind filter. */
      kindCounts: computed(() => {
        const view = store.view();
        const counts = new Map<JobKind, number>(JOB_KINDS.map((k) => [k, 0]));
        for (const j of store.snapshot().jobs) if (inView(j, view)) counts.set(j.kind, (counts.get(j.kind) ?? 0) + 1);
        return counts;
      }),
      jobs: filtered,
      running: computed(() => filtered().filter((j) => j.state === 'running')),
      queued: computed(() => filtered().filter((j) => j.state === 'queued')),
      finished: computed(() => filtered().filter((j) => !isActive(j))),
      /** Running and queued jobs of every kind (the badge). */
      activeCount: computed(() => store.snapshot().jobs.filter(isActive).length),
      paused: computed(() => store.snapshot().paused),
    };
  }),
  withMethods((store, backend = inject(JobsBackend)) => {
    let last: (() => Promise<unknown>) | null = null;
    const run = async <T>(f: () => Promise<T>): Promise<T | null> => {
      last = f;
      patchState(store, { error: null });
      try {
        return await f();
      } catch (e) {
        patchState(store, { error: message(e) });
        return null;
      }
    };
    return {
      apply(snapshot: JobsSnapshot): void {
        patchState(store, { snapshot, loaded: true });
      },

      /** Reads the list (Retry of a failed first load reads and applies it again). */
      async load(): Promise<void> {
        await run(async () => {
          const s = await backend.list();
          patchState(store, { snapshot: s, loaded: true });
          return s;
        });
      },

      cancel: (id: number) => run(() => backend.cancel(id)),
      retry: (id: number) => run(() => backend.retry(id)),
      move: (id: number, up: boolean) => run(() => backend.move(id, up)),
      clearFinished: () => run(() => backend.clearFinished()),
      markSeen: (id: number) => run(() => backend.markSeen(id)),

      setView(view: JobsView): void {
        patchState(store, { view });
      },

      /** Shows only these kinds (none: all). */
      setKinds(kinds: JobKind[]): void {
        patchState(store, { kinds: [...new Set(kinds)] });
      },

      toggleKind(kind: JobKind): void {
        const kinds = store.kinds();
        patchState(store, { kinds: kinds.includes(kind) ? kinds.filter((k) => k !== kind) : [...kinds, kind] });
      },

      /** The chat's turn while it waits in the AI queue (for "queued (position n)"). */
      queuedTurn(sessionId: string): JobInfo | null {
        return store.snapshot().jobs.find((j) => j.state === 'queued' && j.source.link?.type === 'chat' && j.source.link.sessionId === sessionId) ?? null;
      },

      /** Retry for the inline error. */
      async retryRequest(): Promise<void> {
        if (last) await run(last);
      },

      dismissError(): void {
        patchState(store, { error: null });
      },
    };
  }),
  withHooks((store, backend = inject(JobsBackend)) => {
    let off: (() => void) | undefined;
    let destroyed = false;
    return {
      onInit() {
        void backend
          .onChange((s) => store.apply(s))
          .then((u) => (destroyed ? u() : (off = u)))
          .catch(() => undefined);
        void store.load();
      },
      onDestroy() {
        destroyed = true;
        off?.();
      },
    };
  }),
);

export type JobsStoreType = InstanceType<typeof JobsStore>;
