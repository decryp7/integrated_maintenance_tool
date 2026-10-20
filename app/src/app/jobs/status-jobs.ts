import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { plural } from '../i18n/locale';
import { NOTIFICATIONS_VIEW } from '../notifications/notice-actions';
import { NotificationsStore } from '../notifications/notifications.store';
import { Icon } from '../shared/icon';
import { JOBS_VIEW } from './job-viewer';
import { JobsStore } from './jobs.store';

/** Status-bar entries: the running jobs (opens the Jobs pane) and the unread notifications (opens Notifications). */
@Component({
  selector: 'imt-status-jobs',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    @if (jobs.activeCount() > 0) {
      <button type="button" class="pill" data-testid="status-jobs" [title]="jobsTitle()" [attr.aria-label]="jobsTitle()" (click)="ctx.showView(jobsView)">
        <imt-icon name="jobs" [size]="12" />{{ jobs.activeCount() }}
      </button>
    }
    <button type="button" class="pill" data-testid="status-notifications" [class.on]="notices.unread() > 0" [title]="noticesTitle()" [attr.aria-label]="noticesTitle()" (click)="ctx.showView(noticesView)">
      <imt-icon name="bell" [size]="12" />
      @if (notices.unread() > 0) {
        {{ notices.unread() }}
      }
    </button>
  `,
  styles: `
    :host { display: inline-flex; gap: 8px; align-items: center; }
    .pill { display: inline-flex; align-items: center; gap: 4px; height: 18px; padding: 0 7px; border-radius: 9px; border: 1px solid transparent; background: transparent; color: inherit; cursor: pointer; font-size: var(--fs-xs); font-weight: 600; }
    .pill:hover, .pill.on { border-color: currentColor; }
    .pill:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
  `,
})
export class StatusJobs {
  protected readonly jobs = inject(JobsStore);
  protected readonly notices = inject(NotificationsStore);
  protected readonly ctx = inject(COMMAND_CONTEXT);
  protected readonly jobsView = JOBS_VIEW;
  protected readonly noticesView = NOTIFICATIONS_VIEW;

  protected readonly jobsTitle = computed(() => {
    const n = this.jobs.activeCount();
    return plural(n, {
      one: $localize`:@@jobs.status.one:${n}:count: job running or waiting`,
      other: $localize`:@@jobs.status.other:${n}:count: jobs running or waiting`,
    });
  });

  protected readonly noticesTitle = computed(() => {
    const n = this.notices.unread();
    return n === 0
      ? $localize`:@@notifications.status.none:Notifications`
      : plural(n, {
          one: $localize`:@@notifications.status.one:${n}:count: unread notification`,
          other: $localize`:@@notifications.status.other:${n}:count: unread notifications`,
        });
  });
}
