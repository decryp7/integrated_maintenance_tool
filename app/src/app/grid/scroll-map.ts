/**
 * Scroll geometry for a virtualised list whose content may be taller than a browser allows.
 *
 * Browsers cap element heights (about 33M px in Chromium, 17M in Firefox), so a list of 50M rows
 * of 26 px (1.3G px) cannot simply be given its true height. The scroll container instead holds a
 * spacer capped at `MAX_SPACER_PX`; the scrollbar position is mapped linearly to a *virtual* offset
 * (the position inside the true, uncapped content), and rows are laid out from the virtual offset.
 * When the content fits under the cap the mapping is the identity, i.e. ordinary native scrolling.
 *
 * Because one physical pixel then stands for many rows, anything that needs row precision (wheel,
 * keyboard, jumps to an anchor) works on the virtual offset and writes the matching scroll position;
 * the virtual offset, not the rounded scrollTop, is the source of truth.
 *
 * All functions are pure and total: NaN, negative or infinite inputs are clamped, never thrown on.
 */

/** Largest scroll spacer we ask the browser for. */
export const MAX_SPACER_PX = 10_000_000;

export interface ScrollGeometry {
  /** Number of rows. */
  readonly total: number;
  readonly rowH: number;
  /** Height of the rows area (without the sticky header). */
  readonly viewportH: number;
  /** True height of all rows. */
  readonly virtualH: number;
  /** Height of the scrollable content, `min(virtualH, cap)`, never below the viewport. */
  readonly physicalH: number;
  /** Largest scrollTop. */
  readonly maxScroll: number;
  /** Largest virtual offset (last row bottom-aligned). */
  readonly maxTop: number;
  /** True when one physical pixel covers more than one virtual pixel. */
  readonly scaled: boolean;
}

function finite(n: number, fallback = 0): number {
  return Number.isFinite(n) ? n : fallback;
}

export function makeGeometry(total: number, rowH: number, viewportH: number, cap = MAX_SPACER_PX): ScrollGeometry {
  const t = Math.max(0, Math.floor(finite(total)));
  const r = Math.max(1, finite(rowH, 1));
  const v = Math.max(0, finite(viewportH));
  const c = Math.max(1, finite(cap, MAX_SPACER_PX));
  const virtualH = t * r;
  const physicalH = Math.max(Math.min(virtualH, c), Math.min(v, virtualH));
  const maxScroll = Math.max(0, physicalH - v);
  const maxTop = Math.max(0, virtualH - v);
  return { total: t, rowH: r, viewportH: v, virtualH, physicalH, maxScroll, maxTop, scaled: maxTop > maxScroll };
}

export function clamp(n: number, lo: number, hi: number): number {
  return Number.isNaN(n) ? lo : Math.min(hi, Math.max(lo, n));
}

/** Virtual offset (px inside the true content) shown at the top for a given scrollTop. */
export function scrollToTop(g: ScrollGeometry, scrollTop: number): number {
  const s = clamp(scrollTop, 0, g.maxScroll);
  if (g.maxScroll === 0) return 0;
  if (!g.scaled) return s;
  return s === g.maxScroll ? g.maxTop : (s / g.maxScroll) * g.maxTop;
}

/** The scrollTop that displays a given virtual offset (inverse of `scrollToTop`, up to pixel rounding). */
export function topToScroll(g: ScrollGeometry, top: number): number {
  const t = clamp(top, 0, g.maxTop);
  if (g.maxTop === 0) return 0;
  if (!g.scaled) return t;
  return t === g.maxTop ? g.maxScroll : (t / g.maxTop) * g.maxScroll;
}

/**
 * Index of the first row for a scrollTop: `round(scrollTop / maxScroll * (total - visibleRows))`.
 * This is the integer row the scrollbar points at; it is 0 at the top and `total - visibleRows` at the bottom.
 */
export function firstRowAtScroll(g: ScrollGeometry, scrollTop: number): number {
  if (g.total === 0 || g.maxScroll === 0) return 0;
  const visibleRows = g.viewportH / g.rowH;
  const lastFirst = Math.max(0, g.total - visibleRows);
  return Math.round(clamp(scrollTop, 0, g.maxScroll) / g.maxScroll * lastFirst);
}

/** First row index containing the virtual offset. */
export function rowAtTop(g: ScrollGeometry, top: number): number {
  return g.total === 0 ? 0 : clamp(Math.floor(clamp(top, 0, g.maxTop) / g.rowH), 0, g.total - 1);
}

export interface RowWindow {
  /** First row to render (inclusive). */
  first: number;
  /** One past the last row to render. */
  last: number;
}

export function visibleWindow(g: ScrollGeometry, top: number, overscan: number): RowWindow {
  if (g.total === 0) return { first: 0, last: 0 };
  const t = clamp(top, 0, g.maxTop);
  const o = Math.max(0, Math.floor(finite(overscan)));
  const first = Math.max(0, Math.floor(t / g.rowH) - o);
  const last = Math.min(g.total, Math.ceil((t + g.viewportH) / g.rowH) + o);
  return { first, last: Math.max(first, last) };
}

/** Pixel offset of a row's top inside the viewport for a given virtual offset. */
export function rowY(g: ScrollGeometry, index: number, top: number): number {
  return index * g.rowH - top;
}

export type RevealMode = 'nearest' | 'center' | 'start';

/**
 * Virtual offset that reveals row `index`. `nearest` leaves `current` alone when the row is fully
 * visible (keyboard navigation), `center` puts it in the middle (jump to anchor), `start` at the top.
 * Landing is exact: the result is clamped to `[0, maxTop]` and never moves a visible row.
 */
export function topForRow(g: ScrollGeometry, index: number, mode: RevealMode, current = 0): number {
  if (g.total === 0) return 0;
  const i = clamp(Math.floor(finite(index)), 0, g.total - 1);
  const y = i * g.rowH;
  const cur = clamp(current, 0, g.maxTop);
  switch (mode) {
    case 'start':
      return clamp(y, 0, g.maxTop);
    case 'center':
      return clamp(y - g.viewportH / 2 + g.rowH / 2, 0, g.maxTop);
    case 'nearest':
      if (y < cur) return clamp(y, 0, g.maxTop);
      if (y + g.rowH > cur + g.viewportH) return clamp(y + g.rowH - g.viewportH, 0, g.maxTop);
      return cur;
  }
}

/** Rows moved by PageUp/PageDown: one screen minus one row, at least 1. */
export function pageRows(g: ScrollGeometry): number {
  return Math.max(1, Math.floor(g.viewportH / g.rowH) - 1);
}

/** Wheel `deltaMode` values. */
export const DOM_DELTA = { pixel: 0, line: 1, page: 2 } as const;

/** Virtual pixels for a wheel event, whatever its `deltaMode`. Lines are 3 rows (as browsers scroll), pages one screen. */
export function wheelToPixels(deltaY: number, deltaMode: number, g: ScrollGeometry): number {
  const d = finite(deltaY);
  if (deltaMode === DOM_DELTA.line) return d * g.rowH * 3;
  if (deltaMode === DOM_DELTA.page) return d * Math.max(g.rowH, g.viewportH - g.rowH);
  return d;
}

/** Next row for a navigation key, or null if the key is not one. `current` is -1 when nothing is selected. */
export function navigate(g: ScrollGeometry, key: string, current: number): number | null {
  if (g.total === 0) return null;
  const last = g.total - 1;
  const from = current < 0 ? -1 : Math.min(current, last);
  switch (key) {
    case 'ArrowDown':
      return clamp(from + 1, 0, last);
    case 'ArrowUp':
      return clamp(from < 0 ? 0 : from - 1, 0, last);
    case 'PageDown':
      return clamp(from + pageRows(g), 0, last);
    case 'PageUp':
      return clamp(from < 0 ? 0 : from - pageRows(g), 0, last);
    case 'Home':
      return 0;
    case 'End':
      return last;
    default:
      return null;
  }
}
