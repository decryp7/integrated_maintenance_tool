import { Directive, inject, Injectable, input, signal, ElementRef } from '@angular/core';

/** A closed time interval in epoch ms. */
export interface TimeRange {
  from: number;
  to: number;
}

/**
 * Shared time cursor: the time the pointer hovers in any time-based view, and the range the user
 * selected (brush). Views publish into it and may read it to draw a vertical line; analysis
 * commands read the selection ("Analyse this moment").
 */
@Injectable({ providedIn: 'root' })
export class TimeCursor {
  private readonly hoverSignal = signal<number | null>(null);
  private readonly selectionSignal = signal<TimeRange | null>(null);

  /** Epoch ms under the pointer, or null when no view is hovered. */
  readonly hovered = this.hoverSignal.asReadonly();
  /** The range selected by the user, or null. */
  readonly selection = this.selectionSignal.asReadonly();

  hover(ts: number | null): void {
    this.hoverSignal.set(ts === null || !Number.isFinite(ts) ? null : ts);
  }

  /** Normalises (from <= to) and stores a selection; an empty/invalid range clears it. */
  select(range: TimeRange | null): void {
    if (!range || !Number.isFinite(range.from) || !Number.isFinite(range.to)) {
      this.selectionSignal.set(null);
      return;
    }
    this.selectionSignal.set({ from: Math.min(range.from, range.to), to: Math.max(range.from, range.to) });
  }

  clear(): void {
    this.hoverSignal.set(null);
    this.selectionSignal.set(null);
  }
}

/** Maps a pointer x (relative to the element's left edge) to a time in `range`. */
export function xToTime(x: number, width: number, range: TimeRange): number {
  if (width <= 0) return range.from;
  const frac = Math.min(1, Math.max(0, x / width));
  return range.from + frac * (range.to - range.from);
}

/**
 * Adopt in any component that draws time on a horizontal axis:
 * `<div [imtTimeCursor]="{ from, to }">` publishes the hovered time to `TimeCursor`.
 * When the plot box is narrower than the host, `[imtTimeCursorLeft]` gives its left inset in px and
 * either `[imtTimeCursorWidth]` its width or `[imtTimeCursorRight]` its right inset; the pointer
 * outside the plot box publishes nothing (null).
 */
@Directive({
  selector: '[imtTimeCursor]',
  host: {
    '(pointermove)': 'move($event)',
    '(pointerleave)': 'cursor.hover(null)',
  },
})
export class TimeCursorSource {
  readonly cursor = inject(TimeCursor);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  /** The time range the plot box spans. */
  readonly imtTimeCursor = input.required<TimeRange | null>();
  /** Distance of the plot box from the host's left edge, px. */
  readonly imtTimeCursorLeft = input(0);
  /** Distance of the plot box from the host's right edge, px (used when no width is given). */
  readonly imtTimeCursorRight = input(0);
  /** Width of the plot box, px; null = the host's width minus the insets. */
  readonly imtTimeCursorWidth = input<number | null>(null);

  move(e: PointerEvent): void {
    const range = this.imtTimeCursor();
    if (!range) return;
    const box = this.host.nativeElement.getBoundingClientRect();
    const width = this.imtTimeCursorWidth() ?? box.width - this.imtTimeCursorLeft() - this.imtTimeCursorRight();
    const x = e.clientX - box.left - this.imtTimeCursorLeft();
    this.cursor.hover(x < 0 || x > width ? null : xToTime(x, width, range));
  }
}

/** Fraction (0..1) of `range` at which `ts` lies, or null when outside it (or no time is given). */
export function timeToFrac(ts: number | null, range: TimeRange | null): number | null {
  if (ts === null || !range || ts < range.from || ts > range.to) return null;
  const span = range.to - range.from;
  return span > 0 ? (ts - range.from) / span : 0;
}
