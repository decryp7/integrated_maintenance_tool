import { FlameModel, FlameOrientation } from './flame-model';
import { FlamePalette, inkFor } from './flame-colors';
import { shortName } from '../profiling-format';

/** The part of CanvasRenderingContext2D the flame graph uses (tests pass a recorder). */
export type FlameCtx = Pick<CanvasRenderingContext2D, 'clearRect' | 'fillRect' | 'strokeRect' | 'fillText' | 'measureText' | 'save' | 'restore' | 'beginPath' | 'rect' | 'clip'> & {
  fillStyle: string | CanvasGradient | CanvasPattern;
  strokeStyle: string | CanvasGradient | CanvasPattern;
  globalAlpha: number;
  lineWidth: number;
  font: string;
  textBaseline: CanvasTextBaseline;
};

export const FLAME_ROW_H = 18;
/** Gap between neighbouring rectangles (the surface shows through). */
const GAP = 1;
const MIN_LABEL_PX = 28;

export interface DrawOptions {
  width: number;
  height: number;
  orientation: FlameOrientation;
  color: (index: number) => string;
  palette: FlamePalette;
  /** A search is active: unmatched rectangles recede. */
  dimUnmatched: boolean;
  /** Keyboard/selection cursor and hover, outlined. */
  current: number | null;
  hovered: number | null;
  font: string;
}

/** Top y of a depth row. */
export function rowY(depth: number, maxDepth: number, orientation: FlameOrientation, rowH = FLAME_ROW_H): number {
  return orientation === 'icicle' ? depth * rowH : (maxDepth - depth) * rowH;
}

/** Depth at a y coordinate (may be out of range). */
export function depthAt(y: number, maxDepth: number, orientation: FlameOrientation, rowH = FLAME_ROW_H): number {
  const row = Math.floor(y / rowH);
  return orientation === 'icicle' ? row : maxDepth - row;
}

/** Text that fits `max` px, ending in "…" when cut. */
export function fitLabel(ctx: Pick<FlameCtx, 'measureText'>, text: string, max: number): string {
  if (ctx.measureText(text).width <= max) return text;
  let lo = 0;
  let hi = text.length;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (ctx.measureText(text.slice(0, mid) + '…').width <= max) lo = mid;
    else hi = mid - 1;
  }
  return lo ? text.slice(0, lo) + '…' : '';
}

/** Draws the rectangles with labels where they fit. */
export function drawFlame(ctx: FlameCtx, model: FlameModel, o: DrawOptions): void {
  ctx.clearRect(0, 0, o.width, o.height);
  ctx.font = o.font;
  ctx.textBaseline = 'middle';
  model.rects.forEach((r, i) => {
    const x = Math.max(0, r.x * o.width);
    const right = Math.min(o.width, (r.x + r.w) * o.width);
    const w = right - x;
    if (w <= 0) return;
    const y = rowY(r.depth, model.maxDepth, o.orientation);
    const fill = o.color(i);
    ctx.globalAlpha = o.dimUnmatched && !r.matched ? 0.3 : 1;
    ctx.fillStyle = fill;
    ctx.fillRect(x, y, Math.max(0.5, w - GAP), FLAME_ROW_H - GAP);
    if (w >= MIN_LABEL_PX) {
      const name = model.frames.get(r.frame)?.name ?? `#${r.frame}`;
      const label = fitLabel(ctx, shortName(name), w - 8);
      if (label) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(x, y, w - GAP, FLAME_ROW_H);
        ctx.clip();
        ctx.fillStyle = inkFor(fill, o.palette);
        ctx.fillText(label, x + 4, y + FLAME_ROW_H / 2);
        ctx.restore();
      }
    }
  });
  ctx.globalAlpha = 1;
  for (const [i, width] of [
    [o.hovered, 1],
    [o.current, 2],
  ] as const) {
    if (i === null || !model.rects[i]) continue;
    const r = model.rects[i];
    ctx.strokeStyle = o.palette.outline;
    ctx.lineWidth = width;
    ctx.strokeRect(Math.max(0, r.x * o.width) + 0.5, rowY(r.depth, model.maxDepth, o.orientation) + 0.5, Math.max(1, r.w * o.width - 1 - GAP), FLAME_ROW_H - 1 - GAP);
  }
  if (o.dimUnmatched) {
    ctx.strokeStyle = o.palette.outline;
    ctx.lineWidth = 1;
    model.rects.forEach((r) => {
      if (r.matched && r.w * o.width >= 2) ctx.strokeRect(Math.max(0, r.x * o.width) + 0.5, rowY(r.depth, model.maxDepth, o.orientation) + 0.5, r.w * o.width - 1 - GAP, FLAME_ROW_H - 1 - GAP);
    });
  }
}
