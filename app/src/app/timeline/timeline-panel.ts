import {
  afterNextRender,
  ChangeDetectionStrategy,
  Component,
  computed,
  DestroyRef,
  effect,
  ElementRef,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import { AnnotationDto, Lane, LEVELS, MarkerKind } from '../core/api.types';
import { plural } from '../i18n/locale';
import { clamp, formatCount, formatDateTime, formatDuration, formatTime, timeTicks } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { HIST_VARS } from '../shared/colors';
import { Icon } from '../shared/icon';
import { MarkerGlyph } from '../markers/marker-glyph';
import { styleOf } from '../markers/marker-styles';
import { MarkerStyles } from '../markers/marker-styles';
import { LaidMarker, layoutLane, MarkerTarget, MarkerVm } from './marker-layout';
import { pan as panRange, Range, RangeZoom, wheelFactor, xToTs } from './zoom';
import { AnalysisTargets } from '../analysis/analysis-targets';
import { TimeCursor, timeToFrac, xToTime } from '../analysis/time-cursor';
import { momentAt, WIDTHS } from '../analysis/windows';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { ContextMenuService, MenuEntry } from '../ui/context-menu.service';
import { LiveWindowControls } from '../live/live-window-controls';
import { SeverityBadge } from '../live/severity-badge';
import { AnnotationVm, layoutAnnotations } from './annotation-layout';
import { AnnotationHover, AnnotationTrack } from './annotation-track';
import { TimelineAnnotations } from './annotations.store';
import { layoutRuns, RunBandVm } from './run-layout';
import { RunHover, RunTrack } from './run-track';
import { runAnomalyText, runDuration, runStatusLabel } from './run-text';
import { ProfileSpanTrack } from '../profiling/integration/profile-span-track';
import { DOCK_PANEL_ID } from '../dock/angular-panel';
import { DockService } from '../dock/dock.service';
import { Switch } from '../shared/switch';
import { AutoFocus } from '../shared/auto-focus';
import { isTimelineInstance, LANE_MODES, LaneMode, TimelineScope } from './timeline-config';
import { TimelineInstanceStore } from './timeline-instance.store';
import { MainTimelineSource, TIMELINE_SOURCE, TimelineSource } from './timeline-source';
import { TimelineViews } from './timeline-views';

const LABEL_W = 250;
/** At most this many annotations are listed in the Evidence menu (the rest: zoom in). */
const MAX_ANN_MENU = 50;
/** Kinds drawn as a thin tick strip when a lane is too busy for full shapes. */
const COMPACT_KINDS: ReadonlySet<string> = new Set(['warning']);
const HIST_H = 30;

/** "12 of 30" when a filter hides some of the lane's events, else just the count. */
function laneCount(lane: Lane): string {
  const matched = formatCount(lane.matched);
  if (lane.matched === lane.total) return matched;
  const total = formatCount(lane.total);
  return $localize`:@@timeline.lane.matchedOfTotal:${matched}:matched: of ${total}:total:`;
}

interface LaneVm {
  lane: Lane;
  left: number;
  width: number;
  densityPath: string;
  markers: LaidMarker[];
  targets: MarkerTarget[];
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

/** The panel's source: its own state for an additional timeline (`timeline#n`), the workspace's for the main one. */
function timelineSource(): TimelineSource {
  return isTimelineInstance(inject(DOCK_PANEL_ID, { optional: true })) ? inject(TimelineInstanceStore) : inject(MainTimelineSource);
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
  imports: [Icon, MarkerGlyph, LiveWindowControls, AnnotationTrack, RunTrack, SeverityBadge, ProfileSpanTrack, Switch, AutoFocus],
  // One store per panel: an additional timeline gets its own `TimelineInstanceStore` (created only when used).
  providers: [TimelineInstanceStore, { provide: TIMELINE_SOURCE, useFactory: timelineSource }],
  host: { '[class.instance]': '!src.main', '(document:keydown)': 'onKey($event)', '(document:keyup)': 'onKey($event)' },
  template: `
    <div class="head">
      @if (renaming()) {
        <input class="input title-input" imtAutoFocus aria-label="Timeline title" i18n-aria-label="@@timeline.panel.titleAria" maxlength="80" [value]="src.title()" (keydown)="renameKey($event)" (blur)="commitRename($event)" />
      } @else {
        <span class="section-title" [attr.data-testid]="src.main ? null : 'timeline-title'" (dblclick)="startRename()">{{ src.title() }}</span>
      }
      @if (inst && !inst.synced()) { <span class="unlinked" title="Zoom, pan and the time cursor are not linked to the main timeline" i18n-title="@@timeline.panel.unlinked.title" i18n="@@timeline.panel.unlinked">Unlinked</span> }
      <span class="faint hint">
        @if (tl(); as t) {
          {{ hint(t.lanes.length) }}
        }
      </span>
      @if (src.loading()) { <span class="spinner" role="status" aria-label="Loading" i18n-aria-label="@@timeline.panel.loading"></span> }
      <span class="spacer"></span>
      @if (src.main) { <imt-live-window-controls /> }
      <div class="zoom" role="group" aria-label="Zoom" i18n-aria-label="@@timeline.panel.zoom">
        <button class="icon-btn" title="Previous range (Alt+←)" i18n-title="@@timeline.panel.back.title" aria-label="Previous range" i18n-aria-label="@@timeline.panel.back.aria" [disabled]="!zoom.canBack()" (click)="zoom.back()">←</button>
        <button class="icon-btn" title="Next range (Alt+→)" i18n-title="@@timeline.panel.forward.title" aria-label="Next range" i18n-aria-label="@@timeline.panel.forward.aria" [disabled]="!zoom.canForward()" (click)="zoom.forward()">→</button>
        <button class="icon-btn" title="Zoom in" i18n-title="@@timeline.panel.zoomIn.title" aria-label="Zoom in" i18n-aria-label="@@timeline.panel.zoomIn.aria" (click)="zoom.zoomIn()"><imt-icon name="plus" [size]="13" /></button>
        <button class="icon-btn" title="Zoom out" i18n-title="@@timeline.panel.zoomOut.title" aria-label="Zoom out" i18n-aria-label="@@timeline.panel.zoomOut.aria" (click)="zoom.zoomOut()"><imt-icon name="minus" [size]="13" /></button>
        <button class="btn" title="Fit all (double-click)" i18n-title="@@timeline.panel.fit.title" [disabled]="!zoom.isZoomed()" (click)="zoom.fit()" i18n="@@timeline.panel.fit"><imt-icon name="zoomOut" [size]="13" /> Fit</button>
      </div>
      <button class="icon-btn" type="button" title="Timeline menu" i18n-title="@@timeline.panel.menu.title" aria-label="Timeline menu" i18n-aria-label="@@timeline.panel.menu.aria" aria-haspopup="menu" (click)="openMenu($event)"><imt-icon name="chevronDown" [size]="13" /></button>
    </div>
    @if (inst; as s) {
      <div class="config" role="group" aria-label="Timeline settings" i18n-aria-label="@@timeline.config.aria">
        <label class="field">
          <span class="faint" i18n="@@timeline.config.events">Events</span>
          <select class="select" aria-label="Events shown" i18n-aria-label="@@timeline.config.eventsShown" [value]="s.config().scope" (change)="s.setScope(asScope($event))">
            <option value="global" i18n="@@timeline.config.scope.global">Workspace filter</option>
            <option value="own" i18n="@@timeline.config.scope.own">Own query</option>
          </select>
        </label>
        @if (s.config().scope === 'own') {
          <input
            class="input mono ql"
            aria-label="Timeline query"
            i18n-aria-label="@@timeline.config.queryAria"
            placeholder="e.g. level >= error and MachineName = WEB01"
            i18n-placeholder="@@timeline.config.queryPlaceholder"
            maxlength="4000"
            [value]="s.config().ql"
            (keydown.enter)="s.setQl(valueOf($event))"
            (change)="s.setQl(valueOf($event))"
          />
        }
        <label class="field">
          <span class="faint" i18n="@@timeline.config.lanes">Lanes</span>
          <select class="select" aria-label="Lanes" i18n-aria-label="@@timeline.config.lanesAria" (change)="s.setLanes(asLanes($event))">
            @for (m of laneModes; track m.mode) { <option [value]="m.mode" [selected]="m.mode === s.config().lanes">{{ m.label }}</option> }
          </select>
        </label>
        @if (s.config().lanes === 'field') {
          <select class="select" aria-label="Lane field" i18n-aria-label="@@timeline.config.laneField" (change)="s.setLanes('field', valueOf($event))">
            <option value="" [selected]="!s.config().laneField" i18n="@@timeline.config.chooseField">Choose a field…</option>
            @for (f of store.fieldNames(); track f) { <option [value]="f" [selected]="f === s.config().laneField">{{ f }}</option> }
          </select>
        }
        <span class="spacer"></span>
        <imt-switch size="small" [checked]="s.synced()" (checkedChange)="s.setSync($event)" i18n="@@timeline.config.syncTime">Sync time</imt-switch>
      </div>
    }
    @if ((store.summary()?.markerKinds?.length ?? 0) > 0 || ann.kinds().length || ann.runKinds().length || ann.error()) {
      <div class="legend-row">
        <div class="legend" role="group" aria-label="Markers" i18n-aria-label="@@timeline.legend.markers" [class.filtering]="src.markers().length > 0">
          @for (k of store.summary()?.markerKinds ?? []; track k.kind) {
            <button
              type="button"
              class="lg"
              [attr.aria-pressed]="legendActive(k.kind)"
              [attr.aria-describedby]="legendDesc(k)"
              [title]="legendTitle(k)"
              (click)="toggleKind(k.kind)"
            ><span class="pill"><imt-marker-glyph [kind]="k.kind" [size]="16" />{{ k.label }}</span></button>
            @if (legendDesc(k); as id) { <span class="sr" [id]="id">{{ legendTitle(k) }}</span> }
          }
        </div>
        @if (ann.runKinds().length) {
          <div class="run-legend" role="group" aria-label="Runs" i18n-aria-label="@@timeline.legend.runsAria">
            <button
              type="button"
              class="lg runs-toggle"
              data-testid="runs-toggle"
              [attr.aria-pressed]="ann.runsShown()"
              [title]="ann.runsShown() ? hideRunsTitle : showRunsTitle"
              (click)="ann.toggleRuns()"
            ><span class="pill"><span class="run-swatch" aria-hidden="true"></span><span class="lg-title" i18n="@@timeline.legend.runs">Runs</span></span></button>
            @if (ann.runsShown()) {
              @for (k of ann.runKinds(); track k.kind) {
                <button
                  type="button"
                  class="lg"
                  [attr.aria-pressed]="ann.isRunKindShown(k.kind)"
                  [attr.data-run-kind]="k.kind"
                  [title]="runKindTitle(k)"
                  (click)="ann.toggleRunKind(k.kind)"
                ><span class="pill">{{ k.label }} <span class="faint mono">{{ k.count }}</span>@if (k.anomalies) { <span class="run-anoms" [attr.aria-label]="anomaliesAria(k.anomalies)">⚠ {{ k.anomalies }}</span> }</span></button>
              }
            }
            @if (!ann.kinds().length && !ann.error() && annInView().length) {
              <!-- Without plugin annotations the keyboard path to the runs lives here. -->
              <button
                type="button"
                class="lg ann-list"
                aria-haspopup="menu"
                title="Show the events of a run in view"
                i18n-title="@@timeline.legend.runEvidenceTitle"
                (click)="annMenu($event)"
              ><span class="pill" i18n="@@timeline.legend.evidence">Evidence…</span></button>
            }
          </div>
        }
        @if (ann.kinds().length || ann.error()) {
          <div class="ann-legend" role="group" aria-label="Annotations" i18n-aria-label="@@timeline.legend.annotationsAria">
            <span class="lg-title" i18n="@@timeline.legend.annotations">Annotations</span>
            @for (k of ann.kinds(); track k.kind) {
              <button
                type="button"
                class="lg"
                [attr.aria-pressed]="ann.isShown(k.kind)"
                [attr.data-kind]="k.kind"
                [title]="annKindTitle(k)"
                (click)="ann.toggle(k.kind)"
              ><span class="pill"><imt-severity-badge [severity]="k.severity" [showLabel]="false" [size]="12" />{{ k.label }}</span></button>
            }
            @if (annInView().length) {
              <!-- The keyboard path to every annotation, and the equivalent control (WCAG 2.5.8) for bands under markers. -->
              <button
                type="button"
                class="lg ann-list"
                aria-haspopup="menu"
                title="Show the evidence of an annotation in view"
                i18n-title="@@timeline.legend.evidenceTitle"
                (click)="annMenu($event)"
              ><span class="pill" i18n="@@timeline.legend.evidence">Evidence…</span></button>
            }
            @if (ann.error(); as e) {
              <span class="ann-err" role="alert" [title]="e" i18n="@@timeline.legend.unavailable">Annotations unavailable</span>
              <button type="button" class="btn" (click)="ann.retry()" i18n="@@timeline.legend.retry">Retry</button>
            }
          </div>
        }
      </div>
    }

    <div
      class="wrap"
      #wrap
      [attr.data-from]="tl()?.from"
      [attr.data-to]="tl()?.to"
      [class.panning]="panning()"
      (pointermove)="publishHover($event)"
      (pointerleave)="cursor.hover(null)"
      (pointerdown)="pointerDown($event)"
      (dblclick)="zoom.fit()"
      (wheel)="onWheel($event)"
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

        @for (l of lanes(); track l.lane.key) {
          <div class="row lane">
            <button class="label lane-label" [class.filtered]="l.filtered" (click)="src.toggleGroup(l.lane.key)" [title]="laneTitle(l)">
              <span class="mono key ellipsis">{{ l.lane.key }}</span>
              <span class="faint meta ellipsis">{{ l.meta }}</span>
            </button>
            <div class="plot track">
              <div class="baseline"></div>
              <div class="bar" [style.left.%]="l.left" [style.width.%]="l.width"></div>
              @if (runLayout().get(l.lane.key); as runs) {
                <imt-run-track [items]="runs" (hovered)="runHover.set($event)" (activated)="ann.showRun($event.ann)" />
              }
              @if (annLayout().lanes.get(l.lane.key); as items) {
                <imt-annotation-track [items]="items" (hovered)="annHover.set($event)" (activated)="ann.showEvidence($event.ann)" />
              }
              <svg class="density" [attr.viewBox]="densityViewBox()" preserveAspectRatio="none" aria-hidden="true"><path [attr.d]="l.densityPath" /></svg>
              @if (l.tickCount) {
                <svg class="ticks" viewBox="0 0 1000 6" preserveAspectRatio="none" role="img" [attr.aria-label]="tickAria(l.tickCount)">
                  <title>{{ tickTitle(l.tickCount) }}</title>
                  <path [attr.d]="l.ticksPath" />
                </svg>
              }
              @for (t of l.targets; track t.primary.cluster.kind + t.primary.cluster.bucket) {
                <!-- A 24px hit area (WCAG 2.5.8) around 16px glyphs; close markers share one (see layoutTargets). -->
                <button
                  type="button"
                  class="marker"
                  [style.left.%]="(t.left / plotWidth()) * 100"
                  [style.width.px]="t.width"
                  [attr.aria-label]="targetAria(t)"
                  (pointerdown)="$event.stopPropagation()"
                  (click)="activate($event, t)"
                  (pointerenter)="hover.set({ x: $event.clientX, y: $event.clientY, lane: l.lane.key, marker: t.primary })"
                  (pointerleave)="hover.set(null)"
                >
                  @for (g of t.members; track g.marker.cluster.kind + g.marker.cluster.bucket) {
                    <span
                      class="glyph"
                      [class.primary]="g.marker === t.primary"
                      [style.left.px]="g.offset"
                      [attr.data-member]="$index"
                      (pointerenter)="hover.set({ x: $event.clientX, y: $event.clientY, lane: l.lane.key, marker: g.marker })"
                    >
                      <imt-marker-glyph [kind]="g.marker.cluster.kind" [size]="16" />
                      @if (g.marker.showCount) { <span class="count mono">{{ g.marker.cluster.count }}</span> }
                    </span>
                  }
                </button>
              }
            </div>
          </div>
        } @empty {
          <div class="empty-state">
            @if (!store.summary()?.sources?.length) {
              <imt-icon name="files" [size]="28" />
              <div i18n="@@timeline.empty.open">Open log files or a folder to build the timeline.</div>
              <button class="btn primary" (click)="store.pickAndOpen(true)" i18n="@@timeline.empty.openFolder">Open folder…</button>
            } @else if (tl()) {
              @if (src.main) {
                <div i18n="@@timeline.empty.noMatchMain">No timestamped events match the current filters.</div>
              } @else {
                <div i18n="@@timeline.empty.noMatchOwn">No timestamped events match the query of this timeline.</div>
              }
            }
          </div>
        }
        @if ((tl()?.truncatedLanes ?? 0) > 0) {
          @if (src.main) {
            <div class="truncated faint" i18n="@@timeline.truncated.main">{{ tl()!.truncatedLanes //i18n(ph="count") }} more instances not shown — narrow the filter or change the group-by.</div>
          } @else {
            <div class="truncated faint" i18n="@@timeline.truncated.own">{{ tl()!.truncatedLanes //i18n(ph="count") }} more lanes not shown — narrow the filter or change the lanes.</div>
          }
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

    @if (src.error(); as err) {
      <div class="err-card" role="alert">
        <strong i18n="@@timeline.error.title">Could not load the timeline</strong>
        <span class="mono">{{ err }}</span>
        <button class="btn" type="button" (click)="src.retry()" i18n="@@timeline.error.retry">Retry</button>
      </div>
    }

    @if (hover(); as h) {
      <div class="popover tip" [style.left.px]="h.x + 14" [style.top.px]="h.y + 12" role="tooltip">
        <div class="tip-head">
          <imt-marker-glyph [kind]="h.marker.cluster.kind" [size]="18" />
          <strong>{{ h.marker.label }}</strong>
          <span class="faint" i18n="@@timeline.tip.count">{h.marker.cluster.count, plural, =1 {1 event} other {{{ h.marker.cluster.count }} events in this interval}}</span>
          <span class="spacer"></span>
          <span class="mono faint">{{ time(h.marker.cluster.ts) }}</span>
        </div>
        @if (describe(h.marker.cluster.kind); as d) { <div class="tip-desc">{{ d }}</div> }
        <div class="mono small">{{ h.lane }}</div>
        <div class="tip-msg">{{ h.marker.cluster.message }}</div>
        @if (h.marker.cluster.count > 1) {
          <div class="faint small" i18n="@@timeline.tip.jumpFirst">Click to jump to the first event</div>
        } @else {
          <div class="faint small" i18n="@@timeline.tip.jumpOne">Click to jump to the event</div>
        }
      </div>
    }

    @if (runHover(); as h) {
      <div class="popover tip" data-testid="run-tip" [style.left.px]="h.x + 14" [style.top.px]="h.y + 12" role="tooltip">
        <div class="tip-head">
          <strong>{{ h.vm.run.spanLabel }}</strong>
          <span class="badge run-status" [attr.data-status]="h.vm.status">{{ statusLabel(h.vm.status) }}</span>
          <span class="spacer"></span>
          <span class="mono faint">{{ runDur(h.vm) }}</span>
        </div>
        <div class="mono small">{{ h.vm.ann.instance }}@if (h.vm.run.group; as g) { · {{ g.field }} = {{ g.value }} }</div>
        @if (h.vm.run.start; as s) {
          <div class="small"><span class="faint" i18n="@@timeline.runTip.started">Started</span> <span class="mono">{{ time(s.ts) }}</span></div>
          <div class="tip-msg">{{ s.message }}</div>
        }
        @if (h.vm.run.stop; as s) {
          <div class="small"><span class="faint" i18n="@@timeline.runTip.stopped">Stopped</span> <span class="mono">{{ time(s.ts) }}</span></div>
          <div class="tip-msg">{{ s.message }}</div>
        } @else if (h.vm.status === 'open') {
          <div class="small faint" i18n="@@timeline.runTip.open">No stop yet: drawn to the lane's last event ({{ time(h.vm.ann.to) //i18n(ph="time") }}).</div>
        } @else if (h.vm.status === 'noStop') {
          <div class="small faint" i18n="@@timeline.runTip.lastEvent">Last event before the next start: {{ time(h.vm.ann.to) //i18n(ph="time") }}</div>
        }
        @if (anomalyText(h.vm); as w) { <div class="small warn-text">⚠ {{ w }}</div> }
        <div class="faint small" i18n="@@timeline.runTip.click">Click to show this run's events</div>
      </div>
    }

    @if (annHover(); as h) {
      <div class="popover tip" data-testid="annotation-tip" [style.left.px]="h.x + 14" [style.top.px]="h.y + 12" role="tooltip">
        <div class="tip-head">
          <imt-severity-badge [severity]="h.vm.ann.severity" />
          <strong>{{ h.vm.ann.label }}</strong>
        </div>
        <div class="mono small">{{ annTime(h.vm.ann.from, h.vm.ann.to) }}</div>
        <div class="faint small" i18n="@@timeline.annTip.from">{{ annKindLabel(h.vm.ann.kind) //i18n(ph="kind") }} · from {{ h.vm.ann.owner //i18n(ph="owner") }}</div>
        @if (h.vm.ann.instance; as inst) { <div class="mono small">{{ inst }}</div> }
        @else if (h.vm.ann.on; as on) { <div class="mono small">{{ on.field }} = {{ on.value }}</div> }
        <div class="faint small" i18n="@@timeline.annTip.evidence">Click to show the evidence</div>
      </div>
    }
  `,
  styles: `
    :host { flex: 1; min-height: 0; min-width: 0; display: flex; flex-direction: column; background: var(--bg-1); overflow: hidden; position: relative; }
    .err-card { position: absolute; top: 60px; left: 50%; transform: translateX(-50%); z-index: 3; display: flex; flex-direction: column; align-items: center; gap: 8px; max-width: 80%; padding: 14px 18px; border: 1px solid var(--border-3); border-radius: 8px; background: var(--bg-3); color: var(--danger-text); text-align: center; }
    .head { height: 32px; flex: none; display: flex; align-items: center; gap: 12px; padding: 0 14px; }
    .hint { font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .section-title { white-space: nowrap; }
    .title-input { width: 200px; height: 24px; padding: 0 6px; }
    .unlinked { flex: none; font-size: 11px; padding: 0 6px; border-radius: 8px; border: 1px dashed var(--border-3); color: var(--text-2); }
    .config { flex: none; display: flex; flex-wrap: wrap; align-items: center; gap: 6px 12px; padding: 0 14px 6px; font-size: 12px; }
    .config .field { display: inline-flex; align-items: center; gap: 6px; }
    .config .select { height: 24px; }
    .config .ql { flex: 1 1 260px; min-width: 160px; height: 24px; padding: 0 6px; }
    .spacer { flex: 1; }
    .legend-row { flex: none; display: flex; flex-wrap: wrap; align-items: center; gap: 4px 16px; padding: 0 14px 6px; }
    .legend, .ann-legend, .run-legend { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 6px; font-size: 12px; color: var(--muted); min-width: 0; }
    /* The button is a transparent 24px hit area (WCAG 2.5.8) around the visible pill; the negative margin keeps the row height. */
    .lg { display: inline-flex; align-items: center; min-height: 24px; margin: -2px 0; padding: 0; border: 0; background: transparent; color: inherit; font: inherit; cursor: pointer; }
    .lg:focus-visible { outline: none; }
    .pill { display: inline-flex; align-items: center; gap: 5px; white-space: nowrap; border: 1px solid transparent; padding: 1px 6px; border-radius: 10px; }
    .lg:focus-visible .pill { outline: 2px solid var(--accent); outline-offset: 1px; }
    .lg:hover .pill { background: var(--bg-hover); }
    .lg[aria-pressed='true'] .pill { border-color: var(--accent); background: var(--accent-soft); }
    .tip-desc { color: var(--text); }
    .lg-title { font-weight: 600; color: var(--text-2); white-space: nowrap; }
    .legend.filtering .lg[aria-pressed='false'], .ann-legend .lg[aria-pressed='false'], .run-legend .lg[aria-pressed='false'] { opacity: 0.5; text-decoration: line-through; }
    .run-legend .lg[aria-pressed='true'] .pill { border-color: transparent; background: transparent; }
    .ann-legend .lg[aria-pressed='true'] .pill { border-color: transparent; background: transparent; }
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
    .marker { position: absolute; top: 0; height: 32px; min-width: 24px; border: 0; padding: 0; background: transparent; cursor: pointer; z-index: 1; }
    /* The focus ring goes on the visible glyph, not the transparent hit area. */
    .marker:focus-visible { outline: none; }
    .glyph { position: absolute; top: 0; height: 32px; width: 16px; margin-left: -8px; display: flex; align-items: center; justify-content: center; }
    .marker:hover .glyph.primary imt-marker-glyph, .glyph:hover imt-marker-glyph { transform: scale(1.35); }
    .marker:focus-visible .glyph.primary { outline: 2px solid var(--accent); outline-offset: 1px; }
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
    .run-swatch { width: 14px; height: 10px; border: 1.5px solid color-mix(in srgb, var(--accent) 75%, transparent); border-radius: 3px; background: color-mix(in srgb, var(--accent) 16%, transparent); }
    .run-legend .lg-title { font-weight: 600; }
    .run-anoms { color: var(--warn-text); }
    .warn-text { color: var(--warn-text); }
    .run-status { font-size: 11px; }
  `,
})
export class TimelinePanel {
  protected readonly store = inject(WorkspaceStore);
  /** What this panel shows: the workspace's timeline, or an additional timeline's own state. */
  protected readonly src = inject(TIMELINE_SOURCE);
  /** The additional timeline's store (settings bar, rename, sync); null for the main timeline. */
  protected readonly inst = this.src.main ? null : inject(TimelineInstanceStore);
  private readonly views = inject(TimelineViews);
  private readonly dock = inject(DockService);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  protected readonly laneModes = LANE_MODES;
  protected readonly renaming = signal(false);
  protected readonly panning = signal(false);

  /** The zoom controller in effect (the workspace's while linked). */
  protected get zoom(): RangeZoom {
    return this.src.zoom();
  }

  /** The time cursor in effect (shared with every time-based view while linked). */
  protected get cursor(): TimeCursor {
    return this.src.cursor();
  }
  /** Lane order from the unzoomed view, so zooming never reshuffles lanes. */
  private readonly laneOrder = new Map<string, number>();
  private spaceDown = false;
  private readonly wrap = viewChild.required<ElementRef<HTMLElement>>('wrap');
  private readonly axis = viewChild.required<ElementRef<HTMLElement>>('axis');

  protected readonly plotLeft = signal(LABEL_W);
  protected readonly plotWidth = signal(800);
  protected readonly brush = signal<{ x0: number; x1: number } | null>(null);
  protected readonly hover = signal<Hover | null>(null);
  private readonly targets = inject(AnalysisTargets);
  private readonly commands = inject(COMMAND_CONTEXT);
  private readonly menu = inject(ContextMenuService);

  protected readonly tl = this.src.timeline;
  protected readonly ann = inject(TimelineAnnotations);
  protected readonly annHover = signal<AnnotationHover | null>(null);
  protected readonly runHover = signal<RunHover | null>(null);
  protected readonly hideRunsTitle = $localize`:@@timeline.legend.hideRuns:Hide the runs between start and stop markers`;
  protected readonly showRunsTitle = $localize`:@@timeline.legend.showRuns:Show the runs between start and stop markers`;
  protected readonly statusLabel = runStatusLabel;

  /** Run bands per lane (behind the markers). */
  protected readonly runLayout = computed(() => {
    const view = this.range();
    if (!view) return new Map<string, RunBandVm[]>();
    return layoutRuns(this.ann.runs(), {
      laneKeys: this.lanes().map((l) => l.lane.key),
      view,
      hidden: this.ann.hiddenRuns(),
      shown: this.ann.runsShown(),
    });
  });

  protected runDur(v: RunBandVm): string {
    return runDuration(v.run);
  }

  protected anomalyText(v: RunBandVm): string | null {
    return runAnomalyText(v.run);
  }

  protected runKindTitle(k: { kind: string; label: string; count: number }): string {
    const { label, count } = k;
    return this.ann.isRunKindShown(k.kind)
      ? $localize`:@@timeline.legend.hideRunKind:Hide ${label}:label: runs (${count}:count:)`
      : $localize`:@@timeline.legend.showRunKind:Show ${label}:label: runs (${count}:count:)`;
  }

  protected anomaliesAria(n: number): string {
    return $localize`:@@timeline.legend.runAnomalies:${n}:count: anomalies`;
  }

  /** Annotation bands and glyphs per lane, plus the full-height ones. */
  protected readonly annLayout = computed(() => {
    const view = this.range();
    if (!view) return { lanes: new Map(), global: [] };
    return layoutAnnotations(this.ann.plugin(), {
      laneKeys: this.lanes().map((l) => l.lane.key),
      view,
      groupBy: [...this.src.groupBy()],
      hidden: this.ann.hidden(),
    });
  });

  /** The annotations and runs in view, once each (one may be drawn on several lanes), in time order. */
  protected readonly annInView = computed(() => {
    const { lanes, global } = this.annLayout();
    const seen = new Set<AnnotationDto>();
    for (const v of [...global, ...[...lanes.values()].flat()] as AnnotationVm[]) seen.add(v.ann);
    for (const v of [...this.runLayout().values()].flat()) seen.add(v.ann);
    return [...seen].sort((a, b) => a.from - b.from || a.label.localeCompare(b.label));
  });

  /** Lists the annotations in view; choosing one shows its evidence like clicking its band or glyph. */
  protected annMenu(e: MouseEvent): void {
    const anns = this.annInView();
    const at = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const items: MenuEntry[] = anns.slice(0, MAX_ANN_MENU).map((a) =>
      a.run
        ? { label: `${a.label} · ${a.instance ?? ''} · ${this.time(a.from)}`, run: () => this.ann.showRun(a) }
        : { label: `${a.label} · ${this.time(a.from)}`, run: () => this.ann.showEvidence(a) },
    );
    const more = anns.length - items.length;
    if (more > 0) items.push({ label: $localize`:@@timeline.legend.evidenceMore:${more}:count: more — zoom in to list them`, run: () => undefined, disabled: true });
    this.menu.open(at.left, at.bottom, items, e.currentTarget as HTMLElement);
  }

  protected annTime(from: number, to: number): string {
    const tz = this.store.timezone();
    return from === to ? formatDateTime(from, tz, true) : `${formatDateTime(from, tz, true)} → ${formatDateTime(to, tz, true)} (${formatDuration(to - from)})`;
  }

  protected hint(n: number): string {
    const bucket = this.bucketLabel();
    return this.src.main
      ? plural(n, {
          one: $localize`:@@timeline.hint.instances.one:${n}:count: instance · bucket ${bucket}:bucket: · drag to zoom · ⌘/Ctrl+wheel zoom · Shift+wheel or Space+drag pan`,
          other: $localize`:@@timeline.hint.instances.other:${n}:count: instances · bucket ${bucket}:bucket: · drag to zoom · ⌘/Ctrl+wheel zoom · Shift+wheel or Space+drag pan`,
        })
      : plural(n, {
          one: $localize`:@@timeline.hint.lanes.one:${n}:count: lane · bucket ${bucket}:bucket: · drag to zoom · ⌘/Ctrl+wheel zoom · Shift+wheel or Space+drag pan`,
          other: $localize`:@@timeline.hint.lanes.other:${n}:count: lanes · bucket ${bucket}:bucket: · drag to zoom · ⌘/Ctrl+wheel zoom · Shift+wheel or Space+drag pan`,
        });
  }

  protected annKindTitle(k: { kind: string; label: string; owner: string; count: number }): string {
    const { label, owner, count } = k;
    return this.ann.isShown(k.kind)
      ? $localize`:@@timeline.legend.hideAnnotation:Hide ${label}:label: (${owner}:owner:, ${count}:count:)`
      : $localize`:@@timeline.legend.showAnnotation:Show ${label}:label: (${owner}:owner:, ${count}:count:)`;
  }

  protected tickAria(n: number): string {
    return $localize`:@@timeline.ticks.aria:${n}:count: warnings (zoom in for detail)`;
  }

  protected tickTitle(n: number): string {
    return $localize`:@@timeline.ticks.title:${n}:count: warnings — zoom in for detail`;
  }

  /** Names a hit area by its markers, the one it activates first. */
  protected targetAria(t: MarkerTarget): string {
    const rest = t.members.filter((g) => g.marker !== t.primary).map((g) => this.markerAria(g.marker));
    return [this.markerAria(t.primary), ...rest].join('; ');
  }

  /** Jumps to the clicked glyph's event, or to the target's primary from the keyboard or the space around glyphs. */
  protected activate(event: Event, t: MarkerTarget): void {
    const member = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-member]')?.dataset['member'] : undefined;
    const picked = member === undefined ? undefined : t.members[Number(member)]?.marker;
    this.store.jumpTo((picked ?? t.primary).cluster.id);
  }

  protected markerAria(m: LaidMarker): string {
    const label = m.label;
    const at = this.time(m.cluster.ts);
    const count = m.cluster.count;
    return count > 1
      ? $localize`:@@timeline.marker.ariaMany:${label}:label: at ${at}:time:, ${count}:count: events`
      : $localize`:@@timeline.marker.ariaOne:${label}:label: at ${at}:time:`;
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
    const m = this.src.markers();
    return m.length === 1 && m[0] === kind;
  }

  /** Clicking a legend item shows only that marker's events; clicking again clears it. */
  protected toggleKind(kind: string): void {
    this.src.showOnlyMarker(this.legendActive(kind) ? null : kind);
  }

  /** The visible time range (null before the first timeline arrives). */
  protected readonly range = computed<Range | null>(() => {
    const t = this.tl();
    return t ? { from: t.from, to: t.to } : null;
  });

  /** X (px in the plot) of the shared time cursor, from any time-based view; null when outside. */
  protected readonly hoverX = computed(() => {
    const frac = timeToFrac(this.src.cursor().hovered(), this.range());
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
        paths[li]?.push(`M${x + 0.1} ${y.toFixed(2)}h0.8v${h.toFixed(2)}h-0.8z`);
      });
    });
    return LEVELS.map((level, i) => ({ level, d: paths[i]?.join('') ?? '', color: HIST_VARS[level] })).filter((p) => p.d);
  });

  protected readonly lanes = computed<LaneVm[]>(() => {
    const t = this.tl();
    if (!t) return [];
    const span = this.span();
    const kinds = this.store.markerKindMap();
    const groups = new Set(this.src.groups());
    const tz = this.store.timezone();
    const priority = [...kinds.keys()];
    const ordered = stableLaneOrder(t.lanes, this.laneOrder, !this.src.hasRange());
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
        targets: layout.targets,
        ticksPath: layout.ticks.map((x) => `M${(x * 10).toFixed(1)} 0v6`).join(''),
        tickCount: layout.tickCount,
        filtered: groups.has(lane.key),
        meta: `${laneCount(lane)} · ${formatTime(lane.first, tz).slice(0, 8)} → ${formatTime(lane.last, tz).slice(0, 8)}`,
      };
    });
  });

  protected readonly fatalLines = computed(() =>
    this.lanes().flatMap((l) => l.markers.filter((m) => m.cluster.kind === 'fatal').map((m) => m.left)),
  );

  protected readonly hoverTime = computed(() => {
    const ts = this.src.cursor().hovered();
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
    // An additional timeline's tab shows its title (restored layouts and renames included).
    const inst = this.inst;
    if (inst) effect(() => this.dock.setTitle(inst.id, inst.title()));
  }

  protected laneTitle(l: LaneVm): string {
    const key = l.lane.key;
    if (this.src.main) {
      return l.filtered
        ? $localize`:@@timeline.laneTitle.removeInstance:${key}:key: — click to remove the instance filter`
        : $localize`:@@timeline.laneTitle.onlyInstance:${key}:key: — click to show only this instance`;
    }
    return l.filtered
      ? $localize`:@@timeline.laneTitle.removeLane:${key}:key: — click to remove the lane filter`
      : $localize`:@@timeline.laneTitle.onlyLane:${key}:key: — click to show only this lane`;
  }

  protected valueOf(e: Event): string {
    return (e.target as HTMLInputElement | HTMLSelectElement).value;
  }

  protected asScope(e: Event): TimelineScope {
    return this.valueOf(e) === 'own' ? 'own' : 'global';
  }

  protected asLanes(e: Event): LaneMode {
    const v = this.valueOf(e);
    return LANE_MODES.find((m) => m.mode === v)?.mode ?? 'instance';
  }

  protected startRename(): void {
    if (this.inst) this.renaming.set(true);
  }

  protected renameKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') this.commitRename(e);
    else if (e.key === 'Escape') {
      e.preventDefault();
      this.renaming.set(false);
    }
  }

  protected commitRename(e: Event): void {
    if (!this.renaming()) return;
    this.renaming.set(false);
    this.inst?.rename(this.valueOf(e));
  }

  /** The panel's own menu: open another timeline; for an additional one also rename, link or unlink, close. */
  protected openMenu(e: MouseEvent): void {
    const anchor = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const inst = this.inst;
    this.menu.open(anchor.left, anchor.bottom, [
      { label: $localize`:@@timeline.menu.new:New timeline`, run: () => void this.views.open() },
      ...(inst
        ? [
            { label: $localize`:@@timeline.menu.rename:Rename…`, run: () => this.renaming.set(true) },
            { label: inst.synced() ? $localize`:@@timeline.menu.unlink:Unlink time from the main timeline` : $localize`:@@timeline.menu.sync:Sync time with the main timeline`, run: () => inst.setSync(!inst.synced()) },
            { label: $localize`:menu item, closes the timeline panel@@timeline.menu.close:Close`, run: () => this.dock.close(inst.id) },
          ]
        : []),
    ]);
  }

  /** Publishes the time under the pointer to the cursor in effect (null outside the plot). */
  protected publishHover(e: PointerEvent): void {
    const view = this.range();
    if (!view) return;
    const x = this.localX(e);
    const w = this.plotWidth();
    this.cursor.hover(x < 0 || x > w ? null : xToTime(x, w, view));
  }

  private measure(): void {
    const wrap = this.wrap().nativeElement.getBoundingClientRect();
    const axis = this.axis().nativeElement.getBoundingClientRect();
    if (axis.width <= 0) return;
    this.plotLeft.set(axis.left - wrap.left);
    this.plotWidth.set(axis.width);
    this.src.setBuckets(Math.floor(axis.width / 2));
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
        label: $localize`:@@timeline.menu.analyseMoment:Analyse this moment`,
        run: () => {
          this.targets.setMoment(momentAt(ts, this.cursor.selection(), WIDTHS[1]?.ms ?? 300_000));
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
    // History keys act once: in an additional timeline while focus is inside it, else in the main timeline.
    const inInstance = !!target?.closest?.('imt-timeline-panel.instance');
    if (this.src.main ? inInstance : !(target && this.host.nativeElement.contains(target))) return;
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
