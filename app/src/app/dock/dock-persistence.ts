import { inject, Injectable, signal } from '@angular/core';
import { DockContext } from './dock-context';
import { LAYOUT_STORE, SavedLayout } from './layout-store';

const PERSIST_DELAY_MS = 250;

/** Saves the dock arrangement (debounced) through the `LayoutStore` and knows the last saved one. */
@Injectable({ providedIn: 'root' })
export class DockPersistence {
  private readonly ctx = inject(DockContext);
  private readonly store = inject(LAYOUT_STORE);
  private timer: ReturnType<typeof setTimeout> | undefined;
  private readonly saved = signal<SavedLayout | null>(null);

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

  /** Persists the layout soon; nothing while a layout is being built or restored. */
  schedule(): void {
    if (this.ctx.restoring || !this.ctx.api) return;
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
