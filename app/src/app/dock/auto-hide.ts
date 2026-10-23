import { DockSlot, Side } from './dock-geometry';

/**
 * Auto-hide (VS "unpin"): a tool window collapses into a tab on a side bar at the nearest window edge and slides
 * out as an overlay on hover or click. Pure state; the dock host renders it and the dock service moves panels.
 */

export interface AutoHideItem {
  id: string;
  title: string;
  side: Side;
  /** Where it was docked, to pin it back there. */
  slot: DockSlot;
  /** Width (left/right) or height (bottom) of the slide-out, px. */
  size: number;
}

export type OpenedBy = 'hover' | 'click' | 'keyboard';

export interface AutoHideState {
  items: readonly AutoHideItem[];
  /** Id of the item slid out, or null. */
  open: string | null;
  openedBy: OpenedBy | null;
}

export type AutoHideEvent =
  | { type: 'unpin'; item: AutoHideItem }
  | { type: 'pin'; id: string }
  | { type: 'hover'; id: string }
  | { type: 'click'; id: string }
  | { type: 'activate'; id: string }
  | { type: 'leave' }
  | { type: 'escape' }
  | { type: 'blur' }
  | { type: 'resize'; id: string; size: number };

export const EMPTY_AUTO_HIDE: AutoHideState = { items: [], open: null, openedBy: null };

export const MIN_FLYOUT = 160;
export const MAX_FLYOUT = 1200;

const closed = (s: AutoHideState): AutoHideState => (s.open === null ? s : { ...s, open: null, openedBy: null });

export function autoHideReduce(state: AutoHideState, event: AutoHideEvent): AutoHideState {
  const has = (id: string) => state.items.some((i) => i.id === id);
  switch (event.type) {
    case 'unpin': {
      const item = { ...event.item, size: clampSize(event.item.size) };
      return { items: [...state.items.filter((i) => i.id !== item.id), item], open: null, openedBy: null };
    }
    case 'pin': {
      if (!has(event.id)) return state;
      const items = state.items.filter((i) => i.id !== event.id);
      return state.open === event.id ? { items, open: null, openedBy: null } : { ...state, items };
    }
    case 'hover':
      // Hover never takes over a slide-out the user opened on purpose.
      if (!has(event.id) || (state.open !== null && state.openedBy !== 'hover')) return state;
      return { ...state, open: event.id, openedBy: 'hover' };
    case 'click':
      if (!has(event.id)) return state;
      // Clicking the tab of a slide-out that is open on purpose hides it; one that only peeked (hover) stays open.
      if (state.open === event.id && state.openedBy !== 'hover') return closed(state);
      return { ...state, open: event.id, openedBy: 'click' };
    case 'activate':
      if (!has(event.id)) return state;
      return { ...state, open: event.id, openedBy: 'keyboard' };
    case 'leave':
      return state.openedBy === 'hover' ? closed(state) : state;
    case 'escape':
    case 'blur':
      return closed(state);
    case 'resize':
      return { ...state, items: state.items.map((i) => (i.id === event.id ? { ...i, size: clampSize(event.size) } : i)) };
  }
}

export function clampSize(size: number): number {
  return Number.isFinite(size) ? Math.min(MAX_FLYOUT, Math.max(MIN_FLYOUT, Math.round(size))) : 320;
}

/** Items of one side bar, in the order they were hidden. */
export function itemsOn(state: AutoHideState, side: Side): AutoHideItem[] {
  return state.items.filter((i) => i.side === side);
}

/** The only edges a tool window can auto-hide to (never the top: that is the menu and document area). */
export const AUTO_HIDE_SIDES: readonly Side[] = ['left', 'right', 'bottom'];

const SIDES: ReadonlySet<string> = new Set(AUTO_HIDE_SIDES);

/** Is this value one of the sides a tool window can auto-hide to? */
export function isAutoHideSide(v: unknown): v is Side {
  return typeof v === 'string' && SIDES.has(v);
}

/** The side to auto-hide to: the one asked for, else the one remembered for the pane, else its nearest edge. */
export function chooseSide(asked: unknown, remembered: unknown, nearest: Side): Side {
  if (isAutoHideSide(asked)) return asked;
  return isAutoHideSide(remembered) ? remembered : nearest;
}

/** Parses the remembered auto-hide side per pane (absent in layouts saved before it existed); drops anything malformed. */
export function sanitizeSides(raw: unknown): Record<string, Side> {
  const out: Record<string, Side> = {};
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return out;
  for (const [id, side] of Object.entries(raw)) if (id && isAutoHideSide(side)) out[id] = side;
  return out;
}
const DIRECTIONS: ReadonlySet<string> = new Set(['left', 'right', 'above', 'below', 'within']);

/** Parses saved auto-hide items (from a layout, project or session); drops anything malformed. */
export function sanitizeAutoHide(raw: unknown): AutoHideItem[] {
  if (!Array.isArray(raw)) return [];
  const out: AutoHideItem[] = [];
  for (const r of raw) {
    if (typeof r !== 'object' || r === null) continue;
    const v = r as Record<string, unknown>;
    const slot = (v['slot'] ?? {}) as Record<string, unknown>;
    if (typeof v['id'] !== 'string' || !v['id'] || typeof v['side'] !== 'string' || !SIDES.has(v['side'])) continue;
    if (out.some((i) => i.id === v['id'])) continue;
    out.push({
      id: v['id'],
      title: typeof v['title'] === 'string' ? v['title'] : v['id'],
      side: v['side'] as Side,
      size: clampSize(typeof v['size'] === 'number' ? v['size'] : 320),
      slot: {
        referencePanel: typeof slot['referencePanel'] === 'string' ? slot['referencePanel'] : null,
        direction: typeof slot['direction'] === 'string' && DIRECTIONS.has(slot['direction']) ? (slot['direction'] as DockSlot['direction']) : 'within',
        ...(typeof slot['index'] === 'number' ? { index: slot['index'] } : {}),
        ...(typeof slot['size'] === 'number' ? { size: slot['size'] } : {}),
      },
    });
  }
  return out;
}
