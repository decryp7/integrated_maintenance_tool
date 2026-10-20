import { afterNextRender, ChangeDetectionStrategy, Component, computed, DestroyRef, effect, ElementRef, inject, signal, untracked, viewChild } from '@angular/core';
import { ProfileEventDto, ProfileEventKind, ThreadLane, ThreadTimeline } from '../../core/api.types';
import { formatDateTime } from '../../core/format';
import { WorkspaceStore } from '../../core/workspace.store';
import { AnalysisState } from '../../analysis/analysis-state';
import { AsyncResource } from '../../analysis/async-resource';
import { ProfileViewState } from '../profile-view-state';
import { ProfilingBackend } from '../profiling-backend';
import { formatNs } from '../profiling-format';
import { ActiveProfile } from '../profiling-navigator';
import { NoCapture, ProfileBar } from '../profiling-shared';
import { PROFILING_STYLES } from '../profiling-styles';

const LABEL_W = 200;
const LANE_H = 28;
const MAX_EVENTS = 500;

/** A distinct glyph per event kind (shape + label, never colour alone). */
export const EVENT_GLYPHS: Record<ProfileEventKind, { label: string; path: string; color: string }> = {
  gc: { label: $localize`:@@profiling.threads.event.gc:GC`, path: 'M6 1L11 6L6 11L1 6Z', color: 'var(--s2)' },
  jit: { label: $localize`:@@profiling.threads.event.jit:JIT`, path: 'M6 1L11 11H1Z', color: 'var(--s3)' },
  exception: { label: $localize`:@@profiling.threads.event.exception:Exception`, path: 'M2 2L10 10M10 2L2 10', color: 'var(--lvl-error)' },
  lockWait: { label: $localize`:@@profiling.threads.event.lockWait:Lock wait`, path: 'M2 2H10V10H2Z', color: 'var(--s4)' },
  wait: { label: $localize`:@@profiling.threads.event.wait:Wait`, path: 'M1 6H11M6 1V6', color: 'var(--s1)' },
  marker: { label: $localize`:@@profiling.threads.event.marker:Marker`, path: 'M6 0V12M3 3H9', color: 'var(--text-2)' },
};

/** Bars of one lane as an SVG path in a 1000 × 100 box. */
export function lanePath(buckets: readonly number[], max: number): string {
  if (!max || !buckets.length) return '';
  const w = 1000 / buckets.length;
  let d = '';
  buckets.forEach((v, i) => {
    if (v <= 0) return;
    const h = Math.max(4, (v / max) * 100);
    d += `M${(i * w).toFixed(2)} 100V${(100 - h).toFixed(2)}H${((i + 1) * w).toFixed(2)}V100Z`;
  });
  return d;
}

/** A brushed pixel span → time range in ns (clamped to the shown range). */
export function brushToRange(x0: number, x1: number, width: number, fromNs: number, toNs: number): { fromNs: number; toNs: number } | null {
  if (width <= 0 || Math.abs(x1 - x0) < 4) return null;
  const f = (x: number) => fromNs + (Math.max(0, Math.min(width, x)) / width) * (toNs - fromNs);
  return { fromNs: Math.floor(f(Math.min(x0, x1))), toNs: Math.ceil(f(Math.max(x0, x1))) };
}

interface EventVm {
  e: ProfileEventDto;
  lane: number;
  left: number;
  width: number;
}

/**
 * Per-thread activity over time plus GC, JIT, exception, lock-wait and marker events. Brushing a
 * range (or typing it) sets the shared profile range, which re-queries every profiling view.
 */
@Component({
  selector: 'imt-thread-timeline-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, ProfileBar, NoCapture],
  template: `
    @if (active.profile(); as p) {
      <imt-profile-bar [showMetric]="false" />
      <div class="bar">
        <span class="section-title" i18n="@@profiling.threads.title">Thread timeline</span>
        <div class="kinds" role="group" aria-label="Event kinds" i18n-aria-label="@@profiling.threads.kinds">
          @for (k of p.eventKinds; track k) {
            <button type="button" class="lg" [attr.aria-pressed]="!hidden().has(k)" [class.hl]="state.eventKind() === k" (click)="toggleKind(k)">
              <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"><path [attr.d]="glyph(k).path" [style.fill]="k === 'exception' || k === 'marker' ? 'none' : glyph(k).color" [style.stroke]="glyph(k).color" stroke-width="1.6" /></svg>
              {{ glyph(k).label }} ({{ kindCount(k) }})
            </button>
          }
        </div>
        <span class="spacer"></span>
        <form class="range" (submit)="applyTyped($event, from.value, to.value)" aria-label="Time range in milliseconds since the capture start" i18n-aria-label="@@profiling.threads.rangeForm">
          <label class="inline"><ng-container i18n="@@profiling.threads.from">From</ng-container>&ngsp;<input #from type="number" min="0" step="any" aria-label="Range start (ms)" i18n-aria-label="@@profiling.threads.rangeStart" [value]="msOf(state.range()?.fromNs)" /></label>
          <label class="inline"><ng-container i18n="@@profiling.threads.to">To</ng-container>&ngsp;<input #to type="number" min="0" step="any" aria-label="Range end (ms)" i18n-aria-label="@@profiling.threads.rangeEnd" [value]="msOf(state.range()?.toNs)" /></label>
          <button class="btn" type="submit" i18n="@@profiling.threads.apply">Apply range</button>
          <button class="btn" type="button" [disabled]="!state.range()" (click)="state.setRange(null)" i18n="@@profiling.threads.clear">Clear range</button>
        </form>
      </div>
      @if (!p.hasTime) {
        <div class="state" role="status" i18n="@@profiling.threads.noTimestamps">This capture has no timestamps (e.g. folded stacks), so there is no timeline.</div>
      } @else {
        <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [hasData]="!!res.data()" [empty]="res.data()?.threads?.length === 0" emptyText="No thread activity in this range." i18n-emptyText="@@profiling.threads.empty" (retry)="res.retry()">
          @if (res.data(); as t) {
            <div class="axis mono" aria-hidden="true">
              <span [style.left.px]="labelW">{{ axisLabel(t, t.fromNs) }}</span>
              <span class="end">{{ axisLabel(t, t.toNs) }}</span>
            </div>
            <div class="lanes" #lanes (pointerdown)="down($event)" (pointermove)="move($event)" (pointerup)="up($event)" (pointercancel)="brush.set(null)">
              @for (l of t.threads; track l.thread.id; let i = $index) {
                <div class="lane">
                  <button class="label" type="button" [class.on]="isFiltered(l)" [attr.aria-pressed]="isFiltered(l)" [title]="onlyTitle(l.thread.name)" (pointerdown)="$event.stopPropagation()" (click)="state.toggleThread(l.thread.id)">
                    <span class="ellipsis">{{ l.thread.name }}</span>
                    <span class="faint mono">{{ l.thread.osId ?? '' }}</span>
                  </button>
                  <div class="plot">
                    <svg viewBox="0 0 1000 100" preserveAspectRatio="none" role="img" [attr.aria-label]="laneLabel(l.thread.name, l.total)"><path [attr.d]="path(l)" /></svg>
                    @for (ev of laneEvents().get(i) ?? []; track $index) {
                      <span class="ev" [class.dim]="state.eventKind() !== null && state.eventKind() !== ev.e.kind" [style.left.%]="ev.left" [title]="eventTitle(ev.e)">
                        @if (ev.width > 0.3) { <span class="dur" [style.width.%]="(ev.width / (100 - ev.left)) * 100" [style.background]="glyph(ev.e.kind).color"></span> }
                        <svg width="10" height="10" viewBox="0 0 12 12" aria-hidden="true"><path [attr.d]="glyph(ev.e.kind).path" [style.fill]="ev.e.kind === 'exception' || ev.e.kind === 'marker' ? 'none' : glyph(ev.e.kind).color" [style.stroke]="glyph(ev.e.kind).color" stroke-width="1.6" /></svg>
                      </span>
                    }
                  </div>
                </div>
              }
              @if (brush(); as b) { <div class="brush" [style.left.px]="b.min" [style.width.px]="b.max - b.min"></div> }
            </div>
            @if (t.eventsTruncated) { <div class="notice" role="status" i18n="@@profiling.threads.truncated">Only the first {{ maxEvents }} events are shown; narrow the range for the rest.</div> }
            <details class="events">
              <summary i18n="@@profiling.threads.eventsInView">Events in view ({{ t.events.length }})</summary>
              <table class="grid">
                <thead><tr><th scope="col" i18n="@@profiling.threads.col.kind">Kind</th><th scope="col" i18n="@@profiling.threads.col.thread">Thread</th><th scope="col" class="num" i18n="@@profiling.threads.col.start">Start</th><th scope="col" class="num" i18n="@@profiling.threads.col.duration">Duration</th><th scope="col" i18n="@@profiling.threads.col.label">Label</th></tr></thead>
                <tbody>
                  @for (e of t.events; track $index) {
                    <tr><td>{{ glyph(e.kind).label }}</td><td>{{ threadName(t, e.thread) }}</td><td class="num">{{ ns(e.startNs) }}</td><td class="num">{{ e.durNs ? ns(e.durNs) : '—' }}</td><td>{{ e.label }}</td></tr>
                  }
                </tbody>
              </table>
            </details>
          }
        </imt-analysis-state>
      }
    } @else {
      <imt-no-capture text="Open a profile with timestamps to see its threads over time." i18n-text="@@profiling.threads.noCapture" />
    }
  `,
  styles: [
    PROFILING_STYLES,
    `
      .state { padding: 24px 12px; color: var(--muted); text-align: center; }
      .kinds { display: flex; gap: 4px; flex-wrap: wrap; }
      .lg { display: inline-flex; align-items: center; gap: 5px; border: 1px solid transparent; background: transparent; color: var(--text-2); font: inherit; padding: 1px 6px; border-radius: 10px; cursor: pointer; }
      .lg[aria-pressed='false'] { opacity: 0.5; text-decoration: line-through; }
      .lg.hl { border-color: var(--accent); background: var(--accent-soft); }
      .range { display: flex; gap: 6px; align-items: center; }
      .range input { width: 90px; }
      .axis { position: relative; height: 18px; font-size: 10px; color: var(--faint); margin: 0 12px; }
      .axis span { position: absolute; top: 2px; }
      .axis .end { right: 0; }
      .lanes { position: relative; flex: 1; min-height: 0; overflow-y: auto; padding: 0 12px 8px; user-select: none; cursor: crosshair; }
      .lane { display: flex; height: ${LANE_H}px; border-bottom: 1px solid var(--border); }
      .label { width: ${LABEL_W}px; flex: none; display: flex; align-items: center; gap: 6px; border: 0; border-left: 2px solid transparent; background: transparent; color: var(--text); font: inherit; text-align: left; padding: 0 8px; cursor: pointer; min-width: 0; }
      .label.on { border-left-color: var(--accent); background: var(--bg-active); }
      .label:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
      .plot { position: relative; flex: 1; min-width: 0; }
      .plot svg[role='img'] { position: absolute; inset: 3px 0; width: 100%; height: ${LANE_H - 6}px; }
      .plot path { fill: var(--s1); opacity: 0.75; }
      .ev { position: absolute; top: 2px; height: 12px; display: flex; align-items: center; margin-left: -5px; }
      .ev.dim { opacity: 0.25; }
      .ev .dur { position: absolute; left: 5px; top: 4px; height: 4px; opacity: 0.6; min-width: 2px; }
      .ev svg path { opacity: 1; }
      .brush { position: absolute; top: 0; bottom: 0; background: var(--accent-soft); border-left: 1px solid var(--accent); border-right: 1px solid var(--accent); pointer-events: none; }
      .events { flex: none; max-height: 160px; overflow: auto; padding: 2px 12px 6px; }
      .events summary { cursor: pointer; color: var(--muted); }
      .faint { color: var(--faint); }
    `,
  ],
})
export class ThreadTimelineView {
  protected readonly active = inject(ActiveProfile);
  protected readonly state = inject(ProfileViewState);
  private readonly backend = inject(ProfilingBackend);
  private readonly workspace = inject(WorkspaceStore);
  private readonly lanes = viewChild<ElementRef<HTMLElement>>('lanes');
  protected readonly res = new AsyncResource<ThreadTimeline>();
  protected readonly hidden = signal<ReadonlySet<ProfileEventKind>>(new Set());
  protected readonly brush = signal<{ start: number; min: number; max: number } | null>(null);
  protected readonly labelW = LABEL_W;
  protected readonly maxEvents = MAX_EVENTS;
  protected readonly ns = formatNs;
  private readonly plotWidth = signal(900);

  /** Events per lane index (thread-less events go on the first lane). */
  protected readonly laneEvents = computed(() => {
    const t = this.res.data();
    const out = new Map<number, EventVm[]>();
    if (!t) return out;
    const span = Math.max(1, t.toNs - t.fromNs);
    for (const e of t.events) {
      const lane = Math.max(0, t.threads.findIndex((l) => l.thread.id === e.thread));
      const left = Math.max(0, Math.min(100, ((e.startNs - t.fromNs) / span) * 100));
      const width = Math.min(100 - left, (e.durNs / span) * 100);
      (out.get(lane) ?? out.set(lane, []).get(lane)!).push({ e, lane, left, width });
    }
    return out;
  });

  constructor() {
    effect(() => {
      const p = this.active.profile();
      const hidden = this.hidden();
      const request = p?.hasTime
        ? { profileId: p.id, view: this.state.view(), buckets: Math.max(50, Math.min(1000, Math.round(this.plotWidth() / 3))), eventKinds: p.eventKinds.filter((k) => !hidden.has(k)), maxEvents: MAX_EVENTS }
        : null;
      untracked(() => (request ? void this.res.load(() => this.backend.threads(request)) : this.res.reset()));
    });
    const destroy = inject(DestroyRef);
    const host = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
    afterNextRender(() => {
      const ro = new ResizeObserver(() => {
        if (host.clientWidth) this.plotWidth.set(host.clientWidth - LABEL_W - 24);
      });
      ro.observe(host);
      destroy.onDestroy(() => ro.disconnect());
    });
  }

  protected glyph(k: ProfileEventKind) {
    return EVENT_GLYPHS[k];
  }

  protected kindCount(k: ProfileEventKind): number {
    return this.res.data()?.events.filter((e) => e.kind === k).length ?? 0;
  }

  protected toggleKind(k: ProfileEventKind): void {
    const next = new Set(this.hidden());
    if (next.has(k)) next.delete(k);
    else next.add(k);
    this.hidden.set(next);
    if (this.state.eventKind() === k) this.state.setEventKind(null);
  }

  protected path(l: ThreadLane): string {
    const max = Math.max(...(this.res.data()?.threads.flatMap((t) => t.buckets) ?? [0]));
    return lanePath(l.buckets, max);
  }

  protected isFiltered(l: ThreadLane): boolean {
    return !!this.state.threads()?.includes(l.thread.id);
  }

  protected threadName(t: ThreadTimeline, id: number | null): string {
    if (id === null) return $localize`:@@profiling.threads.process:process`;
    return t.threads.find((l) => l.thread.id === id)?.thread.name ?? $localize`:@@profiling.threads.threadId:thread ${id}:id:`;
  }

  protected onlyTitle(name: string): string {
    return $localize`:@@profiling.threads.onlyTitle:Show only ${name}:name: (click again to show all)`;
  }

  protected laneLabel(name: string, total: number): string {
    return $localize`:@@profiling.threads.laneLabel:${name}:name:: ${total}:count: samples`;
  }

  protected eventTitle(e: ThreadTimeline['events'][number]): string {
    const kind = this.glyph(e.kind).label;
    const label = e.label;
    const at = this.ns(e.startNs);
    if (!e.durNs) return $localize`:@@profiling.threads.eventTitle:${kind}:kind:: ${label}:label: at ${at}:start:`;
    const dur = this.ns(e.durNs);
    return $localize`:@@profiling.threads.eventTitleDur:${kind}:kind:: ${label}:label: at ${at}:start: for ${dur}:duration:`;
  }

  protected axisLabel(t: ThreadTimeline, ns: number): string {
    return t.startedMs !== null ? formatDateTime(t.startedMs + ns / 1e6, this.workspace.timezone(), true) : formatNs(ns);
  }

  protected msOf(ns: number | undefined): string {
    return ns === undefined ? '' : String(Math.round(ns / 1e4) / 100);
  }

  protected applyTyped(e: Event, from: string, to: string): void {
    e.preventDefault();
    const a = Number(from);
    const b = Number(to);
    if (from === '' || to === '' || !Number.isFinite(a) || !Number.isFinite(b) || a < 0 || b < 0) return;
    this.state.setRange({ fromNs: a * 1e6, toNs: b * 1e6 });
  }

  /** x inside the plot (px) for a pointer event. */
  private plotX(e: PointerEvent): number {
    const el = this.lanes()?.nativeElement;
    const box = el?.getBoundingClientRect();
    return box ? e.clientX - box.left - 12 - LABEL_W : 0;
  }

  private width(): number {
    const el = this.lanes()?.nativeElement;
    return el ? el.getBoundingClientRect().width - 24 - LABEL_W : 0;
  }

  protected down(e: PointerEvent): void {
    if (e.button !== 0) return;
    const x = this.plotX(e);
    if (x < 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    this.brush.set({ start: x, min: x + LABEL_W + 12, max: x + LABEL_W + 12 });
  }

  protected move(e: PointerEvent): void {
    const b = this.brush();
    if (!b) return;
    const x = Math.max(0, Math.min(this.width(), this.plotX(e)));
    this.brush.set({ start: b.start, min: Math.min(b.start, x) + LABEL_W + 12, max: Math.max(b.start, x) + LABEL_W + 12 });
  }

  protected up(e: PointerEvent): void {
    const b = this.brush();
    this.brush.set(null);
    const t = this.res.data();
    if (!b || !t) return;
    const range = brushToRange(b.start, this.plotX(e), this.width(), t.fromNs, t.toNs);
    if (range) this.state.setRange(range);
  }
}
