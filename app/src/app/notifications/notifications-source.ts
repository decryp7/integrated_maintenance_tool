import { computed, inject, Injectable, Injector } from '@angular/core';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { HostAction, HostNotification, NotificationSource } from '../health/notification-host.model';
import { actionLabel, NoticeActions, NOTIFICATIONS_VIEW } from './notice-actions';
import { NotificationsStore } from './notifications.store';
import { Notice, NoticeAction } from './notifications.types';

/**
 * The notification centre's new messages and the UI's own toasts as host notifications: the engine's cause key is the
 * toast's cause, so a repeated message updates the toast in place (with its count) instead of adding another.
 */
@Injectable({ providedIn: 'root' })
export class NotificationsSource implements NotificationSource {
  private readonly store = inject(NotificationsStore);
  private readonly actions = inject(NoticeActions);
  private readonly injector = inject(Injector);

  readonly items = computed<HostNotification[]>(() => this.store.toasts().map((t) => this.adapt(t)));

  private adapt(t: Notice): HostNotification {
    const run = async (a: NoticeAction): Promise<void> => {
      const err = await this.actions.run(t, a);
      if (err) throw new Error(err);
      this.store.dismissToast(t.id);
    };
    const act = (a: NoticeAction | null, kind: HostAction['kind'], id: string): HostAction[] => (a ? [{ label: actionLabel(a), kind, id, run: () => run(a) }] : []);
    const actions: HostAction[] = [...act(t.primary, 'primary', 'primary')];
    // A question with three answers ("Update", "Later", "Skip this version") shows all of them.
    if (t.tertiary) actions.push(...act(t.secondary, 'default', 'secondary'), ...act(t.tertiary, 'default', 'tertiary'));
    if (t.id > 0) actions.push({ label: $localize`:@@notifications.toasts.show:Show in Notifications`, kind: 'ghost', id: 'show', run: () => this.show(t) });
    return {
      cause: t.cause,
      severity: t.severity,
      title: t.title,
      body: t.body,
      count: t.count,
      actions,
      dismiss: () => this.store.dismissToast(t.id),
      stamp: `${t.at}:${t.count}`,
    };
  }

  private show(t: Notice): void {
    this.store.dismissToast(t.id);
    this.store.setFocus(t.id);
    this.injector.get(COMMAND_CONTEXT).showView(NOTIFICATIONS_VIEW);
  }
}
