/**
 * The events-per-bucket histogram above the lanes, as SVG paths (pure). One path per level, bars stacked from the
 * baseline with the most severe first, so errors sit on the axis and are never hidden behind a tall info bar; a
 * bucket's error and warning segments keep a minimum height when present, so a single error still shows.
 */
import { Level, LEVELS } from '../core/api.types';
import { HIST_VARS } from '../shared/colors';

/** Height of the drawing space in viewBox units; CSS scales it to `--hist-h`. */
export const HIST_VB = 100;
/** Kept free above the busiest bucket (viewBox units). */
const TOP_PAD = 4;
/** Stack order from the baseline. */
export const HIST_STACK: readonly Level[] = ['Fatal', 'Error', 'Warning', 'Information', 'Debug', 'Verbose'];
/** Smallest segment drawn for a level that is present in a bucket (viewBox units). */
const MIN_SEGMENT: Partial<Record<Level, number>> = { Fatal: 3, Error: 3, Warning: 2 };
/** Bar width as a share of the bucket; the rest is the gap between bars. */
const BAR = 0.76;

export interface HistPath {
  level: Level;
  d: string;
  color: string;
}

export interface Histogram {
  paths: HistPath[];
  /** Events in the busiest bucket (the scale). */
  max: number;
}

/** The histogram of `buckets` (`[bucket][level index in LEVELS order]`). */
export function histogramPaths(buckets: readonly (readonly number[])[]): Histogram {
  let max = 0;
  for (const b of buckets) max = Math.max(max, b.reduce((a, c) => a + c, 0));
  const scale = (HIST_VB - TOP_PAD) / Math.max(1, max);
  const parts = new Map<Level, string[]>(HIST_STACK.map((l) => [l, []]));
  const x0 = (1 - BAR) / 2;
  buckets.forEach((b, x) => {
    let y = HIST_VB;
    for (const level of HIST_STACK) {
      const count = b[LEVELS.indexOf(level)] ?? 0;
      if (count <= 0) continue;
      const h = Math.min(y, Math.max(count * scale, MIN_SEGMENT[level] ?? 0));
      if (h <= 0) continue;
      y -= h;
      parts.get(level)?.push(`M${(x + x0).toFixed(2)} ${y.toFixed(2)}h${BAR}v${h.toFixed(2)}h-${BAR}z`);
    }
  });
  const paths = HIST_STACK.map((level) => ({ level, d: parts.get(level)?.join('') ?? '', color: HIST_VARS[level] })).filter((p) => p.d);
  return { paths, max };
}
