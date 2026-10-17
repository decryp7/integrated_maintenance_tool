import { Point, Rect } from '../dock/dock-geometry';

/** A monitor's usable area (without task bar / menu bar), logical px. */
export interface WorkArea extends Rect {
  name: string | null;
  primary: boolean;
}

export const MIN_POPOUT = { width: 320, height: 220 } as const;
const GAP = 8;
/** Title bar and borders the panel content gains as a window. */
const CHROME_HEIGHT = 40;

function area(r: Rect): number {
  return Math.max(0, r.width) * Math.max(0, r.height);
}

export function intersection(a: Rect, b: Rect): Rect {
  const x = Math.max(a.x, b.x);
  const y = Math.max(a.y, b.y);
  return { x, y, width: Math.max(0, Math.min(a.x + a.width, b.x + b.width) - x), height: Math.max(0, Math.min(a.y + a.height, b.y + b.height) - y) };
}

function primaryOf(areas: readonly WorkArea[]): WorkArea {
  return areas.find((a) => a.primary) ?? areas[0];
}

/** Fits `b` inside `a`: no larger than it, moved in from the edges. */
export function fitInto(b: Rect, a: Rect): Rect {
  const width = Math.min(Math.max(b.width, MIN_POPOUT.width), a.width);
  const height = Math.min(Math.max(b.height, MIN_POPOUT.height), a.height);
  const x = Math.min(Math.max(b.x, a.x), a.x + a.width - width);
  const y = Math.min(Math.max(b.y, a.y), a.y + a.height - height);
  return { x: Math.round(x), y: Math.round(y), width: Math.round(width), height: Math.round(height) };
}

/**
 * Where a restored pop-out goes: on its saved monitor when that still exists, else on the monitor it overlaps
 * most; when it is on no visible monitor (or its monitor is gone) it is centred on the primary display. Always
 * fully inside one work area. With no monitor information the bounds are returned unchanged.
 */
export function clampToWorkAreas(b: Rect, areas: readonly WorkArea[], monitor: string | null = null): Rect {
  if (!areas.length) return { ...b };
  const named = monitor ? areas.find((a) => a.name === monitor) : undefined;
  if (monitor && !named) return centreIn(b, primaryOf(areas));
  const target = named ?? areas.reduce<WorkArea | null>((best, a) => (area(intersection(b, a)) > (best ? area(intersection(b, best)) : 0) ? a : best), null);
  return target ? fitInto(b, target) : centreIn(b, primaryOf(areas));
}

export function centreIn(b: Rect, a: Rect): Rect {
  const width = Math.min(b.width, a.width);
  const height = Math.min(b.height, a.height);
  return fitInto({ x: a.x + (a.width - width) / 2, y: a.y + (a.height - height) / 2, width, height }, a);
}

/** The work area containing a point (or the primary one). */
export function areaAt(p: Point, areas: readonly WorkArea[]): WorkArea | null {
  if (!areas.length) return null;
  return areas.find((a) => p.x >= a.x && p.x < a.x + a.width && p.y >= a.y && p.y < a.y + a.height) ?? primaryOf(areas);
}

/** The window size for a panel of this size (content plus window chrome, at least the minimum). */
export function sizeForPanel(panel: { width: number; height: number } | null): { width: number; height: number } {
  const w = panel?.width ?? 560;
  const h = panel?.height ?? 380;
  return { width: Math.round(Math.max(MIN_POPOUT.width, w)), height: Math.round(Math.max(MIN_POPOUT.height, h + CHROME_HEIGHT)) };
}

/**
 * "Move to window next to host": right of the host window when that fits on its monitor, else left of it, else
 * over the host's right edge. Then clamped to the work areas.
 */
export function placeNextToHost(host: Rect | null, size: { width: number; height: number }, areas: readonly WorkArea[]): Rect {
  if (!host) return clampToWorkAreas({ x: 80, y: 80, ...size }, areas);
  const monitor = areaAt({ x: host.x + host.width / 2, y: host.y + host.height / 2 }, areas);
  const y = host.y + 40;
  const right = { x: host.x + host.width + GAP, y, ...size };
  const left = { x: host.x - size.width - GAP, y, ...size };
  const fits = (r: Rect) => !monitor || (r.x >= monitor.x && r.x + r.width <= monitor.x + monitor.width);
  const chosen = fits(right) ? right : fits(left) ? left : { x: host.x + host.width - size.width - 24, y: host.y + 64, ...size };
  return monitor ? fitInto(chosen, monitor) : chosen;
}

/** A pop-out dragged out of the window opens under the pointer (its title bar at the cursor). */
export function placeAtPointer(p: Point, size: { width: number; height: number }, areas: readonly WorkArea[]): Rect {
  const b = { x: p.x - Math.min(120, size.width / 2), y: p.y - 12, ...size };
  const a = areaAt(p, areas);
  return a ? fitInto(b, a) : b;
}
