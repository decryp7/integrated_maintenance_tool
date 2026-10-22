import { DOCUMENT } from '@angular/common';
import { DestroyRef, inject, Injectable } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';

/** Below this window width (px) the Explorer closes by itself (H1); wider again, it comes back if it was closed this way. */
export const EXPLORER_AUTO_HIDE_PX = 1100;

/** Below this window width (px) docked tool windows auto-hide into side-bar tabs (H1); wider again, they are pinned back. */
export const DOCKS_AUTO_HIDE_PX = 1000;

/**
 * Keeps the window usable when it is narrow: the Explorer side bar closes when the window drops below
 * `EXPLORER_AUTO_HIDE_PX` and comes back when the window is wide again, unless the user closed or opened it in
 * between (their choice wins). The Explorer can still be opened with Ctrl+B at any width. Docked tool windows
 * (side docks) auto-hide below `DOCKS_AUTO_HIDE_PX` the same way and are pinned back where they were.
 */
@Injectable({ providedIn: 'root' })
export class NarrowWindow {
  private readonly store = inject(WorkspaceStore);
  private readonly dock = inject(DockService);
  private readonly doc = inject(DOCUMENT);
  private readonly destroyRef = inject(DestroyRef);
  /** The sidebar was open when the window narrowed and was closed by this service. */
  private closedByNarrowing = false;
  /** Tool windows this service auto-hid when the window narrowed. */
  private hiddenDocks: string[] = [];
  private started = false;

  /** Starts following the window width (idempotent). Without `matchMedia` nothing happens. */
  start(): void {
    if (this.started) return;
    this.started = true;
    this.follow(EXPLORER_AUTO_HIDE_PX, (narrow) => this.apply(narrow));
    this.follow(DOCKS_AUTO_HIDE_PX, (narrow) => this.applyDocks(narrow));
  }

  private follow(px: number, apply: (narrow: boolean) => void): void {
    const query = this.doc.defaultView?.matchMedia?.(`(max-width: ${px - 1}px)`);
    if (!query) return;
    apply(query.matches);
    const listener = (e: MediaQueryListEvent) => apply(e.matches);
    query.addEventListener('change', listener);
    this.destroyRef.onDestroy(() => query.removeEventListener('change', listener));
  }

  /** @internal The window became narrow (true) or wide (false) for the docks; exposed for tests. */
  applyDocks(narrow: boolean): void {
    if (narrow) {
      this.hiddenDocks = [...new Set([...this.hiddenDocks, ...this.dock.autoHideAll()])];
    } else {
      const ids = this.hiddenDocks;
      this.hiddenDocks = [];
      this.dock.pinIds(ids);
    }
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
