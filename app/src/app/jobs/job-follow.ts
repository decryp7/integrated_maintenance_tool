import { DestroyRef, inject, Injectable, Injector } from '@angular/core';
import { NotificationsStore } from '../notifications/notifications.store';
import { JobsBackend } from './jobs-backend';
import { JobOutcome, JobResult } from './jobs.types';

/**
 * Work the UI starts as a background job (an AI request, an export, a survey): the "running in Jobs" toast, and
 * following the job while the dialog that started it is open. Closing the dialog only stops following: the job
 * goes on and its message (View, Show in folder) tells the user when it is done.
 */
@Injectable({ providedIn: 'root' })
export class JobFollow {
  private readonly injector = inject(Injector);
  private readonly backend = inject(JobsBackend);

  /** "Exporting report.html — running in Jobs", with "Show in Jobs". */
  started(title: string): void {
    this.injector.get(NotificationsStore).showLocal({ severity: 'info', title, primary: { type: 'showJobs' } });
  }

  /** A request that did not start (refused, queue full): says why. */
  notStarted(title: string, reason: string): void {
    this.injector.get(NotificationsStore).showLocal({ severity: 'error', title, body: reason });
  }

  /**
   * Starts work with `start` (resolves with the job at once), then follows it while `owner` lives. Resolves
   * with the job's result, or null when it was cancelled or the dialog closed first; rejects when it did not
   * start or failed (with the engine's reason).
   */
  async run(start: () => Promise<number>, owner: DestroyRef, started?: (id: number) => void): Promise<JobResult | null> {
    const id = await start();
    started?.(id);
    const end = await this.follow(id, owner);
    if (!end) return null;
    if (end.state === 'failed') throw new Error(end.error ?? $localize`:@@jobs.follow.failed:The job failed.`);
    return end.state === 'done' ? end.result : null;
  }

  /**
   * Waits for job `id` while `owner` lives: its outcome, or null once `owner` was destroyed (the dialog closed;
   * the job then posts its message as usual).
   */
  async follow(id: number, owner: DestroyRef): Promise<JobOutcome | null> {
    let closed = false;
    const off = owner.onDestroy(() => {
      closed = true;
      void this.backend.unwait(id).catch(() => undefined);
    });
    const outcome = await this.backend.wait(id);
    if (closed) return null;
    off();
    return outcome;
  }
}
