import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { errorMessage } from '../../tools/tool-errors';
import { StorageBackend } from './storage-backend';
import { JobsBackend } from '../../jobs/jobs-backend';
import { orphaned, stillOrphaned } from './stores-format';
import { StorageCategory, StorageCategoryId, StorageClearResult, StoragePolicy, StorageUsage, StoreClearResult, StoreFolder, StoreFolders } from './storage.types';
import { setError, setLoaded, setLoading, withRequestStatus } from '../../shared/store/with-request-status';

interface StorageState {
  usage: StorageUsage | null;
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
  /** The store folders on disk (Project stores). */
  stores: StoreFolders | null;
  storesLoading: boolean;
  /** Listing the stores failed (shown with Retry). */
  storesError: string | null;
  /** The orphaned stores ticked for clean-up. */
  selected: string[];
  /** A store clean-up is running. */
  storesClearing: boolean;
  /** What the last store clean-up did. */
  storesResult: StoreClearResult | null;
  /** The last store clean-up failed. */
  storesActionError: string | null;
  /** The automatic clean-up setting; null until read. */
  policy: StoragePolicy | null;
  /** Reading or saving the setting failed (the engine's message). */
  policyError: string | null;
}

const INITIAL: StorageState = { usage: null, clearing: null, job: null, result: null, actionError: null, retry: null, revealed: null,
  stores: null, storesLoading: false, storesError: null, selected: [], storesClearing: false, storesResult: null, storesActionError: null, policy: null, policyError: null,
};

/**
 * State of Settings ▸ Storage: the engine's usage report and the outcome of clearing. The engine decides
 * what each category holds and what may go; confirmations are the component's business.
 */
export const StorageStore = signalStore(
  { providedIn: 'root' },
  withState<StorageState>(INITIAL),
  withRequestStatus(),
  withComputed((store) => {
    const categories = computed<StorageCategory[]>(() => store.usage()?.categories ?? []);
    /** What "Clear cache" removes: every clearable cache category with something to remove. */
    const cache = computed(() => categories().filter((c) => c.kind === 'cache' && c.clearable && c.clearableBytes > 0));
    return {
      categories,
      cache,
      cacheBytes: computed(() => cache().reduce((n, c) => n + c.clearableBytes, 0)),
      busy: computed(() => store.isLoading() || store.clearing() !== null),
      folders: computed<StoreFolder[]>(() => store.stores()?.folders ?? []),
      orphanedFolders: computed<StoreFolder[]>(() => orphaned(store.stores()?.folders ?? [])),
      /** What cleaning up the ticked stores would free. */
      selectedBytes: computed(() => (store.stores()?.folders ?? []).filter((f) => store.selected().includes(f.id)).reduce((n, f) => n + f.bytes, 0)),
    };
  }),
  withMethods((store, backend = inject(StorageBackend), jobs = inject(JobsBackend)) => {
    const load = async (): Promise<void> => {
      patchState(store, setLoading());
      try {
        patchState(store, { usage: await backend.usage() }, setLoaded());
      } catch (e) {
        patchState(store, setError(errorMessage(e)));
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
    const loadStores = async (): Promise<void> => {
      patchState(store, { storesLoading: true, storesError: null });
      try {
        const stores = await backend.stores();
        patchState(store, (s) => ({ stores, selected: stillOrphaned(s.selected, stores.folders) }));
      } catch (e) {
        patchState(store, { storesError: errorMessage(e) });
      } finally {
        patchState(store, { storesLoading: false });
      }
    };
    const clearStores = async (ids: string[]): Promise<void> => {
      if (!ids.length || store.storesClearing()) return;
      patchState(store, { storesClearing: true, storesActionError: null, storesResult: null });
      try {
        patchState(store, { storesResult: await backend.clearStores(ids) });
      } catch (e) {
        patchState(store, { storesActionError: errorMessage(e) });
      } finally {
        patchState(store, { storesClearing: false });
      }
      await loadStores();
    };
    const loadPolicy = async (): Promise<void> => {
      try {
        patchState(store, { policy: await backend.policy(), policyError: null });
      } catch (e) {
        patchState(store, { policyError: errorMessage(e) });
      }
    };
    return {
      load,
      clear,
      loadStores,
      loadPolicy,
      clearStores,
      /** Ticks or unticks a store; only an orphaned one can be ticked. */
      toggleStore(id: string, on: boolean): void {
        if (!store.orphanedFolders().some((f) => f.id === id)) return;
        patchState(store, (s) => ({ selected: on ? [...new Set([...s.selected, id])] : s.selected.filter((x) => x !== id) }));
      },
      /** Cleans up the ticked stores. */
      clearSelectedStores: () => clearStores(store.selected()),
      /** Cleans up every orphaned store. */
      clearAllOrphaned: () => clearStores(store.orphanedFolders().map((f) => f.id)),
      dismissStores: () => patchState(store, { storesResult: null, storesActionError: null }),
      /** Saves the automatic clean-up setting; a refusal (out of range) is kept as `policyError`. */
      async setAutoCleanDays(autoCleanDays: number): Promise<void> {
        try {
          patchState(store, { policy: await backend.setPolicy({ autoCleanDays }), policyError: null });
        } catch (e) {
          patchState(store, { policyError: errorMessage(e) });
        }
      },
      /** Shows a message about the setting (for input the UI already refused). */
      setPolicyError: (policyError: string | null) => patchState(store, { policyError }),
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
