import { ChangeDetectionStrategy, Component, computed, inject, input, linkedSignal, signal } from '@angular/core';
import { AiBackend, toAiError } from '../ai/ai-backend';
import { DIALOG_REF } from '../ui/dialog.service';
import { QuerySource, Segment } from './browser.types';
import { Segments } from './chat-cards';

/**
 * Research the web: the question, the search text the AI drafted from it and the event (the engine's text without a
 * model), with what the leak guard found in it, and how many results to read. Nothing is searched before the user has
 * seen the search text here: they write it, or let the AI suggest it ("Suggest search text"), and edit it. With workspace data in it, the user confirms
 * it ("Research anyway"). The research then runs as a job and its answer goes to the chat.
 */
@Component({
  selector: 'imt-research-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Segments],
  template: `
    <label for="research-question" i18n="@@browser.researchDialog.question">What should the AI find out?</label>
    <textarea id="research-question" class="input" rows="2" maxlength="1000" [value]="q()" (input)="q.set(asValue($event))" data-testid="research-question"></textarea>
    <label for="research-query" i18n="@@browser.researchDialog.query">Search text</label>
    <input id="research-query" class="input" type="text" maxlength="300" [value]="text()" (input)="edit(asValue($event))" aria-describedby="research-query-note" data-testid="research-query" />
    <p id="research-query-note" class="muted small" data-testid="research-source">{{ sourceNote() }}</p>
    @if (flagged().length) {
      <div class="flagged" role="alert" data-testid="research-flagged">
        <p><imt-segments [segments]="flagged()" /></p>
        <p class="warn small" i18n="@@browser.researchDialog.flagged">Highlighted parts come from your logs or look private. Edit them out, or research anyway to send them to the search engine.</p>
      </div>
    }
    <label for="research-results" i18n="@@browser.researchDialog.results">Results to read</label>
    <select id="research-results" class="select" [value]="results()" (change)="results.set(asValue($event))" data-testid="research-results">
      <option value="" i18n="@@browser.researchDialog.resultsDefault">Default (Settings ▸ Web browser)</option>
      <option value="10" i18n="@@browser.researchDialog.results10">10</option>
      <option value="20" i18n="@@browser.researchDialog.results20">20</option>
      <option value="30" i18n="@@browser.researchDialog.results30">30</option>
    </select>
    <p class="muted small" i18n="@@browser.researchDialog.note">The AI searches with your search engine in a real browser window next to the app and reads the result pages only (no logins, forms or typing). It runs as a job; the answer arrives in the chat with its sources.</p>
    @if (error(); as e) {
      <p class="err" role="alert">{{ e }}</p>
    }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@browser.researchDialog.cancel">Cancel</button>
      <button type="button" class="btn" [disabled]="!q().trim() || busy()" [title]="q().trim() ? suggestTitle : needQuestion" (click)="draft()" data-testid="research-draft" i18n="@@browser.researchDialog.suggest">Suggest search text</button>
      <button type="button" class="btn primary" [disabled]="!q().trim() || !text().trim() || busy()" [title]="startTitle()" (click)="run()" data-testid="research-run">{{ runLabel() }}</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 6px; }
    p { margin: 0; }
    .small { font-size: var(--fs-xs); }
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
  /** Who wrote `query` (null: the user, or nobody yet). */
  readonly source = input<QuerySource | null>(null);
  readonly sessionId = input<string | null>(null);
  readonly recordId = input<number | null>(null);
  readonly maxResults = input<number | null>(null);

  protected readonly q = linkedSignal(() => this.question());
  protected readonly text = linkedSignal(() => this.query());
  protected readonly flagged = linkedSignal(() => this.segments());
  protected readonly by = linkedSignal(() => this.source());
  protected readonly results = linkedSignal(() => (this.maxResults() ? String(this.maxResults()) : ''));
  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly suggestTitle = $localize`:@@browser.researchDialog.suggestTitle:The AI writes the search text from the question; nothing is searched yet`;
  protected readonly needQuestion = $localize`:@@browser.researchDialog.needQuestion:Say what the AI should find out first`;
  protected readonly runLabel = computed(() =>
    this.flagged().length ? $localize`:@@browser.researchDialog.anyway:Research anyway` : $localize`:@@browser.researchDialog.startResearch:Start research`,
  );
  /** Why Start research is off, or what it does. */
  protected readonly startTitle = computed(() => {
    if (!this.q().trim()) return this.needQuestion;
    if (!this.text().trim()) return $localize`:@@browser.researchDialog.needText:Write a search text, or let the AI suggest one`;
    return $localize`:@@browser.researchDialog.startTitle:Searches with this text and reads the results; the answer arrives in the chat`;
  });
  protected readonly sourceNote = computed(() => {
    switch (this.by()) {
      case 'model':
        return $localize`:@@browser.researchDialog.byModel:The AI wrote it from the question and the event, asked to leave out ids, names, paths and other values of this installation. Check it before it is sent.`;
      case 'engine':
        return $localize`:@@browser.researchDialog.byEngine:Built by IMT (the AI could not write it). Check it before it is sent.`;
      default:
        return $localize`:@@browser.researchDialog.byUser:Only this text goes to the search engine.`;
    }
  });

  protected asValue(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }

  /** Editing the search text drops the old findings (the engine checks the new text) and makes it the user's. */
  protected edit(v: string): void {
    this.text.set(v);
    this.flagged.set([]);
    this.by.set(null);
  }

  /** The AI writes the search text for the question (and the event): shown here, nothing is searched. */
  protected async draft(): Promise<void> {
    this.busy.set(true);
    this.error.set(null);
    try {
      const d = await this.backend.researchDraft({ question: this.q().trim(), recordId: this.recordId() });
      this.text.set(d.checked.query);
      this.flagged.set(d.checked.needsConfirm ? d.checked.segments : []);
      this.by.set(d.source);
    } catch (e) {
      this.error.set(toAiError(e).message);
    } finally {
      this.busy.set(false);
    }
  }

  protected async run(): Promise<void> {
    // The search never runs on a text the user has not seen.
    if (!this.text().trim()) return;
    this.busy.set(true);
    this.error.set(null);
    try {
      const r = await this.backend.research({
        question: this.q().trim(),
        query: this.text().trim(),
        sessionId: this.sessionId(),
        recordId: null,
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
