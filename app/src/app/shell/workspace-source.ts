import { computed, inject, Injectable } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { HostNotification, NotificationSource } from '../health/notification-host.model';

/** The workspace's error and notice ("Saved Case 7") as host notifications: one cause each, updated in place. */
@Injectable({ providedIn: 'root' })
export class WorkspaceSource implements NotificationSource {
  private readonly store = inject(WorkspaceStore);

  readonly items = computed<HostNotification[]>(() => {
    const out: HostNotification[] = [];
    const error = this.store.error();
    if (error) out.push({ cause: 'workspace.error', severity: 'error', title: error, testId: 'workspace-error', dismiss: () => this.store.dismissError() });
    const notice = this.store.notice();
    if (notice) {
      const action = this.store.noticeAction();
      out.push({
        cause: 'workspace.notice',
        severity: 'success',
        title: notice,
        testId: 'workspace-notice',
        actions: action ? [{ label: action.label, kind: 'ghost', testId: 'notice-action', run: () => this.store.runNoticeAction() }] : [],
        dismiss: () => this.store.setNotice(null),
        hold: () => this.store.holdNotice(),
        release: () => this.store.releaseNotice(),
      });
    }
    return out;
  });
}
