import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { describeError } from '../core/logger';
import { ErrorSnapshotService } from './error-snapshot.service';
import { ExportTarget } from './export-target';
import { SnapshotBackend } from './snapshot-backend';
import { SnapshotInfo } from '../core/api.types';

interface State {
  items: SnapshotInfo[];
  /** Ticked ids for export / delete. */
  selected: string[];
  /** The snapshot shown large. */
  open: string | null;
  /** Loaded images by id (data URLs). */
  images: Record<string, string>;
  loading: boolean;
  busy: boolean;
  error: string | null;
  info: string | null;
}

/** State of the "Error snapshots" view. */
export const SnapshotsStore = signalStore(
  { providedIn: 'root' },
  withState<State>({ items: [], selected: [], open: null, images: {}, loading: false, busy: false, error: null, info: null }),
  withComputed((s) => ({
    current: computed(() => s.items().find((i) => i.id === s.open()) ?? null),
    allSelected: computed(() => s.items().length > 0 && s.selected().length === s.items().length),
  })),
  withMethods((store, backend = inject(SnapshotBackend), service = inject(ErrorSnapshotService), target = inject(ExportTarget)) => {
    const fail = (e: unknown) => patchState(store, { error: describeError(e), info: null });
    const loadImage = async (id: string) => {
      if (store.images()[id]) return;
      try {
        const url = await backend.image(id);
        patchState(store, (st) => ({ images: { ...st.images, [id]: url } }));
      } catch {
        /* a missing image shows a placeholder */
      }
    };
    const refresh = async () => {
      patchState(store, { loading: true, error: null });
      try {
        const items = await backend.list();
        patchState(store, (st) => ({
          items,
          loading: false,
          selected: st.selected.filter((id) => items.some((i) => i.id === id)),
          open: items.some((i) => i.id === st.open) ? st.open : null,
        }));
        for (const item of items) void loadImage(item.id);
      } catch (e) {
        patchState(store, { loading: false });
        fail(e);
      }
    };
    return {
      refresh,
      show(id: string | null) {
        patchState(store, { open: id });
      },
      toggle(id: string) {
        patchState(store, (st) => ({ selected: st.selected.includes(id) ? st.selected.filter((x) => x !== id) : [...st.selected, id] }));
      },
      toggleAll() {
        patchState(store, (st) => ({ selected: st.selected.length === st.items.length ? [] : st.items.map((i) => i.id) }));
      },
      async remove(ids: string[]) {
        patchState(store, { busy: true, error: null, info: null });
        let failure: unknown = null;
        try {
          for (const id of ids) await backend.remove(id);
        } catch (e) {
          failure = e;
        }
        patchState(store, { busy: false });
        await refresh();
        if (failure !== null) fail(failure);
      },
      async openFolder() {
        try {
          patchState(store, { info: `Folder: ${await backend.openFolder()}`, error: null });
        } catch (e) {
          fail(e);
        }
      },
      /** Asks where to save, then zips the selected snapshots (or all when none is ticked) with the recent IMT log. */
      async exportSelected() {
        const ids = store.selected().length ? store.selected() : store.items().map((i) => i.id);
        if (!ids.length) return;
        patchState(store, { busy: true, error: null, info: null });
        try {
          const path = await target.pickZip('imt-diagnostics.zip');
          if (path) {
            const files = await backend.exportDiagnostics(path, ids);
            patchState(store, { info: `Exported ${ids.length} snapshot${ids.length === 1 ? '' : 's'} (${files} files) to ${path}` });
          }
        } catch (e) {
          fail(e);
        }
        patchState(store, { busy: false });
      },
      async captureNow() {
        await service.manual();
      },
    };
  }),
);
