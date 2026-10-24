import { computed, inject, Injectable } from '@angular/core';
import { formatTime } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { HostNotification, HostSeverity, NotificationSource } from '../health/notification-host.model';
import { AlertEntry, AlertsStore, kindLabel, severityLabel } from './alerts.store';
import { LiveAlerts } from './live-alerts';

/** Live alert toasts as host notifications ("Show" and "Mute this kind for 15 min"); each alert is its own cause. */
@Injectable({ providedIn: 'root' })
export class AlertsSource implements NotificationSource {
  private readonly alerts = inject(AlertsStore);
  private readonly live = inject(LiveAlerts);
  private readonly ws = inject(WorkspaceStore);

  readonly items = computed<HostNotification[]>(() => this.alerts.toasts().map((t) => this.adapt(t)));

  private adapt(t: AlertEntry): HostNotification {
    const where = t.alert.instance ? ` · ${t.alert.instance}` : '';
    const severity: HostSeverity = t.alert.severity === 'critical' ? 'critical' : t.alert.severity === 'warning' ? 'warning' : 'info';
    return {
      cause: `live-alert:${t.id}`,
      severity,
      title: `${kindLabel(t.alert)}${where}`,
      body: `${severityLabel(t.alert.severity)} · ${formatTime(t.alert.to, this.ws.timezone()).slice(0, 8)} ${t.alert.message}`,
      ttl: 10_000,
      actions: [
        { label: $localize`:@@live.alertToasts.show:Show`, kind: 'default', run: () => this.show(t) },
        { label: $localize`:@@live.alertToasts.mute:Mute this kind for 15 min`, kind: 'ghost', run: () => this.alerts.mute(t.alert) },
      ],
      dismiss: () => this.alerts.dismissToast(t.id),
    };
  }

  private show(t: AlertEntry): void {
    this.live.show(t.alert);
    this.alerts.dismissToast(t.id);
  }
}
