/**
 * Pure geometry of the dock: where a panel was (so it can go back there after a pop-out or auto-hide), which
 * window edge is nearest, and the drop guides shown while dragging (VS-style compass and edge guides).
 */

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Point {
  x: number;
  y: number;
}

export type Direction = 'left' | 'right' | 'above' | 'below';
export type DropPosition = 'center' | 'left' | 'right' | 'top' | 'bottom';

/** Where a panel sat in the layout: next to (or tabbed with) a reference panel. */
export interface DockSlot {
  /** A panel that is still in the layout to place it against; null = anywhere (default placement). */
  referencePanel: string | null;
  direction: Direction | 'within';
  /** Index in the reference panel's tab strip, for `within`. */
  index?: number;
  /** Size of its group, px (width for left/right, height for above/below). */
  size?: number;
  /** The group was a full-span column (left/right) or strip (bottom) on a window edge: put it back there, not beside one neighbour. */
  edge?: Side;
}

function overlap(a0: number, a1: number, b0: number, b1: number): number {
  return Math.max(0, Math.min(a1, b1) - Math.max(a0, b0));
}

/**
 * The neighbour a group was docked against and on which side of it the group sat: the other group sharing the
 * longest edge with it (within `tolerance` px). Null when there is no adjacent group.
 */
export function adjacentNeighbour<T>(rect: Rect, others: readonly { rect: Rect; value: T }[], tolerance = 6): { value: T; direction: Direction } | null {
  let best: { value: T; direction: Direction; shared: number } | null = null;
  const consider = (value: T, direction: Direction, shared: number) => {
    if (shared > 0 && (!best || shared > best.shared)) best = { value, direction, shared };
  };
  for (const { rect: o, value } of others) {
    const vertical = overlap(rect.y, rect.y + rect.height, o.y, o.y + o.height);
    const horizontal = overlap(rect.x, rect.x + rect.width, o.x, o.x + o.width);
    // `direction` is where the group sits relative to the neighbour.
    if (Math.abs(rect.x - (o.x + o.width)) <= tolerance) consider(value, 'right', vertical);
    if (Math.abs(rect.x + rect.width - o.x) <= tolerance) consider(value, 'left', vertical);
    if (Math.abs(rect.y - (o.y + o.height)) <= tolerance) consider(value, 'below', horizontal);
    if (Math.abs(rect.y + rect.height - o.y) <= tolerance) consider(value, 'above', horizontal);
  }
  const found = best as { value: T; direction: Direction; shared: number } | null;
  return found ? { value: found.value, direction: found.direction } : null;
}

export type Side = 'left' | 'right' | 'bottom';

/**
 * The window edge a group is a full-span column or strip on: touching the left or right edge and as tall as the window,
 * or touching the bottom edge and as wide as it. Null for a group beside only part of the layout (it has neighbours
 * to be put back against instead; `adjacentNeighbour` would pick the one sharing the longest edge, which for a
 * full-height column is just the top row or the bottom one).
 */
export function fullSpanEdge(rect: Rect, root: Rect, tolerance = 2): Side | null {
  const fullHeight = rect.height >= root.height - tolerance;
  const fullWidth = rect.width >= root.width - tolerance;
  if (fullHeight && !fullWidth) {
    if (root.x + root.width - (rect.x + rect.width) <= tolerance) return 'right';
    if (rect.x - root.x <= tolerance) return 'left';
  }
  if (fullWidth && !fullHeight && root.y + root.height - (rect.y + rect.height) <= tolerance) return 'bottom';
  return null;
}

/** The window edge (left, right or bottom) a panel is nearest to: where its auto-hide tab goes. */
export function nearestSide(rect: Rect, root: Rect): Side {
  const left = rect.x - root.x;
  const right = root.x + root.width - (rect.x + rect.width);
  const bottom = root.y + root.height - (rect.y + rect.height);
  // Ties prefer the sides (VS puts most tool windows there).
  if (left <= right && left <= bottom) return 'left';
  if (right <= bottom) return 'right';
  return 'bottom';
}

/**
 * Is a group docked along this window edge: touching it, and not stretching to the opposite one? A full-height
 * column on the right is at the right edge; a full-width strip at the bottom is at the bottom edge (where
 * `nearestSide` would call it "left", being 0 px from every side it spans).
 */
export function atEdge(rect: Rect, root: Rect, side: 'right' | 'bottom', tolerance = 2): boolean {
  if (side === 'right') return root.x + root.width - (rect.x + rect.width) <= tolerance && rect.x - root.x > tolerance;
  return root.y + root.height - (rect.y + rect.height) <= tolerance && rect.y - root.y > tolerance;
}

/* ---------------------------------------------------------------- drop guides */

/** Size of one compass cell and the gap between cells, px. */
export const GUIDE = { cell: 32, gap: 4, edgeInset: 10 } as const;

export interface GuideCell {
  position: DropPosition;
  rect: Rect;
}

/**
 * The five cells of the compass ("diamond") centred in a group's content area of `width` x `height`: tab into
 * (centre) and dock left, right, top or bottom of the group. Fewer cells fit in very small groups: none below
 * 3 cells' width/height (then only the edge guides and tabs accept drops).
 */
export function compassCells(width: number, height: number): GuideCell[] {
  const { cell, gap } = GUIDE;
  const span = cell * 3 + gap * 2;
  if (width < span || height < span) return width >= cell && height >= cell ? [{ position: 'center', rect: { x: (width - cell) / 2, y: (height - cell) / 2, width: cell, height: cell } }] : [];
  const cx = (width - cell) / 2;
  const cy = (height - cell) / 2;
  const step = cell + gap;
  const at = (position: DropPosition, x: number, y: number): GuideCell => ({ position, rect: { x, y, width: cell, height: cell } });
  return [at('center', cx, cy), at('left', cx - step, cy), at('right', cx + step, cy), at('top', cx, cy - step), at('bottom', cx, cy + step)];
}

/** The outer guides of the whole dock area: one cell at the middle of each edge. */
export function edgeCells(width: number, height: number): GuideCell[] {
  const { cell, edgeInset } = GUIDE;
  if (width < cell * 3 || height < cell * 3) return [];
  const midX = (width - cell) / 2;
  const midY = (height - cell) / 2;
  const at = (position: DropPosition, x: number, y: number): GuideCell => ({ position, rect: { x, y, width: cell, height: cell } });
  return [at('left', edgeInset, midY), at('right', width - edgeInset - cell, midY), at('top', midX, edgeInset), at('bottom', midX, height - edgeInset - cell)];
}

function inside(p: Point, r: Rect): boolean {
  return p.x >= r.x && p.x <= r.x + r.width && p.y >= r.y && p.y <= r.y + r.height;
}

/** The guide cell under a point (coordinates relative to the same box as the cells), or null. */
export function hitGuide(cells: readonly GuideCell[], p: Point): DropPosition | null {
  return cells.find((c) => inside(p, c.rect))?.position ?? null;
}

/**
 * Resolves a drop the VS way: over a group's content its compass cells dock (the centre tabs in); the edge guides
 * of the whole dock dock against the whole edge. `layout` is true for dockview's root target (args relative to the
 * whole dock); for a group's target, `inRoot` is the pointer relative to the whole dock (its size `root`), so an
 * edge guide over a group still docks to the layout edge (`edge: true`, which dockview routes to the root).
 * `zones` are the positions the target currently accepts. Returns null for "no drop here".
 */
export function resolveGuideDrop(
  args: { x: number; y: number; width: number; height: number; zones: ReadonlySet<string> },
  layout: boolean,
  inRoot: Point | null = null,
  root: { width: number; height: number } | null = null,
): { position: DropPosition; edge?: boolean } | null {
  if (layout) {
    const hit = hitGuide(edgeCells(args.width, args.height), { x: args.x, y: args.y });
    return hit && args.zones.has(hit) ? { position: hit } : null;
  }
  const cell = hitGuide(compassCells(args.width, args.height), { x: args.x, y: args.y });
  if (cell) return args.zones.has(cell) ? { position: cell } : null;
  const edge = inRoot && root ? hitGuide(edgeCells(root.width, root.height), inRoot) : null;
  return edge && edge !== 'center' ? { position: edge, edge: true } : null;
}

/** Is a drop target of this size the whole layout (dockview's root target) rather than a group's content? */
export function isLayoutTarget(target: { width: number; height: number }, root: { width: number; height: number } | null, tolerance = 2): boolean {
  return !!root && Math.abs(target.width - root.width) <= tolerance && Math.abs(target.height - root.height) <= tolerance;
}

/** The rectangle a drop at `position` would give the dragged panel within a target box (the preview). */
export function previewRect(position: DropPosition, box: Rect, fraction = 0.5): Rect {
  switch (position) {
    case 'center':
      return { ...box };
    case 'left':
      return { x: box.x, y: box.y, width: box.width * fraction, height: box.height };
    case 'right':
      return { x: box.x + box.width * (1 - fraction), y: box.y, width: box.width * fraction, height: box.height };
    case 'top':
      return { x: box.x, y: box.y, width: box.width, height: box.height * fraction };
    case 'bottom':
      return { x: box.x, y: box.y + box.height * (1 - fraction), width: box.width, height: box.height * fraction };
  }
}
