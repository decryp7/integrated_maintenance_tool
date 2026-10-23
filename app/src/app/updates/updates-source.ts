import { computed, inject, Injectable } from '@angular/core';
import { HostNotification, NotificationSource } from '../health/notification-host.model';
import { UpdatesCommands } from './updates.commands';
import { UpdatesStore } from './updates.store';

/** "Debrief x.y is available" after an automatic check, as a host notification with View. */
@Injectable({ providedIn: 'root' })
export class UpdatesSource implements NotificationSource {
  private readonly store = inject(UpdatesStore);
  private readonly commands = inject(UpdatesCommands);

  readonly items = computed<HostNotification[]>(() => {
    const version = this.store.notice();
    if (!version) return [];
    return [
      {
        cause: `update:${version}`,
        severity: 'info',
        title: $localize`:@@updates.toast.text:Debrief ${version}:version: is available`,
        testId: 'updates-toast',
        actions: [{ label: $localize`:@@updates.toast.view:View`, kind: 'primary', run: () => this.view() }],
        dismiss: () => this.store.dismissNotice(),
      },
    ];
  });

  private view(): void {
    this.store.dismissNotice();
    this.commands.open();
  }
}
