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
import { AsyncResource } from './async-resource';

/** Message templates ranked by frequency, with a level bar; clicking one filters the workspace by it. */
@Component({
  selector: 'imt-patterns-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState],
  template: `
    <div class="bar">
      <span class="section-title">Patterns</span>
      @if (res.data(); as list) { <span class="faint">{{ list.length }} templates</span> }
      <span class="spacer"></span>
      <button class="btn" type="button" (click)="res.retry()">Refresh</button>
    </div>
    <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [hasData]="!!res.data()" [empty]="res.data()?.length === 0" emptyText="No events to group." (retry)="res.retry()">
      <ul class="list body" aria-label="Patterns">
        @for (p of res.data() ?? []; track p.id) {
          <li>
            <button class="row" type="button" (click)="filter(p)" [title]="'Filter by this pattern'">
              <span class="num count">{{ count(p.count) }}</span>
              <span class="bars" [style.width.px]="90" role="img" [attr.aria-label]="levelLabel(p)">
                @for (l of levels(p); track l.level) { <span [style.width.%]="share(l.count, p.count)" [style.background]="color(l.level)"></span> }
              </span>
              <span class="mono grow ellipsis" [title]="p.template">{{ p.template }}</span>
              <span class="mono muted range">{{ time.format(p.firstTs) }} → {{ time.format(p.lastTs) }}</span>
            </button>
            @if (p.sampleIds.length) {
              <button class="btn ghost sample" type="button" (click)="showSample(p)" [attr.aria-label]="'Show an example event of ' + p.template">Example</button>
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
      .bars { flex: none; }
      .range { flex: none; font-size: 11px; }
      li { display: flex; align-items: center; border-bottom: 1px solid var(--border); }
      li > .row { flex: 1; min-width: 0; border-bottom: 0; }
      .sample { flex: none; height: 22px; padding: 0 8px; margin-right: 8px; font-size: 11px; }
    `,
  ],
})
export class PatternsView {
  protected readonly res = new AsyncResource<Pattern[]>();
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

  protected share(n: number, total: number): number {
    return barWidth(n, total);
  }

  protected color(level: Level): string {
    return LEVEL_VARS[level];
  }

  protected levelLabel(p: Pattern): string {
    return levelCounts(p.levels).map((l) => `${l.level} ${l.count}`).join(', ') || 'no events';
  }

  protected filter(p: Pattern): void {
    this.actions.narrowQl(templateQl(p.template, p.source === 'Mined'));
  }

  protected showSample(p: Pattern): void {
    const id = p.sampleIds[0];
    if (id !== undefined) void this.actions.jumpTo(id);
  }
}
