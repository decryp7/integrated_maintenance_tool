import { ChangeDetectionStrategy, Component, computed, effect, inject, untracked } from '@angular/core';
import type { EChartsCoreOption } from 'echarts/core';
import { ChartTheme, ChartTokens } from '../metrics/chart-theme';
import { EChart } from '../metrics/echart';
import { formatCount, formatNumber } from '../core/format';
import { FieldStats, NumericStats } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { ImtEvents } from '../extensions/events';
import { AnalysisActions } from './analysis-actions';
import { barWidth } from './analysis-format';
import { AnalysisState } from './analysis-state';
import { ANALYSIS_STYLES } from './analysis-styles';
import { AnalysisTargets } from './analysis-targets';
import { AsyncResource } from './async-resource';

export const HISTOGRAM_BUCKETS = 20;

/** ECharts options for a numeric histogram (bars per engine bin). */
export function histogramOption(stats: NumericStats, t: ChartTokens): EChartsCoreOption {
  const labels = stats.histogram.map((b) => `${formatNumber(b.from)} – ${formatNumber(b.to)}`);
  return {
    animation: false,
    textStyle: { color: t.muted, fontFamily: t.fontSans },
    grid: { left: 44, right: 12, top: 12, bottom: 28 },
    tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' }, backgroundColor: t.tooltipBg, borderColor: t.border, textStyle: { color: t.text } },
    xAxis: { type: 'category', data: labels, axisLabel: { color: t.muted, hideOverlap: true }, axisLine: { lineStyle: { color: t.border } } },
    yAxis: { type: 'value', axisLabel: { color: t.muted }, splitLine: { lineStyle: { color: t.grid } } },
    series: [{ type: 'bar', data: stats.histogram.map((b) => b.count), itemStyle: { color: t.series[0] }, barCategoryGap: '8%' }],
  };
}

/** Distribution of one field: numeric histogram with percentiles, and the most common values. */
@Component({
  selector: 'imt-field-stats-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, EChart],
  template: `
    <div class="bar">
      <span class="section-title">Field statistics</span>
      <label class="lbl">Field
        <select class="select mono" aria-label="Field" (change)="targets.setField($any($event.target).value || null)">
          <option value="" [selected]="!targets.field()">Choose a field</option>
          @for (f of fields(); track f) { <option [value]="f" [selected]="f === targets.field()">{{ f }}</option> }
        </select>
      </label>
    </div>
    <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [hasData]="!!res.data()" [empty]="!targets.field()" emptyText="Choose a field to see its distribution." (retry)="res.retry()">
      @if (res.data(); as s) {
        <div class="body" tabindex="0" role="region" aria-label="Field statistics">
          <p class="facts mono">{{ count(s.count) }} events with the field · {{ count(s.missing) }} without · {{ count(s.distinct) }} distinct</p>
          @if (s.numeric; as n) {
            <h3 class="sec first">Distribution</h3>
            <div class="chart" role="img" [attr.aria-label]="'Histogram of ' + targets.field() + ' from ' + num(n.min) + ' to ' + num(n.max)">
              <imt-echart [options]="option()!" />
            </div>
            <dl class="pcts mono">
              @for (p of percentiles(n); track p[0]) { <div><dt>{{ p[0] }}</dt><dd>{{ num(p[1]) }}</dd></div> }
            </dl>
          }
          <h3 class="sec">Top values</h3>
          @if (s.top.length) {
            <ul class="list">
              @for (v of s.top; track v.value) {
                <li>
                  <button class="row top" type="button" (click)="filter(v.value)" [title]="'Filter ' + targets.field() + ' = ' + v.value">
                    <span class="val mono ellipsis">{{ v.value }}</span>
                    <span class="track"><span class="fill" [style.width.%]="share(v.count, s.top[0].count)"></span></span>
                    <span class="num">{{ count(v.count) }}</span>
                  </button>
                </li>
              }
            </ul>
          } @else { <p class="muted">No values.</p> }
        </div>
      }
    </imt-analysis-state>
  `,
  styles: [
    ANALYSIS_STYLES,
    `
      .lbl { display: inline-flex; align-items: center; gap: 6px; color: var(--muted); }
      .facts { margin: 0 0 4px; }
      .sec.first { margin-top: 8px; }
      .chart { height: 220px; border: 1px solid var(--border); border-radius: 6px; padding: 4px; }
      imt-echart { height: 100%; }
      .pcts { display: flex; gap: 18px; flex-wrap: wrap; margin: 8px 0 0; }
      .pcts div { display: flex; gap: 6px; }
      .pcts dt { color: var(--muted); }
      .pcts dd { margin: 0; color: var(--text); }
      .top { padding: 4px 6px; }
      .val { width: 240px; flex: none; }
      .track { flex: 1; height: 8px; background: var(--lane-bar); border-radius: 4px; overflow: hidden; max-width: 320px; }
      .fill { display: block; height: 100%; background: var(--s1); }
      .num { width: 70px; flex: none; }
    `,
  ],
})
export class FieldStatsView {
  protected readonly targets = inject(AnalysisTargets);
  protected readonly res = new AsyncResource<FieldStats>();
  protected readonly count = formatCount;
  protected readonly num = formatNumber;
  private readonly events = inject(ImtEvents);
  private readonly backend = inject(AnalysisBackend);
  private readonly actions = inject(AnalysisActions);
  private readonly theme = inject(ChartTheme);

  protected readonly fields = computed(() => (this.events.workspace()?.fields ?? []).map((f) => f.name));
  protected readonly option = computed(() => {
    const n = this.res.data()?.numeric;
    return n ? histogramOption(n, this.theme.tokens()) : null;
  });

  constructor() {
    effect(() => {
      const field = this.targets.field();
      const query = this.targets.query();
      if (!field) return untracked(() => this.res.reset());
      untracked(() => void this.res.load(() => this.backend.fieldStats(query, field, HISTOGRAM_BUCKETS)));
    });
  }

  protected percentiles(n: NumericStats): [string, number][] {
    return [['min', n.min], ['p50', n.p50], ['p90', n.p90], ['p95', n.p95], ['p99', n.p99], ['max', n.max], ['mean', n.mean], ['σ', n.stddev]];
  }

  protected share(n: number, max: number): number {
    return barWidth(n, max);
  }

  protected filter(value: string): void {
    const field = this.targets.field();
    if (field) this.actions.filterField(field, value);
  }
}
