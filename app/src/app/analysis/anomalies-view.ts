import { ChangeDetectionStrategy, Component, effect, inject, untracked } from '@angular/core';
import { Anomaly } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { AnalysisActions } from './analysis-actions';
import { anomalyIcon, anomalyLabel } from './analysis-format';
import { AnalysisState } from './analysis-state';
import { ANALYSIS_STYLES } from './analysis-styles';
import { AnalysisTargets } from './analysis-targets';
import { AnalysisTime } from './analysis-time';
import { AsyncResource } from './async-resource';
import { CoverageNote } from './coverage-note';
import { TimeCursor } from './time-cursor';

/** Findings of the engine's anomaly detectors; clicking one zooms to its range and selects a sample event. */
@Component({
  selector: 'imt-anomalies-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, CoverageNote],
  template: `
    <div class="bar">
      <span class="section-title" i18n="@@analysis.anomalies.title">Anomalies</span>
      @if (res.data(); as list) { <span class="faint" i18n="@@analysis.anomalies.found">{{ list.length }} found</span> }
      <span class="spacer"></span>
      <button class="btn" type="button" (click)="res.retry()" i18n="@@analysis.anomalies.refresh">Refresh</button>
    </div>
    <imt-coverage-note [query]="targets.query()" />
    <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [retryable]="res.retryable()" [hasData]="!!res.data()" [empty]="res.data()?.length === 0" emptyText="No anomalies detected in the current query." i18n-emptyText="@@analysis.anomalies.emptyText" emptyHint="IMT looks for crashes, error spikes, silences, slow outliers, rate changes and memory growth in the events the query matches." i18n-emptyHint="@@analysis.anomalies.emptyHint" suggest (retry)="res.retry()">
      <ul class="list body" aria-label="Anomalies" i18n-aria-label="@@analysis.anomalies.listLabel">
        @for (a of res.data() ?? []; track a.kind + ':' + a.instance + ':' + a.from + ':' + a.to) {
          <li>
            <button class="row" type="button" (click)="open(a)" (mouseenter)="cursor.hover(a.from)" (focus)="cursor.hover(a.from)" (mouseleave)="cursor.hover(null)" (blur)="cursor.hover(null)">
              <span class="kind" aria-hidden="true">{{ icon(a.kind) }}</span>
              <span class="klabel">{{ label(a.kind) }}</span>
              <span class="inst ellipsis" [title]="a.instance ?? ''">{{ a.instance ?? allInstances }}</span>
              <span class="mono when">{{ time.format(a.from) }}{{ a.to > a.from ? ' → ' + time.format(a.to) : '' }}</span>
              <span class="grow ellipsis" [title]="a.message">{{ a.message }}</span>
              <span class="mono score" title="Score" i18n-title="@@analysis.anomalies.score">{{ a.score }}</span>
            </button>
          </li>
        }
      </ul>
    </imt-analysis-state>
  `,
  styles: [
    ANALYSIS_STYLES,
    `
      .list.body { padding: 0; }
      .kind { width: 16px; text-align: center; color: var(--danger-text); }
      .klabel { width: 96px; flex: none; font-weight: 600; color: var(--text); }
      .inst { width: 170px; flex: none; }
      .when { width: 250px; flex: none; color: var(--muted); }
      .score { width: 48px; flex: none; text-align: right; }
    `,
  ],
})
export class AnomaliesView {
  protected readonly res = new AsyncResource<Anomaly[]>();
  protected readonly time = inject(AnalysisTime);
  protected readonly cursor = inject(TimeCursor);
  protected readonly allInstances = $localize`:@@analysis.anomalies.allInstances:all instances`;
  protected readonly icon = anomalyIcon;
  protected readonly label = anomalyLabel;
  private readonly backend = inject(AnalysisBackend);
  protected readonly targets = inject(AnalysisTargets);
  private readonly actions = inject(AnalysisActions);

  constructor() {
    effect(() => {
      const query = this.targets.query();
      untracked(() => void this.res.load(() => this.backend.queryAnomalies(query)));
    });
  }

  protected open(a: Anomaly): void {
    // Zoom first, then select the first sample under the zoomed query so the grid can show it.
    void this.actions.revealSample({ from: a.from, to: Math.max(a.to, a.from + 1) }, a.sampleIds[0]);
  }
}
