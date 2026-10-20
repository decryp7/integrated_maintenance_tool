import { afterNextRender, ChangeDetectionStrategy, Component, computed, DestroyRef, effect, ElementRef, inject, signal, viewChild } from '@angular/core';
import { AnnotationDto, LEVELS } from '../core/api.types';
import { clamp, formatDateTime, timeTicks } from '../core/format';
import { SelectionStore } from '../core/workspace/selection.store';
import { UiPrefsStore } from '../core/workspace/ui-prefs.store';
import { WorkspaceDataStore } from '../core/workspace/workspace-data.store';
import { HIST_VARS } from '../shared/colors';
import { LaidMarker } from './marker-layout';
import { Range, xToTs } from './zoom';
import { AnalysisTargets } from '../analysis/analysis-targets';
import { TimeCursor, TimeCursorLine } from '../analysis/time-cursor';
import { bucketsForWidth, RESIZE_SETTLE_MS } from '../shared/bucket-count';
import { Debouncer } from '../shared/throttle';
import { momentAt, WIDTHS } from '../analysis/windows';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { ContextMenuService } from '../ui/context-menu.service';
import { AnnotationVm, layoutAnnotations } from './annotation-layout';
import { AnnotationHover, AnnotationTrack } from './annotation-track';
import { TimelineAnnotations } from './annotations.store';
import { layoutRuns, RunBandVm } from './run-layout';
import { RunHover } from './run-track';
import { ProfileSpanTrack } from '../profiling/integration/profile-span-track';
import { DOCK_PANEL_ID } from '../dock/angular-panel';
import { DockService } from '../dock/dock.service';
import { isTimelineInstance } from './timeline-config';
import { TimelineInstanceStore } from './timeline-instance.store';
import { MainTimelineSource, TIMELINE_SOURCE, TimelineSource } from './timeline-source';
import { LANE_DOTS, SourceColors } from '../sources/source-colors.store';
import { buildLaneVms, HIST_H, LABEL_W } from './timeline-lane-vm';
import { LanesEmpty, MarkerHover, TimelineLanes } from './timeline-lanes';
import { TimelineHeader } from './timeline-header';
import { TimelineLegend } from './timeline-legend';
import { TimelineTooltips } from './timeline-tooltips';
import { TimelineInteraction } from './timeline-interaction';

/** The panel's source: its own state for an additional timeline (`timeline#n`), the workspace's for the main one. */
function timelineSource(): TimelineSource {
  return isTimelineInstance(inject(DOCK_PANEL_ID, { optional: true })) ? inject(TimelineInstanceStore) : inject(MainTimelineSource);
}

/**
 * A timeline panel: header, legend, histogram and time axis, lanes, overlays (annotations, profiling spans, fatal
 * lines, brush, time cursor) and hover cards. It composes the parts and computes their view models; the parts draw
 * them, and `TimelineInteraction` handles pointer and keyboard input on the plot.
 */
@Component({
  selector: 'imt-timeline-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [TimeCursorLine, AnnotationTrack, ProfileSpanTrack, TimelineHeader, TimelineLegend, TimelineLanes, TimelineTooltips, TimelineInteraction],
  // One store per panel: an additional timeline gets its own `TimelineInstanceStore` (created only when used).
  providers: [TimelineInstanceStore, { provide: TIMELINE_SOURCE, useFactory: timelineSource }],
  host: { '[class.instance]': '!src.main' },
  template: `
    <imt-timeline-header />
    <imt-timeline-legend [inView]="annInView()" />

    <div
      class="wrap"
      #wrap
      [imtTimelineInteraction]="range()"
      #ix="imtTimelineInteraction"
      [plotLeft]="plotLeft()"
      [plotWidth]="plotWidth()"
      [panel]="hostEl"
      [attr.data-from]="tl()?.from"
      [attr.data-to]="tl()?.to"
    >
      <div class="scroller">
        <div class="sticky">
          <div class="row hist-row">
            <div class="label faint small" i18n="@@timeline.panel.eventsPerBucket">events / bucket</div>
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
            <div class="plot axis" #axis (contextmenu)="axisMenu($event)" title="Right-click to analyse a moment" i18n-title="@@timeline.panel.axisTitle">
              @for (t of ticks(); track t.ts) {
                <span class="tick mono" [style.left.%]="t.frac * 100">{{ t.label }}</span>
              }
            </div>
          </div>
        </div>

        <imt-timeline-lanes
          [lanes]="lanes()"
          [runLayout]="runLayout()"
          [annLanes]="annLayout().lanes"
          [densityViewBox]="densityViewBox()"
          [main]="src.main"
          [empty]="empty()"
          [truncated]="tl()?.truncatedLanes ?? 0"
          (toggleGroup)="src.toggleGroup($event)"
          (jump)="selection.jumpTo($event)"
          (markerHover)="hover.set($event)"
          (runHover)="runHover.set($event)"
          (annHover)="annHover.set($event)"
          (showRun)="ann.showRun($event)"
          (showEvidence)="ann.showEvidence($event)"
          (openFolder)="data.pickAndOpen(true)"
        />
      </div>

      <div class="overlay" [style.left.px]="plotLeft()" [style.width.px]="plotWidth()">
        @if (annLayout().global.length) {
          <imt-annotation-track class="global" [items]="annLayout().global" (hovered)="annHover.set($event)" (activated)="ann.showEvidence($event.ann)" />
        }
        <imt-profile-span-track [view]="range()" [width]="plotWidth()" />
        @for (x of fatalLines(); track $index) {
          <div class="fatal-line" [style.left.%]="x"></div>
        }
        @if (ix.brush(); as b) {
          <div class="brush" [style.left.px]="b.x0" [style.width.px]="b.x1 - b.x0"></div>
        }
        <div class="cursor" [imtTimeCursorLine]="range()" [imtTimeCursorLineOf]="cursor" [imtTimeCursorLineWidth]="plotWidth()" [imtTimeCursorLineLabel]="cursorLabel">
          <span class="cursor-label mono" data-cursor-label></span>
        </div>
      </div>
    </div>

    @if (src.error(); as err) {
      <div class="err-card" role="alert">
        <strong i18n="@@timeline.error.title">Could not load the timeline</strong>
        <span class="mono">{{ err }}</span>
        <button class="btn" type="button" (click)="src.retry()" i18n="@@timeline.error.retry">Retry</button>
      </div>
    }

    <imt-timeline-tooltips [marker]="hover()" [run]="runHover()" [annotation]="annHover()" />
  `,
  styles: `
    :host { flex: 1; min-height: 0; min-width: 0; display: flex; flex-direction: column; background: var(--bg-1); overflow: hidden; position: relative; }
    .err-card { position: absolute; top: 60px; left: 50%; transform: translateX(-50%); z-index: 3; display: flex; flex-direction: column; align-items: center; gap: 8px; max-width: 80%; padding: 14px 18px; border: 1px solid var(--border-3); border-radius: 8px; background: var(--bg-3); color: var(--danger-text); text-align: center; }
    .wrap { position: relative; flex: 1; min-height: 0; user-select: none; }
    .wrap.panning { cursor: grabbing; }
    .scroller { position: absolute; inset: 0; overflow-y: auto; overflow-x: hidden; scrollbar-gutter: stable; padding-bottom: 8px; }
    .sticky { position: sticky; top: 0; z-index: 2; background: var(--bg-1); }
    .row { display: flex; align-items: stretch; }
    .label { width: ${LABEL_W}px; flex: none; }
    .plot { flex: 1; min-width: 0; position: relative; margin-right: 14px; }
    .small { font-size: var(--fs-xs); }
    .hist-row .label { display: flex; align-items: flex-end; padding: 0 14px 2px; }
    .hist { display: block; width: 100%; height: ${HIST_H}px; }
    .axis { height: 20px; border-bottom: 1px solid var(--border); }
    .tick { position: absolute; top: 3px; transform: translateX(-50%); font-size: var(--fs-2xs); color: var(--faint); white-space: nowrap; }
    .overlay { position: absolute; top: 0; bottom: 0; pointer-events: none; }
    .brush { position: absolute; top: 0; bottom: 0; background: var(--accent-soft); border-left: 1px solid var(--accent); border-right: 1px solid var(--accent); }
    .fatal-line { position: absolute; top: ${HIST_H}px; bottom: 0; width: 0; border-left: 1px dashed var(--mk-fatal); opacity: 0.55; }
    .cursor { position: absolute; top: 0; bottom: 0; width: 0; border-left: 1px solid var(--border-3); }
    .cursor-label { position: absolute; top: ${HIST_H + 2}px; left: 4px; font-size: var(--fs-2xs); padding: 1px 4px; border-radius: 3px; background: var(--tooltip-bg); border: 1px solid var(--border-3); color: var(--text-2); white-space: nowrap; }
  `,
})
export class TimelinePanel {
  /** Source / instance colour dots of the lane labels (View › Source colours). */
  private readonly sourceColors = inject(SourceColors);
  protected readonly data = inject(WorkspaceDataStore);
  protected readonly selection = inject(SelectionStore);
  private readonly prefs = inject(UiPrefsStore);
  /** What this panel shows: the workspace's timeline, or an additional timeline's own state. */
  protected readonly src = inject(TIMELINE_SOURCE);
  /** The additional timeline's store; null for the main timeline. */
  private readonly inst = this.src.main ? null : inject(TimelineInstanceStore);
  private readonly dock = inject(DockService);
  protected readonly hostEl = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;

  /** The time cursor in effect (shared with every time-based view while linked). */
  protected get cursor(): TimeCursor {
    return this.src.cursor();
  }
  private readonly wrap = viewChild.required<ElementRef<HTMLElement>>('wrap');
  private readonly axis = viewChild.required<ElementRef<HTMLElement>>('axis');
  private readonly interaction = viewChild.required(TimelineInteraction);

  protected readonly plotLeft = signal(LABEL_W);
  protected readonly plotWidth = signal(800);
  protected readonly hover = signal<MarkerHover | null>(null);
  private readonly targets = inject(AnalysisTargets);
  private readonly commands = inject(COMMAND_CONTEXT);
  private readonly menu = inject(ContextMenuService);

  protected readonly tl = this.src.timeline;
  protected readonly ann = inject(TimelineAnnotations);
  protected readonly annHover = signal<AnnotationHover | null>(null);
  protected readonly runHover = signal<RunHover | null>(null);

  /** The visible time range (null before the first timeline arrives). */
  protected readonly range = computed<Range | null>(() => {
    const t = this.tl();
    return t ? { from: t.from, to: t.to } : null;
  });

  /** The lanes as drawn (order kept stable by the source while zoomed). */
  protected readonly lanes = computed(() => {
    const t = this.tl();
    if (!t) return [];
    const tz = this.prefs.timezone();
    const sourceMode = this.sourceColors.mode() === 'source';
    return buildLaneVms(this.src.lanes(), t, {
      kinds: this.data.markerKindMap(),
      groups: new Set(this.src.groups()),
      timezone: tz,
      plotWidth: this.plotWidth(),
      main: this.src.main,
      marks: (lane) => this.sourceColors.forLane(lane),
      moreSources: (lane) => (sourceMode ? Math.max(0, lane.sources.length - LANE_DOTS) : 0),
      markerAria: (m) => this.markerAria(m),
    });
  });

  protected readonly laneKeys = computed(() => this.lanes().map((l) => l.lane.key));

  /** Run bands per lane (behind the markers). */
  protected readonly runLayout = computed(() => {
    const view = this.range();
    if (!view) return new Map<string, RunBandVm[]>();
    return layoutRuns(this.ann.runs(), { laneKeys: this.laneKeys(), view, hidden: this.ann.hiddenRuns(), shown: this.ann.runsShown() });
  });

  /** Annotation bands and glyphs per lane, plus the full-height ones. */
  protected readonly annLayout = computed(() => {
    const view = this.range();
    if (!view) return { lanes: new Map<string, AnnotationVm[]>(), global: [] as AnnotationVm[] };
    return layoutAnnotations(this.ann.plugin(), { laneKeys: this.laneKeys(), view, groupBy: [...this.src.groupBy()], hidden: this.ann.hidden() });
  });

  /** The annotations and runs in view, once each (one may be drawn on several lanes), in time order. */
  protected readonly annInView = computed(() => {
    const { lanes, global } = this.annLayout();
    const seen = new Set<AnnotationDto>();
    for (const v of [...global, ...[...lanes.values()].flat()]) seen.add(v.ann);
    for (const v of [...this.runLayout().values()].flat()) seen.add(v.ann);
    return [...seen].sort((a, b) => a.from - b.from || a.label.localeCompare(b.label));
  });

  /** What the lanes say when there are none. */
  protected readonly empty = computed<LanesEmpty>(() => {
    if (!this.data.summary()?.sources?.length) return 'noSources';
    if (!this.tl()) return null;
    return this.src.main ? 'noMatchMain' : 'noMatchOwn';
  });

  protected readonly ticks = computed(() => {
    const t = this.tl();
    if (!t) return [];
    return timeTicks(t.from, t.to, Math.max(2, Math.floor(this.plotWidth() / 110)), this.prefs.timezone());
  });

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
        paths[li]?.push(`M${x + 0.1} ${y.toFixed(2)}h0.8v${h.toFixed(2)}h-0.8z`);
      });
    });
    return LEVELS.map((level, i) => ({ level, d: paths[i]?.join('') ?? '', color: HIST_VARS[level] })).filter((p) => p.d);
  });

  protected readonly fatalLines = computed(() => this.lanes().flatMap((l) => l.markers.filter((m) => m.cluster.kind === 'fatal').map((m) => m.left)));

  /** The cursor line's label: the hovered time (written by `TimeCursorLine`, so hovering does not re-render this view). */
  protected readonly cursorLabel = (ts: number): string => formatDateTime(ts, this.prefs.timezone(), true);

  /** Resizing redraws at once but re-queries the timeline only once the size has settled. */
  private readonly resized = new Debouncer(RESIZE_SETTLE_MS);
  private bucketsSet = false;

  constructor() {
    // The annotations are fetched while a timeline (or another view showing them) is visible.
    this.ann.gate.use();
    const destroyRef = inject(DestroyRef);
    destroyRef.onDestroy(() => this.resized.cancel());
    afterNextRender(() => {
      const ro = new ResizeObserver(() => this.measure());
      ro.observe(this.axis().nativeElement);
      ro.observe(this.wrap().nativeElement);
      destroyRef.onDestroy(() => ro.disconnect());
      this.measure();
    });
    // An additional timeline's tab shows its title (restored layouts and renames included).
    const inst = this.inst;
    if (inst) effect(() => this.dock.setTitle(inst.id, inst.title()));
  }

  private markerAria(m: LaidMarker): string {
    const label = m.label;
    const at = this.time(m.cluster.ts);
    const count = m.cluster.count;
    return count > 1
      ? $localize`:@@timeline.marker.ariaMany:${label}:label: at ${at}:time:, ${count}:count: events`
      : $localize`:@@timeline.marker.ariaOne:${label}:label: at ${at}:time:`;
  }

  private measure(): void {
    const wrap = this.wrap().nativeElement.getBoundingClientRect();
    const axis = this.axis().nativeElement.getBoundingClientRect();
    if (axis.width <= 0) return;
    this.plotLeft.set(axis.left - wrap.left);
    this.plotWidth.set(axis.width);
    const buckets = bucketsForWidth(axis.width);
    if (!this.bucketsSet) {
      // The first size is known at once; later ones (a splitter drag) only once they settle.
      this.bucketsSet = true;
      this.src.setBuckets(buckets);
    } else this.resized.run(() => this.src.setBuckets(buckets));
  }

  private time(ts: number): string {
    return formatDateTime(ts, this.prefs.timezone(), true);
  }

  /** Right-click on the time axis: analyse the moment under the pointer. */
  protected axisMenu(e: MouseEvent): void {
    const view = this.range();
    if (!view) return;
    e.preventDefault();
    const ts = xToTs(clamp(this.interaction().localX(e), 0, this.plotWidth()), this.plotWidth(), view);
    this.menu.open(e.clientX, e.clientY, [
      {
        label: $localize`:@@timeline.menu.analyseMoment:Analyse this moment`,
        run: () => {
          this.targets.setMoment(momentAt(ts, this.cursor.selection(), WIDTHS[1]?.ms ?? 300_000));
          this.commands.showView('moment');
        },
      },
    ]);
  }
}
