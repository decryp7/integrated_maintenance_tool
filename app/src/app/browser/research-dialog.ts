import { ChangeDetectionStrategy, Component, inject, input, linkedSignal, signal } from '@angular/core';
import { AiBackend, toAiError } from '../ai/ai-backend';
import { DIALOG_REF } from '../ui/dialog.service';
import { Segment } from './browser.types';
import { Segments } from './chat-cards';

/**
 * Research the web: the question, the search text the engine built (with what the leak guard found in it), and how
 * many results to read. The engine checks the search text; with workspace data in it, the user confirms it here
 * (or edits it) before anything is searched. The research then runs as a job and its answer goes to the chat.
 */
@Component({
  selector: 'imt-research-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Segments],
  template: `
    <label for="research-question" i18n="@@browser.researchDialog.question">What should the AI find out?</label>
    <textarea id="research-question" class="input" rows="2" maxlength="1000" [value]="q()" (input)="q.set(asValue($event))" data-testid="research-question"></textarea>
    <label for="research-query" i18n="@@browser.researchDialog.query">Search text (optional; the question is used when empty)</label>
    <input id="research-query" class="input" type="text" maxlength="300" [value]="text()" (input)="edit(asValue($event))" data-testid="research-query" />
    @if (flagged().length) {
      <div class="flagged" role="alert" data-testid="research-flagged">
        <p><imt-segments [segments]="flagged()" /></p>
        <p class="warn small" i18n="@@browser.researchDialog.flagged">Highlighted parts come from your logs or look private. Edit them out, or research anyway to send them to the search engine.</p>
      </div>
    }
    <label for="research-results" i18n="@@browser.researchDialog.results">Results to read</label>
    <select id="research-results" class="select" [value]="results()" (change)="results.set(asValue($event))" data-testid="research-results">
      <option value="" i18n="@@browser.researchDialog.resultsDefault">Default (Settings ▸ Web browser)</option>
      <option value="3" i18n="@@browser.researchDialog.results3">3</option>
      <option value="5" i18n="@@browser.researchDialog.results5">5</option>
      <option value="10" i18n="@@browser.researchDialog.results10">10</option>
    </select>
    <p class="muted small" i18n="@@browser.researchDialog.note">The AI searches with your search engine in IMT's browser and reads the result pages only (no logins, forms or typing). It runs as a job; the answer arrives in the chat with its sources.</p>
    @if (error(); as e) {
      <p class="err" role="alert">{{ e }}</p>
    }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@browser.researchDialog.cancel">Cancel</button>
      <button type="button" class="btn primary" [disabled]="!q().trim() || busy()" (click)="run()" data-testid="research-run">{{ flagged().length ? anyway : start }}</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 6px; }
    p { margin: 0; }
    .small { font-size: 11.5px; }
    .warn { color: var(--warn-text); }
    .err { color: var(--danger-text, var(--lvl-error)); }
    .flagged { border: 1px solid var(--warn-text); border-radius: 6px; padding: 6px 8px; display: flex; flex-direction: column; gap: 4px; }
    .foot { display: flex; justify-content: flex-end; gap: 8px; margin-top: 6px; }
  `,
})
export class ResearchDialog {
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(AiBackend);
  readonly question = input('');
  readonly query = input('');
  /** What the leak guard found in `query` (empty: nothing to confirm). */
  readonly segments = input<Segment[]>([]);
  readonly sessionId = input<string | null>(null);
  readonly recordId = input<number | null>(null);
  readonly maxResults = input<number | null>(null);

  protected readonly q = linkedSignal(() => this.question());
  protected readonly text = linkedSignal(() => this.query());
  protected readonly flagged = linkedSignal(() => this.segments());
  protected readonly results = linkedSignal(() => (this.maxResults() ? String(this.maxResults()) : ''));
  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly start = $localize`:@@browser.researchDialog.start:Research`;
  protected readonly anyway = $localize`:@@browser.researchDialog.anyway:Research anyway`;

  protected asValue(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }

  /** Editing the search text drops the old findings: the engine checks the new text. */
  protected edit(v: string): void {
    this.text.set(v);
    this.flagged.set([]);
  }

  protected async run(): Promise<void> {
    this.busy.set(true);
    this.error.set(null);
    try {
      const r = await this.backend.research({
        question: this.q().trim(),
        query: this.text().trim() || null,
        sessionId: this.sessionId(),
        recordId: this.text().trim() ? null : this.recordId(),
        maxResults: this.results() ? Number(this.results()) : null,
        // Only what the user saw highlighted is confirmed.
        confirmed: this.flagged().length > 0,
      });
      if (r.status === 'confirm') {
        this.text.set(r.checked.query);
        this.flagged.set(r.checked.segments);
      } else {
        this.ref.close(r.jobId);
      }
    } catch (e) {
      this.error.set(toAiError(e).message);
    } finally {
      this.busy.set(false);
    }
  }
}
