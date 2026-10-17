import { computed, inject, Injectable, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';

/** A closed time interval in epoch ms. */
export interface Range {
  from: number;
  to: number;
}

export const MIN_SPAN_MS = 1;

export function span(r: Range): number {
  return r.to - r.from;
}

/** Keeps a range inside the extent, preserving its span when possible. */
export function clampRange(r: Range, extent: Range, minSpan = MIN_SPAN_MS): Range {
  const extentSpan = Math.max(minSpan, span(extent));
  const s = Math.min(Math.max(span(r), minSpan), extentSpan);
  let from = r.from;
  if (from < extent.from) from = extent.from;
  if (from + s > extent.from + extentSpan) from = extent.from + extentSpan - s;
  return { from, to: from + s };
}

/** True when the range covers the whole extent (i.e. "fit all"). */
export function coversExtent(r: Range, extent: Range): boolean {
  return r.from <= extent.from && r.to >= extent.to;
}

/**
 * Zooms by `factor` (<1 zooms in, >1 zooms out) keeping `anchor` at the same screen position.
 */
export function zoomAt(r: Range, anchor: number, factor: number, extent: Range, minSpan = MIN_SPAN_MS): Range {
  const s = span(r);
  const next = Math.max(minSpan, s * factor);
  const frac = s > 0 ? (anchor - r.from) / s : 0.5;
  const from = anchor - frac * next;
  return clampRange({ from, to: from + next }, extent, minSpan);
}

/** Shifts the range by deltaMs without leaving the extent. */
export function pan(r: Range, deltaMs: number, extent: Range): Range {
  return clampRange({ from: r.from + deltaMs, to: r.to + deltaMs }, extent);
}

/** Converts a pixel offset within a plot of `width` px to a timestamp. */
export function xToTs(x: number, width: number, r: Range): number {
  return r.from + (width > 0 ? x / width : 0) * span(r);
}

/** Wheel delta (px) to a zoom factor; positive delta zooms out. */
export function wheelFactor(deltaY: number, deltaMode = 0): number {
  const px = deltaMode === 1 ? deltaY * 16 : deltaMode === 2 ? deltaY * 400 : deltaY;
  return Math.exp(px * 0.002);
}

/** Back/forward stack of visited ranges; null means "all". */
export class RangeHistory {
  private stack: (Range | null)[] = [null];
  private index = 0;

  get current(): Range | null {
    return this.stack[this.index];
  }
  get canBack(): boolean {
    return this.index > 0;
  }
  get canForward(): boolean {
    return this.index < this.stack.length - 1;
  }

  push(r: Range | null): void {
    const cur = this.current;
    if ((cur === null && r === null) || (cur && r && cur.from === r.from && cur.to === r.to)) return;
    this.stack = [...this.stack.slice(0, this.index + 1), r].slice(-100);
    this.index = this.stack.length - 1;
  }

  back(): Range | null {
    if (this.canBack) this.index--;
    return this.current;
  }

  forward(): Range | null {
    if (this.canForward) this.index++;
    return this.current;
  }
}

const WHEEL_COMMIT_MS = 150;
const STEP = 0.5;

/** All time-range changes go through here so the timeline, grid and charts stay in sync with history. */
@Injectable({ providedIn: 'root' })
export class TimelineZoom {
  private readonly store = inject(WorkspaceStore);
  private readonly history = new RangeHistory();
  private readonly version = signal(0);
  private pending: Range | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;

  readonly canBack = computed(() => (this.version(), this.history.canBack));
  readonly canForward = computed(() => (this.version(), this.history.canForward));

  /** Full data extent, or null before anything is loaded. */
  readonly extent = computed<Range | null>(() => {
    const s = this.store.summary();
    return s?.from != null && s.to != null ? { from: s.from, to: Math.max(s.to, s.from + MIN_SPAN_MS) } : null;
  });

  /** The range on screen: the query range, or the full extent. */
  readonly view = computed<Range | null>(() => {
    const ext = this.extent();
    const q = this.store.query();
    if (!ext) return null;
    return { from: q.from ?? ext.from, to: q.to ?? ext.to };
  });

  readonly isZoomed = computed(() => this.store.hasRange());

  /** Sets a range (null = fit all) and records it in history. */
  apply(r: Range | null): void {
    const ext = this.extent();
    let next = r;
    if (next && ext) {
      next = clampRange(next, ext);
      if (coversExtent(next, ext)) next = null;
    }
    this.cancelPending();
    this.history.push(next);
    this.version.update((v) => v + 1);
    this.store.setRange(next ? Math.floor(next.from) : null, next ? Math.ceil(next.to) : null);
  }

  fit(): void {
    this.apply(null);
  }

  zoomIn(anchor?: number): void {
    this.zoomBy(STEP, anchor);
  }

  zoomOut(anchor?: number): void {
    this.zoomBy(1 / STEP, anchor);
  }

  zoomBy(factor: number, anchor?: number): void {
    const view = this.pending ?? this.view();
    const ext = this.extent();
    if (!view || !ext) return;
    this.apply(zoomAt(view, anchor ?? (view.from + view.to) / 2, factor, ext));
  }

  /** Wheel/pinch zoom: compounds quickly, commits after a short pause. */
  wheelZoom(factor: number, anchor: number): void {
    const view = this.pending ?? this.view();
    const ext = this.extent();
    if (!view || !ext) return;
    this.schedule(zoomAt(view, anchor, factor, ext));
  }

  /** Pans by deltaMs; debounced like wheel zoom when `debounced`. */
  panBy(deltaMs: number, debounced = true): void {
    const view = this.pending ?? this.view();
    const ext = this.extent();
    if (!view || !ext || !this.store.hasRange()) return;
    const next = pan(view, deltaMs, ext);
    if (debounced) this.schedule(next);
    else this.apply(next);
  }

  back(): void {
    this.cancelPending();
    const r = this.history.back();
    this.version.update((v) => v + 1);
    this.store.setRange(r?.from ?? null, r?.to ?? null);
  }

  forward(): void {
    this.cancelPending();
    const r = this.history.forward();
    this.version.update((v) => v + 1);
    this.store.setRange(r?.from ?? null, r?.to ?? null);
  }

  /** The range a pending wheel gesture will commit (for immediate visual feedback). */
  pendingRange(): Range | null {
    return this.pending;
  }

  private schedule(r: Range): void {
    this.pending = r;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      const p = this.pending;
      this.pending = null;
      this.apply(p);
    }, WHEEL_COMMIT_MS);
  }

  private cancelPending(): void {
    clearTimeout(this.timer);
    this.pending = null;
  }
}
