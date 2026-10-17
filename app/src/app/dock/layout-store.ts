import { InjectionToken } from '@angular/core';
import type { SerializedDockview } from 'dockview';

/** Bump when the set of built-in panels or their meaning changes; older saved layouts are ignored. */
export const LAYOUT_VERSION = 1;

/** Persists the dock layout. Implementations must never throw. */
export interface LayoutStore {
  /** The saved layout, or null when none was saved or it is incompatible. */
  load(): SerializedDockview | null;
  save(layout: SerializedDockview): void;
  clear(): void;
}

interface Envelope {
  version: number;
  layout: SerializedDockview;
}

export const LAYOUT_KEY = 'imt.layout';

/** Stores the layout as versioned JSON in `localStorage`. */
export class LocalStorageLayoutStore implements LayoutStore {
  constructor(
    private readonly key = LAYOUT_KEY,
    private readonly version = LAYOUT_VERSION,
  ) {}

  load(): SerializedDockview | null {
    try {
      const raw = localStorage.getItem(this.key);
      if (!raw) return null;
      const parsed = JSON.parse(raw) as Partial<Envelope> | null;
      if (!parsed || parsed.version !== this.version || !parsed.layout || typeof parsed.layout !== 'object') return null;
      return parsed.layout;
    } catch {
      return null;
    }
  }

  save(layout: SerializedDockview): void {
    try {
      localStorage.setItem(this.key, JSON.stringify({ version: this.version, layout } satisfies Envelope));
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
