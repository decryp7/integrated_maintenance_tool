/** Smallest vertical distance between two end labels, px (a 10.5 px label plus a little air). */
export const END_LABEL_GAP = 13;

/**
 * Spreads end labels that would overprint: given each label's natural y (px), returns the vertical offset to apply to
 * each so that neighbours are at least `gap` apart and every label stays within [min, max]. Order along y is kept
 * (ties keep input order); labels already apart get offset 0. Pure, so it is unit-testable without a chart.
 */
export function spreadLabels(ys: readonly number[], gap = END_LABEL_GAP, min = -Infinity, max = Infinity): number[] {
  const order = ys.map((y, i) => i).sort((a, b) => (ys[a] ?? 0) - (ys[b] ?? 0) || a - b);
  const pos = order.map((i) => Math.min(Math.max(ys[i] ?? 0, min), max));
  for (let k = 1; k < pos.length; k++) pos[k] = Math.max(pos[k] ?? 0, (pos[k - 1] ?? 0) + gap);
  // Pushed past the bottom edge: pull the stack back up from the end.
  const last = pos.length - 1;
  if (last >= 0 && (pos[last] ?? 0) > max) {
    pos[last] = max;
    for (let k = last - 1; k >= 0; k--) pos[k] = Math.min(pos[k] ?? 0, (pos[k + 1] ?? 0) - gap);
  }
  const out = new Array<number>(ys.length).fill(0);
  order.forEach((i, k) => (out[i] = (pos[k] ?? 0) - (ys[i] ?? 0)));
  return out;
}
