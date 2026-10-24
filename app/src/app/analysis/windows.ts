import { TimeRange } from './time-cursor';

/** Pure helpers that build the two windows of a comparison. */

export function span(r: TimeRange): number {
  return r.to - r.from;
}

/** The window of the same length that ends where `r` starts. */
export function previousWindow(r: TimeRange): TimeRange {
  const s = Math.max(1, span(r));
  return { from: r.from - s, to: r.from };
}

/**
 * The windows to compare for `range` out of data spanning `extent`: the range against the one just before it, or, when
 * that one lies (mostly) before the data (the range is all of it), the range's first half against its second half.
 */
export function defaultWindows(range: TimeRange, extent: TimeRange | null): { a: TimeRange; b: TimeRange } {
  const before = previousWindow(range);
  const inData = !extent || before.from >= extent.from - span(before) * 0.1;
  if (inData && (!extent || before.to > extent.from)) return { a: before, b: range };
  const mid = midpoint(range);
  return { a: { from: range.from, to: mid }, b: { from: mid, to: range.to } };
}

/** `[t - width, t]` and `[t, t + width]`. */
export function aroundTime(t: number, width: number): { a: TimeRange; b: TimeRange } {
  const w = Math.max(1, width);
  return { a: { from: t - w, to: t }, b: { from: t, to: t + w } };
}

export function midpoint(r: TimeRange): number {
  return Math.round((r.from + r.to) / 2);
}

export const WIDTHS: { label: string; ms: number }[] = [
  { label: $localize`:@@analysis.windows.minute1:1 minute`, ms: 60_000 },
  { label: $localize`:@@analysis.windows.minutes5:5 minutes`, ms: 5 * 60_000 },
  { label: $localize`:@@analysis.windows.minutes15:15 minutes`, ms: 15 * 60_000 },
  { label: $localize`:@@analysis.windows.hour1:1 hour`, ms: 60 * 60_000 },
];

/** The moment to analyse at `t`: the selected range when it contains `t`, else `width` centred on `t`. */
export function momentAt(t: number, selection: TimeRange | null, width: number): TimeRange {
  if (selection && selection.from <= t && t <= selection.to && selection.to > selection.from) return selection;
  const half = Math.max(1, Math.round(width / 2));
  return { from: t - half, to: t + half };
}
