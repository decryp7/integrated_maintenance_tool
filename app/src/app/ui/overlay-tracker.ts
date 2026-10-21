import { DestroyRef, inject, Injectable, signal } from '@angular/core';
import { DOCUMENT } from '@angular/common';

/**
 * What counts as an IMT overlay: CDK overlay panes (menus, tooltips, selects of Angular Material) and anything IMT
 * marks with `data-imt-overlay` (dialogs, the command palette, context menus, toasts, the tour, dock drag guides).
 */
export const OVERLAY_SELECTOR = '.cdk-overlay-pane, [data-imt-overlay]';

export interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** Whether two boxes share an area (touching edges do not count). */
export function intersects(a: Box, b: Box): boolean {
  return a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
}

/** Whether any of `overlays` with an area covers part of `area`. */
export function covered(area: Box, overlays: readonly Box[]): boolean {
  return overlays.some((o) => o.right > o.left && o.bottom > o.top && intersects(area, o));
}

/**
 * Knows when IMT draws something floating (a menu, a dialog, a toast, the drag guides) and where, so the internal
 * browser's page, a native webview drawn above all of IMT's HTML, can step aside while it is open (ADR 0024).
 * Generic on purpose: overlays are found by `OVERLAY_SELECTOR`, not by asking each component. `changes` ticks (at
 * most once per frame) when overlays may have come or gone; `hold()` covers what the DOM cannot show (a drag in
 * progress).
 */
@Injectable({ providedIn: 'root' })
export class OverlayTracker {
  private readonly doc = inject(DOCUMENT);
  private readonly tick = signal(0);
  private readonly holds = signal(0);
  private observer: MutationObserver | null = null;
  private watchers = 0;
  private frame = 0;

  /** Ticks when overlays may have changed (read it in an effect or computed to re-check). */
  readonly changes = this.tick.asReadonly();
  /** Something holds the page hidden, wherever it is. */
  readonly held = () => this.holds() > 0;

  /** Starts observing for the caller's lifetime (the observer runs only while someone watches). */
  watch(destroyRef: Pick<DestroyRef, 'onDestroy'> = inject(DestroyRef)): void {
    this.watchers++;
    if (this.watchers === 1) this.start();
    destroyRef.onDestroy(() => {
      this.watchers--;
      if (this.watchers === 0) this.stop();
    });
  }

  /** Hides the page wherever it is until the returned release is called (once). */
  hold(): () => void {
    this.holds.update((n) => n + 1);
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.holds.update((n) => Math.max(0, n - 1));
    };
  }

  /** The boxes of the overlays shown now. */
  boxes(): Box[] {
    const out: Box[] = [];
    for (const el of Array.from(this.doc.querySelectorAll<HTMLElement>(OVERLAY_SELECTOR))) {
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) out.push({ left: r.left, top: r.top, right: r.right, bottom: r.bottom });
    }
    return out;
  }

  /** Whether an overlay (or a hold) covers part of `area` now. */
  covers(area: Box): boolean {
    return this.held() || covered(area, this.boxes());
  }

  private start(): void {
    const body = this.doc.body;
    const Observer = this.doc.defaultView?.MutationObserver;
    if (!body || !Observer) return;
    this.observer = new Observer(() => this.schedule());
    this.observer.observe(body, { childList: true, subtree: true, attributes: true, attributeFilter: ['data-imt-overlay', 'hidden'] });
  }

  private stop(): void {
    this.observer?.disconnect();
    this.observer = null;
    const win = this.doc.defaultView;
    if (this.frame && win) win.cancelAnimationFrame(this.frame);
    this.frame = 0;
  }

  /** One tick per frame, however many mutations came in it. */
  private schedule(): void {
    const win = this.doc.defaultView;
    if (this.frame || !win) return;
    this.frame = win.requestAnimationFrame(() => {
      this.frame = 0;
      this.tick.update((n) => n + 1);
    });
  }
}
