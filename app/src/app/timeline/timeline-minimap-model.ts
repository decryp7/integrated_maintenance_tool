/**
 * The timeline minimap's pure geometry: where the visible range sits within the whole data extent, and the range each
 * gesture (drag the window, drag an edge, click to centre, arrow keys) leads to. Times are epoch ms.
 */
import { clamp } from '../core/format';
import { clampRange, MIN_SPAN_MS, pan, Range, span } from './zoom';

/** The window's position as fractions of the extent (0..1). */
export interface WindowRect {
  left: number;
  width: number;
}

/** Narrowest window drawn (a share of the strip), so a tiny range stays visible and grabbable. */
export const MIN_WINDOW_FRACTION = 0.012;

/** The visible `view` within `extent`; null when there is no usable extent. */
export function windowRect(view: Range, extent: Range): WindowRect | null {
  const total = span(extent);
  if (total <= 0) return null;
  const left = clamp((view.from - extent.from) / total, 0, 1);
  const right = clamp((view.to - extent.from) / total, 0, 1);
  const width = Math.min(1 - left, Math.max(MIN_WINDOW_FRACTION, right - left));
  return { left: Math.min(left, 1 - width), width };
}

/** The time at fraction `f` (0..1) of the extent. */
export function fractionToTs(f: number, extent: Range): number {
  return extent.from + clamp(f, 0, 1) * span(extent);
}

/** The range the same size as `view`, centred on `ts`, kept inside the extent (a click on the strip). */
export function centreOn(ts: number, view: Range, extent: Range): Range {
  const half = span(view) / 2;
  return clampRange({ from: ts - half, to: ts + half }, extent);
}

export type WindowDrag = 'move' | 'left' | 'right';

/** The range after dragging by `deltaMs` from `start`: the whole window, or one edge (never narrower than `minSpan`). */
export function dragRange(kind: WindowDrag, start: Range, deltaMs: number, extent: Range, minSpan = MIN_SPAN_MS): Range {
  switch (kind) {
    case 'move':
      return pan(start, deltaMs, extent);
    case 'left':
      return { from: clamp(start.from + deltaMs, extent.from, start.to - minSpan), to: start.to };
    case 'right':
      return { from: start.from, to: clamp(start.to + deltaMs, start.from + minSpan, extent.to) };
  }
}

/** The range after an arrow key: a step of `fraction` of the window's own span left (-1) or right (1). */
export function stepRange(view: Range, extent: Range, direction: -1 | 1, fraction = 0.1): Range {
  return pan(view, direction * span(view) * fraction, extent);
}
