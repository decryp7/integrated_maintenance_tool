import { InjectionToken } from '@angular/core';
import type { SerializedDockview } from 'dockview';
import { AutoHideItem, sanitizeAutoHide, sanitizeSides } from './auto-hide';
import { DockSlot, Rect, Side } from './dock-geometry';

/** Bump when the set of built-in panels or their meaning changes; older saved layouts are ignored. */
export const LAYOUT_VERSION = 1;

/**
 * Version of the stored envelope: the dockview layout plus what dockview does not know about
 * (auto-hidden tool windows, pinned tabs, panels popped out to OS windows). Other versions are ignored.
 */
export const ENVELOPE_VERSION = 2;

/** A panel shown in a pop-out window, with where that window was. */
export interface PoppedOutPanel {
  id: string;
  /** Screen bounds of the window, logical px. */
  bounds: Rect | null;
  /** Name of the monitor it was on, when known. */
  monitor: string | null;
  /** Where the panel docks back to. */
  slot: DockSlot;
}

/** A view the layout plan moved out of its own place for lack of room, and where it returns when there is room again. */
export interface DisplacedView {
  id: string;
  /** Where it was docked before it was moved. */
  slot: DockSlot;
  /** The view whose group it joined as a tab, or null when it was auto-hidden to an edge. */
  into: string | null;
}

/** The sizes the user dragged views to (a wish the plan honours, never below a minimum); absent until the user drags. */
export type UserSizes = Record<string, { width?: number; height?: number }>;

/** What dockview's own JSON does not hold. */
export interface DockExtras {
  autoHidden: AutoHideItem[];
  pinnedTabs: string[];
  poppedOut: PoppedOutPanel[];
  /** Side each tool window was last auto-hidden to, so it goes there again (absent in older layouts). */
  autoHideSides?: Record<string, Side>;
  /** Views the layout plan tabbed or auto-hid for lack of room (absent in older layouts). */
  displaced?: DisplacedView[];
  /** The sizes the user dragged views to (absent in older layouts). */
  sizes?: UserSizes;
}

/** A complete saved arrangement: the dockview grid and the extras. */
export interface SavedLayout {
  layout: SerializedDockview;
  extras: DockExtras;
}

export const EMPTY_EXTRAS: DockExtras = { autoHidden: [], pinnedTabs: [], poppedOut: [] };

/** Persists the dock layout. Implementations must never throw. */
export interface LayoutStore {
  /** The saved layout, or null when none was saved or it is incompatible. */
  load(): SavedLayout | null;
  save(layout: SavedLayout): void;
  clear(): void;
}

interface Envelope {
  version: number;
  layout: SerializedDockview;
  extras?: unknown;
}

export const LAYOUT_KEY = 'debrief.layout';

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

function finite(...n: unknown[]): boolean {
  return n.every((x) => typeof x === 'number' && Number.isFinite(x));
}

function sanitizeSlot(raw: unknown): DockSlot {
  const v = isRecord(raw) ? raw : {};
  const direction = ['left', 'right', 'above', 'below', 'within'].includes(v['direction'] as string) ? (v['direction'] as DockSlot['direction']) : 'within';
  return {
    referencePanel: typeof v['referencePanel'] === 'string' ? v['referencePanel'] : null,
    direction,
    ...(finite(v['index']) ? { index: v['index'] as number } : {}),
    ...(finite(v['size']) ? { size: v['size'] as number } : {}),
  };
}

/** Parses saved extras; anything malformed is dropped (a layout is a convenience, never an error). */
export function sanitizeExtras(raw: unknown): DockExtras {
  if (!isRecord(raw)) return { ...EMPTY_EXTRAS };
  const ids = (a: unknown) => (Array.isArray(a) ? [...new Set(a.filter((x): x is string => typeof x === 'string' && !!x))] : []);
  const popped: PoppedOutPanel[] = [];
  for (const p of Array.isArray(raw['poppedOut']) ? raw['poppedOut'] : []) {
    if (!isRecord(p) || typeof p['id'] !== 'string' || !p['id'] || popped.some((x) => x.id === p['id'])) continue;
    const b = isRecord(p['bounds']) ? p['bounds'] : null;
    popped.push({
      id: p['id'],
      bounds: b && finite(b['x'], b['y'], b['width'], b['height']) ? { x: b['x'] as number, y: b['y'] as number, width: b['width'] as number, height: b['height'] as number } : null,
      monitor: typeof p['monitor'] === 'string' ? p['monitor'] : null,
      slot: sanitizeSlot(p['slot']),
    });
  }
  const sides = sanitizeSides(raw['autoHideSides']);
  const displaced: DisplacedView[] = [];
  for (const d of Array.isArray(raw['displaced']) ? raw['displaced'] : []) {
    if (!isRecord(d) || typeof d['id'] !== 'string' || !d['id'] || displaced.some((x) => x.id === d['id'])) continue;
    displaced.push({ id: d['id'], slot: sanitizeSlot(d['slot']), into: typeof d['into'] === 'string' && d['into'] ? d['into'] : null });
  }
  const sizes: UserSizes = {};
  if (isRecord(raw['sizes'])) {
    for (const [id, v] of Object.entries(raw['sizes'])) {
      if (!isRecord(v)) continue;
      const w = finite(v['width']) && (v['width'] as number) > 0 ? { width: v['width'] as number } : {};
      const h = finite(v['height']) && (v['height'] as number) > 0 ? { height: v['height'] as number } : {};
      if (w.width || h.height) sizes[id] = { ...w, ...h };
    }
  }
  return {
    autoHidden: sanitizeAutoHide(raw['autoHidden']),
    pinnedTabs: ids(raw['pinnedTabs']),
    poppedOut: popped,
    ...(Object.keys(sides).length ? { autoHideSides: sides } : {}),
    ...(displaced.length ? { displaced } : {}),
    ...(Object.keys(sizes).length ? { sizes } : {}),
  };
}

/** Reads a stored envelope; null for unknown versions and anything malformed. */
export function readLayout(raw: unknown): SavedLayout | null {
  if (!isRecord(raw) || !isRecord(raw['layout'])) return null;
  const layout = raw['layout'] as unknown as SerializedDockview;
  if (raw['version'] === ENVELOPE_VERSION) return { layout, extras: sanitizeExtras(raw['extras']) };
  return null;
}

export function toEnvelope(saved: SavedLayout): Envelope {
  return { version: ENVELOPE_VERSION, layout: saved.layout, extras: saved.extras };
}

/** Stores the layout as versioned JSON in `localStorage`. */
export class LocalStorageLayoutStore implements LayoutStore {
  constructor(private readonly key = LAYOUT_KEY) {}

  load(): SavedLayout | null {
    try {
      const raw = localStorage.getItem(this.key);
      return raw ? readLayout(JSON.parse(raw)) : null;
    } catch {
      return null;
    }
  }

  save(layout: SavedLayout): void {
    try {
      localStorage.setItem(this.key, JSON.stringify(toEnvelope(layout)));
    } catch {
      /* storage unavailable or full: the layout is a convenience only */
    }
  }

  clear(): void {
    try {
      localStorage.removeItem(this.key);
    } catch {
      /* ignore */
    }
  }
}

export const LAYOUT_STORE = new InjectionToken<LayoutStore>('LAYOUT_STORE', {
  providedIn: 'root',
  factory: () => new LocalStorageLayoutStore(),
});
