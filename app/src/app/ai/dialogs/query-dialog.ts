import { ChangeDetectionStrategy, Component, computed, DestroyRef, inject, input, OnInit, signal } from '@angular/core';
import { JobFollow } from '../../jobs/job-follow';
import { JobWait } from '../../jobs/job-wait';
import { DIALOG_REF } from '../../ui/dialog.service';
import { AiBackend, toAiError } from '../ai-backend';
import { QueryProposal } from '../ai.types';
import { AiProposalCard } from '../proposals/proposal-card';

/** "Ask AI to write a query": a question in plain words → a query the engine checked, then Apply. */
@Component({
  selector: 'debrief-ai-query-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AiProposalCard, JobWait],
  template: `
    <form (submit)="$event.preventDefault(); ask()">
      <label for="ai-query-question" i18n="@@ai.query.label">What do you want to find?</label>
      <div class="row">
        <input id="ai-query-question" class="input" maxlength="2000" placeholder="e.g. errors on WEB02 in the last hour" i18n-placeholder="@@ai.query.placeholder" [value]="question()" (input)="question.set(asValue($event))" />
        <button type="submit" class="btn primary" [disabled]="!question().trim() || busy()">{{ writeLabel() }}</button>
      </div>
    </form>
    @if (jobId(); as id) {
      <debrief-job-wait [jobId]="id" [busyText]="busyText" />
    }
    @if (error(); as e) {
      <p class="err" role="alert">{{ e }}</p>
    }
    @if (proposal(); as p) {
      <debrief-ai-proposal-card [proposal]="p" (applied)="ref.close(true)" (dismissed)="proposal.set(null)" />
    }
    <div class="foot"><button type="button" class="btn ghost" (click)="ref.close()" i18n="@@ai.query.close">Close</button></div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    form { display: flex; flex-direction: column; gap: 4px; }
    .row { display: flex; gap: 8px; }
    .row .input { flex: 1; }
    .err { margin: 0; color: var(--danger-text, var(--lvl-error)); }
    .foot { display: flex; justify-content: flex-end; }
  `,
})
export class AiQueryDialog implements OnInit {
  static readonly dialogId = 'ai-query';
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(AiBackend);
  private readonly follow = inject(JobFollow);
  private readonly destroyRef = inject(DestroyRef);
  readonly initial = input('');
  /** The AI job of the running request (the dialog follows it; closing leaves it running). */
  protected readonly jobId = signal<number | null>(null);
  protected readonly busyText = $localize`:@@ai.query.busy:The local model is writing the query; the engine checks it…`;
  protected readonly question = signal('');
  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly proposal = signal<QueryProposal | null>(null);
  protected readonly writeLabel = computed(() => (this.busy() ? $localize`:@@ai.query.writing:Writing…` : $localize`:@@ai.query.write:Write query`));

  ngOnInit(): void {
    this.question.set(this.initial());
    if (this.initial().trim()) void this.ask();
  }

  protected asValue(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }

  async ask(): Promise<void> {
    const q = this.question().trim();
    if (!q || this.busy()) return;
    this.busy.set(true);
    this.error.set(null);
    this.proposal.set(null);
    try {
      const result = await this.follow.run(() => this.backend.start({ op: 'query', question: q }), this.destroyRef, (id) => this.jobId.set(id));
      if (result?.type === 'proposal') this.proposal.set(result.proposal as QueryProposal);
    } catch (e) {
      this.error.set(toAiError(e).message);
    } finally {
      this.busy.set(false);
      this.jobId.set(null);
    }
  }
}
