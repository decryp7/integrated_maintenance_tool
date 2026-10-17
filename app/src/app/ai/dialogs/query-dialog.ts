import { ChangeDetectionStrategy, Component, inject, input, OnInit, signal } from '@angular/core';
import { DIALOG_REF } from '../../ui/dialog.service';
import { AiBackend, toAiError } from '../ai-backend';
import { QueryProposal } from '../ai.types';
import { AiProposalCard } from '../proposals/proposal-card';

/** "Ask AI to write a query": a question in plain words → a query the engine checked, then Apply. */
@Component({
  selector: 'imt-ai-query-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AiProposalCard],
  template: `
    <form (submit)="$event.preventDefault(); ask()">
      <label for="ai-query-question">What do you want to find?</label>
      <div class="row">
        <input id="ai-query-question" class="input" maxlength="2000" placeholder="e.g. errors on WEB02 in the last hour" [value]="question()" (input)="question.set(asValue($event))" />
        <button type="submit" class="btn primary" [disabled]="!question().trim() || busy()">{{ busy() ? 'Writing…' : 'Write query' }}</button>
      </div>
    </form>
    @if (busy()) {
      <p class="muted" role="status">The local model is writing the query; the engine checks it…</p>
    }
    @if (error(); as e) {
      <p class="err" role="alert">{{ e }}</p>
    }
    @if (proposal(); as p) {
      <imt-ai-proposal-card [proposal]="p" (applied)="ref.close(true)" (dismissed)="proposal.set(null)" />
    }
    <div class="foot"><button type="button" class="btn ghost" (click)="ref.close()">Close</button></div>
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
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(AiBackend);
  readonly initial = input('');
  protected readonly question = signal('');
  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly proposal = signal<QueryProposal | null>(null);

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
      this.proposal.set(await this.backend.nlToQuery(q));
    } catch (e) {
      this.error.set(toAiError(e).message);
    } finally {
      this.busy.set(false);
    }
  }
}
