import { afterNextRender, ChangeDetectionStrategy, Component, computed, DestroyRef, effect, ElementRef, inject, signal, viewChild } from '@angular/core';
import { ErrorDetails } from '../ui/error-details';
import { AnnotationDto } from '../core/api.types';
import { clamp, formatCount, formatDateTime, formatDuration, formatTime, timeTicks } from '../core/format';
import { SelectionStore } from '../core/workspace/selection.store';
import { UiPrefsStore } from '../core/workspace/ui-prefs.store';
import { WorkspaceDataStore } from '../core/workspace/workspace-data.store';
import { LaidMarker } from './marker-layout';
import { Range, xToTs } from './zoom';
import { AnalysisTargets } from '../analysis/analysis-targets';
import { TimeCursor, TimeCursorLine } from '../analysis/time-cursor';
import { anchors, NoteActions } from '../notes/note-actions';
import { NoteTimelineTrack } from '../notes/note-timeline-track';
import { bucketsForWidth, RESIZE_SETTLE_MS } from '../shared/bucket-count';
import { Debouncer } from '../shared/throttle';
import { momentAt, WIDTHS } from '../analysis/windows';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { ContextMenuService } from '../ui/context-menu.service';
import { AnnotationVm, layoutAnnotations } from './annotation-layout';
import { AnnotationHover, AnnotationTrack, SpanMenu } from './annotation-track';
import { TimelineAnnotations } from './annotations.store';
import { layoutRuns, RunBandVm } from './run-layout';
import { RunHover } from './run-track';
import { ProfileSpanTrack } from '../profiling/integration/profile-span-track';
import { DumpMarkerTrack } from '../dumps/dump-marker-track';
import { DOCK_PANEL_ID } from '../dock/angular-panel';
import { DockService } from '../dock/dock.service';
import { isTimelineInstance } from './timeline-config';
import { TimelineInstanceStore } from './timeline-instance.store';
import { MainTimelineSource, TIMELINE_SOURCE, TimelineSource } from './timeline-source';
import { LANE_DOTS, SourceColors } from '../sources/source-colors.store';
import { buildLaneVms, DENSITY_VB, LABEL_W } from './timeline-lane-vm';
import { HIST_VB, histogramPaths } from './histogram';
import { LanesEmpty, MarkerHover, MarkerMenu, TimelineLanes } from './timeline-lanes';
import { TimelineHeader } from './timeline-header';
import { TimelineLegend } from './timeline-legend';
import { TimelineMinimap } from './timeline-minimap';
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
  imports: [TimeCursorLine, AnnotationTrack, ProfileSpanTrack, DumpMarkerTrack, NoteTimelineTrack, TimelineHeader, TimelineLegend, TimelineMinimap, TimelineLanes, TimelineTooltips, TimelineInteraction, ErrorDetails],
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
          <div class="row mini-row">
            <div class="label"></div>
            <div class="plot"><imt-timeline-minimap /></div>
          </div>
          <div class="row hist-row">
            <div class="label hist-label small">
              <span class="faint" i18n="@@timeline.panel.eventsPerBucket">events / bucket</span>
              @if (hist().max > 0) {
                <span class="scale mono" title="Events in the busiest bucket" i18n-title="@@timeline.panel.histMaxTitle" i18n="Scale of the events histogram@@timeline.panel.histMax">max {{ histMax() }}</span>
              }
            </div>
            <div class="plot">
              <svg class="hist" [attr.viewBox]="histViewBox()" preserveAspectRatio="none" aria-hidden="true">
                @for (p of hist().paths; track p.level) {
                  <path [attr.d]="p.d" [style.fill]="p.color" [attr.data-level]="p.level" />
                }
              </svg>
            </div>
          </div>
          <div class="row axis-row">
            <div class="label"></div>
            <div class="plot axis" #axis (contextmenu)="axisMenu($event)" title="Right-click to analyse a moment" i18n-title="@@timeline.panel.axisTitle">
              @for (t of ticks(); track t.ts) {
                <span class="tick mono" [class.major]="t.major" [style.left.%]="t.frac * 100">{{ t.label }}</span>
              }
            </div>
          </div>
        </div>

        <imt-timeline-lanes
          [lanes]="lanes()"
          [runLayout]="runLayout()"
          [annLanes]="annLayout().lanes"
          [densityViewBox]="densityViewBox()"
          [selectedId]="selection.selected()?.id ?? null"
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
          (markerMenu)="markerMenu($event)"
          (spanMenu)="spanMenu($event)"
        />
      </div>

      <div class="overlay" [style.left.px]="plotLeft()" [style.width.px]="plotWidth()">
        @if (annLayout().global.length) {
          <imt-annotation-track class="global" [items]="annLayout().global" (hovered)="annHover.set($event)" (activated)="ann.showEvidence($event.ann)" (menuRequested)="spanMenu($event)" />
        }
        <imt-profile-span-track [view]="range()" [width]="plotWidth()" />
        <imt-dump-marker-track [view]="range()" [width]="plotWidth()" />
        <imt-note-timeline-track [view]="range()" [width]="plotWidth()" />
        @for (x of fatalLines(); track $index) {
          <!-- The band shows only where the theme asks for it (data-crash-band); the rule always. -->
          <div class="crash-band" [style.left.%]="x" aria-hidden="true"></div>
          <div class="fatal-line" [style.left.%]="x"></div>
        }
        @if (ix.brush(); as b) {
          <div class="brush" [style.left.px]="b.x0" [style.width.px]="b.x1 - b.x0"><span class="handle l"></span><span class="handle r"></span></div>
          <!-- The floating summary shows only where the theme asks for it (data-brush-pill). -->
          <div class="brush-pill mono" data-testid="brush-pill" [style.left.px]="(b.x0 + b.x1) / 2" aria-hidden="true">{{ brushText(b) }}</div>
        }
        <div class="cursor" [imtTimeCursorLine]="range()" [imtTimeCursorLineOf]="cursor" [imtTimeCursorLineWidth]="plotWidth()" [imtTimeCursorLineLabel]="cursorLabel">
          <span class="cursor-label mono" data-cursor-label></span>
        </div>
      </div>
    </div>

    @if (src.error(); as err) {
      <div class="err-card" role="alert">
        <imt-error-details summary="Couldn't load the timeline." i18n-summary="@@timeline.error.summary" [detail]="err" (retry)="src.retry()" />
      </div>
    }

    <imt-timeline-tooltips [marker]="hover()" [run]="runHover()" [annotation]="annHover()" />
  `,
  styles: `
    :host { flex: 1; min-height: 0; min-width: 0; display: flex; flex-direction: column; background: var(--bg-1); overflow: hidden; position: relative; }
    .err-card { position: absolute; top: 60px; left: 50%; transform: translateX(-50%); z-index: 3; display: flex; flex-direction: column; align-items: center; gap: 8px; max-width: 80%; padding: 14px 18px; border: 1px solid var(--border-3); border-radius: 8px; background: var(--bg-3); color: var(--text); text-align: center; }
    .wrap { position: relative; flex: 1; min-height: 0; user-select: none; }
    .wrap.panning { cursor: grabbing; }
    .scroller { position: absolute; inset: 0; overflow-y: auto; overflow-x: hidden; scrollbar-gutter: stable; padding-bottom: 8px; }
    .sticky { position: sticky; top: 0; z-index: 2; background: var(--bg-1); }
    .row { display: flex; align-items: stretch; }
    .label { width: var(--tl-label-w); flex: none; }
    .plot { flex: 1; min-width: 0; position: relative; margin-right: var(--s-5); }
    .small { font-size: var(--fs-xs); }
    .hist-label { display: flex; flex-direction: column; justify-content: flex-end; gap: 1px; padding: 0 var(--s-3) 3px var(--s-4); }
    .scale { font-size: var(--fs-2xs); color: var(--muted); }
    .hist { display: block; width: 100%; height: var(--hist-h); }
    .axis { height: 22px; border-bottom: 1px solid var(--border); }
    .tick { position: absolute; top: 5px; transform: translateX(-50%); font-size: var(--fs-2xs); color: var(--muted); white-space: nowrap; }
    .tick::before { content: ''; position: absolute; left: 50%; top: -5px; height: 2px; border-left: 1px solid var(--border-3); }
    .tick.major { color: var(--text-2); font-weight: 500; }
    .tick.major::before { height: 4px; }
    .overlay { position: absolute; top: 0; bottom: 0; pointer-events: none; }
    .brush { position: absolute; top: 0; bottom: 0; background: var(--brush-fill); border-left: 1px solid var(--brush-edge); border-right: 1px solid var(--brush-edge); }
    .handle { position: absolute; top: calc(var(--hist-h) / 2 - 9px); width: 6px; height: 18px; margin-left: -3px; border-radius: 3px; background: var(--bg-1); border: 1px solid var(--brush-edge); box-shadow: var(--shadow-1); }
    .handle.l { left: 0; }
    .handle.r { left: 100%; }
    .brush-pill { display: none; position: absolute; top: 6px; transform: translateX(-50%); padding: 3px 10px; border-radius: 999px; background: var(--tooltip-bg); border: 1px solid var(--border-3); box-shadow: var(--shadow-pop); color: var(--text); font-size: var(--fs-xs); white-space: nowrap; z-index: 3; }
    :host-context([data-brush-pill='on']) .brush-pill { display: block; }
    .fatal-line { position: absolute; top: var(--hist-h); bottom: 0; width: 0; border-left: 1px dashed var(--mk-fatal); opacity: 0.45; }
    .crash-band { display: none; position: absolute; top: 0; bottom: 0; width: 34px; margin-left: -17px; background: linear-gradient(90deg, transparent, color-mix(in srgb, var(--mk-fatal) 16%, transparent), transparent); }
    :host-context([data-crash-band='on']) .crash-band { display: block; }
    :host-context([data-crash-band='on']) .fatal-line { border-left-style: solid; opacity: 0.35; }
    .cursor { position: absolute; top: 0; bottom: 0; width: 0; border-left: 1px solid var(--crosshair); }
    .cursor-label { position: absolute; top: calc(var(--hist-h) + 3px); left: 4px; font-size: var(--fs-2xs); padding: 1px 5px; border-radius: var(--r-1); background: var(--readout-bg); color: var(--readout-text); white-space: nowrap; }
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
  private readonly notes = inject(NoteActions);

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
    return layoutRuns(this.ann.runs(), { laneKeys: this.laneKeys(), view, hidden: this.ann.hiddenRuns(), shown: this.ann.runsShown(), plotWidth: this.plotWidth() });
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
    // Whole hours (and days) are the major ticks: stronger label, longer tick.
    return timeTicks(t.from, t.to, Math.max(2, Math.floor(this.plotWidth() / 110)), this.prefs.timezone()).map((tick) => ({ ...tick, major: /:00$/.test(tick.label) || !tick.label.includes(':') }));
  });

  protected readonly histViewBox = computed(() => `0 0 ${Math.max(1, this.tl()?.histogram.length ?? 1)} ${HIST_VB}`);
  protected readonly densityViewBox = computed(() => `0 0 ${Math.max(1, this.tl()?.histogram.length ?? 1)} ${DENSITY_VB}`);

  /** One path per level, the most severe on the baseline, scaled to the busiest bucket. */
  protected readonly hist = computed(() => histogramPaths(this.tl()?.histogram ?? []));
  protected readonly histMax = computed(() => formatCount(this.hist().max));

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
    // The badge shows the formatted count, so the accessible name carries the same text (WCAG 2.5.3).
    const count = formatCount(m.cluster.count);
    return m.cluster.count > 1
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

  /** "10:36:05 → 10:52:40 · 16m 35s": the brushed range while dragging. */
  protected brushText(b: { x0: number; x1: number }): string {
    const view = this.range();
    const w = this.plotWidth();
    if (!view || w <= 0) return '';
    const from = xToTs(Math.min(b.x0, b.x1), w, view);
    const to = xToTs(Math.max(b.x0, b.x1), w, view);
    const tz = this.prefs.timezone();
    return `${formatTime(from, tz).slice(0, 8)} → ${formatTime(to, tz).slice(0, 8)} · ${formatDuration(to - from)}`;
  }

  /** Right-click on a marker: write a note on it (its kind's label and time), or show its event. */
  protected markerMenu(m: MarkerMenu): void {
    const { cluster, label } = m.marker;
    this.menu.open(m.x, m.y, [
      { label: $localize`:@@timeline.menu.noteMarker:Add note to this marker…`, run: () => void this.notes.write(anchors.marker(label, cluster.ts)) },
      { label: $localize`:@@timeline.menu.showMarkerEvent:Show its event`, run: () => this.selection.jumpTo(cluster.id) },
    ]);
  }

  /** Right-click on a run or an annotation: write a note on its span, or show its evidence. */
  protected spanMenu(s: SpanMenu): void {
    const a = s.ann;
    this.menu.open(s.x, s.y, [
      { label: $localize`:@@timeline.menu.noteSpan:Add note to this span…`, run: () => void this.notes.write(anchors.marker(a.label, a.from, a.to)) },
      a.run
        ? { label: $localize`:@@timeline.menu.showRun:Show this run`, run: () => this.ann.showRun(a) }
        : { label: $localize`:@@timeline.menu.showEvidence:Show the evidence`, run: () => this.ann.showEvidence(a) },
    ]);
  }

  /** Right-click on the time axis: analyse the moment under the pointer, or write a note on it (or on the selected range). */
  protected axisMenu(e: MouseEvent): void {
    const view = this.range();
    if (!view) return;
    e.preventDefault();
    const ts = xToTs(clamp(this.interaction().localX(e), 0, this.plotWidth()), this.plotWidth(), view);
    const selected = this.cursor.selection();
    this.menu.open(e.clientX, e.clientY, [
      {
        label: $localize`:@@timeline.menu.analyseMoment:Analyse this moment`,
        run: () => {
          this.targets.setMoment(momentAt(ts, this.cursor.selection(), WIDTHS[1]?.ms ?? 300_000));
          this.commands.showView('moment');
        },
      },
      { label: $localize`:@@timeline.menu.noteMoment:Add note at this moment…`, run: () => void this.notes.write(anchors.instant(ts)) },
      ...(selected && selected.to > selected.from
        ? [{ label: $localize`:@@timeline.menu.noteRange:Add note to the selected range…`, run: () => void this.notes.write(anchors.range(selected.from, selected.to)) }]
        : []),
    ]);
  }
}
