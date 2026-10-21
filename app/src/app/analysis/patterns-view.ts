import { ChangeDetectionStrategy, Component, effect, inject, untracked } from '@angular/core';
import { Level, Pattern } from '../core/api.types';
import { formatCount } from '../core/format';
import { AnalysisBackend } from '../core/analysis-backend';
import { LEVEL_VARS } from '../shared/colors';
import { AnalysisActions } from './analysis-actions';
import { barWidth, levelCounts, templateQl } from './analysis-format';
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
              <span class="mono grow ellipsis" [title]="p.template">{{ p.template }}</span>
              <span class="mono muted range">{{ time.format(p.firstTs) }} → {{ time.format(p.lastTs) }}</span>
            </button>
            @if (p.sampleIds.length) {
              <button class="btn ghost sample" type="button" (click)="showSample(p)" [attr.aria-label]="exampleLabel(p.template)" i18n="@@analysis.patterns.example">Example</button>
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
      li { display: flex; align-items: center; border-bottom: 1px solid var(--border); }
      li > .row { flex: 1; min-width: 0; border-bottom: 0; }
      .sample { flex: none; height: 22px; padding: 0 8px; margin-right: 8px; font-size: var(--fs-xs); }
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
  private readonly backend = inject(AnalysisBackend);
  private readonly targets = inject(AnalysisTargets);
  private readonly actions = inject(AnalysisActions);

  constructor() {
    effect(() => {
      const query = this.targets.query();
      untracked(() => void this.res.load(() => this.backend.queryPatterns(query)));
    });
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
