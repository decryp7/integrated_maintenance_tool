/**
 * Stacking of overlapping intervals into sub-tracks (rows) so that none overlap: greedy interval packing. Taken in
 * start order, each interval goes to the first row that is free again by its start, else to a new row; this uses the
 * fewest rows possible (the most intervals overlapping at any one moment). Pure; used for runs on a timeline lane,
 * e.g. several views opened and closed at different times.
 */

export interface Interval {
  start: number;
  end: number;
}

export interface PackOptions {
  /** Space kept free between two intervals on one row (same unit as the intervals). */
  gap?: number;
  /** At most this many rows; intervals that do not fit share the row that frees up first (they are `overflow`). */
  maxRows?: number;
}

export interface Packing {
  /** The row of each interval, in input order (0 = top). */
  rows: number[];
  /** Rows used (0 for no intervals). */
  count: number;
  /** Intervals placed over another one because `maxRows` was reached. */
  overflow: number;
}

/** Packs `items` into as few rows as possible without overlap (up to `maxRows`). */
export function packRows(items: readonly Interval[], o: PackOptions = {}): Packing {
  const gap = Math.max(0, o.gap ?? 0);
  const maxRows = Math.max(1, Math.floor(o.maxRows ?? Number.POSITIVE_INFINITY));
  const order = items.map((_, i) => i).sort((a, b) => items[a]!.start - items[b]!.start || items[a]!.end - items[b]!.end || a - b);
  /** Where each row is free again. */
  const freeAt: number[] = [];
  const rows = new Array<number>(items.length).fill(0);
  let overflow = 0;
  for (const i of order) {
    const it = items[i]!;
    const end = Math.max(it.start, it.end);
    let row = freeAt.findIndex((f) => f + gap <= it.start);
    if (row < 0 && freeAt.length < maxRows) row = freeAt.push(Number.NEGATIVE_INFINITY) - 1;
    if (row < 0) {
      // Full: share the row that frees up first.
      row = freeAt.indexOf(Math.min(...freeAt));
      overflow++;
    }
    rows[i] = row;
    freeAt[row] = Math.max(freeAt[row]!, end);
  }
  return { rows, count: freeAt.length, overflow };
}
