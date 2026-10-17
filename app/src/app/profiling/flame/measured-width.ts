import { afterNextRender, DestroyRef, ElementRef, inject, Signal, signal } from '@angular/core';

/** Horizontal padding of the flame chart's drawing area (`.scroll { padding: 0 12px }`). */
export const FLAME_INSET_PX = 24;

/** Narrowest layout the views ask the engine for. */
const MIN_WIDTH_PX = 100;

/** Drawing width for an element `clientWidth` wide; null while it has no layout (0 wide). */
export function drawingWidth(clientWidth: number, inset = FLAME_INSET_PX): number | null {
  return clientWidth > 0 ? Math.max(MIN_WIDTH_PX, Math.round(clientWidth - inset)) : null;
}

/**
 * The drawing width of the component's host element, measured after the first render and on every
 * resize; null until the host has a non-zero width. Flame views wait for it before their first
 * request, so the engine lays out once for the real width instead of for a guess and again.
 * Call in an injection context (a component constructor or field initialiser).
 */
export function measuredWidth(inset = FLAME_INSET_PX): Signal<number | null> {
  const host = inject<ElementRef<HTMLElement>>(ElementRef);
  const destroy = inject(DestroyRef);
  const width = signal<number | null>(null);
  const measure = () => {
    const w = drawingWidth(host.nativeElement.clientWidth, inset);
    if (w !== null && w !== width()) width.set(w);
  };
  afterNextRender(() => {
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(host.nativeElement);
    destroy.onDestroy(() => ro.disconnect());
  });
  return width.asReadonly();
}
