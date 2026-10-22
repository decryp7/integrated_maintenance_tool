import { effect, inject, Injectable, Injector, untracked } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { UndoStack } from './undo-stack';

/** The time range of the view: what zooming and panning change. */
export interface TimeRange {
  from: number | null;
  to: number | null;
}

const ZOOM_LABEL = $localize`:Undo history entry|@@undo.range.zoom:zoom`;

export function rangeOf(q: { from?: number | null; to?: number | null }): TimeRange {
  return { from: q.from ?? null, to: q.to ?? null };
}

/**
 * Records zooming and panning on the undo stack, so Undo takes the timeline back to where it was (the filters are
 * `QueryUndo`'s). A stream of small steps (a drag, the wheel) is one entry. A sliding live window is not recorded:
 * the engine moves it by itself.
 */
@Injectable({ providedIn: 'root' })
export class RangeUndo {
  private readonly store = inject(WorkspaceStore);
  private readonly undo = inject(UndoStack);
  private readonly injector = inject(Injector);
  private last: TimeRange | null = null;
  private started = false;

  /** Starts observing (idempotent). */
  start(): void {
    if (this.started) return;
    this.started = true;
    effect(
      () => {
        const q = this.store.query();
        if (q.window) return;
        untracked(() => this.observe(rangeOf(q)));
      },
      { injector: this.injector },
    );
  }

  /** Takes the current range as the new starting point without recording (a project or session was opened). */
  rebase(): void {
    this.last = rangeOf(this.store.query());
  }

  /** @internal Exposed for tests. */
  observe(next: TimeRange): void {
    const prev = this.last;
    this.last = next;
    if (!prev || (prev.from === next.from && prev.to === next.to)) return;
    this.undo.record({ label: ZOOM_LABEL, mergeKey: 'query.range', undo: () => this.restore(prev), do: () => this.restore(next) });
  }

  private restore(range: TimeRange): void {
    // Seen as the starting point, so the effect does not record the restore itself.
    this.last = range;
    this.store.restoreQuery({ ...this.store.query(), ...range });
  }
}
