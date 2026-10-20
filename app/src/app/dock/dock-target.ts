import { compassCells, DropPosition, edgeCells, hitGuide, Point, previewRect, Rect } from './dock-geometry';

/**
 * Where something dropped on the dock goes when it is not a dockview drag: a pop-out window dragged back over its
 * host, or a tab dragged out of the window. Pure: hit-testing and the drag state machines.
 */
export type DockTarget = { kind: 'group'; groupId: string; position: DropPosition } | { kind: 'edge'; position: DropPosition };

export interface GroupBox {
  id: string;
  /** The group's content area, in the same coordinates as the point. */
  rect: Rect;
}

function inside(p: Point, r: Rect): boolean {
  return p.x >= r.x && p.x <= r.x + r.width && p.y >= r.y && p.y <= r.y + r.height;
}

/**
 * The guide under `p`: the compass of the group under it first (as dockview's nested drop targets do), else an
 * edge guide of the whole dock.
 */
export function hitDockTarget(p: Point, root: Rect, groups: readonly GroupBox[]): DockTarget | null {
  if (!inside(p, root)) return null;
  const group = groups.find((g) => inside(p, g.rect));
  const cell = group && hitGuide(compassCells(group.rect.width, group.rect.height), { x: p.x - group.rect.x, y: p.y - group.rect.y });
  if (group && cell) return { kind: 'group', groupId: group.id, position: cell };
  const edge = hitGuide(edgeCells(root.width, root.height), { x: p.x - root.x, y: p.y - root.y });
  return edge ? { kind: 'edge', position: edge } : null;
}

/** The area a drop on `target` would give the panel (for the preview rectangle). */
export function targetPreview(target: DockTarget, root: Rect, groups: readonly GroupBox[]): Rect | null {
  if (target.kind === 'edge') return previewRect(target.position, root, 0.25);
  const g = groups.find((x) => x.id === target.groupId);
  return g ? previewRect(target.position, g.rect) : null;
}

/** A screen point in the coordinates of a window's content, whose top-left is at `origin` on screen. */
export function screenToClient(screen: Point, origin: Point): Point {
  return { x: screen.x - origin.x, y: screen.y - origin.y };
}

/* ------------------------------------------------- a pop-out dragged back over its host */

/**
 * How long the pointer must rest on a guide before the dragged pop-out docks there, on platforms that cannot report
 * when the OS move of a window ends (elsewhere the panel docks only when the window is released).
 */
export const SETTLE_MS = 300;

export interface PopoutDrag {
  label: string;
  viewId: string;
  /** The pointer in the host's client coordinates; null while it is outside the host. */
  point: Point | null;
  target: DockTarget | null;
  /** When the target last changed (ms). */
  since: number;
  /** The release cannot be observed: resting on a guide for `SETTLE_MS` docks instead. */
  settle: boolean;
}

export type PopoutDragEvent =
  | { type: 'move'; label: string; viewId: string; point: Point | null; target: DockTarget | null; now: number; settle: boolean }
  | { type: 'tick'; now: number }
  | { type: 'end'; label: string; now: number }
  | { type: 'cancel' };

export interface DockAction {
  label: string;
  viewId: string;
  target: DockTarget;
}

function sameTarget(a: DockTarget | null, b: DockTarget | null): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * The host's view of a pop-out window being moved over it: guides follow the pointer; the panel docks when the
 * window is released on a guide (`end`), or, only where the release cannot be observed (`settle`), when it rests on
 * one for `SETTLE_MS` (`tick`).
 */
export function popoutDragReduce(state: PopoutDrag | null, e: PopoutDragEvent): { state: PopoutDrag | null; dock: DockAction | null } {
  switch (e.type) {
    case 'move': {
      if (state && state.label !== e.label && state.point) return { state, dock: null };
      const since = state && state.label === e.label && sameTarget(state.target, e.target) ? state.since : e.now;
      return { state: { label: e.label, viewId: e.viewId, point: e.point, target: e.point ? e.target : null, since, settle: e.settle }, dock: null };
    }
    case 'tick':
      if (state?.settle && state.target && e.now - state.since >= SETTLE_MS) return { state: null, dock: { label: state.label, viewId: state.viewId, target: state.target } };
      return { state, dock: null };
    case 'end':
      if (!state || state.label !== e.label) return { state, dock: null };
      return { state: null, dock: state.target ? { label: state.label, viewId: state.viewId, target: state.target } : null };
    case 'cancel':
      return { state: null, dock: null };
  }
}

/* ------------------------------------------------- a tab dragged out of the window (pointer events) */

/** Pointer travel before a press on a tab counts as a drag (dockview's pointer drag starts at a similar distance). */
export const DRAG_THRESHOLD = 5;

export interface TabDrag {
  id: string;
  start: Point;
  last: Point;
  lastScreen: Point;
  dragging: boolean;
}

/** A pressed tab: not a drag yet. */
export function pressTab(id: string, client: Point, screen: Point): TabDrag {
  return { id, start: client, last: client, lastScreen: screen, dragging: false };
}

export function moveTab(d: TabDrag, client: Point, screen: Point): TabDrag {
  const dragging = d.dragging || Math.hypot(client.x - d.start.x, client.y - d.start.y) >= DRAG_THRESHOLD;
  return { ...d, last: client, lastScreen: screen, dragging };
}

/** Is a client point outside the window (`margin` px beyond its edge)? Released there, a dragged tab pops out. */
export function outsideViewport(client: Point, viewport: { width: number; height: number }, margin = 0): boolean {
  return client.x < -margin || client.y < -margin || client.x > viewport.width + margin || client.y > viewport.height + margin;
}
