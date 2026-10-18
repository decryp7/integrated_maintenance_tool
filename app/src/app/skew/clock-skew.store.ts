import { inject } from '@angular/core';
import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { OffsetChange, OffsetSpec, RecordView, SuggestResult } from '../core/api.types';
import { describeError } from '../core/logger';
import { SkewBackend } from '../core/skew-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { BookmarkStore } from '../projects/bookmarks/bookmark.store';
import { formatOffset } from './skew-format';

/** The event another source is aligned to: what the user marked with "Use as clock reference". */
export interface ClockReference {
  id: number;
  ts: number;
  source: number;
  sourceName: string;
  message: string;
}

interface ClockSkewState {
  reference: ClockReference | null;
  busy: boolean;
  error: string | null;
  suggestion: SuggestResult | null;
  suggesting: boolean;
  lastChange: OffsetChange | null;
}

const initial: ClockSkewState = { reference: null, busy: false, error: null, suggestion: null, suggesting: false, lastChange: null };

/**
 * Clock-skew correction state: the reference event, suggestions and applying offsets. The engine
 * computes and applies offsets; this store keeps the selection and bookmarks on the moved records.
 */
export const ClockSkewStore = signalStore(
  { providedIn: 'root' },
  withState<ClockSkewState>(initial),
  withMethods((store, backend = inject(SkewBackend), workspace = inject(WorkspaceStore), bookmarks = inject(BookmarkStore)) => ({
    /** Marks a timed event as the reference other sources are aligned to. Returns false for an untimed one. */
    setReference(record: RecordView | null): boolean {
      if (!record || record.ts === null) return false;
      patchState(store, { reference: { id: record.id, ts: record.ts, source: record.source, sourceName: record.sourceName, message: record.message } });
      return true;
    },
    clearReference: () => patchState(store, { reference: null }),
    clearError: () => patchState(store, { error: null }),

    /**
     * Applies an offset. The engine moves the source's records; the summary is fetched again (every
     * view reloads) and the selection and bookmarks follow their records. Resolves with the change, or
     * null after a failure (shown as `error`).
     */
    async apply(spec: OffsetSpec): Promise<OffsetChange | null> {
      patchState(store, { busy: true, error: null });
      const selected = workspace.selected();
      const marked = bookmarks.items().flatMap((b) => (b.kind === 'record' && b.recordId !== undefined ? [b.recordId] : []));
      const reference = store.reference();
      const track = [...new Set([...(selected ? [selected.id] : []), ...(reference ? [reference.id] : []), ...marked])];
      try {
        const change = await backend.setSourceOffset({ ...spec, track });
        const moved = new Map<number, number | null>(track.map((id, i) => [id, change.tracked[i] ?? null]));
        bookmarks.remapRecords(new Map([...moved].filter(([a, b]) => b !== null && a !== b)));
        if (reference) {
          const id = moved.get(reference.id);
          patchState(store, { reference: id == null ? null : { ...reference, id, ts: reference.source === change.source ? reference.ts + change.offsetMs - change.previousMs : reference.ts } });
        }
        await workspace.refreshSummary();
        const next = selected ? moved.get(selected.id) : null;
        if (next != null) workspace.jumpTo(next);
        const source = sourceLabel(workspace, change.source);
        const offset = formatOffset(change.offsetMs);
        workspace.setNotice($localize`:@@skew.store.shifted:Clock of ${source}:source: shifted by ${offset}:offset:`);
        patchState(store, { busy: false, lastChange: change, suggestion: null });
        return change;
      } catch (e) {
        patchState(store, { busy: false, error: describeError(e) });
        return null;
      }
    },

    /** Asks the engine for suggestions (aligned to `reference`, or the best connected source). */
    async suggest(reference: number | null = null): Promise<void> {
      patchState(store, { suggesting: true, error: null });
      try {
        const suggestion = await backend.suggestOffsets(reference === null ? {} : { reference });
        patchState(store, { suggestion, suggesting: false });
      } catch (e) {
        patchState(store, { suggesting: false, error: describeError(e) });
      }
    },
  })),
);

function sourceLabel(workspace: InstanceType<typeof WorkspaceStore>, id: number): string {
  return workspace.summary()?.sources.find((s) => s.id === id)?.name ?? $localize`:@@skew.dialog.sourceFallback:source ${id}:id:`;
}
