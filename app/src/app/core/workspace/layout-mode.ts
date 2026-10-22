import { computed, Injectable, signal } from '@angular/core';

/**
 * How much the window shows by itself:
 * - `focused` (default): after opening data, the timeline over the events; Home, Query and Metrics open on demand;
 * - `full`: the richer layout, where the query bar is part of the default layout and Home opens after imports.
 */
export type LayoutMode = 'focused' | 'full';

export const LAYOUT_MODE_KEY = 'imt.layout.mode.v1';

function readMode(): LayoutMode {
  try {
    return localStorage.getItem(LAYOUT_MODE_KEY) === 'full' ? 'full' : 'focused';
  } catch {
    return 'focused';
  }
}

/** View › Layout: Focused or Full (a per-viewer preference; saved layouts are never changed by it). */
@Injectable({ providedIn: 'root' })
export class LayoutModeStore {
  private readonly current = signal<LayoutMode>(readMode());
  readonly mode = this.current.asReadonly();
  readonly full = computed(() => this.current() === 'full');

  set(mode: LayoutMode): void {
    this.current.set(mode);
    try {
      localStorage.setItem(LAYOUT_MODE_KEY, mode);
    } catch {
      /* storage unavailable: the choice lasts for this session only */
    }
  }
}
