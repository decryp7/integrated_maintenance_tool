import { ChangeDetectionStrategy, Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { Anomaly } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { AnalysisActions } from './analysis-actions';
import { anomalyIcon, anomalyLabel } from './analysis-format';
import { AnomalyColumn, AnomalySort, DEFAULT_ANOMALY_SORT, nextSort, sortAnomalies } from './anomaly-sort';
import { AnalysisState } from './analysis-state';
import { ANALYSIS_STYLES } from './analysis-styles';
import { AnalysisTargets } from './analysis-targets';
import { AnalysisTime } from './analysis-time';
import { AnalysisResource } from './async-resource';
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
      @if (res.result(); as list) { <span class="faint" i18n="@@analysis.anomalies.found">{{ list.length }} found</span> }
      <span class="spacer"></span>
      <button class="btn" type="button" (click)="res.retry()" i18n="@@analysis.anomalies.refresh">Refresh</button>
    </div>
    <imt-coverage-note [coverage]="res.coverage()" />
    <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [retryable]="res.retryable()" [hasData]="!!res.result()" [empty]="res.result()?.length === 0" emptyText="No anomalies detected in the current query." i18n-emptyText="@@analysis.anomalies.emptyText" emptyHint="IMT looks for crashes, error spikes, silences, slow outliers, rate changes and memory growth in the events the query matches." i18n-emptyHint="@@analysis.anomalies.emptyHint" suggest (retry)="res.retry()">
      <div class="head" role="group" aria-label="Sort anomalies by" i18n-aria-label="@@analysis.anomalies.sortLabel" data-testid="anomaly-head">
        <span class="kind"></span>
        @for (c of columns; track c.id) {
          <button type="button" class="th" [class]="'th ' + c.id" [attr.aria-pressed]="sort().column === c.id" [attr.data-testid]="'anomaly-sort-' + c.id" [attr.aria-label]="c.sortLabel" (click)="sortBy(c.id)">
            {{ c.label }} <span aria-hidden="true">{{ arrow(c.id) }}</span>
          </button>
        }
      </div>
      <ul class="list body" aria-label="Anomalies" i18n-aria-label="@@analysis.anomalies.listLabel">
        @for (a of sorted(); track a.kind + ':' + a.instance + ':' + a.from + ':' + a.to) {
          <li>
            <button class="row" type="button" (click)="open(a)" (mouseenter)="cursor.hover(a.from)" (focus)="cursor.hover(a.from)" (mouseleave)="cursor.hover(null)" (blur)="cursor.hover(null)">
              <span class="kind" aria-hidden="true">{{ icon(a.kind) }}</span>
              <span class="klabel">{{ label(a.kind) }}</span>
              <span class="inst ellipsis" [title]="a.instance ?? ''">{{ a.instance ?? allInstances }}</span>
              <span class="mono when">{{ time.format(a.from) }}{{ a.to > a.from ? ' → ' + time.format(a.to) : '' }}</span>
              <span class="grow msg" [title]="a.message">{{ a.message }}</span>
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
      .msg { white-space: normal; overflow-wrap: anywhere; min-width: 0; text-align: start; }
      .head { display: flex; align-items: center; gap: 10px; padding: 2px 12px; border-bottom: 1px solid var(--border); }
      .th { border: 0; background: transparent; color: var(--muted); font: inherit; font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: 0.06em; font-weight: 600; cursor: pointer; text-align: start; padding: 2px 0; }
      .th[aria-pressed='true'] { color: var(--text); }
      .th:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
      .th.kind { width: 96px; flex: none; }
      .th.instance { width: 170px; flex: none; }
      .th.time { width: 250px; flex: none; }
      .th.message { flex: 1; }
      .th.score { width: 48px; flex: none; text-align: right; }
    `,
  ],
})
export class AnomaliesView {
  protected readonly res = new AnalysisResource<Anomaly[]>();
  protected readonly time = inject(AnalysisTime);
  protected readonly cursor = inject(TimeCursor);
  protected readonly allInstances = $localize`:@@analysis.anomalies.allInstances:all instances`;
  protected readonly icon = anomalyIcon;
  protected readonly label = anomalyLabel;
  protected readonly sort = signal<AnomalySort>(DEFAULT_ANOMALY_SORT);
  protected readonly sorted = computed(() => sortAnomalies(this.res.result() ?? [], this.sort()));
  protected readonly columns: readonly { id: AnomalyColumn; label: string; sortLabel: string }[] = [
    { id: 'kind', label: $localize`:@@analysis.anomalies.col.kind:Kind`, sortLabel: $localize`:@@analysis.anomalies.sort.kind:Sort by kind` },
    { id: 'instance', label: $localize`:@@analysis.anomalies.col.instance:Instance`, sortLabel: $localize`:@@analysis.anomalies.sort.instance:Sort by instance` },
    { id: 'time', label: $localize`:@@analysis.anomalies.col.time:Time`, sortLabel: $localize`:@@analysis.anomalies.sort.time:Sort by time` },
    { id: 'message', label: $localize`:@@analysis.anomalies.col.what:What`, sortLabel: $localize`:@@analysis.anomalies.sort.what:Sort by description` },
    { id: 'score', label: $localize`:@@analysis.anomalies.col.score:Score`, sortLabel: $localize`:@@analysis.anomalies.sort.score:Sort by score` },
  ];
  private readonly backend = inject(AnalysisBackend);
  protected readonly targets = inject(AnalysisTargets);
  private readonly actions = inject(AnalysisActions);

  constructor() {
    effect(() => {
      const query = this.targets.query();
      untracked(() => void this.res.load(() => this.backend.queryAnomalies(query)));
    });
  }

  protected sortBy(column: AnomalyColumn): void {
    this.sort.update((s) => nextSort(s, column));
  }

  protected arrow(column: AnomalyColumn): string {
    const s = this.sort();
    return s.column !== column ? '' : s.descending ? '▼' : '▲';
  }

  protected open(a: Anomaly): void {
    // Zoom first, then select the first sample under the zoomed query so the grid can show it.
    void this.actions.revealSample({ from: a.from, to: Math.max(a.to, a.from + 1) }, a.sampleIds[0]);
  }
}
