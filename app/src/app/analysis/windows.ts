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
