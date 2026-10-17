import { ChangeDetectionStrategy, Component, computed, effect, inject, untracked } from '@angular/core';
import { LEVELS, Level, MomentSummary, UserCount } from '../core/api.types';
import { formatCount, formatNumber } from '../core/format';
import { AnalysisBackend } from '../core/analysis-backend';
import { AnalysisActions } from './analysis-actions';
import { AnalysisState } from './analysis-state';
import { ANALYSIS_STYLES } from './analysis-styles';
import { AnalysisTargets } from './analysis-targets';
import { AnalysisTime } from './analysis-time';
import { anomalyIcon, anomalyLabel, ARROWS, direction, formatChange } from './analysis-format';
import { AsyncResource } from './async-resource';
import { qlEquals } from './ql-text';
import { TimeCursor } from './time-cursor';

/** Everything that happened in a time window: levels, instances, metrics, users, activity and anomalies. */
@Component({
  selector: 'imt-moment-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState],
  template: `
    <div class="bar">
      <span class="section-title">Moment</span>
      <span class="mono">{{ time.range(range()) }}</span>
      <span class="spacer"></span>
      <button class="btn" type="button" [disabled]="!range()" (click)="zoom()">Zoom to this range</button>
    </div>
    <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [hasData]="!!res.data()" [empty]="!range()" emptyText="Select a time range (or load a workspace) to analyse a moment." (retry)="res.retry()">
      @if (res.data(); as m) {
        <div class="body" tabindex="0" role="region" aria-label="Moment details">
          <div class="summary">
            <strong class="total">{{ count(m.total) }} events</strong>
            @for (l of levelChips(m); track l.level) {
              <span class="lvl" [class]="'lvl ' + l.level">{{ l.level }} {{ count(l.count) }}</span>
            }
          </div>

          <h3 class="sec">Instances</h3>
          @if (m.instances.length) {
            <div class="cards">
              @for (i of m.instances; track i.key) {
                <section class="card" [attr.aria-label]="'Instance ' + i.key">
                  <div class="head"><strong class="ellipsis" [title]="i.key">{{ i.key }}</strong></div>
                  <div class="counts mono">
                    <span>{{ count(i.events) }} events</span>
                    <span class="warn">{{ count(i.warnings) }} warnings</span>
                    <span [class.errs]="i.errors">{{ count(i.errors) }} errors</span>
                  </div>
                  <ul class="list pats">
                    @for (p of i.patterns; track p.id) {
                      <li><span class="num">{{ count(p.count) }}</span><span class="ellipsis" [title]="p.template">{{ p.template }}</span></li>
                    }
                  </ul>
                  @if (i.firstError !== null) {
                    <button class="btn" type="button" (click)="select(i.firstError)">Go to first error</button>
                  }
                </section>
              }
            </div>
          } @else {
            <p class="muted">No events in this window.</p>
          }

          <h3 class="sec">Metrics</h3>
          @if (m.metrics.length) {
            <table class="grid">
              <thead>
                <tr><th scope="col">Series</th><th scope="col">Metric</th><th scope="col" class="num">Min</th><th scope="col" class="num">Avg</th><th scope="col" class="num">Max</th><th scope="col" class="num">Last</th><th scope="col" class="num">Previous avg</th><th scope="col" class="num">Change</th></tr>
              </thead>
              <tbody>
                @for (x of m.metrics; track x.series + x.metric) {
                  <tr>
                    <td>{{ x.series }}</td><td>{{ x.metric }}</td>
                    <td class="num">{{ num(x.min) }}</td><td class="num">{{ num(x.avg) }}</td><td class="num">{{ num(x.max) }}</td><td class="num">{{ num(x.last) }}</td>
                    <td class="num">{{ x.previousAvg === null ? '—' : num(x.previousAvg) }}</td>
                    <td class="num" [class]="'num ' + dir(x.changePct)"><span aria-hidden="true">{{ arrow(x.changePct) }}</span> {{ change(x.changePct) }}</td>
                  </tr>
                }
              </tbody>
            </table>
          } @else {
            <p class="muted">No numeric metrics in this window.</p>
          }

          <h3 class="sec">Users</h3>
          @if (m.users.length) {
            <div class="chips" role="group" aria-label="Users active in this window">
              @for (u of m.users; track u.field + '=' + u.value) {
                <button class="chip" type="button" (click)="filterUser(u)" [title]="'Filter by ' + u.field + ' = ' + u.value">{{ u.value }} <span class="mono">{{ count(u.count) }}</span></button>
              }
            </div>
          } @else {
            <p class="muted">No user information in this window.</p>
          }

          <h3 class="sec">User activity</h3>
          @if (m.activity.length) {
            <ul class="list activity">
              @for (a of m.activity; track a.id) {
                <li>
                  <button class="row" type="button" (click)="select(a.id)" (mouseenter)="cursor.hover(a.ts)" (focus)="cursor.hover(a.ts)" (mouseleave)="cursor.hover(null)" (blur)="cursor.hover(null)">
                    <span class="mono time">{{ time.format(a.ts) }}</span>
                    <span class="ellipsis inst">{{ a.instance }}</span>
                    <span class="ellipsis user">{{ a.user ?? '—' }}</span>
                    <span class="ellipsis grow">{{ a.action }}</span>
                  </button>
                </li>
              }
            </ul>
          } @else {
            <p class="muted">No user actions recorded.</p>
          }

          <h3 class="sec">Anomalies in this window</h3>
          @if (m.anomalies.length) {
            <ul class="list">
              @for (a of m.anomalies; track $index) {
                <li>
                  <button class="row" type="button" (click)="selectSample(a.sampleIds[0])">
                    <span aria-hidden="true">{{ icon(a.kind) }}</span><strong>{{ label(a.kind) }}</strong>
                    <span class="muted ellipsis">{{ a.instance ?? 'all instances' }}</span><span class="grow ellipsis">{{ a.message }}</span>
                  </button>
                </li>
              }
            </ul>
          } @else {
            <p class="muted">None.</p>
          }
        </div>
      }
    </imt-analysis-state>
  `,
  styles: [
    ANALYSIS_STYLES,
    `
      .summary { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
      .total { font-size: 14px; color: var(--text); margin-right: 6px; }
      .head { margin-bottom: 4px; color: var(--text); }
      .counts { display: flex; gap: 12px; font-size: 11px; margin-bottom: 6px; }
      .warn { color: var(--warn-text); }
      .errs { color: var(--danger-text); }
      .pats li { display: flex; gap: 8px; padding: 2px 0; font-size: 11px; }
      .pats .num { width: 36px; flex: none; }
      .chips { display: flex; flex-wrap: wrap; gap: 6px; }
      .chips .chip { cursor: pointer; color: var(--chip-text); }
      .time { width: 120px; flex: none; }
      .inst { width: 160px; flex: none; }
      .user { width: 110px; flex: none; }
      .up, .down, .flat { font-weight: 600; }
    `,
  ],
})
export class MomentView {
  protected readonly actions = inject(AnalysisActions);
  protected readonly time = inject(AnalysisTime);
  protected readonly cursor = inject(TimeCursor);
  private readonly backend = inject(AnalysisBackend);
  private readonly targets = inject(AnalysisTargets);

  protected readonly res = new AsyncResource<MomentSummary>();
  protected readonly range = computed(() => this.targets.moment() ?? this.targets.currentRange());

  protected readonly count = formatCount;
  protected readonly num = formatNumber;
  protected readonly change = formatChange;
  protected readonly icon = anomalyIcon;
  protected readonly label = anomalyLabel;

  constructor() {
    effect(() => {
      const range = this.range();
      const query = this.targets.query();
      if (!range) return untracked(() => this.res.reset());
      untracked(() => void this.res.load(() => this.backend.queryMoment(query, range.from, range.to)));
    });
  }

  protected levelChips(m: MomentSummary): { level: Level; count: number }[] {
    return LEVELS.map((level) => ({ level, count: m.levels[level] ?? 0 })).filter((l) => l.count > 0);
  }

  protected dir(pct: number | null): string {
    return direction(pct);
  }

  protected arrow(pct: number | null): string {
    return ARROWS[direction(pct)];
  }

  protected zoom(): void {
    const r = this.range();
    if (r) this.actions.zoomTo(r);
  }

  /** Narrows the workspace to the user's events (ANDed with the current query). */
  protected filterUser(u: UserCount): void {
    this.actions.narrowQl(qlEquals(u.field, u.value));
  }

  protected select(id: number): void {
    void this.actions.jumpTo(id);
  }

  protected selectSample(id: number | undefined): void {
    if (id !== undefined) this.select(id);
  }
}
