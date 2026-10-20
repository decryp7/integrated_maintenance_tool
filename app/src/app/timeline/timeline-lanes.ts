import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';
import { AnnotationDto } from '../core/api.types';
import { Icon } from '../shared/icon';
import { MarkerGlyph } from '../markers/marker-glyph';
import { SourceSwatch } from '../sources/source-swatch';
import { AnnotationVm } from './annotation-layout';
import { AnnotationHover, AnnotationTrack } from './annotation-track';
import { LaidMarker } from './marker-layout';
import { RunBandVm } from './run-layout';
import { RunHover, RunTrack } from './run-track';
import { LABEL_W, LaneVm, TargetVm } from './timeline-lane-vm';

/** The marker under the pointer, for the tooltip. */
export interface MarkerHover {
  x: number;
  y: number;
  lane: string;
  marker: LaidMarker;
}

/** What the lanes show besides themselves when there are none. */
export type LanesEmpty = 'noSources' | 'noMatchMain' | 'noMatchOwn' | null;

/**
 * The lane rows of a timeline: label, extent bar, density, run and annotation bands, and marker hit areas. It draws
 * the view model it is given and reports what the user did; it knows nothing about stores.
 */
@Component({
  selector: 'imt-timeline-lanes',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, MarkerGlyph, SourceSwatch, AnnotationTrack, RunTrack],
  template: `
    @for (l of lanes(); track l.lane.key) {
      <div class="row lane">
        <button class="label lane-label" [class.filtered]="l.filtered" (click)="toggleGroup.emit(l.lane.key)" [title]="l.title">
          <span class="key-row">
            @for (m of l.marks; track m.swatch.slot) {
              <imt-source-swatch [swatch]="m.swatch" [label]="m.label" [size]="8" />
            }
            @if (l.moreSources > 0) {
              <span class="faint more-src">+{{ l.moreSources }}</span>
            }
            <span class="mono key ellipsis">{{ l.lane.key }}</span>
          </span>
          <span class="faint meta ellipsis">{{ l.meta }}</span>
        </button>
        <div class="plot track">
          <div class="baseline"></div>
          <div class="bar" [style.left.%]="l.left" [style.width.%]="l.width"></div>
          @if (runLayout().get(l.lane.key); as runs) {
            <imt-run-track [items]="runs" (hovered)="runHover.emit($event)" (activated)="showRun.emit($event.ann)" />
          }
          @if (annLanes().get(l.lane.key); as items) {
            <imt-annotation-track [items]="items" (hovered)="annHover.emit($event)" (activated)="showEvidence.emit($event.ann)" />
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
              [style.left.%]="t.leftPct"
              [style.width.px]="t.width"
              [attr.aria-label]="t.aria"
              (pointerdown)="$event.stopPropagation()"
              (click)="activate($event, t)"
              (pointerenter)="markerHover.emit({ x: $event.clientX, y: $event.clientY, lane: l.lane.key, marker: t.primary })"
              (pointerleave)="markerHover.emit(null)"
            >
              @for (g of t.members; track g.marker.cluster.kind + g.marker.cluster.bucket) {
                <span
                  class="glyph"
                  [class.primary]="g.marker === t.primary"
                  [style.left.px]="g.offset"
                  [attr.data-member]="$index"
                  (pointerenter)="markerHover.emit({ x: $event.clientX, y: $event.clientY, lane: l.lane.key, marker: g.marker })"
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
        @switch (empty()) {
          @case ('noSources') {
            <imt-icon name="files" [size]="28" />
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
    .label { width: ${LABEL_W}px; flex: none; }
    .plot { flex: 1; min-width: 0; position: relative; margin-right: 14px; }
    .lane { height: 32px; }
    .lane-label { display: flex; flex-direction: column; justify-content: center; align-items: flex-start; gap: 0; padding: 0 10px 0 12px; border: 0; border-left: 2px solid transparent; background: transparent; cursor: pointer; text-align: left; min-width: 0; }
    .lane-label:hover { background: var(--bg-hover); }
    .lane-label.filtered { border-left-color: var(--accent); background: var(--bg-active); }
    .key { font-size: 12px; color: var(--text); max-width: 100%; min-width: 0; }
    .key-row { display: flex; align-items: center; gap: 4px; max-width: 100%; min-width: 0; }
    .more-src { font-size: 10px; }
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
    .truncated { padding: 6px 14px; font-size: 12px; }
  `,
})
export class TimelineLanes {
  readonly lanes = input.required<readonly LaneVm[]>();
  readonly runLayout = input.required<ReadonlyMap<string, RunBandVm[]>>();
  readonly annLanes = input.required<ReadonlyMap<string, AnnotationVm[]>>();
  readonly densityViewBox = input.required<string>();
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

  protected tickAria(n: number): string {
    return $localize`:@@timeline.ticks.aria:${n}:count: warnings (zoom in for detail)`;
  }

  protected tickTitle(n: number): string {
    return $localize`:@@timeline.ticks.title:${n}:count: warnings — zoom in for detail`;
  }

  /** Jumps to the clicked glyph's event, or to the target's primary from the keyboard or the space around glyphs. */
  protected activate(event: Event, t: TargetVm): void {
    const member = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-member]')?.dataset['member'] : undefined;
    const picked = member === undefined ? undefined : t.members[Number(member)]?.marker;
    this.jump.emit((picked ?? t.primary).cluster.id);
  }
}
