import { TileCfg } from '../../core/api.types';

/**
 * Pure layout math of a dashboard grid: tiles are rectangles in column/row units. Every edit returns a
 * new tile list with the edited tile clamped to the grid and the tiles it would overlap pushed down.
 */

/** Tallest tile, in rows (the engine's limit). */
export const MAX_TILE_H = 48;

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The tile with its size and position kept inside a grid of `columns` (whole numbers, at least 1×1). */
export function clampTile<T extends Rect>(t: T, columns: number): T {
  const cols = Math.max(1, Math.floor(columns));
  const w = Math.min(cols, Math.max(1, Math.round(t.w)));
  const h = Math.min(MAX_TILE_H, Math.max(1, Math.round(t.h)));
  const x = Math.min(cols - w, Math.max(0, Math.round(t.x)));
  const y = Math.max(0, Math.round(t.y));
  return { ...t, x, y, w, h };
}

export function overlaps(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

/** First row below every tile. */
export function bottomOf(tiles: readonly Rect[]): number {
  return tiles.reduce((m, t) => Math.max(m, t.y + t.h), 0);
}

/**
 * Keeps tile `fixedId` where it is and moves every other tile it (or a moved tile) overlaps down, top to
 * bottom, until nothing overlaps. Order of the list is preserved.
 */
export function resolveCollisions(tiles: readonly TileCfg[], fixedId: string): TileCfg[] {
  const fixed = tiles.find((t) => t.id === fixedId);
  if (!fixed) return [...tiles];
  const placed: TileCfg[] = [fixed];
  const others = tiles.filter((t) => t.id !== fixedId).sort((a, b) => a.y - b.y || a.x - b.x);
  const moved = new Map<string, TileCfg>([[fixed.id, fixed]]);
  for (const t of others) {
    let cur = t;
    for (let hit = placed.find((p) => overlaps(p, cur)); hit; hit = placed.find((p) => overlaps(p, cur))) {
      cur = { ...cur, y: hit.y + hit.h };
    }
    placed.push(cur);
    moved.set(cur.id, cur);
  }
  return tiles.map((t) => moved.get(t.id) ?? t);
}

function edit(tiles: readonly TileCfg[], id: string, columns: number, change: (t: TileCfg) => TileCfg): TileCfg[] {
  if (!tiles.some((t) => t.id === id)) return [...tiles];
  const next = tiles.map((t) => (t.id === id ? clampTile(change(t), columns) : t));
  return resolveCollisions(next, id);
}

/** Moves a tile by whole cells. */
export function moveTile(tiles: readonly TileCfg[], id: string, dx: number, dy: number, columns: number): TileCfg[] {
  return edit(tiles, id, columns, (t) => ({ ...t, x: t.x + dx, y: t.y + dy }));
}

/** Puts a tile at a cell (pointer drag). */
export function placeTile(tiles: readonly TileCfg[], id: string, x: number, y: number, columns: number): TileCfg[] {
  return edit(tiles, id, columns, (t) => ({ ...t, x, y }));
}

/** Grows or shrinks a tile by whole cells (its top-left corner stays). */
export function resizeTile(tiles: readonly TileCfg[], id: string, dw: number, dh: number, columns: number): TileCfg[] {
  return edit(tiles, id, columns, (t) => {
    const w = Math.min(Math.max(1, t.w + dw), columns - t.x);
    return { ...t, w, h: t.h + dh };
  });
}

/** Sets a tile's size (pointer resize). */
export function sizeTile(tiles: readonly TileCfg[], id: string, w: number, h: number, columns: number): TileCfg[] {
  return edit(tiles, id, columns, (t) => ({ ...t, w: Math.min(Math.max(1, w), columns - t.x), h }));
}

export function removeTile(tiles: readonly TileCfg[], id: string): TileCfg[] {
  return tiles.filter((t) => t.id !== id);
}

/** Adds a tile below the others, clamped to the grid. */
export function addTile(tiles: readonly TileCfg[], tile: TileCfg, columns: number): TileCfg[] {
  return [...tiles, clampTile({ ...tile, x: 0, y: bottomOf(tiles) }, columns)];
}

/** An id not used by any tile: `base`, `base-2`, `base-3`... (letters, digits and dashes only). */
export function uniqueId(used: readonly string[], base: string): string {
  const clean = base.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'tile';
  if (!used.includes(clean)) return clean;
  for (let i = 2; ; i++) if (!used.includes(`${clean}-${i}`)) return `${clean}-${i}`;
}
