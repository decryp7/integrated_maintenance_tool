import { DOCUMENT } from '@angular/common';
import { DestroyRef, inject, Injectable } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';

/** Below this window width (px) the Explorer closes by itself (H1); wider again, it comes back if it was closed this way. */
export const EXPLORER_AUTO_HIDE_PX = 1100;

/**
 * Keeps the window usable when it is narrow: the Explorer side bar closes when the window drops below
 * `EXPLORER_AUTO_HIDE_PX` and comes back when the window is wide again, unless the user closed or opened it in
 * between (their choice wins). The Explorer can still be opened with Ctrl+B at any width.
 */
@Injectable({ providedIn: 'root' })
export class NarrowWindow {
  private readonly store = inject(WorkspaceStore);
  private readonly doc = inject(DOCUMENT);
  private readonly destroyRef = inject(DestroyRef);
  /** The sidebar was open when the window narrowed and was closed by this service. */
  private closedByNarrowing = false;
  private started = false;

  /** Starts following the window width (idempotent). Without `matchMedia` nothing happens. */
  start(): void {
    if (this.started) return;
    this.started = true;
    const query = this.doc.defaultView?.matchMedia?.(`(max-width: ${EXPLORER_AUTO_HIDE_PX - 1}px)`);
    if (!query) return;
    const onChange = (narrow: boolean) => this.apply(narrow);
    this.apply(query.matches);
    const listener = (e: MediaQueryListEvent) => onChange(e.matches);
    query.addEventListener('change', listener);
    this.destroyRef.onDestroy(() => query.removeEventListener('change', listener));
  }

  /** @internal The window became narrow (true) or wide (false); exposed for tests. */
  apply(narrow: boolean): void {
    if (narrow) {
      if (this.store.sidebarOpen()) {
        this.closedByNarrowing = true;
        this.store.toggleSidebar();
      }
    } else if (this.closedByNarrowing) {
      this.closedByNarrowing = false;
      if (!this.store.sidebarOpen()) this.store.toggleSidebar();
    }
  }
}
