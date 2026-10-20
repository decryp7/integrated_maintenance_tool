import { inject, Injectable, Injector } from '@angular/core';
import { AiBackend } from '../ai/ai-backend';
import { LifecycleBackend } from '../core/tools-backend';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { JobViewer, JOBS_VIEW } from '../jobs/job-viewer';
import { JobsBackend } from '../jobs/jobs-backend';
import { JobsStore } from '../jobs/jobs.store';
import { DialogService } from '../ui/dialog.service';
import { Notice, NoticeAction } from './notifications.types';
import { NotificationsStore } from './notifications.store';

export const NOTIFICATIONS_VIEW = 'notifications';

/** The label of an action's button. */
export function actionLabel(a: NoticeAction): string {
  switch (a.type) {
    case 'viewJob':
      return $localize`:@@notifications.action.view:View`;
    case 'retryJob':
      return $localize`:@@notifications.action.retry:Retry`;
    case 'reviewProposals':
      return $localize`:@@notifications.action.review:Review proposals`;
    case 'resetPlugin':
      return $localize`:@@notifications.action.resetPlugin:Reset plugin`;
    case 'setUpFormat':
      return $localize`:@@notifications.action.setUpFormat:Set up format`;
    case 'fixTimestamps':
      return $localize`:@@notifications.action.fixTimestamps:Fix timestamps`;
    case 'resetAi':
      return $localize`:@@notifications.action.resetAi:Reset AI`;
    case 'installUpdate':
      return $localize`:@@notifications.action.installUpdate:Install update`;
    case 'showAlert':
      return $localize`:@@notifications.action.showAlert:Show`;
    case 'showJobs':
      return $localize`:@@notifications.action.showJobs:Show in Jobs`;
    case 'showInFolder':
      return $localize`:@@notifications.action.showInFolder:Show in folder`;
    case 'updateAi':
      return $localize`:@@notifications.action.updateAi:Update`;
    case 'laterAiUpdate':
      return $localize`:@@notifications.action.laterAiUpdate:Later`;
    case 'skipAiUpdate':
      return $localize`:@@notifications.action.skipAiUpdate:Skip this version`;
  }
}

/** Actions that answer the message: once done it is dismissed (an AI update was installed, put off or skipped). */
function answers(a: NoticeAction): boolean {
  return a.type === 'updateAi' || a.type === 'laterAiUpdate' || a.type === 'skipAiUpdate';
}

/**
 * Runs a message's action with the existing feature (View of the job, Retry, the plugin reset, the format wizard,
 * Reset AI, the update download, the live alert's evidence). Running it marks the message read.
 */
@Injectable({ providedIn: 'root' })
export class NoticeActions {
  private readonly injector = inject(Injector);

  /** Runs `action` of `notice`; resolves with an error message when it failed. */
  async run(notice: Notice, action: NoticeAction): Promise<string | null> {
    const store = this.injector.get(NotificationsStore);
    // Toasts of the UI's own (negative ids) are not messages of the centre.
    if (notice.id > 0) void store.markRead(notice.id);
    try {
      const err = await this.perform(action);
      if (!err && answers(action) && notice.id > 0) await store.dismiss(notice.id);
      return err;
    } catch (e) {
      return e instanceof Error ? e.message : typeof e === 'string' ? e : $localize`:@@notifications.action.failed:That did not work.`;
    }
  }

  private async perform(a: NoticeAction): Promise<string | null> {
    const ctx = this.injector.get(COMMAND_CONTEXT);
    switch (a.type) {
      case 'viewJob':
      case 'reviewProposals': {
        const job = this.injector.get(JobsStore).snapshot().jobs.find((j) => j.id === a.jobId);
        return this.injector.get(JobViewer).view({ id: a.jobId, title: job?.title ?? '', resultType: job?.resultType ?? null });
      }
      case 'retryJob':
        await this.injector.get(JobsBackend).retry(a.jobId);
        ctx.showView(JOBS_VIEW);
        return null;
      case 'showJobs':
        ctx.showView(JOBS_VIEW);
        return null;
      case 'showInFolder':
        await this.injector.get(JobsBackend).reveal(a.jobId);
        return null;
      case 'resetPlugin':
        await this.injector.get(LifecycleBackend).pluginReset(a.pluginId);
        return null;
      case 'setUpFormat': {
        const { FormatWizard } = await import('../format/format-wizard');
        this.injector.get(DialogService).open(FormatWizard, {
          title: $localize`:@@notifications.action.setUpFormatTitle:Set up format`,
          inputs: { path: a.path, name: a.path.split(/[\\/]/).pop() ?? a.path },
          width: 760,
        });
        return null;
      }
      case 'fixTimestamps': {
        const { FormatWizard } = await import('../format/format-wizard');
        this.injector.get(DialogService).open(FormatWizard, {
          title: $localize`:@@notifications.action.fixTimestampsTitle:Fix timestamps`,
          inputs: { path: a.path, name: a.path.split(/[\\/]/).pop() ?? a.path, timestampFormat: a.format },
          width: 760,
        });
        return null;
      }
      case 'resetAi':
        await this.injector.get(AiBackend).reset();
        return null;
      case 'installUpdate': {
        const { UpdatesStore } = await import('../updates/updates.store');
        await this.injector.get(UpdatesStore).download();
        return null;
      }
      case 'updateAi':
      case 'laterAiUpdate':
      case 'skipAiUpdate': {
        const { AiUpdatesStore } = await import('../ai/updates/ai-updates.store');
        const updates = this.injector.get(AiUpdatesStore);
        const ok = a.type === 'updateAi' ? await updates.apply(a.key) : a.type === 'laterAiUpdate' ? await updates.later(a.key) : await updates.skip(a.key);
        if (a.type === 'updateAi' && ok) ctx.showView(JOBS_VIEW);
        return ok ? null : updates.error();
      }
      case 'showAlert': {
        const [{ AlertsStore }, { LiveAlerts }] = await Promise.all([import('../live/alerts.store'), import('../live/live-alerts')]);
        const entry = this.injector.get(AlertsStore).alerts().find((e) => e.alert.id === a.alertId);
        if (entry) this.injector.get(LiveAlerts).show(entry.alert);
        else ctx.showView('live-alerts');
        return null;
      }
    }
  }
}
