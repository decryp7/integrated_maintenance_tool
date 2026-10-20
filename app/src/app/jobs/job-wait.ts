import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { JobsBackend } from './jobs-backend';
import { queuedLabel } from './jobs-text';
import { JobsStore } from './jobs.store';

/**
 * A dialog's request while it runs as a job: where it is (queued, its progress), Cancel, and that the dialog may
 * be closed (the job goes on and its message tells when it is done).
 */
@Component({
  selector: 'imt-job-wait',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="wait" role="status" data-testid="job-wait">
      <span class="line">{{ line() }}</span>
      <button type="button" class="btn ghost small" [disabled]="cancelling()" (click)="cancel()" data-testid="job-wait-cancel" i18n="@@jobs.wait.cancel">Cancel</button>
    </div>
    <p class="hint" i18n="@@jobs.wait.hint">You can close this dialog: the request goes on in Jobs and tells you when it is done.</p>
    @if (error(); as e) {
      <p class="err" role="alert">{{ e }}</p>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 2px; }
    .wait { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
    .line { color: var(--text-muted, inherit); }
    .hint { margin: 0; font-size: 12px; color: var(--text-muted, inherit); }
    .err { margin: 0; color: var(--danger-text, var(--lvl-error)); }
  `,
})
export class JobWait {
  private readonly backend = inject(JobsBackend);
  private readonly jobs = inject(JobsStore);
  readonly jobId = input.required<number>();
  /** What runs ("The local model proposes; the engine checks it…"), shown while no progress is reported. */
  readonly busyText = input('');
  protected readonly cancelling = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly line = computed(() => {
    const job = this.jobs.snapshot().jobs.find((j) => j.id === this.jobId());
    if (job?.state === 'queued') return queuedLabel(job.position);
    return job?.message ?? this.busyText();
  });

  protected async cancel(): Promise<void> {
    this.cancelling.set(true);
    try {
      await this.backend.cancel(this.jobId());
    } catch (e) {
      this.cancelling.set(false);
      this.error.set(e instanceof Error ? e.message : String(e));
    }
  }
}
