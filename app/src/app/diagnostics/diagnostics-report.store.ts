import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { describeError } from '../core/logger';
import { DiagnosticsReportState, DiagnosticsReportView } from './diagnostics-report.types';
import { SnapshotBackend } from './snapshot-backend';

interface State {
  report: DiagnosticsReportView | null;
  loading: boolean;
  error: string | null;
  /** The snapshot shown large. */
  selected: string | null;
  /** Loaded images by snapshot id (data URLs). */
  images: Record<string, string>;
  /** Snapshot ids whose image could not be loaded. */
  imageErrors: Record<string, string>;
}

/** State of the "Diagnostics report" view: the report this window opened, read-only. */
export const DiagnosticsReportStore = signalStore(
  { providedIn: 'root' },
  withState<State>({ report: null, loading: false, error: null, selected: null, images: {}, imageErrors: {} }),
  withComputed((s) => ({
    current: computed(() => s.report()?.snapshots.find((x) => x.id === s.selected()) ?? null),
  })),
  withMethods((store, backend = inject(SnapshotBackend)) => {
    const loadImage = async (id: string): Promise<void> => {
      if (store.images()[id]) return;
      try {
        const url = await backend.diagnosticsReportImage(id);
        patchState(store, (st) => ({ images: { ...st.images, [id]: url }, imageErrors: omit(st.imageErrors, id) }));
      } catch (e) {
        patchState(store, (st) => ({ imageErrors: { ...st.imageErrors, [id]: describeError(e) } }));
      }
    };
    return {
      /** Asks the engine for the report; resolves to its answer, or null when the request failed (then `error` is set). */
      async refresh(): Promise<DiagnosticsReportState | null> {
        patchState(store, { loading: true, error: null });
        try {
          const state = await backend.diagnosticsReport();
          const sameReport = state.report !== null && state.report.name === store.report()?.name;
          patchState(store, (st) => ({
            report: state.report,
            loading: false,
            ...(sameReport ? {} : { selected: null, images: {}, imageErrors: {} }),
            ...(state.report?.snapshots.some((x) => x.id === st.selected) ? {} : { selected: null }),
          }));
          return state;
        } catch (e) {
          patchState(store, { loading: false, error: describeError(e) });
          return null;
        }
      },
      /** Loads the thumbnail of every snapshot of the report. */
      async loadImages(): Promise<void> {
        await Promise.all((store.report()?.snapshots ?? []).map((x) => loadImage(x.id)));
      },
      /** Opens (or, with null, closes) the large view of one snapshot. */
      async show(id: string | null): Promise<void> {
        patchState(store, { selected: id });
        if (id) await loadImage(id);
      },
    };
  }),
);

function omit(map: Record<string, string>, key: string): Record<string, string> {
  const { [key]: _removed, ...rest } = map;
  return rest;
}
