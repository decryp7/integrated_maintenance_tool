import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { formatTime } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { AlertEntry, AlertsStore, kindLabel } from './alerts.store';
import { LiveAlerts } from './live-alerts';
import { SeverityBadge } from './severity-badge';

/** Toasts for live alerts (newest at the bottom), with "Show" and "Mute this kind for 15 min". Mounted once over the app. */
@Component({
  selector: 'imt-alert-toasts',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SeverityBadge],
  host: { 'data-snapshot-ignore': '' },
  template: `
    <div class="stack" role="region" aria-label="Live alerts" i18n-aria-label="@@live.alertToasts.region" aria-live="polite">
      @for (t of alerts.toasts(); track t.id) {
        <div class="toast" [class]="t.alert.severity" data-testid="alert-toast">
          <div class="head">
            <imt-severity-badge [severity]="t.alert.severity" />
            <strong class="kind">{{ kind(t) }}</strong>
            @if (t.alert.instance) { <span class="mono faint inst">{{ t.alert.instance }}</span> }
            <span class="mono faint time">{{ time(t) }}</span>
          </div>
          <div class="msg">{{ t.alert.message }}</div>
          <div class="actions">
            <button type="button" class="btn" (click)="show(t)" i18n="@@live.alertToasts.show">Show</button>
            <button type="button" class="btn ghost" (click)="alerts.mute(t.alert)" i18n="@@live.alertToasts.mute">Mute this kind for 15 min</button>
          </div>
          <button type="button" class="x" aria-label="Dismiss alert" i18n-aria-label="@@live.alertToasts.dismiss" (click)="alerts.dismissToast(t.id)">×</button>
        </div>
      }
    </div>
  `,
  styles: `
    :host { position: fixed; right: 16px; bottom: 32px; z-index: 1900; pointer-events: none; }
    .stack { display: flex; flex-direction: column; gap: 8px; align-items: flex-end; }
    .toast { pointer-events: auto; position: relative; width: 380px; max-width: calc(100vw - 32px); padding: 10px 32px 10px 12px; border: 1px solid var(--border-2); border-left: 3px solid var(--accent); border-radius: 8px; background: var(--bg-3); color: var(--text); box-shadow: var(--shadow); display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-md); }
    .toast.critical { border-left-color: var(--lvl-fatal); }
    .toast.warning { border-left-color: var(--lvl-warn); }
    .head { display: flex; align-items: center; gap: 8px; min-width: 0; }
    .kind { white-space: nowrap; }
    .inst { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; font-size: var(--fs-xs); }
    .time { margin-left: auto; font-size: var(--fs-xs); white-space: nowrap; }
    .msg { color: var(--text-2); line-height: 1.4; overflow-wrap: anywhere; }
    .actions { display: flex; gap: 6px; }
    .x { all: unset; position: absolute; top: 6px; right: 8px; cursor: pointer; color: var(--text-2); font-size: var(--fs-lg); line-height: 1; }
    .x:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  `,
})
export class AlertToasts {
  protected readonly alerts = inject(AlertsStore);
  private readonly actions = inject(LiveAlerts);
  private readonly ws = inject(WorkspaceStore);

  protected kind(t: AlertEntry): string {
    return kindLabel(t.alert);
  }

  protected time(t: AlertEntry): string {
    return formatTime(t.alert.to, this.ws.timezone()).slice(0, 8);
  }

  protected show(t: AlertEntry): void {
    this.actions.show(t.alert);
    this.alerts.dismissToast(t.id);
  }
}
