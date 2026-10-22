import { inject } from '@angular/core';
import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { OffsetChange, OffsetSpec, RecordView, SuggestResult } from '../core/api.types';
import { isUnsupported, sentenceText } from '../core/cmd-error';
import { describeError } from '../core/logger';
import { SkewBackend } from '../core/skew-backend';
import { WorkspaceStore } from '../core/workspace.store';
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
  /** The engine does not offer clock correction (yet): what it says, shown as a calm note instead of an error. */
  unavailable: string | null;
  suggestion: SuggestResult | null;
  suggesting: boolean;
  lastChange: OffsetChange | null;
}

const initial: ClockSkewState = { reference: null, busy: false, error: null, unavailable: null, suggestion: null, suggesting: false, lastChange: null };

/**
 * Clock-skew correction state: the reference event, suggestions and applying offsets. The engine
 * computes and applies offsets; this store keeps the selection and the reference on the moved records.
 */
export const ClockSkewStore = signalStore(
  { providedIn: 'root' },
  withState<ClockSkewState>(initial),
  withMethods((store, backend = inject(SkewBackend), workspace = inject(WorkspaceStore)) => ({
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
     * view reloads) and the selection and the reference keep their records. Resolves with the change, or
     * null after a failure (shown as `error`).
     */
    async apply(spec: OffsetSpec): Promise<OffsetChange | null> {
      patchState(store, { busy: true, error: null });
      const selected = workspace.selected();
      const reference = store.reference();
      try {
        const change = await backend.setSourceOffset(spec);
        // Event ids are stable when events move in time: the selection and the reference keep theirs.
        if (reference) {
          patchState(store, { reference: { ...reference, ts: reference.source === change.source ? reference.ts + change.offsetMs - change.previousMs : reference.ts } });
        }
        await workspace.refreshSummary();
        if (selected) workspace.jumpTo(selected.id);
        const source = sourceLabel(workspace, change.source);
        const offset = formatOffset(change.offsetMs);
        workspace.setNotice($localize`:@@skew.store.shifted:Clock of ${source}:source: shifted by ${offset}:offset:`);
        patchState(store, { busy: false, lastChange: change, suggestion: null });
        return change;
      } catch (e) {
        patchState(store, { busy: false, ...failure(e) });
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
        patchState(store, { suggesting: false, ...failure(e) });
      }
    },
  })),
);

/** A failure as state: `unavailable` when the engine does not offer the operation, otherwise `error`. */
function failure(e: unknown): Pick<ClockSkewState, 'error' | 'unavailable'> {
  return isUnsupported(e) ? { error: null, unavailable: sentenceText(e) } : { error: describeError(e), unavailable: null };
}

function sourceLabel(workspace: InstanceType<typeof WorkspaceStore>, id: number): string {
  return workspace.summary()?.sources.find((s) => s.id === id)?.name ?? $localize`:@@skew.dialog.sourceFallback:source ${id}:id:`;
}
