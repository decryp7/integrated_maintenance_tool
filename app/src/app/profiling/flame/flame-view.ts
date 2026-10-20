import { ChangeDetectionStrategy, Component, computed, DestroyRef, effect, inject, signal, untracked } from '@angular/core';
import { FlameGraph, FlameOrder, TreeDirection } from '../../core/api.types';
import { AnalysisState } from '../../analysis/analysis-state';
import { AsyncResource } from '../../analysis/async-resource';
import { ProfileViewState } from '../profile-view-state';
import { ProfilingBackend } from '../profiling-backend';
import { formatMetric, formatPct } from '../profiling-format';
import { ActiveProfile } from '../profiling-navigator';
import { CaptureNotes, NoCapture, ProfileBar } from '../profiling-shared';
import { PROFILING_STYLES } from '../profiling-styles';
import { FlameChart } from './flame-chart';
import { ColorBy } from './flame-colors';
import { breadcrumbs, extendFocus, FlameOrientation, frameNamer } from './flame-model';
import { measuredWidth } from './measured-width';

/**
 * Flame graph / icicle of the active profile. The engine lays the rectangles out for the drawing
 * width, zoom (focus path), search and view; the chart draws them on a canvas. Nothing is requested
 * until the view has been laid out and its width is known.
 */
@Component({
  selector: 'imt-flame-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, ProfileBar, CaptureNotes, NoCapture, FlameChart],
  template: `
    @if (active.profile(); as p) {
      <imt-profile-bar />
      <imt-capture-notes [notes]="p.notes" [name]="p.name" />
      <div class="bar">
        <span class="section-title" i18n="@@profiling.flameView.title">Flame graph</span>
        <div class="seg" role="group" aria-label="Layout" i18n-aria-label="@@profiling.flameView.layout">
          <button type="button" class="btn" [attr.aria-pressed]="orientation() === 'flame'" (click)="orientation.set('flame')" i18n="@@profiling.flameView.flame">Flame</button>
          <button type="button" class="btn" [attr.aria-pressed]="orientation() === 'icicle'" (click)="orientation.set('icicle')" i18n="@@profiling.flameView.icicle">Icicle</button>
        </div>
        <label class="inline"><ng-container i18n="@@profiling.flameView.stacks">Stacks</ng-container>&ngsp;
          <select aria-label="Stack direction" i18n-aria-label="@@profiling.flameView.stackDirection" (change)="setDirection($event)">
            <option value="topDown" [selected]="direction() === 'topDown'" i18n="@@profiling.flameView.topDown">Callers first (top-down)</option>
            <option value="bottomUp" [selected]="direction() === 'bottomUp'" i18n="@@profiling.flameView.bottomUp">Callees first (inverted)</option>
          </select>
        </label>
        <label class="inline"><ng-container i18n="@@profiling.flameView.colourBy">Colour by</ng-container>&ngsp;
          <select aria-label="Colour by" i18n-aria-label="@@profiling.flameView.colourByLabel" (change)="colorBy.set($any($event.target).value)">
            <option value="kind" [selected]="colorBy() === 'kind'" i18n="@@profiling.flameView.byKind">Frame kind</option>
            <option value="module" [selected]="colorBy() === 'module'" i18n="@@profiling.flameView.byModule">Module</option>
          </select>
        </label>
        <label class="inline"><ng-container i18n="@@profiling.flameView.order">Order</ng-container>&ngsp;
          <select aria-label="Order" i18n-aria-label="@@profiling.flameView.orderLabel" (change)="order.set($any($event.target).value)">
            <option value="name" [selected]="order() === 'name'" i18n="@@profiling.flameView.byName">By name</option>
            <option value="value" [selected]="order() === 'value'" i18n="@@profiling.flameView.byValue">By value</option>
          </select>
        </label>
        <input type="search" placeholder="Highlight…" i18n-placeholder="@@profiling.flameView.highlight" aria-label="Search frames" i18n-aria-label="@@profiling.flameView.searchLabel" [value]="state.flameSearch()" (input)="onSearch($event)" />
        <label class="inline" i18n="@@profiling.flameView.regex"><input type="checkbox" [checked]="regex()" (change)="regex.set($any($event.target).checked)" /> Regex</label>
        @if (res.data(); as g) {
          @if (state.flameSearch()) { <span class="muted" data-testid="flame-matched" role="status" i18n="@@profiling.flameView.matched">Matched {{ pct(g.matchedPct) // i18n(ph: "pct") }} ({{ value(g, g.matchedValue) // i18n(ph: "value") }})</span> }
        }
      </div>
      <nav class="crumbs" aria-label="Zoom path" i18n-aria-label="@@profiling.flameView.zoomPath">
        @for (c of crumbs(); track $index; let last = $last) {
          @if (last) {
            <span class="crumb current" aria-current="location">{{ c.label }}</span>
          } @else {
            <button class="link crumb" type="button" (click)="setFocus(c.path)">{{ c.label }}</button><span aria-hidden="true" class="sep">›</span>
          }
        }
        @if (state.flameFocus().length) { <button class="mini" type="button" (click)="setFocus([])" i18n="@@profiling.flameView.resetZoom">Reset zoom</button> }
      </nav>
      @if (res.data()?.truncated) {
        <div class="notice" role="status" i18n="@@profiling.flameView.truncated">More frames than the drawing limit: the smallest are left out. Zoom in for detail.</div>
      }
      <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [retryable]="res.retryable()" [hasData]="!!res.data()" [empty]="res.data()?.rects?.length === 0" emptyText="No samples match this view." i18n-emptyText="@@profiling.flameView.empty" (retry)="res.retry()">
        <imt-flame-chart
          [graph]="res.data()"
          [orientation]="orientation()"
          [colorBy]="colorBy()"
          [metrics]="p.metrics"
          [label]="chartLabel(p.name)"
          (zoom)="zoomTo($event)"
          (zoomOut)="zoomOut()"
          (selected)="state.selectFrame($event)"
        />
      </imt-analysis-state>
    } @else {
      <imt-no-capture text="Open a profile to see its flame graph." i18n-text="@@profiling.flameView.noCapture" />
    }
  `,
  styles: [
    PROFILING_STYLES,
    `
      .seg { display: inline-flex; gap: 2px; }
      .seg .btn[aria-pressed='true'] { border-color: var(--accent); background: var(--accent-soft); }
      .crumbs { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; padding: 4px 12px; font-size: 12px; min-height: 26px; }
      .crumb.current { color: var(--text); font-weight: 600; }
      .sep { color: var(--faint); }
    `,
  ],
})
export class FlameView {
  protected readonly active = inject(ActiveProfile);
  protected readonly state = inject(ProfileViewState);
  private readonly backend = inject(ProfilingBackend);
  protected readonly res = new AsyncResource<FlameGraph>();
  protected readonly orientation = signal<FlameOrientation>('flame');
  protected readonly direction = signal<TreeDirection>('topDown');
  protected readonly colorBy = signal<ColorBy>('kind');
  protected readonly order = signal<FlameOrder>('name');
  protected readonly regex = signal(false);
  protected readonly pct = formatPct;
  private searchTimer: ReturnType<typeof setTimeout> | undefined;
  /** Drawing width of the view; null until it has been laid out. */
  private readonly widthPx = measuredWidth();

  /** Zoom breadcrumbs, named from the engine's answer (its focus frames). */
  protected readonly crumbs = computed(() => breadcrumbs(this.state.flameFocus(), frameNamer(this.res.data())));

  constructor() {
    effect(() => {
      const p = this.active.profile();
      const metric = this.active.metric();
      const widthPx = this.widthPx();
      // Not laid out yet: wait for the real width rather than asking for a guessed one.
      if (p && metric && widthPx === null) return;
      const request = p && metric && widthPx !== null
        ? { profileId: p.id, view: this.state.view(), direction: this.direction(), focus: this.state.flameFocus(), widthPx, metric, order: this.order(), search: this.state.flameSearch() || null, searchRegex: this.regex() }
        : null;
      untracked(() => {
        if (!request) return this.res.reset();
        void this.res.load(() => this.backend.flame(request));
      });
    });
    inject(DestroyRef).onDestroy(() => clearTimeout(this.searchTimer));
  }

  protected chartLabel(name: string): string {
    return $localize`:@@profiling.flameView.chartLabel:Flame graph of ${name}:name:`;
  }

  protected value(g: FlameGraph, v: number): string {
    return formatMetric(v, g.metric, this.active.profile()?.metrics);
  }

  protected setDirection(e: Event): void {
    this.direction.set((e.target as HTMLSelectElement).value as TreeDirection);
    this.state.setFlameFocus([]);
  }

  protected onSearch(e: Event): void {
    const value = (e.target as HTMLInputElement).value;
    clearTimeout(this.searchTimer);
    this.searchTimer = setTimeout(() => this.state.setFlameSearch(value), 200);
  }

  protected setFocus(path: number[]): void {
    this.state.setFlameFocus(path);
  }

  /** Clicking (or Enter on) a frame zooms to it: its path continues the current focus. */
  protected zoomTo(chain: number[]): void {
    this.state.setFlameFocus(extendFocus(this.state.flameFocus(), chain));
  }

  protected zoomOut(): void {
    this.state.setFlameFocus(this.state.flameFocus().slice(0, -1));
  }
}
