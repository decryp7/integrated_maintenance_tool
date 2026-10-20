import { ChangeDetectionStrategy, Component, effect, inject, untracked } from '@angular/core';
import { Butterfly, ButterflyRow, FrameDto, MetricValues } from '../../core/api.types';
import { AnalysisState } from '../../analysis/analysis-state';
import { AsyncResource } from '../../analysis/async-resource';
import { FrameNames } from '../captures.store';
import { ProfileViewState } from '../profile-view-state';
import { ProfilingBackend } from '../profiling-backend';
import { formatMetric, formatPct, measurementOf } from '../profiling-format';
import { ActiveProfile, ProfilingNavigator } from '../profiling-navigator';
import { MeasurementTag, NoCapture, ProfileBar } from '../profiling-shared';
import { PROFILING_STYLES } from '../profiling-styles';

/** Callers above, callees below the selected function; clicking one makes it the selection. */
@Component({
  selector: 'imt-butterfly-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, ProfileBar, MeasurementTag, NoCapture],
  template: `
    @if (active.profile()) {
      <imt-profile-bar />
      <div class="bar">
        <span class="section-title" i18n="@@profiling.butterfly.title">Callers and callees</span>
        @if (state.selectedFrame(); as f) { <span class="mono ellipsis grow" [title]="f.name">{{ f.name }}</span> }
      </div>
      <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [retryable]="res.retryable()" [hasData]="!!res.data()" [empty]="!state.selectedFrame()" emptyText="Select a function (in the hotspots, a tree or the flame graph) to see who calls it and what it calls." i18n-emptyText="@@profiling.butterfly.empty" (retry)="res.retry()">
        @if (res.data(); as b) {
          <div class="body">
            <h3 class="sec" id="bf-callers" i18n="@@profiling.butterfly.callers">Callers ({{ b.callers.length }})</h3>
            <ul class="list" aria-labelledby="bf-callers">
              @for (r of b.callers; track $index) {
                <li>
                  <button class="row" type="button" [disabled]="!r.frame" (click)="go(r.frame)" [attr.aria-label]="label(r, 'caller')">
                    <span class="meter" aria-hidden="true"><span [style.width.%]="r.pct"></span></span>
                    <span class="num pct">{{ pct(r.pct) }}</span>
                    <span class="num val">{{ value(r.values) }}</span>
                    <span class="ellipsis grow" [class.muted]="!r.frame">{{ r.frame?.name ?? noCaller }}</span>
                  </button>
                </li>
              } @empty { <li class="muted pad" i18n="@@profiling.butterfly.noCallers">No callers.</li> }
            </ul>

            <div class="focus" role="group" aria-label="Selected function" i18n-aria-label="@@profiling.butterfly.selected">
              <strong class="ellipsis" [title]="b.function.name">{{ b.function.name }}</strong>
              <span class="mono" i18n="@@profiling.butterfly.totals">total {{ value(b.function.total) }} ({{ pct(b.function.totalPct) }}) · self {{ value(b.function.self) }} ({{ pct(b.function.selfPct) }})</span>
              @if (measurement(); as m) { <imt-measurement [m]="m" /> }
              <span class="spacer"></span>
              @if (b.function.frame; as f) {
                <button class="mini" type="button" (click)="nav.showInFlame(f)" i18n="@@profiling.butterfly.showInFlame">Show in flame graph</button>
                <button class="mini" type="button" (click)="nav.focusInFlame(f, b.metric)" i18n="@@profiling.butterfly.zoomFlame">Zoom flame graph to it</button>
              }
            </div>

            <h3 class="sec" id="bf-callees" i18n="@@profiling.butterfly.callees">Callees ({{ b.callees.length }})</h3>
            <ul class="list" aria-labelledby="bf-callees">
              @for (r of b.callees; track $index) {
                <li>
                  <button class="row" type="button" [disabled]="!r.frame" (click)="go(r.frame)" [attr.aria-label]="label(r, 'callee')">
                    <span class="meter" aria-hidden="true"><span [style.width.%]="r.pct"></span></span>
                    <span class="num pct">{{ pct(r.pct) }}</span>
                    <span class="num val">{{ value(r.values) }}</span>
                    <span class="ellipsis grow">{{ r.frame?.name }}</span>
                  </button>
                </li>
              } @empty { <li class="muted pad" i18n="@@profiling.butterfly.noCallees">No callees: all of its time is self time.</li> }
            </ul>
          </div>
        }
      </imt-analysis-state>
    } @else {
      <imt-no-capture text="Open a profile to see callers and callees." i18n-text="@@profiling.butterfly.noCapture" />
    }
  `,
  styles: [
    PROFILING_STYLES,
    `
      .focus { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin: 10px 0; padding: 8px 10px; border: 1px solid var(--accent); border-radius: 6px; background: var(--accent-soft); color: var(--text); }
      .pct { width: 52px; flex: none; }
      .val { width: 80px; flex: none; }
      .pad { padding: 4px 12px; }
      .row:disabled { cursor: default; }
    `,
  ],
})
export class ButterflyView {
  protected readonly active = inject(ActiveProfile);
  protected readonly state = inject(ProfileViewState);
  protected readonly nav = inject(ProfilingNavigator);
  private readonly backend = inject(ProfilingBackend);
  private readonly names = inject(FrameNames);
  protected readonly res = new AsyncResource<Butterfly>();
  protected readonly pct = formatPct;
  protected readonly noCaller = $localize`:@@profiling.butterfly.topOfStack:(top of the stack: no caller)`;

  constructor() {
    effect(() => {
      const p = this.active.profile();
      const frame = this.state.selectedFrame();
      const metric = this.active.metric();
      const view = this.state.view();
      untracked(() => {
        if (!p || !frame || !metric) return this.res.reset();
        void this.res.load(async () => {
          const b = await this.backend.butterfly({ profileId: p.id, view, frame: frame.id, metric, limit: 50 });
          this.names.remember(p.id, [...b.callers, ...b.callees].map((r) => r.frame));
          return b;
        });
      });
    });
  }

  protected measurement() {
    const m = this.active.metric();
    return m ? measurementOf(this.active.profile(), m) : null;
  }

  protected value(v: MetricValues): string {
    const m = this.active.metric();
    return m ? formatMetric(v[m], m, this.active.profile()?.metrics) : '—';
  }

  protected label(r: ButterflyRow, role: 'caller' | 'callee'): string {
    const name = r.frame?.name ?? $localize`:@@profiling.butterfly.topOfStackShort:(top of the stack)`;
    const pct = formatPct(r.pct);
    const value = this.value(r.values);
    return role === 'caller'
      ? $localize`:@@profiling.butterfly.callerLabel:Caller ${name}:name:: ${pct}:pct: of the function, ${value}:value:`
      : $localize`:@@profiling.butterfly.calleeLabel:Callee ${name}:name:: ${pct}:pct: of the function, ${value}:value:`;
  }

  protected go(frame: FrameDto | null): void {
    if (frame) this.state.selectFrame(frame);
  }
}
