import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { errorMessage } from '../../tools/tool-errors';
import { StorageBackend } from './storage-backend';
import { JobsBackend } from '../../jobs/jobs-backend';
import { StorageCategory, StorageCategoryId, StorageClearResult, StorageUsage } from './storage.types';

interface StorageState {
  usage: StorageUsage | null;
  loading: boolean;
  /** Measuring failed (shown with Retry). */
  loadError: string | null;
  /** The categories being cleared now. */
  clearing: StorageCategoryId[] | null;
  /** The maintenance job that clears them, while Settings follows it. */
  job: number | null;
  /** What the last clear did. */
  result: StorageClearResult | null;
  /** The last clear or "Show in folder" failed (shown with Retry). */
  actionError: string | null;
  /** What Retry repeats after a failed action. */
  retry: { kind: 'clear'; ids: StorageCategoryId[] } | { kind: 'reveal'; id: StorageCategoryId } | null;
  /** The folder "Show in folder" opened last. */
  revealed: string | null;
}

const INITIAL: StorageState = { usage: null, loading: false, loadError: null, clearing: null, job: null, result: null, actionError: null, retry: null, revealed: null };

/**
 * State of Settings ▸ Storage: the engine's usage report and the outcome of clearing. The engine decides
 * what each category holds and what may go; confirmations are the component's business.
 */
export const StorageStore = signalStore(
  { providedIn: 'root' },
  withState<StorageState>(INITIAL),
  withComputed((store) => {
    const categories = computed<StorageCategory[]>(() => store.usage()?.categories ?? []);
    /** What "Clear cache" removes: every clearable cache category with something to remove. */
    const cache = computed(() => categories().filter((c) => c.kind === 'cache' && c.clearable && c.clearableBytes > 0));
    return {
      categories,
      cache,
      cacheBytes: computed(() => cache().reduce((n, c) => n + c.clearableBytes, 0)),
      busy: computed(() => store.loading() || store.clearing() !== null),
    };
  }),
  withMethods((store, backend = inject(StorageBackend), jobs = inject(JobsBackend)) => {
    const load = async (): Promise<void> => {
      patchState(store, { loading: true, loadError: null });
      try {
        patchState(store, { usage: await backend.usage() });
      } catch (e) {
        patchState(store, { loadError: errorMessage(e) });
      } finally {
        patchState(store, { loading: false });
      }
    };
    const clear = async (ids: StorageCategoryId[]): Promise<void> => {
      if (!ids.length || store.clearing()) return;
      patchState(store, { clearing: ids, actionError: null, retry: null, result: null, revealed: null });
      try {
        // A background job: Settings shows it while open; closed, the job's message tells the outcome.
        const id = await backend.clear(ids);
        patchState(store, { job: id });
        const end = await jobs.wait(id);
        if (store.job() !== id) return;
        if (end.state === 'failed') throw end.error ?? $localize`:@@settings.storage.clearFailed:Clearing failed.`;
        if (end.result?.type === 'storage') patchState(store, { result: end.result.result as StorageClearResult });
      } catch (e) {
        patchState(store, { actionError: errorMessage(e), retry: { kind: 'clear', ids } });
      } finally {
        patchState(store, { clearing: null, job: null });
      }
      await load();
    };
    const reveal = async (id: StorageCategoryId): Promise<void> => {
      patchState(store, { actionError: null, retry: null, revealed: null });
      try {
        patchState(store, { revealed: await backend.reveal(id) });
      } catch (e) {
        patchState(store, { actionError: errorMessage(e), retry: { kind: 'reveal', id } });
      }
    };
    return {
      load,
      clear,
      reveal,
      /** Settings closed while a clear runs: stop following it (its message tells the outcome). */
      detach(): void {
        const id = store.job();
        if (id === null) return;
        patchState(store, { job: null, clearing: null });
        void jobs.unwait(id).catch(() => undefined);
      },
      /** View of a finished clear: what it freed and kept. */
      showResult(result: StorageClearResult): void {
        patchState(store, { result, actionError: null, retry: null });
        void load();
      },
      /** Clears every cache category ("Clear cache"). */
      clearCache: () => clear(store.cache().map((c) => c.id)),
      /** Repeats the failed action. */
      retry: async (): Promise<void> => {
        const r = store.retry();
        if (r?.kind === 'clear') await clear(r.ids);
        else if (r?.kind === 'reveal') await reveal(r.id);
      },
      dismiss: () => patchState(store, { result: null, actionError: null, retry: null, revealed: null }),
    };
  }),
);
