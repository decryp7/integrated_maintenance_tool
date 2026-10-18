import { ChangeDetectionStrategy, Component, computed, effect, inject, untracked } from '@angular/core';
import { AiButton } from '../ai/ai-button';
import { AiLauncher } from '../ai/ai-launcher';
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
import { ProfileMomentSection } from '../profiling/integration/profile-moment';

/** Everything that happened in a time window: levels, instances, metrics, users, activity and anomalies. */
@Component({
  selector: 'imt-moment-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, ProfileMomentSection, AiButton],
  template: `
    <div class="bar">
      <span class="section-title" i18n="@@analysis.moment.title">Moment</span>
      <span class="mono">{{ time.range(range()) }}</span>
      <span class="spacer"></span>
      <button class="btn" type="button" [disabled]="!range()" (click)="zoom()" i18n="@@analysis.moment.zoom">Zoom to this range</button>
      <imt-ai-button label="Summarize" i18n-label="@@analysis.moment.summarize" hint="The local model summarizes what happened in this range" i18n-hint="@@analysis.moment.summarizeHint" testId="ai-summarize" [disabled]="!range()" (pressed)="summarize()" />
    </div>
    <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [hasData]="!!res.data()" [empty]="!range()" emptyText="Select a time range (or load a workspace) to analyse a moment." i18n-emptyText="@@analysis.moment.emptyText" (retry)="res.retry()">
      @if (res.data(); as m) {
        <div class="body" tabindex="0" role="region" aria-label="Moment details" i18n-aria-label="@@analysis.moment.region">
          <div class="summary">
            <strong class="total" i18n="@@analysis.moment.total">{{ count(m.total) }} events</strong>
            @for (l of levelChips(m); track l.level) {
              <span class="lvl" [class]="'lvl ' + l.level">{{ l.level }} {{ count(l.count) }}</span>
            }
          </div>

          <h3 class="sec" i18n="@@analysis.moment.instances">Instances</h3>
          @if (m.instances.length) {
            <div class="cards">
              @for (i of m.instances; track i.key) {
                <section class="card" [attr.aria-label]="instanceLabel(i.key)">
                  <div class="head"><strong class="ellipsis" [title]="i.key">{{ i.key }}</strong></div>
                  <div class="counts mono">
                    <span i18n="@@analysis.moment.events">{{ count(i.events) }} events</span>
                    <span class="warn" i18n="@@analysis.moment.warnings">{{ count(i.warnings) }} warnings</span>
                    <span [class.errs]="i.errors" i18n="@@analysis.moment.errors">{{ count(i.errors) }} errors</span>
                  </div>
                  <ul class="list pats">
                    @for (p of i.patterns; track p.id) {
                      <li><span class="num">{{ count(p.count) }}</span><span class="ellipsis" [title]="p.template">{{ p.template }}</span></li>
                    }
                  </ul>
                  @if (i.firstError !== null) {
                    <button class="btn" type="button" (click)="select(i.firstError)" i18n="@@analysis.moment.firstError">Go to first error</button>
                  }
                </section>
              }
            </div>
          } @else {
            <p class="muted" i18n="@@analysis.moment.noEvents">No events in this window.</p>
          }

          <h3 class="sec" i18n="@@analysis.moment.metrics">Metrics</h3>
          @if (m.metrics.length) {
            <table class="grid">
              <thead>
                <tr><th scope="col" i18n="@@analysis.moment.col.series">Series</th><th scope="col" i18n="@@analysis.moment.col.metric">Metric</th><th scope="col" class="num" i18n="@@analysis.moment.col.min">Min</th><th scope="col" class="num" i18n="@@analysis.moment.col.avg">Avg</th><th scope="col" class="num" i18n="@@analysis.moment.col.max">Max</th><th scope="col" class="num" i18n="@@analysis.moment.col.last">Last</th><th scope="col" class="num" i18n="@@analysis.moment.col.previousAvg">Previous avg</th><th scope="col" class="num" i18n="@@analysis.moment.col.change">Change</th></tr>
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
            <p class="muted" i18n="@@analysis.moment.noMetrics">No numeric metrics in this window.</p>
          }

          <h3 class="sec" i18n="@@analysis.moment.users">Users</h3>
          @if (m.users.length) {
            <div class="chips" role="group" aria-label="Users active in this window" i18n-aria-label="@@analysis.moment.usersLabel">
              @for (u of m.users; track u.field + '=' + u.value) {
                <button class="chip" type="button" (click)="filterUser(u)" title="Filter by {{ u.field }} = {{ u.value }}" i18n-title="@@analysis.moment.filterUser">{{ u.value }} <span class="mono">{{ count(u.count) }}</span></button>
              }
            </div>
          } @else {
            <p class="muted" i18n="@@analysis.moment.noUsers">No user information in this window.</p>
          }

          <h3 class="sec" i18n="@@analysis.moment.userActivity">User activity</h3>
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
            <p class="muted" i18n="@@analysis.moment.noActivity">No user actions recorded.</p>
          }

          <h3 class="sec" i18n="@@analysis.moment.anomalies">Anomalies in this window</h3>
          @if (m.anomalies.length) {
            <ul class="list">
              @for (a of m.anomalies; track $index) {
                <li>
                  <button class="row" type="button" (click)="selectSample(a.sampleIds[0])">
                    <span aria-hidden="true">{{ icon(a.kind) }}</span><strong>{{ label(a.kind) }}</strong>
                    <span class="muted ellipsis">{{ a.instance ?? allInstances }}</span><span class="grow ellipsis">{{ a.message }}</span>
                  </button>
                </li>
              }
            </ul>
          } @else {
            <p class="muted" i18n="@@analysis.moment.none">None.</p>
          }

          <imt-profile-moment [range]="range()" />
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
  protected instanceLabel(key: string): string {
    return $localize`:@@analysis.moment.instanceN:Instance ${key}:instance:`;
  }
  private readonly ai = inject(AiLauncher);
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
  protected readonly allInstances = $localize`:@@analysis.anomalies.allInstances:all instances`;
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

  /** "Summarize this range" with the local AI model. */
  protected summarize(): void {
    const r = this.range();
    if (r) void this.ai.summarize(r.from, r.to);
  }
}
