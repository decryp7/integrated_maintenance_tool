import { ChangeDetectionStrategy, Component, computed, DestroyRef, effect, inject, signal, untracked } from '@angular/core';
import { DiffSort, FlameGraph, FunctionDiffPage, FunctionDiffRow } from '../../core/api.types';
import { AnalysisState } from '../../analysis/analysis-state';
import { AsyncResource } from '../../analysis/async-resource';
import { CapturesStore } from '../captures.store';
import { ProfileViewState } from '../profile-view-state';
import { ProfilingBackend } from '../profiling-backend';
import { formatMetric, formatPct, formatPp } from '../profiling-format';
import { ActiveProfile } from '../profiling-navigator';
import { NoCapture, ProfileBar } from '../profiling-shared';
import { PROFILING_STYLES } from '../profiling-styles';
import { FlameChart } from '../flame/flame-chart';
import { breadcrumbs, extendFocus, frameNamer } from '../flame/flame-model';
import { measuredWidth } from '../flame/measured-width';

const DIFF_PAGE = 200;

const SORTS: { id: DiffSort; label: string }[] = [
  { id: 'absDeltaSelf', label: $localize`:@@profiling.compare.sort.absDeltaSelf:Largest self change` },
  { id: 'absDeltaTotal', label: $localize`:@@profiling.compare.sort.absDeltaTotal:Largest total change` },
  { id: 'deltaSelf', label: $localize`:@@profiling.compare.sort.deltaSelf:Self growth` },
  { id: 'deltaTotal', label: $localize`:@@profiling.compare.sort.deltaTotal:Total growth` },
  { id: 'name', label: $localize`:@@profiling.compare.sort.name:Name` },
];

const STATUS_LABEL = {
  new: $localize`:@@profiling.compare.status.new:new`,
  removed: $localize`:@@profiling.compare.status.removed:removed`,
  changed: $localize`:@@profiling.compare.status.changed:changed`,
  same: $localize`:@@profiling.compare.status.same:same`,
} as const;

/**
 * Baseline vs target (the active profile): differential flame graph (red = larger share than the
 * baseline, blue = smaller, grey = unchanged) and the per-function changes sorted by size.
 */
@Component({
  selector: 'imt-compare-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, ProfileBar, NoCapture, FlameChart],
  template: `
    @if (active.profile(); as p) {
      <imt-profile-bar />
      <div class="bar">
        <span class="section-title" i18n="@@profiling.compare.title">Compare</span>
        <label class="inline"><ng-container i18n="@@profiling.compare.baseline">Baseline</ng-container>&ngsp;
          <select aria-label="Baseline profile" i18n-aria-label="@@profiling.compare.baselineLabel" (change)="pickBaseline($event)">
            <option value="" [selected]="state.baselineId() === null" i18n="@@profiling.compare.chooseBaseline">Choose a baseline…</option>
            @for (c of baselines(); track c.id) { <option [value]="c.id" [selected]="c.id === state.baselineId()">{{ c.name }}</option> }
          </select>
        </label>
        <span class="muted" i18n="@@profiling.compare.target">→ target <strong>{{ p.name }}</strong></span>
      </div>
      @if (state.baselineId() === null) {
        <div class="state" role="status">{{ baselineHint() }}</div>
      } @else {
        <div class="split">
          <section class="pane flame" aria-label="Differential flame graph" i18n-aria-label="@@profiling.compare.flameSection">
            <nav class="crumbs" aria-label="Zoom path" i18n-aria-label="@@profiling.compare.zoomPath">
              @for (c of crumbs(); track $index; let last = $last) {
                @if (last) { <span class="current" aria-current="location">{{ c.label }}</span> }
                @else { <button class="link" type="button" (click)="focus.set(c.path)">{{ c.label }}</button><span aria-hidden="true" class="sep">›</span> }
              }
            </nav>
            <imt-analysis-state [loading]="flame.loading()" [error]="flame.error()" [hasData]="!!flame.data()" (retry)="flame.retry()">
              <imt-flame-chart [graph]="flame.data()" [diff]="true" [metrics]="p.metrics" [label]="flameLabel(p.name)" (zoom)="focus.set(extend($event))" (zoomOut)="focus.set(focus().slice(0, -1))" (selected)="state.selectFrame($event)" />
            </imt-analysis-state>
          </section>
          <section class="pane table" aria-label="Function changes" i18n-aria-label="@@profiling.compare.tableSection">
            <div class="bar">
              <span class="section-title" i18n="@@profiling.compare.changes">Function changes</span>
              <label class="inline"><ng-container i18n="@@profiling.compare.sort">Sort</ng-container>&ngsp;
                <select aria-label="Sort changes" i18n-aria-label="@@profiling.compare.sortLabel" (change)="sort.set($any($event.target).value)">
                  @for (s of sorts; track s.id) { <option [value]="s.id" [selected]="s.id === sort()">{{ s.label }}</option> }
                </select>
              </label>
              <input type="search" placeholder="Search…" i18n-placeholder="@@profiling.compare.searchPlaceholder" aria-label="Search changes" i18n-aria-label="@@profiling.compare.searchLabel" (input)="onSearch($event)" />
              @if (diff.data(); as d) { <span class="muted" i18n="@@profiling.compare.summary">{{ d.totalRows // i18n(ph: "count") }} functions · baseline {{ value(d.baselineTotal) // i18n(ph: "baseline") }} · target {{ value(d.targetTotal) // i18n(ph: "target") }}</span> }
            </div>
            <imt-analysis-state [loading]="diff.loading()" [error]="diff.error()" [hasData]="!!diff.data()" [empty]="diff.data()?.totalRows === 0" emptyText="No functions match." i18n-emptyText="@@profiling.compare.noMatch" (retry)="diff.retry()">
              <div class="body tight" tabindex="0" role="region" aria-label="Function change rows" i18n-aria-label="@@profiling.compare.rowsRegion">
                <table class="grid" aria-label="Function changes, sorted by change" i18n-aria-label="@@profiling.compare.tableLabel">
                  <thead>
                    <tr>
                      <th scope="col" i18n="@@profiling.compare.col.function">Function</th><th scope="col" i18n="@@profiling.compare.col.status">Status</th>
                      <th scope="col" class="num" i18n="@@profiling.compare.col.baselineSelf">Baseline self %</th><th scope="col" class="num" i18n="@@profiling.compare.col.targetSelf">Target self %</th><th scope="col" class="num" i18n="@@profiling.compare.col.deltaSelf">Δ self</th><th scope="col" class="num" i18n="@@profiling.compare.col.deltaTotal">Δ total</th>
                    </tr>
                  </thead>
                  <tbody>
                    @for (r of rows(); track r.key) {
                      <tr>
                        <td class="ellipsis fn" [title]="r.name">{{ r.name }}</td>
                        <td><span class="tag" [class]="'tag ' + r.status">{{ status(r) }}</span></td>
                        <td class="num">{{ pct(r.baseline.selfPct) }}</td>
                        <td class="num">{{ pct(r.target.selfPct) }}</td>
                        <td class="num" [class.up]="r.deltaSelfPct > 0.05" [class.down]="r.deltaSelfPct < -0.05"><span aria-hidden="true">{{ arrow(r.deltaSelfPct) }}</span> {{ pp(r.deltaSelfPct) }}</td>
                        <td class="num">{{ pp(r.deltaTotalPct) }}</td>
                      </tr>
                    }
                  </tbody>
                </table>
                @if (more() > 0) { <button class="btn more" type="button" (click)="loadMore()" i18n="@@profiling.compare.loadMore">Load {{ more() // i18n(ph: "count") }} more</button> }
              </div>
            </imt-analysis-state>
          </section>
        </div>
      }
    } @else {
      <imt-no-capture text="Open two profiles to compare them." i18n-text="@@profiling.compare.noCapture" />
    }
  `,
  styles: [
    PROFILING_STYLES,
    `
      .state { padding: 24px 12px; color: var(--muted); text-align: center; }
      .split { flex: 1; min-height: 0; display: flex; flex-direction: column; }
      .pane { min-height: 0; display: flex; flex-direction: column; }
      .pane.flame { flex: 3; border-bottom: 1px solid var(--border); }
      .pane.table { flex: 2; }
      .tight { padding: 0; }
      .fn { max-width: 460px; }
      .crumbs { display: flex; gap: 4px; align-items: center; flex-wrap: wrap; padding: 4px 12px; font-size: var(--fs-sm); }
      .current { color: var(--text); font-weight: 600; }
      .sep { color: var(--faint); }
      .more { margin: 8px 12px; }
    `,
  ],
})
export class CompareView {
  protected readonly active = inject(ActiveProfile);
  protected readonly state = inject(ProfileViewState);
  private readonly captures = inject(CapturesStore);
  private readonly backend = inject(ProfilingBackend);
  protected readonly flame = new AsyncResource<FlameGraph>();
  protected readonly diff = new AsyncResource<FunctionDiffPage>();
  protected readonly focus = signal<number[]>([]);
  protected readonly sort = signal<DiffSort>('absDeltaSelf');
  protected readonly search = signal('');
  protected readonly extra = signal<FunctionDiffRow[]>([]);
  protected readonly sorts = SORTS;
  protected readonly pct = formatPct;
  protected readonly pp = formatPp;
  protected readonly extend = (chain: number[]) => extendFocus(this.focus(), chain);
  private searchTimer: ReturnType<typeof setTimeout> | undefined;

  protected readonly baselines = computed(() => this.captures.profiles().filter((c) => c.id !== this.active.profile()?.id));
  protected readonly baselineHint = computed(() =>
    this.baselines().length
      ? $localize`:@@profiling.compare.chooseHint:Choose a baseline profile to compare with.`
      : $localize`:@@profiling.compare.openSecondHint:Open a second profile to use as the baseline.`,
  );
  protected readonly rows = computed(() => [...(this.diff.data()?.rows ?? []), ...this.extra()]);
  protected readonly more = computed(() => Math.max(0, (this.diff.data()?.totalRows ?? 0) - this.rows().length));
  /** Zoom breadcrumbs, named from the engine's answer (its focus frames). */
  protected readonly crumbs = computed(() => breadcrumbs(this.focus(), frameNamer(this.flame.data())));
  /** Drawing width; null until the view has been laid out (no request before). */
  private readonly widthPx = measuredWidth();

  private readonly common = computed(() => {
    const p = this.active.profile();
    const baselineId = this.state.baselineId();
    const metric = this.active.metric();
    return p && metric && baselineId !== null && baselineId !== p.id ? { baselineId, targetId: p.id, view: this.state.view(), metric } : null;
  });

  constructor() {
    // A different pair starts unzoomed.
    effect(() => {
      this.common();
      untracked(() => this.focus.set([]));
    });
    effect(() => {
      const c = this.common();
      const widthPx = this.widthPx();
      if (c && widthPx === null) return;
      const request = c && widthPx !== null ? { ...c, focus: this.focus(), widthPx } : null;
      untracked(() => {
        if (!request) return this.flame.reset();
        void this.flame.load(() => this.backend.diffFlame(request));
      });
    });
    effect(() => {
      const c = this.common();
      const request = c ? { ...c, sort: this.sort(), search: this.search() || null, offset: 0, limit: DIFF_PAGE } : null;
      untracked(() => {
        this.extra.set([]);
        if (!request) return this.diff.reset();
        void this.diff.load(() => this.backend.functionDiff(request));
      });
    });
    inject(DestroyRef).onDestroy(() => clearTimeout(this.searchTimer));
  }

  protected pickBaseline(e: Event): void {
    const v = (e.target as HTMLSelectElement).value;
    this.state.setBaseline(v ? Number(v) : null);
  }

  protected onSearch(e: Event): void {
    const value = (e.target as HTMLInputElement).value.trim();
    clearTimeout(this.searchTimer);
    this.searchTimer = setTimeout(() => this.search.set(value), 200);
  }

  protected async loadMore(): Promise<void> {
    const c = this.common();
    if (!c) return;
    try {
      const page = await this.backend.functionDiff({ ...c, sort: this.sort(), search: this.search() || null, offset: this.rows().length, limit: DIFF_PAGE });
      this.extra.update((x) => [...x, ...page.rows]);
    } catch {
      /* the failure is logged by the backend wrapper; the button stays for another try */
    }
  }

  protected value(v: number): string {
    const m = this.active.metric();
    return m ? formatMetric(v, m, this.active.profile()?.metrics) : '—';
  }

  protected flameLabel(name: string): string {
    return $localize`:@@profiling.compare.flameLabel:Differential flame graph of ${name}:name:`;
  }

  protected status(r: FunctionDiffRow): string {
    return STATUS_LABEL[r.status];
  }

  protected arrow(pp: number): string {
    return pp > 0.05 ? '▲' : pp < -0.05 ? '▼' : '';
  }
}
