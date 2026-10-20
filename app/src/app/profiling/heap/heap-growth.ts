import { ChangeDetectionStrategy, Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { HeapGrowthPage } from '../../core/api.types';
import { AnalysisState } from '../../analysis/analysis-state';
import { AsyncResource } from '../../analysis/async-resource';
import { CapturesStore } from '../captures.store';
import { ProfilingBackend } from '../profiling-backend';
import { formatBytes, formatDelta } from '../profiling-format';
import { PROFILING_STYLES } from '../profiling-styles';
import { byTaken } from './heap-diff';
import { HeapSelection, sparkline } from './heap-selection';

/** Growth across three or more snapshots: steady growers (leak candidates) first, with a trend line. */
@Component({
  selector: 'imt-heap-growth',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState],
  template: `
    <div class="bar">
      <fieldset class="snaps">
        <legend class="muted" i18n="@@profiling.heapGrowth.snapshots">Snapshots (oldest first, at least 3)</legend>
        @for (h of heaps(); track h.id) {
          <label class="inline"><input type="checkbox" [checked]="chosen().includes(h.id)" (change)="toggle(h.id)" /> {{ h.name }}</label>
        }
      </fieldset>
      <label class="inline" i18n="@@profiling.heapGrowth.steadyOnly"><input type="checkbox" [checked]="steadyOnly()" (change)="steadyOnly.set($any($event.target).checked)" /> Steady growth only</label>
      @if (res.data(); as d) { <span class="muted" role="status" i18n="@@profiling.heapGrowth.steadyCount">{d.steadyCount, plural, =1 {{{ d.steadyCount }} type grew in every snapshot} other {{{ d.steadyCount }} types grew in every snapshot}}</span> }
    </div>
    @if (chosen().length < 3) {
      <div class="state" role="status">{{ needMore() }}</div>
    } @else {
      <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [retryable]="res.retryable()" [hasData]="!!res.data()" [empty]="res.data()?.totalRows === 0" emptyText="No types grew." i18n-emptyText="@@profiling.heapGrowth.empty" (retry)="res.retry()">
        <div class="body tight" tabindex="0" role="region" aria-label="Growth rows" i18n-aria-label="@@profiling.heapGrowth.rowsRegion">
          <table class="grid" aria-label="Growth across snapshots" i18n-aria-label="@@profiling.heapGrowth.tableLabel">
            <thead><tr><th scope="col" i18n="@@profiling.heapGrowth.col.type">Type</th><th scope="col" i18n="@@profiling.heapGrowth.col.trend">Trend</th><th scope="col" i18n="@@profiling.heapGrowth.col.bytes">Bytes per snapshot</th><th scope="col" class="num" i18n="@@profiling.heapGrowth.col.growth">Growth</th><th scope="col" i18n="@@profiling.heapGrowth.col.flag">Flag</th></tr></thead>
            <tbody>
              @for (r of res.data()?.rows ?? []; track r.name) {
                <tr>
                  <td class="ellipsis fn"><button class="link" type="button" (click)="sel.showInstances(r.name)">{{ r.name }}</button></td>
                  <td><svg width="80" height="20" viewBox="0 0 80 20" role="img" [attr.aria-label]="trendLabel(r.name, r.bytes)"><polyline [attr.points]="spark(r.bytes)" fill="none" stroke-width="2" [class.steady]="r.steady" /></svg></td>
                  <td class="mono">{{ seriesText(r.bytes) }}</td>
                  <td class="num" [class.up]="r.growthBytes > 0" [class.down]="r.growthBytes < 0">{{ delta(r.growthBytes, 'bytes') }}</td>
                  <td>@if (r.steady) { <span class="tag steady" title="Never shrank and grew overall: a leak candidate" i18n-title="@@profiling.heapGrowth.steadyTitle" i18n="@@profiling.heapGrowth.steadyTag">⚠ steady growth</span> }</td>
                </tr>
              }
            </tbody>
          </table>
        </div>
      </imt-analysis-state>
    }
  `,
  styles: [
    PROFILING_STYLES,
    `
      .tight { padding: 0; }
      .fn { max-width: 380px; }
      .state { padding: 24px 12px; color: var(--muted); text-align: center; }
      .snaps { display: flex; gap: 10px; flex-wrap: wrap; border: 0; margin: 0; padding: 0; }
      .snaps legend { float: left; margin-right: 6px; padding: 0; }
      polyline { stroke: var(--s1); }
      polyline.steady { stroke: var(--lvl-error); }
    `,
  ],
})
export class HeapGrowth {
  protected readonly sel = inject(HeapSelection);
  private readonly captures = inject(CapturesStore);
  private readonly backend = inject(ProfilingBackend);
  protected readonly res = new AsyncResource<HeapGrowthPage>();
  protected readonly steadyOnly = signal(false);
  private readonly excluded = signal<ReadonlySet<number>>(new Set());
  protected readonly heaps = computed(() => byTaken(this.captures.heaps()));
  protected readonly chosen = computed(() => this.heaps().filter((h) => !this.excluded().has(h.id)).map((h) => h.id));
  protected readonly spark = (v: number[]) => sparkline(v);
  protected readonly delta = formatDelta;

  constructor() {
    effect(() => {
      const ids = this.chosen();
      const steadyOnly = this.steadyOnly();
      untracked(() => (ids.length >= 3 ? void this.res.load(() => this.backend.heapGrowth({ snapshotIds: ids, steadyOnly, limit: 500 })) : this.res.reset()));
    });
  }

  protected toggle(id: number): void {
    const next = new Set(this.excluded());
    if (next.has(id)) next.delete(id);
    else next.add(id);
    this.excluded.set(next);
  }

  protected readonly needMore = computed(() => {
    const missing = 3 - this.heaps().length;
    return missing > 0
      ? $localize`:@@profiling.heapGrowth.needMore:Growth needs at least three heap snapshots: open ${missing}:count: more.`
      : $localize`:@@profiling.heapGrowth.need:Growth needs at least three heap snapshots.`;
  });

  protected trendLabel(name: string, bytes: number[]): string {
    const series = this.seriesText(bytes);
    return $localize`:@@profiling.heapGrowth.trendLabel:Trend of ${name}:name:: ${series}:series:`;
  }

  protected seriesText(values: number[]): string {
    return values.map(formatBytes).join(' → ');
  }
}
