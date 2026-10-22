import { ChangeDetectionStrategy, Component, computed, ElementRef, inject, signal } from '@angular/core';
import { formatDateTime, formatDuration } from '../core/format';
import { UiPrefsStore } from '../core/workspace/ui-prefs.store';
import { HIST_VB, histogramPaths } from './histogram';
import { centreOn, dragRange, fractionToTs, stepRange, WindowDrag, windowRect } from './timeline-minimap-model';
import { TimelineMinimapStore } from './timeline-minimap-store';
import { TIMELINE_SOURCE } from './timeline-source';
import { Range, span } from './zoom';

/** A gesture in progress: which part, where it started and the range it started from. */
interface Drag {
  kind: WindowDrag;
  startX: number;
  start: Range;
  pointer: number;
}

/**
 * The timeline minimap: a thin strip over the whole data extent (events by level, the engine's history) with a window
 * for the range on screen. Drag the window to pan, drag an edge to zoom, click the strip to centre the window there,
 * double-click to fit everything; the window is also a slider (arrow keys pan, Home/End go to the ends).
 * The range is applied on release, so dragging stays smooth while the lanes below load once.
 */
@Component({
  selector: 'imt-timeline-minimap',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (rect(); as r) {
      <div
        class="strip"
        data-testid="timeline-minimap"
        role="group"
        aria-label="Timeline overview"
        i18n-aria-label="@@timeline.minimap.label"
        (pointerdown)="stripDown($event)"
        (dblclick)="fit()"
      >
        <svg class="mini" [attr.viewBox]="viewBox()" preserveAspectRatio="none" aria-hidden="true">
          @for (p of hist().paths; track p.level) {
            <path [attr.d]="p.d" [style.fill]="p.color" />
          }
        </svg>
        <div class="dim l" [style.width.%]="r.left * 100" aria-hidden="true"></div>
        <div class="dim r" [style.left.%]="(r.left + r.width) * 100" aria-hidden="true"></div>
        <div
          class="window"
          data-testid="minimap-window"
          role="slider"
          tabindex="0"
          aria-orientation="horizontal"
          aria-label="Visible time range"
          i18n-aria-label="@@timeline.minimap.window"
          [attr.aria-valuemin]="0"
          [attr.aria-valuemax]="100"
          [attr.aria-valuenow]="valueNow()"
          [attr.aria-valuetext]="valueText()"
          [style.left.%]="r.left * 100"
          [style.width.%]="r.width * 100"
          (pointerdown)="windowDown($event, 'move')"
          (keydown)="key($event)"
        >
          <span class="edge l" data-testid="minimap-edge-left" aria-hidden="true" (pointerdown)="windowDown($event, 'left')"></span>
          <span class="edge r" data-testid="minimap-edge-right" aria-hidden="true" (pointerdown)="windowDown($event, 'right')"></span>
        </div>
      </div>
    }
  `,
  styles: `
    :host { display: block; }
    .strip { position: relative; height: var(--minimap-h, 26px); border-bottom: 1px solid var(--border); background: var(--bg-2); cursor: pointer; touch-action: none; user-select: none; }
    .mini { position: absolute; inset: 0; width: 100%; height: 100%; opacity: 0.85; }
    .dim { position: absolute; top: 0; bottom: 0; background: color-mix(in srgb, var(--bg-1) 62%, transparent); pointer-events: none; }
    .dim.l { left: 0; }
    .dim.r { right: 0; }
    .window { position: absolute; top: 0; bottom: 0; border: 1px solid var(--brush-edge); background: var(--brush-fill); cursor: grab; box-sizing: border-box; }
    .window:active { cursor: grabbing; }
    .window:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -1px; }
    .edge { position: absolute; top: 0; bottom: 0; width: 8px; cursor: ew-resize; }
    .edge.l { left: -4px; }
    .edge.r { right: -4px; }
    .edge::after { content: ''; position: absolute; top: 25%; bottom: 25%; left: 3px; width: 2px; border-radius: 1px; background: var(--brush-edge); }
  `,
})
export class TimelineMinimap {
  private readonly src = inject(TIMELINE_SOURCE);
  private readonly store = inject(TimelineMinimapStore);
  private readonly prefs = inject(UiPrefsStore);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;

  /** The range being dragged (shown at once; applied on release). */
  private readonly dragging = signal<Range | null>(null);
  private drag: Drag | null = null;

  protected readonly zoom = computed(() => this.src.zoom());
  protected readonly extent = computed(() => this.zoom().extent());
  protected readonly view = computed(() => this.dragging() ?? this.zoom().view());
  protected readonly hist = computed(() => histogramPaths(this.store.histogram()));
  protected readonly viewBox = computed(() => `0 0 ${Math.max(1, this.store.histogram().length)} ${HIST_VB}`);
  /** Null without a time extent: no timestamps, nothing to overview. */
  protected readonly rect = computed(() => {
    const ext = this.extent();
    const view = this.view();
    return ext && view ? windowRect(view, ext) : null;
  });
  protected readonly valueNow = computed(() => Math.round((this.rect()?.left ?? 0) * 100));
  protected readonly valueText = computed(() => {
    const v = this.view();
    if (!v) return '';
    const tz = this.prefs.timezone();
    const from = formatDateTime(v.from, tz);
    const to = formatDateTime(v.to, tz);
    const duration = formatDuration(span(v));
    return $localize`:@@timeline.minimap.valueText:${from}:from: to ${to}:to: (${duration}:duration:)`;
  });

  constructor() {
    this.store.gate.use();
  }

  protected fit(): void {
    this.zoom().fit();
  }

  /** A press on the strip outside the window centres the window there. */
  protected stripDown(e: PointerEvent): void {
    // Presses on the window (and its edges) stop here; only the strip and its dimmed parts get this far.
    if (e.button !== 0) return;
    // The timeline's own brush (on the plot) must not start from the strip.
    e.stopPropagation();
    const ext = this.extent();
    const view = this.zoom().view();
    const width = this.host.getBoundingClientRect().width;
    if (!ext || !view || width <= 0) return;
    const left = this.host.getBoundingClientRect().left;
    this.zoom().apply(centreOn(fractionToTs((e.clientX - left) / width, ext), view, ext));
  }

  protected windowDown(e: PointerEvent, kind: WindowDrag): void {
    if (e.button !== 0) return;
    const view = this.zoom().view();
    if (!view) return;
    // The edges sit inside the window: their press must not also start a move.
    e.stopPropagation();
    this.drag = { kind, startX: e.clientX, start: view, pointer: e.pointerId };
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture?.(e.pointerId);
    const move = (ev: PointerEvent) => this.dragMove(ev);
    const up = (ev: PointerEvent) => {
      target.removeEventListener('pointermove', move);
      target.removeEventListener('pointerup', up);
      target.removeEventListener('pointercancel', up);
      this.dragEnd(ev);
    };
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', up);
    target.addEventListener('pointercancel', up);
  }

  private dragMove(e: PointerEvent): void {
    const d = this.drag;
    const ext = this.extent();
    const width = this.host.getBoundingClientRect().width;
    if (!d || !ext || width <= 0 || e.pointerId !== d.pointer) return;
    const deltaMs = ((e.clientX - d.startX) / width) * span(ext);
    this.dragging.set(dragRange(d.kind, d.start, deltaMs, ext));
  }

  private dragEnd(e: PointerEvent): void {
    const d = this.drag;
    this.drag = null;
    const range = this.dragging();
    this.dragging.set(null);
    if (!d || e.type === 'pointercancel' || !range) return;
    if (range.from !== d.start.from || range.to !== d.start.to) this.zoom().apply(range);
  }

  protected key(e: KeyboardEvent): void {
    const ext = this.extent();
    const view = this.zoom().view();
    if (!ext || !view) return;
    let next: Range | null = null;
    switch (e.key) {
      case 'ArrowLeft':
        next = stepRange(view, ext, -1, e.shiftKey ? 0.5 : 0.1);
        break;
      case 'ArrowRight':
        next = stepRange(view, ext, 1, e.shiftKey ? 0.5 : 0.1);
        break;
      case 'Home':
        next = { from: ext.from, to: ext.from + span(view) };
        break;
      case 'End':
        next = { from: ext.to - span(view), to: ext.to };
        break;
      default:
        return;
    }
    e.preventDefault();
    this.zoom().apply(next);
  }
}
