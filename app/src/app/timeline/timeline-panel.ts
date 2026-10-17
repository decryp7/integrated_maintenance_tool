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
import { Lane, LEVELS, MarkerKind } from '../core/api.types';
import { clamp, formatCount, formatDateTime, formatDuration, formatTime, timeTicks } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { HIST_VARS } from '../shared/colors';
import { Icon } from '../shared/icon';
import { MarkerGlyph } from '../markers/marker-glyph';
import { styleOf } from '../markers/marker-styles';
import { MarkerStyles } from '../markers/marker-styles';
import { LaidMarker, layoutLane, MarkerVm } from './marker-layout';
import { pan as panRange, Range, TimelineZoom, wheelFactor, xToTs } from './zoom';
import { AnalysisTargets } from '../analysis/analysis-targets';
import { TimeCursor, TimeCursorSource, timeToFrac } from '../analysis/time-cursor';
import { momentAt, WIDTHS } from '../analysis/windows';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { ContextMenuService } from '../ui/context-menu.service';
import { LiveWindowControls } from '../live/live-window-controls';
import { SeverityBadge } from '../live/severity-badge';
import { layoutAnnotations } from './annotation-layout';
import { AnnotationHover, AnnotationTrack } from './annotation-track';
import { TimelineAnnotations } from './annotations.store';
import { ProfileSpanTrack } from '../profiling/integration/profile-span-track';

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
  imports: [Icon, MarkerGlyph, TimeCursorSource, LiveWindowControls, AnnotationTrack, SeverityBadge, ProfileSpanTrack],
  host: { '(document:keydown)': 'onKey($event)', '(document:keyup)': 'onKey($event)' },
  template: `
    <div class="head">
      <span class="section-title">Timeline</span>
      <span class="faint hint">
        @if (tl(); as t) {
          {{ t.lanes.length }} instance{{ t.lanes.length === 1 ? '' : 's' }} · bucket {{ bucketLabel() }} · drag to zoom · ⌘/Ctrl+wheel zoom · Shift+wheel or Space+drag pan
        }
      </span>
      @if (store.timelineLoading()) { <span class="spinner" role="status" aria-label="Loading"></span> }
      <span class="spacer"></span>
      <imt-live-window-controls />
      <div class="legend">
        @for (k of store.summary()?.markerKinds ?? []; track k.kind) {
          <button
            type="button"
            class="lg"
            [attr.aria-pressed]="legendActive(k.kind)"
            [attr.aria-describedby]="legendDesc(k)"
            [title]="legendTitle(k)"
            (click)="toggleKind(k.kind)"
          ><imt-marker-glyph [kind]="k.kind" [size]="16" />{{ k.label }}</button>
          @if (legendDesc(k); as id) { <span class="sr" [id]="id">{{ legendTitle(k) }}</span> }
        }
      </div>
      @if (ann.kinds().length || ann.error()) {
        <div class="ann-legend" role="group" aria-label="Annotations">
          <span class="lg-title">Annotations</span>
          @for (k of ann.kinds(); track k.kind) {
            <button
              type="button"
              class="lg"
              [attr.aria-pressed]="ann.isShown(k.kind)"
              [attr.data-kind]="k.kind"
              [title]="(ann.isShown(k.kind) ? 'Hide ' : 'Show ') + k.label + ' (' + k.owner + ', ' + k.count + ')'"
              (click)="ann.toggle(k.kind)"
            ><imt-severity-badge [severity]="k.severity" [showLabel]="false" [size]="12" />{{ k.label }}</button>
          }
          @if (ann.error(); as e) {
            <span class="ann-err" role="alert" [title]="e">Annotations unavailable</span>
            <button type="button" class="btn" (click)="ann.retry()">Retry</button>
          }
        </div>
      }
      <div class="zoom" role="group" aria-label="Zoom">
        <button class="icon-btn" title="Previous range (Alt+←)" aria-label="Previous range" [disabled]="!zoom.canBack()" (click)="zoom.back()">←</button>
        <button class="icon-btn" title="Next range (Alt+→)" aria-label="Next range" [disabled]="!zoom.canForward()" (click)="zoom.forward()">→</button>
        <button class="icon-btn" title="Zoom in" aria-label="Zoom in" (click)="zoom.zoomIn()"><imt-icon name="plus" [size]="13" /></button>
        <button class="icon-btn" title="Zoom out" aria-label="Zoom out" (click)="zoom.zoomOut()"><imt-icon name="minus" [size]="13" /></button>
        <button class="btn" title="Fit all (double-click)" [disabled]="!zoom.isZoomed()" (click)="zoom.fit()"><imt-icon name="zoomOut" [size]="13" /> Fit</button>
      </div>
    </div>

    <div
      class="wrap"
      #wrap
      [attr.data-from]="tl()?.from"
      [attr.data-to]="tl()?.to"
      [class.panning]="panning()"
      [imtTimeCursor]="range()"
      [imtTimeCursorLeft]="plotLeft()"
      [imtTimeCursorWidth]="plotWidth()"
      (pointerdown)="pointerDown($event)"
      (dblclick)="zoom.fit()"
      (wheel)="onWheel($event)"
    >
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
            <div class="plot axis" #axis (contextmenu)="axisMenu($event)" title="Right-click to analyse a moment">
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
              @if (annLayout().lanes.get(l.lane.key); as items) {
                <imt-annotation-track [items]="items" (hovered)="annHover.set($event)" (activated)="ann.showEvidence($event.ann)" />
              }
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
                  <imt-marker-glyph [kind]="m.cluster.kind" [size]="16" />
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
        @if (annLayout().global.length) {
          <imt-annotation-track class="global" [items]="annLayout().global" (hovered)="annHover.set($event)" (activated)="ann.showEvidence($event.ann)" />
        }
        <imt-profile-span-track [view]="range()" [width]="plotWidth()" />
        @for (x of fatalLines(); track $index) {
          <div class="fatal-line" [style.left.%]="x"></div>
        }
        @if (brush(); as b) {
          <div class="brush" [style.left.px]="b.x0" [style.width.px]="b.x1 - b.x0"></div>
        }
        @if (hoverX() !== null) {
          <div class="cursor" [style.left.px]="hoverX()">
            <span class="cursor-label mono">{{ hoverTime() }}</span>
          </div>
        }
      </div>
    </div>

    @if (store.timelineError(); as err) {
      <div class="err-card" role="alert">
        <strong>Could not load the timeline</strong>
        <span class="mono">{{ err }}</span>
        <button class="btn" type="button" (click)="store.retryTimeline()">Retry</button>
      </div>
    }

    @if (hover(); as h) {
      <div class="popover tip" [style.left.px]="h.x + 14" [style.top.px]="h.y + 12" role="tooltip">
        <div class="tip-head">
          <imt-marker-glyph [kind]="h.marker.cluster.kind" [size]="18" />
          <strong>{{ h.marker.label }}</strong>
          <span class="faint">{{ h.marker.cluster.count === 1 ? '1 event' : h.marker.cluster.count + ' events in this interval' }}</span>
          <span class="spacer"></span>
          <span class="mono faint">{{ time(h.marker.cluster.ts) }}</span>
        </div>
        @if (describe(h.marker.cluster.kind); as d) { <div class="tip-desc">{{ d }}</div> }
        <div class="mono small">{{ h.lane }}</div>
        <div class="tip-msg">{{ h.marker.cluster.message }}</div>
        <div class="faint small">Click to jump to {{ h.marker.cluster.count > 1 ? 'the first event' : 'the event' }}</div>
      </div>
    }

    @if (annHover(); as h) {
      <div class="popover tip" data-testid="annotation-tip" [style.left.px]="h.x + 14" [style.top.px]="h.y + 12" role="tooltip">
        <div class="tip-head">
          <imt-severity-badge [severity]="h.vm.ann.severity" />
          <strong>{{ h.vm.ann.label }}</strong>
        </div>
        <div class="mono small">{{ annTime(h.vm.ann.from, h.vm.ann.to) }}</div>
        <div class="faint small">{{ annKindLabel(h.vm.ann.kind) }} · from {{ h.vm.ann.owner }}</div>
        @if (h.vm.ann.instance; as inst) { <div class="mono small">{{ inst }}</div> }
        @else if (h.vm.ann.on; as on) { <div class="mono small">{{ on.field }} = {{ on.value }}</div> }
        <div class="faint small">Click to show the evidence</div>
      </div>
    }
  `,
  styles: `
    :host { flex: 1; min-height: 0; min-width: 0; display: flex; flex-direction: column; background: var(--bg-1); overflow: hidden; position: relative; }
    .err-card { position: absolute; top: 60px; left: 50%; transform: translateX(-50%); z-index: 3; display: flex; flex-direction: column; align-items: center; gap: 8px; max-width: 80%; padding: 14px 18px; border: 1px solid var(--border-3); border-radius: 8px; background: var(--bg-3); color: var(--danger-text); text-align: center; }
    .head { height: 32px; flex: none; display: flex; align-items: center; gap: 12px; padding: 0 14px; }
    .hint { font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .spacer { flex: 1; }
    .legend, .ann-legend { display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--muted); flex-wrap: nowrap; overflow-x: auto; max-width: 60%; }
    .lg { display: inline-flex; align-items: center; gap: 5px; white-space: nowrap; border: 1px solid transparent; background: transparent; color: inherit; font: inherit; padding: 1px 6px; border-radius: 10px; cursor: pointer; }
    .lg:hover { background: var(--bg-hover); }
    .lg[aria-pressed='true'] { border-color: var(--accent); background: var(--accent-soft); }
    .tip-desc { color: var(--text); }
    .ann-legend { max-width: 30%; }
    .lg-title { font-weight: 600; color: var(--text-2); white-space: nowrap; }
    .lg[aria-pressed='false'] { opacity: 0.5; text-decoration: line-through; }
    .ann-legend .lg[aria-pressed='true'] { border-color: transparent; background: transparent; }
    .ann-err { color: var(--danger-text); white-space: nowrap; }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
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
    .marker:hover imt-marker-glyph { transform: scale(1.35); }
    .count { position: absolute; left: 17px; top: 2px; font-size: 9px; padding: 0 2px; border-radius: 2px; color: var(--text-2); background: var(--bg-1); pointer-events: none; }
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
  protected readonly cursor = inject(TimeCursor);
  private readonly targets = inject(AnalysisTargets);
  private readonly commands = inject(COMMAND_CONTEXT);
  private readonly menu = inject(ContextMenuService);

  protected readonly tl = this.store.timeline;
  protected readonly ann = inject(TimelineAnnotations);
  protected readonly annHover = signal<AnnotationHover | null>(null);

  /** Annotation bands and glyphs per lane, plus the full-height ones. */
  protected readonly annLayout = computed(() => {
    const view = this.range();
    if (!view) return { lanes: new Map(), global: [] };
    const q = this.store.query();
    return layoutAnnotations(this.ann.items(), {
      laneKeys: this.lanes().map((l) => l.lane.key),
      view,
      groupBy: q.groupBy.length ? q.groupBy : (this.store.summary()?.defaultGroupBy ?? []),
      hidden: this.ann.hidden(),
    });
  });

  protected annTime(from: number, to: number): string {
    const tz = this.store.timezone();
    return from === to ? formatDateTime(from, tz, true) : `${formatDateTime(from, tz, true)} → ${formatDateTime(to, tz, true)} (${formatDuration(to - from)})`;
  }

  protected annKindLabel(kind: string): string {
    return this.ann.kinds().find((k) => k.kind === kind)?.label ?? kind;
  }
  private readonly markerStyles = inject(MarkerStyles);

  protected describe(kind: string): string | null {
    return this.markerStyles.description(kind);
  }

  protected legendTitle(k: MarkerKind): string {
    return styleOf(k).description ?? k.label;
  }

  /** Id of the hidden description of a legend item (null when the marker has none). */
  protected legendDesc(k: MarkerKind): string | null {
    return styleOf(k).description ? 'lg-desc-' + k.kind.replace(/[^\w-]/g, '_') : null;
  }

  /** True when the query is restricted to exactly this marker kind. */
  protected legendActive(kind: string): boolean {
    const m = this.store.query().markers;
    return m.length === 1 && m[0] === kind;
  }

  /** Clicking a legend item shows only that marker's events; clicking again clears it. */
  protected toggleKind(kind: string): void {
    this.store.patchQuery({ markers: this.legendActive(kind) ? [] : [kind] });
  }

  /** The visible time range (null before the first timeline arrives). */
  protected readonly range = computed<Range | null>(() => {
    const t = this.tl();
    return t ? { from: t.from, to: t.to } : null;
  });

  /** X (px in the plot) of the shared time cursor, from any time-based view; null when outside. */
  protected readonly hoverX = computed(() => {
    const frac = timeToFrac(this.cursor.hovered(), this.range());
    return frac === null ? null : frac * this.plotWidth();
  });

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
    const ts = this.cursor.hovered();
    return ts === null || this.hoverX() === null ? '' : formatDateTime(ts, this.store.timezone(), true);
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

  private viewRange(): Range | null {
    return this.range();
  }

  /** Right-click on the time axis: analyse the moment under the pointer. */
  protected axisMenu(e: MouseEvent): void {
    const view = this.range();
    if (!view) return;
    e.preventDefault();
    const ts = xToTs(clamp(this.localX(e), 0, this.plotWidth()), this.plotWidth(), view);
    this.menu.open(e.clientX, e.clientY, [
      {
        label: 'Analyse this moment',
        run: () => {
          this.targets.setMoment(momentAt(ts, this.cursor.selection(), WIDTHS[1].ms));
          this.commands.showView('moment');
        },
      },
    ]);
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
    if (!view || !this.zoom.canPan()) return;
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
      const t = this.tl();
      if (!b || !t || b.x1 - b.x0 < 5) return;
      const view = { from: t.from, to: t.to };
      const selected = { from: xToTs(b.x0, w, view), to: xToTs(b.x1, w, view) };
      this.cursor.select(selected);
      this.zoom.apply(selected);
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
  }
}
