import {
  effect,
  EnvironmentInjector,
  EnvironmentProviders,
  inject,
  Injectable,
  makeEnvironmentProviders,
  provideAppInitializer,
  signal,
  untracked,
} from '@angular/core';
import { Logger } from '../core/logger';
import { DockService } from '../dock/dock.service';
import { DEBRIEF_COMMANDS, DEBRIEF_VIEWS, DebriefCommand, DebriefView } from '../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS, UiStateContribution } from '../lifecycle/ui-state';
import { NOTIFICATIONS_VIEW } from './notice-actions';
import { NotificationsSource } from './notifications-source';
import { provideNotificationSource } from '../health/provide-notification-host';
import { NotificationsBackend } from './notifications-backend';
import { NotificationsStore } from './notifications.store';
import { NoticesRecord } from './notifications.types';

export function notificationsViews(): DebriefView[] {
  return [
    {
      id: NOTIFICATIONS_VIEW,
      title: $localize`:View title@@notifications.view.title:Notifications`,
      icon: 'bell',
      category: 'core',
      defaultPosition: 'right',
      area: 'status',
      component: () => import('./notifications-panel').then((m) => m.NotificationsPanel),
    },
  ];
}

export function notificationsCommands(store: InstanceType<typeof NotificationsStore>): DebriefCommand[] {
  const category = $localize`:Command palette category@@notifications.category:View`;
  return [
    { id: 'notifications.show', title: $localize`:@@notifications.cmd.show:View: Notifications`, category, view: NOTIFICATIONS_VIEW, run: (ctx) => ctx.showView(NOTIFICATIONS_VIEW) },
    { id: 'notifications.markAllRead', title: $localize`:@@notifications.cmd.markAll:Notifications: Mark all read`, category, when: () => store.unread() > 0, run: () => void store.markAllRead() },
  ];
}

/** The project's messages in the project and session (key `notifications`, `NoticesRecord` v1). */
@Injectable({ providedIn: 'root' })
export class NotificationsPersistence {
  private readonly backend = inject(NotificationsBackend);
  private readonly logger = inject(Logger);
  private readonly store = inject(NotificationsStore);
  readonly record = signal<NoticesRecord | null>(null);
  readonly warning = signal<string | null>(null);

  constructor() {
    let last = '';
    effect(() => {
      const key = this.store
        .snapshot()
        .notices.filter((n) => n.scope === 'project')
        .map((n) => `${n.id}:${n.count}:${n.read}`)
        .join(',');
      if (key === last) return;
      last = key;
      untracked(() => void this.sync());
    });
  }

  async sync(): Promise<void> {
    try {
      const r = await this.backend.record();
      this.record.set(r.notices.length ? r : null);
    } catch (e) {
      this.logger.warn('notifications', 'Notifications could not be read for saving', { Error: e instanceof Error ? e.message : String(e) });
    }
  }

  async restore(value: unknown): Promise<void> {
    try {
      const r = await this.backend.restore(value ?? null);
      this.warning.set(r.warning);
      this.store.apply({ ...r.snapshot, posted: null });
      await this.sync();
    } catch (e) {
      this.logger.warn('notifications', 'Saved notifications not restored', { Error: e instanceof Error ? e.message : String(e) });
    }
  }

  readonly contribution: UiStateContribution = {
    key: 'notifications',
    capture: () => this.record(),
    restore: (v) => void this.restore(v),
    reset: () => void this.restore(null),
    tracksChanges: false,
  };
}

/** Opens Notifications when an OS notification was clicked. */
export function startNotifications(): void {
  const injector = inject(EnvironmentInjector);
  const store = inject(NotificationsStore);
  const dock = inject(DockService);
  effect(
    () => {
      if (store.focus() !== null) untracked(() => dock.show(NOTIFICATIONS_VIEW));
    },
    { injector },
  );
}

/** One line in `app.config.ts`: Notifications (view, button, commands, toasts, saved project messages). */
export function provideNotifications(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: DEBRIEF_VIEWS, multi: true, useFactory: notificationsViews },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => notificationsCommands(inject(NotificationsStore)) },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => [inject(NotificationsPersistence).contribution] },
    provideNotificationSource(NotificationsSource),
    provideAppInitializer(() => startNotifications()),
  ]);
}
