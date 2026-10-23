import { ChangeDetectionStrategy, Component, effect, inject, signal, untracked } from '@angular/core';
import { formatCount } from '../core/format';
import { Outlier, OutlierReport } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { AnalysisActions } from './analysis-actions';
import { formatLift, formatP } from './analysis-format';
import { AnalysisState } from './analysis-state';
import { ANALYSIS_STYLES } from './analysis-styles';
import { AnalysisTargets } from './analysis-targets';
import { AnalysisResource } from './async-resource';
import { CoverageNote } from './coverage-note';

/**
 * Error outliers (ADR 0045): the fields and values that are over-represented among errors (or any subset the user
 * names as a filter) against the rest of the same window, ranked by lift and significance. The engine computes
 * everything; a row filters the workspace to those events, and the evidence links open single events.
 */
@Component({
  selector: 'imt-outliers-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, CoverageNote],
  template: `
    <div class="bar">
      @if (res.result(); as r) {
        <span class="faint" i18n="@@analysis.outliers.summary">{{ count(r.subsetTotal) }} selected events against {{ count(r.restTotal) }} others</span>
      }
      <span class="spacer"></span>
      <label class="subset">
        <span class="muted" i18n="@@analysis.outliers.subsetLabel">Selected events</span>
        <input type="text" class="input mono" [value]="subset()" (input)="subset.set($any($event.target).value)" (keydown.enter)="apply()" placeholder="Errors (or a filter such as host = &quot;B&quot;)" i18n-placeholder="@@analysis.outliers.subsetPlaceholder" spellcheck="false" autocomplete="off" />
      </label>
      <button class="btn" type="button" (click)="apply()" i18n="@@analysis.outliers.apply">Compare</button>
    </div>
    <imt-coverage-note [coverage]="res.coverage()" />
    <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [retryable]="res.retryable()" [hasData]="!!res.result()" [empty]="res.result()?.outliers?.length === 0" emptyText="No value stands out among the selected events." i18n-emptyText="@@analysis.outliers.emptyText" emptyHint="Outliers need at least three selected events sharing a value that is clearly rarer in the rest. Try another window or a different selection." i18n-emptyHint="@@analysis.outliers.emptyHint" suggest (retry)="res.retry()">
      <ul class="list body" aria-label="Error outliers" i18n-aria-label="@@analysis.outliers.listLabel">
        @for (o of res.result()?.outliers ?? []; track o.query) {
          <li>
            <button class="row" type="button" (click)="filter(o)" title="Show these events" i18n-title="@@analysis.outliers.filter" [attr.data-testid]="'outlier-' + o.field">
              <span class="mono field ellipsis" [title]="o.field">{{ o.field }}</span>
              <span class="mono grow ellipsis" [title]="o.value">{{ o.value }}</span>
              <span class="num share" [attr.aria-label]="shareLabel(o)">{{ count(o.subsetCount) }} vs {{ count(o.restCount) }}</span>
              <span class="num lift up">{{ lift(o.lift) }}</span>
              <span class="muted p">{{ p(o.pValue) }}</span>
            </button>
            @if (o.sampleIds.length) {
              <span class="evidence">
                @for (id of o.sampleIds; track id) {
                  <button class="btn ghost ev" type="button" (click)="select(id)" [attr.aria-label]="eventLabel(id)">#{{ id }}</button>
                }
              </span>
            }
          </li>
        }
      </ul>
    </imt-analysis-state>
  `,
  styles: [
    ANALYSIS_STYLES,
    `
      .subset { display: inline-flex; align-items: center; gap: 6px; }
      .subset input { width: 280px; max-width: 40vw; }
      .list.body { padding: 0; }
      li { display: flex; align-items: center; border-bottom: 1px solid var(--border); }
      li > .row { flex: 1; min-width: 0; border-bottom: 0; }
      .field { flex: none; width: 160px; color: var(--chip-text); }
      .share { flex: none; width: 110px; }
      .lift { flex: none; width: 60px; }
      .p { flex: none; width: 90px; font-size: var(--fs-xs); }
      .evidence { display: flex; gap: 4px; flex: none; margin-right: 8px; }
      .ev { height: 22px; padding: 0 6px; font-size: var(--fs-xs); }
    `,
  ],
})
export class OutliersView {
  protected readonly res = new AnalysisResource<OutlierReport>();
  protected readonly count = formatCount;
  protected readonly lift = formatLift;
  protected readonly p = formatP;
  /** The subset filter being typed; empty selects the errors. */
  protected readonly subset = signal('');
  private readonly applied = signal('');
  private readonly backend = inject(AnalysisBackend);
  private readonly targets = inject(AnalysisTargets);
  private readonly actions = inject(AnalysisActions);

  constructor() {
    effect(() => {
      const query = this.targets.query();
      const subset = this.applied();
      untracked(() => void this.res.load(() => this.backend.queryOutliers(query, subset, [])));
    });
  }

  protected apply(): void {
    this.applied.set(this.subset().trim());
  }

  protected shareLabel(o: Outlier): string {
    return $localize`:@@analysis.outliers.shareLabel:${o.subsetCount}:selected: of the selected events and ${o.restCount}:rest: of the others carry this value`;
  }

  protected eventLabel(id: number): string {
    return $localize`:@@analysis.outliers.eventLabel:Show event ${id}:id:`;
  }

  protected filter(o: Outlier): void {
    this.actions.setQl(o.query);
  }

  protected select(id: number): void {
    void this.actions.jumpTo(id);
  }
}
