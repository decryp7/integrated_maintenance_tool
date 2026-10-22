import { DestroyRef, Directive, DOCUMENT, ElementRef, inject, input, signal } from '@angular/core';
import { clamp } from '../core/format';
import { xToTime } from '../analysis/time-cursor';
import { FrameThrottle } from '../shared/throttle';
import { TIMELINE_SOURCE } from './timeline-source';
import { pan as panRange, Range, wheelFactor, xToTs } from './zoom';

/** True when typing in a field (keys belong to it, not to the timeline). */
function isTyping(target: HTMLElement | null): boolean {
  return !!target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable);
}

/**
 * The timeline's pointer and keyboard interaction on its plot area: brush to zoom (and select on the time cursor),
 * Space+drag or middle-drag to pan, Ctrl/⌘+wheel to zoom around the pointer, Shift+wheel to pan, double-click to
 * fit, Alt+←/→ through the zoom history, and the hovered time on the time cursor (once per frame).
 *
 * The keyboard listeners are added to the document directly, not as host listeners, so a key pressed anywhere does
 * not mark the timeline for change detection; only what a key changes (zoom history) re-renders.
 */
@Directive({
  selector: '[debriefTimelineInteraction]',
  exportAs: 'debriefTimelineInteraction',
  host: {
    '[class.panning]': 'panning()',
    '(pointermove)': 'publishHover($event)',
    '(pointerleave)': 'leaveHover()',
    '(pointerdown)': 'pointerDown($event)',
    '(dblclick)': 'zoom.fit()',
    '(wheel)': 'onWheel($event)',
  },
})
export class TimelineInteraction {
  /** The visible time range (null before the first timeline arrives). */
  readonly range = input.required<Range | null>({ alias: 'debriefTimelineInteraction' });
  /** Left edge of the plot inside this element (px). */
  readonly plotLeft = input.required<number>();
  readonly plotWidth = input.required<number>();
  /** The whole panel: history keys act while focus is inside it (for an additional timeline). */
  readonly panel = input.required<HTMLElement>();

  private readonly src = inject(TIMELINE_SOURCE);
  private readonly el = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
  /** The brushed span while dragging (px in the plot). */
  readonly brush = signal<{ x0: number; x1: number } | null>(null);
  readonly panning = signal(false);
  private spaceDown = false;
  /** Pointer moves publish the hovered time at most once per frame. */
  private readonly hoverFrame = new FrameThrottle();

  constructor() {
    const doc = inject(DOCUMENT);
    const onKey = (e: KeyboardEvent) => this.onKey(e);
    doc.addEventListener('keydown', onKey);
    doc.addEventListener('keyup', onKey);
    inject(DestroyRef).onDestroy(() => {
      doc.removeEventListener('keydown', onKey);
      doc.removeEventListener('keyup', onKey);
      this.hoverFrame.cancel();
    });
  }

  protected get zoom() {
    return this.src.zoom();
  }

  private get cursor() {
    return this.src.cursor();
  }

  /** x of a pointer in the plot (px; negative left of it). */
  localX(e: Pick<MouseEvent, 'clientX'>): number {
    return e.clientX - this.el.getBoundingClientRect().left - this.plotLeft();
  }

  /** Publishes the time under the pointer to the cursor in effect (null outside the plot), at most once per frame. */
  protected publishHover(e: PointerEvent): void {
    const clientX = e.clientX;
    this.hoverFrame.run(() => {
      const view = this.range();
      if (!view) return;
      const x = this.localX({ clientX });
      const w = this.plotWidth();
      this.cursor.hover(x < 0 || x > w ? null : xToTime(x, w, view));
    });
  }

  protected leaveHover(): void {
    this.hoverFrame.cancel();
    this.cursor.hover(null);
  }

  protected onWheel(e: WheelEvent): void {
    const view = this.range();
    if (!view) return;
    const x = this.localX(e);
    const w = this.plotWidth();
    if (e.ctrlKey || e.metaKey) {
      // Ctrl/⌘+wheel, and trackpad pinch (reported as ctrl+wheel): zoom around the cursor.
      e.preventDefault();
      this.zoom.wheelZoom(wheelFactor(e.deltaY, e.deltaMode), xToTs(clamp(x, 0, w), w, view));
    } else if (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
      e.preventDefault();
      const dx = e.shiftKey && !e.deltaX ? e.deltaY : e.deltaX;
      this.zoom.panBy((dx / Math.max(1, w)) * (view.to - view.from));
    }
  }

  /** Space (held, for Space+drag) anywhere; Alt+←/→ once: in an additional timeline while focus is inside it, else in the main one. */
  private onKey(e: KeyboardEvent): void {
    const target = e.target as HTMLElement | null;
    if (e.type === 'keyup' && e.code === 'Space') this.spaceDown = false;
    if (e.type !== 'keydown' || isTyping(target)) return;
    if (e.code === 'Space') this.spaceDown = true;
    if (!e.altKey || (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight')) return;
    const panel = this.panel();
    const inInstance = !!target?.closest?.('debrief-timeline-panel.instance');
    if (this.src.main ? inInstance : !(target && panel.contains(target))) return;
    e.preventDefault();
    if (e.key === 'ArrowLeft') this.zoom.back();
    else this.zoom.forward();
  }

  protected pointerDown(e: PointerEvent): void {
    if (e.button === 1 || (e.button === 0 && this.spaceDown)) this.panStart(e);
    else this.brushStart(e);
  }

  private panStart(e: PointerEvent): void {
    const view = this.range();
    if (!view || !this.zoom.canPan()) return;
    e.preventDefault();
    const el = this.el;
    el.setPointerCapture(e.pointerId);
    this.panning.set(true);
    const startX = e.clientX;
    const msPerPx = (view.to - view.from) / Math.max(1, this.plotWidth());
    const extent = this.zoom.extent();
    const up = (ev: PointerEvent) => {
      el.removeEventListener('pointerup', up);
      this.panning.set(false);
      const dx = ev.clientX - startX;
      if (extent && Math.abs(dx) > 2) this.zoom.apply(panRange(view, -dx * msPerPx, extent));
    };
    el.addEventListener('pointerup', up);
  }

  private brushStart(e: PointerEvent): void {
    if (e.button !== 0 || !this.range()) return;
    const x0 = this.localX(e);
    if (x0 < 0 || x0 > this.plotWidth()) return;
    const el = this.el;
    const w = this.plotWidth();
    // The pointer is captured only once it moves, so a plain click still reaches what is under it
    // (an annotation band), while a drag that starts on a band brushes as anywhere else.
    let captured = false;
    const move = (ev: PointerEvent) => {
      if (!captured) {
        if (Math.abs(this.localX(ev) - x0) < 3) return;
        captured = true;
        el.setPointerCapture?.(e.pointerId);
      }
      const x = clamp(this.localX(ev), 0, w);
      this.brush.set({ x0: Math.min(x0, x), x1: Math.max(x0, x) });
    };
    const up = () => {
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
      const b = this.brush();
      this.brush.set(null);
      const view = this.range();
      if (!b || !view || b.x1 - b.x0 < 5) return;
      const selected = { from: xToTs(b.x0, w, view), to: xToTs(b.x1, w, view) };
      this.cursor.select(selected);
      this.zoom.apply(selected);
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
  }
}
