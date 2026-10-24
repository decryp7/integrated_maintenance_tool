import { inject, Injectable, Injector } from '@angular/core';
import { AiBackend } from '../ai/ai-backend';
import { LifecycleBackend } from '../core/tools-backend';
import { ServiceHealthStore } from '../health/service-health.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { JobViewer, JOBS_VIEW } from '../jobs/job-viewer';
import { JobsBackend } from '../jobs/jobs-backend';
import { JobsStore } from '../jobs/jobs.store';
import { DockService } from '../dock/dock.service';
import { DialogService } from '../ui/dialog.service';
import { Notice, NoticeAction } from './notifications.types';
import { NoticeTarget, NotificationsStore } from './notifications.store';

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
    case 'openAssistant':
      return $localize`:@@notifications.action.openAssistant:Open the assistant`;
    case 'setUpAssistant':
      return $localize`:@@notifications.action.setUpAssistant:Retry`;
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
    case 'repairService':
      return $localize`:@@notifications.action.repairService:Try again`;
    case 'openSettings':
      return $localize`:@@notifications.action.openSettings:Open Settings…`;
  }
}

/** The job an action works on (View, Review proposals, Retry, Show in folder); null for actions that need none. */
export function actionJob(a: NoticeAction): number | null {
  switch (a.type) {
    case 'viewJob':
    case 'reviewProposals':
    case 'retryJob':
    case 'showInFolder':
      return a.jobId;
    default:
      return null;
  }
}

/**
 * Whether `a` can still run: false for a job action whose job is no longer in the Jobs list (cleared, or gone with a
 * restart) — the engine no longer has its result, the message keeps the outcome. `listed` is null while the list is
 * not loaded yet (then every action stays offered).
 */
export function actionAvailable(a: NoticeAction, listed: ReadonlySet<number> | null): boolean {
  const job = actionJob(a);
  return job === null || listed === null || listed.has(job);
}

/** The ids of the window's listed jobs, or null while the list has not loaded. */
export function listedJobs(jobs: InstanceType<typeof JobsStore>): ReadonlySet<number> | null {
  return jobs.loaded() ? new Set(jobs.snapshot().jobs.map((j) => j.id)) : null;
}

/** The note shown instead of the actions of a job that was cleared from Jobs. */
export function jobClearedNote(): string {
  return $localize`:@@notifications.action.jobCleared:This job was cleared from Jobs; the outcome above is all that is kept.`;
}

/** Where `a` takes the user (see `NoticeTarget`): every action declares it, so none leaves Notifications open over what it opened. */
export function actionTarget(a: NoticeAction): NoticeTarget {
  switch (a.type) {
    case 'viewJob':
    case 'reviewProposals':
    case 'retryJob':
    case 'showJobs':
    case 'openAssistant':
    case 'setUpAssistant':
    case 'updateAi':
    case 'showAlert':
      return 'view';
    case 'setUpFormat':
    case 'fixTimestamps':
    case 'openSettings':
      return 'window';
    case 'showInFolder':
    case 'resetPlugin':
    case 'resetAi':
    case 'installUpdate':
    case 'laterAiUpdate':
    case 'skipAiUpdate':
    case 'repairService':
      return null;
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
      if (!err) this.leaveFor(actionTarget(action));
      return err;
    } catch (e) {
      return e instanceof Error ? e.message : typeof e === 'string' ? e : $localize`:@@notifications.action.failed:That did not work.`;
    }
  }

  /** Runs the action of a message of the UI's own (it is done), then goes where it declared. */
  runLocal(id: number): void {
    this.leaveFor(this.injector.get(NotificationsStore).runLocal(id));
  }

  /**
   * The one place that decides what Notifications does after an action: an action that went somewhere slides the
   * Notifications window in (a docked or popped-out one stays), and one that opened a view moves the focus to it.
   * An action that acts in place, or failed, leaves Notifications as it was.
   */
  private leaveFor(target: NoticeTarget): void {
    if (target === null) return;
    const dock = this.injector.get(DockService, null);
    dock?.slideIn(NOTIFICATIONS_VIEW);
    if (target === 'view') dock?.focusFront();
  }

  private async perform(a: NoticeAction): Promise<string | null> {
    const ctx = this.injector.get(COMMAND_CONTEXT);
    if (!actionAvailable(a, listedJobs(this.injector.get(JobsStore)))) return jobClearedNote();
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
      case 'openAssistant':
        ctx.showView('ai-chat');
        return null;
      case 'setUpAssistant':
        await this.injector.get(AiBackend).setupBackground(a.modelId);
        ctx.showView(JOBS_VIEW);
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
      case 'repairService':
        await this.injector.get(ServiceHealthStore).repair(a.service);
        return null;
      case 'openSettings': {
        const { SettingsDialogService } = await import('../settings/settings-dialog');
        await this.injector.get(SettingsDialogService).open(a.section);
        return null;
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
