import { FlameGraph, FlameRect, FrameDto } from '../../core/api.types';

/** Flame graph = depth grows upwards (root at the bottom); icicle = depth grows downwards. */
export type FlameOrientation = 'flame' | 'icicle';

/**
 * The engine's pre-laid-out rectangles plus navigation links: the engine gives each rectangle its
 * parent's index; children and rows are indexes over them (a purely visual transform).
 */
export interface FlameModel {
  rects: FlameRect[];
  frames: Map<number, FrameDto>;
  /** Index of the parent rectangle, -1 at depth 0. */
  parent: number[];
  /** Child indices, largest value first. */
  children: number[][];
  /** Indices per depth, left to right. */
  byDepth: number[][];
  maxDepth: number;
}

export function buildFlameModel(g: FlameGraph): FlameModel {
  const rects = g.rects;
  const byDepth: number[][] = [];
  rects.forEach((r, i) => (byDepth[r.depth] ??= []).push(i));
  for (let d = 0; d < byDepth.length; d++) byDepth[d] = (byDepth[d] ?? []).sort((a, b) => rects[a].x - rects[b].x);
  const valid = (p: number | null | undefined): p is number => typeof p === 'number' && p >= 0 && p < rects.length;
  const parent = rects.map((r) => (valid(r.parent) ? r.parent : -1));
  const children: number[][] = rects.map(() => []);
  parent.forEach((p, i) => p >= 0 && children[p].push(i));
  for (const c of children) c.sort((a, b) => rects[b].value - rects[a].value);
  return { rects, frames: new Map(g.frames.map((f) => [f.id, f])), parent, children, byDepth, maxDepth: Math.max(0, byDepth.length - 1) };
}

/** Indices from depth 0 down to `index`. */
export function chainTo(model: FlameModel, index: number): number[] {
  const chain: number[] = [];
  for (let i = index; i >= 0; i = model.parent[i]) chain.unshift(i);
  return chain;
}

/**
 * The focus path after zooming into a rectangle whose frames from depth 0 are `chain`: with no
 * focus, the chain itself; when already zoomed, depth 0 is the focused node, so the chain
 * continues its path.
 */
export function extendFocus(focus: readonly number[], chain: readonly number[]): number[] {
  return focus.length ? [...focus.slice(0, -1), ...chain] : [...chain];
}

/** Rectangle under a point: x as a fraction of the width, depth from the row. */
export function hitTest(model: FlameModel, xFrac: number, depth: number): number | null {
  for (const i of model.byDepth[depth] ?? []) {
    const r = model.rects[i];
    if (xFrac >= r.x && xFrac < r.x + r.w) return i;
  }
  return null;
}

export type FlameKeyAction = { type: 'move'; index: number } | { type: 'zoom'; index: number } | { type: 'zoomOut' } | null;

/**
 * Keyboard navigation of the frames: the arrow pointing away from the root goes to the largest
 * child, the other to the parent; Left/Right go to the neighbours at the same depth; Enter zooms,
 * Escape zooms out, Home goes to the first top-level frame.
 */
export function flameKey(model: FlameModel, current: number | null, key: string, orientation: FlameOrientation): FlameKeyAction {
  if (key === 'Escape' || key === 'Backspace') return { type: 'zoomOut' };
  if (!model.rects.length) return null;
  const first = model.byDepth[0]?.[0] ?? 0;
  if (current === null || current >= model.rects.length) {
    return ['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Home'].includes(key) ? { type: 'move', index: first } : null;
  }
  const deeper = orientation === 'flame' ? 'ArrowUp' : 'ArrowDown';
  const shallower = orientation === 'flame' ? 'ArrowDown' : 'ArrowUp';
  const row = model.byDepth[model.rects[current].depth];
  const at = row.indexOf(current);
  switch (key) {
    case deeper: {
      const child = model.children[current][0];
      return child === undefined ? null : { type: 'move', index: child };
    }
    case shallower: {
      const p = model.parent[current];
      return p < 0 ? null : { type: 'move', index: p };
    }
    case 'ArrowLeft':
      return at > 0 ? { type: 'move', index: row[at - 1] } : null;
    case 'ArrowRight':
      return at < row.length - 1 ? { type: 'move', index: row[at + 1] } : null;
    case 'Home':
      return { type: 'move', index: first };
    case 'Enter':
    case ' ':
      return { type: 'zoom', index: current };
    default:
      return null;
  }
}

/** The largest rectangles in view, for the accessible list. */
export function topRects(model: FlameModel, limit: number): number[] {
  return model.rects
    .map((_, i) => i)
    .sort((a, b) => model.rects[b].value - model.rects[a].value || model.rects[a].depth - model.rects[b].depth)
    .slice(0, limit);
}

/**
 * Names frame ids from an engine answer: its focus frames (the zoom path) and the frames its
 * rectangles reference. Ids the answer does not name show as "frame #id" (e.g. while the next answer loads).
 */
export function frameNamer(g: Pick<FlameGraph, 'frames' | 'focusFrames'> | null): (id: number) => string {
  const names = new Map<number, string>();
  for (const f of [...(g?.frames ?? []), ...(g?.focusFrames ?? [])]) names.set(f.id, f.name);
  return (id) => names.get(id) ?? `frame #${id}`;
}

/** Zoom breadcrumbs: the focus path as named steps (the first is "All"). */
export function breadcrumbs(focus: readonly number[], name: (id: number) => string): { label: string; path: number[] }[] {
  return [{ label: 'All', path: [] }, ...focus.map((id, i) => ({ label: name(id), path: focus.slice(0, i + 1) }))];
}
