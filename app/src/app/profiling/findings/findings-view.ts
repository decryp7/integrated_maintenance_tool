import { ChangeDetectionStrategy, Component, effect, inject, untracked } from '@angular/core';
import { Evidence, EvidenceView, ProfileFinding } from '../../core/api.types';
import { AnalysisState } from '../../analysis/analysis-state';
import { AsyncResource } from '../../analysis/async-resource';
import { SeverityBadge } from '../../live/severity-badge';
import { ProfileViewState } from '../profile-view-state';
import { ProfilingBackend } from '../profiling-backend';
import { formatNs, formatPct } from '../profiling-format';
import { ActiveProfile, ProfilingNavigator } from '../profiling-navigator';
import { NoCapture, ProfileBar } from '../profiling-shared';
import { PROFILING_STYLES } from '../profiling-styles';

const VIEW_LABEL: Record<EvidenceView, string> = {
  hotspots: 'hotspots',
  callTree: 'call tree',
  flame: 'flame graph',
  butterfly: 'callers and callees',
  timeline: 'thread timeline',
  diff: 'comparison',
};

/** Where an evidence link goes, in words ("call tree · OrderRepository.Save · 2 threads"). */
export function evidenceSummary(e: Evidence): string {
  const parts = [VIEW_LABEL[e.view]];
  if (e.frame) parts.push(e.frame.name);
  else if (e.path.length) parts.push(`path of ${e.path.length} frames`);
  if (e.range) parts.push(`${formatNs(e.range.fromNs)} – ${formatNs(e.range.toNs)}`);
  if (e.threads?.length) parts.push(`${e.threads.length} thread${e.threads.length === 1 ? '' : 's'}`);
  if (e.eventKind) parts.push(`${e.eventKind} events`);
  return parts.join(' · ');
}

/** The engine's automatic findings for the active profile (and baseline), each linking to its evidence. */
@Component({
  selector: 'imt-findings-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, ProfileBar, NoCapture, SeverityBadge],
  template: `
    @if (active.profile(); as p) {
      <imt-profile-bar [showMetric]="false" />
      <div class="bar">
        <span class="section-title">Findings</span>
        @if (state.baselineId() !== null) { <span class="muted">including regressions against the baseline</span> }
      </div>
      <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [hasData]="!!res.data()" [empty]="res.data()?.length === 0" emptyText="No findings: nothing in this view crosses a threshold." (retry)="res.retry()">
        <ul class="list body" aria-label="Findings">
          @for (f of res.data() ?? []; track $index) {
            <li class="finding" [attr.data-kind]="f.kind">
              <div class="head">
                <imt-severity-badge [severity]="f.severity" />
                <strong class="title">{{ f.title }}</strong>
                @if (f.valuePct !== null) { <span class="mono muted">{{ pct(f.valuePct) }}</span> }
              </div>
              <p class="detail">{{ f.detail }}</p>
              <div class="foot">
                <button class="btn" type="button" (click)="open(f)" [attr.aria-label]="'Open evidence for ' + f.title">Open evidence</button>
                <span class="muted ellipsis">{{ summary(f.evidence) }}</span>
              </div>
            </li>
          }
        </ul>
      </imt-analysis-state>
    } @else {
      <imt-no-capture text="Open a profile to see automatic findings." />
    }
  `,
  styles: [
    PROFILING_STYLES,
    `
      .finding { border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); padding: 8px 10px; margin-bottom: 8px; }
      .head { display: flex; align-items: center; gap: 8px; }
      .title { color: var(--text); }
      .detail { margin: 4px 0 6px; color: var(--text-2); }
      .foot { display: flex; align-items: center; gap: 8px; }
    `,
  ],
})
export class FindingsView {
  protected readonly active = inject(ActiveProfile);
  protected readonly state = inject(ProfileViewState);
  private readonly backend = inject(ProfilingBackend);
  private readonly nav = inject(ProfilingNavigator);
  protected readonly res = new AsyncResource<ProfileFinding[]>();
  protected readonly pct = formatPct;
  protected readonly summary = evidenceSummary;

  constructor() {
    effect(() => {
      const p = this.active.profile();
      const baselineId = this.state.baselineId();
      const view = this.state.view();
      untracked(() => (p ? void this.res.load(() => this.backend.findings({ profileId: p.id, baselineId: baselineId !== p.id ? baselineId : null, view })) : this.res.reset()));
    });
  }

  protected open(f: ProfileFinding): void {
    this.nav.openEvidence(f.evidence);
  }
}
