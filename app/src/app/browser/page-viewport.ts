import { DestroyRef, effect, ElementRef, EnvironmentInjector, inject, Signal, signal, untracked } from '@angular/core';
import { DOCUMENT } from '@angular/common';
import { panelVisible } from '../core/panel-visibility';
import { DockService } from '../dock/dock.service';
import { OverlayTracker } from '../ui/overlay-tracker';
import { CssRect, Placement } from './browser.types';

/** What decides whether the page shows in the panel. */
export interface PlacementInput {
  /** The panel's tab is the active one of its group. */
  panelVisible: boolean;
  /** A page is open. */
  pageOpen: boolean;
  /** The window is minimised or in the background (`document.hidden`). */
  documentHidden: boolean;
  /** An IMT overlay covers part of the page area. */
  covered: boolean;
  /** The page area in CSS pixels (null: not in the document). */
  rect: CssRect | null;
  scale: number;
}

/** Rounds to 1/100 px: sub-pixel jitter of a layout is not a move. */
const round = (v: number) => Math.round(v * 100) / 100;

/** The placement to report: the page area while it is laid out, and whether the page may show there now. */
export function placementFor(i: PlacementInput): Placement {
  const r = i.rect;
  const laidOut = !!r && r.width > 0 && r.height > 0;
  const rect = laidOut ? { x: round(r.x), y: round(r.y), width: round(r.width), height: round(r.height) } : null;
  return { visible: laidOut && i.panelVisible && i.pageOpen && !i.documentHidden && !i.covered, rect, scale: i.scale };
}

export function samePlacement(a: Placement | null, b: Placement | null): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** A placement that keeps the page hidden (the panel is gone). */
export function hiddenPlacement(scale: number): Placement {
  return { visible: false, rect: null, scale };
}

export interface PageViewport {
  /** An IMT overlay covers the page area now (the page steps aside; the panel says why). */
  readonly covered: Signal<boolean>;
  /** The page shows in the panel now (as last reported). */
  readonly shown: Signal<boolean>;
  /** Measures again at the next frame. */
  schedule(): void;
}

/**
 * Keeps the native page webview over the Browser panel's page area (ADR 0024): measures the area whenever it may have
 * moved or changed (its size, the dock's layout, the window's size and scale, the panel's visibility, IMT overlays)
 * and reports the placement through `place` (at most once per frame, only when it changed). Hides the page when the
 * panel goes. Call in a component's injection context.
 */
export function pageViewport(opts: { area: Signal<ElementRef<HTMLElement> | undefined>; open: Signal<boolean>; place: (p: Placement) => Promise<unknown> }): PageViewport {
  const doc = inject(DOCUMENT);
  const win = doc.defaultView;
  const visible = panelVisible();
  const overlays = inject(OverlayTracker);
  const dock = inject(DockService, { optional: true });
  const destroyRef = inject(DestroyRef);
  const covered = signal(false);
  const shown = signal(false);
  let last: Placement | null = null;
  let frame = 0;
  let destroyed = false;
  let observed: HTMLElement | null = null;
  overlays.watch(destroyRef);

  const send = (p: Placement): void => {
    if (samePlacement(p, last)) return;
    last = p;
    shown.set(p.visible);
    void opts.place(p).catch(() => undefined);
  };

  const measure = (): void => {
    frame = 0;
    if (destroyed) return;
    const el = opts.area()?.nativeElement ?? null;
    observe(el);
    const box = el?.isConnected ? el.getBoundingClientRect() : null;
    const rect = box ? { x: box.left, y: box.top, width: box.width, height: box.height } : null;
    const isCovered = !!box && box.width > 0 && overlays.covers(box);
    covered.set(isCovered);
    send(
      placementFor({
        panelVisible: visible(),
        pageOpen: opts.open(),
        documentHidden: doc.hidden,
        covered: isCovered,
        rect,
        scale: win?.devicePixelRatio || 1,
      }),
    );
  };

  const schedule = (): void => {
    if (destroyed || frame || !win) return;
    frame = win.requestAnimationFrame(measure);
  };

  const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(schedule);
  const observe = (el: HTMLElement | null): void => {
    if (el === observed) return;
    if (observed) resize?.unobserve(observed);
    observed = el;
    if (el) resize?.observe(el);
  };

  // The device pixel ratio changes with the monitor's scale and the UI zoom: listen for the current one each time.
  let dpr: MediaQueryList | null = null;
  const watchScale = (): void => {
    dpr?.removeEventListener('change', onScale);
    dpr = win?.matchMedia?.(`(resolution: ${win.devicePixelRatio || 1}dppx)`) ?? null;
    dpr?.addEventListener('change', onScale);
  };
  const onScale = (): void => {
    watchScale();
    schedule();
  };
  watchScale();
  win?.addEventListener('resize', schedule);
  doc.addEventListener('visibilitychange', schedule);

  // A root effect: it runs while the panel's own view is detached from change detection (a hidden tab), which is
  // exactly when the page must hide.
  const ref = effect(
    () => {
      visible();
      opts.open();
      opts.area();
      overlays.changes();
      overlays.held();
      dock?.layoutChanged();
      untracked(schedule);
    },
    { injector: inject(EnvironmentInjector) },
  );

  destroyRef.onDestroy(() => {
    destroyed = true;
    ref.destroy();
    if (frame && win) win.cancelAnimationFrame(frame);
    resize?.disconnect();
    dpr?.removeEventListener('change', onScale);
    win?.removeEventListener('resize', schedule);
    doc.removeEventListener('visibilitychange', schedule);
    // The panel is gone (closed, or its window): the page hides until a panel places it again.
    const p = hiddenPlacement(win?.devicePixelRatio || 1);
    if (!samePlacement(p, last)) void opts.place(p).catch(() => undefined);
  });

  return { covered: covered.asReadonly(), shown: shown.asReadonly(), schedule };
}
