import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { LiveAlert, SEVERITIES } from '../core/api.types';
import { formatDateTime, formatTime } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { DialogService } from '../ui/dialog.service';
import { LiveTail } from '../workspace/live-tail';
import { AlertRulesEditor } from './alert-rules-editor';
import { AlertsStore, kindLabel, muteLabel, SeverityFilter, severityLabel } from './alerts.store';
import { LiveAlerts } from './live-alerts';
import { SeverityBadge } from './severity-badge';
import { activeLocale } from '../i18n/locale';

/** Dock view: the live alerts with their evidence, a severity filter, mutes and the rules editor. */
@Component({
  selector: 'imt-alerts-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SeverityBadge],
  template: `
    <div class="bar">
      <span class="section-title" i18n="@@live.alertsView.title">Alerts</span>
      <div class="filters" role="group" aria-label="Filter by severity" i18n-aria-label="@@live.alertsView.filterAria">
        <button type="button" class="chip" [attr.aria-pressed]="store.filter() === 'all'" (click)="store.setFilter('all')" i18n="@@live.alertsView.all">All {{ store.alerts().length }}</button>
        @for (s of severities; track s) {
          <button type="button" class="chip" [attr.aria-pressed]="store.filter() === s" (click)="store.setFilter(s)">
            <imt-severity-badge [severity]="s" [size]="12" [showLabel]="false" />{{ sevLabel(s) }} {{ store.counts()[s] }}
          </button>
        }
      </div>
      <span class="spacer"></span>
      @if (!live.enabled()) { <span class="faint small" i18n="@@live.alertsView.whileLive">Alerts are raised while live tail is on.</span> }
      <button type="button" class="btn" (click)="editRules()" i18n="@@live.alertsView.rules">Alert rules…</button>
      <button type="button" class="btn ghost" [disabled]="!store.alerts().length" (click)="store.clear()" i18n="alert list|Empties the alert list@@live.alertsView.clear">Clear</button>
    </div>

    @if (mutes().length) {
      <div class="mutes" role="group" aria-label="Muted alert kinds" i18n-aria-label="@@live.alertsView.mutedAria">
        <span class="faint small" i18n="@@live.alertsView.muted">Muted:</span>
        @for (m of mutes(); track m.key) {
          <span class="mute" i18n="@@live.alertsView.muteUntil">{{ muteName(m.key) }} until {{ clock(m.until) }}
            <button type="button" class="link" [attr.aria-label]="unmuteLabel(m.key)" (click)="store.unmute(m.key)">Unmute</button></span>
        }
      </div>
    }

    <div class="list" tabindex="0" role="region" aria-label="Alert list" i18n-aria-label="@@live.alertsView.listAria">
      @if (store.visible().length) {
        <table>
          <thead>
            <tr><th scope="col" i18n="@@live.alertsView.colSeverity">Severity</th><th scope="col" i18n="@@live.alertsView.colKind">Kind</th><th scope="col" i18n="@@live.alertsView.colInstance">Instance</th><th scope="col" i18n="@@live.alertsView.colTime">Time</th><th scope="col" i18n="@@live.alertsView.colMessage">Message</th><th scope="col"><span class="sr" i18n="@@live.alertsView.colActions">Actions</span></th></tr>
          </thead>
          <tbody>
            @for (e of store.visible(); track e.id) {
              <tr [class.muted-row]="e.muted" data-testid="alert-row">
                <td><imt-severity-badge [severity]="e.alert.severity" /></td>
                <td>{{ kind(e.alert) }}@if (e.muted) { <span class="faint small" i18n="@@live.alertsView.mutedRow"> (muted)</span> }</td>
                <td class="mono">{{ e.alert.instance ?? '—' }}</td>
                <td class="mono nowrap" [title]="range(e.alert)">{{ time(e.alert.to) }}</td>
                <td class="msg">{{ e.alert.message }}</td>
                <td class="actions nowrap">
                  <button type="button" class="btn" [attr.aria-label]="showLabel(e.alert)" (click)="actions.show(e.alert)" i18n="@@live.alertsView.show">Show</button>
                  <button type="button" class="btn ghost" [attr.aria-label]="muteLabel(e.alert)" (click)="store.mute(e.alert)" i18n="@@live.alertsView.mute15">Mute 15 min</button>
                  <button type="button" class="icon-btn" [attr.aria-label]="removeLabel(e.alert)" title="Remove" i18n-title="@@live.alertsView.remove" (click)="store.remove(e.id)">×</button>
                </td>
              </tr>
            }
          </tbody>
        </table>
      } @else {
        <div class="empty-state">
          @if (store.alerts().length) {
            <div i18n="@@live.alertsView.noFiltered">No {{ filterName() }} alerts.</div>
          } @else {
            <div i18n="@@live.alertsView.none">No alerts yet.</div>
            <div class="faint" i18n="@@live.alertsView.noneHelp">While live tail is on, error spikes, silences, crashes and your alert rules appear here.</div>
            @if (!live.enabled() && live.available()) {
              <button type="button" class="btn" (click)="live.toggle()" i18n="@@live.alertsView.turnOn">Turn on live tail</button>
            }
          }
        </div>
      }
    </div>
  `,
  styles: `
    :host { flex: 1; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .bar { flex: none; display: flex; align-items: center; gap: 10px; padding: 8px 12px; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
    .filters { display: flex; gap: 4px; flex-wrap: wrap; }
    .chip { display: inline-flex; align-items: center; gap: 4px; height: 22px; padding: 0 8px; border-radius: 11px; border: 1px solid var(--border-2); background: transparent; color: var(--text-2); font: inherit; font-size: 12px; cursor: pointer; }
    .chip[aria-pressed='true'] { border-color: var(--accent); background: var(--accent-soft); color: var(--text); }
    .spacer { flex: 1; }
    .small { font-size: 11px; }
    .mutes { flex: none; display: flex; flex-wrap: wrap; gap: 6px 12px; align-items: center; padding: 6px 12px; border-bottom: 1px solid var(--border); font-size: 12px; }
    .mute { color: var(--text-2); }
    .link { all: unset; cursor: pointer; color: var(--accent); text-decoration: underline; margin-left: 4px; }
    .link:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
    .list { flex: 1; min-height: 0; overflow: auto; }
    table { width: 100%; border-collapse: collapse; font-size: 12px; }
    th { position: sticky; top: 0; background: var(--bg-2); text-align: left; font-weight: 600; color: var(--muted); padding: 5px 8px; border-bottom: 1px solid var(--border); }
    td { padding: 5px 8px; border-bottom: 1px solid var(--border); vertical-align: top; color: var(--text-2); }
    .msg { color: var(--text); min-width: 200px; }
    .nowrap { white-space: nowrap; }
    .actions { display: flex; gap: 4px; align-items: center; }
    .muted-row td { opacity: 0.75; }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
  `,
})
export class AlertsView {
  protected readonly store = inject(AlertsStore);
  protected readonly actions = inject(LiveAlerts);
  protected readonly live = inject(LiveTail);
  private readonly ws = inject(WorkspaceStore);
  private readonly dialogs = inject(DialogService);

  protected readonly severities = SEVERITIES;
  protected readonly sevLabel = severityLabel;
  protected readonly kind = kindLabel;
  protected readonly muteName = muteLabel;

  /** Re-evaluated whenever the mutes change (expired ones drop out on the next change). */
  protected readonly mutes = computed(() => (this.store.mutes(), this.store.activeMutes()));
  protected readonly filterName = computed(() => {
    const f: SeverityFilter = this.store.filter();
    return f === 'all' ? '' : severityLabel(f).toLowerCase();
  });

  protected unmuteLabel(key: string): string {
    return $localize`:@@live.alertsView.unmuteAria:Unmute ${muteLabel(key)}:name:`;
  }

  protected showLabel(a: LiveAlert): string {
    return $localize`:@@live.alertsView.showAria:Show evidence: ${kindLabel(a)}:kind:`;
  }

  protected muteLabel(a: LiveAlert): string {
    return $localize`:@@live.alertsView.muteAria:Mute 15 min: ${kindLabel(a)}:kind:`;
  }

  protected removeLabel(a: LiveAlert): string {
    return $localize`:@@live.alertsView.removeAria:Remove alert ${kindLabel(a)}:kind:`;
  }

  protected time(ts: number): string {
    return formatTime(ts, this.ws.timezone()).slice(0, 8);
  }

  protected clock(ts: number): string {
    return new Date(ts).toLocaleTimeString(activeLocale(), { hour: '2-digit', minute: '2-digit' });
  }

  protected range(a: LiveAlert): string {
    const tz = this.ws.timezone();
    return `${formatDateTime(a.from, tz)} → ${formatDateTime(a.to, tz)}`;
  }

  protected editRules(): void {
    this.dialogs.open(AlertRulesEditor, { title: $localize`:@@live.alertsView.dialogTitle:Live alert rules`, width: 760 });
  }
}
