import { DestroyRef, inject, Injectable, signal } from '@angular/core';
import { DockContext } from './dock-context';
import { LAYOUT_STORE, SavedLayout } from './layout-store';

const PERSIST_DELAY_MS = 250;

/** Saves the dock arrangement (debounced) through the `LayoutStore` and knows the last saved one. */
@Injectable({ providedIn: 'root' })
export class DockPersistence {
  private readonly ctx = inject(DockContext);
  private readonly store = inject(LAYOUT_STORE);
  private timer: ReturnType<typeof setTimeout> | undefined;
  private held = false;
  private readonly saved = signal<SavedLayout | null>(null);

  constructor() {
    // A save still waiting when the injector goes away (a window closing, a test ending) has nothing to read.
    inject(DestroyRef).onDestroy(() => this.cancel());
  }

  /** The layout as last saved (projects and sessions save it too). */
  readonly savedLayout = this.saved.asReadonly();

  /** The whole arrangement (grid, auto-hidden tool windows, pinned tabs, pop-outs), or null before attach. */
  current(): SavedLayout | null {
    const api = this.ctx.api;
    if (!api) return null;
    return {
      layout: api.toJSON(),
      extras: {
        autoHidden: [...this.ctx.autoHide().items],
        pinnedTabs: [...this.ctx.pinned()],
        poppedOut: this.ctx.windows()?.records() ?? [],
        ...(Object.keys(this.ctx.autoHideSides()).length ? { autoHideSides: { ...this.ctx.autoHideSides() } } : {}),
      },
    };
  }

  /** The saved arrangement to restore at start, if any. */
  load(): SavedLayout | null {
    return this.store.load();
  }

  /** Forgets the saved arrangement (reset, or one that could not be restored). */
  clear(): void {
    this.store.clear();
  }

  /**
   * Stops saving: the arrangement on screen was fitted to a small window (ADR 0056), and the saved one stays on disk
   * untouched so it returns on a larger window. Ends with `release` (an explicit change, Undo, reset).
   */
  hold(): void {
    this.held = true;
    clearTimeout(this.timer);
  }

  release(): void {
    this.held = false;
  }

  /** Persists the layout soon; nothing while a layout is being built or restored, or held. */
  schedule(): void {
    if (this.ctx.restoring || this.held || !this.ctx.api) return;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      const layout = this.current();
      if (!layout) return;
      this.store.save(layout);
      this.saved.set(layout);
    }, PERSIST_DELAY_MS);
  }

  cancel(): void {
    clearTimeout(this.timer);
  }
}
