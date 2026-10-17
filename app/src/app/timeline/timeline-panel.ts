import {
  afterNextRender,
  ChangeDetectionStrategy,
  Component,
  computed,
  DestroyRef,
  ElementRef,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import { Lane, LEVELS, MarkerCluster } from '../core/api.types';
import { clamp, formatCount, formatDateTime, formatDuration, formatTime, timeTicks } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { HIST_VARS } from '../shared/colors';
import { Icon } from '../shared/icon';
import { MarkerIcon } from '../shared/marker-icon';
import { LaidMarker, layoutLane, MarkerVm } from './marker-layout';
import { pan as panRange, Range, TimelineZoom, wheelFactor, xToTs } from './zoom';

const LABEL_W = 250;
/** Kinds drawn as a thin tick strip when a lane is too busy for full shapes. */
const COMPACT_KINDS: ReadonlySet<string> = new Set(['warning']);
const HIST_H = 30;

interface LaneVm {
  lane: Lane;
  left: number;
  width: number;
  densityPath: string;
  markers: LaidMarker[];
  ticksPath: string;
  tickCount: number;
  filtered: boolean;
  meta: string;
}

/**
 * Orders lanes by their position in the full (unzoomed) view. When `reset` is true the
 * given lanes define the order; otherwise known lanes keep their slot and new ones follow.
 */
export function stableLaneOrder(lanes: Lane[], order: Map<string, number>, reset: boolean): Lane[] {
  if (reset) {
    order.clear();
    lanes.forEach((l, i) => order.set(l.key, i));
    return lanes;
  }
  const rank = (l: Lane) => order.get(l.key) ?? Number.MAX_SAFE_INTEGER;
  return [...lanes].sort((a, b) => rank(a) - rank(b) || a.first - b.first);
}

interface Hover {
  x: number;
  y: number;
  lane: string;
  marker: LaidMarker;
}

@Component({
  selector: 'imt-timeline-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, MarkerIcon],
  host: { '(document:keydown)': 'onKey($event)', '(document:keyup)': 'onKey($event)' },
  template: `
    <div class="head">
      <span class="section-title">Timeline</span>
      <span class="faint hint">
        @if (tl(); as t) {
          {{ t.lanes.length }} instance{{ t.lanes.length === 1 ? '' : 's' }} · bucket {{ bucketLabel() }} · drag to zoom · ⌘/Ctrl+wheel zoom · Shift+wheel or Space+drag pan
        }
      </span>
      @if (store.timelineLoading()) { <span class="spinner" aria-label="Loading"></span> }
      <span class="spacer"></span>
      <div class="legend">
        @for (k of store.summary()?.markerKinds ?? []; track k.kind) {
          <span class="lg"><imt-marker-icon [kind]="k.kind" [color]="k.color" [label]="k.label" />{{ k.label }}</span>
        }
      </div>
      <div class="zoom" role="group" aria-label="Zoom">
        <button class="icon-btn" title="Previous range (Alt+←)" aria-label="Previous range" [disabled]="!zoom.canBack()" (click)="zoom.back()">←</button>
        <button class="icon-btn" title="Next range (Alt+→)" aria-label="Next range" [disabled]="!zoom.canForward()" (click)="zoom.forward()">→</button>
        <button class="icon-btn" title="Zoom in" aria-label="Zoom in" (click)="zoom.zoomIn()"><imt-icon name="plus" [size]="13" /></button>
        <button class="icon-btn" title="Zoom out" aria-label="Zoom out" (click)="zoom.zoomOut()"><imt-icon name="minus" [size]="13" /></button>
        <button class="btn" title="Fit all (double-click)" [disabled]="!zoom.isZoomed()" (click)="zoom.fit()"><imt-icon name="zoomOut" [size]="13" /> Fit</button>
      </div>
    </div>

    <div class="wrap" #wrap [class.panning]="panning()" (pointerdown)="pointerDown($event)" (pointermove)="pointerMove($event)" (pointerleave)="hoverX.set(null)" (dblclick)="zoom.fit()" (wheel)="onWheel($event)">
      <div class="scroller">
        <div class="sticky">
          <div class="row hist-row">
            <div class="label faint small">events / bucket</div>
            <div class="plot">
              <svg class="hist" [attr.viewBox]="histViewBox()" preserveAspectRatio="none" aria-hidden="true">
                @for (p of histPaths(); track p.level) {
                  <path [attr.d]="p.d" [style.fill]="p.color" />
                }
              </svg>
            </div>
          </div>
          <div class="row axis-row">
            <div class="label"></div>
            <div class="plot axis" #axis>
              @for (t of ticks(); track t.ts) {
                <span class="tick mono" [style.left.%]="t.frac * 100">{{ t.label }}</span>
              }
            </div>
          </div>
        </div>

        @for (l of lanes(); track l.lane.key) {
          <div class="row lane">
            <button class="label lane-label" [class.filtered]="l.filtered" (click)="store.toggleGroup(l.lane.key)" [title]="l.lane.key + ' — click to ' + (l.filtered ? 'remove the instance filter' : 'show only this instance')">
              <span class="mono key ellipsis">{{ l.lane.key }}</span>
              <span class="faint meta ellipsis">{{ l.meta }}</span>
            </button>
            <div class="plot track">
              <div class="baseline"></div>
              <div class="bar" [style.left.%]="l.left" [style.width.%]="l.width"></div>
              <svg class="density" [attr.viewBox]="densityViewBox()" preserveAspectRatio="none" aria-hidden="true"><path [attr.d]="l.densityPath" /></svg>
              @if (l.tickCount) {
                <svg class="ticks" viewBox="0 0 1000 6" preserveAspectRatio="none" role="img" [attr.aria-label]="l.tickCount + ' warnings (zoom in for detail)'">
                  <title>{{ l.tickCount }} warnings — zoom in for detail</title>
                  <path [attr.d]="l.ticksPath" />
                </svg>
              }
              @for (m of l.markers; track m.cluster.kind + m.cluster.bucket) {
                <button
                  class="marker"
                  [style.left.%]="m.left"
                  [attr.aria-label]="m.label + ' at ' + time(m.cluster.ts) + (m.cluster.count > 1 ? ', ' + m.cluster.count + ' events' : '')"
                  (pointerdown)="$event.stopPropagation()"
                  (click)="store.jumpTo(m.cluster.id)"
                  (pointerenter)="hover.set({ x: $event.clientX, y: $event.clientY, lane: l.lane.key, marker: m })"
                  (pointerleave)="hover.set(null)"
                >
                  <imt-marker-icon [kind]="m.cluster.kind" [color]="m.color" />
                  @if (m.showCount) { <span class="count mono">{{ m.cluster.count }}</span> }
                </button>
              }
            </div>
          </div>
        } @empty {
          <div class="empty-state">
            @if (!store.summary()?.sources?.length) {
              <imt-icon name="files" [size]="28" />
              <div>Open log files or a folder to build the timeline.</div>
              <button class="btn primary" (click)="store.pickAndOpen(true)">Open folder…</button>
            } @else if (tl()) {
              <div>No timestamped events match the current filters.</div>
            }
          </div>
        }
        @if ((tl()?.truncatedLanes ?? 0) > 0) {
          <div class="truncated faint">{{ tl()!.truncatedLanes }} more instances not shown — narrow the filter or change the group-by.</div>
        }
      </div>

      <div class="overlay" [style.left.px]="plotLeft()" [style.width.px]="plotWidth()">
        @for (x of fatalLines(); track $index) {
          <div class="fatal-line" [style.left.%]="x"></div>
        }
        @if (brush(); as b) {
          <div class="brush" [style.left.px]="b.x0" [style.width.px]="b.x1 - b.x0"></div>
        }
        @if (hoverX(); as hx) {
          <div class="cursor" [style.left.px]="hx">
            <span class="cursor-label mono">{{ hoverTime() }}</span>
          </div>
        }
      </div>
    </div>

    @if (hover(); as h) {
      <div class="popover tip" [style.left.px]="h.x + 14" [style.top.px]="h.y + 12" role="tooltip">
        <div class="tip-head">
          <imt-marker-icon [kind]="h.marker.cluster.kind" [color]="h.marker.color" />
          <strong>{{ h.marker.label }}</strong>
          <span class="faint">{{ h.marker.cluster.count === 1 ? '1 event' : h.marker.cluster.count + ' events in this interval' }}</span>
          <span class="spacer"></span>
          <span class="mono faint">{{ time(h.marker.cluster.ts) }}</span>
        </div>
        <div class="mono small">{{ h.lane }}</div>
        <div class="tip-msg">{{ h.marker.cluster.message }}</div>
        <div class="faint small">Click to jump to {{ h.marker.cluster.count > 1 ? 'the first event' : 'the event' }}</div>
      </div>
    }
  `,
  styles: `
    :host { flex: 1; min-height: 0; min-width: 0; display: flex; flex-direction: column; background: var(--bg-1); overflow: hidden; }
    .head { height: 32px; flex: none; display: flex; align-items: center; gap: 12px; padding: 0 14px; }
    .hint { font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .spacer { flex: 1; }
    .legend { display: flex; align-items: center; gap: 12px; font-size: 12px; color: var(--muted); flex-wrap: nowrap; overflow: hidden; }
    .lg { display: inline-flex; align-items: center; gap: 5px; white-space: nowrap; }
    .spinner { width: 10px; height: 10px; border-radius: 50%; border: 2px solid var(--border-3); border-top-color: var(--accent); animation: spin 0.8s linear infinite; flex: none; }
    @keyframes spin { to { transform: rotate(360deg); } }
    .wrap { position: relative; flex: 1; min-height: 0; user-select: none; }
    .wrap.panning { cursor: grabbing; }
    .zoom { display: flex; align-items: center; gap: 2px; }
    .zoom .icon-btn:disabled { opacity: 0.35; cursor: default; }
    .scroller { position: absolute; inset: 0; overflow-y: auto; overflow-x: hidden; scrollbar-gutter: stable; padding-bottom: 8px; }
    .sticky { position: sticky; top: 0; z-index: 2; background: var(--bg-1); }
    .row { display: flex; align-items: stretch; }
    .label { width: ${LABEL_W}px; flex: none; }
    .plot { flex: 1; min-width: 0; position: relative; margin-right: 14px; }
    .small { font-size: 11px; }
    .hist-row .label { display: flex; align-items: flex-end; padding: 0 14px 2px; }
    .hist { display: block; width: 100%; height: ${HIST_H}px; }
    .axis { height: 20px; border-bottom: 1px solid var(--border); }
    .tick { position: absolute; top: 3px; transform: translateX(-50%); font-size: 10px; color: var(--faint); white-space: nowrap; }
    .lane { height: 32px; }
    .lane-label { display: flex; flex-direction: column; justify-content: center; align-items: flex-start; gap: 0; padding: 0 10px 0 12px; border: 0; border-left: 2px solid transparent; background: transparent; cursor: pointer; text-align: left; min-width: 0; }
    .lane-label:hover { background: var(--bg-hover); }
    .lane-label.filtered { border-left-color: var(--accent); background: var(--bg-active); }
    .key { font-size: 12px; color: var(--text); max-width: 100%; }
    .meta { font-size: 11px; max-width: 100%; }
    .track { height: 32px; }
    .baseline { position: absolute; left: 0; right: 0; top: 16px; height: 1px; background: var(--grid-line); }
    .bar { position: absolute; top: 7px; height: 18px; min-width: 2px; background: var(--lane-bar); border: 1px solid var(--lane-border); border-radius: 4px; }
    .density { position: absolute; left: 0; top: 8px; width: 100%; height: 16px; pointer-events: none; }
    .density path { fill: var(--density); }
    .ticks { position: absolute; left: 0; bottom: 1px; width: 100%; height: 6px; }
    .ticks path { stroke: var(--mk-warning); stroke-width: 1.5; vector-effect: non-scaling-stroke; }
    .marker { position: absolute; top: 0; height: 32px; width: 16px; margin-left: -8px; display: flex; align-items: center; justify-content: center; border: 0; padding: 0; background: transparent; cursor: pointer; z-index: 1; }
    .marker:hover imt-marker-icon { transform: scale(1.35); }
    .count { position: absolute; left: 13px; top: 2px; font-size: 9px; padding: 0 2px; border-radius: 2px; color: var(--text-2); background: var(--bg-1); pointer-events: none; }
    .overlay { position: absolute; top: 0; bottom: 0; pointer-events: none; }
    .brush { position: absolute; top: 0; bottom: 0; background: var(--accent-soft); border-left: 1px solid var(--accent); border-right: 1px solid var(--accent); }
    .fatal-line { position: absolute; top: ${HIST_H}px; bottom: 0; width: 0; border-left: 1px dashed var(--mk-fatal); opacity: 0.55; }
    .cursor { position: absolute; top: 0; bottom: 0; width: 0; border-left: 1px solid var(--border-3); }
    .cursor-label { position: absolute; top: ${HIST_H + 2}px; left: 4px; font-size: 10px; padding: 1px 4px; border-radius: 3px; background: var(--tooltip-bg); border: 1px solid var(--border-3); color: var(--text-2); white-space: nowrap; }
    .truncated { padding: 6px 14px; font-size: 12px; }
    .tip { position: fixed; width: 340px; display: flex; flex-direction: column; gap: 4px; pointer-events: none; line-height: 1.45; }
    .tip-head { display: flex; align-items: center; gap: 8px; }
    .tip-msg { color: var(--text-2); }
  `,
})
export class TimelinePanel {
  protected readonly store = inject(WorkspaceStore);
  protected readonly zoom = inject(TimelineZoom);
  protected readonly panning = signal(false);
  /** Lane order from the unzoomed view, so zooming never reshuffles lanes. */
  private readonly laneOrder = new Map<string, number>();
  private spaceDown = false;
  private readonly wrap = viewChild.required<ElementRef<HTMLElement>>('wrap');
  private readonly axis = viewChild.required<ElementRef<HTMLElement>>('axis');

  protected readonly plotLeft = signal(LABEL_W);
  protected readonly plotWidth = signal(800);
  protected readonly brush = signal<{ x0: number; x1: number } | null>(null);
  protected readonly hover = signal<Hover | null>(null);
  protected readonly hoverX = signal<number | null>(null);

  protected readonly tl = this.store.timeline;

  protected readonly span = computed(() => {
    const t = this.tl();
    return t ? Math.max(1, t.to - t.from) : 1;
  });

  protected readonly ticks = computed(() => {
    const t = this.tl();
    if (!t) return [];
    return timeTicks(t.from, t.to, Math.max(2, Math.floor(this.plotWidth() / 110)), this.store.timezone());
  });

  protected readonly bucketLabel = computed(() => formatDuration(this.tl()?.bucketMs ?? 0));
  protected readonly histViewBox = computed(() => `0 0 ${Math.max(1, this.tl()?.histogram.length ?? 1)} ${HIST_H}`);
  protected readonly densityViewBox = computed(() => `0 0 ${Math.max(1, this.tl()?.histogram.length ?? 1)} 16`);

  /** One path per level, stacked bottom-up, scaled to the busiest bucket. */
  protected readonly histPaths = computed(() => {
    const t = this.tl();
    if (!t) return [];
    let max = 1;
    for (const b of t.histogram) max = Math.max(max, b.reduce((a, c) => a + c, 0));
    const paths = LEVELS.map(() => [] as string[]);
    t.histogram.forEach((b, x) => {
      let y = HIST_H;
      b.forEach((count, li) => {
        if (!count) return;
        const h = (count / max) * (HIST_H - 2);
        y -= h;
        paths[li].push(`M${x + 0.1} ${y.toFixed(2)}h0.8v${h.toFixed(2)}h-0.8z`);
      });
    });
    return LEVELS.map((level, i) => ({ level, d: paths[i].join(''), color: HIST_VARS[level] })).filter((p) => p.d);
  });

  protected readonly lanes = computed<LaneVm[]>(() => {
    const t = this.tl();
    if (!t) return [];
    const span = this.span();
    const kinds = this.store.markerKindMap();
    const groups = new Set(this.store.query().groups);
    const tz = this.store.timezone();
    const priority = [...kinds.keys()];
    const ordered = stableLaneOrder(t.lanes, this.laneOrder, !this.store.hasRange());
    return ordered.map((lane) => {
      const max = Math.max(1, ...lane.density);
      let d = '';
      lane.density.forEach((v, x) => {
        if (!v) return;
        const h = 2 + (v / max) * 14;
        d += `M${x + 0.1} ${(16 - h).toFixed(2)}h0.8v${h.toFixed(2)}h-0.8z`;
      });
      const layout = layoutLane(
        lane.markers.map((cluster): MarkerVm => {
          const k = kinds.get(cluster.kind);
          return {
            cluster,
            left: clamp(((cluster.ts - t.from) / span) * 100, 0, 100),
            color: k?.color ?? null,
            label: k?.label ?? cluster.kind,
          };
        }),
        { plotWidth: this.plotWidth(), priority, compactKinds: COMPACT_KINDS },
      );
      return {
        lane,
        left: clamp(((lane.first - t.from) / span) * 100, 0, 100),
        width: Math.max(0.2, ((lane.last - lane.first) / span) * 100),
        densityPath: d,
        markers: layout.shapes,
        ticksPath: layout.ticks.map((x) => `M${(x * 10).toFixed(1)} 0v6`).join(''),
        tickCount: layout.tickCount,
        filtered: groups.has(lane.key),
        meta: `${formatCount(lane.matched)}${lane.matched !== lane.total ? ' of ' + formatCount(lane.total) : ''} · ${formatTime(lane.first, tz).slice(0, 8)} → ${formatTime(lane.last, tz).slice(0, 8)}`,
      };
    });
  });

  protected readonly fatalLines = computed(() =>
    this.lanes().flatMap((l) => l.markers.filter((m) => m.cluster.kind === 'fatal').map((m) => m.left)),
  );

  protected readonly hoverTime = computed(() => {
    const x = this.hoverX();
    const t = this.tl();
    if (x === null || !t) return '';
    return formatDateTime(t.from + (x / this.plotWidth()) * this.span(), this.store.timezone(), true);
  });

  constructor() {
    const destroyRef = inject(DestroyRef);
    afterNextRender(() => {
      const ro = new ResizeObserver(() => this.measure());
      ro.observe(this.axis().nativeElement);
      ro.observe(this.wrap().nativeElement);
      destroyRef.onDestroy(() => ro.disconnect());
      this.measure();
    });
  }

  private measure(): void {
    const wrap = this.wrap().nativeElement.getBoundingClientRect();
    const axis = this.axis().nativeElement.getBoundingClientRect();
    if (axis.width <= 0) return;
    this.plotLeft.set(axis.left - wrap.left);
    this.plotWidth.set(axis.width);
    this.store.setBuckets(Math.floor(axis.width / 2));
  }

  protected time(ts: number): string {
    return formatDateTime(ts, this.store.timezone(), true);
  }

  private localX(e: MouseEvent): number {
    const wrap = this.wrap().nativeElement.getBoundingClientRect();
    return e.clientX - wrap.left - this.plotLeft();
  }

  protected pointerMove(e: PointerEvent): void {
    const x = this.localX(e);
    this.hoverX.set(x >= 0 && x <= this.plotWidth() && this.tl() ? x : null);
  }

  private viewRange(): Range | null {
    const t = this.tl();
    return t ? { from: t.from, to: t.to } : null;
  }

  protected onWheel(e: WheelEvent): void {
    const view = this.viewRange();
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

  protected onKey(e: KeyboardEvent): void {
    const target = e.target as HTMLElement | null;
    const typing = !!target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable);
    if (e.type === 'keyup' && e.code === 'Space') this.spaceDown = false;
    if (e.type !== 'keydown' || typing) return;
    if (e.code === 'Space') this.spaceDown = true;
    if (e.altKey && e.key === 'ArrowLeft') {
      e.preventDefault();
      this.zoom.back();
    } else if (e.altKey && e.key === 'ArrowRight') {
      e.preventDefault();
      this.zoom.forward();
    }
  }

  protected pointerDown(e: PointerEvent): void {
    if (e.button === 1 || (e.button === 0 && this.spaceDown)) this.panStart(e);
    else this.brushStart(e);
  }

  private panStart(e: PointerEvent): void {
    const view = this.viewRange();
    if (!view || !this.store.hasRange()) return;
    e.preventDefault();
    const el = this.wrap().nativeElement;
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

  protected brushStart(e: PointerEvent): void {
    if (e.button !== 0 || !this.tl()) return;
    const x0 = this.localX(e);
    if (x0 < 0 || x0 > this.plotWidth()) return;
    const el = this.wrap().nativeElement;
    el.setPointerCapture(e.pointerId);
    const w = this.plotWidth();
    const move = (ev: PointerEvent) => {
      const x = clamp(this.localX(ev), 0, w);
      this.brush.set({ x0: Math.min(x0, x), x1: Math.max(x0, x) });
    };
    const up = () => {
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
      const b = this.brush();
      this.brush.set(null);
      const t = this.tl();
      if (!b || !t || b.x1 - b.x0 < 5) return;
      const view = { from: t.from, to: t.to };
      this.zoom.apply({ from: xToTs(b.x0, w, view), to: xToTs(b.x1, w, view) });
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
  }
}
