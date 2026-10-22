import { ChangeDetectionStrategy, Component, effect, inject, input, untracked } from '@angular/core';
import { ProfileMoment } from '../../core/api.types';
import { AsyncResource } from '../../analysis/async-resource';
import { AnalysisState } from '../../analysis/analysis-state';
import { ProfilingBackend } from '../profiling-backend';
import { ProfileViewState } from '../profile-view-state';
import { formatCount } from '../../core/format';
import { formatNs, formatPct } from '../profiling-format';
import { ProfilingNavigator } from '../profiling-navigator';
import { MeasurementTag } from '../profiling-shared';

/**
 * "Profile samples in this window" for the Moment view: the open profiles' top functions while the
 * log window lasted, with a link that opens the profile on that range. Hidden without profiles.
 */
@Component({
  selector: 'debrief-profile-moment',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, MeasurementTag],
  template: `
    @if (backend && (res.loading() || res.error() || res.data()?.length)) {
      <h3 class="sec" i18n="@@profiling.moment.title">Profile samples in this window</h3>
      <debrief-analysis-state [loading]="res.loading()" [error]="res.error()" [retryable]="res.retryable()" [hasData]="!!res.data()" (retry)="res.retry()">
        @for (m of res.data() ?? []; track m.profileId) {
          <section class="card" [attr.aria-label]="profileLabel(m.name)">
            <div class="head">
              <strong class="ellipsis">{{ m.name }}</strong>
              <span class="mono muted" i18n="@@profiling.moment.samples">{{ count(m.samples) // i18n(ph: "samples") }} samples · {{ ns(m.value) // i18n(ph: "value") }}</span>
              <debrief-measurement [m]="m.measurement" />
              <span class="spacer"></span>
              <button class="btn" type="button" (click)="open(m)" i18n="@@profiling.moment.open">Open capture on this range</button>
            </div>
            <ol class="top">
              @for (r of m.top; track r.key) {
                <li><span class="mono pct">{{ pct(r.selfPct) }}</span><span class="ellipsis" [title]="r.name">{{ r.name }}</span></li>
              }
            </ol>
          </section>
        }
      </debrief-analysis-state>
    }
  `,
  styles: `
    :host { display: block; }
    h3.sec { margin: 14px 0 6px; font-size: var(--fs-xs); font-weight: 600; letter-spacing: 0.08em; text-transform: uppercase; color: var(--muted); }
    .card { border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); padding: 8px 10px; margin-bottom: 8px; }
    .head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; color: var(--text); }
    .spacer { flex: 1; }
    .muted { color: var(--muted); }
    .mono { font-family: var(--font-mono); }
    .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
    .top { margin: 6px 0 0; padding: 0; list-style: none; font-size: var(--fs-xs); }
    .top li { display: flex; gap: 8px; padding: 1px 0; }
    .pct { width: 48px; flex: none; text-align: right; }
  `,
})
export class ProfileMomentSection {
  /** The log window (Unix ms); null hides the section. */
  readonly range = input<{ from: number; to: number } | null>(null);
  protected readonly backend = inject(ProfilingBackend, { optional: true });
  private readonly state = inject(ProfileViewState);
  private readonly nav = inject(ProfilingNavigator);
  protected readonly res = new AsyncResource<ProfileMoment[]>();
  protected readonly count = formatCount;
  protected readonly ns = formatNs;
  protected readonly pct = formatPct;

  constructor() {
    effect(() => {
      const r = this.range();
      this.state.capturesVersion();
      const backend = this.backend;
      untracked(() => (r && backend ? void this.res.load(() => backend.moment({ fromMs: r.from, toMs: r.to, top: 5 })) : this.res.reset()));
    });
  }

  protected profileLabel(name: string): string {
    return $localize`:@@profiling.moment.profileLabel:Profile ${name}:name:`;
  }

  protected open(m: ProfileMoment): void {
    this.nav.openOnRange(m.profileId, m.range);
  }
}
