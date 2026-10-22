import { ChangeDetectionStrategy, Component, effect, inject, signal, untracked } from '@angular/core';
import { Level, Pattern, Placeholder } from '../core/api.types';
import { formatCount } from '../core/format';
import { AnalysisBackend } from '../core/analysis-backend';
import { LEVEL_VARS } from '../shared/colors';
import { AnalysisActions } from './analysis-actions';
import { barWidth, levelCounts, peakBucket, sparkBars, SparkBar, templateQl, templateValueQl } from './analysis-format';
import { AnalysisState } from './analysis-state';
import { ANALYSIS_STYLES } from './analysis-styles';
import { AnalysisTargets } from './analysis-targets';
import { AnalysisTime } from './analysis-time';
import { AnalysisResource } from './async-resource';
import { CoverageNote } from './coverage-note';

/** Message templates ranked by frequency, with a level bar; clicking one filters the workspace by it. */
@Component({
  selector: 'imt-patterns-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, CoverageNote],
  template: `
    <div class="bar">
      <span class="section-title" i18n="@@analysis.patterns.title">Patterns</span>
      @if (res.result(); as list) { <span class="faint" i18n="@@analysis.patterns.templates">{{ list.length }} templates</span> }
      <span class="spacer"></span>
      <button class="btn" type="button" (click)="res.retry()" i18n="@@analysis.patterns.refresh">Refresh</button>
    </div>
    <imt-coverage-note [coverage]="res.coverage()" />
    <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [retryable]="res.retryable()" [hasData]="!!res.result()" [empty]="res.result()?.length === 0" emptyText="No events to group." i18n-emptyText="@@analysis.patterns.emptyText" emptyHint="Patterns group the matching events by message template, most frequent first." i18n-emptyHint="@@analysis.patterns.emptyHint" suggest (retry)="res.retry()">
      <ul class="list body" aria-label="Patterns" i18n-aria-label="@@analysis.patterns.listLabel">
        @for (p of res.result() ?? []; track p.id) {
          <li>
            <button class="row" type="button" (click)="filter(p)" title="Filter by this pattern" i18n-title="@@analysis.patterns.filter">
              <span class="num count">{{ count(p.count) }}</span>
              <span class="bar-slot">
                <span class="bars" [style.width.px]="barPx(p)" role="img" [attr.aria-label]="levelLabel(p)">
                  @for (l of levels(p); track l.level) { <span [style.width.%]="share(l.count, p.count)" [style.background]="color(l.level)"></span> }
                </span>
              </span>
              @if (p.trend.length) {
                <svg class="spark" width="72" height="18" viewBox="0 0 72 18" role="img" [attr.aria-label]="trendLabel(p)" data-testid="pattern-trend">
                  @for (b of bars(p); track b.x) { <rect [attr.x]="b.x" [attr.y]="b.y" [attr.width]="b.width" [attr.height]="b.height" /> }
                </svg>
              } @else {
                <span class="spark-slot"></span>
              }
              <span class="mono grow ellipsis" [title]="p.template">{{ p.template }}</span>
              <span class="mono muted range">{{ time.format(p.firstTs) }} → {{ time.format(p.lastTs) }}</span>
            </button>
            @if (p.placeholders.length) {
              <button class="btn ghost values-btn" type="button" (click)="toggleValues(p)" [attr.aria-expanded]="isOpen(p)" [attr.aria-label]="valuesLabel(p.template)" i18n="@@analysis.patterns.values">Values</button>
            }
            @if (p.sampleIds.length) {
              <button class="btn ghost sample" type="button" (click)="showSample(p)" [attr.aria-label]="exampleLabel(p.template)" i18n="@@analysis.patterns.example">Example</button>
            }
            @if (isOpen(p)) {
              <div class="values" data-testid="pattern-values">
                @for (h of p.placeholders; track h.index) {
                  <section class="ph" [attr.aria-label]="placeholderLabel(h)">
                    <h3 class="sec" i18n="@@analysis.patterns.placeholder">Value {{ h.index + 1 }} · {{ count(h.total) }} events</h3>
                    <ul class="list">
                      @for (v of h.values; track v.value) {
                        <li class="vrow">
                          <button class="link mono ellipsis" type="button" (click)="filterValue(p, v.value)" [title]="v.value" [attr.aria-label]="filterValueLabel(v.value)">{{ v.value }}</button>
                          <span class="num">{{ count(v.count) }}</span>
                        </li>
                      }
                    </ul>
                    @if (!h.exact) { <p class="muted note" i18n="@@analysis.patterns.approximate">Many distinct values: the most frequent are listed and their counts are approximate.</p> }
                  </section>
                }
              </div>
            }
          </li>
        }
      </ul>
    </imt-analysis-state>
  `,
  styles: [
    ANALYSIS_STYLES,
    `
      .list.body { padding: 0; }
      .count { width: 56px; flex: none; color: var(--text); }
      .bars { flex: none; min-width: 0; }
      .bar-slot { width: 90px; flex: none; display: flex; }
      .range { flex: none; font-size: var(--fs-xs); }
      li { display: flex; align-items: center; flex-wrap: wrap; border-bottom: 1px solid var(--border); }
      .spark, .spark-slot { flex: none; width: 72px; height: 18px; }
      .spark rect { fill: var(--accent); }
      .values { flex-basis: 100%; display: flex; flex-wrap: wrap; gap: 4px 24px; padding: 0 12px 8px 24px; }
      .ph { min-width: 220px; max-width: 360px; }
      .vrow { display: flex; gap: 12px; padding: 1px 0; border: 0; }
      .vrow .link { flex: 1; min-width: 0; }
      .note { margin: 2px 0 0; font-size: var(--fs-xs); }
      li > .row { flex: 1; min-width: 0; border-bottom: 0; }
      .sample, .values-btn { flex: none; height: 22px; padding: 0 8px; margin-right: 8px; font-size: var(--fs-xs); }
    `,
  ],
})
export class PatternsView {
  protected exampleLabel(template: string): string {
    return $localize`:@@analysis.patterns.exampleLabel:Show an example event of ${template}:template:`;
  }
  protected readonly res = new AnalysisResource<Pattern[]>();
  protected readonly time = inject(AnalysisTime);
  protected readonly count = formatCount;
  /** Patterns whose values are shown, by id. */
  protected readonly open = signal<ReadonlySet<number>>(new Set());
  private readonly backend = inject(AnalysisBackend);
  private readonly targets = inject(AnalysisTargets);
  private readonly actions = inject(AnalysisActions);

  constructor() {
    effect(() => {
      const query = this.targets.query();
      untracked(() => void this.res.load(() => this.backend.queryPatterns(query)));
    });
  }

  protected bars(p: Pattern): SparkBar[] {
    return sparkBars(p.trend, 72, 18);
  }

  protected trendLabel(p: Pattern): string {
    const peak = peakBucket(p.trend);
    const at = p.trendFrom !== null && peak >= 0 ? this.time.format(p.trendFrom + peak * p.trendBucketMs) : '';
    return $localize`:@@analysis.patterns.trendLabel:Occurrences over time, busiest around ${at}:time:`;
  }

  protected isOpen(p: Pattern): boolean {
    return this.open().has(p.id);
  }

  protected toggleValues(p: Pattern): void {
    const next = new Set(this.open());
    if (!next.delete(p.id)) next.add(p.id);
    this.open.set(next);
  }

  protected valuesLabel(template: string): string {
    return $localize`:@@analysis.patterns.valuesLabel:Show the values behind the placeholders of ${template}:template:`;
  }

  protected placeholderLabel(h: Placeholder): string {
    const n = h.index + 1;
    return $localize`:@@analysis.patterns.placeholderLabel:Values of placeholder ${n}:n:`;
  }

  protected filterValueLabel(value: string): string {
    return $localize`:@@analysis.patterns.filterValueLabel:Filter by the value ${value}:value:`;
  }

  protected filterValue(p: Pattern, value: string): void {
    this.actions.narrowQl(templateValueQl(p.template, p.source === 'Mined', value));
  }

  protected levels(p: Pattern) {
    return levelCounts(p.levels);
  }

  /** Bar length relative to the most frequent pattern (90px), at least 4px so rare patterns stay visible. */
  protected barPx(p: Pattern): number {
    const max = Math.max(0, ...(this.res.result() ?? []).map((x) => x.count));
    return max > 0 ? Math.max(4, Math.round((90 * p.count) / max)) : 4;
  }

  protected share(n: number, total: number): number {
    return barWidth(n, total);
  }

  protected color(level: Level): string {
    return LEVEL_VARS[level];
  }

  protected levelLabel(p: Pattern): string {
    return levelCounts(p.levels).map((l) => `${l.level} ${l.count}`).join(', ') || $localize`:@@analysis.patterns.noEvents:no events`;
  }

  protected filter(p: Pattern): void {
    this.actions.narrowQl(templateQl(p.template, p.source === 'Mined'));
  }

  protected showSample(p: Pattern): void {
    const id = p.sampleIds[0];
    if (id !== undefined) void this.actions.jumpTo(id);
  }
}
