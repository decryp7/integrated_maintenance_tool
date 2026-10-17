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
        <span class="section-title">Flame graph</span>
        <div class="seg" role="group" aria-label="Layout">
          <button type="button" class="btn" [attr.aria-pressed]="orientation() === 'flame'" (click)="orientation.set('flame')">Flame</button>
          <button type="button" class="btn" [attr.aria-pressed]="orientation() === 'icicle'" (click)="orientation.set('icicle')">Icicle</button>
        </div>
        <label class="inline">Stacks
          <select aria-label="Stack direction" (change)="setDirection($event)">
            <option value="topDown" [selected]="direction() === 'topDown'">Callers first (top-down)</option>
            <option value="bottomUp" [selected]="direction() === 'bottomUp'">Callees first (inverted)</option>
          </select>
        </label>
        <label class="inline">Colour by
          <select aria-label="Colour by" (change)="colorBy.set($any($event.target).value)">
            <option value="kind" [selected]="colorBy() === 'kind'">Frame kind</option>
            <option value="module" [selected]="colorBy() === 'module'">Module</option>
          </select>
        </label>
        <label class="inline">Order
          <select aria-label="Order" (change)="order.set($any($event.target).value)">
            <option value="name" [selected]="order() === 'name'">By name</option>
            <option value="value" [selected]="order() === 'value'">By value</option>
          </select>
        </label>
        <input type="search" placeholder="Highlight…" aria-label="Search frames" [value]="state.flameSearch()" (input)="onSearch($event)" />
        <label class="inline"><input type="checkbox" [checked]="regex()" (change)="regex.set($any($event.target).checked)" /> Regex</label>
        @if (res.data(); as g) {
          @if (state.flameSearch()) { <span class="muted" data-testid="flame-matched" role="status">Matched {{ pct(g.matchedPct) }} ({{ value(g, g.matchedValue) }})</span> }
        }
      </div>
      <nav class="crumbs" aria-label="Zoom path">
        @for (c of crumbs(); track $index; let last = $last) {
          @if (last) {
            <span class="crumb current" aria-current="location">{{ c.label }}</span>
          } @else {
            <button class="link crumb" type="button" (click)="setFocus(c.path)">{{ c.label }}</button><span aria-hidden="true" class="sep">›</span>
          }
        }
        @if (state.flameFocus().length) { <button class="mini" type="button" (click)="setFocus([])">Reset zoom</button> }
      </nav>
      @if (res.data()?.truncated) {
        <div class="notice" role="status">More frames than the drawing limit: the smallest are left out. Zoom in for detail.</div>
      }
      <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [hasData]="!!res.data()" [empty]="res.data()?.rects?.length === 0" emptyText="No samples match this view." (retry)="res.retry()">
        <imt-flame-chart
          [graph]="res.data()"
          [orientation]="orientation()"
          [colorBy]="colorBy()"
          [metrics]="p.metrics"
          [label]="'Flame graph of ' + p.name"
          (zoom)="zoomTo($event)"
          (zoomOut)="zoomOut()"
          (selected)="state.selectFrame($event)"
        />
      </imt-analysis-state>
    } @else {
      <imt-no-capture text="Open a profile to see its flame graph." />
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
