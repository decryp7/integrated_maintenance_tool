import { ChangeDetectionStrategy, Component, computed, effect, inject, untracked } from '@angular/core';
import { formatDateTime } from '../../core/format';
import { WorkspaceStore } from '../../core/workspace.store';
import { liveOption } from '../../metrics/chart-card';
import { ChartTheme } from '../../metrics/chart-theme';
import { EChart } from '../../metrics/echart';
import { MONITOR_WINDOWS, monitorOption, MonitorWindow } from './monitor-charts-model';
import { MonitorChartsStore } from './monitor-charts.store';

/**
 * The process monitor's "Live" panel: small multiples (CPU, memory, handles, threads, IO) with one line per tracked
 * process over the last minutes, following new samples as live tail brings them in. The engine computes every series.
 */
@Component({
  selector: 'imt-monitor-live-charts',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [EChart],
  providers: [MonitorChartsStore],
  template: `
    @if (store.visible()) {
      <section class="live" aria-labelledby="pmon-live" data-testid="monitor-live">
        <div class="head">
          <h2 id="pmon-live" class="section-title" i18n="live chart panel heading|@@tools.monitor.live">Live</h2>
          <div class="seg" role="group" aria-label="Time window" i18n-aria-label="@@tools.monitor.live.window">
            @for (w of windows; track w) {
              <button type="button" class="chip" [attr.aria-pressed]="store.windowMin() === w" (click)="setWindow(w)" i18n="@@tools.monitor.live.minutes">{{ w }} min</button>
            }
          </div>
          <button
            type="button"
            class="chip follow"
            data-testid="monitor-follow"
            [attr.aria-pressed]="store.following()"
            [title]="followTitle()"
            (click)="store.following() ? store.pause() : store.follow()"
          ><span class="dot" aria-hidden="true"></span>{{ followLabel() }}</button>
          <span class="spacer"></span>
          @if (until(); as u) { <span class="faint mono small" data-testid="monitor-live-until" i18n="@@tools.monitor.live.until">to {{ u }}</span> }
          @if (store.loading() && store.charts().length) { <span class="spinner" role="status" aria-label="Updating" i18n-aria-label="@@tools.monitor.live.updating"></span> }
        </div>

        @if (store.legend().length) {
          <div class="legend" role="group" aria-label="Processes" i18n-aria-label="@@tools.monitor.live.processes">
            @for (l of store.legend(); track l.key) {
              <button type="button" class="chip key" [attr.aria-pressed]="!store.hiddenSet().has(l.key)" [title]="toggleTitle(l.key)" (click)="store.toggle(l.key)">
                <svg class="swatch" width="16" height="6" aria-hidden="true">
                  <line x1="0" y1="3" x2="16" y2="3" [attr.stroke]="color(l.slot)" stroke-width="2.5" />
                </svg>
                <span class="mono">{{ l.key }}</span>
              </button>
            }
          </div>
        }

        @if (store.error(); as e) {
          <div class="state err" role="alert" data-testid="monitor-live-error">
            <span i18n="@@tools.monitor.live.error">Could not load the live charts: {{ e }}</span>
            <button type="button" class="btn" (click)="store.retry()" i18n="@@tools.monitor.live.retry">Retry</button>
          </div>
        } @else if (!store.charts().length) {
          <div class="state faint" role="status" data-testid="monitor-live-loading" i18n="@@tools.monitor.live.loading"><span class="spinner" aria-hidden="true"></span>Loading live charts…</div>
        } @else if (store.empty()) {
          <div class="state faint" role="status" data-testid="monitor-live-empty" i18n="@@tools.monitor.live.empty">
            No samples in the last {{ store.windowMin() }} minutes yet. They appear here as the monitor writes them.
          </div>
        } @else {
          @for (g of store.groups(); track g.id) {
            <div class="group" role="group" [attr.aria-label]="g.title" [attr.data-group]="g.id">
              @if (store.groups().length > 1) { <h3 class="group-title">{{ g.title }}</h3> }
              <div class="grid">
                @for (m of g.metrics; track m.metric.id) {
                  <figure class="card" [attr.data-metric]="m.metric.id" [attr.aria-label]="chartLabel(m.metric.title, m.metric.unit)">
                    <figcaption><span class="title">{{ m.metric.title }}</span>@if (m.metric.unit) { <span class="faint mono small">{{ m.metric.unit }}</span> }</figcaption>
                    @if (options()[m.index]; as o) {
                      <imt-echart class="chart" [options]="o" [merge]="store.following()" />
                    } @else {
                      <div class="none faint" i18n="@@tools.monitor.live.notRecorded">Not recorded for these processes.</div>
                    }
                  </figure>
                }
              </div>
            </div>
          }
        }
      </section>
    }
  `,
  styles: `
    :host { display: contents; }
    .live { display: flex; flex-direction: column; gap: 8px; }
    .head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    h2 { margin: 0; }
    .spacer { flex: 1; }
    .small { font-size: var(--fs-xs); }
    .seg, .legend { display: inline-flex; gap: 4px; flex-wrap: wrap; }
    .chip { display: inline-flex; align-items: center; gap: 6px; height: 22px; padding: 0 9px; border-radius: 11px; border: 1px solid var(--border-2); background: transparent; color: var(--text-2); font: inherit; font-size: var(--fs-sm); cursor: pointer; white-space: nowrap; }
    .chip:hover { background: var(--bg-hover); }
    .chip[aria-pressed='true'] { border-color: var(--accent); background: var(--accent-soft); color: var(--text); }
    .key[aria-pressed='false'] { opacity: 0.55; text-decoration: line-through; }
    .key[aria-pressed='true'] { border-color: var(--border-3); background: var(--bg-2); }
    .follow .dot { width: 7px; height: 7px; border-radius: 50%; border: 1px solid currentColor; }
    .follow[aria-pressed='true'] .dot { background: var(--accent); border-color: var(--accent); }
    .follow[aria-pressed='false'] { border-color: var(--warn-text); color: var(--text); background: var(--bg-3); }
    .swatch { flex: none; }
    .group { display: flex; flex-direction: column; gap: 6px; }
    .group-title { margin: 0; font-size: var(--fs-sm); font-weight: 600; color: var(--text-2); }
    .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 8px; }
    .card { margin: 0; padding: 6px 8px 2px; border: 1px solid var(--border-3); border-radius: 8px; background: var(--bg-2); display: flex; flex-direction: column; min-width: 0; }
    figcaption { display: flex; align-items: baseline; gap: 6px; }
    .title { font-weight: 600; }
    .chart { height: 160px; }
    .none { height: 160px; display: flex; align-items: center; justify-content: center; font-size: var(--fs-sm); }
    .state { display: flex; align-items: center; justify-content: center; gap: 10px; min-height: 120px; padding: 12px; border: 1px dashed var(--border-3); border-radius: 8px; text-align: center; flex-wrap: wrap; }
    .err { color: var(--danger-text); border-style: solid; }
    .spinner { width: 10px; height: 10px; border-radius: 50%; border: 2px solid var(--border-3); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
    @keyframes spin { to { transform: rotate(360deg); } }
    @media (prefers-reduced-motion: reduce) { .spinner { animation: none; } }
  `,
})
export class MonitorLiveCharts {
  protected readonly store = inject(MonitorChartsStore);
  private readonly workspace = inject(WorkspaceStore);
  private readonly theme = inject(ChartTheme);
  protected readonly windows = MONITOR_WINDOWS;
  protected readonly followTitle = computed(() =>
    this.store.following()
      ? $localize`:@@tools.monitor.live.pauseTitle:Pause: keep the charts as they are`
      : $localize`:@@tools.monitor.live.followTitle:Follow new samples as they are written`,
  );
  protected readonly followLabel = computed(() =>
    this.store.following() ? $localize`:@@tools.monitor.live.following:Following` : $localize`:@@tools.monitor.live.paused:Paused`,
  );

  protected toggleTitle(key: string): string {
    return this.store.hiddenSet().has(key) ? $localize`:@@tools.monitor.live.show:Show ${key}:key:` : $localize`:@@tools.monitor.live.hide:Hide ${key}:key:`;
  }

  protected chartLabel(title: string, unit: string): string {
    const minutes = this.store.windowMin();
    return unit
      ? $localize`:@@tools.monitor.live.chartUnit:${title}:title: (${unit}:unit:), last ${minutes}:minutes: minutes`
      : $localize`:@@tools.monitor.live.chart:${title}:title:, last ${minutes}:minutes: minutes`;
  }

  protected readonly until = computed(() => {
    const to = this.store.until();
    return to === null ? null : formatDateTime(to, this.workspace.timezone());
  });

  /** One option per metric; null where the engine found no samples of that field. */
  protected readonly options = computed(() => {
    const tokens = this.theme.tokens();
    const tz = this.workspace.timezone();
    const slots = this.store.slots();
    const hidden = this.store.hiddenSet();
    const following = this.store.following();
    return this.store.charts().map((data) => {
      if (!data.series.length) return null;
      const option = monitorOption({ data, tokens, tz, slots, hidden });
      return following ? liveOption(option) : option;
    });
  });

  constructor() {
    // Load when the panel appears, the scope or window changes, on Retry, and on every data change while following.
    // After a failure live changes do not reload (no hammering a failing engine): the user retries.
    let last = '';
    let version = -1;
    effect(() => {
      if (!this.store.visible()) return;
      const failed = this.store.error() !== null;
      if (this.store.following() && !failed) version = this.workspace.dataVersion();
      const key = JSON.stringify([this.store.sources(), this.store.windowMin(), this.store.following() ? version : null, this.store.attempt()]);
      if (key === last) return;
      last = key;
      untracked(() => void this.store.load());
    });
  }

  protected setWindow(w: MonitorWindow): void {
    this.store.setWindow(w);
  }

  protected color(slot: number | null): string {
    const t = this.theme.tokens();
    return slot === null ? t.other : (t.series[slot] ?? t.other);
  }
}
