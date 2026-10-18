import { computed, Injectable, signal } from '@angular/core';
import { RecordView } from '../core/api.types';

const sameRecords = (a: readonly RecordView[], b: readonly RecordView[]) => a.length === b.length && a.every((r, i) => r.id === b[i]?.id);

/**
 * The events grid's multi-selection, for features outside the grid (palette commands, panels) that act on
 * "the selected lines". The grid publishes the loaded records of its selection, in grid order and capped;
 * everyone else only reads.
 */
@Injectable({ providedIn: 'root' })
export class GridSelection {
  private readonly selected = signal<readonly RecordView[]>([], { equal: sameRecords });

  /** The selected records the grid has in memory, in grid order (empty when nothing is multi-selected). */
  readonly records = this.selected.asReadonly();
  readonly count = computed(() => this.selected().length);

  /** Called by the grid whenever its selection or loaded rows change. */
  publish(records: readonly RecordView[]): void {
    this.selected.set([...records]);
  }

  clear(): void {
    this.selected.set([]);
  }

  /** The records an action on "the selected lines" should use: the grid's selection, else `fallback` alone, else none. */
  recordsOr(fallback: RecordView | null): readonly RecordView[] {
    const records = this.selected();
    if (records.length) return records;
    return fallback ? [fallback] : [];
  }
}
