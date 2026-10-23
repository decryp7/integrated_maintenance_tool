import { formatCount } from '../shared/format/units';
import { ChangeDetectionStrategy, Component, computed, input, output, signal } from '@angular/core';
import { AnnotationDto } from '../core/api.types';
import { Icon } from '../shared/icon';
import { MarkerGlyph } from '../markers/marker-glyph';
import { SourceSwatch } from '../sources/source-swatch';
import { AnnotationVm } from './annotation-layout';
import { AnnotationHover, AnnotationTrack, SpanMenu } from './annotation-track';
import { LaidMarker } from './marker-layout';
import { RunBandVm, runRowCount } from './run-layout';
import { RunHover, RunTrack } from './run-track';
import { LaneVm, TargetVm } from './timeline-lane-vm';

/** The marker under the pointer, for the tooltip. */
export interface MarkerHover {
  x: number;
  y: number;
  lane: string;
  marker: LaidMarker;
}

/** A marker's context menu: where, and which marker. */
export interface MarkerMenu {
  x: number;
  y: number;
  marker: LaidMarker;
}

/** What the lanes show besides themselves when there are none. */
export type LanesEmpty = 'noSources' | 'noMatchMain' | 'noMatchOwn' | null;

/**
 * The lane rows of a timeline: label, extent bar, density, run and annotation bands, and marker hit areas. It draws
 * the view model it is given and reports what the user did; it knows nothing about stores.
 */
@Component({
  selector: 'debrief-timeline-lanes',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, MarkerGlyph, SourceSwatch, AnnotationTrack, RunTrack],
  template: `
    @for (l of lanes(); track l.lane.key; let odd = $odd) {
      @let runs = runLayout().get(l.lane.key);
      <!-- A lane: the band (volume, annotations, markers), then its runs on stacked sub-tracks. -->
      <div class="row lane" [class.odd]="odd" [class.colored]="!!l.color" [class.sparse]="l.sparse" [style.--run-rows]="rowsOf(runs)" [style.--lane-c]="l.color">
        <button class="label lane-label" [class.filtered]="l.filtered" (click)="toggleGroup.emit(l.lane.key)" [title]="l.title + (l.meta ? '\n' + l.meta : '')">
          <span class="label-body">
            <span class="key-row">
              @for (m of l.marks; track m.swatch.slot) {
                <debrief-source-swatch [swatch]="m.swatch" [label]="m.label" [size]="8" />
              }
              @if (l.moreSources > 0) {
                <span class="faint more-src">+{{ l.moreSources }}</span>
              }
              @let parts = keyParts(l.lane.key);
              <span class="key"><span class="key-name ellipsis">{{ parts[0] }}</span>@if (parts[1]) {<span class="key-tail">{{ parts[1] }}</span>}</span>
            </span>
            <span class="meta ellipsis">{{ l.meta }}</span>
          </span>
        </button>
        <div class="plot track">
          <div class="band">
            <div class="bar" [style.left.%]="l.left" [style.width.%]="l.width"></div>
            @if (annLanes().get(l.lane.key); as items) {
              <debrief-annotation-track [items]="items" (hovered)="annHover.emit($event)" (activated)="showEvidence.emit($event.ann)" (menuRequested)="spanMenu.emit($event)" />
            }
            <!-- The theme's lane fill decides which of these shows (data-lane-fill). -->
            <svg class="density" [attr.viewBox]="densityViewBox()" preserveAspectRatio="none" aria-hidden="true">
              <path class="area" [attr.d]="l.densityArea" />
              <path class="edge" [attr.d]="l.densityEdge" />
              <path class="bars" [attr.d]="l.densityPath" />
            </svg>
            @if (l.sparse) {
              <svg class="sparse-ticks" [attr.viewBox]="densityViewBox()" preserveAspectRatio="none" aria-hidden="true"><path [attr.d]="l.sparsePath" /></svg>
            }
            @if (l.tickCount) {
              <svg class="ticks" viewBox="0 0 1000 6" preserveAspectRatio="none" role="img" [attr.aria-label]="tickAria(l.tickCount)">
                <title>{{ tickTitle(l.tickCount) }}</title>
                <path [attr.d]="l.ticksPath" />
              </svg>
            }
            @for (t of l.targets; track t.primary.cluster.kind + t.primary.cluster.bucket; let j = $index) {
              <!-- A 24px hit area (WCAG 2.5.8) around 14px glyphs; close markers share one (see layoutTargets). One tab stop per lane: arrows move between its markers. -->
              <button
                type="button"
                class="marker"
                [attr.tabindex]="j === tabStops().get(l.lane.key) ? 0 : -1"
                (keydown)="markerKey($event, l, j)"
                (focus)="setStop(l.lane.key, j)"
                [style.left.%]="t.leftPct"
                [style.width.px]="t.width"
                [attr.aria-label]="t.aria"
                (pointerdown)="$event.stopPropagation()"
                (click)="activate($event, t)"
                (contextmenu)="menu($event, t)"
                (pointerenter)="markerHover.emit({ x: $event.clientX, y: $event.clientY, lane: l.lane.key, marker: t.primary })"
                (pointerleave)="markerHover.emit(null)"
              >
                @for (g of t.members; track g.marker.cluster.kind + g.marker.cluster.bucket) {
                  <span
                    class="glyph"
                    [class.primary]="g.marker === t.primary"
                    [class.selected]="g.marker.cluster.id === selectedId()"
                    [attr.data-kind]="g.marker.cluster.kind"
                    [style.left.px]="g.offset"
                    [attr.data-member]="$index"
                    (pointerenter)="markerHover.emit({ x: $event.clientX, y: $event.clientY, lane: l.lane.key, marker: g.marker })"
                  >
                    <debrief-marker-glyph [kind]="g.marker.cluster.kind" [size]="14" />
                    @if (g.marker.showCount) { <span class="count mono" aria-hidden="true" [attr.data-count]="fmt(g.marker.cluster.count)"></span> }
                  </span>
                }
              </button>
            }
          </div>
          @if (runs) {
            <debrief-run-track [items]="runs" [laneColor]="l.color" (hovered)="runHover.emit($event)" (activated)="showRun.emit($event.ann)" (menuRequested)="spanMenu.emit($event)" />
          }
        </div>
      </div>
    } @empty {
      <div class="empty-state">
        @switch (empty()) {
          @case ('noSources') {
            <debrief-icon name="files" [size]="28" />
            <div i18n="@@timeline.empty.open">Open log files or a folder to build the timeline.</div>
            <button class="btn primary" (click)="openFolder.emit()" i18n="@@timeline.empty.openFolder">Open folder…</button>
          }
          @case ('noMatchMain') {
            <div i18n="@@timeline.empty.noMatchMain">No timestamped events match the current filters.</div>
          }
          @case ('noMatchOwn') {
            <div i18n="@@timeline.empty.noMatchOwn">No timestamped events match the query of this timeline.</div>
          }
        }
      </div>
    }
    @if (truncated() > 0) {
      @if (main()) {
        <div class="truncated faint" i18n="@@timeline.truncated.main">{{ truncated() //i18n(ph="count") }} more instances not shown — narrow the filter or change the group-by.</div>
      } @else {
        <div class="truncated faint" i18n="@@timeline.truncated.own">{{ truncated() //i18n(ph="count") }} more lanes not shown — narrow the filter or change the lanes.</div>
      }
    }
  `,
  styles: `
    :host { display: block; }
    .row { display: flex; align-items: stretch; }
    .label { width: var(--tl-label-w); flex: none; }
    .plot { flex: 1; min-width: 0; position: relative; margin-right: var(--s-5); }
    /* A lane is its band plus one sub-track per row of runs (--run-rows, set per lane). */
    .lane { --run-rows: 0; height: calc(var(--lane-h) + min(var(--run-rows), 1) * 3px + var(--run-rows) * var(--run-row-h)); transition: background-color var(--dur-fast) var(--ease); }
    .lane:hover { background: var(--bg-hover); }
    .lane-label { position: relative; display: block; padding: 0 var(--s-2) 0 var(--s-4); border: 0; background: transparent; cursor: pointer; text-align: left; min-width: 0; color: inherit; }
    /* The source stripe: the lane's source colour (none when source colours are off). */
    .lane-label::before { content: ''; position: absolute; left: 4px; top: 5px; height: calc(var(--lane-h) - 10px); width: 3px; border-radius: 2px; background: var(--lane-c, transparent); }
    .label-body { height: var(--lane-h); display: flex; flex-direction: column; justify-content: center; gap: 1px; min-width: 0; }
    .lane-label.filtered { background: var(--bg-sel); }
    .lane-label.filtered .key { color: var(--accent-text); }
    /* "Name · host · pid": the name gives way with an ellipsis, the host and pid (what tells instances apart) stay readable. */
    .key { display: flex; min-width: 0; max-width: 100%; font-size: var(--fs-sm); font-weight: 600; color: var(--text); }
    .key-name { flex: 0 1 auto; min-width: 3ch; }
    .key-tail { flex: none; white-space: pre; }
    .key-row { display: flex; align-items: center; gap: 4px; max-width: 100%; min-width: 0; }
    .more-src { font-size: var(--fs-2xs); }
    .meta { font-size: var(--fs-2xs); color: var(--muted); max-width: 100%; }
    .track { height: 100%; }
    .band { position: absolute; left: 0; right: 0; top: 0; height: var(--lane-h); }
    /* The lane's extent: a hairline under its data (flat), or a rounded track (cards). */
    .bar { position: absolute; bottom: 4px; height: 1px; min-width: 2px; background: var(--lane-border); }
    :host-context([data-surface='cards']) .bar { top: 3px; bottom: 3px; height: auto; border-radius: var(--r-2); background: var(--lane-bar); }
    :host-context([data-surface='flat']) .lane.odd { background: var(--lane-bar); }
    :host-context([data-surface='flat']) .lane.odd:hover, :host-context([data-surface='flat']) .lane:hover { background: var(--bg-hover); }
    .density, .sparse-ticks { position: absolute; left: 0; top: 6px; width: 100%; height: calc(var(--lane-h) - 11px); pointer-events: none; overflow: visible; }
    .density .area { fill: var(--density); }
    .density .edge { fill: none; stroke: none; }
    .density .bars { display: none; fill: var(--muted); }
    .sparse-ticks path { stroke: var(--muted); stroke-width: 1; vector-effect: non-scaling-stroke; }
    /* laneFill: bySource. The volume in the lane's source colour, with its top edge. */
    :host-context([data-lane-fill='bySource']) .lane.colored .density .area { fill: color-mix(in srgb, var(--lane-c) 26%, transparent); }
    :host-context([data-lane-fill='bySource']) .lane.colored .density .edge { stroke: var(--lane-c); stroke-width: 1.25; vector-effect: non-scaling-stroke; }
    :host-context([data-lane-fill='bySource']) .lane.colored .sparse-ticks path { stroke: var(--lane-c); }
    /* laneFill: barcode. One bar per bucket, scaled within the lane. */
    :host-context([data-lane-fill='barcode']) .density .area { display: none; }
    :host-context([data-lane-fill='barcode']) .density .bars { display: inline; }
    .ticks { position: absolute; left: 0; bottom: 1px; width: 100%; height: 6px; }
    .ticks path { stroke: var(--mk-warning); stroke-width: 1.5; vector-effect: non-scaling-stroke; }
    .marker { position: absolute; top: 0; height: var(--lane-h); min-width: 24px; border: 0; padding: 0; background: transparent; cursor: pointer; z-index: 1; }
    /* The focus ring goes on the visible glyph, not the transparent hit area. */
    .marker:focus-visible { outline: none; }
    .glyph { position: absolute; top: 0; height: var(--lane-h); width: 14px; margin-left: -7px; display: flex; align-items: center; justify-content: center; }
    .glyph debrief-marker-glyph { transition: transform var(--dur-fast) var(--ease); }
    .marker:hover .glyph.primary debrief-marker-glyph, .glyph:hover debrief-marker-glyph { transform: scale(1.3); }
    .marker:focus-visible .glyph.primary debrief-marker-glyph, .glyph.selected debrief-marker-glyph { outline: 2px solid var(--focus-ring); outline-offset: 2px; border-radius: 50%; }
    /* crashBand: a soft halo around crashes. */
    .glyph[data-kind='fatal']::before { content: ''; display: none; position: absolute; left: 50%; top: 50%; width: 22px; height: 22px; margin: -11px 0 0 -11px; border-radius: 50%; border: 2px solid color-mix(in srgb, var(--mk-fatal) 35%, transparent); pointer-events: none; }
    :host-context([data-crash-band='on']) .glyph[data-kind='fatal']::before { display: block; }
    .count { position: absolute; left: 12px; top: calc(var(--lane-h) / 2 - 14px); font-size: var(--fs-2xs); font-weight: 600; line-height: 12px; padding: 0 3px; border-radius: 3px; color: var(--text-2); background: var(--bg-1); box-shadow: 0 0 0 1px var(--border-2); pointer-events: none; }
    /* Drawn as generated content: the count is in the marker's accessible name already, and visible text that differs from it would fail WCAG 2.5.3. */
    .count::before { content: attr(data-count); }
    :host-context([data-surface='cards']) .count { border-radius: 999px; background: var(--bg-3); box-shadow: 0 0 0 1px var(--border-3); }
    .truncated { padding: 6px 14px; font-size: var(--fs-sm); }
  `,
})
export class TimelineLanes {
  protected readonly fmt = formatCount;
  readonly lanes = input.required<readonly LaneVm[]>();
  readonly runLayout = input.required<ReadonlyMap<string, RunBandVm[]>>();
  readonly annLanes = input.required<ReadonlyMap<string, AnnotationVm[]>>();
  readonly densityViewBox = input.required<string>();
  /** The selected event: its marker gets a ring. */
  readonly selectedId = input<number | null>(null);
  readonly main = input.required<boolean>();
  /** What to say when there are no lanes. */
  readonly empty = input<LanesEmpty>(null);
  /** Lanes the engine left out. */
  readonly truncated = input(0);

  readonly toggleGroup = output<string>();
  /** A marker was activated: the id of the event to jump to. */
  readonly jump = output<number>();
  readonly markerHover = output<MarkerHover | null>();
  readonly runHover = output<RunHover | null>();
  readonly annHover = output<AnnotationHover | null>();
  readonly showRun = output<AnnotationDto>();
  readonly showEvidence = output<AnnotationDto>();
  readonly openFolder = output<void>();
  /** A marker's context menu was asked for (right-click, the context-menu key). */
  readonly markerMenu = output<MarkerMenu>();
  /** A run's or annotation's context menu was asked for. */
  readonly spanMenu = output<SpanMenu>();

  /** The marker of each lane chosen with the arrow keys (by index; clamped when the lane has fewer now). */
  private readonly chosen = signal<ReadonlyMap<string, number>>(new Map());
  /** The one marker per lane in the tab order. */
  protected readonly tabStops = computed(() => {
    const chosen = this.chosen();
    return new Map(this.lanes().map((l) => [l.lane.key, Math.min(chosen.get(l.lane.key) ?? 0, Math.max(0, l.targets.length - 1))]));
  });

  protected setStop(lane: string, index: number): void {
    if (this.chosen().get(lane) === index) return;
    this.chosen.update((m) => new Map(m).set(lane, index));
  }

  /** Left/Right (and Home/End) move between the lane's markers; focus follows. */
  protected markerKey(e: KeyboardEvent, lane: LaneVm, index: number): void {
    // Alt+arrows go through the zoom history (TimelineInteraction).
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    const n = lane.targets.length;
    let next: number;
    switch (e.key) {
      case 'ArrowRight':
        next = Math.min(n - 1, index + 1);
        break;
      case 'ArrowLeft':
        next = Math.max(0, index - 1);
        break;
      case 'Home':
        next = 0;
        break;
      case 'End':
        next = n - 1;
        break;
      default:
        return;
    }
    e.preventDefault();
    if (next === index) return;
    this.setStop(lane.lane.key, next);
    const buttons = (e.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLElement>(':scope > .marker');
    buttons?.[next]?.focus();
  }

  /** Sub-tracks of a lane's runs (0 without runs). */
  protected rowsOf(runs: readonly RunBandVm[] | undefined): number {
    return runRowCount(runs);
  }

  /** A lane key as [name, " · rest"]: the name may be cut for room, the rest is what distinguishes the instance. */
  protected keyParts(key: string): [string, string] {
    const i = key.indexOf(' · ');
    return i > 0 ? [key.slice(0, i), key.slice(i)] : [key, ''];
  }

  protected tickAria(n: number): string {
    return $localize`:@@timeline.ticks.aria:${n}:count: warnings (zoom in for detail)`;
  }

  protected tickTitle(n: number): string {
    return $localize`:@@timeline.ticks.title:${n}:count: warnings — zoom in for detail`;
  }

  /** Jumps to the clicked glyph's event, or to the target's primary from the keyboard or the space around glyphs. */
  protected activate(event: Event, t: TargetVm): void {
    this.jump.emit(this.picked(event, t).cluster.id);
  }

  /** Right-click (or the context-menu key) on a marker: the timeline offers "Add note…" on the glyph under the pointer. */
  protected menu(event: MouseEvent, t: TargetVm): void {
    event.preventDefault();
    event.stopPropagation();
    this.markerMenu.emit({ x: event.clientX, y: event.clientY, marker: this.picked(event, t) });
  }

  /** The glyph the event hit, else the target's primary. */
  private picked(event: Event, t: TargetVm): LaidMarker {
    const member = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-member]')?.dataset['member'] : undefined;
    return (member === undefined ? undefined : t.members[Number(member)]?.marker) ?? t.primary;
  }
}
