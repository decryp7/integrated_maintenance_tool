import { ChangeDetectionStrategy, Component, computed, effect, inject, untracked } from '@angular/core';
import { AiButton } from '../ai/ai-button';
import { AiLauncher } from '../ai/ai-launcher';
import { Finding } from '../core/api.types';
import { formatCount, formatDateTime, formatDuration } from '../core/format';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { AnalysisState } from '../analysis/analysis-state';
import { AsyncResource } from '../analysis/async-resource';
import { ShareBackend } from '../share/share-backend';
import { Icon } from '../shared/icon';
import { HelpHint } from '../ui/help-hint';
import { ActionButton, actionButtons, groupBySeverity, overviewStats } from './overview-model';
import { OverviewActions } from './overview-actions';
import { OverviewPrefs } from './overview-prefs';

/** The automatic health check: headline numbers and the engine's findings, most severe first, each with its next step. */
@Component({
  selector: 'imt-overview-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, Icon, HelpHint, AiButton],
  template: `
    <div class="bar">
      <h2 class="section-title" i18n="@@overview.view.title">Overview</h2>
      <imt-help label="Overview" i18n-label="@@overview.view.helpLabel" text="A health check of everything you opened: what went wrong and where to look first. It is computed by the engine; your files are never changed." i18n-text="@@overview.view.helpText" />
      <span class="spacer"></span>
      <button class="btn" type="button" (click)="res.retry()" [disabled]="res.loading()" i18n="@@overview.view.refresh">Refresh</button>
      <imt-ai-button label="Root causes" i18n-label="@@overview.view.rootCauses" hint="The local model reads the findings and exceptions and lists the likely root causes" i18n-hint="@@overview.view.rootCausesHint" testId="ai-root-causes" (pressed)="ai.rootCauses()" />
    </div>
    <div class="scroll" tabindex="0" role="region" aria-label="Health check" i18n-aria-label="@@overview.view.healthCheck">
      @if (stats(); as s) {
        <dl class="stats" aria-label="Summary" i18n-aria-label="@@overview.view.summary">
          <div><dt i18n="@@overview.view.sources">Sources</dt><dd>{{ count(s.sources) }}</dd></div>
          <div><dt i18n="@@overview.view.events">Events</dt><dd>{{ count(s.events) }}</dd></div>
          <div><dt i18n="@@overview.view.instances">Instances</dt><dd>{{ count(s.instances) }}</dd></div>
          <div class="wide"><dt i18n="@@overview.view.timeSpan">Time span</dt><dd>{{ span() }}</dd></div>
          <div [class.bad]="s.errors > 0"><dt i18n="@@overview.view.errors">Errors</dt><dd>{{ count(s.errors) }}</dd></div>
          <div [class.bad]="s.crashes > 0"><dt i18n="@@overview.view.crashes">Crashes</dt><dd>{{ count(s.crashes) }}</dd></div>
        </dl>
        @if (s.unrecognized.length) {
          <section class="sub" aria-labelledby="ov-unrec">
            <h3 id="ov-unrec" i18n="@@overview.view.unrecognized">{s.unrecognized.length, plural, =1 {{{ s.unrecognized.length }} file not opened} other {{{ s.unrecognized.length }} files not opened}}</h3>
            <ul>
              @for (u of s.unrecognized; track u.path) {
                <li><span class="mono name" [title]="u.path">{{ u.name }}</span><span class="muted"> — {{ u.reason }}</span></li>
              }
            </ul>
          </section>
        }
        @if (s.plugins.length) {
          <section class="sub" aria-labelledby="ov-plugins">
            <h3 id="ov-plugins" i18n="@@overview.view.pluginProblems">{s.plugins.length, plural, =1 {{{ s.plugins.length }} plugin with problems} other {{{ s.plugins.length }} plugins with problems}}</h3>
            <ul>
              @for (p of s.plugins; track p.id) {
                <li><strong>{{ p.name }}</strong><span class="muted"> — {{ p.problem }}</span></li>
              }
            </ul>
          </section>
        }
      } @else {
        <p class="muted pad" i18n="@@overview.view.empty">Open files or a folder to see a health check here.</p>
      }

      <imt-analysis-state
        [loading]="res.loading()"
        [error]="res.error()" [retryable]="res.retryable()"
        [hasData]="!!res.data()"
        [empty]="false"
        (retry)="res.retry()"
      >
        @if (res.data(); as findings) {
          @if (!findings.length) {
            <div class="none" role="status">
              <imt-icon name="check" [size]="16" />
              <div>
                <strong i18n="@@overview.view.none">Nothing alarming found</strong>
                <p class="muted" i18n="@@overview.view.noneDetail">No crashes, error spikes or unreadable files. Explore the timeline, or search the events for what you are after.</p>
              </div>
            </div>
          }
          @for (g of groups(); track g.severity) {
            <section class="group" [attr.aria-labelledby]="'ov-sev-' + g.severity" [attr.data-severity]="g.severity">
              <h3 class="sev" [id]="'ov-sev-' + g.severity">
                <imt-icon [name]="g.icon" [size]="14" />{{ g.label }} <span class="faint">({{ g.findings.length }})</span>
              </h3>
              <ul class="findings">
                @for (f of g.findings; track f.kind + ':' + f.title + ':' + f.detail) {
                  <li class="finding">
                    <div class="title"><span class="tag">{{ severityText(f) }}</span> {{ f.title }}</div>
                    <p class="detail">{{ f.detail }}</p>
                    @if (buttons(f); as bs) {
                      @if (bs.length) {
                        <div class="acts">
                          @for (b of bs; track b.id) {
                            <button class="btn" type="button" (click)="act(f, b)">{{ b.label }}</button>
                          }
                        </div>
                      }
                    }
                  </li>
                }
              </ul>
            </section>
          }
        }
      </imt-analysis-state>

      <label class="auto" i18n="@@overview.view.autoOpen">
        <input type="checkbox" [checked]="prefs.autoOpen()" (change)="prefs.setAutoOpen($any($event.target).checked)" />
        Show the overview after opening files
      </label>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--bg-1); color: var(--text); }
    .bar { flex: none; display: flex; align-items: center; gap: 8px; padding: 6px 10px; border-bottom: 1px solid var(--border); }
    .bar h2 { margin: 0; }
    .spacer { flex: 1; }
    .scroll { flex: 1; min-height: 0; overflow: auto; padding: 10px 12px 16px; display: flex; flex-direction: column; gap: 12px; }
    .pad { margin: 4px 0; }
    .stats { margin: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(96px, 1fr)); gap: 6px; }
    .stats > div { padding: 6px 8px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); }
    .stats > div.wide { grid-column: span 2; }
    .stats dt { font-size: var(--fs-xs); color: var(--muted); }
    .stats dd { margin: 2px 0 0; font-size: var(--fs-lg); font-weight: 600; font-variant-numeric: tabular-nums; }
    .stats .wide dd { font-size: var(--fs-sm); font-weight: 500; }
    .stats > div.bad dd { color: var(--danger-text); }
    .sub h3, .sev { margin: 0 0 4px; font-size: var(--fs-sm); font-weight: 600; display: flex; align-items: center; gap: 6px; }
    .sub ul, .findings { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 4px; }
    .sub li { font-size: var(--fs-sm); overflow-wrap: anywhere; }
    .name { color: var(--text); }
    .muted { color: var(--muted); }
    [data-severity='critical'] .sev imt-icon { color: var(--danger-text); }
    [data-severity='warning'] .sev imt-icon { color: var(--warn-text); }
    [data-severity='info'] .sev imt-icon { color: var(--accent-text); }
    .finding { padding: 8px 10px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); }
    .title { font-weight: 600; font-size: var(--fs-sm); overflow-wrap: anywhere; }
    .tag { display: inline-block; padding: 0 5px; margin-right: 4px; border-radius: 4px; font-size: var(--fs-2xs); font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; border: 1px solid var(--border-3); color: var(--text-2); }
    [data-severity='critical'] .tag { border-color: var(--lvl-error); color: var(--danger-text); }
    [data-severity='warning'] .tag { border-color: var(--lvl-warn); color: var(--warn-text); }
    .detail { margin: 4px 0 0; font-size: var(--fs-sm); color: var(--text-2); overflow-wrap: anywhere; }
    .acts { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 6px; }
    .none { display: flex; gap: 10px; align-items: flex-start; padding: 12px; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); }
    .none imt-icon { color: var(--ok); margin-top: 2px; }
    .none p { margin: 4px 0 0; font-size: var(--fs-sm); }
    .auto { display: flex; align-items: center; gap: 8px; font-size: var(--fs-sm); color: var(--text-2); }
  `,
})
export class OverviewView {
  protected readonly ai = inject(AiLauncher);
  protected readonly store = inject(WorkspaceStore);
  protected readonly prefs = inject(OverviewPrefs);
  private readonly share = inject(ShareBackend);
  private readonly actions = inject(OverviewActions);
  protected readonly res = new AsyncResource<Finding[]>();
  protected readonly count = formatCount;

  protected readonly groups = computed(() => groupBySeverity(this.res.data() ?? []));
  protected readonly stats = computed(() => {
    const summary = this.store.summary();
    return summary && summary.sources.length + summary.unrecognized.length > 0 ? overviewStats(summary, this.store.instanceCount(), this.res.data() ?? []) : null;
  });
  protected readonly span = computed(() => {
    const s = this.stats();
    if (!s || s.from === null || s.to === null) return $localize`:@@overview.view.noTimestamps:No timestamps`;
    const tz = this.store.timezone();
    const from = formatDateTime(s.from, tz);
    const to = formatDateTime(s.to, tz);
    const duration = formatDuration(s.to - s.from);
    return $localize`:@@overview.view.span:${from}:from: → ${to}:to: (${duration}:duration:)`;
  });

  constructor() {
    effect(() => {
      this.store.dataVersion();
      const summary = this.store.summary();
      const groupBy = this.store.effectiveGroupBy();
      if (!summary) return;
      // The health check covers everything opened, not the current filters.
      untracked(() => void this.res.load(() => this.share.overview({ ...EMPTY_QUERY, groupBy })));
    });
  }

  protected severityText(f: Finding): string {
    switch (f.severity) {
      case 'critical':
        return $localize`:@@overview.view.tag.critical:critical`;
      case 'warning':
        return $localize`:@@overview.view.tag.warning:warning`;
      default:
        return $localize`:@@overview.view.tag.info:info`;
    }
  }

  protected buttons(f: Finding): ActionButton[] {
    return actionButtons(f.action);
  }

  protected act(f: Finding, b: ActionButton): void {
    void this.actions.run(f, b.id);
  }
}
