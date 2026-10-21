import { NoteRegion } from '../../core/api.types';

/**
 * Pure geometry of the image view (ADR 0021 §4): where the image sits on screen (`Viewport`: scale and pan, in
 * screen pixels per image pixel of the original), turning a drag into a region in the original's pixels, zooming
 * around a point and hit-testing regions. The engine re-checks every region against the image's size.
 */

export interface Point {
  x: number;
  y: number;
}

/** Screen position of image pixel (x, y): `x * scale + panX`. */
export interface Viewport {
  scale: number;
  panX: number;
  panY: number;
}

export const MIN_SCALE = 0.05;
export const MAX_SCALE = 16;
/** A drag shorter than this many image pixels on either side is a click, not a region. */
export const MIN_REGION_PX = 2;

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** The image `w`×`h` fit into a `boxW`×`boxH` box (never enlarged past 100 %), centred. */
export function fitViewport(w: number, h: number, boxW: number, boxH: number): Viewport {
  if (w <= 0 || h <= 0 || boxW <= 0 || boxH <= 0) return { scale: 1, panX: 0, panY: 0 };
  const scale = clamp(Math.min(boxW / w, boxH / h, 1), MIN_SCALE, MAX_SCALE);
  return { scale, panX: (boxW - w * scale) / 2, panY: (boxH - h * scale) / 2 };
}

export function toImage(p: Point, vp: Viewport): Point {
  return { x: (p.x - vp.panX) / vp.scale, y: (p.y - vp.panY) / vp.scale };
}

export function toScreen(p: Point, vp: Viewport): Point {
  return { x: p.x * vp.scale + vp.panX, y: p.y * vp.scale + vp.panY };
}

/** Zooms by `factor` keeping the screen point `at` over the same image pixel. */
export function zoomAt(vp: Viewport, factor: number, at: Point): Viewport {
  const scale = clamp(vp.scale * factor, MIN_SCALE, MAX_SCALE);
  const img = toImage(at, vp);
  return { scale, panX: at.x - img.x * scale, panY: at.y - img.y * scale };
}

export function panBy(vp: Viewport, dx: number, dy: number): Viewport {
  return { ...vp, panX: vp.panX + dx, panY: vp.panY + dy };
}

/**
 * The region a drag from screen point `a` to `b` marks on a `w`×`h` image: in whole image pixels, clamped to the
 * image; null when it is (nearly) empty or entirely outside.
 */
export function regionFromDrag(a: Point, b: Point, vp: Viewport, w: number, h: number): NoteRegion | null {
  const p = toImage(a, vp);
  const q = toImage(b, vp);
  const x0 = clamp(Math.floor(Math.min(p.x, q.x)), 0, w);
  const y0 = clamp(Math.floor(Math.min(p.y, q.y)), 0, h);
  const x1 = clamp(Math.ceil(Math.max(p.x, q.x)), 0, w);
  const y1 = clamp(Math.ceil(Math.max(p.y, q.y)), 0, h);
  if (x1 - x0 < MIN_REGION_PX || y1 - y0 < MIN_REGION_PX) return null;
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

/** The region's box on screen (px). */
export function screenBox(r: NoteRegion, vp: Viewport): { left: number; top: number; width: number; height: number } {
  const p = toScreen({ x: r.x, y: r.y }, vp);
  return { left: p.x, top: p.y, width: r.w * vp.scale, height: r.h * vp.scale };
}

/** Index of the smallest region containing image point `p` (the innermost when they nest), or -1. */
export function regionAt(p: Point, regions: readonly NoteRegion[]): number {
  let best = -1;
  let area = Infinity;
  regions.forEach((r, i) => {
    if (p.x >= r.x && p.x < r.x + r.w && p.y >= r.y && p.y < r.y + r.h && r.w * r.h < area) {
      best = i;
      area = r.w * r.h;
    }
  });
  return best;
}

/** The viewport that shows region `r` centred in a `boxW`×`boxH` box with some margin (at most `MAX_SCALE`). */
export function focusRegion(r: NoteRegion, boxW: number, boxH: number): Viewport {
  const scale = clamp(Math.min(boxW / (r.w * 2), boxH / (r.h * 2)), MIN_SCALE, MAX_SCALE);
  return { scale, panX: boxW / 2 - (r.x + r.w / 2) * scale, panY: boxH / 2 - (r.y + r.h / 2) * scale };
}
