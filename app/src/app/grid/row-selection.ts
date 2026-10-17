/**
 * Pure multi-row selection model for the events grid, by row index.
 *
 * Selected rows are kept as sorted, disjoint, inclusive index ranges, so selecting a range of millions of
 * rows stays cheap. `anchor` is where shift-extension starts; `focus` is the row the user acted on last
 * (the one the inspector shows); `base` is the selection a shift-extension adds its range to, so repeated
 * shift+clicks from the same anchor replace the previous range instead of accumulating.
 */

/** Inclusive index range. */
export interface IndexRange {
  readonly start: number;
  readonly end: number;
}

export interface RowSelection {
  readonly ranges: readonly IndexRange[];
  readonly anchor: number;
  readonly focus: number;
  readonly base: readonly IndexRange[];
}

export const EMPTY_SELECTION: RowSelection = { ranges: [], anchor: -1, focus: -1, base: [] };

/** Sorts and merges overlapping or adjacent ranges. */
export function normalize(ranges: readonly IndexRange[]): IndexRange[] {
  const sorted = ranges.filter((r) => r.end >= r.start).sort((a, b) => a.start - b.start);
  const out: IndexRange[] = [];
  for (const r of sorted) {
    const last = out.at(-1);
    if (last && r.start <= last.end + 1) out[out.length - 1] = { start: last.start, end: Math.max(last.end, r.end) };
    else out.push({ start: r.start, end: r.end });
  }
  return out;
}

/** Plain click or plain keyboard move: only this row. */
export function single(index: number): RowSelection {
  const ranges = index < 0 ? [] : [{ start: index, end: index }];
  return { ranges, anchor: index, focus: index, base: [] };
}

export function contains(sel: RowSelection, index: number): boolean {
  return sel.ranges.some((r) => index >= r.start && index <= r.end);
}

/** Number of selected rows. */
export function count(sel: RowSelection): number {
  return sel.ranges.reduce((n, r) => n + r.end - r.start + 1, 0);
}

/** Ctrl/Cmd+click: adds or removes one row; it becomes the anchor for a following shift-extension. */
export function toggle(sel: RowSelection, index: number): RowSelection {
  let ranges: IndexRange[];
  if (contains(sel, index)) {
    ranges = sel.ranges.flatMap((r) =>
      index < r.start || index > r.end
        ? [r]
        : [
            { start: r.start, end: index - 1 },
            { start: index + 1, end: r.end },
          ].filter((p) => p.end >= p.start),
    );
  } else {
    ranges = normalize([...sel.ranges, { start: index, end: index }]);
  }
  return { ranges, anchor: index, focus: index, base: ranges };
}

/** Shift+click or shift+arrow: the base selection plus the range from the anchor to `index`. */
export function extendTo(sel: RowSelection, index: number): RowSelection {
  if (sel.anchor < 0) return single(index);
  const range = { start: Math.min(sel.anchor, index), end: Math.max(sel.anchor, index) };
  return { ranges: normalize([...sel.base, range]), anchor: sel.anchor, focus: index, base: sel.base };
}

/** Moves the focus without changing what is selected (e.g. right-click inside the selection). */
export function withFocus(sel: RowSelection, index: number): RowSelection {
  return { ...sel, focus: index };
}

/** New rows were inserted above every selected row (live tail, newest first): move the selection down. */
export function shift(sel: RowSelection, delta: number): RowSelection {
  if (sel.anchor < 0 && !sel.ranges.length) return sel;
  const move = (rs: readonly IndexRange[]) => rs.map((r) => ({ start: r.start + delta, end: r.end + delta }));
  return { ranges: move(sel.ranges), base: move(sel.base), anchor: sel.anchor + delta, focus: sel.focus + delta };
}

/** Selected indexes in ascending order, at most `limit` of them. */
export function indices(sel: RowSelection, limit: number): number[] {
  const out: number[] = [];
  for (const r of sel.ranges) {
    for (let i = r.start; i <= r.end; i++) {
      if (out.length >= limit) return out;
      out.push(i);
    }
  }
  return out;
}
